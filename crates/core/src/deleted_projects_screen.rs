//! `/app/deleted-projects*`: the deleted-projects screen, served on the main
//! server behind identity. Modeled directly on `token_screen.rs`'s
//! auth/CSRF convention (SPEC-0014 FR-NEW-017): there is no identity
//! middleware in this codebase (DRIFT-008), so this handler resolves the
//! caller inline exactly as `token_screen` does, through the same three
//! sources (forwarded header, then `Authorization`, then the read-only
//! `mcpfs_token` cookie verified by [`IdentityResolver::verify`]).
//!
//! Listing and undelete delegate to the exact functions the
//! `admin.list_deleted_projects` and `admin.undelete_project` tool handlers
//! call (`crate::tools::admin::{list_deleted_projects, undelete_project}`),
//! never a second implementation: this screen only renders what the tool
//! already computes and only triggers what the tool already does.
//!
//! CSRF follows `token_screen`'s own mechanism verbatim: `GET
//! /app/deleted-projects` issues a fresh, single-use `csrf_token` bound to
//! the requesting person, held only in this router's own in-memory CSRF
//! store (a separate instance from the token screen's, scoped to this
//! screen alone). `POST /app/deleted-projects/undelete` requires it, but
//! only when the request was authenticated through the ambient
//! `mcpfs_token` cookie; a bearer-authenticated request carries no ambient
//! credential to forge and is exempt.

use crate::errors::{Result, ToolError, code};
use crate::identity::IdentityResolver;
use crate::state::AppState;
use crate::tools::admin;
use crate::tools::registry_support::ToolCtx;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The deleted-projects screen router. Always mounted: unlike the git token
/// screen, this one has no companion subsystem flag to gate it on.
pub fn router(state: Arc<AppState>) -> Router {
    let screen_state = ScreenState { app: state, csrf: Arc::new(CsrfStore::default()) };
    Router::new()
        .route("/app/deleted-projects", get(show))
        .route("/app/deleted-projects/undelete", post(undelete))
        .with_state(screen_state)
}

/// Extractor state for this router: the shared server state plus this
/// screen's own anti-forgery store (never shared with `token_screen`'s).
#[derive(Clone)]
struct ScreenState {
    app: Arc<AppState>,
    csrf: Arc<CsrfStore>,
}

/// In-memory anti-forgery tokens: UUIDv4 string -> the person it was issued
/// to. Never persisted. Removed the moment it is redeemed by the person it
/// was issued to, so it cannot be replayed; a wrong-person or unknown token
/// leaves the map untouched.
#[derive(Default)]
struct CsrfStore(Mutex<HashMap<String, String>>);

impl CsrfStore {
    fn issue(&self, person: &str) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(token.clone(), person.to_string());
        token
    }

    fn consume(&self, token: &str, person: &str) -> bool {
        let mut map = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match map.get(token) {
            Some(p) if p == person => {
                map.remove(token);
                true
            }
            _ => false,
        }
    }
}

/// Which of the three identity sources authenticated a request. Only
/// [`IdentitySource::Cookie`] carries an ambient credential a third party
/// page could trigger, so only it is subject to the `csrf_token` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentitySource {
    Header,
    Cookie,
}

#[derive(Deserialize)]
struct UndeleteForm {
    project_id: String,
    #[serde(default)]
    csrf_token: Option<String>,
}

