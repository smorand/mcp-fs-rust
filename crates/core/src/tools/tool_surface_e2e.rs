//! SPEC-0027 US-0030 and US-0031: the MCP tool surface a server serves follows
//! its `git.enabled` and `search.enabled` flags, and the five optional
//! families (`web`, `context7`, `sqlite`, `db`, `doc`) follow theirs. Driven over the real HTTP MCP
//! endpoint (initialize, `tools/list`, `tools/call`), the same way
//! `crate::app`'s own tests drive it, against an app built from the stated
//! configuration (DEC-005).
//!
//! BT-005 and BT-006 are exploit tests (CWE-284): each sends the literal
//! JSON-RPC a member would send, then reads the absence of effect on a second
//! channel (the repository directory and git index, a recording server's
//! connection count, the volume contents) so a refusal that still ran the tool
//! cannot pass.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::config::{GitConfig, ServerConfig};
use crate::keys;
use crate::state::AppState;

const MIA: &str = "mia@test.com";
const PROJECT: &str = "atlas";

/// Families a default configuration must not serve (BT-007).
const OFF_BY_DEFAULT: [&str; 7] =
    ["git.", "search.", "web.", "context7.", "sqlite.", "db.", "doc."];

/// A config wired to a throwaway state root and a fresh RS256 keypair, plus a
/// token for Mia. Every flag stays at its default.
fn setup(root: &Path) -> (ServerConfig, String) {
    let (key_path, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
    let token =
        keys::mint_token_from_file(&key_path, MIA, keys::DEFAULT_ISSUER, keys::DEFAULT_CLAIM, 3600)
            .unwrap();
    let mut c = ServerConfig::default();
    c.auth.jwt.public_key_path = pub_path.display().to_string();
    c.infra.meta.dir = root.join("volumes").display().to_string();
    c.infra.blob.dir = root.join("blobs").display().to_string();
    c.infra.admin.path = root.join("admin.db").display().to_string();
    c.search.tantivy_dir = root.join("tantivy").display().to_string();
    (c, token)
}

/// Build the app and create `atlas` with Mia as its owner, so every refusal
/// below is the served surface's doing, never a membership check.
async fn app_with_atlas(c: ServerConfig) -> (Router, Arc<AppState>) {
    let (router, state) = crate::app::build_with_state(c).await.unwrap();
    state.admin.create_project(PROJECT, MIA).await.unwrap();
    (router, state)
}

fn rpc(token: &str, session: Option<&str>, payload: &Value) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Host", "localhost")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json");
    if let Some(s) = session {
        b = b.header("Mcp-Session-Id", s);
    }
    b.body(Body::from(payload.to_string())).unwrap()
}

async fn body_string(r: axum::response::Response) -> String {
    let b = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(b.to_vec()).unwrap()
}

/// The JSON-RPC envelope of the last `data:` frame that parses as JSON.
fn sse_json(body: &str) -> Value {
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
        .next_back()
        .unwrap_or_else(|| panic!("no JSON data frame in SSE body: {body}"))
}

async fn establish_session(app: &Router, token: &str) -> String {
    let init = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "tool-surface-test", "version": "0.0"},
        },
    });
    let r = app.clone().oneshot(rpc(token, None, &init)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK, "initialize must succeed");
    let session = r.headers()["Mcp-Session-Id"].to_str().unwrap().to_string();
    let notified = app
        .clone()
        .oneshot(rpc(
            token,
            Some(&session),
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        ))
        .await
        .unwrap();
    assert_eq!(notified.status(), StatusCode::ACCEPTED);
    session
}

/// One raw JSON-RPC message over a fresh session, the decoded envelope back.
async fn send(app: &Router, token: &str, payload: Value) -> Value {
    let session = establish_session(app, token).await;
    let r = app.clone().oneshot(rpc(token, Some(&session), &payload)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    sse_json(&body_string(r).await)
}

async fn list_tools(app: &Router, token: &str) -> Vec<Value> {
    let v = send(app, token, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
    v["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list must return a tools array: {v}"))
        .clone()
}

async fn list_names(app: &Router, token: &str) -> Vec<String> {
    list_tools(app, token).await.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

fn assert_tool_not_found(v: &Value) {
    assert_eq!(v["error"]["code"], -32602, "expected JSON-RPC INVALID_PARAMS, got {v}");
    assert_eq!(v["error"]["message"], "tool not found", "got {v}");
}

/// The git index rows `atlas` owns. On the default SQLite backend the index
/// is the one file `config.git_db_path`, so its absence is zero rows.
fn git_index_exists(state: &AppState) -> bool {
    state.config.git_db_path(PROJECT).exists()
}

// ── SPEC-0027/BT-005 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_005_a_disabled_git_tool_cannot_create_a_repository() {
    let d = tempfile::tempdir().unwrap();
    let (c, token) = setup(d.path());
    assert!(!c.git.enabled, "BT-005 runs on the default configuration");
    let (app, state) = app_with_atlas(c).await;

    let payload: Value = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"git.init","arguments":{"mount_id":"atlas"}}}"#,
    )
    .unwrap();
    let v = send(&app, &token, payload).await;

    assert_tool_not_found(&v);
    assert_eq!(v["id"], 7);
    let repo = state.config.git_repo_dir(PROJECT);
    assert!(!repo.exists(), "git.init must not have created {}", repo.display());
    assert!(!git_index_exists(&state), "no git index row may exist for atlas");
}

