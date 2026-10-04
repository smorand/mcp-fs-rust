//! Full-stack, opt-in end-to-end test.
//!
//! Boots the real axum server (`app::build`) on an ephemeral port, backed by a
//! **real PostgreSQL** (`infra.meta`/`infra.admin`/`infra.git`) and a **real
//! MinIO/S3** (`infra.blob`), and drives it exactly like an external caller
//! would: real HTTP requests (MCP JSON-RPC and the REST `/api/fs` plane) and a
//! real `git` CLI subprocess for clone/add/remove/commit/push/pull. Nothing here
//! calls an internal Rust function of the tool dispatch path; every assertion
//! crosses the wire.
//!
//! ## Why PostgreSQL and MinIO are mandatory, not skipped
//!
//! Every other opt-in suite in this workspace (`storage::conformance`,
//! `search::e2e`) skips itself gracefully when its DSN env var is unset, so the
//! default `cargo test --workspace` stays green with zero external services.
//! This test keeps that same opt-in *entry point* (it is `#[ignore]`d, so it
//! never runs by accident), but once invoked it treats a missing or unreachable
//! service as a hard failure rather than a silent skip: the whole point of this
//! test is to prove the relational-store and object-store code paths work
//! against the real engines the C# reference and every declared config schema
//! ("postgres"/"minio") actually target, not against the SQLite/local defaults
//! every other unit and e2e test already covers thoroughly.
//!
//! ## Running it
//!
//! ```bash
//! export MCPFS_TEST_PG_DSN=postgres://<user>@127.0.0.1:5432/postgres
//! export MCPFS_MINIO_SECRET_KEY=<your MinIO/S3 secret key>
//! # optional, default to the usual local MinIO console defaults:
//! export MCPFS_MINIO_ENDPOINT=http://127.0.0.1:9000
//! export MCPFS_MINIO_ACCESS_KEY=admin
//! make test-e2e-full
//! ```
//!
//! `make test-e2e-full` is `cargo test -p mcp-fs-core --features postgres --test
//! full_stack_e2e -- --ignored --nocapture`.
//!
//! ## Setup and teardown
//!
//! Setup creates nothing ahead of time: a fresh PostgreSQL **schema**
//! (`e2e_<8 hex>`) and a fresh MinIO **bucket** (`mcpfs-e2e-<8 hex>-<project>`)
//! are provisioned lazily, by the server itself, the same way a real deployment
//! provisions its first volume (`PostgresRelationalDb::connect`'s
//! `CREATE SCHEMA IF NOT EXISTS`, `StoreManager::provision_volume`'s
//! `ensure_bucket`). Teardown is a `Drop` guard ([`Teardown`]), not a final
//! statement: a `Drop` impl runs even when an assertion panics partway through,
//! where a plain "cleanup at the end" block would not, and this test's own
//! premise is that leftover schemas/buckets from a previous *failed* run must
//! never poison the next one.

#![cfg(feature = "postgres")]

use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use mcp_fs_core::app;
use mcp_fs_core::config::{Dsn, ServerConfig};
use mcp_fs_core::keys;
use mcp_fs_core::storage::rel::{PoolSettings, PostgresRelationalDb};
use serde_json::{Value, json};

const ADMIN: &str = "e2e-admin@mcp-fs.test";

// ── environment ──────────────────────────────────────────────────────────────

struct Env {
    pg_dsn: String,
    minio_endpoint: String,
    minio_access_key: String,
    minio_secret_key: String,
}

fn required_env() -> Env {
    let pg_dsn = std::env::var("MCPFS_TEST_PG_DSN").expect(
        "MCPFS_TEST_PG_DSN is required: point it at a running PostgreSQL, e.g. \
         postgres://<user>@127.0.0.1:5432/postgres. This test does not fall back to SQLite: \
         proving the postgres/minio backends work end to end is its entire purpose.",
    );
    let minio_secret_key = std::env::var("MCPFS_MINIO_SECRET_KEY").expect(
        "MCPFS_MINIO_SECRET_KEY is required: point it at a running MinIO/S3's secret key. \
         This test does not fall back to the local blob backend.",
    );
    let minio_endpoint =
        std::env::var("MCPFS_MINIO_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9000".into());
    let minio_access_key =
        std::env::var("MCPFS_MINIO_ACCESS_KEY").unwrap_or_else(|_| "admin".into());
    Env { pg_dsn, minio_endpoint, minio_access_key, minio_secret_key }
}

