//! Safety contract: path normalization, must-read-before-write, per-session write
//! quota, audit log, trash path. 1:1 port of the C# `Safety/SafetyManager.cs`.
//!
//! Session state is in memory, keyed by `(person, project_id)`.

use crate::config::SafetyConfig;
use crate::errors::{Result, ToolError};
use crate::util::{PosixPath, now_unix};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

const AUDIT_CAP: usize = 500;

/// A single recorded mutation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditEntry {
    pub timestamp: f64,
    pub op: String,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Default)]
struct SessionState {
    read_paths: HashSet<String>,
    bytes_written: i64,
    audit: VecDeque<AuditEntry>,
}

pub struct SafetyManager {
    config: SafetyConfig,
    /// The metadata backend's path ceiling, from
    /// [`crate::storage::meta::max_path_len`]. `None` when the backend stores
    /// unbounded text, which is every engine except SQL Server.
    max_path_len: Option<usize>,
    sessions: Mutex<HashMap<(String, String), SessionState>>,
}

impl SafetyManager {
    /// `max_path_len` is required rather than optional so every construction has to
    /// state the backend's ceiling: a call site that silently defaulted to no limit
    /// would let an over long path reach the database and fail there instead.
    pub fn new(config: SafetyConfig, max_path_len: Option<usize>) -> Self {
        Self { config, max_path_len, sessions: Mutex::new(HashMap::new()) }
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.config
    }

    /// `ERR_INVALID_ARGUMENT` when `path` cannot fit in the metadata backend.
    ///
    /// The message is read by an LLM, so it carries the measurement, the ceiling,
    /// the engine that imposes it and a concrete remedy: a caller that only learns
    /// "too long" cannot tell whether to shorten a name or give up. The path is NOT
    /// rewritten or truncated, because writing to a location the caller did not ask
    /// for is a worse failure than refusing.
    pub fn ensure_path_fits(&self, path: &str) -> Result<()> {
        let Some(limit) = self.max_path_len else {
            return Ok(());
        };
        // Characters, not bytes: the ceiling comes from NVARCHAR(n), which counts
        // UTF-16 code units, and every column is sized in those same units.
        let len = path.chars().count();
        if len <= limit {
            return Ok(());
        }
        let backend = crate::config::backend::SQLSERVER;
        Err(ToolError::invalid_argument(format!(
            "path is {len} characters but this backend ({backend}) allows at most {limit}: \
             shorten a directory name or move the entry nearer the volume root, for example \
             '/src/x.rs' rather than '/very/deeply/nested/tree/of/directories/x.rs'"
        )))
    }

    fn with_session<T>(
        &self,
        person: &str,
        project: &str,
        f: impl FnOnce(&mut SessionState) -> T,
    ) -> T {
        let mut guard = self.sessions.lock().expect("safety mutex poisoned");
        let s = guard.entry((person.to_string(), project.to_string())).or_default();
        f(s)
    }

    /// Normalize an in-volume path. Rejects NUL bytes and anything escaping the root.
    pub fn normalize_path(&self, path: &str) -> Result<String> {
        if path.contains('\0') {
            return Err(ToolError::path_out_of_bounds("path contains a NUL byte"));
        }
        let candidate = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        let normalized = PosixPath::normpath(&candidate);
        if !normalized.starts_with('/') || normalized.starts_with("/..") {
            return Err(ToolError::path_out_of_bounds(format!(
                "path escapes the volume root: {path}"
            )));
        }
        // Checked on the normalized form, which is what gets stored, and before any
        // database call so the caller gets a deterministic, actionable error rather
        // than a driver level failure. `parent` shares the column width but is always
        // a prefix of the path, so a path that fits guarantees a parent that fits.
        self.ensure_path_fits(&normalized)?;
        Ok(normalized)
    }

    pub fn record_read(&self, person: &str, project: &str, path: &str) {
        self.with_session(person, project, |s| {
            s.read_paths.insert(path.to_string());
        });
    }

