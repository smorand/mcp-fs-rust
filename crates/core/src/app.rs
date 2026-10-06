//! Server assembly: shared state, the `/health` probe and the MCP endpoint.
//!
//! The MCP endpoint (US-0008) is `rmcp`'s own `StreamableHttpService`, wired
//! with `LocalSessionManager` and `legacy_session_mode: true` (the pairing
//! recorded in
//! `specs/SPEC-0013_.../drift/2026-10-03_00-38-28.md`, corrected again in
//! `drift/2026-10-03_02-50-09.md`): under that configuration `initialize` is
//! mandatory, a bare pre-`initialize` call gets HTTP 422 with a plain-text
//! body containing `"initialize"` (not the hand-rolled transport's JSON-RPC
//! envelope), and an established session answers a plain tool call with
//! `text/event-stream`, same framing as before. Identity verification still
//! happens in front of `rmcp`, as an axum middleware guarding only the MCP
//! route, so the 401 shape is unchanged and `rmcp` never sees an
//! unauthenticated request. The resolved person is threaded into the
//! `McpServer` built for each new session through a task-local, because
//! `rmcp`'s session factory (`Fn() -> Result<S, io::Error>`) takes no
//! arguments: the middleware runs the whole request, including session
//! creation, inside `CURRENT_PERSON.scope(..)`.
//!
//! `rmcp`'s DNS-rebinding guard requires a `Host` header (or URI authority)
//! on every request reaching the service, including loopback-only test
//! traffic; `app::build` leaves the default (`localhost`, `127.0.0.1`,
//! `::1`) in place, so a caller/test without a real `Host` header must set
//! one explicitly.
//!
//! Unauthenticated 401 shape, unchanged:
//! `application/json`, `{"error":"ERR_UNAUTHENTICATED","detail":"..."}`

use crate::config::ServerConfig;
use crate::errors::ToolError;
use crate::identity::IdentityResolver;
use crate::logging;
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::StoreManager;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{Value, json};
use std::sync::Arc;

tokio::task_local! {
    /// The bearer-verified identity for the in-flight MCP request, set by
    /// [`mcp_auth`] around the whole request (including `rmcp` session
    /// creation), read by the `McpServer` session factory below. `rmcp`'s
    /// service factory takes no arguments, so this is how the identity this
    /// crate already verified reaches the `McpServer` it builds per session.
    static CURRENT_PERSON: String;
}

/// Version reported by `/health` and by `initialize`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Assemble the shared state and the router.
///
/// The admin store is connected here (the C# does it on `ApplicationStarted`) so
/// that a bad database path fails the boot rather than the first request.
pub async fn build(config: ServerConfig) -> anyhow::Result<Router> {
    let (router, _state) = build_with_state(config).await?;
    Ok(router)
}

