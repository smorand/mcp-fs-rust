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
    sessions: Mutex<HashMap<(String, String), SessionState>>,
}

impl SafetyManager {
    pub fn new(config: SafetyConfig) -> Self {
        Self { config, sessions: Mutex::new(HashMap::new()) }
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.config
    }

    fn with_session<T>(&self, person: &str, project: &str, f: impl FnOnce(&mut SessionState) -> T) -> T {
        let mut guard = self.sessions.lock().expect("safety mutex poisoned");
        let s = guard.entry((person.to_string(), project.to_string())).or_default();
        f(s)
    }

    /// Normalize an in-volume path. Rejects NUL bytes and anything escaping the root.
    pub fn normalize_path(&self, path: &str) -> Result<String> {
        if path.contains('\0') {
            return Err(ToolError::path_out_of_bounds("path contains a NUL byte"));
        }
        let candidate =
            if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
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

    #[test]
    fn normalize_makes_paths_absolute() {
        let m = mgr();
        assert_eq!(m.normalize_path("a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/b.txt").unwrap(), "/a/b.txt");
        assert_eq!(m.normalize_path("/a/./b/../c.txt").unwrap(), "/a/c.txt");
        assert_eq!(m.normalize_path("/").unwrap(), "/");
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
}
