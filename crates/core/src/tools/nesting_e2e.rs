//! SPEC-0092 tests: a PDF nesting that would overflow the reader's stack is
//! refused by the SPEC-0089 walk before the reader runs. Each exploit test runs
//! its attacked call in a child process (`in_child`), so a pre fix abort shows
//! as a failed assertion, and reads the absence of effect on second channels:
//! `nothing_left`, the bystander reading `hello`, the same pid.

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use crate::tools::nesting_testkit::*;
use crate::tools::pdf_testkit::{
    PDF_BOMB, PdfObj, STREAM_LIMIT, decoded_held, flate, pages_writer, pdf_held_reset, plain,
    zlib_run,
};
use serde_json::Value;
use std::time::{Duration, Instant};

const MODULE: &str = "tools::nesting_e2e::";

/// The form depth refusal of page 1 of `doc`.
fn depth_refusal(v: &Result<Value>, doc: &str) {
    let Err(e) = v else { panic!("the extraction must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!("could not extract {doc}: page 1 draws PDF forms nested more than 64 deep")
    );
}

/// Store `bytes` at `doc`, extract it while the bystander reads, then check the
/// refusal (within `limit` when given), nothing left and the server still up.
/// The spec's bound is 2 seconds (SPEC-0092 NFR). The test build does not optimize
/// the PDF crates and the machine may be loaded, so a debug build gets five times
/// that: the point is that a 10 000 form chain is refused in bounded time, not
/// after the 2 MiB stack is gone.
fn refusal_bound() -> Duration {
    if cfg!(debug_assertions) { Duration::from_secs(10) } else { Duration::from_secs(2) }
}

async fn exploit(doc: &str, bytes: Vec<u8>, limit: Option<Duration>) {
    let f = nesting_fixture().await;
    seed(&f, doc, &bytes).await;
    let pid_before = std::process::id();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let bystander = tokio::spawn(bystander_loop(f.state.clone(), rx));

    let start = Instant::now();
    let v = extract_text(&f, OWNER, doc, false).await;
    let took = start.elapsed();

    stop.send(true).expect("the bystander listens");
    let reads = bystander.await.expect("every bystander read answered hello");
    assert!(reads >= 1, "the bystander never read");
    depth_refusal(&v, doc);
    if let Some(limit) = limit {
        assert!(took < limit, "the refusal took {took:?}, more than {limit:?}");
    }
    nothing_left(&f, doc, None).await;
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_sec_001_a_form_that_draws_itself() {
    in_child(MODULE, "e2e_0092_sec_001_a_form_that_draws_itself", || {
        exploit("/docs/loop.pdf", loop_pdf(), None)
    });
}

#[test]
fn e2e_0092_sec_002_a_form_inheriting_the_resources_t() {
    in_child(MODULE, "e2e_0092_sec_002_a_form_inheriting_the_resources_t", || {
        exploit("/docs/inherit.pdf", inherit_pdf(), None)
    });
}

#[test]
fn e2e_0092_sec_003_two_forms_drawing_each_other() {
    in_child(MODULE, "e2e_0092_sec_003_two_forms_drawing_each_other", || {
        exploit("/docs/pair.pdf", pair_pdf(), None)
    });
}

#[test]
fn e2e_0092_sec_004_a_chain_of_10_000_distinct_forms() {
    in_child(MODULE, "e2e_0092_sec_004_a_chain_of_10_000_distinct_forms", || async {
        let chain = forms_chain_pdf(10_000, "Deep");
        assert!(chain.len() > 1_000_000, "the chain is about 1 700 000 bytes: {}", chain.len());
        exploit("/docs/chain.pdf", chain, Some(refusal_bound())).await
    });
}

#[tokio::test]
async fn e2e_0092_001_pdf_with_three_nested_forms() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/forms.pdf", &forms_pdf()).await;

    let v = extract_text(&f, OWNER, "/docs/forms.pdf", false).await.expect("three forms extract");

    assert_eq!(v["md_path"], "/docs/forms.md");
    assert_eq!(v["format"], "pdf");
    assert_eq!(v["meta"]["pages"], 1);
    assert_eq!(v["cached"], false);
    let preview = v["preview"].as_str().unwrap();
    let at = |s: &str| preview.find(s).unwrap_or_else(|| panic!("{s} missing from {preview}"));
    assert!(at("*[Page 1]*") < at("Cover") && at("Cover") < at("Deep text"), "{preview}");
    let md = read(&f, "/docs/forms.md").await.expect("the companion");
    assert!(String::from_utf8_lossy(&md).contains("Deep text"));
    let lines = f.state.safety.audit(OWNER, MOUNT);
    let lines = lines.iter().filter(|e| e.op == "extract_text" && e.path == "/docs/forms.md");
    assert_eq!(lines.count(), 1);
}

#[tokio::test]
async fn e2e_0092_005_forms_nested_exactly_64_deep() {
    let f = nesting_fixture().await;
    let pid_before = std::process::id();
    seed(&f, "/docs/deep64.pdf", &forms_chain_pdf(64, "Bottom")).await;

    let v = extract_text(&f, OWNER, "/docs/deep64.pdf", false).await.expect("64 forms extract");

    assert!(v["preview"].as_str().unwrap().contains("Bottom"), "{v}");
    still_up(&f, pid_before).await;
}

#[tokio::test]
async fn e2e_0092_008_one_form_drawn_50_times_side_by_side() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/repeat.pdf", &repeat_pdf(50)).await;

    let v = extract_text(&f, OWNER, "/docs/repeat.pdf", false).await.expect("50 draws extract");

    assert_eq!(v["preview"].as_str().unwrap().matches("Tile").count(), 50, "{v}");
}

