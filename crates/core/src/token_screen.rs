//! `/app/tokens*`: the token screen, served on the main server behind identity.
//!
//! Registered only when `git.enabled` is true (FR-NEW-046). There is no identity
//! middleware in this codebase (DRIFT-008): each handler resolves the caller
//! inline, exactly as `api::dataplane` does for its own routes. Three sources are
//! tried in order: the configured forwarded header, then `Authorization`, then a
//! read-only `mcpfs_token` cookie verified by the exact same
//! [`IdentityResolver::verify`] a header bearer token goes through (FR-NEW-047).
//! The cookie is never set, refreshed or cleared by any response from this
//! module, and no route outside this module accepts it.
//!
//! Seeding and revocation delegate to the exact functions the `git.token_set`
//! and `git.auth_revoke` tool handlers call
//! (`crate::tools::git_auth::{token_set, auth_revoke}`), never a second
//! implementation. Listing reuses `git_auth::auth_status` directly, the exact
//! per-person, host-ascending enumeration `git.auth_status` returns
//! (FR-NEW-038, FR-NEW-067, DRIFT-010): a second adapter over one operation,
//! never a second implementation. No token value is ever rendered.
//!
//! CSRF (FR-NEW-048, FR-NEW-058, DEC-038): `GET /app/tokens` issues a fresh,
//! single-use `csrf_token` bound to the requesting person, held only in this
//! router's own in-memory [`CsrfStore`] (never persisted, never part of the
//! broader [`AppState`]: the mechanism is specific to this browser-facing
//! screen). Both `POST` routes require it, but only when the request was
//! authenticated through the ambient `mcpfs_token` cookie: a request carrying
//! a bearer header instead has no ambient credential to forge, so it is
//! exempt.

use crate::errors::{Result, ToolError, code};
use crate::identity::IdentityResolver;
use crate::mcp::registry::ToolCtx;
use crate::state::AppState;
use crate::tools::git_auth;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The token screen router. Merge only when `git.enabled` (FR-NEW-046).
///
/// Builds its own [`CsrfStore`], scoped to this router alone: the mechanism
/// belongs to the browser-facing screen, not to the wider [`AppState`] every
/// other route shares.
pub fn router(state: Arc<AppState>) -> Router {
    let screen_state = ScreenState { app: state, csrf: Arc::new(CsrfStore::default()) };
    Router::new()
        .route("/app/tokens", get(show).post(seed))
        .route("/app/tokens/revoke", post(revoke))
        .with_state(screen_state)
}

/// Extractor state for this router: the shared server state plus this
/// screen's own anti-forgery store.
#[derive(Clone)]
struct ScreenState {
    app: Arc<AppState>,
    csrf: Arc<CsrfStore>,
}

/// In-memory anti-forgery tokens (FR-NEW-058): UUIDv4 string -> the person it
/// was issued to. Never persisted. A token is removed from the map the moment
/// it is redeemed by the person it was issued to, so it cannot be replayed;
/// an attempt by a different person, or with an unknown value, leaves the map
/// untouched.
#[derive(Default)]
struct CsrfStore(Mutex<HashMap<String, String>>);

impl CsrfStore {
    /// Mint a fresh token bound to `person`.
    fn issue(&self, person: &str) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(token.clone(), person.to_string());
        token
    }

    /// Consume `token` iff it is unconsumed and was issued to `person`.
    /// Returns whether the match succeeded. A wrong-person or unknown token is
    /// left in the map untouched, so it never burns someone else's chance to
    /// use their own valid token (FR-NEW-048).
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

/// Which of the three identity sources authenticated a request (FR-NEW-047).
/// Only [`IdentitySource::Cookie`] carries an ambient credential a third party
/// page could trigger, so only it is subject to the `csrf_token` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentitySource {
    Header,
    Cookie,
}

#[derive(Deserialize)]
struct SeedForm {
    host: String,
    token: String,
    #[serde(default)]
    csrf_token: Option<String>,
}

#[derive(Deserialize)]
struct RevokeForm {
    host: String,
    #[serde(default)]
    csrf_token: Option<String>,
}

