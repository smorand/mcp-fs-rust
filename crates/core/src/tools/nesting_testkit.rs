//! Helpers of the SPEC-0092 nesting tests: the child process harness (DEC-008),
//! the bystander project `orbit` (DEC-010) and the PDF builders (DEC-012). The
//! SPEC-0088 and SPEC-0089 kits are imported read only (DEC-011).

use crate::mcp::server::{McpServer, ReadArgs};
use crate::state::AppState;
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive_testkit::office_fixture;
use crate::tools::pdf_testkit::{PdfObj, bomb, pages_writer, plain};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ContentBlock;
use std::future::Future;
use std::io::Read as _;
use std::os::unix::process::ExitStatusExt as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const CHILD_ENV: &str = "MCPFS_CHILD_CASE";
const PASSED: &str = "MCPFS_CHILD_CASE_PASSED";
/// The member who reads `/notes/hello.txt` of `orbit` while an attack runs.
pub(crate) const BYSTANDER: &str = "leo@example.com";
const CHILD_LIMIT: Duration = Duration::from_secs(120);

/// Run `body` in a child process of this test binary, on the runtime shape of
/// production (2 workers, 2 MiB thread stacks), so a stack overflow that ends
/// the process fails this assertion instead of the test runner. In the child
/// (`MCPFS_CHILD_CASE` equal to `name`) it runs `body` and prints a marker; in
/// the parent it re runs the binary for this one test and checks a normal exit
/// and the marker. Time: bounded by `CHILD_LIMIT`; the child is killed past it.
pub(crate) fn in_child<F, Fut>(module: &str, name: &str, body: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if std::env::var(CHILD_ENV).as_deref() == Ok(name) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_stack_size(2 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("the child runtime");
        runtime.block_on(body());
        println!("{PASSED} {name}");
        return;
    }
    let exe = std::env::current_exe().expect("the test binary");
    let mut child = std::process::Command::new(exe)
        .args(["--exact", &format!("{module}{name}"), "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, name)
        .env_remove("RUST_MIN_STACK")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the child");
    // Drained on threads: a full pipe would otherwise block the child.
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the child") {
            break status;
        }
        if start.elapsed() > CHILD_LIMIT {
            let _ = child.kill();
            break child.wait().expect("reap the child");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = err.join().unwrap_or_default();
    let tail = String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(4096)..]).into_owned();
    let how = match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "no status".to_string(),
    };
    assert!(
        status.success() && stdout.contains(&format!("{PASSED} {name}")),
        "the child did not exit normally: {how}; stderr tail:\n{tail}"
    );
}

/// [`office_fixture`] plus the project `orbit` of [`BYSTANDER`] holding `/notes/hello.txt`.
pub(crate) async fn nesting_fixture() -> Fixture {
    let f = office_fixture().await;
    f.seed_project("orbit", BYSTANDER).await;
    let c = f.state.stores.client("orbit").await.expect("orbit volume");
    c.makedirs("/notes", true).await.expect("notes folder");
    c.write_bytes_atomic("/notes/hello.txt", b"hello").await.expect("hello note");
    f
}

