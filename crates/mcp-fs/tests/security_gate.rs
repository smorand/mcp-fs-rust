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

fn tools_installed() -> bool {
    Command::new("cargo-deny").arg("--version").output().is_ok()
        && Command::new("cargo-audit").arg("--version").output().is_ok()
}

/// SPEC-0035/DT-012: once every accepted advisory is recorded, the audit shows
/// no vulnerability and no warning. The spec expected two yanked zip warnings;
/// crates.io un-yanked zip 2.4.2 and 3.0.0 (checked 2026-10-17), which its
/// assumption on DT-012 anticipated.
#[test]
fn cargo_audit_reports_no_vulnerability_and_no_warning() {
    if !tools_installed() {
        eprintln!("cargo-audit not installed locally; skipping");
        return;
    }
    let out = Command::new("cargo")
        .args(["audit", "--json"])
        .current_dir(repo_root())
        .output()
        .expect("run cargo audit");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("cargo audit json");
    assert_eq!(
        report["vulnerabilities"]["count"], 0,
        "vulnerabilities: {}",
        report["vulnerabilities"]
    );
    assert_eq!(report["warnings"], serde_json::json!({}), "unexpected audit warnings");
    assert!(out.status.success(), "cargo audit must exit 0");
}

/// SPEC-0035/DT-014: no stale git2 ignore, so cargo deny reports no
/// `advisory-not-detected`.
#[test]
fn deny_toml_drops_the_fixed_git2_ignores() {
    let raw = fs::read_to_string(repo_root().join("deny.toml")).expect("deny.toml");
    assert!(raw.contains("RUSTSEC-2023-0071"));
    for id in ["RUSTSEC-2026-0183", "RUSTSEC-2026-0184"] {
        assert!(!raw.contains(id), "deny.toml still ignores {id}");
    }
    if !tools_installed() {
        eprintln!("cargo-deny not installed locally; skipping the run");
        return;
    }
    let out = Command::new("cargo")
        .args(["deny", "check"])
        .current_dir(repo_root())
        .output()
        .expect("run cargo deny");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "cargo deny check failed: {stderr}");
    assert!(!stderr.contains("advisory-not-detected"), "{stderr}");
}

/// SPEC-0035/DT-015: accepted advisories live once in `.cargo/audit.toml`,
/// each with a reason, and the Makefile carries no `--ignore` flag.
#[test]
fn accepted_advisories_are_recorded_once_with_a_reason() {
    let raw = fs::read_to_string(repo_root().join(".cargo/audit.toml")).expect(".cargo/audit.toml");
    // One quoted id per line inside the `ignore` array; text parsing keeps the
    // test free of a toml dependency.
    let mut ids: Vec<&str> =
        raw.lines().filter_map(|l| l.trim().strip_prefix('"')?.split('"').next()).collect();
    ids.sort_unstable();
    let mut expected = [
        "RUSTSEC-2023-0071",
        "RUSTSEC-2026-0192",
        "RUSTSEC-2026-0367",
        "RUSTSEC-2026-0368",
        "RUSTSEC-2026-0369",
    ];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    for id in expected {
        let line = raw.lines().find(|l| l.contains(id)).expect("id line");
        assert!(
            line.split_once('#').is_some_and(|(_, r)| r.trim().len() > 10),
            "{id} has no reason"
        );
    }
    let make = fs::read_to_string(repo_root().join("Makefile")).expect("Makefile");
    assert!(!make.contains("cargo audit --ignore"), "Makefile still passes --ignore");
    assert!(make.contains("\tcargo audit\n"));
}

