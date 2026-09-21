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
//! never a second implementation. No token value is ever rendered. CSRF
//! protection is US-017's.

use crate::errors::{Result, ToolError};
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
use std::sync::Arc;

/// The token screen router. Merge only when `git.enabled` (FR-NEW-046).
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/app/tokens", get(show).post(seed))
        .route("/app/tokens/revoke", post(revoke))
        .with_state(state)
}

#[derive(Deserialize)]
struct SeedForm {
    host: String,
    token: String,
}

#[derive(Deserialize)]
struct RevokeForm {
    host: String,
}

async fn show(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let person = match resolve_person(&state.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match list_tokens(&state, person.clone()).await {
        Ok(rows) => {
            let hosts = crate::git::remote::credentialed_hosts();
            let body = page_shell(&person, &rows, &hosts);
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<SeedForm>,
) -> Response {
    let person = match resolve_person(&state.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match seed_token(&state, person, &form.host, &form.token).await {
        Ok(()) => redirect_to_tokens(),
        Err(e) => error_response(&e),
    }
}

async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<RevokeForm>,
) -> Response {
    let person = match resolve_person(&state.identity, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match revoke_token(&state, person, &form.host).await {
        Ok(()) => redirect_to_tokens(),
        Err(e) => error_response(&e),
    }
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
/// read-only `mcpfs_token` cookie (FR-NEW-047). The cookie's JWT is handed to the
/// exact same [`IdentityResolver::verify`] a header bearer token goes through: no
/// second verification path.
fn resolve_person(identity: &IdentityResolver, headers: &HeaderMap) -> Result<String> {
    let header_err = match identity
        .resolve(|name| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string))
    {
        Ok(person) => return Ok(person),
        Err(e) => e,
    };
    match cookie_value(headers, "mcpfs_token") {
        Some(jwt) => identity.verify(&jwt),
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
fn page_shell(person: &str, rows: &[TokenRow], hosts: &[String]) -> String {
    let person = escape_html(person);
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
}
