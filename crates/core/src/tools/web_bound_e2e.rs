//! SPEC-0093: the remote answers of the web and documentation families are read
//! under a bound. Driven through the MCP route of the served app (DEC-009)
//! against a raw TCP stand in site (DEC-008); the bound is read on two channels
//! besides the answer: the stand in's record and the held hook (DEC-003).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::tools::archive_testkit::heavy;
use crate::tools::tool_surface_e2e::{call_tool, success_payload};
use crate::tools::web::{ANSWER_LIMIT, answer_held, answer_held_reset};
use crate::tools::web_testkit::{FLOOD_BYTES, Record, Script, served, served_with_base, stand_in};

fn refusal_text(v: &Value) -> String {
    assert!(v["error"].is_null(), "the tool is served, so it is dispatched: {v}");
    let shown: String = v.to_string().chars().take(300).collect();
    assert_eq!(v["result"]["isError"], Value::Bool(true), "the call must be refused: {shown}");
    v["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{v}")).to_string()
}

fn assert_over_limit(v: &Value) {
    let text = refusal_text(v);
    assert!(text.contains("ERR_INVALID_ARGUMENT"), "{text}");
    assert!(text.contains("response from host '127.0.0.1' exceeds the 16 MiB limit"), "{text}");
}

async fn wait_closed_early(rec: &Record) {
    for _ in 0..100 {
        if rec.closed_early.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the stand in never saw the connection close early");
}

fn fetch_args(port: u16, path: &str) -> (String, Value) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let args = json!({"url": url});
    (url, args)
}

// ── SPEC-0093/E2E-SEC-004 ───────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_sec_004_page_fetch_flood_originating_occu() {
    let _heavy = heavy().await;
    let (app, token, _state, _d) = served(true, None).await;
    let (port, rec) =
        stand_in(vec![("/flood", Script::Flood { endless: false, encoding: None })]).await;
    let (url, args) = fetch_args(port, "/flood");
    answer_held_reset();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    assert_over_limit(&v);
    let held = answer_held(&url);
    assert!(held.iter().all(|h| *h <= 17_825_792), "the fetch held {held:?}");
    assert!(
        held.iter().any(|h| *h > ANSWER_LIMIT),
        "the refusal must count what it read: {held:?}"
    );
    wait_closed_early(&rec).await;
    let sent = rec.body_sent.load(Ordering::SeqCst);
    assert!(sent < FLOOD_BYTES, "the stand in sent the whole flood: {sent}");
}

// ── SPEC-0093/E2E-001 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_001_page_fetch_within_the_limit() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, _rec) = stand_in(vec![(
        "/page",
        Script::Fixed {
            body: b"<p>Hello &amp; welcome</p>".to_vec(),
            content_type: Some("text/html".into()),
            chunked: false,
            encoding: None,
        },
    )])
    .await;
    let (url, args) = fetch_args(port, "/page");
    answer_held_reset();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    assert_eq!(success_payload(&v), Value::String("Hello & welcome".into()));
    let held = answer_held(&url);
    assert!(held.iter().all(|h| *h <= 1_048_576), "{held:?}");
    assert!(held.contains(&26), "the guard counts the 26 bytes read: {held:?}");
}

// ── SPEC-0093/E2E-006 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_006_fetch_answer_of_exactly_the_answer_li() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, _rec) = stand_in(vec![(
        "/exact",
        Script::Letters { letter: b'b', n: ANSWER_LIMIT, chunked: true },
    )])
    .await;
    let (url, args) = fetch_args(port, "/exact");
    answer_held_reset();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    assert_eq!(success_payload(&v), Value::String("b".repeat(50_000)));
    let held = answer_held(&url);
    assert!(held.iter().all(|h| *h <= 17_825_792), "{held:?}");
    assert!(held.contains(&ANSWER_LIMIT), "the whole answer was read: {held:?}");
}

// ── SPEC-0093/E2E-011 ───────────────────────────────────────────────────────

