//! SPEC-0091 tests: a document decodes apart from the request threads, behind a
//! ceiling of as many places as the runtime has worker threads. Every test
//! drives the real handlers on a served app (`McpServer::fs_extract_text`,
//! `fs_list_dir`, `fs_read`, `GET /health` on the router) and times the probes
//! as the requests a member and the operator would send meanwhile.
//!
//! Timing: a probe must add at most 250 ms to its own answer time without a
//! decoding (FR-NEW-001). The test build is not optimized and the machine may
//! be loaded, so `probe_bound()` gives a debug build three times that, and the
//! limit is measured against the same probe answered before anything started
//! (`baseline`), never against zero. The waits this story removes are seconds
//! long, so the scaling does not hide them. Every test that decodes a 100 MB
//! document takes `heavy()` first (each can hold about 1 GB).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::config::ServerConfig;
use crate::keys;
use crate::mcp::server::{ExtractTextArgs, ListDirArgs, ListTablesArgs, McpServer, ReadArgs};
use crate::state::AppState;
use crate::tools::archive_testkit::{RawPart, deflate_run, heavy, office_zip, raw_part, zip_raw};
use crate::tools::pdf_testkit::{
    STREAM_LIMIT, content_bomb_pdf, large_content_pdf, page_pdf, plain, report_pdf,
};

const PROJECT: &str = "atlas";
const MIA: &str = "mia@example.com";
const NOA: &str = "noa@example.com";
const OLA: &str = "ola@example.com";
const EVE: &str = "eve@example.com";

/// The session write allowance of every member of the served app, so the
/// remaining allowance after an extraction is a literal.
const ALLOWANCE: i64 = 10_000_000;

/// Largest decoded size of one Office part (SPEC-0088).
const PART_LIMIT: usize = 104_857_600;

const MEMO_BODY: &str = r#"<w:document xmlns:w="w"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Results</w:t></w:r></w:p><w:p><w:r><w:t>Quarterly results</w:t></w:r></w:p></w:body></w:document>"#;
const MEMO_PREVIEW: &str = "# Results\n\nQuarterly results";
const PRESENTATION: &str = r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#;
const PRESENTATION_RELS: &str = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/slide" Target="slides/slide1.xml"/></Relationships>"#;
const INTRO: &str =
    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Intro</a:t></a:r></a:p></p:sld>"#;
const WORKBOOK: &str = r#"<workbook xmlns:r="r"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const WORKBOOK_S: &str = r#"<workbook xmlns:r="r"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const WORKBOOK_RELS: &str = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
const SHEET: &str = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>cell</t></is></c></row></sheetData></worksheet>"#;

/// The app of a test: the router, its state, and the directory that keeps it alive.
struct Served {
    router: Router,
    state: Arc<AppState>,
    _dir: tempfile::TempDir,
}