// ── SPEC-0027/BT-006 ────────────────────────────────────────────────────────

/// A local endpoint that counts every connection it accepts and closes it at
/// once. The remote pipeline only accepts `https` (FR-NEW-041), so a plain
/// HTTP bare repository would be rejected before any socket opens and could
/// never show a dispatched clone; counting accepted TCP connections on the
/// `https` URL does, since `git2` must connect before its TLS handshake fails.
async fn spawn_recorder() -> (u16, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(sock);
        }
    });
    (port, seen)
}

#[test]
fn bt_006_a_disabled_git_tool_cannot_reach_a_remote() {
    // The git host map is process wide: serialize with every other test that
    // publishes it, and declare the recorder's host so nothing but the served
    // surface can stop the clone before it connects.
    let _guard = crate::git::remote::tests::lock_for_test();
    let rt =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let mut hosts = GitConfig::default();
        hosts.hosts.0 = vec![("127.0.0.1".to_string(), "anonymous".to_string())];
        crate::git::remote::validate_hosts(&hosts).unwrap();

        let d = tempfile::tempdir().unwrap();
        let (c, token) = setup(d.path());
        assert!(!c.git.enabled, "BT-006 runs on the default configuration");
        let (app, state) = app_with_atlas(c).await;
        let (port, seen) = spawn_recorder().await;
        let url = format!("https://127.0.0.1:{port}/r.git");

        let v = send(
            &app,
            &token,
            json!({
                "jsonrpc": "2.0", "id": 8, "method": "tools/call",
                "params": {"name": "git.remote_clone", "arguments": {"mount_id": PROJECT, "url": url}},
            }),
        )
        .await;

        assert_tool_not_found(&v);
        assert_eq!(seen.load(Ordering::SeqCst), 0, "the remote must never be contacted");
        let vol = state.stores.client(PROJECT).await.unwrap();
        let files: Vec<String> =
            vol.walk("/").await.unwrap().into_iter().flat_map(|(_, _, f)| f).collect();
        assert!(files.is_empty(), "atlas must hold no file, found {files:?}");
    });
}

// ── SPEC-0027/BT-007 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_007_disabled_tools_are_not_listed() {
    let d = tempfile::tempdir().unwrap();
    let (c, token) = setup(d.path());
    let (app, _state) = app_with_atlas(c).await;

    let names = list_names(&app, &token).await;

    let leaked: Vec<&String> =
        names.iter().filter(|n| OFF_BY_DEFAULT.iter().any(|p| n.starts_with(p))).collect();
    assert!(leaked.is_empty(), "the default configuration must not list {leaked:?}");
    // The `t.*` tools are this crate's own `#[cfg(test)]` router (US-0008),
    // absent from any real build, so they are not part of the served count.
    let served = names.iter().filter(|n| !n.starts_with("t.")).count();
    assert_eq!(served, 60, "default surface is 45 fs.* plus 15 admin.*, got {names:?}");
}

// ── SPEC-0027/BT-020 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_020_enabled_git_and_search_are_served_unchanged() {
    let d = tempfile::tempdir().unwrap();
    let (mut c, token) = setup(d.path());
    c.git.enabled = true;
    c.search.enabled = true;
    let (app, _state) = app_with_atlas(c).await;

    let names = list_names(&app, &token).await;
    let served = names.iter().filter(|n| !n.starts_with("t.")).count();
    assert_eq!(served, 113, "git and search on serve 113 tools, got {names:?}");

    for name in ["git.init", "git.status"] {
        let v = send(
            &app,
            &token,
            json!({
                "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": {"name": name, "arguments": {"mount_id": PROJECT}},
            }),
        )
        .await;
        assert!(v["error"].is_null(), "{name} must be dispatched: {v}");
        assert_ne!(v["result"]["isError"], Value::Bool(true), "{name} must succeed: {v}");
    }
}

// ── SPEC-0027/BT-021 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_021_git_http_and_token_screen_stay_unrouted_by_default() {
    let d = tempfile::tempdir().unwrap();
    let (c, _token) = setup(d.path());
    let (app, _state) = app_with_atlas(c).await;

    for uri in ["/git/atlas/info/refs?service=git-upload-pack", "/app/tokens"] {
        let r = app
            .clone()
            .oneshot(Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{uri} must not be routed");
    }
}