/// One read of the note, through the real handler, as the bystander.
async fn read_hello(state: &Arc<AppState>) {
    let server = McpServer::new(state.clone(), BYSTANDER.to_string());
    let args = ReadArgs {
        mount_id: "orbit".into(),
        path: "/notes/hello.txt".into(),
        offset_lines: 0,
        limit_lines: 10,
        line_numbered: false,
    };
    let r = tokio::time::timeout(Duration::from_secs(1), server.fs_read(Parameters(args)))
        .await
        .expect("the bystander read answers within 1 s")
        .expect("a tool result");
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text block");
    assert_ne!(r.is_error, Some(true), "{text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("a json answer");
    assert_eq!(v["content"], "hello", "{text}");
}

/// The process still answers: same pid, and the bystander reads `hello` within 1 s.
pub(crate) async fn still_up(f: &Fixture, pid_before: u32) {
    assert_eq!(std::process::id(), pid_before, "the process was replaced");
    read_hello(&f.state).await;
}

/// Read the note every 50 ms until `stop` turns true; returns the number of reads.
pub(crate) async fn bystander_loop(
    state: Arc<AppState>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> usize {
    let mut reads = 0;
    while !*stop.borrow() {
        read_hello(&state).await;
        reads += 1;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            _ = stop.changed() => {}
        }
    }
    reads
}

const FORM_HEAD: &str = "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1]";

/// A form stream: `resources` is the inside of its `/Resources << >>` (None: no entry).
pub(crate) fn form(resources: Option<&str>, content: &str) -> PdfObj {
    let res = resources.map(|r| format!(" /Resources << {r} >>")).unwrap_or_default();
    PdfObj::Stream(
        format!("{FORM_HEAD}{res} /Length {} >>", content.len()),
        content.as_bytes().to_vec(),
    )
}

const SHOW: &str = "BT /F1 12 Tf 72 600 Td";

/// E2E-001: the page draws `/A`, which draws `/B`, which draws `/C`, which shows `Deep text`.
pub(crate) fn forms_pdf() -> Vec<u8> {
    let mut w = pages_writer(
        vec![plain("BT /F1 12 Tf 72 700 Td (Cover) Tj ET /A Do")],
        "/XObject << /A 10 0 R >>",
    );
    w.object(10, form(Some("/XObject << /B 11 0 R >> /Font << /F1 5 0 R >>"), "/B Do"));
    w.object(11, form(Some("/XObject << /C 12 0 R >> /Font << /F1 5 0 R >>"), "/C Do"));
    w.object(12, form(Some("/Font << /F1 5 0 R >>"), &format!("{SHOW} (Deep text) Tj ET")));
    w.finish()
}

/// `forms` forms in a chain from object 10, the page drawing the first, the last showing `bottom`.
pub(crate) fn forms_chain_pdf(forms: u32, bottom: &str) -> Vec<u8> {
    chain_writer(forms, form(Some("/Font << /F1 5 0 R >>"), &format!("{SHOW} ({bottom}) Tj ET")))
        .finish()
}

fn chain_writer(forms: u32, last: PdfObj) -> crate::tools::pdf_testkit::PdfWriter {
    let mut w = pages_writer(vec![plain("/A Do")], "/XObject << /A 10 0 R >>");
    for n in 10..10 + forms - 1 {
        w.object(n, form(Some(&format!("/XObject << /A {} 0 R >>", n + 1)), "/A Do"));
    }
    w.object(10 + forms - 1, last);
    w
}

/// The objects of E2E-SEC-001: object 10 draws itself through its own resources.
fn loop_writer() -> crate::tools::pdf_testkit::PdfWriter {
    let mut w = pages_writer(
        vec![plain("BT /F1 12 Tf 72 700 Td (Hello) Tj ET /A Do")],
        "/XObject << /A 10 0 R >>",
    );
    w.object(10, form(Some("/XObject << /A 10 0 R >>"), "/A Do"));
    w
}

/// E2E-SEC-001: object 10 draws itself through its own resources.
pub(crate) fn loop_pdf() -> Vec<u8> {
    loop_writer().finish()
}

/// E2E-019: [`loop_pdf`] plus an object stream (number 20) of 2 000 000 pairs `1 0` over one
/// `null`, stored in under 1 000 000 bytes: the load refuses it before any page is walked.
pub(crate) fn both_pdf() -> Vec<u8> {
    let mut w = loop_writer();
    w.object_stream_pairs(20, (1, 0), 2_000_000, "null");
    crate::tools::pdf_testkit::small(w.finish())
}

/// E2E-020: page 1 is a PDF bomb stream, page 2 draws the self drawing form (object 10).
pub(crate) fn two_pages_bomb_then_loop_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![bomb(""), plain("/A Do")], "/XObject << /A 10 0 R >>");
    w.object(10, form(Some("/XObject << /A 10 0 R >>"), "/A Do"));
    crate::tools::pdf_testkit::small(w.finish())
}

/// DEC-013: the page content is `defect` then `/A Do` (the self drawing form, object 10);
/// `resources` is the page's extra resource entries, `extra` the objects the defect needs.
pub(crate) fn parity_pdf(defect: &str, resources: &str, extra: Vec<(u32, PdfObj)>) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(&format!("{defect} /A Do"))], resources);
    w.object(10, form(Some("/XObject << /A 10 0 R >>"), "/A Do"));
    for (n, obj) in extra {
        w.object(n, obj);
    }
    w.finish()
}

/// The page resources of every parity document that needs no more than `/A`.
pub(crate) const PARITY_A: &str = "/XObject << /A 10 0 R >>";

/// E2E-SEC-002: as [`loop_pdf`] but the form has no resources and takes the page's.
pub(crate) fn inherit_pdf() -> Vec<u8> {
    let mut w = pages_writer(
        vec![plain("BT /F1 12 Tf 72 700 Td (Hello) Tj ET /A Do")],
        "/XObject << /A 10 0 R >>",
    );
    w.object(10, form(None, "/A Do"));
    w.finish()
}

/// E2E-SEC-003: `/A` (object 10) and `/B` (object 11) draw each other.
pub(crate) fn pair_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![plain("/A Do")], "/XObject << /A 10 0 R >>");
    w.object(10, form(Some("/XObject << /B 11 0 R >>"), "/B Do"));
    w.object(11, form(Some("/XObject << /A 10 0 R >>"), "/A Do"));
    w.finish()
}

/// E2E-008: the page draws `/A` `times` times, side by side.
pub(crate) fn repeat_pdf(times: usize) -> Vec<u8> {
    let content = vec!["/A Do"; times].join(" ");
    let mut w = pages_writer(vec![plain(&content)], "/XObject << /A 10 0 R >>");
    w.object(10, form(Some("/Font << /F1 5 0 R >>"), &format!("{SHOW} (Tile) Tj ET")));
    w.finish()
}

