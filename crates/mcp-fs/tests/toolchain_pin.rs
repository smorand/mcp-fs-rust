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
