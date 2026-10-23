//! SPEC-0051: SQL Server support was removed. These tests drive the real `mcp-fs` binary,
//! because the observables (standard output, standard error, exit status, what lands on
//! disk) only exist at the process boundary.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_mcp-fs");
const ADMIN: &str = "admin@example.com";
const MEMBER: &str = "member@example.com";
const PROJECT: &str = "atlas";
const MSSQL_DSN: &str = "server=h;database=d";

fn removal(section: &str) -> String {
    format!(
        "infra.{section}.backend 'sqlserver' is no longer supported: use 'postgres' or 'sqlite'"
    )
}

// ── configuration ────────────────────────────────────────────────────────────

/// One store section: its backend name and its connection string (empty: none).
type Store<'a> = (&'a str, &'a str);

const SQLITE: Store = ("sqlite", "");

fn config_yaml(port: u16, key: &Path, stores: [Store; 4], extra: &str) -> String {
    let mut y = format!(
        "server:\n  host: 127.0.0.1\n  port: {port}\nauth:\n  jwt:\n    public_key_path: \"{}\"\n  admins: [\"{ADMIN}\"]\n{extra}infra:\n",
        key.display()
    );
    for (name, (backend, dsn)) in ["meta", "admin", "git", "oauth"].into_iter().zip(stores) {
        y.push_str(&format!("  {name}:\n    backend: {backend}\n"));
        if !dsn.is_empty() {
            y.push_str(&format!("    dsn: \"{dsn}\"\n"));
        }
    }
    y
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    l.local_addr().expect("addr").port()
}

/// A scratch deployment: `cwd` is where `state/` lands, `conf` holds the configuration.
struct Deployment {
    cwd: tempfile::TempDir,
    conf: tempfile::TempDir,
    private_key: PathBuf,
    public_key: PathBuf,
}

impl Deployment {
    fn new() -> Self {
        let (cwd, conf) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let out = Command::new(BIN).arg("keys").arg("--dir").arg(conf.path()).output().unwrap();
        assert!(out.status.success(), "keys: {}", String::from_utf8_lossy(&out.stderr));
        Self {
            private_key: conf.path().join("jwt.key"),
            public_key: conf.path().join("jwt.pub"),
            cwd,
            conf,
        }
    }

    fn write_config(&self, name: &str, stores: [Store; 4], extra: &str) -> PathBuf {
        let path = self.conf.path().join(name);
        fs::write(&path, config_yaml(free_port(), &self.public_key, stores, extra)).unwrap();
        path
    }