fn require_git_cli() {
    let ok =
        Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    assert!(ok, "the real `git` CLI is required on PATH for full_stack_e2e");
}

// ── teardown ─────────────────────────────────────────────────────────────────

/// Drops the throwaway PostgreSQL schema and MinIO bucket this run created.
///
/// Runs on a fresh OS thread with its own throwaway current-thread runtime:
/// `Drop::drop` cannot `.await`, and blocking on the still-running multi-thread
/// test runtime from inside its own shutdown path panics ("Cannot start a
/// runtime from within a Tokio runtime"), the same constraint `search::e2e`'s
/// PostgreSQL cleanup works around.
struct Teardown {
    config: Arc<ServerConfig>,
    pg_dsn: String,
    schema: String,
    project_id: String,
}

impl Drop for Teardown {
    fn drop(&mut self) {
        let config = self.config.clone();
        let pg_dsn = self.pg_dsn.clone();
        let schema = self.schema.clone();
        let project_id = self.project_id.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("teardown runtime");
            rt.block_on(async move {
                match mcp_fs_core::storage::build_blob_store(&config, &project_id) {
                    Ok(blob) => {
                        if let Err(e) = blob.remove_bucket().await {
                            eprintln!("full_stack_e2e teardown: bucket removal failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("full_stack_e2e teardown: blob store build failed: {e}"),
                }
                match PostgresRelationalDb::connect(&pg_dsn, &schema, PoolSettings::default()).await
                {
                    Ok(db) => {
                        let sql = format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE");
                        if let Err(e) =
                            sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).execute(db.pool()).await
                        {
                            eprintln!("full_stack_e2e teardown: DROP SCHEMA failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("full_stack_e2e teardown: schema connect failed: {e}"),
                }
            });
        })
        .join()
        .expect("teardown thread must not panic");
    }
}

// ── MCP / REST client helpers ────────────────────────────────────────────────

/// `initialize` + `notifications/initialized`, the handshake `rmcp`'s
/// `LocalSessionManager` + `legacy_session_mode: true` now requires before
/// any `tools/call` (SPEC-0013/US-0008, DR-008/DR-009). Returns the
/// `Mcp-Session-Id` the server issued.
async fn establish_session(client: &reqwest::Client, base: &str, token: &str) -> String {
    let init = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "full-stack-e2e", "version": "0.0"},
        },
    });
    let resp = client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&init)
        .send()
        .await
        .unwrap_or_else(|e| panic!("MCP initialize: request failed: {e}"));
    assert!(resp.status().is_success(), "MCP initialize: http {}", resp.status());
    let session_id = resp
        .headers()
        .get("Mcp-Session-Id")
        .unwrap_or_else(|| panic!("MCP initialize: no Mcp-Session-Id header"))
        .to_str()
        .expect("session id header must be ASCII")
        .to_string();

    let notified = client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("Mcp-Session-Id", &session_id)
        .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap_or_else(|e| panic!("MCP notifications/initialized: request failed: {e}"));
    assert!(
        notified.status().is_success(),
        "MCP notifications/initialized: http {}",
        notified.status()
    );
    session_id
}

/// One JSON-RPC `tools/call`, decoded from the server's SSE framing. Every
/// response on an established session is `text/event-stream`, and the stream
/// can carry a priming frame ahead of the real one, so this takes the LAST
/// `data:` line that parses as JSON rather than assuming the whole body is
/// one frame (mirrors `crates/agent/src/mcp.rs::parse_body`, empirically
/// verified shape, SPEC-0013/US-0008). Panics with the tool name and the
/// server's error payload on failure, so a failing scenario step names
/// itself in the test output instead of surfacing as a generic "assertion
/// failed".
async fn call_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    arguments: Value,
) -> Value {
    let session_id = establish_session(client, base, token).await;
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    });
    let resp = client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("Mcp-Session-Id", &session_id)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("MCP {name}: request failed: {e}"));
    let status = resp.status();
    let text = resp.text().await.expect("mcp response body must be readable");
    assert!(status.is_success(), "MCP {name}: http {status}: {text}");
    let envelope: Value = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
        .next_back()
        .unwrap_or_else(|| panic!("MCP {name}: response is not JSON: {text}"));
    if let Some(err) = envelope.get("error") {
        panic!("MCP {name}: tool error: {err}");
    }
    let content = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("MCP {name}: no result content: {envelope}"));
    serde_json::from_str(content)
        .unwrap_or_else(|e| panic!("MCP {name}: result content is not JSON: {e}: {content}"))
}