/// Build the app like `tool_surface_e2e::setup`, create `atlas` owned by mia
/// with noa and ola as members, `/notes/todo.txt` (`buy milk`) and an empty
/// `/docs`.
async fn served() -> Served {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (_key, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
    let mut c = ServerConfig::default();
    c.auth.jwt.public_key_path = pub_path.display().to_string();
    c.infra.meta.dir = root.join("volumes").display().to_string();
    c.infra.blob.dir = root.join("blobs").display().to_string();
    c.infra.admin.path = root.join("admin.db").display().to_string();
    c.search.tantivy_dir = root.join("tantivy").display().to_string();
    c.safety.write_quota_bytes = ALLOWANCE;
    let (router, state) = crate::app::build_with_state(c).await.unwrap();
    state.admin.create_project(PROJECT, MIA).await.unwrap();
    state.stores.provision_volume(PROJECT).await.unwrap();
    state.admin.add_member(PROJECT, NOA, MIA).await.unwrap();
    state.admin.add_member(PROJECT, OLA, MIA).await.unwrap();
    let client = state.stores.client(PROJECT).await.unwrap();
    client.makedirs("/notes", true).await.unwrap();
    client.makedirs("/docs", true).await.unwrap();
    client.write_bytes_atomic("/notes/todo.txt", b"buy milk").await.unwrap();
    Served { router, state, _dir: dir }
}

impl Served {
    async fn seed(&self, path: &str, bytes: &[u8]) {
        let client = self.state.stores.client(PROJECT).await.unwrap();
        client.write_bytes_atomic(path, bytes).await.unwrap();
    }

    async fn read(&self, path: &str) -> Option<Vec<u8>> {
        let client = self.state.stores.client(PROJECT).await.unwrap();
        if client.exists(path).await.unwrap() {
            Some(client.read_bytes(path).await.unwrap())
        } else {
            None
        }
    }

    /// The most extractions that decoded at once on this server.
    fn most_at_once(&self) -> usize {
        self.state.safety.decode_places().most_at_once()
    }

    /// The audit lines `person` has for extractions of `path` (the companion).
    fn extraction_lines(&self, person: &str, companion: &str) -> usize {
        let lines = self.state.safety.audit(person, PROJECT);
        lines.iter().filter(|e| e.op == "extract_text" && e.path == companion).count()
    }
}

/// The text of a tool answer, and whether it is an error.
fn answer_text(r: &rmcp::model::CallToolResult) -> (String, bool) {
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text block");
    (text, r.is_error == Some(true))
}

/// What a tool call answered: the JSON value, or the error text.
type Answer = std::result::Result<Value, String>;

fn parse(r: rmcp::model::CallToolResult) -> Answer {
    let (text, is_error) = answer_text(&r);
    if is_error { Err(text) } else { Ok(serde_json::from_str(&text).expect("a json answer")) }
}

/// `fs.extract_text` with the defaults, through the real handler.
async fn extract(state: Arc<AppState>, person: &str, path: &str) -> Answer {
    let server = McpServer::new(state, person.to_string());
    let args = ExtractTextArgs {
        mount_id: PROJECT.into(),
        path: path.into(),
        max_chars: 200_000,
        preview_chars: 4_000,
        ocr: false,
        refresh: false,
    };
    parse(server.fs_extract_text(Parameters(args)).await.expect("a tool result"))
}

/// Start an extraction on its own task, so several run "at the same moment".
fn ask(
    s: &Served,
    person: &'static str,
    path: &'static str,
) -> tokio::task::JoinHandle<(Answer, Instant)> {
    let state = s.state.clone();
    tokio::spawn(async move {
        let v = extract(state, person, path).await;
        (v, Instant::now())
    })
}

/// The answer of a joined extraction task.
async fn done(h: tokio::task::JoinHandle<(Answer, Instant)>) -> Answer {
    h.await.expect("the extraction task").0
}

/// What the probes send.
#[derive(Clone, Copy)]
enum Probe {
    /// The operator's `GET /health`.
    Health,
    /// noa lists `/notes` and sees `todo.txt`.
    List,
    /// noa reads `/notes/todo.txt` and gets `buy milk`.
    Read,
}

/// One probe due at `start`, asserted, and its answer time counted from `start`:
/// a probe that could not even begin before a worker was free is late by that wait.
async fn probe_once(
    router: &Router,
    state: &Arc<AppState>,
    kind: Probe,
    start: Instant,
) -> Duration {
    match kind {
        Probe::Health => {
            let req = Request::builder().uri("/health").body(Body::empty()).unwrap();
            let r = router.clone().oneshot(req).await.unwrap();
            let took = start.elapsed();
            assert_eq!(r.status(), StatusCode::OK);
            took
        }
        Probe::List => {
            let server = McpServer::new(state.clone(), NOA.to_string());
            let args = ListDirArgs {
                mount_id: PROJECT.into(),
                path: "/notes".into(),
                include_hidden: false,
                sort_by: "name".into(),
                with_sizes: false,
            };
            let r = server.fs_list_dir(Parameters(args)).await.expect("a tool result");
            let took = start.elapsed();
            let v = parse(r).expect("the listing answers");
            assert!(v.to_string().contains("todo.txt"), "{v}");
            took
        }
        Probe::Read => {
            let server = McpServer::new(state.clone(), NOA.to_string());
            let args = ReadArgs {
                mount_id: PROJECT.into(),
                path: "/notes/todo.txt".into(),
                offset_lines: 0,
                limit_lines: 10,
                line_numbered: false,
            };
            let r = server.fs_read(Parameters(args)).await.expect("a tool result");
            let took = start.elapsed();
            let v = parse(r).expect("the read answers");
            assert_eq!(v["content"], "buy milk", "{v}");
            took
        }
    }
}

/// One probe every 100 ms from 50 ms after the call, until `stop` says so;
/// the instant each was due and its answer time, in order.
fn probe_loop(
    s: &Served,
    kind: Probe,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<Vec<(Instant, Duration)>> {
    let (router, state) = (s.router.clone(), s.state.clone());
    tokio::spawn(async move {
        let mut times = Vec::new();
        let mut due = Instant::now() + Duration::from_millis(50);
        while !*stop.borrow() {
            // Biased: a probe that came due while the workers were busy is sent
            // (and counted late) even when the stop signal is ready too.
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(due.into()) => {}
                _ = stop.changed() => break,
            }
            times.push((due, probe_once(&router, &state, kind, due).await));
            due += Duration::from_millis(100);
        }
        times
    })
}

/// The most a probe may add to its answer time while documents decode: 250 ms
/// (FR-NEW-001); three times that in a debug build, which is not optimized and
/// shares the machine with the decoding threads.
fn probe_bound() -> Duration {
    if cfg!(debug_assertions) { Duration::from_millis(750) } else { Duration::from_millis(250) }
}

/// The answer time of `kind` with nothing decoding.
async fn baseline(s: &Served, kind: Probe) -> Duration {
    probe_once(&s.router, &s.state, kind, Instant::now()).await
}

/// Wait until `due` (a sleep on a runtime whose workers are all decoding wakes
/// late, which is why every wait is to an instant and every time counts from it).
async fn until(due: Instant) {
    tokio::time::sleep_until(due.into()).await;
}

/// One probe due at `due`, sent from a request task as a client's request
/// would be served, and its answer time counted from `due`.
async fn probe_at(s: &Served, kind: Probe, due: Instant) -> Duration {
    let (router, state) = (s.router.clone(), s.state.clone());
    until(due).await;
    tokio::spawn(async move { probe_once(&router, &state, kind, due).await })
        .await
        .expect("the probe task")
}

/// An extraction due at `due` on a request task, and the time its answer took from `due`.
async fn timed_ask(
    s: &Served,
    person: &'static str,
    path: &'static str,
    due: Instant,
) -> (Answer, Duration) {
    until(due).await;
    let (v, at) = ask(s, person, path).await.expect("the extraction task");
    (v, at - due)
}

/// The answer times of probes.
fn tooks(probed: &[(Instant, Duration)]) -> Vec<Duration> {
    probed.iter().map(|(_, took)| *took).collect()
}

/// Probes due while a decoding ran were answered before it ended, and at
/// least `min` were due then. A short decoding (the PDF bomb, a refused
/// workbook) ends within the time bound of a release build, so the bound alone
/// cannot tell a probe answered during the decoding from one that waited for
/// it; the order of the two answers can.
fn answered_during_decoding(s: &Served, probed: &[(Instant, Duration)], min: usize) {
    let windows = s.state.safety.decode_places().windows();
    let mut inside = 0;
    for (due, took) in probed {
        for (start, end) in &windows {
            let end = end.expect("every decoding ended");
            if *due >= *start && *due < end {
                inside += 1;
                assert!(
                    *due + *took < end,
                    "a probe due during a decoding waited for its end: {took:?}"
                );
            }
        }
    }
    assert!(inside >= min, "only {inside} probes were due during a decoding, wanted {min}");
}

/// Every probe was answered, at least `min` of them, each within the bound over `base`.
fn in_time(times: &[Duration], base: Duration, min: usize) {
    assert!(times.len() >= min, "only {} probes were answered: {times:?}", times.len());
    let limit = base + probe_bound();
    for t in times {
        assert!(
            *t < limit,
            "a probe took {t:?}, more than {limit:?} (baseline {base:?}): {times:?}"
        );
    }
}

/// Run `extractions` at once while `kind` probes, then return the answers, the
/// probes (due instant and answer time) and the instant the last extraction answered.
async fn under_probes(
    s: &Served,
    kind: Probe,
    extractions: Vec<(&'static str, &'static str)>,
) -> (Vec<Answer>, Vec<(Instant, Duration)>, Instant) {
    let (stop, rx) = tokio::sync::watch::channel(false);
    let probes = probe_loop(s, kind, rx);
    let handles: Vec<_> = extractions.into_iter().map(|(p, d)| ask(s, p, d)).collect();
    let mut answers = Vec::new();
    let mut end = Instant::now();
    for h in handles {
        let (v, at) = h.await.expect("the extraction task");
        answers.push(v);
        end = end.max(at);
    }
    stop.send(true).unwrap();
    (answers, probes.await.expect("the probe task"), end)
}

// Documents.

fn large_pdf() -> Vec<u8> {
    large_content_pdf(STREAM_LIMIT)
}

/// A Word document whose main part is one paragraph of the letter `b`, exactly `total` bytes.
fn letters_docx(total: usize) -> Vec<u8> {
    let (open, close) =
        ("<w:document><w:body><w:p><w:r><w:t>", "</w:t></w:r></w:p></w:body></w:document>");
    let (data, crc) = deflate_run(open, b'b', total - open.len() - close.len(), close);
    zip_raw(&[RawPart {
        name: "word/document.xml".into(),
        declared: total as u64,
        crc: crc.sum(),
        data,
    }])
}

/// A workbook whose one shared text is 1 000 000 times `letter` and whose sheet
/// `S` has `rows` rows, each a single cell referring to that text.
fn repeated_text(letter: u8, rows: usize) -> Vec<u8> {
    let (data, crc) = deflate_run("<sst><si><t>", letter, 1_000_000, "</t></si></sst>");
    let strings = RawPart {
        name: "xl/sharedStrings.xml".to_string(),
        declared: 12 + 1_000_000 + 15,
        crc: crc.sum(),
        data,
    };
    let row = r#"<row><c t="s"><v>0</v></c></row>"#;
    let sheet = format!("<worksheet><sheetData>{}</sheetData></worksheet>", row.repeat(rows));
    let doc = zip_raw(&[
        strings,
        raw_part("xl/workbook.xml", WORKBOOK_S.as_bytes()),
        raw_part("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        raw_part("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    assert!(doc.len() < 100_000, "the workbook is stored in under 100 000 bytes: {}", doc.len());
    doc
}

fn memo_docx() -> Vec<u8> {
    office_zip(&[("word/document.xml", MEMO_BODY.as_bytes())])
}

fn book_xlsx() -> Vec<u8> {
    office_zip(&[
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        ("xl/worksheets/sheet1.xml", SHEET.as_bytes()),
    ])
}

fn deck_pptx() -> Vec<u8> {
    office_zip(&[
        ("ppt/presentation.xml", PRESENTATION.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS.as_bytes()),
        ("ppt/slides/slide1.xml", INTRO.as_bytes()),
    ])
}

/// Seed `/docs/{a,b,c}.pdf` with the large PDF.
async fn seed_three_pdfs(s: &Served) {
    for name in ["a", "b", "c"] {
        s.seed(&format!("/docs/{name}.pdf"), &large_pdf()).await;
    }
}

/// The error text of a refused extraction: code and message.
fn refusal(v: &Answer, code: &str, message: &str) {
    let Err(text) = v else { panic!("the extraction must be refused: {v:?}") };
    let want = format!("{code}: {message}");
    assert!(text.contains(&want), "{text}");
}

/// The answer of the large PDF (SPEC-0089 E2E-007): truncated, only `b` after the page heading.
fn large_pdf_answer(v: &Answer, path: &str) {
    let v = v.as_ref().unwrap_or_else(|e| panic!("the extraction must succeed: {e}"));
    assert_eq!(v["path"], path, "{v}");
    assert_eq!(v["truncated"], json!(true), "{v}");
    let preview = v["preview"].as_str().unwrap();
    let text = preview.trim_start_matches("---\n*[Page 1]*\n\n");
    assert!(!text.is_empty() && text.chars().all(|c| c == 'b'), "{preview}");
}

/// The answer of a refused bomb PDF.
fn bomb_refusal(v: &Answer, path: &str) {
    refusal(
        v,
        "ERR_INVALID_ARGUMENT",
        &format!("could not extract {path}: a PDF stream decodes to more than 104857600 bytes"),
    );
}

// Tests.

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_001_health_check_during_a_large_pdf_extra() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    let base = baseline(&s, Probe::Health).await;

    let (answers, probed, _end) =
        under_probes(&s, Probe::Health, vec![(MIA, "/docs/large.pdf")]).await;

    in_time(&tooks(&probed), base, 2);
    answered_during_decoding(&s, &probed, 1);
    large_pdf_answer(&answers[0], "/docs/large.pdf");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_002_folder_listing_during_a_large_word_ex() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.docx", &letters_docx(PART_LIMIT)).await;
    let base = baseline(&s, Probe::List).await;

    let (answers, probed, _end) =
        under_probes(&s, Probe::List, vec![(MIA, "/docs/large.docx")]).await;

    in_time(&tooks(&probed), base, 2);
    answered_during_decoding(&s, &probed, 1);
    let v = answers[0].as_ref().expect("the large document extracts");
    assert_eq!(v["truncated"], json!(true), "{v}");
    let preview = v["preview"].as_str().unwrap();
    assert!(!preview.is_empty() && preview.chars().all(|c| c == 'b'), "{preview}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_003_every_place_taken_one_more_waiting_pr() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;
    let base = baseline(&s, Probe::Read).await;

    let (answers, probed, _end) = under_probes(
        &s,
        Probe::Read,
        vec![(MIA, "/docs/a.pdf"), (NOA, "/docs/b.pdf"), (OLA, "/docs/c.pdf")],
    )
    .await;

    in_time(&tooks(&probed), base, 2);
    answered_during_decoding(&s, &probed, 1);
    for (a, p) in answers.iter().zip(["/docs/a.pdf", "/docs/b.pdf", "/docs/c.pdf"]) {
        large_pdf_answer(a, p);
    }
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_004_a_pdf_refused_by_its_limit_while_prob() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/bomb.pdf", &content_bomb_pdf()).await;
    let base = baseline(&s, Probe::Health).await;

    let (answers, probed, _end) =
        under_probes(&s, Probe::Health, vec![(MIA, "/docs/bomb.pdf")]).await;

    bomb_refusal(&answers[0], "/docs/bomb.pdf");
    in_time(&tooks(&probed), base, 1);
    answered_during_decoding(&s, &probed, 1);
    assert!(s.read("/docs/bomb.md").await.is_none(), "no companion");
    assert_eq!(s.extraction_lines(MIA, "/docs/bomb.md"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_005_an_excel_workbook_refused_by_the_cont() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/long.xlsx", &repeated_text(b'd', 140)).await;
    let base = baseline(&s, Probe::List).await;

    let (answers, probed, _end) =
        under_probes(&s, Probe::List, vec![(MIA, "/docs/long.xlsx")]).await;

    refusal(
        &answers[0],
        "ERR_INVALID_ARGUMENT",
        "could not extract /docs/long.xlsx: the extracted content exceeds 268435456 bytes",
    );
    in_time(&tooks(&probed), base, 1);
    answered_during_decoding(&s, &probed, 1);
    assert!(s.read("/docs/long.md").await.is_none(), "no companion");
    assert_eq!(s.extraction_lines(MIA, "/docs/long.md"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_006_three_extractions_two_places() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;

    let (a, b, c) =
        (ask(&s, MIA, "/docs/a.pdf"), ask(&s, NOA, "/docs/b.pdf"), ask(&s, OLA, "/docs/c.pdf"));

    large_pdf_answer(&done(a).await, "/docs/a.pdf");
    large_pdf_answer(&done(b).await, "/docs/b.pdf");
    large_pdf_answer(&done(c).await, "/docs/c.pdf");
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_007_one_place_arrival_order() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/a.pdf", &large_pdf()).await;
    s.seed("/docs/b.pdf", &large_pdf()).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    let base = baseline(&s, Probe::Health).await;

    let t0 = Instant::now();
    let a = ask(&s, MIA, "/docs/a.pdf");
    until(t0 + Duration::from_millis(50)).await;
    let memo = ask(&s, NOA, "/docs/memo.docx");
    until(t0 + Duration::from_millis(100)).await;
    let b = ask(&s, OLA, "/docs/b.pdf");
    let health = probe_at(&s, Probe::Health, t0 + Duration::from_millis(200)).await;

    let ((a, a_at), (memo, memo_at), (b, b_at)) =
        (a.await.unwrap(), memo.await.unwrap(), b.await.unwrap());
    in_time(&[health], base, 1);
    assert!(
        a_at < memo_at && memo_at < b_at,
        "answered out of order: {a_at:?} {memo_at:?} {b_at:?}"
    );
    large_pdf_answer(&a, "/docs/a.pdf");
    large_pdf_answer(&b, "/docs/b.pdf");
    assert_eq!(memo.expect("memo")["preview"], MEMO_PREVIEW);
    assert_eq!(s.most_at_once(), 1);
    assert_eq!(
        s.state.safety.decode_places().started(),
        ["/docs/a.pdf", "/docs/memo.docx", "/docs/b.pdf"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_008_a_refused_decoding_frees_its_place() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/m.pdf", &content_bomb_pdf()).await;
    s.seed("/docs/n.pdf", &content_bomb_pdf()).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;

    let t0 = Instant::now();
    let (m, n) = (ask(&s, MIA, "/docs/m.pdf"), ask(&s, NOA, "/docs/n.pdf"));
    until(t0 + Duration::from_millis(50)).await;
    let memo = ask(&s, OLA, "/docs/memo.docx");

    bomb_refusal(&done(m).await, "/docs/m.pdf");
    bomb_refusal(&done(n).await, "/docs/n.pdf");
    assert_eq!(done(memo).await.expect("memo")["preview"], MEMO_PREVIEW);
    assert_eq!(s.most_at_once(), 2);
    assert!(s.read("/docs/m.md").await.is_none() && s.read("/docs/n.md").await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_018_an_up_to_date_companion_answers_durin() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    extract(s.state.clone(), MIA, "/docs/memo.docx").await.expect("memo extracts once");
    let t0 = Instant::now();
    let (a, b) = (ask(&s, NOA, "/docs/a.pdf"), ask(&s, OLA, "/docs/b.pdf"));

    let (memo, took) = timed_ask(&s, MIA, "/docs/memo.docx", t0 + Duration::from_millis(50)).await;
    let memo = memo.expect("memo from companion");

    assert!(took < probe_bound(), "the cached answer took {took:?}");
    assert_eq!(memo["cached"], json!(true), "{memo}");
    assert_eq!(memo["preview"], MEMO_PREVIEW);
    done(a).await.expect("a");
    done(b).await.expect("b");
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_021_not_a_member_during_a_full_ceiling() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    let places = s.state.safety.decode_places();
    let (a, b) = (ask(&s, MIA, "/docs/a.pdf"), ask(&s, NOA, "/docs/b.pdf"));
    in_flight(places, &["/docs/a.pdf", "/docs/b.pdf"]).await;

    let (eve, took) = timed_ask(&s, EVE, "/docs/memo.docx", Instant::now()).await;
    assert_eq!(places.running(), 2, "a decoding ended before eve was answered");

    assert!(took < probe_bound(), "the refusal took {took:?}");
    refusal(&eve, "ERR_FORBIDDEN", "'eve@example.com' is not a member of 'atlas'");
    done(a).await.expect("a");
    done(b).await.expect("b");
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_022_missing_path_during_a_full_ceiling() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;
    let places = s.state.safety.decode_places();
    let (a, b) = (ask(&s, MIA, "/docs/a.pdf"), ask(&s, NOA, "/docs/b.pdf"));
    in_flight(places, &["/docs/a.pdf", "/docs/b.pdf"]).await;

    let (none, took) = timed_ask(&s, OLA, "/docs/none.pdf", Instant::now()).await;
    assert_eq!(places.running(), 2, "a decoding ended before ola was answered");

    assert!(took < probe_bound(), "the refusal took {took:?}");
    refusal(&none, "ERR_NOT_FOUND", "not a file: /docs/none.pdf");
    done(a).await.expect("a");
    done(b).await.expect("b");
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_023_the_extraction_s_own_traces_are_uncha() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    let (answers, probed, _end) =
        under_probes(&s, Probe::Health, vec![(MIA, "/docs/large.pdf")]).await;
    assert!(answers[0].is_ok() && !probed.is_empty());

    let companion = s.read("/docs/large.md").await.expect("the companion");

    record(
        "large companion",
        &format!("{} {:?}", companion.len(), String::from_utf8_lossy(&companion[..40])),
    );
    assert_eq!(companion, recorded_large_companion().as_bytes());
    assert_eq!(s.extraction_lines(MIA, "/docs/large.md"), 1);
    assert_eq!(s.extraction_lines(NOA, "/docs/large.md"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_024_three_extractions_three_companions_th() {
    let _heavy = heavy().await;
    let s = served().await;
    seed_three_pdfs(&s).await;
    let (a, b, c) =
        (ask(&s, MIA, "/docs/a.pdf"), ask(&s, NOA, "/docs/b.pdf"), ask(&s, OLA, "/docs/c.pdf"));
    done(a).await.expect("a");
    done(b).await.expect("b");
    done(c).await.expect("c");

    for (person, md) in [(MIA, "/docs/a.md"), (NOA, "/docs/b.md"), (OLA, "/docs/c.md")] {
        let companion = s.read(md).await.unwrap_or_else(|| panic!("{md} exists"));
        assert_eq!(companion, recorded_large_companion().as_bytes(), "{md}");
        assert_eq!(s.extraction_lines(person, md), 1, "{person}");
        let remaining = s.state.safety.session_remaining(person, PROJECT);
        record("large remaining", &remaining.to_string());
        assert_eq!(remaining, ALLOWANCE - RECORDED_LARGE_CHARGE, "{person}");
    }
}

#[tokio::test]
async fn e2e_0091_025_companion_artifacts_and_charge_as_rec() {
    let s = served().await;
    s.seed("/docs/book.xlsx", &book_xlsx()).await;
    extract(s.state.clone(), MIA, "/docs/book.xlsx").await.expect("the workbook extracts");

    let server = McpServer::new(s.state.clone(), MIA.to_string());
    let args =
        ListTablesArgs { mount_id: PROJECT.into(), path: "/docs/book.xlsx".into(), marker: None };
    let tables = parse(server.fs_list_tables(Parameters(args)).await.expect("a tool result"))
        .expect("the table list");

    record("tables", &tables.to_string());
    assert_eq!(tables.to_string(), RECORDED_TABLES);
    let remaining = s.state.safety.session_remaining(MIA, PROJECT);
    record("book remaining", &remaining.to_string());
    assert_eq!(remaining, RECORDED_BOOK_REMAINING);
}

#[tokio::test]
async fn e2e_0091_016_every_format_answers_as_recorded() {
    let s = served().await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    s.seed("/docs/book.xlsx", &book_xlsx()).await;
    s.seed("/docs/deck.pptx", &deck_pptx()).await;
    s.seed("/docs/report.pdf", &report_pdf()).await;
    s.seed("/docs/page.html", "<h1>Title</h1><p>Body é</p>".as_bytes()).await;
    s.seed("/docs/data.csv", b"a,b\n1,2\n").await;
    s.seed("/docs/notes.txt", b"line one").await;
    s.seed("/docs/scan.png", b"\x89PNG\r\n\x1a\n").await;

    let mut seen = Vec::new();
    for (path, companion) in [
        ("/docs/memo.docx", Some("/docs/memo.md")),
        ("/docs/book.xlsx", Some("/docs/book.md")),
        ("/docs/deck.pptx", Some("/docs/deck.md")),
        ("/docs/report.pdf", Some("/docs/report.md")),
        ("/docs/page.html", Some("/docs/page.md")),
        ("/docs/data.csv", Some("/docs/data.md")),
        ("/docs/notes.txt", None),
        ("/docs/scan.png", None),
    ]
    .into_iter()
    {
        let v = extract(s.state.clone(), MIA, path).await.unwrap_or_else(|e| panic!("{e}"));
        let written = match companion {
            Some(c) => s.read(c).await.map(|b| String::from_utf8(b).unwrap()),
            None => None,
        };
        record(&format!("answer {path}"), &v.to_string());
        record(&format!("companion {path}"), &format!("{written:?}"));
        seen.push((v.to_string(), written));
    }
    for (i, (answer, written)) in seen.iter().enumerate() {
        let (want, md) = RECORDED[i];
        assert_eq!(answer, want, "format {i}");
        assert_eq!(written.as_deref(), md, "format {i}");
    }
}

// A reader that stops, a requester who gives up (US-0229).

const STOPPED: &str =
    "could not extract /docs/report.pdf: the document reader stopped unexpectedly";

/// A one page PDF reading `Revenue`.
fn revenue_pdf() -> Vec<u8> {
    page_pdf(plain("BT /F1 12 Tf 72 700 Td (Revenue) Tj ET"), "", vec![])
}

/// Nothing the extraction of `doc` for `person` could leave is there: no
/// companion, no artifact, no audit line naming the document or its
/// companion, and the whole session allowance is still free (a write of that
/// size succeeds).
async fn nothing_left(s: &Served, person: &str, doc: &str) {
    let md = crate::docs::extract::companion_md_path(doc);
    assert!(s.read(&md).await.is_none(), "{md} must not exist");
    for listed in [
        crate::tools::artifacts::list_tables(&s.state, PROJECT, doc, None).await,
        crate::tools::artifacts::list_images(&s.state, PROJECT, doc, None).await,
    ] {
        let e = listed.expect_err("no artifact may be listed");
        assert_eq!(
            (e.code, e.message.as_str()),
            (crate::errors::code::NOT_FOUND, crate::docs::artifacts::NO_ARTIFACTS)
        );
    }
    let log = s.state.safety.audit(person, PROJECT);
    assert!(!log.iter().any(|e| e.path == doc || e.path == md), "{log:?}");
    assert_eq!(s.state.safety.session_remaining(person, PROJECT), ALLOWANCE);
    let client = s.state.stores.client(PROJECT).await.unwrap();
    crate::core::fs_ops::write_bytes(
        &client,
        &s.state.safety,
        person,
        PROJECT,
        "/docs/fill.bin",
        &vec![b'w'; ALLOWANCE as usize],
        true,
        true,
    )
    .await
    .expect("nothing was charged");
}

/// Wait until the decodings of `paths` (and only those) have started and are
/// all still running: an event, so a fast machine cannot let a test act after
/// a decoding it meant to interrupt has already ended.
pub(crate) async fn in_flight(places: &crate::docs::extract::DecodePlaces, paths: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(300);
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let want = sorted(paths.iter().map(|p| p.to_string()).collect());
    while sorted(places.started()) != want || places.running() != paths.len() {
        assert!(Instant::now() < deadline, "never in flight: {:?}", places.started());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Wait until `n` calls have asked for a place, granted or waiting.
async fn asked(s: &Served, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(300);
    while s.state.safety.decode_places().asked() < n {
        assert!(Instant::now() < deadline, "only {} asked", s.state.safety.decode_places().asked());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Wait until every decoding the places saw has ended.
async fn all_decodings_ended(s: &Served) {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let windows = s.state.safety.decode_places().windows();
        if !windows.is_empty() && windows.iter().all(|(_, end)| end.is_some()) {
            return;
        }
        assert!(Instant::now() < deadline, "a decoding never ended");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn e2e_0091_009_a_reader_stops_unexpectedly() {
    let s = served().await;
    s.seed("/docs/report.pdf", &revenue_pdf()).await;
    s.state.safety.decode_places().inject_fault("/docs/report.pdf");

    let v = extract(s.state.clone(), MIA, "/docs/report.pdf").await;

    refusal(&v, "ERR_INTERNAL_ERROR", STOPPED);
    nothing_left(&s, MIA, "/docs/report.pdf").await;
}

/// What the operator reads after an unexpected stop: one error line naming the
/// code and the path, no audit line, and a health check answered healthy. The
/// capture records on the thread that took the lock, so the whole body runs on
/// that one thread.
#[test]
fn e2e_0091_010_what_the_operator_sees_after_an_unexp() {
    let _capture = crate::logging::capture::lock_for_test();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let s = served().await;
        s.seed("/docs/report.pdf", &revenue_pdf()).await;
        s.state.safety.decode_places().inject_fault("/docs/report.pdf");
        let base = baseline(&s, Probe::Health).await;
        crate::logging::capture::clear();

        let v = extract(s.state.clone(), MIA, "/docs/report.pdf").await;

        refusal(&v, "ERR_INTERNAL_ERROR", STOPPED);
        let events = crate::logging::capture::events();
        let errors: Vec<_> = events
            .iter()
            .filter(|e| {
                let message = &e.fields["message"];
                e.fields["level"] == "ERROR"
                    && message.contains("ERR_INTERNAL_ERROR")
                    && message.contains("/docs/report.pdf")
            })
            .collect();
        assert_eq!(errors.len(), 1, "{events:?}");
        assert_eq!(events.iter().filter(|e| e.fields["level"] == "ERROR").count(), 1, "{events:?}");
        let log = s.state.safety.audit(MIA, PROJECT);
        assert!(!log.iter().any(|e| e.path.contains("report")), "{log:?}");
        let health = probe_at(&s, Probe::Health, Instant::now()).await;
        in_time(&[health], base, 1);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_0091_012_one_reader_stops_another_extraction_r() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/report.pdf", &revenue_pdf()).await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    s.state.safety.decode_places().inject_fault("/docs/report.pdf");

    let t0 = Instant::now();
    let noa = ask(&s, NOA, "/docs/large.pdf");
    let (mia, took) = timed_ask(&s, MIA, "/docs/report.pdf", t0 + Duration::from_millis(50)).await;
    refusal(&mia, "ERR_INTERNAL_ERROR", STOPPED);
    let refused = Instant::now();
    assert!(took < probe_bound(), "the refusal took {took:?}");
    let ola = ask(&s, OLA, "/docs/memo.docx");
    until(refused + Duration::from_millis(50)).await;

    let (ola, ola_at) = ola.await.unwrap();
    assert_eq!(ola.expect("memo")["preview"], MEMO_PREVIEW);
    large_pdf_answer(&noa.await.unwrap().0, "/docs/large.pdf");
    // Ola was answered while noa's decoding still ran: a place the stopped
    // reader did not give back would have made memo wait for it.
    let places = s.state.safety.decode_places();
    let at = places.started().iter().position(|p| p == "/docs/large.pdf").unwrap();
    let large_end = places.windows()[at].1.expect("noa's decoding ended");
    assert!(ola_at < large_end, "ola waited {:?} for noa's decoding", ola_at - large_end);
    assert_eq!(s.most_at_once(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_013_mia_gives_up_mid_decoding() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;

    let places = s.state.safety.decode_places();
    let mia = ask(&s, MIA, "/docs/large.pdf");
    in_flight(places, &["/docs/large.pdf"]).await;
    mia.abort();
    assert_eq!(places.running(), 1, "the decoding ended before mia gave up");
    all_decodings_ended(&s).await;

    assert!(mia.await.unwrap_err().is_cancelled());
    assert_eq!(s.state.safety.decode_places().started(), ["/docs/large.pdf"]);
    nothing_left(&s, MIA, "/docs/large.pdf").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_014_an_abandoned_decoding_keeps_its_place() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    s.seed("/docs/memo.docx", &memo_docx()).await;
    let base = baseline(&s, Probe::Health).await;

    let places = s.state.safety.decode_places();
    let mia = ask(&s, MIA, "/docs/large.pdf");
    in_flight(places, &["/docs/large.pdf"]).await;
    mia.abort();
    assert_eq!(places.running(), 1, "the decoding ended before mia gave up");
    let due = Instant::now() + Duration::from_millis(100);
    let noa = timed_ask(&s, NOA, "/docs/memo.docx", due);
    let health = probe_at(&s, Probe::Health, due);
    let ((memo, took), health) = tokio::join!(noa, health);

    let memo_at = due + took;
    assert_eq!(memo.expect("memo")["preview"], MEMO_PREVIEW);
    let (start, end) = s.state.safety.decode_places().windows()[0];
    let end = end.expect("the large decoding ended");
    // The story's "no earlier than 4 seconds" is a machine speed figure (this
    // decoding takes about 3.3 s here); what matters is that the decoding was
    // still running when noa asked, and noa waited for its end.
    assert!(end > due && start < due, "the decoding was not running when noa asked");
    assert!(memo_at >= end, "memo answered {:?} before the decoding ended", end - memo_at);
    assert_eq!(s.most_at_once(), 1);
    in_time(&[health], base, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_015_asking_again_after_giving_up() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    let places = s.state.safety.decode_places();
    let mia = ask(&s, MIA, "/docs/large.pdf");
    in_flight(places, &["/docs/large.pdf"]).await;
    mia.abort();
    assert_eq!(places.running(), 1, "the decoding ended before mia gave up");
    all_decodings_ended(&s).await;

    let again = extract(s.state.clone(), MIA, "/docs/large.pdf").await;

    large_pdf_answer(&again, "/docs/large.pdf");
    assert_eq!(again.as_ref().unwrap()["cached"], json!(false), "{again:?}");
    assert_eq!(s.read("/docs/large.md").await.unwrap(), recorded_large_companion().as_bytes());
    assert_eq!(s.extraction_lines(MIA, "/docs/large.md"), 1);
}

#[tokio::test]
async fn e2e_0091_026_the_same_document_after_the_fault_is() {
    let recorded = served().await;
    recorded.seed("/docs/report.pdf", &revenue_pdf()).await;
    let want = extract(recorded.state.clone(), MIA, "/docs/report.pdf").await.expect("recorded");
    let s = served().await;
    s.seed("/docs/report.pdf", &revenue_pdf()).await;
    let places = s.state.safety.decode_places();
    places.inject_fault("/docs/report.pdf");
    refusal(
        &extract(s.state.clone(), MIA, "/docs/report.pdf").await,
        "ERR_INTERNAL_ERROR",
        STOPPED,
    );
    places.clear_faults();

    let v = extract(s.state.clone(), MIA, "/docs/report.pdf").await.expect("after the fault");

    assert_eq!(v, want);
    assert!(v["preview"].as_str().unwrap().contains("Revenue"), "{v}");
    assert!(s.read("/docs/report.md").await.is_some());
    assert_eq!(s.extraction_lines(MIA, "/docs/report.md"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn e2e_0091_028_giving_up_while_waiting_for_a_place() {
    let _heavy = heavy().await;
    let s = served().await;
    s.seed("/docs/large.pdf", &large_pdf()).await;
    s.seed("/docs/c.pdf", &large_pdf()).await;
    s.seed("/docs/report.pdf", &revenue_pdf()).await;
    let places = s.state.safety.decode_places();

    let noa = ask(&s, NOA, "/docs/large.pdf");
    in_flight(places, &["/docs/large.pdf"]).await;
    let mia = ask(&s, MIA, "/docs/c.pdf");
    asked(&s, 2).await;
    mia.abort();
    assert_eq!(places.running(), 1, "noa's decoding ended before mia gave up");
    large_pdf_answer(&done(noa).await, "/docs/large.pdf");
    // Places are granted in arrival order: had mia's wait survived the abort,
    // c.pdf would start before ola's report, so the order is the event.
    extract(s.state.clone(), OLA, "/docs/report.pdf").await.expect("ola's report");

    assert!(mia.await.unwrap_err().is_cancelled());
    assert_eq!(s.most_at_once(), 1);
    assert_eq!(places.started(), ["/docs/large.pdf", "/docs/report.pdf"]);
    nothing_left(&s, MIA, "/docs/c.pdf").await;
}

// Recorded on the code before SPEC-0091 (US-0227's tree), DEC-009.

fn record(label: &str, value: &str) {
    if std::env::var_os("MCPFS_PRINT_RECORD").is_some() {
        eprintln!("RECORD {label}: {value}");
    }
}

fn recorded_large_companion() -> String {
    format!("---\n*[Page 1]*\n\n{}", "b".repeat(199_984))
}

const RECORDED_LARGE_CHARGE: i64 = 400_000;
const RECORDED_TABLES: &str = r#"{"tables":[{"id":"table-1","caption":"Data","rows":0,"columns":1,"csv_quality":"CSV quality: exact"}],"marker":null}"#;
const RECORDED_BOOK_REMAINING: i64 = 9_999_912;
const RECORDED: [(&str, Option<&str>); 8] = [
    (
        r###"{"path":"/docs/memo.docx","md_path":"/docs/memo.md","format":"docx","chars":28,"cached":false,"preview":"# Results\n\nQuarterly results","truncated":false,"meta":{"paragraphs":2},"note":"","artifacts_report":"0 images, 0 tables"}"###,
        Some("# Results\n\nQuarterly results"),
    ),
    (
        r###"{"path":"/docs/book.xlsx","md_path":"/docs/book.md","format":"xlsx","chars":32,"cached":false,"preview":"## Sheet: Data\n\n| cell |\n| --- |","truncated":false,"meta":{"sheets":1},"note":"","artifacts_report":"0 images, 1 tables"}"###,
        Some("## Sheet: Data\n\n| cell |\n| --- |"),
    ),
    (
        r###"{"path":"/docs/deck.pptx","md_path":"/docs/deck.md","format":"pptx","chars":17,"cached":false,"preview":"## Slide 1\n\nIntro","truncated":false,"meta":{"slides":1},"note":"","artifacts_report":"0 images, 0 tables"}"###,
        Some("## Slide 1\n\nIntro"),
    ),
    (
        r###"{"path":"/docs/report.pdf","md_path":"/docs/report.md","format":"pdf","chars":58,"cached":false,"preview":"---\n*[Page 1]*\n\nQuarterly results\n\n---\n*[Page 2]*\n\nOutlook","truncated":false,"meta":{"pages":2,"title":"Report 2026"},"note":"","artifacts_report":"0 images, 0 tables"}"###,
        Some("---\n*[Page 1]*\n\nQuarterly results\n\n---\n*[Page 2]*\n\nOutlook"),
    ),
    (
        r###"{"path":"/docs/page.html","md_path":"/docs/page.md","format":"html","chars":11,"cached":false,"preview":"TitleBody é","truncated":false,"meta":{},"note":"","artifacts_report":"0 images, 0 tables"}"###,
        Some("TitleBody é"),
    ),
    (
        r###"{"path":"/docs/data.csv","md_path":"/docs/data.md","format":"csv","chars":33,"cached":false,"preview":"| a | b |\n| --- | --- |\n| 1 | 2 |","truncated":false,"meta":{"rows":2},"note":"","artifacts_report":"0 images, 1 tables"}"###,
        Some("| a | b |\n| --- | --- |\n| 1 | 2 |"),
    ),
    (
        r###"{"path":"/docs/notes.txt","md_path":null,"format":"text","chars":8,"cached":false,"preview":"line one","truncated":false,"meta":{},"note":""}"###,
        None,
    ),
    (
        r###"{"path":"/docs/scan.png","md_path":null,"format":"image","chars":0,"cached":false,"preview":"","truncated":false,"meta":{},"note":"image: no OCR text. Configure extract.ocr.provider=multimodal to enable image understanding","artifacts_report":"0 images, 0 tables"}"###,
        None,
    ),
];