    fn token(&self, email: &str) -> String {
        let out = Command::new(BIN)
            .args(["token", email, "--key"])
            .arg(&self.private_key)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Run a verb that is expected to end by itself; a hang is a failure.
    fn run(&self, args: &[&str]) -> Output {
        let mut child = Command::new(BIN)
            .args(args)
            .current_dir(self.cwd.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().ok();
                panic!("`mcp-fs {args:?}` was expected to stop by itself");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().unwrap()
    }

    fn serve_refused(&self, config: &Path) -> Output {
        self.run(&["serve", "--config", config.to_str().unwrap()])
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The refusal contract of FR-NEW-001: exit 1, empty stdout, the message as the last
/// standard error line, in the `error: ERR_INVALID_ARGUMENT: ` form.
fn assert_refused(out: &Output, message: &str) {
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(out));
    assert!(out.stdout.is_empty(), "stdout: {}", String::from_utf8_lossy(&out.stdout));
    let last = stderr(out).lines().rfind(|l| !l.trim().is_empty()).unwrap_or_default().to_string();
    assert_eq!(last, format!("error: ERR_INVALID_ARGUMENT: {message}"));
}

fn assert_nothing_written(d: &Deployment) {
    let left: Vec<_> = fs::read_dir(d.cwd.path()).unwrap().collect();
    assert!(left.is_empty(), "the refusal wrote {left:?}");
}

// ── a running server ─────────────────────────────────────────────────────────

struct Server {
    child: Child,
    base: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        // SIGTERM first so SQLite closes cleanly, then the hard kill as a backstop.
        Command::new("kill").arg(self.child.id().to_string()).status().ok();
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

async fn start(d: &Deployment, stores: [Store<'_>; 4]) -> Server {
    let port = free_port();
    let path = d.conf.path().join("serve.yaml");
    fs::write(&path, config_yaml(port, &d.public_key, stores, "")).unwrap();
    let child = Command::new(BIN)
        .args(["serve", "--config"])
        .arg(&path)
        .current_dir(d.cwd.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server { child, base: format!("http://127.0.0.1:{port}") };
    let http = reqwest::Client::new();
    for _ in 0..300 {
        if let Ok(r) = http.get(format!("{}/health", server.base)).send().await
            && r.status().is_success()
        {
            return server;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the server never answered /health");
}

const MCP_ACCEPT: &str = "application/json, text/event-stream";

/// One MCP `tools/call` over the real wire, returning the tool's JSON result.
async fn call_tool(server: &Server, token: &str, name: &str, arguments: Value) -> Value {
    let http = reqwest::Client::new();
    let url = format!("{}/mcp", server.base);
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-03-26", "capabilities": {},
        "clientInfo": {"name": "sql-server-removed", "version": "0.0"}}});
    let r = http
        .post(&url)
        .bearer_auth(token)
        .header("Accept", MCP_ACCEPT)
        .json(&init)
        .send()
        .await
        .unwrap();
    let session = r.headers()["Mcp-Session-Id"].to_str().unwrap().to_string();
    http.post(&url)
        .bearer_auth(token)
        .header("Accept", MCP_ACCEPT)
        .header("Mcp-Session-Id", &session)
        .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}});
    let text = http
        .post(&url)
        .bearer_auth(token)
        .header("Accept", MCP_ACCEPT)
        .header("Mcp-Session-Id", &session)
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let envelope: Value = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
        .next_back()
        .unwrap_or_else(|| panic!("{name}: not JSON: {text}"));
    let content = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: no content: {envelope}"));
    serde_json::from_str(content).unwrap_or_else(|_| panic!("{name}: {content}"))
}

/// The files a member of `atlas` lists at the root, through the REST plane.
async fn list_root(server: &Server, token: &str) -> Vec<String> {
    let r = reqwest::Client::new()
        .get(format!("{}/api/fs/{PROJECT}/list", server.base))
        .bearer_auth(token)
        .query(&[("path", "/")])
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "list: {}", r.status());
    let v: Value = r.json().await.unwrap();
    let mut names: Vec<String> = v["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("no entries in {v}"))
        .iter()
        .filter(|e| e["type"] != "directory")
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

// ── the shared setup: a state written by the previous version ────────────────

fn fixture_state() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state_v1/state_dir")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file under `dir` with its size, for a before and after comparison.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().display().to_string();
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A deployment whose `state/` is a copy of the committed fixture.
fn deployment_with_atlas() -> Deployment {
    let d = Deployment::new();
    copy_dir(&fixture_state(), &d.cwd.path().join("state"));
    d
}

const ALL_SQLITE: [Store; 4] = [SQLITE; 4];

/// Written once, on the previous version's code, to create `tests/fixtures/state_v1`:
/// `cargo test -p mcp-fs --test sql_server_removed spec_0051_regenerate_state_fixture -- --ignored`.
/// Never rerun it to make a failing test pass: the fixture is the proof that a database
/// written by an earlier version still opens.
#[tokio::test]
#[ignore = "regenerates the committed fixture"]
async fn spec_0051_regenerate_state_fixture() {
    let d = Deployment::new();
    let server = start(&d, ALL_SQLITE).await;
    let admin = d.token(ADMIN);
    let created = call_tool(
        &server,
        &admin,
        "admin.create_project",
        json!({"project_id": PROJECT, "owner": MEMBER}),
    )
    .await;
    assert_eq!(created["project_id"], PROJECT);
    let member = d.token(MEMBER);
    for (name, body) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")] {
        let r = reqwest::Client::new()
            .post(format!("{}/api/fs/{PROJECT}/write", server.base))
            .bearer_auth(&member)
            .json(&json!({"path": format!("/{name}"), "content": body, "overwrite": true}))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "write {name}: {}", r.status());
    }
    drop(server);
    let target = fixture_state();
    if target.exists() {
        fs::remove_dir_all(&target).unwrap();
    }
    copy_dir(&d.cwd.path().join("state"), &target);
}

// ── E2E-001, E2E-016: the previous version's database still opens ────────────

#[tokio::test]
async fn spec_0051_a_sqlite_server_starts_and_lists_the_previous_atlas() {
    let d = deployment_with_atlas();
    let server = start(&d, ALL_SQLITE).await;
    let health = reqwest::get(format!("{}/health", server.base)).await.unwrap();
    assert!(health.status().is_success());
    assert_eq!(list_root(&server, &d.token(MEMBER)).await, ["a.txt", "b.txt", "c.txt"]);
}

// ── E2E-005 to E2E-009, E2E-013, E2E-014, E2E-021, E2E-022: startup refusals ──

fn refuse(stores: [Store; 4]) -> (Deployment, Output) {
    let d = Deployment::new();
    let cfg = d.write_config("refuse.yaml", stores, "");
    let out = d.serve_refused(&cfg);
    (d, out)
}

#[test]
fn spec_0051_the_metadata_store_cannot_be_sql_server() {
    let (d, out) = refuse([("sqlserver", MSSQL_DSN), SQLITE, SQLITE, SQLITE]);
    assert_refused(&out, &removal("meta"));
    assert_nothing_written(&d);
}

#[test]
fn spec_0051_the_admin_store_cannot_be_sql_server() {
    let (d, out) = refuse([SQLITE, ("sqlserver", MSSQL_DSN), SQLITE, SQLITE]);
    assert_refused(&out, &removal("admin"));
    assert_nothing_written(&d);
}

#[test]
fn spec_0051_the_oauth_store_cannot_be_sql_server() {
    let (d, out) = refuse([SQLITE, SQLITE, SQLITE, ("sqlserver", MSSQL_DSN)]);
    assert_refused(&out, &removal("oauth"));
    assert_nothing_written(&d);
}

#[test]
fn spec_0051_without_a_connection_string_the_removal_message_still_comes_first() {
    let (_d, out) = refuse([("sqlserver", ""), SQLITE, SQLITE, SQLITE]);
    assert_refused(&out, &removal("meta"));
    assert!(!stderr(&out).contains("dsn"), "{}", stderr(&out));
}

#[test]
fn spec_0051_other_unknown_names_list_two_databases() {
    for name in ["mssql", "oracle"] {
        let (_d, out) = refuse([(name, ""), SQLITE, SQLITE, SQLITE]);
        assert_refused(
            &out,
            &format!("unknown infra.meta.backend '{name}', expected one of sqlite, postgres"),
        );
    }
}

#[test]
fn spec_0051_with_several_sections_naming_sql_server_the_first_is_named() {
    let (_d, out) = refuse([SQLITE, SQLITE, ("sqlserver", MSSQL_DSN), ("sqlserver", MSSQL_DSN)]);
    assert_refused(&out, &removal("git"));
    let (_d, out) = refuse([("sqlserver", MSSQL_DSN), SQLITE, SQLITE, ("sqlserver", MSSQL_DSN)]);
    assert_refused(&out, &removal("meta"));
}

#[test]
fn spec_0051_the_connection_string_is_never_printed() {
    let (_d, out) =
        refuse([("sqlserver", "Server=h;Password=Secret1;Database=d"), SQLITE, SQLITE, SQLITE]);
    assert_refused(&out, &removal("meta"));
    let seen = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    for secret in ["Secret1", "Server=h", "Database=d"] {
        assert!(!seen.contains(secret), "{secret} leaked: {seen}");
    }
}

#[test]
fn spec_0051_the_removal_refusal_comes_before_any_other_configuration_error() {
    let (_d, out) = refuse([("postgres", ""), ("sqlserver", MSSQL_DSN), SQLITE, SQLITE]);
    assert_refused(&out, &removal("admin"));
    let (_d, out) = refuse([("mssql", ""), ("sqlserver", MSSQL_DSN), SQLITE, SQLITE]);
    assert_refused(&out, &removal("admin"));
}

#[test]
fn spec_0051_a_file_that_cannot_be_read_keeps_its_own_error() {
    let d = Deployment::new();
    let cfg = d.write_config(
        "diff.yaml",
        [SQLITE, ("sqlserver", MSSQL_DSN), SQLITE, SQLITE],
        "safety:\n  max_diff_file_bytes: 0\n",
    );
    assert_refused(&d.serve_refused(&cfg), &removal("admin"));

    let broken = d.conf.path().join("broken.yaml");
    fs::write(&broken, "infra: [\n  meta: {backend: sqlserver}\n").unwrap();
    let out = d.serve_refused(&broken);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let last = stderr(&out).lines().rfind(|l| !l.trim().is_empty()).unwrap().to_string();
    assert!(last.starts_with("error: ERR_INVALID_ARGUMENT: invalid config: "), "{last}");
}

// ── E2E-010, E2E-011: migrate ────────────────────────────────────────────────

fn assert_migration_refused(out: &Output, section: &str) {
    assert_refused(out, &removal(section));
    assert!(!stderr(out).contains("migrating"), "{}", stderr(out));
}

#[test]
fn spec_0051_a_migration_from_sql_server_is_refused() {
    let d = Deployment::new();
    let from = d.write_config("from.yaml", [("sqlserver", MSSQL_DSN), SQLITE, SQLITE, SQLITE], "");
    // The destination is PostgreSQL when the opt in service is up, else a SQLite file.
    let pg = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty());
    let to = match &pg {
        Some(dsn) => d.write_config("to.yaml", [("postgres", dsn); 4], "").to_owned(),
        None => d.write_config("to.yaml", ALL_SQLITE, ""),
    };
    let out = d.run(&["migrate", "--from", from.to_str().unwrap(), "--to", to.to_str().unwrap()]);
    assert_migration_refused(&out, "meta");
    assert_nothing_written(&d);
}

