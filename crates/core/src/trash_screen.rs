//! `/app/trash*`: the trash listing/recovery screen, served on the main
//! server behind identity. Modeled directly on `deleted_projects_screen.rs`'s
//! identity resolution and CSRF convention (SPEC-0011 FR-NEW-012/018): there
//! is no identity middleware in this codebase (DRIFT-008), so this handler
//! resolves the caller inline through the same three sources (forwarded
//! header, then `Authorization`, then the read-only `mcpfs_token` cookie
//! verified by [`IdentityResolver::verify`]).
//!
//! Listing and restoring delegate to the exact functions the
//! `fs.trash_list`/`fs.trash_restore` tool handlers call
//! (`crate::tools::trash::{trash_list, trash_restore}`), never a second
//! implementation: this screen only renders what the tool already computes
//! and only triggers what the tool already does. Authorization uses the same
//! membership gate the MCP tools use (`AppState::authorize`).
//!
//! When `mount_id` is absent, the screen renders a project picker built from
//! `AdminBackend::list_projects_for(person)`, the viewer's own memberships.
//!
//! CSRF follows `deleted_projects_screen`'s own mechanism verbatim: `GET
//! /app/trash` issues a fresh, single-use `csrf_token` bound to the
//! requesting person, held only in this router's own in-memory CSRF store (a
//! separate instance from every other screen's). `POST /app/trash/restore`
//! requires it, but only when the request was authenticated through the
//! ambient `mcpfs_token` cookie; a bearer-authenticated request carries no
//! ambient credential to forge and is exempt.

use crate::errors::{Result, ToolError, code};
use crate::identity::IdentityResolver;
use crate::state::AppState;
use crate::tools::trash;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const TRASH_LIST_LIMIT: i64 = 500;

/// The trash screen router. Always mounted, like `deleted_projects_screen`'s.
pub fn router(state: Arc<AppState>) -> Router {
    let screen_state = ScreenState { app: state, csrf: Arc::new(CsrfStore::default()) };
    Router::new()
        .route("/app/trash", get(show))
        .route("/app/trash/restore", post(restore))
        .with_state(screen_state)
}

/// Extractor state for this router: the shared server state plus this
/// screen's own anti-forgery store (never shared with another screen's).
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
struct ShowQuery {
    mount_id: Option<String>,
}

#[derive(Deserialize)]
struct RestoreForm {
    mount_id: String,
    trash_path: String,
    #[serde(default)]
    csrf_token: Option<String>,
}