    /// `ERR_EDIT_WITHOUT_PRIOR_READ` unless the file was read in this session.
    /// A no-op when `safety.read_guard` is false.
    pub fn ensure_read_before_write(&self, person: &str, project: &str, path: &str) -> Result<()> {
        if !self.config.read_guard {
            return Ok(());
        }
        let seen = self.with_session(person, project, |s| s.read_paths.contains(path));
        if seen {
            Ok(())
        } else {
            Err(ToolError::edit_without_prior_read(format!(
                "edit '{path}' requires reading it first in this session"
            )))
        }
    }

    /// Charge bytes against the session quota, rejecting when it would be exceeded.
    pub fn charge_write(&self, person: &str, project: &str, num_bytes: i64) -> Result<()> {
        let quota = self.config.write_quota_bytes;
        self.with_session(person, project, |s| {
            if s.bytes_written + num_bytes > quota {
                return Err(ToolError::write_quota_exceeded(format!(
                    "session write quota of {quota} bytes exceeded"
                )));
            }
            s.bytes_written += num_bytes;
            Ok(())
        })
    }

    /// Give back bytes charged for a write that never landed.
    ///
    /// Only sound when the caller knows nothing was written, which is what the
    /// atomic apply's pass 1 pre-check guarantees: it refuses before the first
    /// byte moves. Clamped at zero so a double refund cannot mint quota.
    pub fn refund_write(&self, person: &str, project: &str, num_bytes: i64) {
        let _: Result<()> = self.with_session(person, project, |s| {
            s.bytes_written = (s.bytes_written - num_bytes).max(0);
            Ok(())
        });
    }

    pub fn record_audit(&self, person: &str, project: &str, op: &str, path: &str, detail: &str) {
        self.with_session(person, project, |s| {
            s.audit.push_back(AuditEntry {
                timestamp: now_unix(),
                op: op.to_string(),
                path: path.to_string(),
                detail: detail.to_string(),
            });
            while s.audit.len() > AUDIT_CAP {
                s.audit.pop_front();
            }
        });
    }

    /// The session audit log, oldest first.
    pub fn audit(&self, person: &str, project: &str) -> Vec<AuditEntry> {
        self.with_session(person, project, |s| s.audit.iter().cloned().collect())
    }

    pub fn bytes_written(&self, person: &str, project: &str) -> i64 {
        self.with_session(person, project, |s| s.bytes_written)
    }

