//! SPEC-0092 tests: a PDF nesting that would overflow the reader's stack is
//! refused by the SPEC-0089 walk before the reader runs. Each exploit test runs
//! its attacked call in a child process (`in_child`), so a pre fix abort shows
//! as a failed assertion, and reads the absence of effect on second channels:
//! `nothing_left`, the bystander reading `hello`, the same pid.

use crate::errors::{Result, code};
use crate::tools::admin::test_support::Fixture;
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
        seed(&f, "/docs/climbloop.pdf", &climb_order_pdf()).await;

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

#[tokio::test]
async fn e2e_0092_007_strings_nested_exactly_64_deep() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/str64.pdf", &cmap_pdf(&nested_map("(", ")", 64))).await;

    let v = extract_text(&f, OWNER, "/docs/str64.pdf", false).await.expect("64 strings extract");

    assert!(v["preview"].as_str().unwrap().contains("Hello"), "{v}");
}

#[tokio::test]
async fn e2e_0092_007_dictionaries_nested_exactly_64_deep() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/dict64.pdf", &cmap_pdf(&nested_map_around("<< /K ", "1", " >>", 64))).await;

    let v =
        extract_text(&f, OWNER, "/docs/dict64.pdf", false).await.expect("64 dictionaries extract");

    assert!(v["preview"].as_str().unwrap().contains("Hello"), "{v}");
}

#[tokio::test]
async fn e2e_0092_014_dictionaries_nested_65_deep() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/dict65.pdf", &cmap_pdf(&nested_map_around("<< /K ", "1", " >>", 65))).await;

    let v = extract_text(&f, OWNER, "/docs/dict65.pdf", false).await;

    map_refusal(&v, "/docs/dict65.pdf");
    nothing_left(&f, "/docs/dict65.pdf", None).await;
}

/// Every level is a string holding a `%` (text inside a string, not a comment) and an
/// escaped `\)` (text, not a close): 65 real levels, refused.
#[tokio::test]
async fn e2e_0092_014_comment_and_escape_do_not_hide_a_level() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/evade.pdf", &cmap_pdf(&nested_map("(% \\) ", ")", 65))).await;

    let v = extract_text(&f, OWNER, "/docs/evade.pdf", false).await;

    map_refusal(&v, "/docs/evade.pdf");
    nothing_left(&f, "/docs/evade.pdf", None).await;
}

// ── `map_nesting` against the real cmap parser (adobe-cmap-parser 0.4.1) ─────

/// The shape the real parser gives `v`: strings by length, scalars as one mark.
fn shape(v: &adobe_cmap_parser::Value) -> String {
    use adobe_cmap_parser::Value as V;
    match v {
        V::Array(xs) => format!("[{}]", xs.iter().map(shape).collect::<Vec<_>>().join(",")),
        V::Dictionary(d) => {
            let mut e: Vec<_> = d.iter().map(|(k, v)| format!("{k}:{}", shape(v))).collect();
            e.sort();
            format!("<{}>", e.join(","))
        }
        V::LiteralString(b) => format!("s{}", b.len()),
        _ => "x".to_string(),
    }
}