/// A real gzip stream of zeros, padded with zero bytes to exactly 100 000 bytes:
/// it would expand to about 90 MB (a gzip stream cannot expand a 100 000 byte body
/// to a gigabyte, the ratio tops out near 1032), which is the point of the test.
fn gzip_bomb() -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let zeros = vec![0u8; 1_048_576];
    for _ in 0..90 {
        enc.write_all(&zeros).unwrap();
    }
    let mut out = enc.finish().unwrap();
    assert!(out.len() <= 100_000, "the bomb must fit in 100 000 bytes: {}", out.len());
    out.resize(100_000, 0);
    out
}

#[tokio::test]
async fn e2e_0093_011_compressed_answer_counted_as_received() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, _rec) = stand_in(vec![(
        "/zip",
        Script::Fixed {
            body: gzip_bomb(),
            content_type: Some("text/plain".into()),
            chunked: false,
            encoding: Some("gzip".into()),
        },
    )])
    .await;
    let (url, args) = fetch_args(port, "/zip");
    answer_held_reset();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    let text = success_payload(&v);
    assert!(text.as_str().unwrap().chars().count() <= 50_000);
    let held = answer_held(&url);
    assert!(held.iter().all(|h| *h <= 1_148_576), "{held:?}");
    assert!(held.contains(&100_000), "counted as received: {held:?}");
}

// ── SPEC-0093/E2E-013 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_013_fetch_answer_one_byte_over_chunked() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, _rec) = stand_in(vec![(
        "/over",
        Script::Letters { letter: b'b', n: ANSWER_LIMIT + 1, chunked: true },
    )])
    .await;
    let (url, args) = fetch_args(port, "/over");
    answer_held_reset();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    assert_over_limit(&v);
    let held = answer_held(&url);
    assert!(held.iter().all(|h| *h <= 17_825_792), "{held:?}");
    assert!(held.iter().any(|h| *h > ANSWER_LIMIT), "the refusal counts what it read: {held:?}");
}

// ── SPEC-0093/E2E-014 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_014_fetch_answer_declaring_one_byte_over() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, rec) = stand_in(vec![(
        "/declared",
        Script::Declared {
            length: ANSWER_LIMIT + 1,
            pause: Duration::from_secs(10),
            then: Box::new(Script::Letters { letter: b'b', n: ANSWER_LIMIT + 1, chunked: false }),
        },
    )])
    .await;
    let (url, args) = fetch_args(port, "/declared");
    answer_held_reset();
    let started = Instant::now();

    let v = call_tool(&app, &token, "web.fetch", args).await;

    assert_over_limit(&v);
    assert!(started.elapsed() < Duration::from_secs(5), "the refusal waited for the body");
    wait_closed_early(&rec).await;
    assert_eq!(rec.body_sent.load(Ordering::SeqCst), 0);
    assert_eq!(answer_held(&url), vec![0]);
}

// ── SPEC-0093/E2E-030 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_030_answer_trickling_until_the_time_limit() {
    let (app, token, _state, _d) = served(true, None).await;
    let (port, _rec) =
        stand_in(vec![("/trickle", Script::Trickle { per_second: 1, seconds: 60 })]).await;
    let url = format!("http://127.0.0.1:{port}/trickle");
    let started = Instant::now();

    let v = call_tool(&app, &token, "web.fetch", json!({"url": url, "timeout_secs": 2})).await;

    let text = refusal_text(&v);
    assert!(text.contains("ERR_INTERNAL_ERROR"), "{text}");
    assert!(text.contains("read response: "), "{text}");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

// ── SPEC-0093/US-0236: search, news and suggestions ────────────────────────