    /// Trash destination for a soft delete: `/{trash_dir}/{epoch_ms}__{flattened path}`.
    pub fn trash_path(&self, path: &str) -> String {
        let flat = path.trim_matches('/').replace('/', "__");
        let stamp = (now_unix() * 1000.0) as i64;
        format!("/{}/{stamp}__{flat}", self.config.trash_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> SafetyManager {
        SafetyManager::new(SafetyConfig::default(), None)
    }

    #[test]
    fn normalize_makes_paths_absolute() {
        let m = mgr();
        assert_eq!(m.normalize_path("a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/./b/../c.txt").unwrap(), "/a/c.txt");
        assert_eq!(m.normalize_path("/").unwrap(), "/");
    }

    /// A SQL Server deployment, whose `path` column is bounded.
    fn sqlserver_mgr() -> SafetyManager {
        let limit = crate::storage::meta::max_path_len(crate::config::backend::SQLSERVER);
        // Not a literal: the ceiling is the SQL Server clustered key budget minus the
        // longest project id, and the guard must move with it.
        assert_eq!(limit, Some(crate::storage::meta::MAX_PATH_CHARS));
        SafetyManager::new(SafetyConfig::default(), limit)
    }

    /// Built from a path of exactly `len` characters, so the boundary is exercised
    /// rather than approximated.
    fn path_of_len(len: usize) -> String {
        let p = format!("/{}", "a".repeat(len - 1));
        assert_eq!(p.chars().count(), len);
        p
    }

    #[test]
    fn a_path_at_the_backend_limit_is_accepted() {
        let m = sqlserver_mgr();
        let at_limit = path_of_len(crate::storage::meta::MAX_PATH_CHARS);
        assert_eq!(m.normalize_path(&at_limit).unwrap(), at_limit);
    }

    #[test]
    fn one_character_past_the_limit_is_rejected_with_an_actionable_message() {
        let m = sqlserver_mgr();
        let limit = crate::storage::meta::MAX_PATH_CHARS;
        let e = m.normalize_path(&path_of_len(limit + 1)).unwrap_err();

        // Caller input, so INVALID_ARGUMENT: a driver level failure would surface as
        // INTERNAL_ERROR and tell an LLM caller nothing it could act on.
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert_ne!(e.code, crate::errors::code::INTERNAL_ERROR);

        // The message has to carry the measurement, the ceiling, the engine and a
        // remedy, because it is consumed by an LLM deciding what to do next.
        assert!(
            e.message.contains(&(limit + 1).to_string()),
            "states the actual length: {}",
            e.message
        );
        assert!(e.message.contains(&limit.to_string()), "states the limit: {}", e.message);
        assert!(e.message.contains("sqlserver"), "names the backend: {}", e.message);
        assert!(
            e.message.contains("shorten") && e.message.contains("nearer the volume root"),
            "offers a remedy: {}",
            e.message
        );
    }

    /// The ceiling exists only because SQL Server cannot index unbounded text, so a
    /// SQLite or PostgreSQL deployment must not inherit it.
    #[test]
    fn the_limit_does_not_apply_to_sqlite_or_postgres() {
        for backend in [crate::config::backend::SQLITE, crate::config::backend::POSTGRES] {
            let limit = crate::storage::meta::max_path_len(backend);
            assert_eq!(limit, None, "{backend} stores unbounded text");
            let m = SafetyManager::new(SafetyConfig::default(), limit);
            let long = path_of_len(4000);
            assert_eq!(m.normalize_path(&long).unwrap(), long, "{backend} must accept it");
        }
    }

    /// The guard counts characters, not bytes: the column is NVARCHAR(n), which is
    /// sized in UTF-16 units, so a multi byte name must not be charged twice.
    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let m = sqlserver_mgr();
        // Accented characters up to the ceiling: the count fits, the byte length does not.
        let limit = crate::storage::meta::MAX_PATH_CHARS;
        let p = format!("/{}", "é".repeat(limit - 1));
        assert_eq!(p.chars().count(), limit);
        assert!(p.len() > limit, "byte length exceeds the limit, character length does not");
        assert!(m.normalize_path(&p).is_ok(), "a path at the ceiling must be accepted");
    }

    /// Normalization shortens, so the check must run on the stored form: a long path
    /// that collapses under `..` is legitimate and must not be refused.
    #[test]
    fn the_limit_applies_after_normalization() {
        let m = sqlserver_mgr();
        let collapsing = format!("/{}/../short.txt", "a".repeat(600));
        assert_eq!(m.normalize_path(&collapsing).unwrap(), "/short.txt");
    }

    /// The guard has to be reachable through the real entry point, not only by
    /// calling it directly: every `fs.*` tool and every REST route normalizes first,
    /// so one check there covers all of them.
    #[test]
    fn every_caller_hits_the_guard_through_normalize_path() {
        let m = sqlserver_mgr();
        let over = path_of_len(crate::storage::meta::MAX_PATH_CHARS + 1);
        // A relative path is made absolute first, so the guard sees the stored form.
        let relative = over.trim_start_matches('/').to_string();
        for candidate in [over, relative] {
            let e = m.normalize_path(&candidate).unwrap_err();
            assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        }
    }

    #[test]
    fn normalize_rejects_nul_byte() {
        let m = mgr();
        let e = m.normalize_path("/a\0b").unwrap_err();
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS);
        assert!(e.message.contains("NUL"));
    }

    /// Traversal is neutralized by normalization: an absolute path cannot escape.
    #[test]
    fn traversal_is_contained_not_escaped() {
        let m = mgr();
        assert_eq!(m.normalize_path("/../../etc/passwd").unwrap(), "/etc/passwd");
        assert_eq!(m.normalize_path("../../etc/passwd").unwrap(), "/etc/passwd");
        assert_eq!(m.normalize_path("/a/../../b").unwrap(), "/b");
    }

    #[test]
    fn read_guard_blocks_unread_edit() {
        let m = mgr();
        let e = m.ensure_read_before_write("a@b.c", "p", "/f.txt").unwrap_err();
        assert_eq!(e.code, crate::errors::code::EDIT_WITHOUT_PRIOR_READ);

        m.record_read("a@b.c", "p", "/f.txt");
        m.ensure_read_before_write("a@b.c", "p", "/f.txt").unwrap();
    }