/// Levels the real parser recursed through to build `v`: a list or a dictionary is one,
/// a string is one plus the nesting of the strings it returned with their parentheses.
/// A hexadecimal string also comes back as a string, so the corpus keeps them above
/// the deepest level, where the extra count changes nothing.
fn tree_depth(v: &adobe_cmap_parser::Value) -> u32 {
    use adobe_cmap_parser::Value as V;
    match v {
        V::Array(xs) => 1 + xs.iter().map(tree_depth).max().unwrap_or(0),
        V::Dictionary(d) => 1 + d.values().map(tree_depth).max().unwrap_or(0),
        V::LiteralString(b) => {
            let (mut depth, mut max) = (0u32, 0u32);
            for c in b {
                match c {
                    b'(' => {
                        depth += 1;
                        max = max.max(depth);
                    }
                    b')' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            1 + max
        }
        _ => 0,
    }
}

/// The depth the real parser reaches on `input`, run on a 64 MiB stack so it never
/// overflows on the corpus (at most 4.6 KB per level in a debug build, measured).
/// An escape returns a bare parenthesis, which `tree_depth` would count, so the depth
/// is read on a twin where every escape is `\n` (same length); the twin must parse to
/// the same shape, which shows the escapes changed no level.
fn real_depth(input: &str) -> u32 {
    let mut twin = input.as_bytes().to_vec();
    let mut at = 0;
    while at + 1 < twin.len() {
        if twin[at] == b'\\' {
            twin[at + 1] = b'n';
            at += 2;
        } else {
            at += 1;
        }
    }
    let input = input.as_bytes().to_vec();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let real = adobe_cmap_parser::parse(&input).expect("the real parser accepts the input");
            let twin = adobe_cmap_parser::parse(&twin).expect("the real parser accepts the twin");
            let shapes = |vs: &[adobe_cmap_parser::Value]| vs.iter().map(shape).collect::<Vec<_>>();
            assert_eq!(shapes(&real), shapes(&twin), "an escape changed the structure");
            twin.iter().map(tree_depth).max().unwrap_or(0)
        })
        .expect("the oracle thread")
        .join()
        .expect("the oracle did not panic")
}

/// Maps nested `n` levels, one per construct and mixes, each wrapped as a map value.
fn map_corpus(n: usize) -> Vec<String> {
    let wrap = |body: String| format!("% head [[[[ (((( <<<<\n/X {body} def");
    let around = |open: &str, middle: &str, close: &str| {
        wrap(format!("{}{middle}{}", open.repeat(n), close.repeat(n)))
    };
    let half = n / 2;
    // A hexadecimal string at every level but the deepest.
    let hex = |open: &str, last: &str, middle: &str, close: &str| {
        wrap(format!("{}{last}{middle}{}", open.repeat(n - 1), close.repeat(n)))
    };
    vec![
        around("[", "", "]"),
        around("<< /K ", "1", " >>"),
        around("(", "", ")"),
        hex("[ <0A0B> ", "[", "", "]"),
        hex("<< /H <0A> /K ", "<< /K ", "1", " >>"),
        around("(\\) \\( ", "", ")"),
        // An escaped close at every level: read as a close, it would hide every level.
        around("(\\) ", "", ")"),
        around("(\\( ", "", ")"),
        around("(\\\\", "", ")"),
        around("(% ", "", ")"),
        // Lists and dictionaries in turn.
        wrap(format!(
            "{}1{}",
            (0..n).map(|i| if i % 2 == 0 { "[ " } else { "<< /K " }).collect::<String>(),
            (0..n).rev().map(|i| if i % 2 == 0 { " ]" } else { " >>" }).collect::<String>()
        )),
        wrap(format!(
            "{}{}{}{}",
            "[".repeat(n - half),
            "(".repeat(half),
            ")".repeat(half),
            "]".repeat(n - half)
        )),
        wrap(format!(
            "{}{}1{}{}",
            "[".repeat(n - half),
            "<< /K ".repeat(half),
            " >>".repeat(half),
            "]".repeat(n - half)
        )),
    ]
}