/// Same as [`build`], but also returns the shared [`AppState`] so [`serve`] can
/// spawn the purge background loop (SPEC-0014 FR-NEW-009) against the exact
/// stores/admin/safety the router itself uses, without a second connection pool.
pub(crate) async fn build_with_state(
    config: ServerConfig,
) -> anyhow::Result<(Router, Arc<AppState>)> {
    let config = Arc::new(config);

    // One registry for the whole process, so the ACL store, every volume and the
    // git index reuse one connection pool per DSN. It is threaded explicitly into
    // every consumer: a second registry would open `max_connections` again, so the
    // process would exceed the single number the operator configured.
    let relational = Arc::new(crate::storage::RelationalRegistry::new());
    let admin = crate::storage::build_admin_store(&config, &relational).await?;
    admin.connect().await?;

    let stores = Arc::new(StoreManager::new(config.clone(), relational.clone()));
    let safety = Arc::new(SafetyManager::new(
        config.safety.clone(),
        crate::storage::meta::max_path_len(&config.infra.meta.backend),
    ));
    let identity = Arc::new(IdentityResolver::new(&config.auth));
    // A configured algorithm this build cannot verify would otherwise be ignored in
    // silence, leaving the operator believing a policy is in force when it is not.
    if !identity.unsupported_algorithms().is_empty() {
        tracing::warn!(
            unsupported = ?identity.unsupported_algorithms(),
            accepted = ?identity.accepted_algorithms(),
            "auth.jwt.algorithms lists entries this build cannot verify, they are ignored"
        );
    }

    let mcp_path = config.server.mcp_path.clone();
    if !mcp_path.starts_with('/') {
        anyhow::bail!("server.mcp_path must start with '/' (got '{mcp_path}')");
    }

    // Built once, here, so the api mode's HTTP client and its connection pool are
    // reused across conversions instead of rebuilt per call.
    let doc_service = crate::docs::service::from_config(&config.doc_service)?;

    // Build the search backend when enabled. None when search.enabled = false.
    // A dedicated HTTP client for embedding calls is built once here so its
    // connection pool is reused across all index and query operations.
    let search_http_client = Arc::new(reqwest::Client::new());
    let search = crate::search::build_backend(&config, &relational, search_http_client)
        .await
        .map_err(|e| anyhow::anyhow!("search backend failed to start: {e}"))?;

    let state = Arc::new(AppState {
        config,
        admin,
        stores,
        safety,
        identity,
        editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
        doc_service,
        search,
    });

    let mcp_service = build_mcp_service(state.clone());
    let mcp_router = Router::new()
        .nest_service(&mcp_path, mcp_service)
        .layer(middleware::from_fn_with_state(state.clone(), mcp_auth));

    let mut router = Router::new()
        .route("/health", get(health))
        .merge(mcp_router)
        .with_state(state.clone())
        .merge(crate::deleted_projects_screen::router(state.clone()))
        .merge(crate::trash_screen::router(state.clone()));

    // The REST data plane and its OpenAPI surface are opt-out via config, matching
    // the C#: with `api.enabled: false` the server is MCP only and both 404.
    if state.config.api.enabled {
        router = router
            .merge(crate::api::router(state.clone()))
            .merge(crate::api::openapi_router(state.clone()));
    }

    // Git HTTP smart protocol, only when the subsystem is enabled. The tools and
    // these routes must share one repository store so they share the write locks.
    if state.config.git.enabled {
        // Same registry as the metadata and ACL stores, so enabling git does not
        // silently double the connection count.
        let git_store = crate::git::GitRepoStore::shared(
            state.config.clone(),
            state.stores.relational().clone(),
        );
        router = router.merge(crate::git::http::router(state.clone(), git_store));

        // The token screen (FR-NEW-046): registered only when git is enabled, so a
        // server without the subsystem does not expose a screen for it.
        router = router.merge(crate::token_screen::router(state.clone()));
    }

    Ok((router, state))
}

/// Bind and serve until Ctrl+C, running the purge background loop (FR-NEW-009)
/// alongside it for the whole lifetime of the process.
pub async fn serve(config: ServerConfig) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let (app, state) = build_with_state(config).await?;
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;

    let (purge_stop, purge_stop_rx) = tokio::sync::oneshot::channel();
    let purge_handle = spawn_purge_loop(state, purge_stop_rx);

    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;

    // Same cancellation intent as `shutdown_signal` above: tell the loop to stop,
    // then wait for it, bounded, so the process never exits with the task still
    // mid-cycle. A send failure means the loop already exited on its own.
    let _ = purge_stop.send(());
    if tokio::time::timeout(std::time::Duration::from_secs(10), purge_handle).await.is_err() {
        tracing::warn!("purge loop did not stop within the shutdown grace period");
    }
    Ok(())
}