#[tokio::test]
async fn e2e_0092_012_forms_nested_65_deep() {
    let f = nesting_fixture().await;
    let pid_before = std::process::id();
    seed(&f, "/docs/deep65.pdf", &forms_chain_pdf(65, "Bottom")).await;

    let v = extract_text(&f, OWNER, "/docs/deep65.pdf", false).await;

    depth_refusal(&v, "/docs/deep65.pdf");
    nothing_left(&f, "/docs/deep65.pdf", None).await;
    still_up(&f, pid_before).await;
}

/// DEC-003: the 65th form is refused before it is measured.
#[tokio::test]
async fn form_depth_is_checked_before_the_form_is_measured() {
    let _heavy = heavy().await;
    let f = nesting_fixture().await;
    seed(&f, "/docs/deepbomb.pdf", &deep_bomb_pdf()).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/deepbomb.pdf", false).await;

    depth_refusal(&v, "/docs/deepbomb.pdf");
    let held = decoded_held("/docs/deepbomb.pdf");
    assert!(held < STREAM_LIMIT as u64, "the bomb was measured: {held} bytes held");
}

/// The page tree refusal of page 1 of `doc`.
fn climb_refusal(v: &Result<Value>, doc: &str) {
    let Err(e) = v else { panic!("the extraction must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!(
            "could not extract {doc}: page 1 inherits from more than 256 levels of the PDF page tree"
        )
    );
}

