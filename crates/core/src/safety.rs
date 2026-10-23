//! Safety contract: path normalization, must-read-before-write, per-session write
//! quota, audit log, trash path. 1:1 port of the C# `Safety/SafetyManager.cs`.
//!
//! Session state is in memory, keyed by `(person, project_id)`.

use crate::config::SafetyConfig;
use crate::errors::{Result, ToolError};
use crate::util::{PosixPath, now_unix};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

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
    sessions: Mutex<HashMap<(String, String), SessionState>>,
    /// Where a project's storage limit is read (SPEC-0020). Attached once, at
    /// composition, because the engine functions only receive this manager and a
    /// volume, never the whole state. Unattached is a composition bug and makes
    /// a growing write fail closed; a caller where no limit applies says so
    /// with [`SafetyManager::attach_no_limits`].
    limits: OnceLock<LimitsSource>,
}

/// Where project limits come from, once a composition has decided.
enum LimitsSource {
    Admin(Arc<dyn crate::storage::traits::AdminBackend>),
    /// Explicit opt out: no project has a limit (unit tests, the purge CLI).
    Unlimited,
}

impl SafetyManager {
    pub fn new(config: SafetyConfig) -> Self {
        Self { config, sessions: Mutex::new(HashMap::new()), limits: OnceLock::new() }
    }

    /// Read project storage limits from `admin` (SPEC-0020). A second call is
    /// ignored: the composition root attaches the one store the process has.
    pub fn attach_limits(&self, admin: Arc<dyn crate::storage::traits::AdminBackend>) {
        let _ = self.limits.set(LimitsSource::Admin(admin));
    }

    /// Declare that no project limit applies to this manager. Only for callers
    /// that never admit a growing write against a real project store: engine
    /// unit tests and the purge CLI, which only deletes. A second call, or a
    /// call after `attach_limits`, is ignored.
    pub fn attach_no_limits(&self) {
        let _ = self.limits.set(LimitsSource::Unlimited);
    }

    /// The project's storage limit in bytes, `None` when it has none.
    ///
    /// `ERR_INTERNAL_ERROR` when no limits source was attached: answering "no
    /// limit" there would silently disable enforcement for every project.
    pub async fn project_limit(&self, project: &str) -> Result<Option<i64>> {
        match self.limits.get() {
            Some(LimitsSource::Admin(admin)) => admin.get_quota(project).await,
            Some(LimitsSource::Unlimited) => Ok(None),
            None => Err(ToolError::internal(
                "project storage limits are not attached to the safety manager; \
                 refusing the write rather than skipping the project limit check",
            )),
        }
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.config
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
        SafetyManager::new(SafetyConfig::default())
    }

    /// A manager nobody attached a limits source to must refuse to answer, so a
    /// composition that forgot `attach_limits` fails loudly instead of letting
    /// every project write without a ceiling (SPEC-0020 fail closed).
    #[tokio::test]
    async fn unattached_manager_refuses_the_limit_lookup() {
        let m = mgr();
        let err = m.project_limit("p").await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INTERNAL_ERROR);
        assert!(err.message.contains("limits"), "{}", err.message);
    }

    #[tokio::test]
    async fn explicit_opt_out_means_no_limit() {
        let m = mgr();
        m.attach_no_limits();
        assert_eq!(m.project_limit("p").await.unwrap(), None);
    }

    #[test]
    fn normalize_makes_paths_absolute() {
        let m = mgr();
        assert_eq!(m.normalize_path("a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/./b/../c.txt").unwrap(), "/a/c.txt");
        assert_eq!(m.normalize_path("/").unwrap(), "/");
    }

    /// Built from a path of exactly `len` characters, so the boundary is exercised
    /// rather than approximated.
    fn path_of_len(len: usize) -> String {
        let p = format!("/{}", "a".repeat(len - 1));
        assert_eq!(p.chars().count(), len);
        p
    }

    /// No engine this server runs on bounds a path, so normalization never refuses
    /// one for its length.
    #[test]
    fn spec_0051_a_very_long_path_is_not_refused() {
        let m = mgr();
        let long = path_of_len(4000);
        assert_eq!(m.normalize_path(&long).unwrap(), long);
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
        let m = SafetyManager::new(cfg);
        m.ensure_read_before_write("a@b.c", "p", "/never-read.txt").unwrap();
    }

    #[test]
    fn quota_accumulates_and_rejects() {
        let cfg = SafetyConfig { write_quota_bytes: 10, ..Default::default() };
        let m = SafetyManager::new(cfg);
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
        let m = SafetyManager::new(cfg);
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
        let m = SafetyManager::new(cfg);
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
