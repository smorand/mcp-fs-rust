//! DR-003: `rust-toolchain.toml` pins the same channel as the workspace's
//! declared `rust-version` (SPEC-0012).

use std::fs;
use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

#[test]
fn toolchain_channel_matches_workspace_rust_version() {
    let root = repo_root();
    let toolchain =
        fs::read_to_string(root.join("rust-toolchain.toml")).expect("rust-toolchain.toml");
    let workspace = fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");

    let rust_version = workspace
        .lines()
        .find_map(|l| l.trim().strip_prefix("rust-version = \""))
        .and_then(|s| s.strip_suffix('"'))
        .expect("workspace rust-version");

    let channel = toolchain
        .lines()
        .find_map(|l| l.trim().strip_prefix("channel = \""))
        .and_then(|s| s.strip_suffix('"'))
        .expect("rust-toolchain.toml channel");

    assert_eq!(channel, rust_version);
}

/// SPEC-0042/DT-001: the floor is 1.94.1, the least aws-sdk-s3 1.152.0 accepts.
#[test]
fn toolchain_channel_is_1_94_1() {
    let raw = fs::read_to_string(repo_root().join("rust-toolchain.toml")).expect("toolchain");
    assert!(
        raw.lines().any(|l| l.trim() == "channel = \"1.94.1\""),
        "channel is not 1.94.1: {raw}"
    );
}
