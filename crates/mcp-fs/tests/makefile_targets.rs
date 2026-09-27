//! DT-012: `make build`/`make test`/`make run` resolve to the documented scripts,
//! with no divergent flags (SPEC-0012, DR-001).

use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn dry_run(target: &str) -> String {
    let out = Command::new("make")
        .args(["-n", target])
        .current_dir(repo_root())
        .output()
        .expect("run make -n");
    assert!(out.status.success(), "make -n {target} failed: {out:?}");
    String::from_utf8(out.stdout).expect("utf8 make output")
}

#[test]
fn make_build_invokes_build_sh() {
    assert!(dry_run("build").contains("build.sh"));
}

#[test]
fn make_test_invokes_test_sh() {
    assert!(dry_run("test").contains("test.sh"));
}

#[test]
fn make_run_invokes_run_sh() {
    assert!(dry_run("run").contains("run.sh"));
}