async fn show(State(state): State<ScreenState>, headers: HeaderMap) -> Response {
    let (person, _source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match list_tokens(&state.app, person.clone()).await {
        Ok(rows) => {
            let hosts = crate::git::remote::credentialed_hosts();
            // Issued only once the page is actually about to render, so a failed
            // render never leaves an unusable token sitting in the store.
            let csrf_token = state.csrf.issue(&person);
            let body = page_shell(&person, &rows, &hosts, &csrf_token);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
                .into_response()
        }
        Err(e) => error_response(&e),
    }
}

/// One row rendered on the screen: host, provider and validity only, never a
/// token value (FR-NEW-038).
struct TokenRow {
    host: String,
    provider: String,
    validity: String,
}

/// The exact per-person enumeration `git.auth_status` reads
/// (`git_auth::auth_status`), already ordered by host ascending: never a
/// second sort, never a second implementation (FR-NEW-067, DRIFT-010). No
/// argument here ever names a person other than the caller resolved from
/// identity: there is no parameter through which one could (FR-NEW-040).
async fn list_tokens(state: &Arc<AppState>, person: String) -> Result<Vec<TokenRow>> {
    let tokens = git_auth::token_store(&state.config, state.stores.relational()).await?;
    let ctx = ToolCtx { person, state: state.clone() };
    let result = git_auth::auth_status(&ctx, None, None, &tokens)?;
    let statuses = result["statuses"].as_array().cloned().unwrap_or_default();
    Ok(statuses
        .into_iter()
        .map(|s| TokenRow {
            host: s["host"].as_str().unwrap_or_default().to_string(),
            provider: s["provider"].as_str().unwrap_or_default().to_string(),
            validity: s["validity"].as_str().unwrap_or_default().to_string(),
        })
        .collect())
}

async fn seed(
    State(state): State<ScreenState>,
    headers: HeaderMap,
    Form(form): Form<SeedForm>,
) -> Response {
    let (person, source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    if let Some(resp) = check_csrf(&state.csrf, source, &person, form.csrf_token.as_deref()) {
        return resp;
    }
    match seed_token(&state.app, person, &form.host, &form.token).await {
        Ok(()) => redirect_to_tokens(),
        Err(e) => error_response(&e),
    }
}

async fn revoke(
    State(state): State<ScreenState>,
    headers: HeaderMap,
    Form(form): Form<RevokeForm>,
) -> Response {
    let (person, source) = match resolve_person(&state.app.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    if let Some(resp) = check_csrf(&state.csrf, source, &person, form.csrf_token.as_deref()) {
        return resp;
    }
    match revoke_token(&state.app, person, &form.host).await {
        Ok(()) => redirect_to_tokens(),
        Err(e) => error_response(&e),
    }
}

/// Enforce FR-NEW-048/FR-NEW-058: a cookie-authenticated request needs a
/// valid, unconsumed `csrf_token` issued to the same person; a
/// header-authenticated request carries no ambient credential and is exempt.
/// `Some` is the rejection response to return as-is; `None` means proceed.
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

/// The exact `403 application/json {"error":"ERR_FORBIDDEN","detail":"..."}`
/// body FR-NEW-058 requires for a missing, unknown, already-consumed or
/// wrong-person `csrf_token`.
fn forbidden_csrf() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": code::FORBIDDEN, "detail": "missing or invalid csrf_token"})),
    )
        .into_response()
}

/// Delegates to the exact function `git.token_set` calls; never a second
/// implementation of token seeding (FR-NEW-046).
async fn seed_token(state: &Arc<AppState>, person: String, host: &str, token: &str) -> Result<()> {
    let tokens = git_auth::token_store(&state.config, state.stores.relational()).await?;
    let ctx = ToolCtx { person, state: state.clone() };
    git_auth::token_set(&ctx, host, token, None, &tokens).await?;
    Ok(())
}

/// Delegates to the exact function `git.auth_revoke` calls; never a second
/// implementation of revocation (FR-NEW-046).
async fn revoke_token(state: &Arc<AppState>, person: String, host: &str) -> Result<()> {
    let tokens = git_auth::token_store(&state.config, state.stores.relational()).await?;
    let ctx = ToolCtx { person, state: state.clone() };
    git_auth::auth_revoke(&ctx, None, Some(host), &tokens).await?;
    Ok(())
}

/// Resolve the caller from the forwarded header, then `Authorization`, then the
/// read-only `mcpfs_token` cookie (FR-NEW-047), also reporting which source
/// authenticated the request so a `POST` handler knows whether an ambient
/// credential was used and `csrf_token` must therefore be checked
/// (FR-NEW-048). The cookie's JWT is handed to the exact same
/// [`IdentityResolver::verify`] a header bearer token goes through: no second
/// verification path.
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

/// The 401 body shape documented at `crate::app` (`app.rs:10-11`):
/// `{"error":"ERR_UNAUTHENTICATED","detail":"..."}`.
fn unauthorized(err: &ToolError) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": err.code, "detail": err.message})))
        .into_response()
}

/// Any other tool error, mapped to its HTTP status (FR-NEW-046: a rejected host
/// or token is `400` with `ERR_INVALID_ARGUMENT`).
fn error_response(err: &ToolError) -> Response {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({"error": err.code, "detail": err.message}))).into_response()
}

fn redirect_to_tokens() -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/app/tokens")]).into_response()
}