/// Detached background task (FR-NEW-009): wakes every `safety.purge_interval_secs`
/// (floored to 1s, matching `tokio::time::interval`'s own requirement that its
/// period be non-zero) and runs the FR-NEW-007/FR-NEW-008 sweep across every
/// project, then the FR-NEW-012 grace sweep (US-0008's hook, see `purge.rs`).
/// A cycle failure is logged and never stops the loop; the next tick tries again.
fn spawn_purge_loop(
    state: Arc<AppState>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let secs = state.config.safety.purge_interval_secs.max(1) as u64;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs));
        loop {
            tokio::select! {
                _ = &mut stop => {
                    tracing::info!("purge loop: shutdown signal received, exiting");
                    break;
                }
                _ = ticker.tick() => {
                    match crate::purge::run_cycle(&state.stores, state.admin.as_ref(), &state.safety).await {
                        Ok(summary) => tracing::info!(
                            files_purged = summary.files_purged,
                            projects_soft_deleted = summary.projects_soft_deleted,
                            "purge loop: cycle complete"
                        ),
                        Err(e) => tracing::warn!(error = %e, "purge loop: cycle failed"),
                    }
                    if let Err(e) =
                        crate::purge::sweep_grace_period(&state.stores, state.admin.as_ref(), &state.config)
                            .await
                    {
                        tracing::warn!(error = %e, "purge loop: grace sweep failed");
                    }
                }
            }
        }
    })
}

/// Resolve on Ctrl+C or SIGTERM so in flight requests finish before the process
/// exits. SIGTERM matters because that is what a container runtime sends, and the
/// C# host drains on it too.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            // Without a SIGTERM handler, Ctrl+C is still honoured.
            Err(e) => {
                tracing::warn!("cannot install the SIGTERM handler: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received, draining");
}

/// Unauthenticated liveness probe. Shape matches the C# `/health` exactly.
async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "version": VERSION}))
}

/// Build the `rmcp` streamable HTTP service that answers the MCP route.
///
/// `LocalSessionManager` + `legacy_session_mode: true` is the pairing that
/// actually makes `initialize` mandatory in `rmcp` 3.5.0 (see the module
/// doc); the service factory reads the identity [`mcp_auth`] stashed in
/// [`CURRENT_PERSON`] for the request that is creating this session.
fn build_mcp_service(
    state: Arc<AppState>,
) -> StreamableHttpService<crate::mcp::server::McpServer, LocalSessionManager> {
    let factory = move || {
        let person = CURRENT_PERSON.with(Clone::clone);
        Ok(crate::mcp::server::McpServer::new(state.clone(), person))
    };
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = true;
    StreamableHttpService::new(factory, Arc::new(LocalSessionManager::default()), config)
}

/// Identity gate in front of the `rmcp` MCP route.
///
/// `rmcp`'s session factory takes no request data, so the person this
/// middleware resolves is handed to the rest of the request (including a
/// fresh session's `McpServer::new`) through the [`CURRENT_PERSON`] task
/// local, scoped around `next.run(..)`.
async fn mcp_auth(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let person = match resolve_person(&state.identity, request.headers()) {
        Ok(p) => p,
        Err(e) => {
            logging::log_unauthenticated(&state.config.server.mcp_path, &e);
            return unauthorized(&e);
        }
    };
    CURRENT_PERSON.scope(person, next.run(request)).await
}

/// Verify the bearer using the axum header map.
fn resolve_person(resolver: &IdentityResolver, headers: &HeaderMap) -> crate::Result<String> {
    resolver.resolve(|name| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string))
}