// ── SPEC-0027 US-0031: optional families ────────────────────────────────────

/// One optional family: the prefix its tools carry, the flag that turns it on,
/// and the catalog `/api/swagger.json` already reads for it.
struct Family {
    prefix: &'static str,
    enable: fn(&mut ServerConfig),
    catalog: fn(&ServerConfig) -> Vec<crate::tools::catalog::ToolCatalogEntry>,
}

const FAMILIES: [Family; 5] = [
    Family {
        prefix: "web.",
        enable: |c| c.web.enabled = true,
        catalog: |c| crate::tools::web::catalog(&c.web),
    },
    Family {
        prefix: "context7.",
        enable: |c| c.context7.enabled = true,
        catalog: |c| crate::tools::context7::catalog(&c.context7),
    },
    Family {
        prefix: "sqlite.",
        enable: |c| c.sqlite.enabled = true,
        catalog: |c| crate::tools::sqlite::catalog(&c.sqlite),
    },
    Family {
        prefix: "db.",
        enable: |c| c.db.enabled = true,
        catalog: |c| crate::tools::db::catalog(&c.db),
    },
    Family {
        prefix: "doc.",
        enable: |c| c.doc.enabled = true,
        catalog: |c| crate::tools::doc::catalog(&c.doc),
    },
];

/// `doc.*` registers nothing without pandoc (`tools/doc.rs`), so its expected
/// count follows the machine the test runs on.
fn pandoc_on_path() -> bool {
    which::which("pandoc").is_ok()
}

fn expected_family_count(prefix: &str) -> usize {
    match prefix {
        "web." => 5,
        "context7." => 2,
        "sqlite." => 8,
        "db." => 5,
        "doc." if pandoc_on_path() => 2,
        "doc." => 0,
        other => panic!("unknown family {other}"),
    }
}

fn served_count(tools: &[Value]) -> usize {
    tools.iter().filter(|t| !t["name"].as_str().unwrap().starts_with("t.")).count()
}

async fn call_tool(app: &Router, token: &str, name: &str, arguments: Value) -> Value {
    send(
        app,
        token,
        json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }),
    )
    .await
}

/// The JSON a successful family call returned, decoded from its text content.
fn success_payload(v: &Value) -> Value {
    assert!(v["error"].is_null(), "call must be dispatched: {v}");
    assert_ne!(v["result"]["isError"], Value::Bool(true), "call must succeed: {v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{v}"));
    serde_json::from_str(text).unwrap()
}

// ── SPEC-0027/BT-001 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_001_an_enabled_sqlite_family_is_callable() {
    let d = tempfile::tempdir().unwrap();
    let (mut c, token) = setup(d.path());
    c.sqlite.enabled = true;
    let (app, _state) = app_with_atlas(c).await;

    let names = list_names(&app, &token).await;
    let sqlite: Vec<&String> = names.iter().filter(|n| n.starts_with("sqlite.")).collect();
    assert_eq!(sqlite.len(), 8, "the 8 sqlite.* tools must be listed, got {names:?}");

    // The family's own schema names the file `db_path` (tools/sqlite.rs).
    let created = call_tool(
        &app,
        &token,
        "sqlite.execute",
        json!({"mount_id": PROJECT, "db_path": "/db.sqlite", "sql": "CREATE TABLE t(a)"}),
    )
    .await;
    success_payload(&created);
    let listed = call_tool(
        &app,
        &token,
        "sqlite.list_tables",
        json!({"mount_id": PROJECT, "db_path": "/db.sqlite"}),
    )
    .await;
    let rows = success_payload(&listed);
    assert!(
        rows.as_array().unwrap().iter().any(|r| r["name"] == "t"),
        "the listing must contain table t, got {rows}"
    );
}

/// Served over MCP, the family handler still authorizes `mount_id` itself: a
/// verified identity outside `atlas` gets the membership error and no file.
#[tokio::test]
async fn a_served_family_tool_still_requires_membership() {
    let d = tempfile::tempdir().unwrap();
    let (mut c, _token) = setup(d.path());
    c.sqlite.enabled = true;
    let key = d.path().join("keys").join(keys::PRIVATE_KEY_FILE);
    let eve = keys::mint_token_from_file(
        &key,
        "eve@test.com",
        keys::DEFAULT_ISSUER,
        keys::DEFAULT_CLAIM,
        3600,
    )
    .unwrap();
    let (app, state) = app_with_atlas(c).await;

    let v = call_tool(
        &app,
        &eve,
        "sqlite.execute",
        json!({"mount_id": PROJECT, "db_path": "/db.sqlite", "sql": "CREATE TABLE t(a)"}),
    )
    .await;

    assert!(v["error"].is_null(), "the tool is served, so it is dispatched: {v}");
    assert_eq!(v["result"]["isError"], Value::Bool(true), "a non member must be refused: {v}");
    let vol = state.stores.client(PROJECT).await.unwrap();
    let files: Vec<String> =
        vol.walk("/").await.unwrap().into_iter().flat_map(|(_, _, f)| f).collect();
    assert!(files.is_empty(), "atlas must hold no file, found {files:?}");
}