/// SPEC-0036/DT-030: zip 9 is the only zip among normal dependencies; 3.0.0
/// stays as a build dependency, read from the lockfile then the graph.
#[test]
fn lockfile_and_graph_carry_zip_9_only_for_normal_dependencies() {
    let mut zips = lock_packages("zip");
    zips.sort_unstable();
    assert_eq!(zips.len(), 2, "exactly two zip packages: {zips:?}");
    assert_eq!(zips[0], "3.0.0", "{zips:?}");
    assert!(zips[1].starts_with("9."), "zip must be 9.x, got {zips:?}");

    let old = Command::new("cargo")
        .args(["tree", "-i", "zip@2.4.2", "--all-features"])
        .current_dir(repo_root())
        .output()
        .expect("run cargo tree");
    assert!(!old.status.success(), "zip@2.4.2 must no longer resolve");
    let stderr = String::from_utf8_lossy(&old.stderr);
    assert!(stderr.contains("did not match any packages"), "{stderr}");

    let graph = Command::new("cargo")
        .args(["tree", "--workspace", "--all-features", "-e", "normal", "--prefix", "none"])
        .current_dir(repo_root())
        .output()
        .expect("run cargo tree");
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    let stdout = String::from_utf8_lossy(&graph.stdout);
    let zip_lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("zip v")).collect();
    assert!(!zip_lines.is_empty(), "zip absent from the normal graph");
    for line in zip_lines {
        assert!(line.starts_with("zip v9."), "non 9.x zip in the normal graph: {line}");
    }
}

/// SPEC-0036/DT-031: no comment still cites zip 2.4.2 source locations.
#[test]
fn archive_rs_cites_no_zip_2_4_2_source() {
    let raw = fs::read_to_string(repo_root().join("crates/core/src/tools/archive.rs"))
        .expect("archive.rs");
    assert!(!raw.contains("zip-2.4.2"), "stale zip-2.4.2 citation in archive.rs");
}

/// SPEC-0036/DT-036: the three zip ids stay accepted only for the build time
/// zip 3.0.0 of utoipa-swagger-ui, tracked by BL-0059.
#[test]
fn zip_acceptances_name_the_build_time_copy() {
    for file in [".cargo/audit.toml", "deny.toml"] {
        let raw = fs::read_to_string(repo_root().join(file)).expect(file);
        assert!(!raw.contains("BL-0058"), "{file} still names BL-0058");
        for id in ["RUSTSEC-2026-0367", "RUSTSEC-2026-0368", "RUSTSEC-2026-0369"] {
            let line =
                raw.lines().find(|l| l.contains(id)).unwrap_or_else(|| panic!("{file} lacks {id}"));
            assert!(
                line.contains("utoipa-swagger-ui") && line.contains("BL-0059"),
                "{file}: {line}"
            );
        }
    }
}

fn rust_and_doc_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_and_doc_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs" || e == "md" || e == "toml") {
            out.push(path);
        }
    }
}

/// SPEC-0040/DT-001: `serde_norway` replaced the old YAML crate, so neither it nor
/// `unsafe-libyaml` is in the lockfile or the graph, and no manifest, source or
/// agent doc names the old crate.
#[test]
fn old_yaml_crate_and_unsafe_libyaml_are_gone() {
    // Built by concat so this file does not itself contain the name it forbids.
    let old = concat!("serde", "_yaml");
    assert!(lock_packages(old).is_empty(), "{old} still in Cargo.lock");
    assert!(lock_packages("unsafe-libyaml").is_empty(), "unsafe-libyaml still in Cargo.lock");

    let tree = Command::new("cargo")
        .args(["tree", "-i", "unsafe-libyaml", "--all-features"])
        .current_dir(repo_root())
        .output()
        .expect("run cargo tree");
    assert!(!tree.status.success(), "unsafe-libyaml must no longer resolve");
    let stderr = String::from_utf8_lossy(&tree.stderr);
    assert!(stderr.contains("did not match any packages"), "{stderr}");

    let root = repo_root();
    let mut files = vec![root.join("Cargo.toml")];
    rust_and_doc_files(&root.join("crates"), &mut files);
    let mut docs = Vec::new();
    rust_and_doc_files(&root.join(".agent_docs"), &mut docs);
    files.extend(docs.into_iter().filter(|p| p.extension().is_some_and(|e| e == "md")));
    for f in &files {
        let text = fs::read_to_string(f).expect("read file");
        assert!(!text.contains(old), "{} still names {old}", f.display());
    }
}