/// Design test: 65 forms in a chain whose last (object 74) is a 200 000 000 byte stream bomb.
pub(crate) fn deep_bomb_pdf() -> Vec<u8> {
    chain_writer(65, bomb("")).finish()
}

const HELLO: &str = "BT /F1 12 Tf 72 700 Td (Hello) Tj ET";
const OWN_RESOURCES: &str = "/Resources << /Font << /F1 5 0 R >> >>";

/// E2E-006, E2E-013, E2E-SEC-006: a page with neither resources nor page size, under
/// `groups` page groups (objects 100 up), the last holding both.
pub(crate) fn parent_chain_pdf(groups: u32) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(HELLO)], "");
    w.object(3, PdfObj::Body("<< /Type /Page /Parent 100 0 R /Contents 4 0 R >>".to_string()));
    for n in 100..100 + groups - 1 {
        w.object(n, PdfObj::Body(format!("<< /Type /Pages /Parent {} 0 R >>", n + 1)));
    }
    w.object(
        100 + groups - 1,
        PdfObj::Body(
            "<< /Type /Pages /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] >>"
                .to_string(),
        ),
    );
    w.finish()
}

/// E2E-009, E2E-017, E2E-021, E2E-SEC-005: the page's parent (object 100) is its own parent.
/// The page keeps its own resources when `resources`, its own page size when `media_box`.
pub(crate) fn parent_loop_pdf(resources: bool, media_box: bool, content: &str) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(content)], "");
    let res = if resources { format!(" {OWN_RESOURCES}") } else { String::new() };
    let size = if media_box { " /MediaBox [0 0 612 792]" } else { "" };
    w.object(
        3,
        PdfObj::Body(format!("<< /Type /Page /Parent 100 0 R /Contents 4 0 R{res}{size} >>")),
    );
    w.object(100, PdfObj::Body("<< /Type /Pages /Parent 100 0 R >>".to_string()));
    w.finish()
}

/// The text the climb tests show.
pub(crate) const HELLO_TEXT: &str = HELLO;

const CMAP_HEAD: &str = "/CIDInit /ProcSet findresource begin begincmap /X ";
const CMAP_TAIL: &str = " def endcmap end";

/// The body of a character map: `open` written `n` times, `close` written `n` times, between
/// the head and the tail of a code map.
pub(crate) fn nested_map(open: &str, close: &str, n: usize) -> String {
    format!("{CMAP_HEAD}{}{}{CMAP_TAIL}", open.repeat(n), close.repeat(n))
}

/// E2E-007, E2E-014, E2E-SEC-007 to SEC-009: font 5 is a Type1 font whose `ToUnicode` is object 9,
/// a plain stream holding `body`; the page shows `Hello`.
pub(crate) fn cmap_pdf(body: &str) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(HELLO)], "");
    w.object(
        5,
        PdfObj::Body(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>".to_string(),
        ),
    );
    w.object(9, plain(body));
    w.finish()
}

/// E2E-SEC-010: font 5 is a composite font whose `Encoding` is object 9, a plain stream holding `body`.
pub(crate) fn type0_pdf(body: &str) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(HELLO)], "");
    w.object(
        5,
        PdfObj::Body(
            "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding 9 0 R /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X >>] >>"
                .to_string(),
        ),
    );
    w.object(9, plain(body));
    w.finish()
}

/// E2E-022: the page draws the self drawing form `/A` (object 10) before it selects `/F2`
/// (font 6), whose `ToUnicode` map nests 1 000 lists.
pub(crate) fn order_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![plain("/A Do BT /F2 12 Tf (x) Tj ET")], "");
    w.object(
        3,
        PdfObj::Body(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /A 10 0 R >> /Font << /F1 5 0 R /F2 6 0 R >> >> /Contents 4 0 R >>"
                .to_string(),
        ),
    );
    w.object(
        6,
        PdfObj::Body(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>".to_string(),
        ),
    );
    w.object(9, plain(&nested_map("[", "]", 1_000)));
    w.object(10, form(Some("/XObject << /A 10 0 R >>"), "/A Do"));
    w.finish()
}

/// A source file of `head`, then `open` written `n` times, `middle`, `close` written `n`
/// times and `tail` (E2E-003 to E2E-SEC-013). Time and space: O(n).
pub(crate) fn deep_source(
    head: &str,
    open: char,
    middle: &str,
    close: char,
    n: usize,
    tail: &str,
) -> String {
    let mut s = String::with_capacity(head.len() + 2 * n + middle.len() + tail.len());
    s.push_str(head);
    s.extend(std::iter::repeat_n(open, n));
    s.push_str(middle);
    s.extend(std::iter::repeat_n(close, n));
    s.push_str(tail);
    s
}