/// `map_nesting` never undercounts the real parser: for every corpus map at 63, 64 and 65
/// levels it reports the depth the real parser reached (capped past the limit, where it
/// returns early), so a map the real parser takes past 64 levels is reported past 64.
#[test]
fn map_nesting_matches_the_real_cmap_parser() {
    use crate::docs::pdf_bound::{MAP_NESTING_LIMIT, map_nesting};
    for n in [1, 2, 63, 64, 65] {
        for map in map_corpus(n) {
            let real = real_depth(&map);
            let counted = map_nesting(map.as_bytes());
            assert_eq!(real, n as u32, "the corpus map is {n} deep: {map}");
            let cap = MAP_NESTING_LIMIT + 1;
            assert_eq!(counted.min(cap), real.min(cap), "{map}");
            if real > MAP_NESTING_LIMIT {
                assert!(counted > MAP_NESTING_LIMIT, "undercount {counted} < {real}: {map}");
            }
        }
    }
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

// ── code search over deeply nested sources (SPEC-0092 DEC-007) ───────────────

const PY_HEAD: &str = "def f():\n    return ";

fn deep_py(n: usize) -> String {
    deep_source(PY_HEAD, '(', "1", ')', n, "\n")
}

/// Authorize, then ask the engine for the definitions of `name` under `/src`, as both doors do.
async fn definitions(f: &Fixture, name: &str) -> Value {
    f.state.authorize(MOUNT, OWNER).await.expect("the owner is a member");
    let c = f.state.stores.client(MOUNT).await.expect("atlas volume");
    crate::core::fs_ops::find_definitions(&c, "/src", name, None).await.expect("an answer")
}

async fn references(f: &Fixture, name: &str) -> Value {
    f.state.authorize(MOUNT, OWNER).await.expect("the owner is a member");
    let c = f.state.stores.client(MOUNT).await.expect("atlas volume");
    crate::core::fs_ops::find_references(&c, "/src", name).await.expect("an answer")
}

async fn seed_src(f: &Fixture, path: &str, source: &str) {
    let c = f.state.stores.client(MOUNT).await.expect("atlas volume");
    c.makedirs("/src", true).await.expect("src folder");
    seed(f, path, source.as_bytes()).await;
}

fn one_definition(v: &Value, path: &str) {
    assert_eq!(
        v["definitions"],
        serde_json::json!([{"path": path, "name": "f", "kind": "function_definition", "line": 1}]),
        "{v}"
    );
}

#[test]
fn e2e_0092_sec_011_definition_search_over_100_000_ne() {
    in_child(MODULE, "e2e_0092_sec_011_definition_search_over_100_000_ne", || async {
        let f = nesting_fixture().await;
        seed_src(&f, "/src/deep.py", &deep_py(100_000)).await;
        let pid_before = std::process::id();
        let (stop, rx) = tokio::sync::watch::channel(false);
        let bystander = tokio::spawn(bystander_loop(f.state.clone(), rx));

        let start = Instant::now();
        let v = definitions(&f, "f").await;
        let took = start.elapsed();

        stop.send(true).expect("the bystander listens");
        let reads = bystander.await.expect("every bystander read answered hello");
        assert!(reads >= 1, "the bystander never read");
        one_definition(&v, "/src/deep.py");
        let limit =
            if cfg!(debug_assertions) { Duration::from_secs(30) } else { Duration::from_secs(10) };
        assert!(took < limit, "the search took {took:?}, more than {limit:?}");
        still_up(&f, pid_before).await;
    });
}

/// SEC-011 through the MCP handler as the owner: the scan the handler reaches is the one
/// that walks with an explicit stack.
#[test]
fn e2e_0092_sec_011_definition_search_through_the_handler() {
    in_child(MODULE, "e2e_0092_sec_011_definition_search_through_the_handler", || async {
        use crate::mcp::server::{FindDefinitionArgs, McpServer};
        use rmcp::handler::server::wrapper::Parameters;
        use rmcp::model::ContentBlock;
        let f = nesting_fixture().await;
        seed_src(&f, "/src/deep.py", &deep_py(100_000)).await;
        let pid_before = std::process::id();
        let server = McpServer::new(f.state.clone(), OWNER.to_string());
        let args = FindDefinitionArgs {
            mount_id: MOUNT.into(),
            name: "f".into(),
            root: "/src".into(),
            kind: None,
        };

        let r = server.fs_find_definition(Parameters(args)).await.expect("a tool result");

        let text = r
            .content
            .iter()
            .find_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .expect("a text block");
        assert_ne!(r.is_error, Some(true), "{text}");
        one_definition(&serde_json::from_str(&text).expect("a json answer"), "/src/deep.py");
        still_up(&f, pid_before).await;
    });
}

#[test]
fn e2e_0092_sec_012_c_definition_whose_name_hides_in() {
    in_child(MODULE, "e2e_0092_sec_012_c_definition_whose_name_hides_in", || async {
        let f = nesting_fixture().await;
        let src = deep_source("int f(void) { return ", '(', "1", ')', 100_000, "; }");
        seed_src(&f, "/src/deep.c", &src).await;
        let pid_before = std::process::id();
        let v = definitions(&f, "f").await;
        one_definition(&v, "/src/deep.c");
        still_up(&f, pid_before).await;
    });
}

#[tokio::test]
async fn e2e_0092_003_definition_in_a_moderately_nested_pyt() {
    let f = nesting_fixture().await;
    seed_src(&f, "/src/mid.py", &deep_py(1_000)).await;
    one_definition(&definitions(&f, "f").await, "/src/mid.py");
}

#[tokio::test]
async fn e2e_0092_010_references_keep_their_order() {
    let f = nesting_fixture().await;
    seed_src(&f, "/src/order.py", "total = 1\nprint(total)\ntotal += 2\n").await;
    let v = references(&f, "total").await;
    assert_eq!(
        v["references"],
        serde_json::json!([
            {"path": "/src/order.py", "line": 1, "kind": "identifier"},
            {"path": "/src/order.py", "line": 2, "kind": "identifier"},
            {"path": "/src/order.py", "line": 3, "kind": "identifier"},
        ]),
        "{v}"
    );
}

#[tokio::test]
async fn e2e_0092_025_not_a_member_deep_source() {
    use crate::mcp::server::{FindDefinitionArgs, McpServer};
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ContentBlock;
    let f = nesting_fixture().await;
    seed_src(&f, "/src/deep.py", &deep_py(100_000)).await;
    let server = McpServer::new(f.state.clone(), "eve@example.com".to_string());
    let args = FindDefinitionArgs {
        mount_id: MOUNT.into(),
        name: "f".into(),
        root: "/src".into(),
        kind: None,
    };
    let r = server.fs_find_definition(Parameters(args)).await.expect("a tool result");
    assert_eq!(r.is_error, Some(true), "{r:?}");
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text block");
    assert!(text.contains(code::FORBIDDEN), "{text}");
    assert!(text.contains("'eve@example.com' is not a member of 'atlas'"), "{text}");
}

// ── order of refusals, no trace, walk and reader in step (US-0226) ──────────

/// Store `bytes` as `/docs/loop.pdf` and ask for its text as the owner.
async fn loop_extract(f: &Fixture, doc: &str, refresh: bool) -> Result<Value> {
    seed(f, doc, &loop_pdf()).await;
    extract_text(f, OWNER, doc, refresh).await
}

#[tokio::test]
async fn e2e_0092_011_up_to_date_companion_without_refresh() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/loop.pdf", &loop_pdf()).await;
    seed(&f, "/docs/loop.md", b"kept").await;
    let pid_before = std::process::id();

    let v = extract_text(&f, OWNER, "/docs/loop.pdf", false).await.expect("the companion answers");

    assert_eq!(v["cached"], true, "{v}");
    assert_eq!(v["preview"], "kept", "{v}");
    still_up(&f, pid_before).await;
}