async fn show(State(state): State<ScreenState>, headers: HeaderMap) -> Response {
    let (person, _source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match list_deleted(&state.app, person.clone()).await {
        Ok(rows) => {
            // Issued only once the page is actually about to render, so a failed
            // render never leaves an unusable token sitting in the store.
            let csrf_token = state.csrf.issue(&person);
            let body = page_shell(&person, &rows, &csrf_token);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
                .into_response()
        }
        Err(e) => error_response(&e),
    }
}

/// One row rendered on the screen.
struct DeletedProjectRow {
    project_id: String,
    owner: String,
    deleted_at: String,
    days_until_permanent_removal: i64,
}

/// The exact enumeration `admin.list_deleted_projects` computes
/// (`admin::list_deleted_projects`): never a second implementation.
async fn list_deleted(state: &Arc<AppState>, person: String) -> Result<Vec<DeletedProjectRow>> {
    let ctx = ToolCtx { person, state: state.clone() };
    let result = admin::list_deleted_projects(&ctx).await?;
    let entries = result["deleted_projects"].as_array().cloned().unwrap_or_default();
    Ok(entries
        .into_iter()
        .map(|e| DeletedProjectRow {
            project_id: e["project_id"].as_str().unwrap_or_default().to_string(),
            owner: e["owner"].as_str().unwrap_or_default().to_string(),
            deleted_at: e["deleted_at"].as_str().unwrap_or_default().to_string(),
            days_until_permanent_removal: e["days_until_permanent_removal"].as_i64().unwrap_or(0),
        })
        .collect())
}

async fn undelete(
    State(state): State<ScreenState>,
    headers: HeaderMap,
    Form(form): Form<UndeleteForm>,
) -> Response {
    let (person, source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    if let Some(resp) = check_csrf(&state.csrf, source, &person, form.csrf_token.as_deref()) {
        return resp;
    }
    match undelete_project(&state.app, person, &form.project_id).await {
        Ok(()) => redirect_to_deleted_projects(),
        Err(e) => error_response(&e),
    }
}

/// A cookie-authenticated request needs a valid, unconsumed `csrf_token`
/// issued to the same person; a header-authenticated request carries no
/// ambient credential and is exempt. `Some` is the rejection response to
/// return as-is; `None` means proceed.
fn check_csrf(
    store: &CsrfStore,
    source: IdentitySource,
    person: &str,
    token: Option<&str>,
) -> Option<Response> {
    if source == IdentitySource::Header {
        return None;
    }
    let matched = token.is_some_and(|t| store.consume(t, person));
    if matched { None } else { Some(forbidden_csrf()) }
}

fn forbidden_csrf() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": code::FORBIDDEN, "detail": "missing or invalid csrf_token"})),
    )
        .into_response()
}

/// Delegates to the exact function `admin.undelete_project` calls; never a
/// second implementation of undelete (FR-NEW-017).
async fn undelete_project(state: &Arc<AppState>, person: String, project_id: &str) -> Result<()> {
    let ctx = ToolCtx { person, state: state.clone() };
    admin::undelete_project(&ctx, project_id).await?;
    Ok(())
}

/// Resolve the caller from the forwarded header, then `Authorization`, then
/// the read-only `mcpfs_token` cookie, also reporting which source
/// authenticated the request so a `POST` handler knows whether an ambient
/// credential was used and `csrf_token` must therefore be checked. The
/// cookie's JWT is handed to the exact same [`IdentityResolver::verify`] a
/// header bearer token goes through: no second verification path.
fn resolve_person(
    identity: &IdentityResolver,
    headers: &HeaderMap,
) -> Result<(String, IdentitySource)> {
    let header_err = match identity
        .resolve(|name| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string))
    {
        Ok(person) => return Ok((person, IdentitySource::Header)),
        Err(e) => e,
    };
    match cookie_value(headers, "mcpfs_token") {
        Some(jwt) => identity.verify(&jwt).map(|p| (p, IdentitySource::Cookie)),
        None => Err(header_err),
    }
}

/// The raw value of one cookie from the `Cookie` header, unparsed otherwise.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.trim().to_string())
    })
}

fn unauthorized(err: &ToolError) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": err.code, "detail": err.message})))
        .into_response()
}

fn error_response(err: &ToolError) -> Response {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({"error": err.code, "detail": err.message}))).into_response()
}

fn redirect_to_deleted_projects() -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/app/deleted-projects")]).into_response()
}

