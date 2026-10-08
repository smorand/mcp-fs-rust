//! `GET /exports/{token}` (SPEC-0012 US-0003): the one route a recipient
//! holding no system credential uses to download an export minted by
//! `fs.export_zip`.
//!
//! The token is the sole authorization (DEC-004, FR-NEW-017): no identity is
//! resolved and no membership is checked. Single use is enforced by the
//! atomic `DELETE ... WHERE token=? AND expires_at>?` of
//! `storage::meta::delete_export_link_if_live` (FR-NEW-013); expiry is
//! evaluated there, once, at request start, and never again (FR-NEW-012).
//!
//! Mounted unconditionally in `app.rs`, outside the `api.enabled` block, so a
//! link minted over MCP stays downloadable on an MCP only deployment
//! (FR-NEW-009b). There is deliberately no `DELETE` route (FR-NEW-018).

use crate::errors::Result;
use crate::state::AppState;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::sync::Arc;

/// The router carrying the download route, merged by `app.rs`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().route("/exports/{token}", get(download)).with_state(state)
}

/// Time: O(P + B), P the number of projects probed for the token's row, B the
/// archive size. Space: O(B), the archive read once into memory and handed to
/// `attachment`, the same fully buffered model `download`/`download-zip` use.
async fn download(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    match claim(&state, &token).await {
        Ok(Some(volume)) => serve(&state, &volume, &token).await,
        Ok(None) => {
            reap_expired(&state, &token).await;
            // One body for never existed, consumed, swept and expired
            // (FR-NEW-010): nothing here may say which case applied.
            StatusCode::NOT_FOUND.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, token_prefix = prefix(&token), "export download failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The volume whose live row for `token` this call deleted, if any.
///
/// The URL carries no volume and the SQLite meta store is one file per
/// volume, so every project is probed. The delete itself is the single
/// expiry check and the single use guarantee: of two racing callers only one
/// sees an affected row (FR-NEW-013). Nothing after this re checks expiry
/// (FR-NEW-012).
async fn claim(state: &AppState, token: &str) -> Result<Option<String>> {
    if !is_canonical(token) {
        return Ok(None);
    }
    let now = crate::tools::export::iso(chrono::Utc::now());
    let mut failure = None;
    for project in state.admin.list_all_projects().await? {
        match delete_row(state, &project.id, token, &now).await {
            Ok(true) => return Ok(Some(project.id)),
            Ok(false) => {}
            // One unreadable volume must not hide a token living in another.
            Err(e) => failure = Some(e),
        }
    }
    failure.map_or(Ok(None), Err)
}

/// Reads, deletes and returns the archive of a row this request already
/// claimed. No expiry check: the claim decided, and a slow transfer is never
/// cut short (FR-NEW-012).
async fn serve(state: &AppState, volume: &str, token: &str) -> Response {
    let key = format!("export:{token}");
    let bytes = match state.stores.client(volume).await {
        Ok(c) => match c.blob.get(&key, 0, None).await {
            Ok(b) => {
                if let Err(e) = c.blob.delete(&key).await {
                    tracing::warn!(error = %e, token_prefix = prefix(token), "export blob delete failed");
                }
                b
            }
            Err(e) => return internal(&e, token),
        },
        Err(e) => return internal(&e, token),
    };
    tracing::info!(volume, token_prefix = prefix(token), outcome = "served", "export download");
    crate::api::dataplane::attachment(bytes, "application/zip", "export.zip")
}

/// Opportunistic cleanup of an expired, never downloaded link the request
/// hit (FR-NEW-011), independent of the background sweep.
///
/// Runs only after `claim` found no live row, so a row still present here is
/// necessarily expired. An empty `now` makes the same atomic primitive match
/// any `expires_at`, which avoids a second delete path in `storage::meta`.
/// Failures are logged only: the caller answers 404 regardless.
async fn reap_expired(state: &AppState, token: &str) {
    if !is_canonical(token) {
        return;
    }
    let projects = match state.admin.list_all_projects().await {
        Ok(p) => p,
        Err(e) => return tracing::warn!(error = %e, "export reap: project listing failed"),
    };
    for project in projects {
        match delete_row(state, &project.id, token, "").await {
            Ok(true) => {
                if let Ok(c) = state.stores.client(&project.id).await {
                    let _ = c.blob.delete(&format!("export:{token}")).await;
                }
                tracing::info!(
                    token_prefix = prefix(token),
                    outcome = "expired",
                    "export download"
                );
                return;
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, volume = %project.id, "export reap failed"),
        }
    }
    tracing::info!(token_prefix = prefix(token), outcome = "not_found", "export download");
}

async fn delete_row(state: &AppState, volume: &str, token: &str, now: &str) -> Result<bool> {
    let db = crate::storage::open_meta_db(&state.config, state.stores.relational(), volume).await?;
    let mut tx = db.begin().await?;
    let hit = crate::storage::meta::delete_export_link_if_live(&mut *tx, token, now).await?;
    tx.commit().await?;
    Ok(hit)
}

/// Only the exact lowercase hyphenated form `fs.export_zip` mints is a
/// token. Rejecting anything else up front keeps garbage off the database
/// and rules out a case insensitive collation matching an uppercase replay.
fn is_canonical(token: &str) -> bool {
    uuid::Uuid::parse_str(token).is_ok_and(|u| u.to_string() == token)
}

/// The full token is a bearer credential: logs only ever see its prefix.
fn prefix(token: &str) -> &str {
    token.get(..8).unwrap_or("")
}

fn internal(e: &crate::errors::ToolError, token: &str) -> Response {
    tracing::error!(error = %e, token_prefix = prefix(token), "export download failed");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::admin::test_support::Fixture;
    use axum::body::Body;
    use axum::http::{Request, header};
    use serde_json::Value;
    use std::io::Read as _;
    use tower::ServiceExt as _;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";
    const MAIN_RS: &[u8] = b"fn main() {}";
    const README: &[u8] = b"# Hello";

    async fn fixture() -> Fixture {
        let f = Fixture::new().await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/src/main.rs", MAIN_RS).await.unwrap();
        c.write_bytes_atomic("/docs/readme.md", README).await.unwrap();
        f
    }

    /// A fresh export of the story's two files, returning its token.
    async fn mint(f: &Fixture) -> String {
        let paths = vec!["/src/main.rs".to_string(), "/docs/readme.md".to_string()];
        let r = crate::tools::export::export_zip(&f.state, MOUNT, &paths).await.unwrap();
        r["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string()
    }

    async fn exec(f: &Fixture, sql: &str, binds: &[&str]) -> u64 {
        let db = crate::storage::open_meta_db(&f.state.config, f.state.stores.relational(), MOUNT)
            .await
            .unwrap();
        let mut tx = db.begin().await.unwrap();
        let mut q = crate::storage::rel::Query::new(sql);
        for b in binds {
            q = q.bind(*b);
        }
        let n = tx.execute(&q).await.unwrap();
        tx.commit().await.unwrap();
        n
    }

    /// Moves a link's `expires_at` to `now + offset_ms` (negative = past).
    async fn set_expiry(f: &Fixture, token: &str, offset_ms: i64) {
        let at = crate::tools::export::iso(
            chrono::Utc::now() + chrono::Duration::milliseconds(offset_ms),
        );
        let n =
            exec(f, "UPDATE export_links SET expires_at=?1 WHERE token=?2", &[&at, token]).await;
        assert_eq!(n, 1, "link must exist before its expiry is moved");
    }

    /// `SELECT COUNT(*)` equivalent, through a no-op update so it needs no
    /// row reader: zero rows touched means zero rows present.
    async fn row_count(f: &Fixture, token: &str) -> u64 {
        exec(f, "UPDATE export_links SET token=token WHERE token=?1", &[token]).await
    }

    async fn blob_present(f: &Fixture, token: &str) -> bool {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.blob.get(&format!("export:{token}"), 0, None).await.is_ok()
    }

    fn get_req(token: &str, headers: &[(&str, &str)]) -> Request<Body> {
        let mut b = Request::builder().uri(format!("/exports/{token}"));
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    async fn send(
        app: &Router,
        req: Request<Body>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let r = app.clone().oneshot(req).await.unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec();
        (status, headers, body)
    }

    async fn get(f: &Fixture, token: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        send(&router(f.state.clone()), get_req(token, &[])).await
    }

    fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let mut out: Vec<(String, Vec<u8>)> = (0..a.len())
            .map(|i| {
                let mut e = a.by_index(i).unwrap();
                let mut d = Vec::new();
                e.read_to_end(&mut d).unwrap();
                (e.name().to_string(), d)
            })
            .collect();
        out.sort();
        out
    }

    fn assert_story_zip(bytes: &[u8]) {
        assert_eq!(
            entries(bytes),
            [
                ("docs/readme.md".to_string(), README.to_vec()),
                ("src/main.rs".to_string(), MAIN_RS.to_vec())
            ]
        );
    }

    /// E2E-NEW-015 + E2E-NEW-023 + E2E-NEW-027: happy path.
    #[tokio::test]
    async fn e2e_new_015_happy_path_serves_the_zip_as_export_zip() {
        let f = fixture().await;
        let t = mint(&f).await;
        let (s, h, b) = get(&f, &t).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h[header::CONTENT_TYPE], "application/zip");
        let cd = h[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(cd.starts_with("attachment; filename=\"export.zip\""), "{cd}");
        assert_story_zip(&b);
    }

    /// E2E-NEW-016 + E2E-NEW-017: row and blob gone right after a 200.
    #[tokio::test]
    async fn e2e_new_016_017_download_deletes_row_and_blob() {
        let f = fixture().await;
        let t = mint(&f).await;
        assert_eq!(row_count(&f, &t).await, 1);
        assert!(blob_present(&f, &t).await);
        assert_eq!(get(&f, &t).await.0, StatusCode::OK);
        assert_eq!(row_count(&f, &t).await, 0);
        assert!(!blob_present(&f, &t).await);
    }

    /// E2E-NEW-018 + E2E-NEW-019 + E2E-NEW-052: no header, a garbage
    /// bearer, a real stranger's bearer, and a garbage forwarded header all
    /// download.
    #[tokio::test]
    async fn e2e_new_018_019_052_no_auth_is_required_or_checked() {
        let f = fixture().await;
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = crate::keys::write_keypair(dir.path()).unwrap();
        let stranger = crate::keys::mint_token_from_file(
            &key,
            "stranger@test.com",
            crate::keys::DEFAULT_ISSUER,
            crate::keys::DEFAULT_CLAIM,
            3600,
        )
        .unwrap();
        assert!(!f.state.admin.is_member(MOUNT, "stranger@test.com").await.unwrap());
        let stranger_bearer = format!("Bearer {stranger}");
        let app = router(f.state.clone());
        let cases: [&[(&str, &str)]; 4] = [
            &[],
            &[("Authorization", "Bearer garbage-token")],
            &[("Authorization", &stranger_bearer)],
            &[("X-Forwarded-Authorization", "Bearer garbage")],
        ];
        for headers in cases {
            let t = mint(&f).await;
            let (s, _, b) = send(&app, get_req(&t, headers)).await;
            assert_eq!(s, StatusCode::OK, "{headers:?}");
            assert_story_zip(&b);
        }
    }

    /// E2E-NEW-020 + E2E-NEW-021: unknown and malformed tokens are 404.
    #[tokio::test]
    async fn e2e_new_020_021_unknown_and_malformed_are_404() {
        let f = fixture().await;
        let (s, _, b) = get(&f, "00000000-0000-0000-0000-000000000000").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let text = String::from_utf8_lossy(&b);
        assert!(!text.contains("export_links") && !text.contains("expired"), "{text}");
        assert_eq!(get(&f, "not-a-valid-uuid-at-all").await.0, StatusCode::NOT_FOUND);
    }

    /// E2E-NEW-024 + E2E-NEW-025 + E2E-NEW-055: replay, plain and uppercased.
    #[tokio::test]
    async fn e2e_new_024_025_055_replay_is_an_identical_404() {
        let f = fixture().await;
        let (_, _, never) = get(&f, "00000000-0000-0000-0000-000000000000").await;
        let t = mint(&f).await;
        assert_eq!(get(&f, &t).await.0, StatusCode::OK);
        let (s, _, b) = get(&f, &t).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(b, never);
        assert_eq!(row_count(&f, &t).await, 0);
        assert_eq!(get(&f, &t.to_uppercase()).await.0, StatusCode::NOT_FOUND);
    }

    /// E2E-NEW-055 on a live token: the uppercase spelling is never accepted.
    #[tokio::test]
    async fn uppercase_spelling_of_a_live_token_is_404() {
        let f = fixture().await;
        let t = mint(&f).await;
        assert_eq!(get(&f, &t.to_uppercase()).await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&f, &t).await.0, StatusCode::OK);
    }

    /// E2E-NEW-026: never existed, expired, replayed give identical bodies.
    #[tokio::test]
    async fn e2e_new_026_three_404_causes_are_byte_identical() {
        let f = fixture().await;
        let t2 = mint(&f).await;
        set_expiry(&f, &t2, -1000).await;
        let t3 = mint(&f).await;
        assert_eq!(get(&f, &t3).await.0, StatusCode::OK);
        let r1 = get(&f, "11111111-1111-4111-8111-111111111111").await;
        let r2 = get(&f, &t2).await;
        let r3 = get(&f, &t3).await;
        for r in [&r1, &r2, &r3] {
            assert_eq!(r.0, StatusCode::NOT_FOUND);
        }
        assert_eq!(r1.2, r2.2);
        assert_eq!(r2.2, r3.2);
        assert_eq!(r1.1.get(header::CONTENT_LENGTH), r2.1.get(header::CONTENT_LENGTH));
        assert_eq!(r1.1.get(header::CONTENT_TYPE), r3.1.get(header::CONTENT_TYPE));
    }

    /// E2E-NEW-028 + E2E-NEW-029: expired is 404 and cleans row and blob now.
    #[tokio::test]
    async fn e2e_new_028_029_expired_is_404_and_cleaned_up_immediately() {
        let f = fixture().await;
        let t = mint(&f).await;
        set_expiry(&f, &t, -1000).await;
        assert_eq!(row_count(&f, &t).await, 1);
        assert!(blob_present(&f, &t).await);
        let (s, _, b) = get(&f, &t).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(b, get(&f, "00000000-0000-0000-0000-000000000000").await.2);
        assert_eq!(row_count(&f, &t).await, 0);
        assert!(!blob_present(&f, &t).await);
    }

    /// E2E-NEW-030: just before and just after the expiry instant.
    #[tokio::test]
    async fn e2e_new_030_boundary_before_and_after_expiry() {
        let f = fixture().await;
        let t = mint(&f).await;
        let t2 = mint(&f).await;
        set_expiry(&f, &t, 1000).await;
        set_expiry(&f, &t2, 100).await;
        assert_eq!(get(&f, &t).await.0, StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(get(&f, &t2).await.0, StatusCode::NOT_FOUND);
    }

    /// E2E-NEW-037 + E2E-NEW-049: two racing GETs, exactly one wins.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn e2e_new_037_049_two_racers_exactly_one_wins() {
        let f = fixture().await;
        let t = mint(&f).await;
        let app = router(f.state.clone());
        let (a, b) = tokio::join!(send(&app, get_req(&t, &[])), send(&app, get_req(&t, &[])));
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::NOT_FOUND]);
        let winner = if a.0 == StatusCode::OK { &a.2 } else { &b.2 };
        assert_story_zip(winner);
        assert_eq!(row_count(&f, &t).await, 0);
        assert!(!blob_present(&f, &t).await);
    }

    /// E2E-NEW-048 + E2E-NEW-049: three racing GETs, exactly one wins.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn e2e_new_048_049_three_racers_exactly_one_wins() {
        let f = fixture().await;
        let t = mint(&f).await;
        let app = router(f.state.clone());
        let (a, b, c) = tokio::join!(
            send(&app, get_req(&t, &[])),
            send(&app, get_req(&t, &[])),
            send(&app, get_req(&t, &[]))
        );
        let ok: Vec<_> = [&a, &b, &c].into_iter().filter(|r| r.0 == StatusCode::OK).collect();
        assert_eq!(ok.len(), 1);
        assert_story_zip(&ok[0].2);
        let lost = [&a, &b, &c].iter().filter(|r| r.0 == StatusCode::NOT_FOUND).count();
        assert_eq!(lost, 2);
        assert_eq!(row_count(&f, &t).await, 0);
    }

    /// E2E-NEW-038 + E2E-NEW-046: the claim commits before expiry, the
    /// expiry then genuinely passes while the bytes are being produced, and
    /// the response is still a full 200. Uses the handler's own two steps
    /// with an injected delay between them (the story's preferred seam).
    #[tokio::test]
    async fn e2e_new_038_046_in_flight_download_survives_expiry() {
        let f = fixture().await;
        let t = mint(&f).await;
        set_expiry(&f, &t, 50).await;
        let volume = claim(&f.state, &t).await.unwrap().expect("claim before expiry");
        let deadline = chrono::Utc::now() + chrono::Duration::milliseconds(50);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(chrono::Utc::now() > deadline, "the injected delay must cross expires_at");
        let r = serve(&f.state, &volume, &t).await;
        assert_eq!(r.status(), StatusCode::OK);
        let b = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        assert_story_zip(&b);
    }

    /// E2E-NEW-047: an expired token is rejected at once while an unrelated
    /// download is still in flight.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn e2e_new_047_expired_rejected_during_unrelated_in_flight_download() {
        let f = fixture().await;
        let live = mint(&f).await;
        let expired = mint(&f).await;
        set_expiry(&f, &expired, -1000).await;
        let state = f.state.clone();
        let live2 = live.clone();
        let in_flight = tokio::spawn(async move {
            let v = claim(&state, &live2).await.unwrap().unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            serve(&state, &v, &live2).await.status()
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let started = std::time::Instant::now();
        assert_eq!(get(&f, &expired).await.0, StatusCode::NOT_FOUND);
        assert!(!in_flight.is_finished(), "the live download must still be in flight");
        assert!(started.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(in_flight.await.unwrap(), StatusCode::OK);
    }

    /// E2E-NEW-054: no DELETE route; the token survives the attempt.
    #[tokio::test]
    async fn e2e_new_054_no_delete_route_exists() {
        let f = fixture().await;
        let t = mint(&f).await;
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/exports/{t}"))
            .body(Body::empty())
            .unwrap();
        let (s, _, _) = send(&router(f.state.clone()), req).await;
        assert!(s == StatusCode::NOT_FOUND || s == StatusCode::METHOD_NOT_ALLOWED, "{s}");
        assert_eq!(get(&f, &t).await.0, StatusCode::OK);
    }

    /// E2E-NEW-058: merged in the unconditional block of `app.rs`, before
    /// the `api.enabled` gate.
    #[test]
    fn e2e_new_058_router_is_merged_unconditionally() {
        let src = include_str!("app.rs");
        let merge = src.find(".merge(crate::exports::router(state.clone()))").expect("merged");
        let gate = src.find("if state.config.api.enabled {").unwrap();
        let screens = src.find(".merge(crate::trash_screen::router(state.clone()))").unwrap();
        assert!(merge < gate, "exports router must precede the api.enabled gate");
        let (lo, hi) = (merge.min(screens), merge.max(screens));
        assert!(!src[lo..hi].contains(';'), "must sit in the screen routers' builder chain");
    }

    // ── E2E-NEW-057 / E2E-NEW-059: a real server with api.enabled=false ────

    fn rpc(token: &str, session: Option<&str>, payload: &str) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("Host", "localhost")
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("X-Forwarded-Authorization", format!("Bearer {token}"));
        if let Some(s) = session {
            b = b.header("Mcp-Session-Id", s);
        }
        b.body(Body::from(payload.to_string())).unwrap()
    }

    /// Last JSON `data:` frame of an SSE body.
    fn sse_json(body: &[u8]) -> Value {
        String::from_utf8_lossy(body)
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
            .next_back()
            .unwrap()
    }

    /// `tools/call` over an established session, returning the tool's JSON.
    async fn call(app: &Router, token: &str, session: &str, name: &str, args: Value) -> Value {
        let payload = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": args}});
        let (s, _, b) = send(app, rpc(token, Some(session), &payload.to_string())).await;
        assert_eq!(s, StatusCode::OK);
        let v = sse_json(&b);
        assert_ne!(v["result"]["isError"], true, "{name} failed: {v}");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[tokio::test]
    async fn e2e_new_057_059_mcp_only_round_trip_downloads() {
        let d = tempfile::tempdir().unwrap();
        let (key, pub_path) = crate::keys::write_keypair(d.path().join("keys")).unwrap();
        let me = "me@test.com";
        let token = crate::keys::mint_token_from_file(
            &key,
            me,
            crate::keys::DEFAULT_ISSUER,
            crate::keys::DEFAULT_CLAIM,
            3600,
        )
        .unwrap();
        let mut c = crate::config::ServerConfig::default();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.auth.admins = vec![me.to_string()];
        c.infra.meta.dir = d.path().join("volumes").display().to_string();
        c.infra.blob.dir = d.path().join("blobs").display().to_string();
        c.infra.admin.path = d.path().join("admin.db").display().to_string();
        c.api.enabled = false;
        let app = crate::app::build(c).await.unwrap();

        // The REST plane really is off, so a 200 below can only come from
        // the unconditional mount.
        let probe = Request::builder().uri("/api/swagger.json").body(Body::empty()).unwrap();
        assert_eq!(send(&app, probe).await.0, StatusCode::NOT_FOUND);

        let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}"#;
        let r = app.clone().oneshot(rpc(&token, None, init)).await.unwrap();
        let session = r.headers()["Mcp-Session-Id"].to_str().unwrap().to_string();
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert_eq!(send(&app, rpc(&token, Some(&session), note)).await.0, StatusCode::ACCEPTED);

        let s = session.as_str();
        call(
            &app,
            &token,
            s,
            "admin.create_project",
            serde_json::json!({"project_id": MOUNT, "owner": me}),
        )
        .await;
        call(
            &app,
            &token,
            s,
            "fs.write",
            serde_json::json!({"mount_id": MOUNT, "path": "/src/main.rs", "content": "fn main() {}"}),
        )
        .await;
        let out = call(
            &app,
            &token,
            s,
            "fs.export_zip",
            serde_json::json!({"mount_id": MOUNT, "paths": ["/src/main.rs"]}),
        )
        .await;
        let url = out["url"].as_str().unwrap();
        assert!(url.starts_with("/exports/"), "{url}");

        let (st, h, b) = send(&app, Request::builder().uri(url).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(h[header::CONTENT_TYPE], "application/zip");
        assert_eq!(entries(&b), [("src/main.rs".to_string(), MAIN_RS.to_vec())]);
    }
}