async fn show(
    State(state): State<ScreenState>,
    Query(query): Query<ShowQuery>,
    headers: HeaderMap,
) -> Response {
    let (person, _source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match query.mount_id {
        Some(mount_id) => show_trash(&state, &person, &mount_id).await,
        None => show_picker(&state, &person).await,
    }
}

async fn show_trash(state: &ScreenState, person: &str, mount_id: &str) -> Response {
    match list_trash(&state.app, person, mount_id).await {
        Ok(rows) => {
            // Issued only once the page is actually about to render, so a failed
            // render never leaves an unusable token sitting in the store.
            let csrf_token = state.csrf.issue(person);
            let body = trash_page(person, mount_id, &rows, &csrf_token);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
                .into_response()
        }
        Err(e) => error_response(&e),
    }
}

async fn show_picker(state: &ScreenState, person: &str) -> Response {
    match state.app.admin.list_projects_for(person).await {
        Ok(projects) => {
            let ids: Vec<String> = projects.into_iter().map(|p| p.id).collect();
            let body = picker_page(person, &ids);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
                .into_response()
        }
        Err(e) => error_response(&e),
    }
}

/// One row rendered on the screen.
struct TrashRow {
    trash_path: String,
    original_path: String,
}

/// The exact enumeration `fs.trash_list` computes (`tools::trash::trash_list`):
/// never a second implementation.
async fn list_trash(state: &Arc<AppState>, person: &str, mount_id: &str) -> Result<Vec<TrashRow>> {
    state.authorize(mount_id, person).await?;
    let result = trash::trash_list(state, mount_id, "", TRASH_LIST_LIMIT, 0).await?;
    let entries = result["entries"].as_array().cloned().unwrap_or_default();
    Ok(entries
        .into_iter()
        .map(|e| TrashRow {
            trash_path: e["trash_path"].as_str().unwrap_or_default().to_string(),
            original_path: e["original_path"].as_str().unwrap_or_default().to_string(),
        })
        .collect())
}

async fn restore(
    State(state): State<ScreenState>,
    headers: HeaderMap,
    Form(form): Form<RestoreForm>,
) -> Response {
    let (person, source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    if let Some(resp) = check_csrf(&state.csrf, source, &person, form.csrf_token.as_deref()) {
        return resp;
    }
    match restore_entry(&state.app, &person, &form.mount_id, &form.trash_path).await {
        Ok(()) => redirect_to_trash(&form.mount_id),
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

/// Delegates to the exact function `fs.trash_restore` calls; never a second
/// implementation of restore (FR-NEW-012).
async fn restore_entry(
    state: &Arc<AppState>,
    person: &str,
    mount_id: &str,
    trash_path: &str,
) -> Result<()> {
    state.authorize(mount_id, person).await?;
    trash::trash_restore(state, mount_id, trash_path).await?;
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

fn redirect_to_trash(mount_id: &str) -> Response {
    let location = format!("/app/trash?mount_id={}", urlencode(mount_id));
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

/// Minimal percent-encoding sufficient for a `mount_id` in a redirect query
/// string; project ids are already constrained to a safe character set
/// elsewhere, this is just defensive.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

/// The trash listing page for one project: every trashed entry visible to
/// the caller, each with its own restore form carrying the shared
/// `csrf_token`.
fn trash_page(person: &str, mount_id: &str, rows: &[TrashRow], csrf_token: &str) -> String {
    let person = escape_html(person);
    let mount_id_escaped = escape_html(mount_id);
    let csrf_token = escape_html(csrf_token);
    let body = if rows.is_empty() {
        "<p>No trashed files.</p>\n".to_string()
    } else {
        let table_rows: String = rows
            .iter()
            .map(|r| {
                let trash_path = escape_html(&r.trash_path);
                let original_path = escape_html(&r.original_path);
                format!(
                    "<tr data-trash-path=\"{trash_path}\"><td>{original_path}</td><td>\
                     <form method=\"post\" action=\"/app/trash/restore\">\
                     <input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">\
                     <input type=\"hidden\" name=\"mount_id\" value=\"{mount_id_escaped}\">\
                     <input type=\"hidden\" name=\"trash_path\" value=\"{trash_path}\">\
                     <button type=\"submit\">Restore</button></form></td></tr>\n"
                )
            })
            .collect();
        format!(
            "<table>\n<thead><tr><th>Original path</th><th></th></tr></thead>\n\
             <tbody>\n{table_rows}</tbody>\n</table>\n"
        )
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <title>Trash: {mount_id_escaped}</title></head>\n<body>\n\
         <h1>Trash: {mount_id_escaped}</h1>\n<p>Signed in as {person}</p>\n{body}</body>\n</html>\n"
    )
}

/// The project picker rendered when no `mount_id` is given: one link per
/// project the viewer is a member of.
fn picker_page(person: &str, project_ids: &[String]) -> String {
    let person = escape_html(person);
    let body = if project_ids.is_empty() {
        "<p>You are not a member of any project.</p>\n".to_string()
    } else {
        let links: String = project_ids
            .iter()
            .map(|id| {
                let id = escape_html(id);
                format!(
                    "<li data-project-id=\"{id}\">\
                     <a href=\"/app/trash?mount_id={id}\">{id}</a></li>\n"
                )
            })
            .collect();
        format!("<ul>\n{links}</ul>\n")
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <title>Trash</title></head>\n<body>\n<h1>Trash</h1>\n<p>Signed in as {person}</p>\n\
         {body}</body>\n</html>\n"
    )
}

/// Minimal HTML escaping: both the identity claim and listed paths are
/// attacker-controlled by the person who named them, never trusted verbatim
/// in the response body.
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
    /// `deleted_projects_screen::tests::test_setup`.
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

    /// The `csrf_token` value rendered on the trash page's restore form.
    fn extract_csrf_token(body: &str) -> String {
        body.split("name=\"csrf_token\" value=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("a rendered csrf_token")
            .to_string()
    }

    /// One JSON-RPC `initialize` + `notifications/initialized` handshake,
    /// mirroring `deleted_projects_screen::tests::establish_session`.
    async fn establish_session(app: &axum::Router, token: &str) -> String {
        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "trash-screen-test", "version": "0.0"},
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

    async fn create_project(app: axum::Router, token: &str, project_id: &str, owner: &str) {
        call_tool(
            app,
            token,
            "admin.create_project",
            json!({"project_id": project_id, "owner": owner}),
        )
        .await;
    }

    async fn write_file(app: axum::Router, token: &str, project_id: &str, path: &str) {
        call_tool(
            app,
            token,
            "fs.write",
            json!({"mount_id": project_id, "path": path, "content": "x", "overwrite": true, "create_parents": true}),
        )
        .await;
    }

    async fn delete_file(app: axum::Router, token: &str, project_id: &str, path: &str) {
        call_tool(
            app,
            token,
            "fs.delete",
            json!({"mount_id": project_id, "path": path, "recursive": false}),
        )
        .await;
    }

    async fn trash_entries(app: axum::Router, token: &str, project_id: &str) -> Vec<Value> {
        let r = call_tool(
            app,
            token,
            "fs.trash_list",
            json!({"mount_id": project_id, "path_prefix": "", "limit": 200, "offset": 0}),
        )
        .await;
        r["entries"].as_array().cloned().unwrap_or_default()
    }

    // ── E2E-NEW-432 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_432_app_trash_renders_entries() {
        let owner = "alice-432@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_project(app.clone(), &token, "proj-gui-1", owner).await;
        write_file(app.clone(), &token, "proj-gui-1", "/a.txt").await;
        write_file(app.clone(), &token, "proj-gui-1", "/b.txt").await;
        delete_file(app.clone(), &token, "proj-gui-1", "/a.txt").await;
        delete_file(app.clone(), &token, "proj-gui-1", "/b.txt").await;

        let r = app.oneshot(bearer_get("/app/trash?mount_id=proj-gui-1", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(body.contains("/a.txt"), "{body}");
        assert!(body.contains("/b.txt"), "{body}");
        assert!(body.contains("Restore"), "{body}");
    }

    // ── E2E-NEW-433 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_433_restore_post_without_valid_csrf_token_is_rejected() {
        let owner = "alice-433@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_project(app.clone(), &token, "proj-gui-1", owner).await;
        write_file(app.clone(), &token, "proj-gui-1", "/a.txt").await;
        delete_file(app.clone(), &token, "proj-gui-1", "/a.txt").await;

        let entries = trash_entries(app.clone(), &token, "proj-gui-1").await;
        let trash_path = entries[0]["trash_path"].as_str().unwrap().to_string();

        let r = app
            .clone()
            .oneshot(cookie_form_post(
                "/app/trash/restore",
                &token,
                &format!("mount_id=proj-gui-1&trash_path={trash_path}"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["error"], crate::errors::code::FORBIDDEN);

        let entries_after = trash_entries(app, &token, "proj-gui-1").await;
        assert_eq!(entries_after.len(), 1, "the trash_entries row must not be deleted");
    }

    // ── E2E-NEW-434 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_434_non_member_mount_id_renders_an_error_page() {
        let owner = "alice-434@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let owner_token = mint(&key_path, owner, 3600);
        let bob_token = mint(&key_path, "bob-434@test.com", 3600);

        create_project(app.clone(), &owner_token, "proj-gui-1", owner).await;

        let r =
            app.oneshot(bearer_get("/app/trash?mount_id=proj-gui-1", &bob_token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["error"], crate::errors::code::FORBIDDEN);
    }

    // ── E2E-NEW-435 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_435_gui_restore_uses_the_same_underlying_function_as_the_mcp_tool() {
        let owner = "alice-435@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_project(app.clone(), &token, "proj-gui-a", owner).await;
        create_project(app.clone(), &token, "proj-gui-b", owner).await;
        write_file(app.clone(), &token, "proj-gui-a", "/doc.txt").await;
        write_file(app.clone(), &token, "proj-gui-b", "/doc.txt").await;
        delete_file(app.clone(), &token, "proj-gui-a", "/doc.txt").await;
        delete_file(app.clone(), &token, "proj-gui-b", "/doc.txt").await;

        let entries_a = trash_entries(app.clone(), &token, "proj-gui-a").await;
        let trash_path_a = entries_a[0]["trash_path"].as_str().unwrap().to_string();
        let entries_b = trash_entries(app.clone(), &token, "proj-gui-b").await;
        let trash_path_b = entries_b[0]["trash_path"].as_str().unwrap().to_string();

        // Restore `proj-gui-a` via the GUI.
        let r = app
            .clone()
            .oneshot(bearer_get("/app/trash?mount_id=proj-gui-a", &token))
            .await
            .unwrap();
        let body = body_string(r).await;
        let csrf = extract_csrf_token(&body);
        let r = app
            .clone()
            .oneshot(bearer_form_post(
                "/app/trash/restore",
                &token,
                &format!("mount_id=proj-gui-a&trash_path={trash_path_a}&csrf_token={csrf}"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);

        // Restore `proj-gui-b` via the MCP tool directly.
        let restored_b = call_tool(
            app.clone(),
            &token,
            "fs.trash_restore",
            json!({"mount_id": "proj-gui-b", "trash_path": trash_path_b}),
        )
        .await;

        // Identical setups restored through the two surfaces must land on the
        // same path, confirming no divergent collision/restore logic between
        // the GUI and the MCP tool.
        assert_eq!(restored_b["restored_path"], "/doc.txt");
        let exists_a = call_tool(
            app.clone(),
            &token,
            "fs.exists",
            json!({"mount_id": "proj-gui-a", "path": "/doc.txt"}),
        )
        .await;
        let exists_b = call_tool(
            app,
            &token,
            "fs.exists",
            json!({"mount_id": "proj-gui-b", "path": "/doc.txt"}),
        )
        .await;
        assert_eq!(exists_a, exists_b);
    }

    // ── E2E-NEW-455 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_455_gui_renders_an_empty_state_for_a_project_with_no_trashed_files() {
        let owner = "alice-455@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_project(app.clone(), &token, "proj-gui-empty", owner).await;

        let r =
            app.oneshot(bearer_get("/app/trash?mount_id=proj-gui-empty", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(body.contains("No trashed files"), "{body}");
        assert!(!body.contains("data-trash-path"), "{body}");
    }

    // ── E2E-NEW-462 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_462_app_trash_with_no_mount_id_renders_a_project_picker() {
        let owner = "alice-462@test.com";
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), owner);
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, owner, 3600);

        create_project(app.clone(), &token, "proj-gui-1", owner).await;
        create_project(app.clone(), &token, "proj-gui-2", owner).await;

        let r = app.oneshot(bearer_get("/app/trash", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(body.contains("/app/trash?mount_id=proj-gui-1"), "{body}");
        assert!(body.contains("/app/trash?mount_id=proj-gui-2"), "{body}");
    }

    // ── E2E-NEW-463 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_new_463_picker_with_zero_memberships() {
        let d = tempfile::tempdir().unwrap();
        let (c, key_path) = test_setup(d.path(), "owner-463@test.com");
        let app = crate::app::build(c).await.unwrap();
        let token = mint(&key_path, "dave-463@test.com", 3600);

        let r = app.oneshot(bearer_get("/app/trash", &token)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_string(r).await;
        assert!(body.contains("not a member of any project"), "{body}");
    }
}