/// The page shell: the person's own held hosts (FR-NEW-038, never a token
/// value) and a host selector limited to declared, non-anonymous hosts
/// (E2E-NEW-103). `data-host`/`data-validity` attributes on each row are for
/// this module's own tests only; no client script depends on them.
fn page_shell(person: &str, rows: &[TokenRow], hosts: &[String], csrf_token: &str) -> String {
    let person = escape_html(person);
    let csrf_token = escape_html(csrf_token);
    let table_rows: String = rows
        .iter()
        .map(|r| {
            let host = escape_html(&r.host);
            let provider = escape_html(&r.provider);
            let validity = escape_html(&r.validity);
            format!(
                "<tr data-host=\"{host}\" data-validity=\"{validity}\"><td>{host}</td><td>{provider}</td><td>{validity}</td></tr>\n"
            )
        })
        .collect();
    let options: String = hosts
        .iter()
        .map(|h| {
            let host = escape_html(h);
            format!("<option value=\"{host}\">{host}</option>\n")
        })
        .collect();
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Git tokens</title></head>\n\
         <body>\n<h1>Git tokens</h1>\n<p>Signed in as {person}</p>\n\
         <table>\n<thead><tr><th>Host</th><th>Provider</th><th>Validity</th></tr></thead>\n\
         <tbody>\n{table_rows}</tbody>\n</table>\n\
         <form method=\"post\" action=\"/app/tokens\">\n\
         <input type=\"hidden\" name=\"csrf_token\" value=\"{csrf_token}\">\n\
         <select name=\"host\">\n{options}</select>\n\
         <input name=\"token\" type=\"password\"><button type=\"submit\">Save</button>\n\
         </form>\n</body>\n</html>\n"
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
    use crate::config::{GitConfig, ServerConfig};
    use crate::keys;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use tower::ServiceExt;

    /// The story's reference host map. `git.unknown.test` is deliberately absent.
    const REFERENCE_HOSTS: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    const GHP: &str = "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH";

    fn declare_reference_hosts() {
        let mut cfg = GitConfig::default();
        cfg.hosts.0 = REFERENCE_HOSTS.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        crate::git::remote::validate_hosts(&cfg).expect("a valid host map");
    }

    /// `crate::git::remote`'s host map is a process wide static, so every test
    /// that reads or publishes it must serialize through this lock, exactly as
    /// `tools::git_auth`'s own tests do.
    fn with_git_hosts_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::git::remote::tests::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// A config wired to a throwaway state root, a fresh RS256 keypair, and
    /// `git.enabled`. Mirrors `crate::app::tests::test_setup`, extended for git.
    fn test_setup(root: &Path) -> (ServerConfig, PathBuf) {
        let (key_path, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
        let mut c = ServerConfig::default();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.infra.meta.dir = root.join("volumes").display().to_string();
        c.infra.blob.dir = root.join("blobs").display().to_string();
        c.infra.admin.path = root.join("admin.db").display().to_string();
        c.git.enabled = true;
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

    /// A JWT signed by a throwaway key the server never trusts.
    fn mint_wrong_key(email: &str) -> String {
        let kp = keys::generate_keypair().unwrap();
        keys::mint_token(&kp.private_pem, email, keys::DEFAULT_ISSUER, keys::DEFAULT_CLAIM, 3600)
            .unwrap()
    }

    async fn body_string(r: Response) -> String {
        let b = to_bytes(r.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    /// The `data-host` attribute of every rendered row, in document order:
    /// the screen's own view of row order, read the same way for every test
    /// that asserts on it (E2E-NEW-230, E2E-NEW-231, E2E-NEW-232).
    fn extract_hosts_in_order(body: &str) -> Vec<String> {
        body.split("data-host=\"")
            .skip(1)
            .map(|s| s.split('"').next().unwrap_or("").to_string())
            .collect()
    }

    /// No substring of `token` at least `min_len` characters long may appear
    /// in `body` (E2E-NEW-104, FR-NEW-038).
    fn assert_no_token_substring(body: &str, token: &str, min_len: usize) {
        let chars: Vec<char> = token.chars().collect();
        if chars.len() < min_len {
            return;
        }
        for window in chars.windows(min_len) {
            let s: String = window.iter().collect();
            assert!(!body.contains(&s), "response leaked token substring: {s}");
        }
    }

    fn get_req(
        uri: &str,
        header_name: Option<&str>,
        token: Option<&str>,
        cookie: Option<&str>,
    ) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri);
        if let (Some(name), Some(t)) = (header_name, token) {
            b = b.header(name, format!("Bearer {t}"));
        }
        if let Some(c) = cookie {
            b = b.header("Cookie", format!("mcpfs_token={c}"));
        }
        b.body(Body::empty()).unwrap()
    }

    fn bearer_get(uri: &str, token: &str) -> Request<Body> {
        get_req(uri, Some("Authorization"), Some(token), None)
    }

    fn cookie_get(uri: &str, jwt: &str) -> Request<Body> {
        get_req(uri, None, None, Some(jwt))
    }

    fn form_post(uri: &str, token: Option<&str>, form_body: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        b.body(Body::from(form_body.to_string())).unwrap()
    }

    /// A `POST` authenticated by the `mcpfs_token` cookie alone, carrying no
    /// bearer header: the shape a cross-origin form submission would take.
    fn cookie_form_post(uri: &str, jwt: &str, form_body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cookie", format!("mcpfs_token={jwt}"))
            .body(Body::from(form_body.to_string()))
            .unwrap()
    }

    /// The `csrf_token` value rendered by `GET /app/tokens`
    /// (`<input type="hidden" name="csrf_token" value="...">`), extracted the
    /// same way every test that needs a valid anti-forgery token reads it.
    fn extract_csrf_token(body: &str) -> String {
        body.split("name=\"csrf_token\" value=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("a rendered csrf_token")
            .to_string()
    }

    /// Percent-encode a value for an `application/x-www-form-urlencoded` body.
    fn form_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 3);
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char)
                }
                b' ' => out.push('+'),
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// One JSON-RPC `tools/call`, decoded from the SSE frame the way `app.rs`'s
    /// own tests do.
    async fn call_tool(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();
        let r = app.oneshot(req).await.unwrap();
        let body = body_string(r).await;
        let json_str = body.trim_start_matches("event: message\ndata: ").trim_end();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_ne!(v["result"]["isError"], Value::Bool(true), "tool call failed: {v}");
        let text = v["result"]["content"][0]["text"].as_str().expect("a text content block");
        serde_json::from_str(text).unwrap()
    }

    // ── E2E-NEW-105 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_105_an_unauthenticated_request_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, _k) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let r = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/app/tokens")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::UNAUTHENTICATED);
            assert!(v["detail"].is_string());
        });
    }

    // ── E2E-NEW-106 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_106_an_invalid_or_expired_jwt_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();

            let wrong = mint_wrong_key("alice@test.com");
            let r = app.clone().oneshot(bearer_get("/app/tokens", &wrong)).await.unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "wrong signing key must be rejected");

            let expired = mint(&key_path, "alice@test.com", -600);
            let r = app.oneshot(bearer_get("/app/tokens", &expired)).await.unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "expired token must be rejected");
        });
    }

    // ── E2E-NEW-164 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_164_the_screen_route_serves_html_to_an_authenticated_person() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "alice@test.com", 3600);
            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let ct = r.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
            assert!(ct.starts_with("text/html"), "content type was {ct}");
        });
    }

    // ── E2E-NEW-165 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_165_seeding_through_the_route_redirects() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let person = "e2e-new-165@test.com";
            let token = mint(&key_path, person, 3600);

            let r = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.ibm.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);
            assert_eq!(r.headers().get(header::LOCATION).unwrap(), "/app/tokens");

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(
                statuses.iter().any(|s| s["host"] == "github.ibm.com"),
                "git.auth_status did not list github.ibm.com: {statuses:?}"
            );
        });
    }

    // ── E2E-NEW-166 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_166_revocation_through_the_route_redirects() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let person = "e2e-new-166@test.com";
            let token = mint(&key_path, person, 3600);

            // Precondition: a token already seeded for this host.
            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host": "github.ibm.com", "token": GHP}),
            )
            .await;

            let r = app
                .clone()
                .oneshot(form_post("/app/tokens/revoke", Some(&token), "host=github.ibm.com"))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);
            assert_eq!(r.headers().get(header::LOCATION).unwrap(), "/app/tokens");

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(
                !statuses.iter().any(|s| s["host"] == "github.ibm.com"),
                "github.ibm.com must be gone after revocation: {statuses:?}"
            );
        });
    }

    // ── E2E-NEW-167 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_167_a_rejected_host_returns_400_with_the_error_body() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-167@test.com", 3600);

            let r = app
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=git.unknown.test&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::INVALID_ARGUMENT);
            assert!(v["detail"].as_str().unwrap().contains("git.unknown.test"), "{v}");
        });
    }

    // ── E2E-NEW-168 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_168_the_screen_routes_are_absent_when_git_is_disabled() {
        with_git_hosts_lock(async {
            let d = tempfile::tempdir().unwrap();
            let (mut c, _k) = test_setup(d.path());
            c.git.enabled = false;
            let app = crate::app::build(c).await.unwrap();
            let r = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/app/tokens")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::NOT_FOUND);
        });
    }

    // ── E2E-NEW-169 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_169_identity_resolves_from_the_forwarded_header() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "alice@test.com", 3600);
            let r = app
                .oneshot(get_req(
                    "/app/tokens",
                    Some("X-Forwarded-Authorization"),
                    Some(&token),
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert!(body.contains("alice@test.com"), "{body}");
        });
    }

    // ── E2E-NEW-170 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_170_identity_resolves_from_the_authorization_header() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "alice@test.com", 3600);
            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert!(body.contains("alice@test.com"), "{body}");
        });
    }

    // ── E2E-NEW-171 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_171_identity_resolves_from_the_cookie() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "alice@test.com", 3600);
            let r = app.oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK, "a cookie-only navigation must be enough");
            let body = body_string(r).await;
            assert!(body.contains("alice@test.com"), "{body}");
        });
    }

    // ── E2E-NEW-172 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_172_a_request_with_no_credential_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, _k) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let r = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/app/tokens")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::UNAUTHENTICATED);
            assert!(v["detail"].is_string());
        });
    }

    // ── E2E-NEW-173 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_173_a_cookie_holding_an_invalid_jwt_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();

            let wrong = mint_wrong_key("alice@test.com");
            let r = app.clone().oneshot(cookie_get("/app/tokens", &wrong)).await.unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "wrong signing key in the cookie");

            let expired = mint(&key_path, "alice@test.com", -600);
            let r = app.oneshot(cookie_get("/app/tokens", &expired)).await.unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "expired token in the cookie");
        });
    }

    // ── E2E-NEW-174 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_174_no_route_outside_the_screen_accepts_the_cookie() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "alice@test.com", 3600);

            let r = app.clone().oneshot(cookie_get("/api/fs/roots", &token)).await.unwrap();
            assert_eq!(
                r.status(),
                StatusCode::UNAUTHORIZED,
                "/api/fs must reject a cookie-only request"
            );

            let payload = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
            let mcp_req = Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("Cookie", format!("mcpfs_token={token}"))
                .header("Content-Type", "application/json")
                .body(Body::from(payload))
                .unwrap();
            let r = app.oneshot(mcp_req).await.unwrap();
            assert_eq!(
                r.status(),
                StatusCode::UNAUTHORIZED,
                "/mcp must reject a cookie-only request"
            );
        });
    }

    // ── E2E-NEW-175 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_175_the_server_never_sets_the_cookie() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-175@test.com", 3600);

            let health = app
                .clone()
                .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert!(health.headers().get(header::SET_COOKIE).is_none());

            let unauth = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/app/tokens")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(unauth.headers().get(header::SET_COOKIE).is_none());

            let shown = app.clone().oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert!(shown.headers().get(header::SET_COOKIE).is_none());

            let seeded = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.ibm.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert!(seeded.headers().get(header::SET_COOKIE).is_none());

            let revoked = app
                .clone()
                .oneshot(form_post("/app/tokens/revoke", Some(&token), "host=github.ibm.com"))
                .await
                .unwrap();
            assert!(revoked.headers().get(header::SET_COOKIE).is_none());

            let mcp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/mcp")
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Content-Type", "application/json")
                        .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(mcp.headers().get(header::SET_COOKIE).is_none());
        });
    }

    // ── E2E-NEW-100 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_100_the_screen_lists_held_hosts() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-100-alice@test.com";
            let token = mint(&key_path, alice, 3600);

            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.com","token":GHP}),
            )
            .await;
            let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP,"expires_at":past}),
            )
            .await;

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert_eq!(
                extract_hosts_in_order(&body),
                vec!["github.com".to_string(), "github.ibm.com".to_string()]
            );
            assert!(body.contains("data-host=\"github.com\" data-validity=\"valid\""), "{body}");
            assert!(
                body.contains("data-host=\"github.ibm.com\" data-validity=\"expired\""),
                "{body}"
            );
        });
    }

    // ── E2E-NEW-103 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_103_the_screen_offers_only_declared_hosts() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-103@test.com", 3600);

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            for host in ["github.com", "github.ibm.com", "gitlab.acme.corp", "git.acme.internal"] {
                assert!(
                    body.contains(&format!("value=\"{host}\"")),
                    "selector must offer {host}: {body}"
                );
            }
            assert!(
                !body.contains("public.example.org"),
                "the anonymous host must never be offered: {body}"
            );
        });
    }

    // ── E2E-NEW-104 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_104_no_token_value_reaches_the_browser() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-104@test.com", 3600);

            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.com","token":GHP}),
            )
            .await;

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert_no_token_substring(&body, GHP, 8);
        });
    }

    // ── E2E-NEW-107 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_107_one_person_cannot_read_anothers_tokens() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-107-alice@test.com";
            let bob = "e2e-new-107-bob@test.com";
            let alice_token = mint(&key_path, alice, 3600);
            let bob_token = mint(&key_path, bob, 3600);

            call_tool(
                app.clone(),
                &alice_token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP}),
            )
            .await;

            // Bob's own request, decorated with a query parameter naming alice: the
            // route accepts no such parameter (FR-NEW-040), so it is never even
            // read by the handler and has no effect.
            let query_alice = alice.replace('@', "%40");
            let r = app
                .oneshot(bearer_get(&format!("/app/tokens?person={query_alice}"), &bob_token))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert!(body.contains(bob), "must show bob's own identity: {body}");
            // The host selector legitimately lists github.ibm.com (a declared host,
            // independent of who holds a token for it): what must never leak is a
            // *row*, which only ever comes from the caller's own held hosts.
            assert!(
                extract_hosts_in_order(&body).is_empty(),
                "bob holds no tokens of his own, so no row for alice's host may appear: {body}"
            );
        });
    }

    // ── E2E-NEW-108 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_108_a_platform_admin_cannot_read_another_persons_tokens() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (mut c, key_path) = test_setup(d.path());
            let bob = "e2e-new-108-bob@test.com";
            c.auth.admins = vec![bob.to_string()];
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-108-alice@test.com";
            let alice_token = mint(&key_path, alice, 3600);
            let bob_token = mint(&key_path, bob, 3600);

            call_tool(
                app.clone(),
                &alice_token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP}),
            )
            .await;

            // Bob, a platform admin, has no code path to view alice's tokens:
            // identity resolution always yields the requesting person, never a
            // named target (FR-NEW-040). His own screen never shows her host.
            let r = app.clone().oneshot(bearer_get("/app/tokens", &bob_token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            // The host selector legitimately lists github.ibm.com (a declared host,
            // independent of who holds a token for it): what must never leak is a
            // *row*, which only ever comes from the caller's own held hosts.
            assert!(
                extract_hosts_in_order(&body).is_empty(),
                "bob holds no tokens of his own, so no row for alice's host may appear: {body}"
            );

            // An admin "attempt" to revoke that host operates only on bob's own
            // (nonexistent) record: an ordinary, idempotent no-op for him, and
            // alice's actual token is left untouched.
            let r = app
                .clone()
                .oneshot(form_post("/app/tokens/revoke", Some(&bob_token), "host=github.ibm.com"))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            let result =
                call_tool(app, &alice_token, "git.auth_status", json!({"host":"github.ibm.com"}))
                    .await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(
                statuses.iter().any(|s| s["host"] == "github.ibm.com" && s["validity"] == "valid"),
                "alice's token must be untouched by bob's admin claim: {statuses:?}"
            );
        });
    }

    // ── E2E-NEW-109 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_109_seeding_an_undeclared_host_through_the_screen_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-109@test.com", 3600);

            // The selector would never offer this host (E2E-NEW-103); this request
            // bypasses it entirely, proving git.token_set's own validation still
            // runs server-side (US-004), reached through the screen route.
            let r = app
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=git.unknown.test&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::INVALID_ARGUMENT);
            assert!(v["detail"].as_str().unwrap().contains("git.unknown.test"), "{v}");
        });
    }

    // ── E2E-NEW-230 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_230_the_screen_orders_rows_by_host() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-230@test.com", 3600);

            // Seed out of order, so ordering cannot pass by accident.
            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP}),
            )
            .await;
            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.com","token":GHP}),
            )
            .await;

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            let body = body_string(r).await;
            assert_eq!(
                extract_hosts_in_order(&body),
                vec!["github.com".to_string(), "github.ibm.com".to_string()]
            );
        });
    }

    // ── E2E-NEW-231 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_231_the_screen_and_the_tool_agree_on_order() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-231@test.com", 3600);

            for host in ["git.acme.internal", "gitlab.acme.corp", "github.ibm.com", "github.com"] {
                call_tool(app.clone(), &token, "git.token_set", json!({"host":host,"token":GHP}))
                    .await;
            }

            let r = app.clone().oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            let body = body_string(r).await;
            let screen_order = extract_hosts_in_order(&body);

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let tool_order: Vec<String> = result["statuses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["host"].as_str().unwrap().to_string())
                .collect();

            assert_eq!(screen_order, tool_order);
            assert_eq!(screen_order.len(), 4);
        });
    }

    // ── E2E-NEW-232 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_232_an_empty_token_list_renders_without_error() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-232@test.com", 3600);

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert!(extract_hosts_in_order(&body).is_empty(), "{body}");
        });
    }

    // ── US-017: seeding, revocation and CSRF (FR-NEW-039, FR-NEW-048, FR-NEW-058, FR-NEW-059) ──

    const GLPAT: &str = "glpat_1111222233334444555566667777";

    // ── E2E-NEW-101 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_101_the_screen_seeds_a_token() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-101-alice@test.com";
            let token = mint(&key_path, alice, 3600);

            let r = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=gitlab.acme.corp&token={GLPAT}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            let result =
                call_tool(app, &token, "git.auth_status", json!({"host":"gitlab.acme.corp"})).await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(
                statuses
                    .iter()
                    .any(|s| s["host"] == "gitlab.acme.corp" && s["validity"] == "valid"),
                "gitlab.acme.corp must be valid after seeding through the screen: {statuses:?}"
            );
        });
    }

    // ── E2E-NEW-102 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_102_the_screen_revokes_a_token() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-102-alice@test.com";
            let token = mint(&key_path, alice, 3600);

            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.com","token":GHP}),
            )
            .await;
            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP}),
            )
            .await;

            let r = app
                .clone()
                .oneshot(form_post("/app/tokens/revoke", Some(&token), "host=github.ibm.com"))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(
                !statuses.iter().any(|s| s["host"] == "github.ibm.com"),
                "github.ibm.com must be gone: {statuses:?}"
            );
            assert!(
                statuses.iter().any(|s| s["host"] == "github.com" && s["validity"] == "valid"),
                "github.com must remain: {statuses:?}"
            );
        });
    }

    // ── E2E-NEW-110 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_110_an_empty_token_submitted_through_the_screen_is_rejected() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-110@test.com", 3600);

            let r = app
                .clone()
                .oneshot(form_post("/app/tokens", Some(&token), "host=github.com&token="))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::INVALID_ARGUMENT);

            let result =
                call_tool(app, &token, "git.auth_status", json!({"host":"github.com"})).await;
            assert!(
                !result["statuses"].as_array().unwrap().iter().any(|s| s["host"] == "github.com"),
                "nothing must be stored under the same rule git.token_set enforces"
            );
        });
    }

    // ── E2E-NEW-112 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_112_the_screen_and_the_tools_agree() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-112@test.com", 3600);

            let r = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            call_tool(
                app.clone(),
                &token,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP}),
            )
            .await;

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            for host in ["github.com", "github.ibm.com"] {
                assert!(
                    statuses.iter().any(|s| s["host"] == host && s["validity"] == "valid"),
                    "{host} must be reported identically regardless of adapter: {statuses:?}"
                );
            }
        });
    }

    // ── E2E-NEW-113 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_113_injection_in_a_submitted_value_is_neutralised() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-113@test.com", 3600);

            let script_payload = "<script>alert(1)</script>ABCDEFGH01234567";
            let sql_payload = "'; DROP TABLE oauth_tokens; --ABCDEFGH01234567";

            let r = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.com&token={}", form_encode(script_payload)),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            let r = app
                .clone()
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.ibm.com&token={}", form_encode(sql_payload)),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            // Neither payload is reflected unescaped, and the screen still renders.
            let r = app.clone().oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            assert!(!body.contains("<script>alert(1)</script>"), "{body}");
            assert!(!body.contains("DROP TABLE"), "{body}");

            // The store is intact: both hosts round trip as valid, proving the
            // underlying table was never dropped and normal reads still work.
            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            for host in ["github.com", "github.ibm.com"] {
                assert!(
                    statuses.iter().any(|s| s["host"] == host && s["validity"] == "valid"),
                    "{host} must survive the injection attempt intact: {statuses:?}"
                );
            }
        });
    }

    // ── E2E-NEW-176 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_176_a_cookie_authenticated_post_without_the_anti_forgery_token_is_refused() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-176@test.com", 3600);

            let r = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::FORBIDDEN);
            let v: Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::FORBIDDEN);

            let result =
                call_tool(app, &token, "git.auth_status", json!({"host":"github.com"})).await;
            assert!(
                !result["statuses"].as_array().unwrap().iter().any(|s| s["host"] == "github.com"),
                "no credential must be stored"
            );
        });
    }

    // ── E2E-NEW-177 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_177_a_post_carrying_the_issued_anti_forgery_token_succeeds() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-177@test.com", 3600);

            let shown = app.clone().oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(shown.status(), StatusCode::OK);
            let csrf = extract_csrf_token(&body_string(shown).await);

            let r = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.com&token={GHP}&csrf_token={csrf}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);
        });
    }

    // ── E2E-NEW-178 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_178_another_persons_anti_forgery_token_is_refused() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let alice = "e2e-new-178-alice@test.com";
            let bob = "e2e-new-178-bob@test.com";
            let alice_token = mint(&key_path, alice, 3600);
            let bob_token = mint(&key_path, bob, 3600);

            let shown = app.clone().oneshot(cookie_get("/app/tokens", &bob_token)).await.unwrap();
            let bob_csrf = extract_csrf_token(&body_string(shown).await);

            let r = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &alice_token,
                    &format!("host=github.com&token={GHP}&csrf_token={bob_csrf}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::FORBIDDEN);

            let result =
                call_tool(app, &alice_token, "git.auth_status", json!({"host":"github.com"})).await;
            assert!(
                !result["statuses"].as_array().unwrap().iter().any(|s| s["host"] == "github.com")
            );
        });
    }

    // ── E2E-NEW-179 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_179_an_anti_forgery_token_is_single_use() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-179@test.com", 3600);

            let shown = app.clone().oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            let csrf = extract_csrf_token(&body_string(shown).await);

            let first = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.com&token={GHP}&csrf_token={csrf}"),
                ))
                .await
                .unwrap();
            assert_eq!(first.status(), StatusCode::SEE_OTHER);

            let second = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.ibm.com&token={GHP}&csrf_token={csrf}"),
                ))
                .await
                .unwrap();
            assert_eq!(
                second.status(),
                StatusCode::FORBIDDEN,
                "a replayed csrf_token must be refused"
            );

            let result = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = result["statuses"].as_array().unwrap();
            assert!(statuses.iter().any(|s| s["host"] == "github.com"));
            assert!(
                !statuses.iter().any(|s| s["host"] == "github.ibm.com"),
                "the replay must have stored nothing"
            );
        });
    }

    // ── E2E-NEW-180 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_180_a_header_authenticated_post_is_exempt() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-180@test.com", 3600);

            let r = app
                .oneshot(form_post(
                    "/app/tokens",
                    Some(&token),
                    &format!("host=github.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(
                r.status(),
                StatusCode::SEE_OTHER,
                "no ambient credential, so no csrf_token is required"
            );
        });
    }

    // ── E2E-NEW-203 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_203_the_anti_forgery_field_is_rendered_with_its_exact_name() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-203@test.com", 3600);

            let r = app.oneshot(bearer_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            let body = body_string(r).await;
            let csrf = extract_csrf_token(&body);
            assert!(
                body.contains(&format!(
                    "<input type=\"hidden\" name=\"csrf_token\" value=\"{csrf}\">"
                )),
                "{body}"
            );
        });
    }

    // ── E2E-NEW-204 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_204_a_post_carrying_csrf_token_succeeds() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-204@test.com", 3600);

            let shown = app.clone().oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            let csrf = extract_csrf_token(&body_string(shown).await);

            let r = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.ibm.com&token={GHP}&csrf_token={csrf}"),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::SEE_OTHER);

            let result =
                call_tool(app, &token, "git.auth_status", json!({"host":"github.ibm.com"})).await;
            assert!(
                result["statuses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["host"] == "github.ibm.com" && s["validity"] == "valid")
            );
        });
    }

    // ── E2E-NEW-205 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_205_a_missing_or_consumed_csrf_token_returns_the_exact_error_body() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-205@test.com", 3600);

            // Missing field: refused, nothing stored.
            let missing = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(missing.status(), StatusCode::FORBIDDEN);
            let v: Value = serde_json::from_str(&body_string(missing).await).unwrap();
            assert_eq!(v["error"], crate::errors::code::FORBIDDEN);
            assert!(v["detail"].is_string());
            assert_eq!(v.as_object().unwrap().len(), 2, "body must be exactly {{error, detail}}");

            let status_after_missing =
                call_tool(app.clone(), &token, "git.auth_status", json!({"host":"github.com"}))
                    .await;
            assert!(
                !status_after_missing["statuses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["host"] == "github.com"),
                "the missing-field attempt must store nothing"
            );

            // Consumed: redeem once, then replay the same token for another host.
            let shown = app.clone().oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            let csrf = extract_csrf_token(&body_string(shown).await);
            let form = format!("host=github.ibm.com&token={GHP}&csrf_token={csrf}");
            let first =
                app.clone().oneshot(cookie_form_post("/app/tokens", &token, &form)).await.unwrap();
            assert_eq!(first.status(), StatusCode::SEE_OTHER);

            let replay_form = format!("host=gitlab.acme.corp&token={GHP}&csrf_token={csrf}");
            let replayed = app
                .clone()
                .oneshot(cookie_form_post("/app/tokens", &token, &replay_form))
                .await
                .unwrap();
            assert_eq!(replayed.status(), StatusCode::FORBIDDEN);
            let v2: Value = serde_json::from_str(&body_string(replayed).await).unwrap();
            assert_eq!(v2["error"], crate::errors::code::FORBIDDEN);
            assert!(v2["detail"].is_string());
            assert_eq!(v2.as_object().unwrap().len(), 2, "body must be exactly {{error, detail}}");

            let final_status = call_tool(app, &token, "git.auth_status", json!({})).await;
            let statuses = final_status["statuses"].as_array().unwrap();
            assert!(
                statuses.iter().any(|s| s["host"] == "github.ibm.com"),
                "the earlier valid redemption must stand"
            );
            assert!(
                !statuses.iter().any(|s| s["host"] == "gitlab.acme.corp"),
                "the replay must store nothing"
            );
        });
    }

    // ── E2E-NEW-206 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_206_no_response_across_the_server_ever_sets_the_session_cookie() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-206@test.com", 3600);

            let mut responses = Vec::new();
            responses.push(
                app.clone()
                    .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
                    .await
                    .unwrap(),
            );
            responses.push(app.clone().oneshot(bearer_get("/app/tokens", &token)).await.unwrap());
            responses.push(app.clone().oneshot(cookie_get("/app/tokens", &token)).await.unwrap());
            responses.push(
                app.clone()
                    .oneshot(form_post(
                        "/app/tokens",
                        Some(&token),
                        &format!("host=github.com&token={GHP}"),
                    ))
                    .await
                    .unwrap(),
            );
            responses.push(
                app.clone()
                    .oneshot(form_post("/app/tokens/revoke", Some(&token), "host=github.com"))
                    .await
                    .unwrap(),
            );
            responses.push(
                app.clone()
                    .oneshot(cookie_form_post(
                        "/app/tokens",
                        &token,
                        &format!("host=github.com&token={GHP}"),
                    ))
                    .await
                    .unwrap(),
            );
            responses.push(
                app.clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/mcp")
                            .header("Authorization", format!("Bearer {token}"))
                            .header("Content-Type", "application/json")
                            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
                            .unwrap(),
                    )
                    .await
                    .unwrap(),
            );
            responses.push(app.clone().oneshot(bearer_get("/api/fs/roots", &token)).await.unwrap());
            responses.push(app.clone().oneshot(cookie_get("/api/fs/roots", &token)).await.unwrap());
            responses.push(
                app.oneshot(Request::builder().uri("/app/tokens").body(Body::empty()).unwrap())
                    .await
                    .unwrap(),
            );

            for r in &responses {
                assert!(
                    r.headers().get(header::SET_COOKIE).is_none(),
                    "a response ({}) must never set mcpfs_token: {:?}",
                    r.status(),
                    r.headers()
                );
            }
        });
    }

    // ── E2E-NEW-207 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_207_the_cross_origin_defence_does_not_depend_on_cookie_attributes() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-207@test.com", 3600);

            // A cross-origin POST carries only whatever `Cookie` header the browser
            // attaches; this server never sets `SameSite` (FR-NEW-059), so this
            // request is indistinguishable, at the wire level, from one sent with
            // no SameSite protection at all.
            let r = app
                .clone()
                .oneshot(cookie_form_post(
                    "/app/tokens",
                    &token,
                    &format!("host=github.com&token={GHP}"),
                ))
                .await
                .unwrap();
            assert_eq!(
                r.status(),
                StatusCode::FORBIDDEN,
                "csrf_token alone must carry the defence"
            );

            let result =
                call_tool(app, &token, "git.auth_status", json!({"host":"github.com"})).await;
            assert!(
                !result["statuses"].as_array().unwrap().iter().any(|s| s["host"] == "github.com")
            );
        });
    }

    // ── E2E-NEW-208 ──────────────────────────────────────────────────────────

    #[test]
    fn e2e_new_208_the_screen_works_behind_a_component_that_sets_the_cookie() {
        with_git_hosts_lock(async {
            declare_reference_hosts();
            let d = tempfile::tempdir().unwrap();
            let (c, key_path) = test_setup(d.path());
            let app = crate::app::build(c).await.unwrap();
            let token = mint(&key_path, "e2e-new-208@test.com", 3600);

            // The token below simulates one an upstream component minted and
            // attached as `mcpfs_token`; the server never mints or re-issues it.
            let r = app.oneshot(cookie_get("/app/tokens", &token)).await.unwrap();
            assert_eq!(
                r.status(),
                StatusCode::OK,
                "the screen must render from the upstream-issued cookie alone"
            );
            assert!(
                r.headers().get(header::SET_COOKIE).is_none(),
                "the server must never re-issue the cookie"
            );
        });
    }
}