    #[test]
    fn read_guard_is_per_person_and_per_project() {
        let m = mgr();
        m.record_read("a@b.c", "p1", "/f.txt");
        // another person has not read it
        assert!(m.ensure_read_before_write("other@b.c", "p1", "/f.txt").is_err());
        // same person, another project
        assert!(m.ensure_read_before_write("a@b.c", "p2", "/f.txt").is_err());
    }

    #[test]
    fn read_guard_can_be_disabled() {
        let cfg = SafetyConfig { read_guard: false, ..Default::default() };
        let m = SafetyManager::new(cfg, None);
        m.ensure_read_before_write("a@b.c", "p", "/never-read.txt").unwrap();
    }

    #[test]
    fn quota_accumulates_and_rejects() {
        let cfg = SafetyConfig { write_quota_bytes: 10, ..Default::default() };
        let m = SafetyManager::new(cfg, None);
        m.charge_write("a@b.c", "p", 6).unwrap();
        assert_eq!(m.bytes_written("a@b.c", "p"), 6);
        m.charge_write("a@b.c", "p", 4).unwrap();
        assert_eq!(m.bytes_written("a@b.c", "p"), 10);

        let e = m.charge_write("a@b.c", "p", 1).unwrap_err();
        assert_eq!(e.code, crate::errors::code::WRITE_QUOTA_EXCEEDED);
        assert!(e.message.contains("10 bytes exceeded"));
        // a rejected write does not consume quota
        assert_eq!(m.bytes_written("a@b.c", "p"), 10);
    }

    #[test]
    fn quota_is_per_session() {
        let cfg = SafetyConfig { write_quota_bytes: 5, ..Default::default() };
        let m = SafetyManager::new(cfg, None);
        m.charge_write("a@b.c", "p", 5).unwrap();
        // a different person has a fresh quota
        m.charge_write("other@b.c", "p", 5).unwrap();
    }

    #[test]
    fn audit_records_in_order_and_is_capped() {
        let m = mgr();
        m.record_audit("a@b.c", "p", "write", "/a.txt", "");
        m.record_audit("a@b.c", "p", "edit", "/a.txt", "1 replacement");
        let log = m.audit("a@b.c", "p");
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].op, "write");
        assert_eq!(log[1].op, "edit");
        assert_eq!(log[1].detail, "1 replacement");
        assert!(log[0].timestamp > 0.0);

        for i in 0..AUDIT_CAP + 50 {
            m.record_audit("a@b.c", "p", "write", &format!("/f{i}.txt"), "");
        }
        assert_eq!(m.audit("a@b.c", "p").len(), AUDIT_CAP, "log is capped");
    }

    #[test]
    fn trash_path_flattens_and_timestamps() {
        let m = mgr();
        let t = m.trash_path("/a/b/c.txt");
        assert!(t.starts_with("/.mcp_trash/"), "got {t}");
        assert!(t.ends_with("__a__b__c.txt"), "got {t}");
    }

    #[test]
    fn trash_path_honours_configured_dir() {
        let cfg = SafetyConfig { trash_dir: ".bin".into(), ..Default::default() };
        let m = SafetyManager::new(cfg, None);
        assert!(m.trash_path("/x.txt").starts_with("/.bin/"));
    }

    // ── GROUP I: new safety test ───────────────────────────────────────────────

    /// A write records the path as read (see `write_text` in fs_ops: it calls
    /// `safety.record_read` after writing). So a second edit on the same path in
    /// the same session must not be blocked by the read guard, because the first
    /// write already satisfied it.
    #[test]
    fn read_guard_is_cleared_after_write_so_second_edit_doesnt_need_reread() {
        let m = mgr();
        // Simulate what fs_ops::write_text does: it calls record_read after writing.
        m.record_read("a@b.c", "p", "/f.txt");
        // Now the read guard is satisfied; a second ensure_read_before_write must pass.
        m.ensure_read_before_write("a@b.c", "p", "/f.txt").unwrap();
        // Writing again would call record_read again, which is idempotent.
        m.record_read("a@b.c", "p", "/f.txt");
        m.ensure_read_before_write("a@b.c", "p", "/f.txt").unwrap();
    }
}