/// The served app with the three search hosts pointed at a stand in site.
async fn search_served(
    routes: Vec<(&'static str, Script)>,
) -> (axum::Router, String, std::sync::Arc<Record>, tempfile::TempDir) {
    let (port, rec) = stand_in(routes).await;
    let (app, token, _state, d) =
        served_with_base(true, None, Some(format!("http://127.0.0.1:{port}"))).await;
    (app, token, rec, d)
}

/// The held figure of the one answer the call read: the refusal must be bounded
/// above and must count what it read from below, and the site must not have
/// sent the whole flood.
fn assert_flood_held(rec: &Record) {
    let urls: Vec<String> =
        crate::tools::web::ANSWER_HELD.with(|h| h.borrow().iter().map(|r| r.url.clone()).collect());
    assert_eq!(urls.len(), 1, "one answer was read: {urls:?}");
    let held = answer_held(&urls[0]);
    assert!(held.iter().all(|h| *h <= 17_825_792), "the answer held {held:?}");
    assert!(
        held.iter().any(|h| *h > ANSWER_LIMIT),
        "the refusal must count what it read: {held:?}"
    );
    let sent = rec.body_sent.load(Ordering::SeqCst);
    assert!(sent < FLOOD_BYTES, "the stand in sent the whole flood: {sent}");
}

#[tokio::test]
async fn e2e_0093_sec_001_search_answer_flood() {
    let _heavy = heavy().await;
    let (app, token, rec, _d) =
        search_served(vec![("/html/", Script::Flood { endless: false, encoding: None })]).await;
    answer_held_reset();

    let v = call_tool(&app, &token, "web.search", json!({"query": "rust"})).await;

    assert_over_limit(&v);
    wait_closed_early(&rec).await;
    assert_flood_held(&rec);
}

#[tokio::test]
async fn e2e_0093_sec_002_news_answer_flood() {
    let _heavy = heavy().await;
    let (app, token, rec, _d) = search_served(vec![(
        "/news.js",
        Script::Prefixed { prefix: b"{\"results\":[".to_vec(), letter: b'a', n: FLOOD_BYTES },
    )])
    .await;
    answer_held_reset();

    let v = call_tool(&app, &token, "web.news", json!({"query": "launch"})).await;

    assert_over_limit(&v);
    wait_closed_early(&rec).await;
    assert_flood_held(&rec);
}

#[tokio::test]
async fn e2e_0093_sec_003_suggestions_answer_flood() {
    let _heavy = heavy().await;
    let (app, token, rec, _d) =
        search_served(vec![("/ac/", Script::Flood { endless: false, encoding: None })]).await;
    answer_held_reset();

    let v = call_tool(&app, &token, "web.suggestions", json!({"query": "ru"})).await;

    assert_over_limit(&v);
    wait_closed_early(&rec).await;
    assert_flood_held(&rec);
}

// ── SPEC-0093/E2E-002 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_002_search_within_the_limit() {
    let page = "<a class=\"result__a\" href=\"https://www.rust-lang.org/\">Rust</a>\
<a class=result__snippet>A language</a>\
<a class=\"result__a\" href=\"https://tokio.rs/\">Tokio</a>\
<a class=result__snippet>A runtime</a>";
    let (app, token, _rec, _d) = search_served(vec![(
        "/html/",
        Script::Fixed {
            body: page.as_bytes().to_vec(),
            content_type: Some("text/html".into()),
            chunked: false,
            encoding: None,
        },
    )])
    .await;

    let v =
        call_tool(&app, &token, "web.search", json!({"query": "rust", "max_results": 10})).await;

    let text = success_payload(&v);
    let results: Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
    assert_eq!(
        results,
        json!([
            {"title": "Rust", "url": "https://www.rust-lang.org/", "snippet": "A language"},
            {"title": "Tokio", "url": "https://tokio.rs/", "snippet": "A runtime"},
        ])
    );
}

// ── SPEC-0093/E2E-007 ───────────────────────────────────────────────────────

fn padded_list(head: &str, total: u64) -> Vec<u8> {
    let mut body = head.as_bytes().to_vec();
    body.resize(total as usize, b' ');
    body
}