/// SPEC-0040/DT-002: the unsafe-libyaml advisory is no longer accepted, because
/// the crate is gone from the build.
#[test]
fn unsafe_libyaml_advisory_is_not_accepted() {
    let raw = fs::read_to_string(repo_root().join(".cargo/audit.toml")).expect(".cargo/audit.toml");
    assert!(!raw.contains("RUSTSEC-2024-0449"), "advisory still listed in .cargo/audit.toml");
}

/// SPEC-0042/DT-002: the lock holds the fixed aws-sdk-s3 and no lru 0.12.5.
#[test]
fn lock_has_fixed_aws_sdk_s3_and_no_lru_0_12_5() {
    let raw = fs::read_to_string(repo_root().join("Cargo.lock")).expect("Cargo.lock");
    let has = |name: &str, version: &str| {
        raw.contains(&format!("name = \"{name}\"\nversion = \"{version}\"\n"))
    };
    assert!(has("aws-sdk-s3", "1.152.0"), "aws-sdk-s3 is not 1.152.0");
    assert!(!has("lru", "0.12.5"), "lru 0.12.5 is still locked");
}

/// `major.minor.patch` of a locked version, numerically comparable.
fn semver_triple(v: &str) -> (u64, u64, u64) {
    let mut parts = v.split('.').map(|p| p.parse::<u64>().expect("numeric version part"));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// SPEC-0048/DT-001: the fixes for RUSTSEC-2026-0382 and RUSTSEC-2026-0381 are
/// read from the lockfile text, so the pin holds even when the advisory
/// database stops listing them.
#[test]
fn lockfile_pins_the_fixed_webpki_releases() {
    for (name, min, line) in
        [("rustls-webpki", (0, 103, 16), (0, 103)), ("webpki-roots", (1, 0, 10), (1, 0))]
    {
        let found = lock_packages(name);
        assert_eq!(found.len(), 1, "{name}: exactly one locked version, found {found:?}");
        let v = semver_triple(&found[0]);
        assert!(
            (v.0, v.1) == line && v >= min,
            "{name} {} is below the fixed release {min:?}",
            found[0]
        );
    }
}

/// SPEC-0055/DT-020: the vulnerable zstd family is read from the lockfile
/// text, so the floor holds even when the advisory database stops listing it.
#[test]
fn lockfile_holds_no_zstd_below_the_fixed_releases() {
    for (name, min) in [("zstd", (0, 14, 3)), ("zstd-safe", (8, 2, 0)), ("zstd-sys", (2, 1, 2))] {
        let found = lock_packages(name);
        assert!(!found.is_empty(), "{name} absent from Cargo.lock");
        for v in &found {
            // `zstd-sys` carries build metadata (`2.1.2+zstd.1.5.7`), dropped here.
            let core = v.split('+').next().unwrap_or(v);
            assert!(semver_triple(core) >= min, "{name} {v} is below the fixed release {min:?}");
        }
    }
}

/// SPEC-0055/DT-022: exactly one git source is allowed, the datafusion
/// repository, so the pinned revision is the only exception to `unknown-git`.
#[test]
fn deny_toml_allows_only_the_datafusion_git_source() {
    let raw = fs::read_to_string(repo_root().join("deny.toml")).expect("deny.toml");
    let lines: Vec<&str> =
        raw.lines().filter(|l| l.trim_start().starts_with("allow-git")).collect();
    assert_eq!(lines.len(), 1, "exactly one allow-git line: {lines:?}");
    let entries: Vec<&str> = lines[0]
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(inside, _)| inside)
        .expect("allow-git array on one line")
        .split(',')
        .map(|e| e.trim().trim_matches('"'))
        .filter(|e| !e.is_empty())
        .collect();
    assert_eq!(entries, vec!["https://github.com/apache/datafusion"]);
    assert!(raw.contains("unknown-git = \"deny\""), "unknown-git must stay deny");
}