#[test]
fn e2e_0092_016_upper_case_suffix() {
    in_child(MODULE, "e2e_0092_016_upper_case_suffix", || async {
        let f = nesting_fixture().await;

        let v = loop_extract(&f, "/docs/LOOP.PDF", false).await;

        depth_refusal(&v, "/docs/LOOP.PDF");
    });
}

#[tokio::test]
async fn e2e_0092_018_not_a_member_self_drawing_form() {
    use crate::mcp::server::{ExtractTextArgs, McpServer};
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ContentBlock;
    let f = nesting_fixture().await;
    seed(&f, "/docs/loop.pdf", &loop_pdf()).await;
    let server = McpServer::new(f.state.clone(), "eve@example.com".to_string());
    let args = ExtractTextArgs {
        mount_id: MOUNT.into(),
        path: "/docs/loop.pdf".into(),
        max_chars: 200_000,
        preview_chars: 4_000,
        ocr: true,
        refresh: false,
    };

    let r = server.fs_extract_text(Parameters(args)).await.expect("a tool result");

    assert_eq!(r.is_error, Some(true), "{r:?}");
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text block");
    assert!(text.contains(code::FORBIDDEN), "{text}");
    assert!(text.contains("'eve@example.com' is not a member of 'atlas'"), "{text}");
    assert!(!text.contains("nested more than"), "{text}");
}

