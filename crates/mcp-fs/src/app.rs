//! Server assembly: shared state, the `/health` probe and the MCP endpoint.
//!
//! Port of the C# `Program.BuildApp`. The C# version leans on the
//! ModelContextProtocol SDK plus an `IdentityMiddleware`; here the MCP endpoint
//! is one axum handler speaking the hand rolled protocol from [`crate::mcp`]
//! (see that module for why the SDK is not used), and identity verification
//! happens inline at the top of that handler, which is the only guarded route.
//!
//! Wire contract reproduced from the running C# server:
//! * missing or invalid bearer: HTTP 401, `application/json`,
//!   `{"error":"ERR_UNAUTHENTICATED","detail":"..."}`
//! * a notification (no `id`): HTTP 202, empty body
//! * everything else: HTTP 200, `text/event-stream`,
//!   `Cache-Control: no-cache,no-store`, body `event: message\ndata: {json}\n\n`

use crate::config::ServerConfig;
use crate::errors::ToolError;
use crate::identity::IdentityResolver;
use crate::logging;
use crate::mcp::registry::ToolCtx;
use crate::mcp::{
    Args, ToolRegistry, initialize_result, rpc_error, rpc_result, sse_frame, tool_err, tool_ok,
};
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::StoreManager;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::sync::Arc;

/// Version reported by `/health` and by `initialize`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Assemble the shared state and the router.
///
/// The admin store is connected here (the C# does it on `ApplicationStarted`) so
/// that a bad database path fails the boot rather than the first request.
pub async fn build(config: ServerConfig) -> anyhow::Result<Router> {
    let config = Arc::new(config);

    let admin = crate::storage::build_admin_store(&config)?;
    admin.connect().await?;

    let stores = Arc::new(StoreManager::new(config.clone()));
    let safety = Arc::new(SafetyManager::new(config.safety.clone()));
    let identity = Arc::new(IdentityResolver::new(&config.auth));

    // The git families are only registered when the subsystem is on, so a server
    // with git disabled advertises exactly the tools it can serve.
    let mut registry = ToolRegistry::new();
    crate::tools::register_all(&mut registry, config.git.enabled);

    let mcp_path = config.server.mcp_path.clone();
    if !mcp_path.starts_with('/') {
        anyhow::bail!("server.mcp_path must start with '/' (got '{mcp_path}')");
    }

    let state = Arc::new(AppState {
        config,
        admin,
        stores,
        safety,
        identity,
        registry: Arc::new(registry),
    });

    let mut router = Router::new()
        .route("/health", get(health))
        .route(&mcp_path, post(mcp_endpoint))
        .with_state(state.clone());

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
        let git_store = crate::git::GitRepoStore::shared(state.config.clone());
        router = router.merge(crate::git::http::router(state.clone(), git_store));
    }

    Ok(router)
}

/// Bind and serve until Ctrl+C.
pub async fn serve(config: ServerConfig) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let app = build(config).await?;
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
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

/// The MCP streamable HTTP endpoint (POST only, like the stateless C# transport).
async fn mcp_endpoint(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    // Identity first: a bad bearer never reaches dispatch, and the 401 is plain
    // JSON (not SSE), matching the C# IdentityMiddleware.
    let person = match resolve_person(&state.identity, &headers) {
        Ok(p) => p,
        Err(e) => {
            logging::log_unauthenticated(&state.config.server.mcp_path, &e);
            return unauthorized(&e);
        }
    };

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            // Parity: the C# transport answers a malformed body with HTTP 500 and a
            // JSON content type, not with a framed JSON-RPC parse error.
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&serde_json::json!({
                    "error": crate::errors::code::INVALID_ARGUMENT,
                    "detail": format!("invalid JSON-RPC request: {e}"),
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            )
                .into_response();
        }
    };

    // No `id` means a notification (`notifications/initialized` and friends):
    // acknowledge with 202 and no body, exactly like the C# transport.
    let id = match payload.get("id") {
        Some(v) if !v.is_null() => v.clone(),
        _ => return StatusCode::ACCEPTED.into_response(),
    };

    let method = payload.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = payload.get("params").cloned().unwrap_or(Value::Null);

    let response = match method {
        "initialize" => rpc_result(id, initialize_result()),
        "tools/list" => rpc_result(id, state.registry.list_payload()),
        "tools/call" => call_tool(&state, person, id, &params).await,
        // Parity: the C# SDK's wording, verified against the running server.
        other => rpc_error(
            id,
            crate::mcp::rpc_error::METHOD_NOT_FOUND,
            format!("Method '{other}' is not available."),
        ),
    };
    sse(&response)
}