#[test]
fn spec_0051_a_migration_to_sql_server_is_refused() {
    let d = deployment_with_atlas();
    let before = snapshot(&d.cwd.path().join("state"));
    let from = d.write_config("from.yaml", ALL_SQLITE, "");
    let to = d.write_config("to.yaml", [("sqlserver", MSSQL_DSN), SQLITE, SQLITE, SQLITE], "");
    let out = d.run(&["migrate", "--from", from.to_str().unwrap(), "--to", to.to_str().unwrap()]);
    assert_migration_refused(&out, "meta");
    assert_eq!(snapshot(&d.cwd.path().join("state")), before, "the source changed");
}

// ── E2E-012, E2E-015: the build option, the lock and the test environment ────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(repo_root());
    c
}

#[test]
fn spec_0051_the_removed_build_option_is_refused() {
    // A scratch target dir: the refusal happens while resolving features, before any
    // build, and this keeps the check clear of the outer cargo's lock.
    let scratch = tempfile::tempdir().unwrap();
    let out = cargo()
        .args(["check", "-p", "mcp-fs", "--features", "sqlserver", "--offline"])
        .env("CARGO_TARGET_DIR", scratch.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("does not contain this feature: sqlserver"),
        "cargo said: {}",
        stderr(&out)
    );

    // `all-backends` stays, and now means PostgreSQL plus vector search.
    let meta = cargo()
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline"])
        .output()
        .unwrap();
    let meta: Value = serde_json::from_slice(&meta.stdout).unwrap();
    for name in ["mcp-fs-core", "mcp-fs"] {
        let pkg = meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap();
        let features = pkg["features"].as_object().unwrap();
        assert!(!features.contains_key("sqlserver"), "{name} still has the feature");
        assert!(features.contains_key("all-backends"), "{name} lost all-backends");
    }
    let core = meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == "mcp-fs-core");
    assert_eq!(core.unwrap()["features"]["all-backends"], json!(["postgres", "rag"]));
}

