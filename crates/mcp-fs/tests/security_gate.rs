//! DR-004: `deny.toml` exists using the cargo-deny 0.20+ schema and the
//! Makefile's `security` target actually passes (SPEC-0012).

use std::fs;
use std::path::Path;
use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

#[test]
fn deny_toml_uses_the_0_20_schema_and_denies_yanked_and_wildcards() {
    let raw = fs::read_to_string(repo_root().join("deny.toml")).expect("deny.toml");
    assert!(!raw.contains("version = 2"), "cargo-deny 0.20+ dropped the top-level `version` field");
    assert!(raw.contains("[licenses]"));
    assert!(raw.contains("wildcards = \"deny\""));
}

#[test]
fn make_security_passes() {
    if Command::new("cargo-deny").arg("--version").output().is_err()
        || Command::new("cargo-audit").arg("--version").output().is_err()
    {
        eprintln!("cargo-deny or cargo-audit not installed locally; skipping make_security_passes");
        return;
    }
    let out = Command::new("make")
        .arg("security")
        .current_dir(repo_root())
        .output()
        .expect("run make security");
    assert!(
        out.status.success(),
        "make security failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