#[tokio::test]
async fn e2e_0093_007_news_answer_of_exactly_the_answer_lim() {
    let head = r#"{"results":[{"title":"Launch","url":"https://x.test/","excerpt":"","date":""}]}"#;
    let (app, token, _rec, _d) = search_served(vec![(
        "/news.js",
        Script::Fixed {
            body: padded_list(head, ANSWER_LIMIT),
            content_type: Some("application/json".into()),
            chunked: true,
            encoding: None,
        },
    )])
    .await;

    let v = call_tool(&app, &token, "web.news", json!({"query": "launch"})).await;

    let text = success_payload(&v);
    let items: Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["title"], "Launch");
}

// ── SPEC-0093/E2E-015 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_015_suggestions_answer_over_the_limit() {
    let (app, token, _rec, _d) = search_served(vec![(
        "/ac/",
        Script::Fixed {
            body: padded_list(r#"["ru",["rust","ruby"]]"#, ANSWER_LIMIT + 1),
            content_type: Some("application/json".into()),
            chunked: true,
            encoding: None,
        },
    )])
    .await;

    let v = call_tool(&app, &token, "web.suggestions", json!({"query": "ru"})).await;

    assert_over_limit(&v);
}

// ── SPEC-0093/E2E-035 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_035_search_answer_declaring_a_length_over() {
    let (app, token, rec, _d) = search_served(vec![(
        "/html/",
        Script::Declared {
            length: 20_000_000,
            pause: Duration::from_secs(10),
            then: Box::new(Script::Letters { letter: b'b', n: 20_000_000, chunked: false }),
        },
    )])
    .await;
    let started = Instant::now();

    let v = call_tool(&app, &token, "web.search", json!({"query": "rust"})).await;

    assert_over_limit(&v);
    assert!(started.elapsed() < Duration::from_secs(5), "the refusal waited for the body");
    wait_closed_early(&rec).await;
    assert_eq!(rec.body_sent.load(Ordering::SeqCst), 0);
}

// ── SPEC-0093/US-0237: library resolution and documentation ────────────────

/// The served app with the documentation family pointed at a stand in site.
async fn library_served(
    routes: Vec<(&'static str, Script)>,
) -> (axum::Router, String, std::sync::Arc<Record>, tempfile::TempDir) {
    let (port, rec) = stand_in(routes).await;
    let (app, token, _state, d) = served(false, Some(format!("http://127.0.0.1:{port}"))).await;
    (app, token, rec, d)
}

/// Every held figure recorded, whatever the URL: the flood tests read one answer.
fn all_held() -> Vec<u64> {
    crate::tools::web::ANSWER_HELD.with(|h| h.borrow().iter().map(|r| r.held).collect())
}

fn assert_library_flood_held(rec: &Record) {
    let held = all_held();
    assert!(!held.is_empty(), "no answer was read");
    assert!(held.iter().all(|h| *h <= 17_825_792), "the answer held {held:?}");
    assert!(
        held.iter().any(|h| *h > ANSWER_LIMIT),
        "the refusal must count what it read: {held:?}"
    );
    let sent = rec.body_sent.load(Ordering::SeqCst);
    assert!(sent < FLOOD_BYTES, "the stand in sent the whole flood: {sent}");
}

#[tokio::test]
async fn e2e_0093_sec_008_library_resolution_flood() {
    let _heavy = heavy().await;
    let (app, token, rec, _d) =
        library_served(vec![("/v1/search", Script::Flood { endless: false, encoding: None })])
            .await;
    answer_held_reset();

    let v =
        call_tool(&app, &token, "context7.resolve_library_id", json!({"library_name": "tokio"}))
            .await;

    assert_over_limit(&v);
    wait_closed_early(&rec).await;
    assert_library_flood_held(&rec);
}

#[tokio::test]
async fn e2e_0093_sec_009_library_documentation_flood() {
    let _heavy = heavy().await;
    let (app, token, rec, _d) = library_served(vec![(
        "/v1/tokio-rs/tokio",
        Script::Flood { endless: false, encoding: None },
    )])
    .await;
    answer_held_reset();

    let v = call_tool(
        &app,
        &token,
        "context7.get_library_docs",
        json!({"library_id": "/tokio-rs/tokio"}),
    )
    .await;

    assert_over_limit(&v);
    wait_closed_early(&rec).await;
    assert_library_flood_held(&rec);
}

// ── SPEC-0093/E2E-003 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_003_library_documentation_within_the_limi() {
    let mut body = b"# Tokio docs\n".to_vec();
    body.extend(std::iter::repeat_n(b't', 10_000));
    let expected = String::from_utf8(body.clone()).unwrap();
    let (app, token, _rec, _d) = library_served(vec![(
        "/v1/tokio-rs/tokio",
        Script::Fixed {
            body,
            content_type: Some("text/markdown".into()),
            chunked: false,
            encoding: None,
        },
    )])
    .await;
    answer_held_reset();

    let v = call_tool(
        &app,
        &token,
        "context7.get_library_docs",
        json!({"library_id": "/tokio-rs/tokio"}),
    )
    .await;

    let text = success_payload(&v);
    assert_eq!(text.as_str().unwrap().len(), 10_013);
    assert_eq!(text, Value::String(expected));
    assert_eq!(all_held(), vec![10_013]);
}

