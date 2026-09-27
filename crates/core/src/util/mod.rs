//! Small shared utilities: POSIX path semantics, text helpers, identity normalization.

pub mod posix;
pub mod text;

pub use posix::PosixPath;

/// Caseless, trimmed identity used for every ACL comparison.
/// Mirrors the C# `IdentityUtil.Normalize`.
pub fn normalize_identity(person: &str) -> String {
    person.trim().to_lowercase()
}

/// Current wall clock as fractional Unix seconds (the `mtime`/`ctime`/`atime` format).
pub fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// ISO 8601 timestamp with offset, the format stored in `project.created_at`.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_normalization_is_caseless_and_trimmed() {
        assert_eq!(normalize_identity("  Alice@Test.COM "), "alice@test.com");
        assert_eq!(normalize_identity("bob@x.io"), "bob@x.io");
    }

    #[test]
    fn now_unix_is_plausible() {
        // after 2025-01-01 and before 2100
        let t = now_unix();
        assert!(t > 1_735_689_600.0, "timestamp too small: {t}");
        assert!(t < 4_102_444_800.0, "timestamp too large: {t}");
    }

    #[test]
    fn now_iso_contains_offset() {
        let s = now_iso();
        assert!(s.contains('T'), "not ISO: {s}");
        assert!(s.contains('+') || s.ends_with('Z'), "no offset: {s}");
    }
}