/// The page shell: every soft-deleted project visible to the caller, each
/// with its own undelete form carrying the one shared `csrf_token`.
fn page_shell(person: &str, rows: &[DeletedProjectRow], csrf_token: &str) -> String {
    let person = escape_html(person);
    let csrf_token = escape_html(csrf_token);
    let table_rows: String = rows
        .iter()
        .map(|r| {
            let project_id = escape_html(&r.project_id);
            let owner = escape_html(&r.owner);
            let deleted_at = escape_html(&r.deleted_at);
            let days = r.days_until_permanent_removal;
            format!(
                "<tr data-project-id=\"{project_id}\"><td>{project_id}</td><td>{owner}</td>\
                 <td>{deleted_at}</td><td>{days}</td><td>\
                 <form method=\"post\" action=\"/app/deleted-projects/undelete\">\
                 <input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">\
                 <input type=\"hidden\" name=\"project_id\" value=\"{project_id}\">\
                 <button type=\"submit\">Undelete</button></form></td></tr>\n"
            )
        })
        .collect();
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <title>Deleted projects</title></head>\n<body>\n<h1>Deleted projects</h1>\n\
         <p>Signed in as {person}</p>\n\
         <table>\n<thead><tr><th>Project</th><th>Owner</th><th>Deleted at</th>\
         <th>Days until permanent removal</th><th></th></tr></thead>\n\
         <tbody>\n{table_rows}</tbody>\n</table>\n</body>\n</html>\n"
    )
}