#[tokio::test]
async fn e2e_0092_019_load_refusal_before_a_form_loop() {
    let _heavy = heavy().await;
    let f = nesting_fixture().await;
    seed(&f, "/docs/both.pdf", &both_pdf()).await;

    let v = extract_text(&f, OWNER, "/docs/both.pdf", false).await;

    let Err(e) = v else { panic!("the extraction must be refused") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        "could not extract /docs/both.pdf: the PDF holds more than 1000000 objects"
    );
}

#[tokio::test]
async fn e2e_0092_020_page_1_stream_limit_before_a_page_2_f() {
    let _heavy = heavy().await;
    let f = nesting_fixture().await;
    seed(&f, "/docs/two.pdf", &two_pages_bomb_then_loop_pdf()).await;

    let v = extract_text(&f, OWNER, "/docs/two.pdf", false).await;

    let Err(e) = v else { panic!("the extraction must be refused") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        "could not extract /docs/two.pdf: a PDF stream decodes to more than 104857600 bytes"
    );
}

#[test]
fn e2e_0092_023_refusal_repeated() {
    in_child(MODULE, "e2e_0092_023_refusal_repeated", || async {
        let f = nesting_fixture().await;
        seed(&f, "/docs/loop.pdf", &loop_pdf()).await;

        let first = extract_text(&f, OWNER, "/docs/loop.pdf", false).await;
        let second = extract_text(&f, OWNER, "/docs/loop.pdf", false).await;

        depth_refusal(&first, "/docs/loop.pdf");
        depth_refusal(&second, "/docs/loop.pdf");
        nothing_left(&f, "/docs/loop.pdf", None).await;
    });
}

#[test]
fn e2e_0092_024_refresh_with_an_existing_companion() {
    in_child(MODULE, "e2e_0092_024_refresh_with_an_existing_companion", || async {
        let f = nesting_fixture().await;
        let pid_before = std::process::id();
        seed(&f, "/docs/loop.pdf", &loop_pdf()).await;
        seed(&f, "/docs/loop.md", b"kept").await;

        let v = extract_text(&f, OWNER, "/docs/loop.pdf", true).await;

        depth_refusal(&v, "/docs/loop.pdf");
        nothing_left(&f, "/docs/loop.pdf", Some(b"kept")).await;
        still_up(&f, pid_before).await;
    });
}