#[test]
fn spec_0051_the_test_environment_no_longer_offers_sql_server() {
    let lock = fs::read_to_string(repo_root().join("Cargo.lock")).unwrap();
    let packages: Vec<&str> =
        lock.lines().filter_map(|l| l.strip_prefix("name = \"")?.strip_suffix('"')).collect();
    for name in &packages {
        assert!(!name.starts_with("tiberius") && *name != "bb8", "the lock still has {name}");
    }
    assert!(packages.len() > 100, "the lock was not parsed: {} packages", packages.len());

    let compose = fs::read_to_string(repo_root().join("docker-compose.test.yml")).unwrap();
    assert!(compose.lines().any(|l| l == "  postgres:"), "the PostgreSQL service must stay");
    assert!(!compose.to_ascii_lowercase().contains("mssql"), "the compose file still names mssql");
}

// ── E2E-002, E2E-003: PostgreSQL is unchanged (opt in: needs the test service) ──

fn pg_dsn() -> Option<String> {
    std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
}

#[test]
fn spec_0051_the_postgres_suite_passes_unchanged() {
    let Some(dsn) = pg_dsn() else {
        eprintln!(
            "skipping: MCPFS_TEST_PG_DSN is unset (docker compose -f docker-compose.test.yml up -d)"
        );
        return;
    };
    let out = cargo()
        .args(["test", "-p", "mcp-fs-core", "--features", "postgres", "--lib", "--"])
        .args(["conformance", "spec_0024", "pg_open", "pg_null", "--test-threads=4"])
        .env("MCPFS_TEST_PG_DSN", dsn)
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(out.status.success(), "the PostgreSQL suite failed:\n{text}");
    assert!(text.contains("test result: ok."), "{text}");
    // Filters that match nothing also end in `ok`: the suite must really have run.
    assert!(!text.contains("running 0 tests"), "no test matched: {text}");
    assert!(text.contains("postgres_passes_the_conformance_suite ... ok"), "{text}");
}