/// As [`exploit`], for a refusal of the climb.
async fn climb_exploit(doc: &str, bytes: Vec<u8>) {
    let f = nesting_fixture().await;
    seed(&f, doc, &bytes).await;
    let pid_before = std::process::id();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let bystander = tokio::spawn(bystander_loop(f.state.clone(), rx));

    let start = Instant::now();
    let v = extract_text(&f, OWNER, doc, false).await;
    let took = start.elapsed();

    stop.send(true).expect("the bystander listens");
    let reads = bystander.await.expect("every bystander read answered hello");
    assert!(reads >= 1, "the bystander never read");
    climb_refusal(&v, doc);
    assert!(took < refusal_bound(), "the refusal took {took:?}");
    nothing_left(&f, doc, None).await;
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_sec_005_a_page_whose_parent_is_itself_a_l() {
    in_child(MODULE, "e2e_0092_sec_005_a_page_whose_parent_is_itself_a_l", || {
        climb_exploit("/docs/parentloop.pdf", parent_loop_pdf(false, true, HELLO_TEXT))
    });
}

#[test]
fn e2e_0092_sec_006_a_parent_chain_of_10_000_page_gro() {
    in_child(MODULE, "e2e_0092_sec_006_a_parent_chain_of_10_000_page_gro", || {
        climb_exploit("/docs/parents.pdf", parent_chain_pdf(10_000))
    });
}

#[tokio::test]
async fn e2e_0092_006_resources_and_page_size_256_page_grou() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/tall.pdf", &parent_chain_pdf(256)).await;

    let v = extract_text(&f, OWNER, "/docs/tall.pdf", false).await.expect("256 groups extract");

    assert!(v["preview"].as_str().unwrap().contains("Hello"), "{v}");
}

#[tokio::test]
async fn e2e_0092_009_looping_parent_chain_above_a_self_suf() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/selfloop.pdf", &parent_loop_pdf(true, true, HELLO_TEXT)).await;

    let v =
        extract_text(&f, OWNER, "/docs/selfloop.pdf", false).await.expect("a self sufficient page");

    assert!(v["preview"].as_str().unwrap().contains("Hello"), "{v}");
}