// ── SPEC-0093/E2E-016 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_016_library_resolution_answer_over_the_li() {
    let (app, token, _rec, _d) = library_served(vec![(
        "/v1/search",
        Script::Letters { letter: b'a', n: ANSWER_LIMIT + 1, chunked: true },
    )])
    .await;

    let v =
        call_tool(&app, &token, "context7.resolve_library_id", json!({"library_name": "tokio"}))
            .await;

    assert_over_limit(&v);
}

// ── SPEC-0093/E2E-032 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_032_documentation_failure_status_with_a_h() {
    let (app, token, rec, _d) = library_served(vec![(
        "/v1/tokio-rs/tokio",
        Script::Status { code: 500, declared: FLOOD_BYTES },
    )])
    .await;
    answer_held_reset();

    let v = call_tool(
        &app,
        &token,
        "context7.get_library_docs",
        json!({"library_id": "/tokio-rs/tokio"}),
    )
    .await;

    let text = refusal_text(&v);
    assert!(text.contains("ERR_INTERNAL_ERROR"), "{text}");
    assert!(text.contains("context7 error: 500 Internal Server Error"), "{text}");
    assert_eq!(all_held(), vec![0]);
    assert_eq!(rec.requests.load(Ordering::SeqCst), 1);
}

// ── SPEC-0093/E2E-036 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_036_documentation_answer_declaring_one_by() {
    let (app, token, rec, _d) = library_served(vec![(
        "/v1/tokio-rs/tokio",
        Script::Declared {
            length: ANSWER_LIMIT + 1,
            pause: Duration::from_secs(10),
            then: Box::new(Script::Letters { letter: b'b', n: ANSWER_LIMIT + 1, chunked: false }),
        },
    )])
    .await;
    answer_held_reset();
    let started = Instant::now();

    let v = call_tool(
        &app,
        &token,
        "context7.get_library_docs",
        json!({"library_id": "/tokio-rs/tokio"}),
    )
    .await;

    assert_over_limit(&v);
    assert!(started.elapsed() < Duration::from_secs(5), "the refusal waited for the body");
    wait_closed_early(&rec).await;
    assert_eq!(rec.body_sent.load(Ordering::SeqCst), 0);
    assert_eq!(all_held(), vec![0]);
}

// ── SPEC-0093/E2E-038 ───────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_0093_038_documentation_family_off() {
    let (port, rec) = stand_in(vec![(
        "/v1/tokio-rs/tokio",
        Script::Letters { letter: b'a', n: 10, chunked: false },
    )])
    .await;
    let (app, token, _state, _d) = served(true, None).await;
    assert!(port > 0);

    let v = call_tool(
        &app,
        &token,
        "context7.get_library_docs",
        json!({"library_id": "/tokio-rs/tokio"}),
    )
    .await;

    crate::tools::tool_surface_e2e::assert_tool_not_found(&v);
    assert_eq!(rec.requests.load(Ordering::SeqCst), 0);
}