/// What the operator reads for a refusal: one informational line naming the code and the
/// reason, and no audit line. The capture records on the thread that took the lock, and the
/// body of `in_child` is polled on the calling thread, so the lock is taken inside it.
#[test]
fn e2e_0092_027_operator_trail_of_a_refusal() {
    in_child(MODULE, "e2e_0092_027_operator_trail_of_a_refusal", || async {
        use crate::mcp::server::{ExtractTextArgs, McpServer};
        use rmcp::handler::server::wrapper::Parameters;
        let _capture = crate::logging::capture::lock_for_test();
        let f = nesting_fixture().await;
        seed(&f, "/docs/loop.pdf", &loop_pdf()).await;
        let server = McpServer::new(f.state.clone(), OWNER.to_string());
        let args = ExtractTextArgs {
            mount_id: MOUNT.into(),
            path: "/docs/loop.pdf".into(),
            max_chars: 200_000,
            preview_chars: 4_000,
            ocr: true,
            refresh: false,
        };
        crate::logging::capture::clear();

        let out = server.fs_extract_text(Parameters(args)).await.expect("a tool result");

        assert_eq!(out.is_error, Some(true), "{out:?}");
        let events = crate::logging::capture::events();
        let level = |e: &crate::logging::capture::CapturedEvent| e.fields["level"].clone();
        let message = |e: &crate::logging::capture::CapturedEvent| e.fields["message"].clone();
        let info: Vec<_> = events
            .iter()
            .filter(|e| level(e) == "INFO" && message(e).contains("ERR_INVALID_ARGUMENT"))
            .collect();
        assert_eq!(info.len(), 1, "{events:?}");
        assert!(
            message(info[0]).contains(
                "could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep"
            ),
            "{events:?}"
        );
        assert!(!events.iter().any(|e| level(e) == "ERROR"), "{events:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        assert!(!log.iter().any(|e| e.path.contains("loop")), "{log:?}");
    });
}

#[test]
fn e2e_0092_028_two_attacks_and_a_normal_extraction_a() {
    in_child(MODULE, "e2e_0092_028_two_attacks_and_a_normal_extraction_a", || async {
        let f = nesting_fixture().await;
        let pid_before = std::process::id();
        seed(&f, "/docs/loop.pdf", &loop_pdf()).await;
        seed(&f, "/docs/pair.pdf", &pair_pdf()).await;
        seed(&f, "/docs/forms.pdf", &forms_pdf()).await;

        let (a, b, c) = tokio::join!(
            extract_text(&f, OWNER, "/docs/loop.pdf", false),
            extract_text(&f, OWNER, "/docs/pair.pdf", false),
            extract_text(&f, OWNER, "/docs/forms.pdf", false),
        );

        depth_refusal(&a, "/docs/loop.pdf");
        depth_refusal(&b, "/docs/pair.pdf");
        let c = c.expect("the ordinary document extracts");
        assert!(c["preview"].as_str().unwrap().contains("Deep text"), "{c}");
        still_up(&f, pid_before).await;
    });
}

// ── walk and reader parity at each stop of the walk (DEC-013) ───────────────

/// The child extracts `/docs/parity.pdf` and must get the reader's contained failure: the
/// reader panics at the defect, so a walk that went on to the self drawing form would
/// answer the form depth message instead.
async fn parity(bytes: Vec<u8>) {
    let f = nesting_fixture().await;
    seed(&f, "/docs/parity.pdf", &bytes).await;

    let v = extract_text(&f, OWNER, "/docs/parity.pdf", false).await;

    let Err(e) = v else { panic!("the extraction must fail: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, "could not extract /docs/parity.pdf: pdf parser failed on this document");
}

#[test]
fn parity_tf_without_name() {
    in_child(MODULE, "parity_tf_without_name", || parity(parity_pdf("12 Tf", PARITY_A, vec![])));
}

#[test]
fn parity_tf_missing_font() {
    in_child(MODULE, "parity_tf_missing_font", || {
        parity(parity_pdf("/F9 12 Tf", PARITY_A, vec![]))
    });
}

#[test]
fn parity_cs_without_name() {
    in_child(MODULE, "parity_cs_without_name", || parity(parity_pdf("cs", PARITY_A, vec![])));
}

#[test]
fn parity_cs_missing_space() {
    in_child(MODULE, "parity_cs_missing_space", || parity(parity_pdf("/CS9 cs", PARITY_A, vec![])));
}

#[test]
fn parity_do_without_name() {
    in_child(MODULE, "parity_do_without_name", || parity(parity_pdf("Do", PARITY_A, vec![])));
}

#[test]
fn parity_do_missing_xobject() {
    in_child(MODULE, "parity_do_missing_xobject", || {
        parity(parity_pdf("/X9 Do", PARITY_A, vec![]))
    });
}

#[test]
fn parity_do_not_a_stream() {
    in_child(MODULE, "parity_do_not_a_stream", || {
        parity(parity_pdf(
            "/N Do",
            "/XObject << /A 10 0 R /N 12 0 R >>",
            vec![(12, PdfObj::Body("<< /Type /XObject /Subtype /Form >>".to_string()))],
        ))
    });
}

/// The story names `(unclosed` as the unparsable content, but the content parser reads it
/// leniently (it decodes), so the walk never stops there; `BI ID`, an inline image with no
/// data, is the smallest content `Content::decode` refuses.
#[test]
fn parity_do_unparsable_form() {
    in_child(MODULE, "parity_do_unparsable_form", || {
        parity(parity_pdf(
            "/G Do",
            "/XObject << /A 10 0 R /G 13 0 R >>",
            vec![(13, form(Some(""), "BI ID"))],
        ))
    });
}