/// SPEC-0055/DT-024: the datafusion option type was renamed upstream; no
/// source still names the old one.
#[test]
fn no_source_names_the_old_ndjson_option_type() {
    // Built by concat so this file does not itself contain the name it forbids.
    let old = concat!("NdJson", "ReadOptions");
    let mut files = Vec::new();
    rust_and_doc_files(&repo_root().join("crates"), &mut files);
    for f in files.iter().filter(|p| p.extension().is_some_and(|e| e == "rs")) {
        let text = fs::read_to_string(f).expect("read file");
        assert!(!text.contains(old), "{} still names {old}", f.display());
    }
}

/// SPEC-0055/DT-023: no stale audit ignore. The acceptance file lists neither
/// advisory that left the graph with datafusion 43, and every id it still
/// lists is reported when the audit runs WITHOUT any ignore, so an id that no
/// longer matches a package cannot hide behind the list.
#[test]
fn audit_ignores_name_only_advisories_still_in_the_graph() {
    let raw = fs::read_to_string(repo_root().join(".cargo/audit.toml")).expect(".cargo/audit.toml");
    for gone in ["RUSTSEC-2024-0384", "RUSTSEC-2024-0436", "RUSTSEC-2026-0253"] {
        assert!(!raw.contains(gone), ".cargo/audit.toml still ignores {gone}");
    }
    if !tools_installed() {
        eprintln!("cargo-audit not installed locally; skipping the graph half");
        return;
    }
    // A scratch project holding only the lockfile and an empty ignore list:
    // cargo-audit reads `.cargo/audit.toml` relative to its working directory.
    let scratch = tempfile::tempdir().expect("tempdir");
    fs::copy(repo_root().join("Cargo.lock"), scratch.path().join("Cargo.lock")).expect("lock");
    fs::create_dir(scratch.path().join(".cargo")).expect("mkdir");
    fs::write(scratch.path().join(".cargo/audit.toml"), "[advisories]\nignore = []\n")
        .expect("audit.toml");
    let out = Command::new("cargo")
        .args(["audit", "--json"])
        .current_dir(scratch.path())
        .output()
        .expect("run cargo audit");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("cargo audit json");
    let mut present: Vec<String> = report["vulnerabilities"]["list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["advisory"]["id"].as_str().map(str::to_owned))
        .collect();
    for list in report["warnings"].as_object().into_iter().flat_map(|m| m.values()) {
        present.extend(
            list.as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| w["advisory"]["id"].as_str().map(str::to_owned)),
        );
    }
    let ids = raw.lines().filter_map(|l| l.trim().strip_prefix('"')?.split('"').next());
    for id in ids {
        assert!(present.iter().any(|p| p == id), "{id} is ignored but no package matches it");
    }
}

/// SPEC-0055/DT-025: the git pin is documented with its reason and exit, the
/// documentation index links it, and the follow up entry exists.
#[test]
fn the_datafusion_pin_is_documented_and_indexed() {
    let doc = fs::read_to_string(repo_root().join(".agent_docs/dependencies.md"))
        .expect(".agent_docs/dependencies.md");
    assert!(doc.contains("6f1436fe1407495019d8afec8a9f55663f418d9a"), "pinned revision missing");
    assert!(doc.contains("BL-0089"), "exit condition missing");
    let agents = fs::read_to_string(repo_root().join("AGENTS.md")).expect("AGENTS.md");
    assert!(agents.contains(".agent_docs/dependencies.md"), "AGENTS.md does not index it");
    assert!(
        fs::read_dir(repo_root().join("sdlc/backlog"))
            .expect("sdlc/backlog")
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("BL-0089-")),
        "BL-0089 is missing"
    );
}