// ── SPEC-0027/BT-010 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_010_each_optional_family_is_listed_with_its_catalog_schema() {
    for family in &FAMILIES {
        let d = tempfile::tempdir().unwrap();
        let (mut c, token) = setup(d.path());
        (family.enable)(&mut c);
        let catalog = (family.catalog)(&c);
        let (app, _state) = app_with_atlas(c).await;

        let tools = list_tools(&app, &token).await;
        let expected = expected_family_count(family.prefix);
        assert_eq!(catalog.len(), expected, "{} catalog size", family.prefix);
        assert_eq!(served_count(&tools), 60 + expected, "{} alone", family.prefix);
        for entry in &catalog {
            let served = tools
                .iter()
                .find(|t| t["name"] == entry.name.as_str())
                .unwrap_or_else(|| panic!("{} must be listed", entry.name));
            assert_eq!(served["description"], entry.description.as_str(), "{}", entry.name);
            assert_eq!(served["inputSchema"], entry.input_schema, "{}", entry.name);
        }
    }
}

// ── SPEC-0027/BT-011 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_011_every_flag_on_serves_the_whole_surface() {
    let d = tempfile::tempdir().unwrap();
    let (mut c, token) = setup(d.path());
    c.git.enabled = true;
    c.search.enabled = true;
    for family in &FAMILIES {
        (family.enable)(&mut c);
    }
    let (app, _state) = app_with_atlas(c).await;

    let tools = list_tools(&app, &token).await;
    let expected = if pandoc_on_path() { 135 } else { 133 };
    assert_eq!(served_count(&tools), expected, "every flag on (pandoc: {})", pandoc_on_path());
}

// ── SPEC-0027/BT-012 ────────────────────────────────────────────────────────

#[tokio::test]
async fn bt_012_a_disabled_family_call_is_unknown() {
    let d = tempfile::tempdir().unwrap();
    let (c, token) = setup(d.path());
    let (app, _state) = app_with_atlas(c).await;

    let web = call_tool(&app, &token, "web.search", json!({"query": "x"})).await;
    assert_tool_not_found(&web);
    let search =
        call_tool(&app, &token, "search.query", json!({"mount_id": PROJECT, "query": "x"})).await;
    assert_tool_not_found(&search);
}

// ── SPEC-0027/BT-014 ────────────────────────────────────────────────────────

const STUB_BODY: &str = "stub page body";

/// A local plain HTTP server answering `/page` with a fixed text body. The web
/// family accepts any `http://` or `https://` URL with no host allow list
/// (tools/web.rs `web_fetch`), so loopback over plain HTTP is a URL it takes.
async fn spawn_stub() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new().route("/page", axum::routing::get(|| async { STUB_BODY }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

#[tokio::test]
async fn bt_014_web_fetch_is_callable_through_mcp() {
    let d = tempfile::tempdir().unwrap();
    let (mut c, token) = setup(d.path());
    c.web.enabled = true;
    let (app, _state) = app_with_atlas(c).await;
    let port = spawn_stub().await;

    let v = call_tool(
        &app,
        &token,
        "web.fetch",
        json!({"url": format!("http://127.0.0.1:{port}/page")}),
    )
    .await;
    assert_eq!(success_payload(&v), Value::String(STUB_BODY.to_string()));
}

/// SPEC-0027 DEC-003 guard: `ToolRouter::add_route` replaces a route of the
/// same name, so no optional family tool may share a contract tool's name.
#[test]
fn no_optional_family_tool_shadows_a_contract_tool() {
    use crate::tools::registry_support::ToolRegistry;
    let config = crate::config::ServerConfig::default();
    let mut registry = ToolRegistry::new();
    crate::tools::web::register(&mut registry, &config.web);
    crate::tools::context7::register(&mut registry, &config.context7);
    crate::tools::sqlite::register(&mut registry, &config.sqlite);
    crate::tools::db::register(&mut registry, &config.db);
    crate::tools::doc::register(&mut registry, &config.doc);
    let router = crate::mcp::server::McpServer::tool_router();
    let clashes: Vec<&str> =
        registry.names().iter().copied().filter(|n| router.has_route(n)).collect();
    assert!(clashes.is_empty(), "optional tools shadowing contract tools: {clashes:?}");
    assert!(registry.names().len() >= 20, "the families registered: {:?}", registry.names());
}