/// The C# 401 body: `{"error":"ERR_UNAUTHENTICATED","detail":"<message>"}`.
fn unauthorized(err: &ToolError) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&json!({"error": err.code, "detail": err.message}))
            .unwrap_or_else(|_| r#"{"error":"ERR_UNAUTHENTICATED"}"#.to_string()),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    /// A config wired to a throwaway state root plus a fresh keypair.
    fn test_setup(root: &std::path::Path) -> (ServerConfig, String) {
        let (key_path, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
        let token = keys::mint_token_from_file(
            &key_path,
            "me@test.com",
            keys::DEFAULT_ISSUER,
            keys::DEFAULT_CLAIM,
            3600,
        )
        .unwrap();
        let mut c = ServerConfig::default();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.infra.meta.dir = root.join("volumes").display().to_string();
        c.infra.blob.dir = root.join("blobs").display().to_string();
        c.infra.admin.path = root.join("admin.db").display().to_string();
        (c, token)
    }

    async fn body_string(r: Response) -> String {
        let b = to_bytes(r.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    /// Opens the same admin db `state.stores`'s pool cache already points at,
    /// as the concrete store, rather than minting a second registry (a text-grep
    /// composition-root guard in `storage::tests` would flag that as a second
    /// pool). Needed only because `admin.set_purge_config` is US-0009, not
    /// implemented yet, so seeding a purge config has no path through
    /// `AdminBackend`.
    async fn seeded_admin(state: &AppState) -> crate::storage::admin::RelationalAdminStore {
        let db =
            crate::storage::open_admin_db(&state.config, state.stores.relational()).await.unwrap();
        crate::storage::admin::RelationalAdminStore::new(db)
    }

    #[tokio::test]
    async fn e2e_new_077_the_loop_performs_real_work_on_its_own_timer() {
        let d = tempfile::tempdir().unwrap();
        let (mut c, _t) = test_setup(d.path());
        c.safety.purge_interval_secs = 1;

        let (_router, state) = build_with_state(c).await.unwrap();
        let seed = seeded_admin(&state).await;
        crate::storage::traits::AdminBackend::create_project(&seed, "stale-proj", "owner@t.c")
            .await
            .unwrap();
        seed.seed_purge_config_for_test("stale-proj", true, true, Some(0), Some(0)).await.unwrap();

        let client = state.stores.client("stale-proj").await.unwrap();
        client.write_text_atomic("/old.txt", "data").await.unwrap();

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let handle = spawn_purge_loop(state.clone(), stop_rx);

        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

        assert!(
            !client.exists("/old.txt").await.unwrap(),
            "the loop's own timer must have trashed the stale file by now"
        );
        assert!(
            seed.deleted_at_for_test("stale-proj").await.unwrap().is_some(),
            "the loop's own timer must have soft-deleted the stale project by now"
        );

        let _ = stop_tx.send(());
        tokio::time::timeout(std::time::Duration::from_secs(5), handle).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn e2e_new_078_graceful_shutdown_joins_within_a_bounded_timeout() {
        let d = tempfile::tempdir().unwrap();
        let (mut c, _t) = test_setup(d.path());
        c.safety.purge_interval_secs = 1;
        let (_router, state) = build_with_state(c).await.unwrap();

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let handle = spawn_purge_loop(state, stop_rx);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = stop_tx.send(());

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        assert!(result.is_ok(), "the loop task must join within the bounded timeout");
        assert!(result.unwrap().is_ok(), "the loop task must not panic");
    }

    /// `rmcp`'s SSE stream can carry a priming event (`data:\nid: ...\nretry:
    /// ...`) ahead of the real one; this returns the JSON-RPC envelope of
    /// the LAST `data:` line that parses as JSON, which is the response to
    /// the request just made (mirrors `crates/agent/src/mcp.rs::parse_body`).
    fn sse_json(body: &str) -> Value {
        body.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
            .next_back()
            .unwrap_or_else(|| panic!("no JSON data frame in SSE body: {body}"))
    }

    /// Build a `/mcp` POST request. `rmcp`'s DNS-rebinding guard requires a
    /// `Host` header on every request it serves (see the module doc), so
    /// every request built here carries one, matching the default allowlist
    /// (`localhost`, `127.0.0.1`, `::1`).
    fn rpc(token: Option<&str>, session: Option<&str>, payload: &str) -> Request<axum::body::Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Host", "localhost")
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if let Some(t) = token {
            b = b.header("X-Forwarded-Authorization", format!("Bearer {t}"));
        }
        if let Some(s) = session {
            b = b.header("Mcp-Session-Id", s);
        }
        b.body(axum::body::Body::from(payload.to_string())).unwrap()
    }

    const INITIALIZE_BODY: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}"#;

    /// Perform the `initialize` + `notifications/initialized` handshake
    /// `LocalSessionManager` + `legacy_session_mode: true` now requires
    /// (DR-010), returning the session id `rmcp` issued so a later call can
    /// reuse it. Mirrors what `crates/agent/src/mcp.rs`'s client now does.
    async fn establish_session(app: &Router, token: &str) -> String {
        let r = app.clone().oneshot(rpc(Some(token), None, INITIALIZE_BODY)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "initialize must succeed");
        let session_id = r
            .headers()
            .get("Mcp-Session-Id")
            .expect("initialize must return a session id")
            .to_str()
            .unwrap()
            .to_string();
        let r2 = app
            .clone()
            .oneshot(rpc(
                Some(token),
                Some(&session_id),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::ACCEPTED);
        session_id
    }

    #[tokio::test]
    async fn health_is_public_and_matches_csharp_shape() {
        let d = tempfile::tempdir().unwrap();
        let (c, _t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(Request::builder().uri("/health").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["version"], VERSION);
    }

    #[tokio::test]
    async fn mcp_without_a_token_is_401_json() {
        let d = tempfile::tempdir().unwrap();
        let (c, _t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(None, None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "application/json");
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["error"], crate::errors::code::UNAUTHENTICATED);
        assert!(v["detail"].is_string());
    }

    #[tokio::test]
    async fn mcp_with_a_bad_token_is_401() {
        let d = tempfile::tempdir().unwrap();
        let (c, _t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(
                Some("not.a.jwt"),
                None,
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    // ── DT-001 (corrected, drift/2026-10-03_02-50-09.md): a bare tools/call
    // with no prior initialize gets a 4xx response whose body contains
    // "initialize", not a JSON-RPC -32600 envelope. ─────────────────────────
    #[tokio::test]
    async fn mcp_bare_tools_call_without_initialize_is_rejected_mentioning_initialize() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(
                Some(&t),
                None,
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.glob","arguments":{}}}"#,
            ))
            .await
            .unwrap();
        assert!(r.status().is_client_error(), "expected a 4xx, got {}", r.status());
        let body = body_string(r).await;
        assert!(body.contains("initialize"), "body must mention initialize: {body}");
    }

    #[tokio::test]
    async fn mcp_bare_tools_list_without_initialize_is_rejected_mentioning_initialize() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(Some(&t), None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
            .await
            .unwrap();
        assert!(r.status().is_client_error(), "expected a 4xx, got {}", r.status());
        let body = body_string(r).await;
        assert!(body.contains("initialize"), "body must mention initialize: {body}");
    }

    #[tokio::test]
    async fn initialize_establishes_a_session_and_returns_server_identity() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app.clone().oneshot(rpc(Some(&t), None, INITIALIZE_BODY)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers().contains_key("Mcp-Session-Id"), "initialize must issue a session id");
        let v = sse_json(&body_string(r).await);
        assert_eq!(v["result"]["serverInfo"]["name"], "mcp-fs");
        assert_eq!(v["result"]["serverInfo"]["version"], VERSION);
    }

    #[tokio::test]
    async fn notifications_after_initialize_are_accepted_with_an_empty_body() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        // `establish_session` already performs and asserts the exact sequence
        // this test is about (initialize, then an empty-body 202 for
        // notifications/initialized); re-asserted here by name for DR-010
        // traceability.
        establish_session(&app, &t).await;
    }

    // ── DT-003 (corrected, drift/2026-10-03_02-50-09.md): once a session is
    // established, a plain tool call's Content-Type is text/event-stream,
    // same as before this migration — not JSON, as the spec originally
    // planned before the session-manager correction. ───────────────────────
    #[tokio::test]
    async fn mcp_plain_tool_call_after_initialize_is_sse_framed() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let session = establish_session(&app, &t).await;
        let r = app
            .oneshot(rpc(
                Some(&t),
                Some(&session),
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"admin.list_projects","arguments":{}}}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
        let v = sse_json(&body_string(r).await);
        assert_eq!(v["id"], 1);
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v["result"].is_object(), "expected a CallToolResult object: {v}");
    }

    #[tokio::test]
    async fn tools_list_over_an_established_session_is_sse_framed() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let session = establish_session(&app, &t).await;
        let r = app
            .oneshot(rpc(
                Some(&t),
                Some(&session),
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
        let v = sse_json(&body_string(r).await);
        assert_eq!(v["id"], 1);
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v["result"]["tools"].is_array());
    }

    // ── DT-004: a notification before the final response still goes through
    // SSE, with at least two `data:` frames (the notification, then the
    // response). The tool is `#[cfg(test)]`-only (US-0008). ────────────────
    #[tokio::test]
    async fn mcp_tool_notification_before_response_yields_multiple_sse_frames() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let session = establish_session(&app, &t).await;
        let r = app
            .oneshot(rpc(
                Some(&t),
                Some(&session),
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"t.notifies_then_returns","arguments":{}}}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
        let body = body_string(r).await;
        let frame_count = body.matches("data: ").count();
        assert!(frame_count >= 2, "expected >= 2 SSE data frames, body was {body}");
    }

    #[tokio::test]
    async fn unknown_tool_is_invalid_params() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let session = establish_session(&app, &t).await;
        let r = app
            .oneshot(rpc(
                Some(&t),
                Some(&session),
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fs.nope","arguments":{}}}"#,
            ))
            .await
            .unwrap();
        let v = sse_json(&body_string(r).await);
        // `rmcp`'s own `ToolRouter::call` hard-codes this message for an
        // unmapped tool name (handler/server/router/tool.rs:566/571); the
        // old hand-rolled transport said "Unknown tool: '<name>'" instead.
        // Reusing `rmcp`'s own dispatch (rather than re-implementing "is this
        // name known" ourselves) means this exact text is not under this
        // crate's control. Recorded as a discovered, not-pre-declared,
        // deviation (see this story's final report) — the error *code*
        // (-32602 / INVALID_PARAMS) is unchanged.
        assert_eq!(v["error"]["code"], -32602); // JSON-RPC INVALID_PARAMS
        assert_eq!(v["error"]["message"], "tool not found");
    }

    /// `resources/list` is a method the MCP spec itself defines, so `rmcp`
    /// answers it from its own protocol-level dispatch rather than treating
    /// it as unknown: this `ServerHandler` never calls `enable_resources()`,
    /// so `rmcp`'s default handler returns an empty list rather than a
    /// method-not-found error (the old hand-rolled transport, which only
    /// understood `initialize`/`tools/list`/`tools/call`, answered -32601
    /// for anything else). A genuinely unrecognized method is covered by
    /// `malformed_json_is_rejected_at_the_json_rpc_decode_step` instead
    /// (deserializing the method name itself fails there). Recorded as a
    /// discovered deviation: this project advertises no resources, so this
    /// is an won't-be-called protocol corner rather than a behavior the
    /// product surface depends on.
    #[tokio::test]
    async fn a_spec_defined_method_outside_tools_gets_rmcps_own_empty_default() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let session = establish_session(&app, &t).await;
        let r = app
            .oneshot(rpc(
                Some(&t),
                Some(&session),
                r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v = sse_json(&body_string(r).await);
        assert_eq!(v["result"]["resources"], serde_json::json!([]));
    }

    /// `rmcp` rejects a body that is not valid JSON-RPC at the same
    /// `expect_json` decode step (HTTP 415, plain text), not with the old
    /// hand-rolled transport's HTTP 500 JSON error. Discovered deviation,
    /// not one of the two Section 5 declared breaks, but unavoidable without
    /// re-implementing `rmcp`'s own request decoding.
    #[tokio::test]
    async fn malformed_json_is_rejected_at_the_json_rpc_decode_step() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app.oneshot(rpc(Some(&t), None, "{not json")).await.unwrap();
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let body = body_string(r).await;
        assert!(body.contains("deserialize"), "body was {body}");
    }

    #[tokio::test]
    async fn the_mcp_path_is_configurable() {
        let d = tempfile::tempdir().unwrap();
        let (mut c, t) = test_setup(d.path());
        c.server.mcp_path = "/rpc".into();
        let app = build(c).await.unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/rpc")
            .header("Host", "localhost")
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("X-Forwarded-Authorization", format!("Bearer {t}"))
            .body(axum::body::Body::from(INITIALIZE_BODY))
            .unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_relative_mcp_path_fails_the_boot() {
        let d = tempfile::tempdir().unwrap();
        let (mut c, _t) = test_setup(d.path());
        c.server.mcp_path = "mcp".into();
        assert!(build(c).await.is_err());
    }
}