#[tokio::test]
async fn e2e_0092_013_resources_257_page_groups_up() {
    let f = nesting_fixture().await;
    let pid_before = std::process::id();
    seed(&f, "/docs/taller.pdf", &parent_chain_pdf(257)).await;

    let v = extract_text(&f, OWNER, "/docs/taller.pdf", false).await;

    climb_refusal(&v, "/docs/taller.pdf");
    nothing_left(&f, "/docs/taller.pdf", None).await;
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_017_page_size_missing_along_a_looping_cha() {
    in_child(MODULE, "e2e_0092_017_page_size_missing_along_a_looping_cha", || async {
        let f = nesting_fixture().await;
        let pid_before = std::process::id();
        seed(&f, "/docs/nosize.pdf", &parent_loop_pdf(true, false, HELLO_TEXT)).await;

        let v = extract_text(&f, OWNER, "/docs/nosize.pdf", false).await;

        climb_refusal(&v, "/docs/nosize.pdf");
        still_up(&f, pid_before).await;
    });
}

#[test]
fn e2e_0092_021_page_tree_climb_before_the_page_s_dra() {
    in_child(MODULE, "e2e_0092_021_page_tree_climb_before_the_page_s_dra", || async {
        let f = nesting_fixture().await;
        seed(&f, "/docs/climbloop.pdf", &parent_loop_pdf(false, false, "/A Do")).await;

        let v = extract_text(&f, OWNER, "/docs/climbloop.pdf", false).await;

        climb_refusal(&v, "/docs/climbloop.pdf");
    });
}

/// The character map refusal of page 1 of `doc`.
fn map_refusal(v: &Result<Value>, doc: &str) {
    let Err(e) = v else { panic!("the extraction must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!("could not extract {doc}: a PDF character map is nested more than 64 deep")
    );
}

/// As [`exploit`], for a refusal of a character map.
async fn map_exploit(doc: &str, bytes: Vec<u8>) {
    let f = nesting_fixture().await;
    seed(&f, doc, &bytes).await;
    let pid_before = std::process::id();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let bystander = tokio::spawn(bystander_loop(f.state.clone(), rx));

    let start = Instant::now();
    let v = extract_text(&f, OWNER, doc, false).await;
    let took = start.elapsed();

    stop.send(true).expect("the bystander listens");
    let reads = bystander.await.expect("every bystander read answered hello");
    assert!(reads >= 1, "the bystander never read");
    map_refusal(&v, doc);
    assert!(took < refusal_bound(), "the refusal took {took:?}");
    nothing_left(&f, doc, None).await;
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_sec_007_character_map_of_10_000_nested_li() {
    in_child(MODULE, "e2e_0092_sec_007_character_map_of_10_000_nested_li", || {
        map_exploit("/docs/cmapdeep.pdf", cmap_pdf(&nested_map("[", "]", 10_000)))
    });
}

#[test]
fn e2e_0092_sec_008_character_map_with_100_000_nested() {
    in_child(MODULE, "e2e_0092_sec_008_character_map_with_100_000_nested", || {
        map_exploit("/docs/cmapparen.pdf", cmap_pdf(&nested_map("(", ")", 100_000)))
    });
}

#[test]
fn e2e_0092_sec_009_character_map_with_100_000_nested() {
    in_child(MODULE, "e2e_0092_sec_009_character_map_with_100_000_nested", || {
        map_exploit("/docs/cmapdict.pdf", cmap_pdf(&nested_map("<< /K ", ">> ", 100_000)))
    });
}

#[test]
fn e2e_0092_sec_010_composite_font_code_map_of_10_000() {
    in_child(MODULE, "e2e_0092_sec_010_composite_font_code_map_of_10_000", || {
        map_exploit("/docs/type0.pdf", type0_pdf(&nested_map("[", "]", 10_000)))
    });
}

#[tokio::test]
async fn e2e_0092_007_character_map_nested_exactly_64_deep() {
    let f = nesting_fixture().await;
    let body = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /X {}{} def endcmap end end",
        "[".repeat(64),
        "]".repeat(64)
    );
    seed(&f, "/docs/cmap64.pdf", &cmap_pdf(&body)).await;

    let v = extract_text(&f, OWNER, "/docs/cmap64.pdf", false).await.expect("64 levels extract");

    assert!(v["preview"].as_str().unwrap().contains("Hello"), "{v}");
}

#[tokio::test]
async fn e2e_0092_014_character_map_nested_65_deep() {
    let f = nesting_fixture().await;
    let pid_before = std::process::id();
    seed(&f, "/docs/cmap65.pdf", &cmap_pdf(&nested_map("[", "]", 65))).await;

    let v = extract_text(&f, OWNER, "/docs/cmap65.pdf", false).await;

    map_refusal(&v, "/docs/cmap65.pdf");
    nothing_left(&f, "/docs/cmap65.pdf", None).await;
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_022_form_depth_before_a_later_character_m() {
    in_child(MODULE, "e2e_0092_022_form_depth_before_a_later_character_m", || async {
        let f = nesting_fixture().await;
        seed(&f, "/docs/order.pdf", &order_pdf()).await;

        let v = extract_text(&f, OWNER, "/docs/order.pdf", false).await;

        depth_refusal(&v, "/docs/order.pdf");
    });
}

/// DEC-005: a map is scanned after its stream limit, not while it is measured.
#[tokio::test]
async fn map_nesting_is_checked_after_the_stream_limit() {
    let _heavy = heavy().await;
    let f = nesting_fixture().await;
    let stored = zlib_run(&"[".repeat(100), b' ', PDF_BOMB, "");
    let mut w = pages_writer(vec![plain(HELLO_TEXT)], "");
    w.object(
        5,
        PdfObj::Body(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>".to_string(),
        ),
    );
    w.object(9, flate("", stored));
    seed(&f, "/docs/mapbomb.pdf", &w.finish()).await;

    let v = extract_text(&f, OWNER, "/docs/mapbomb.pdf", false).await;

    let Err(e) = v else { panic!("the extraction must be refused") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        "could not extract /docs/mapbomb.pdf: a PDF stream decodes to more than 104857600 bytes"
    );
}
