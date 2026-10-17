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

fn lock_packages(name: &str) -> Vec<String> {
    let raw = fs::read_to_string(repo_root().join("Cargo.lock")).expect("Cargo.lock");
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    for line in raw.lines() {
        if let Some(n) = line.strip_prefix("name = ") {
            current = Some(n.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("version = ")
            && current.as_deref() == Some(name)
        {
            found.push(v.trim_matches('"').to_string());
        }
    }
    found
}

/// SPEC-0035/DT-010: the advisory fix is read from the lockfile text.
#[test]
fn lockfile_pins_git2_0_21_and_libgit2_sys_unchanged() {
    let git2 = lock_packages("git2");
    assert_eq!(git2.len(), 1, "exactly one git2 package: {git2:?}");
    assert!(git2[0].starts_with("0.21."), "git2 must be 0.21.x, got {}", git2[0]);
    assert_eq!(lock_packages("libgit2-sys"), vec!["0.18.8+1.9.7".to_string()]);
}

/// SPEC-0035/DT-013: the resolved graph agrees with the lockfile, second channel.
#[test]
fn resolved_graph_has_no_vulnerable_git2_and_no_yanked_yoke_derive() {
    let tree = |spec: &str| {
        Command::new("cargo")
            .args(["tree", "-i", spec, "--all-features"])
            .current_dir(repo_root())
            .output()
            .expect("run cargo tree")
    };
    for old in ["git2@0.20.4", "yoke-derive@0.8.3"] {
        let out = tree(old);
        assert!(!out.status.success(), "{old} must no longer resolve");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("did not match"),
            "unexpected cargo tree failure for {old}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let new = tree("git2@0.21.0");
    assert!(
        new.status.success(),
        "git2@0.21.0 must resolve: {}",
        String::from_utf8_lossy(&new.stderr)
    );
}