async fn rest_post(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    mount: &str,
    sub: &str,
    body: Value,
) -> Value {
    let resp = client
        .post(format!("{base}/api/fs/{mount}/{sub}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("REST POST {sub}: request failed: {e}"));
    let status = resp.status();
    let json: Value = resp.json().await.expect("REST response body must be JSON");
    assert!(status.is_success(), "REST POST {sub}: http {status}: {json}");
    json
}

async fn rest_get(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    mount: &str,
    sub: &str,
    query: &[(&str, &str)],
) -> Value {
    let resp = client
        .get(format!("{base}/api/fs/{mount}/{sub}"))
        .bearer_auth(token)
        .query(query)
        .send()
        .await
        .unwrap_or_else(|e| panic!("REST GET {sub}: request failed: {e}"));
    let status = resp.status();
    let json: Value = resp.json().await.expect("REST response body must be JSON");
    assert!(status.is_success(), "REST GET {sub}: http {status}: {json}");
    json
}

/// One `git` CLI invocation against `dir`, authenticated with the admin bearer
/// token via `http.extraHeader` (the real client-side mechanism this server's
/// `Authorization: Bearer` git gate expects; see `.agent_docs/git.md`).
fn git(dir: &Path, token: &str, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .arg("-c")
        .arg(format!("http.extraHeader=Authorization: Bearer {token}"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"))
}

fn git_ok(dir: &Path, token: &str, args: &[&str]) {
    let out = git(dir, token, args);
    assert!(
        out.status.success(),
        "git {args:?} in {dir:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── the scenario ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a running PostgreSQL and MinIO/S3; see the module docs for setup"]
async fn full_stack_lifecycle() {
    require_git_cli();
    let env = required_env();

    let run_id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let project_id = format!("e2e-{run_id}");
    let schema = format!("e2e_{run_id}");

    // ── keys and config ─────────────────────────────────────────────────────
    let key_dir = tempfile::tempdir().expect("key temp dir");
    let (private_key, public_key) =
        keys::write_keypair(key_dir.path()).expect("keypair must be writable");

    let mut config = ServerConfig::default();
    config.auth.jwt.public_key_path = public_key.display().to_string();
    config.auth.admins = vec![ADMIN.to_string()];

    config.infra.meta.backend = "postgres".into();
    config.infra.meta.dsn = Dsn::new(env.pg_dsn.clone());
    config.infra.meta.schema = schema.clone();
    config.infra.admin.backend = "postgres".into();
    config.infra.admin.dsn = Dsn::new(env.pg_dsn.clone());
    config.infra.admin.schema = schema.clone();
    config.infra.git.backend = "postgres".into();
    config.infra.git.dsn = Dsn::new(env.pg_dsn.clone());
    config.infra.git.schema = schema.clone();

    config.infra.blob.backend = "minio".into();
    config.infra.blob.endpoint = env.minio_endpoint.clone();
    config.infra.blob.access_key = env.minio_access_key.clone();
    config.infra.blob.secret_key = env.minio_secret_key.clone();
    config.infra.blob.bucket_prefix = format!("mcpfs-e2e-{run_id}-");

    config.git.enabled = true;
    let config = Arc::new(config);

    // Registered before the server boots, so a panic between here and the end
    // of the test still tears the schema/bucket down (see the `Teardown` docs).
    let _teardown = Teardown {
        config: config.clone(),
        pg_dsn: env.pg_dsn.clone(),
        schema: schema.clone(),
        project_id: project_id.clone(),
    };

    let router =
        app::build((*config).clone()).await.expect("app::build must succeed against real infra");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move { axum::serve(listener, router).await.expect("server task") });
    let base = format!("http://127.0.0.1:{port}");

    let admin_token = keys::mint_token_from_file(
        &private_key,
        ADMIN,
        keys::DEFAULT_ISSUER,
        keys::DEFAULT_CLAIM,
        3600,
    )
    .expect("admin token must be mintable");

    let http = reqwest::Client::new();

    // ── 1. project + membership (real PostgreSQL, real MinIO bucket) ────────
    let created = call_tool(
        &http,
        &base,
        &admin_token,
        "admin.create_project",
        json!({
            "project_id": project_id, "owner": ADMIN,
        }),
    )
    .await;
    assert_eq!(created["project_id"], project_id);

    // ── 2. write every kind of file the REST plane supports ─────────────────
    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "write",
        json!({
            "path": "/docs/readme.md",
            "content": "# full-stack e2e\n\nHello from the mandatory-postgres-and-minio test.\n",
            "overwrite": true,
        }),
    )
    .await;

    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "write",
        json!({
            "path": "/data/config.json",
            "content": "{\"mode\":\"e2e\",\"count\":3}",
            "overwrite": true,
        }),
    )
    .await;

    let binary: Vec<u8> = (0u8..=255).collect();
    let binary_b64 = base64_encode(&binary);
    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "write-bytes",
        json!({
            "path": "/assets/logo.bin",
            "base64": binary_b64,
            "overwrite": true,
        }),
    )
    .await;

    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "write-docx",
        json!({
            "path": "/docs/report.docx",
            "markdown": "# Report\n\nGenerated during the full-stack e2e run.\n",
            "title": "E2E Report",
            "overwrite": true,
        }),
    )
    .await;

    let form = reqwest::multipart::Form::new().text("directory", "/uploads").part(
        "files",
        reqwest::multipart::Part::bytes("uploaded via multipart\n".as_bytes().to_vec())
            .file_name("notes.txt"),
    );
    let resp = http
        .post(format!("{base}/api/fs/{project_id}/upload"))
        .bearer_auth(&admin_token)
        .multipart(form)
        .send()
        .await
        .expect("upload request must send");
    assert!(resp.status().is_success(), "upload failed: {}", resp.status());

    // ── 3. read every kind back over the wire ────────────────────────────────
    let read_md = rest_get(
        &http,
        &base,
        &admin_token,
        &project_id,
        "read",
        &[("path", "/docs/readme.md"), ("line_numbered", "false")],
    )
    .await;
    assert!(read_md["content"].as_str().unwrap().contains("Hello from the mandatory"));

    let read_bin = rest_get(
        &http,
        &base,
        &admin_token,
        &project_id,
        "read-bytes",
        &[("path", "/assets/logo.bin")],
    )
    .await;
    assert_eq!(base64_decode(read_bin["base64"].as_str().unwrap()), binary);

    let globbed =
        rest_get(&http, &base, &admin_token, &project_id, "glob", &[("pattern", "**/*")]).await;
    let matches: Vec<&str> =
        globbed["matches"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    for expected in [
        "/docs/readme.md",
        "/data/config.json",
        "/assets/logo.bin",
        "/docs/report.docx",
        "/uploads/notes.txt",
    ] {
        assert!(matches.contains(&expected), "glob missing {expected}: {matches:?}");
    }

    let grepped = rest_get(
        &http,
        &base,
        &admin_token,
        &project_id,
        "grep",
        &[("pattern", "full-stack e2e"), ("output_mode", "files")],
    )
    .await;
    let grep_files: Vec<&str> =
        grepped["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(grep_files.contains(&"/docs/readme.md"), "grep missed readme.md: {grep_files:?}");

    // ── 4. git.init + git.commit snapshot the volume as it stands ───────────
    let initialized =
        call_tool(&http, &base, &admin_token, "git.init", json!({"mount_id": project_id})).await;
    assert_eq!(initialized["initialized"], true);

    let first_commit = call_tool(
        &http,
        &base,
        &admin_token,
        "git.commit",
        json!({
            "mount_id": project_id, "message": "initial import",
        }),
    )
    .await;
    assert_eq!(first_commit["message"], "initial import");

    // ── 5. a real `git clone` sees the snapshot ──────────────────────────────
    let work = tempfile::tempdir().expect("work temp dir");
    let clone1 = work.path().join("clone1");
    git_ok(
        work.path(),
        &admin_token,
        &["clone", &format!("{base}/git/{project_id}"), clone1.to_str().unwrap()],
    );
    for rel in [
        "docs/readme.md",
        "data/config.json",
        "assets/logo.bin",
        "docs/report.docx",
        "uploads/notes.txt",
    ] {
        assert!(clone1.join(rel).is_file(), "clone1 missing {rel}");
    }
    assert_eq!(
        std::fs::read(clone1.join("assets/logo.bin")).expect("read cloned binary"),
        binary,
        "the cloned binary file must be byte identical to what write-bytes wrote"
    );

    git_ok(&clone1, &admin_token, &["config", "user.email", "you@example.com"]);
    git_ok(&clone1, &admin_token, &["config", "user.name", "You"]);

    // ── 6. add a file, remove a file, commit and push from the real client ──
    std::fs::write(clone1.join("clone-added.txt"), "added from the clone\n")
        .expect("write new file");
    let mut readme = std::fs::OpenOptions::new()
        .append(true)
        .open(clone1.join("docs/readme.md"))
        .expect("open readme for append");
    writeln!(readme, "Appended by the clone.").expect("append to readme");
    drop(readme);
    git_ok(&clone1, &admin_token, &["rm", "-q", "assets/logo.bin"]);
    git_ok(&clone1, &admin_token, &["add", "-A"]);
    git_ok(&clone1, &admin_token, &["commit", "-q", "-m", "clone edits: add, edit, remove"]);
    git_ok(&clone1, &admin_token, &["push", "-q", "origin", "main"]);

    // ── 7. the server's own git.log agrees ───────────────────────────────────
    let log =
        call_tool(&http, &base, &admin_token, "git.log", json!({"mount_id": project_id})).await;
    let commits = log["commits"].as_array().expect("commits array");
    assert_eq!(commits.len(), 2, "expected 2 commits, got {commits:?}");
    assert_eq!(commits[0]["message"], "clone edits: add, edit, remove");
    assert_eq!(commits[1]["message"], "initial import");

    // ── 8. a second, independent clone sees the pushed state ────────────────
    let clone2 = work.path().join("clone2");
    git_ok(
        work.path(),
        &admin_token,
        &["clone", &format!("{base}/git/{project_id}"), clone2.to_str().unwrap()],
    );
    assert!(clone2.join("clone-added.txt").is_file(), "clone2 missing the file pushed from clone1");
    assert!(
        !clone2.join("assets/logo.bin").exists(),
        "clone2 must not see the file removed in clone1"
    );
    let readme2 =
        std::fs::read_to_string(clone2.join("docs/readme.md")).expect("read readme in clone2");
    assert!(readme2.contains("Appended by the clone."), "clone2's readme missing the append");

    // ── 9. a push updates the git objects, not the live volume ──────────────
    // Deliberate: this project's git model has no working tree, `git.commit`
    // snapshots the volume INTO git, but nothing checks a push back OUT onto
    // the volume (`.agent_docs/git.md`). `fs.read` on the file removed in the
    // clone must therefore still succeed against the untouched volume.
    let still_there = rest_get(
        &http,
        &base,
        &admin_token,
        &project_id,
        "read-bytes",
        &[("path", "/assets/logo.bin")],
    )
    .await;
    assert_eq!(base64_decode(still_there["base64"].as_str().unwrap()), binary);

    // ── 10. the fs -> git direction: edit the volume, snapshot, reclone ──────
    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "delete",
        json!({
            "path": "/data/config.json",
        }),
    )
    .await;
    rest_post(
        &http,
        &base,
        &admin_token,
        &project_id,
        "write",
        json!({
            "path": "/data/summary.txt",
            "content": "written directly through fs.write after the git push round trip\n",
            "overwrite": true,
        }),
    )
    .await;
    let second_commit = call_tool(
        &http,
        &base,
        &admin_token,
        "git.commit",
        json!({
            "mount_id": project_id, "message": "fs edits: delete config.json, add summary.txt",
        }),
    )
    .await;
    assert_eq!(second_commit["message"], "fs edits: delete config.json, add summary.txt");

    let clone3 = work.path().join("clone3");
    git_ok(
        work.path(),
        &admin_token,
        &["clone", &format!("{base}/git/{project_id}"), clone3.to_str().unwrap()],
    );
    assert!(!clone3.join("data/config.json").exists(), "clone3 must not see the fs-deleted file");
    assert!(clone3.join("data/summary.txt").is_file(), "clone3 missing the fs-written file");

    let final_log =
        call_tool(&http, &base, &admin_token, "git.log", json!({"mount_id": project_id})).await;
    assert_eq!(final_log["commits"].as_array().unwrap().len(), 3, "expected 3 commits total");
}

// ── small local base64 helpers (avoid pulling the `base64` crate into the test
// binary's own dependency closure just for two one-line calls; mcp-fs-core
// already re-exposes the exact same encoding via the REST wire format) ───────

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).expect("valid base64")
}