/// Dispatch one `tools/call`. A tool failure is a RESULT carrying `isError`, not
/// a JSON-RPC error: only an unknown tool is a protocol level error.
async fn call_tool(state: &Arc<AppState>, person: String, id: Value, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    let args = Args::new(params.get("arguments").cloned().unwrap_or(Value::Null));
    let ctx = ToolCtx { person, state: state.clone() };

    match state.registry.call(&name, ctx, args).await {
        None => rpc_error(
            id,
            crate::mcp::rpc_error::INVALID_PARAMS,
            format!("Unknown tool: '{name}'"),
        ),
        Some(Ok(v)) => rpc_result(id, tool_ok(&v)),
        Some(Err(e)) => {
            logging::log_tool_failure(&name, &e);
            rpc_result(id, tool_err(&name, &e))
        }
    }
}

/// Verify the bearer using the axum header map.
fn resolve_person(resolver: &IdentityResolver, headers: &HeaderMap) -> crate::Result<String> {
    resolver.resolve(|name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    })
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

/// One JSON-RPC message framed as a single SSE event, with the C# headers.
fn sse(payload: &Value) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache,no-store"),
        ],
        sse_frame(payload),
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

    fn rpc(token: Option<&str>, payload: &str) -> Request<axum::body::Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if let Some(t) = token {
            b = b.header("X-Forwarded-Authorization", format!("Bearer {t}"));
        }
        b.body(axum::body::Body::from(payload.to_string())).unwrap()
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
            .oneshot(rpc(None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
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
            .oneshot(rpc(Some("not.a.jwt"), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tools_list_is_sse_framed() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(Some(&t), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache,no-store");
        let body = body_string(r).await;
        assert!(body.starts_with("event: message\ndata: "), "body was {body}");
        assert!(body.ends_with("\n\n"));
        let json = body.trim_start_matches("event: message\ndata: ").trim_end();
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn initialize_returns_the_advertised_protocol() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(Some(&t), r#"{"jsonrpc":"2.0","id":2,"method":"initialize"}"#))
            .await
            .unwrap();
        let body = body_string(r).await;
        let json = body.trim_start_matches("event: message\ndata: ").trim_end();
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["result"]["protocolVersion"], crate::mcp::PROTOCOL_VERSION);
        assert_eq!(v["result"]["serverInfo"]["name"], "mcp-fs");
    }

    #[tokio::test]
    async fn notifications_are_accepted_with_an_empty_body() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(Some(&t), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::ACCEPTED);
        assert_eq!(body_string(r).await, "");
    }

    #[tokio::test]
    async fn unknown_tool_is_invalid_params() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(
                Some(&t),
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fs.nope","arguments":{}}}"#,
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        let json = body.trim_start_matches("event: message\ndata: ").trim_end();
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["error"]["code"], crate::mcp::rpc_error::INVALID_PARAMS);
        assert_eq!(v["error"]["message"], "Unknown tool: 'fs.nope'");
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app
            .oneshot(rpc(Some(&t), r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#))
            .await
            .unwrap();
        let body = body_string(r).await;
        let json = body.trim_start_matches("event: message\ndata: ").trim_end();
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["error"]["code"], crate::mcp::rpc_error::METHOD_NOT_FOUND);
    }

    /// Parity: the C# transport answers a malformed body with HTTP 500 and a JSON
    /// content type, not with a framed JSON-RPC parse error. Verified against the
    /// running reference server.
    #[tokio::test]
    async fn malformed_json_mirrors_the_csharp_500() {
        let d = tempfile::tempdir().unwrap();
        let (c, t) = test_setup(d.path());
        let app = build(c).await.unwrap();
        let r = app.oneshot(rpc(Some(&t), "{not json")).await.unwrap();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            r.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = body_string(r).await;
        let v: Value = serde_json::from_str(&body).expect("a JSON body");
        assert_eq!(v["error"], crate::errors::code::INVALID_ARGUMENT);
        assert!(v["detail"].as_str().unwrap().contains("invalid JSON-RPC request"));
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
            .header("X-Forwarded-Authorization", format!("Bearer {t}"))
            .body(axum::body::Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
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