/// A destination whose four stores live in the PostgreSQL schema `schema`.
fn postgres_yaml(port: u16, key: &Path, dsn: &str, schema: &str) -> String {
    let mut y = config_yaml(port, key, [SQLITE; 4], "");
    y.truncate(y.find("infra:\n").unwrap());
    y.push_str("infra:\n");
    for name in ["meta", "admin", "git", "oauth"] {
        y.push_str(&format!(
            "  {name}:\n    backend: postgres\n    dsn: \"{dsn}\"\n    schema: {schema}\n"
        ));
    }
    y
}

#[cfg(feature = "postgres")]
#[test]
fn spec_0051_the_migration_from_sqlite_to_postgres_still_copies_the_state() {
    let Some(dsn) = pg_dsn() else {
        eprintln!("skipping: MCPFS_TEST_PG_DSN is unset");
        return;
    };
    let d = deployment_with_atlas();
    let schema = format!("spec0051_{}", std::process::id());
    let from = d.write_config("from.yaml", ALL_SQLITE, "");
    let to = d.conf.path().join("to.yaml");
    fs::write(&to, postgres_yaml(free_port(), &d.public_key, &dsn, &schema)).unwrap();

    let out = d.run(&["migrate", "--from", from.to_str().unwrap(), "--to", to.to_str().unwrap()]);
    let psql = |sql: &str| {
        Command::new("psql").args([&dsn, "-tA", "-c", sql]).output().expect("psql on PATH")
    };
    let rows = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>();
    let nodes = psql(&format!("SELECT count(*) FROM \"{schema}\".nodes"));
    psql(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"));
    assert!(out.status.success(), "migrate failed: {}", stderr(&out));
    assert!(rows.is_ok_and(|n| n > 0), "no row total on stdout");
    assert!(stderr(&out).contains("migrating"), "{}", stderr(&out));
    let nodes: u64 = String::from_utf8_lossy(&nodes.stdout).trim().parse().unwrap();
    assert!(nodes >= 3, "the three files of atlas must reach PostgreSQL, found {nodes}");
}