/// Minimal HTML escaping: the identity claim is attacker-controlled by the
/// person it names, never trusted verbatim in the response body.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::keys;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use tower::ServiceExt;

    /// A config wired to a throwaway state root and a fresh RS256 keypair,
    /// with `owner` also declared a platform admin so the test can seed and
    /// tear down its own project through the real `admin.*` tools. Mirrors
    /// `token_screen::tests::test_setup`.
    fn test_setup(root: &Path, owner: &str) -> (ServerConfig, PathBuf) {
        let (key_path, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
        let mut c = ServerConfig::default();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.infra.meta.dir = root.join("volumes").display().to_string();
        c.infra.blob.dir = root.join("blobs").display().to_string();
        c.infra.admin.path = root.join("admin.db").display().to_string();
        c.auth.admins = vec![owner.to_string()];
        (c, key_path)
    }

    fn mint(key_path: &Path, email: &str, ttl_seconds: i64) -> String {
        keys::mint_token_from_file(
            key_path,
            email,
            keys::DEFAULT_ISSUER,
            keys::DEFAULT_CLAIM,
            ttl_seconds,
        )
        .unwrap()
    }

    async fn body_string(r: Response) -> String {
        let b = to_bytes(r.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    fn bearer_get(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn cookie_get(uri: &str, jwt: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("Cookie", format!("mcpfs_token={jwt}"))
            .body(Body::empty())
            .unwrap()
    }

    fn cookie_form_post(uri: &str, jwt: &str, form_body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cookie", format!("mcpfs_token={jwt}"))
            .body(Body::from(form_body.to_string()))
            .unwrap()
    }

    fn bearer_form_post(uri: &str, token: &str, form_body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::from(form_body.to_string()))
            .unwrap()
    }

    /// The `csrf_token` value rendered for `project_id`'s own undelete form.
    fn extract_csrf_token(body: &str) -> String {
        body.split("name=\"csrf_token\" value=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("a rendered csrf_token")
            .to_string()
    }

    /// One JSON-RPC `initialize` + `notifications/initialized` handshake,
    /// mirroring `token_screen::tests::establish_session`.
    async fn establish_session(app: &axum::Router, token: &str) -> String {
        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "deleted-projects-screen-test", "version": "0.0"},
            },
        });
        let r = app.clone().oneshot(mcp_req(token, None, &init)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "initialize must succeed");
        let session_id = r
            .headers()
            .get("Mcp-Session-Id")
            .expect("initialize must return a session id")
            .to_str()
            .unwrap()
            .to_string();
        let notified = app
            .clone()
            .oneshot(mcp_req(
                token,
                Some(&session_id),
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            ))
            .await
            .unwrap();
        assert_eq!(notified.status(), StatusCode::ACCEPTED);
        session_id
    }

    fn mcp_req(token: &str, session: Option<&str>, payload: &Value) -> Request<Body> {
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

    fn last_sse_json(body: &str) -> Value {
        body.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
            .next_back()
            .unwrap_or_else(|| panic!("no JSON data frame in SSE body: {body}"))
    }

    async fn call_tool(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
        let session_id = establish_session(&app, token).await;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        });
        let r = app.oneshot(mcp_req(token, Some(&session_id), &payload)).await.unwrap();
        let body = body_string(r).await;
        let v = last_sse_json(&body);
        assert_ne!(v["result"]["isError"], Value::Bool(true), "tool call failed: {v}");
        let text = v["result"]["content"][0]["text"].as_str().expect("a text content block");
        serde_json::from_str(text).unwrap()
    }

    /// `admin.delete_project` hard-deletes (storage::admin::delete_project); the
    /// soft-delete state this screen lists comes only from the auto-purge
    /// background path (`AdminBackend::soft_delete_project`), so the setup here
    /// creates the project through the real tool and then soft-deletes it the
    /// same way that background path does, directly on the shared store.
    async fn create_and_soft_delete(
        app: axum::Router,
        state: &Arc<AppState>,
        token: &str,
        project_id: &str,
        owner: &str,
    ) {
        call_tool(
            app,
            token,
            "admin.create_project",
            json!({"project_id": project_id, "owner": owner}),
        )
        .await;
        state.admin.soft_delete_project(project_id).await.unwrap();
    }

    async fn deleted_project_ids(app: axum::Router, token: &str) -> Vec<String> {
        let result = call_tool(app, token, "admin.list_deleted_projects", json!({})).await;
        result["deleted_projects"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|e| e["project_id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    // ── E2E-NEW-069 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_069_happy_path_lists_soft_deleted_projects() {
        let owner = "e2e-new-069@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let (app, state) = crate::app::build_with_state(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_and_soft_delete(app.clone(), &state, &token, "proj1", owner).await;

        let r = app.oneshot(bearer_get("/app/deleted-projects", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(body.contains("proj1"), "{body}");
        assert!(body.contains("Undelete"), "{body}");
    }

    // ── E2E-NEW-070 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_070_unauthenticated_access_rejected() {
        let d = tempfile::tempdir().unwrap();
        let (c, _k) = test_setup(d.path(), "owner@test.com");
        let app = crate::app::build(c).await.unwrap();
        let r = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/app/deleted-projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["error"], crate::errors::code::UNAUTHENTICATED);
    }

    // ── E2E-NEW-071 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_071_undelete_action_calls_the_real_tool() {
        let owner = "e2e-new-071@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let (app, state) = crate::app::build_with_state(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_and_soft_delete(app.clone(), &state, &token, "proj1", owner).await;

        let r = app.clone().oneshot(bearer_get("/app/deleted-projects", &token)).await.unwrap();
        let body = body_string(r).await;
        let csrf = extract_csrf_token(&body);

        let r = app
            .clone()
            .oneshot(bearer_form_post(
                "/app/deleted-projects/undelete",
                &token,
                &format!("project_id=proj1&csrf_token={csrf}"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);

        // Cross-checks E2E-NEW-063 (US-0011): the screen produced the exact
        // same effect as the tool, because it called the tool itself.
        let ids = deleted_project_ids(app, &token).await;
        assert!(!ids.contains(&"proj1".to_string()), "proj1 must no longer be soft-deleted");
    }

    // ── E2E-NEW-072 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_072_csrf_rejected() {
        let owner = "e2e-new-072@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let (app, state) = crate::app::build_with_state(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_and_soft_delete(app.clone(), &state, &token, "proj1", owner).await;

        let r = app
            .clone()
            .oneshot(cookie_form_post("/app/deleted-projects/undelete", &token, "project_id=proj1"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["error"], crate::errors::code::FORBIDDEN);

        let ids = deleted_project_ids(app, &token).await;
        assert!(ids.contains(&"proj1".to_string()), "no state change must occur");
    }

    // ── E2E-NEW-073 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_073_empty_state() {
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), "owner@test.com");
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, "e2e-new-073@test.com", 3600);

        let r = app.oneshot(bearer_get("/app/deleted-projects", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(!body.contains("data-project-id"), "{body}");
    }

    // ── identity resolution, mirroring token_screen's own coverage ─────────

    #[tokio::test]
    async fn identity_resolves_from_the_cookie_too() {
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), "owner@test.com");
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, "alice@test.com", 3600);
        let r = app.oneshot(cookie_get("/app/deleted-projects", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "a cookie-only navigation must be enough");
        let body = body_string(r).await;
        assert!(body.contains("alice@test.com"), "{body}");
    }
}
