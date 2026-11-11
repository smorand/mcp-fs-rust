//! SPEC-0089 tests: every PDF stream the text pass decodes is measured first and
//! refused past 104 857 600 bytes. Each exploit test carries its literal payload
//! (a stream of exactly 200 000 000 spaces, stored in under 300 000 bytes) and
//! checks the second channels: the decoded bytes held through the hook, the
//! listing of `/docs`, the artifact lists, the audit trail and a following
//! 52 428 800 byte write. Every test takes `heavy()` first: before the fix a
//! bomb stream is inflated whole.

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use crate::tools::pdf_testkit::*;
use serde_json::{Value, json};

/// The stream limit refusal for `doc`.
fn stream_refusal(v: &Result<Value>, doc: &str, held: u64) {
    let Err(e) = v else {
        panic!("the extraction must be refused; it held {held} decoded bytes: {v:?}")
    };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!("could not extract {doc}: a PDF stream decodes to more than 104857600 bytes")
    );
}

/// Store `bytes` at `doc`, extract it as the member, then check the stream
/// limit refusal, more than [`STREAM_LIMIT`] and at most [`BOMB_HELD`] decoded
/// bytes held (the measurer really decoded up to the limit, and no further),
/// and nothing left.
async fn refused_on(doc: &str, bytes: &[u8]) {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, doc, false).await;

    let held = decoded_held(doc);
    assert!(held <= BOMB_HELD, "the extraction held {held} decoded bytes, more than {BOMB_HELD}");
    assert!(held > STREAM_LIMIT as u64, "the refusal decoded only {held} bytes");
    stream_refusal(&v, doc, held);
    nothing_left(&f, doc, None).await;
}

#[tokio::test]
async fn e2e_0089_sec_001_page_content_stream_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/bomb.pdf", &content_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_002_compressed_object_stream_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/objstm.pdf", &object_stream_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_003_font_file_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/font.pdf", &font_file_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_004_character_map_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/cmap.pdf", &cmap_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_005_colour_profile_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/icc.pdf", &colour_profile_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_006_image_bomb_drawn_once() {
    let _heavy = heavy().await;
    refused_on("/docs/image.pdf", &image_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_sec_012_title_read_decoding_again() {
    let _heavy = heavy().await;
    refused_on("/docs/titled.pdf", &titled_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_007_stream_of_exactly_the_stream_limit() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/large.pdf", &large_content_pdf(STREAM_LIMIT)).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/large.pdf", false).await.expect("within the limit");

    let held = decoded_held("/docs/large.pdf");
    assert!(held <= BOMB_HELD, "the extraction held {held} decoded bytes, more than {BOMB_HELD}");
    assert_eq!(v["truncated"], json!(true), "{v}");
    let preview = v["preview"].as_str().unwrap();
    let text = preview.trim_start_matches("---\n*[Page 1]*\n\n");
    assert!(!text.is_empty() && text.chars().all(|c| c == 'b'), "{preview}");
}

#[tokio::test]
async fn e2e_0089_008_stream_one_byte_over_the_stream_limit() {
    let _heavy = heavy().await;
    refused_on("/docs/large.pdf", &large_content_pdf(STREAM_LIMIT + 1)).await;
}

/// Today's answer for `/docs/photo.pdf`, recorded on main's code before any
/// production change (SPEC-0089 DEC-020).
const PHOTO_ANSWER: &str = r#"{"path":"/docs/photo.pdf","md_path":"/docs/photo.md","format":"pdf","chars":23,"cached":false,"preview":"---\n*[Page 1]*\n\nCaption","truncated":false,"meta":{"pages":1},"note":"","artifacts_report":"0 images, 0 tables"}"#;

#[tokio::test]
async fn e2e_0089_017_a_stream_the_extraction_does_not_deco() {
    let f = office_fixture().await;
    seed(&f, "/docs/photo.pdf", &photo_pdf()).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/photo.pdf", false).await.expect("answered as today");

    assert_eq!(v, serde_json::from_str::<Value>(PHOTO_ANSWER).unwrap());
    let held = decoded_held("/docs/photo.pdf");
    assert!(held <= 10_000, "the extraction held {held} decoded bytes, more than 10 000");
}

#[tokio::test]
async fn e2e_0089_032_title_read_under_the_same_limits() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/titled.pdf", &titled_bomb_pdf()).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/titled.pdf", false).await;

    stream_refusal(&v, "/docs/titled.pdf", decoded_held("/docs/titled.pdf"));
    assert!(!format!("{v:?}").contains("Report 2026"), "no title is reported: {v:?}");
}

/// The body the library's own pass gives for `bytes`: its pages trimmed, each
/// non empty one behind its page marker, as the extraction assembles them.
fn library_preview(bytes: &[u8]) -> String {
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes).expect("the library reads it");
    let mut body = String::new();
    for (i, text) in pages.iter().enumerate() {
        if !text.trim().is_empty() {
            body.push_str(&format!("\n\n---\n*[Page {}]*\n\n{}", i + 1, text.trim()));
        }
    }
    body.trim().to_string()
}

/// Store `bytes` at `doc`, extract it, and check the preview is `expected`
/// and the library's own pass over the same bytes.
async fn unchanged_on(doc: &str, bytes: &[u8], expected: &str) -> Value {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;

    let v = extract_text(&f, OWNER, doc, false).await.expect("answered");

    assert_eq!(v["preview"], json!(expected), "{v}");
    assert_eq!(library_preview(bytes), expected);
    v
}

const SECRET_PREVIEW: &str = "---\n*[Page 1]*\n\nQuarterly results\n\n---\n*[Page 2]*\n\nOutlook";

#[tokio::test]
async fn pdf_cross_reference_stream_bomb_is_refused() {
    let _heavy = heavy().await;
    refused_on("/docs/xref.pdf", &xref_stream_bomb_pdf()).await;
}

#[tokio::test]
async fn pdf_encrypted_object_stream_bomb_is_refused() {
    let _heavy = heavy().await;
    refused_on("/docs/locked.pdf", &locked_bomb_pdf()).await;
}

#[tokio::test]
async fn pdf_encrypted_document_text_is_unchanged() {
    let v = unchanged_on("/docs/secret.pdf", &secret_pdf("/Encrypt"), SECRET_PREVIEW).await;
    assert_eq!(v["meta"]["pages"], json!(2), "{v}");
    assert_eq!(v["meta"]["title"], json!("Report 2026"), "{v}");
}

#[tokio::test]
async fn pdf_escaped_encrypt_key_is_found() {
    let bytes = secret_pdf("/Encr#79pt");
    assert!(bytes.windows(10).any(|w| w == b"/Encr#79pt"), "the key is escaped");
    let v = unchanged_on("/docs/escaped.pdf", &bytes, SECRET_PREVIEW).await;
    assert_eq!(v["meta"]["title"], json!("Report 2026"), "{v}");
}

#[tokio::test]
async fn pdf_lzw_content_bomb_is_refused() {
    let _heavy = heavy().await;
    refused_on("/docs/lzw.pdf", &lzw_bomb_pdf()).await;
}

#[tokio::test]
async fn pdf_ascii85_then_flate_bomb_is_refused() {
    let _heavy = heavy().await;
    refused_on("/docs/a85.pdf", &ascii85_bomb_pdf()).await;
}

#[tokio::test]
async fn pdf_lzw_and_ascii85_text_is_unchanged() {
    let expected = "---\n*[Page 1]*\n\nOld LZW\n\n---\n*[Page 2]*\n\nOld A85";
    unchanged_on("/docs/old.pdf", &old_filters_pdf(), expected).await;
}

/// The refusal of `doc` with `message`, and the decoded bytes held at most `cap`.
async fn refused_with(doc: &str, bytes: &[u8], message: &str, cap: u64) {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, doc, false).await;

    let held = decoded_held(doc);
    assert!(held <= cap, "the extraction held {held} decoded bytes, more than {cap}");
    let Err(e) = &v else { panic!("the extraction must be refused; it held {held} bytes: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, format!("could not extract {doc}: {message}"));
    nothing_left(&f, doc, None).await;
}

const LOAD_MESSAGE: &str = "the PDF object streams decode to more than 268435456 bytes together";
const OBJECTS_MESSAGE: &str = "the PDF holds more than 1000000 objects";

/// Store `bytes` at `doc`, extract it, and check the preview is `expected`.
async fn accepted_with(doc: &str, bytes: &[u8], expected: &str) {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, doc, false).await.expect("at the limit");

    let held = decoded_held(doc);
    assert!(held <= LOAD_HELD, "the extraction held {held} decoded bytes, more than {LOAD_HELD}");
    assert_eq!(v["preview"], json!(expected), "{v}");
}

#[tokio::test]
async fn e2e_0089_sec_009_three_object_streams_over_the_loa() {
    let _heavy = heavy().await;
    let bytes = small(object_streams_pdf("Fine", [100_000_000; 3]));
    refused_with("/docs/objs3.pdf", &bytes, LOAD_MESSAGE, LOAD_HELD).await;
}

#[tokio::test]
async fn e2e_0089_sec_010_two_million_objects() {
    let _heavy = heavy().await;
    refused_with("/docs/nulls.pdf", &nulls_pdf(), OBJECTS_MESSAGE, LOAD_HELD).await;
}

#[tokio::test]
async fn e2e_0089_009_object_streams_totalling_exactly_the() {
    let _heavy = heavy().await;
    let bytes = object_streams_pdf("Fine", [100_000_000, 100_000_000, 68_435_456]);
    accepted_with("/docs/objs.pdf", &bytes, "---\n*[Page 1]*\n\nFine").await;
}

#[tokio::test]
async fn e2e_0089_010_object_streams_one_byte_over_the_load() {
    let _heavy = heavy().await;
    let bytes = object_streams_pdf("Fine", [100_000_000, 100_000_000, 68_435_457]);
    refused_with("/docs/objs.pdf", &bytes, LOAD_MESSAGE, LOAD_HELD).await;
}

#[tokio::test]
async fn e2e_0089_011_exactly_1_000_000_objects() {
    let _heavy = heavy().await;
    accepted_with("/docs/many.pdf", &many_pdf(0), "---\n*[Page 1]*\n\nMany").await;
}

#[tokio::test]
async fn e2e_0089_012_1_000_001_objects() {
    let _heavy = heavy().await;
    refused_with("/docs/many.pdf", &many_pdf(1), OBJECTS_MESSAGE, LOAD_HELD).await;
}

#[tokio::test]
async fn e2e_0089_026_object_count_met_before_a_later_objec() {
    let _heavy = heavy().await;
    refused_with("/docs/order1.pdf", &order_pdf(), OBJECTS_MESSAGE, LOAD_HELD).await;
}

/// The load total is met inside a third object stream that is also a bomb:
/// decoding stops at the load total, 68 435 456 bytes in, not at the stream limit.
#[tokio::test]
async fn e2e_0089_sec_009_load_total_met_inside_a_bomb_stream() {
    let _heavy = heavy().await;
    let bytes = small(object_streams_pdf("Fine", [100_000_000, 100_000_000, PDF_BOMB]));
    refused_with("/docs/objs3.pdf", &bytes, LOAD_MESSAGE, LOAD_HELD).await;
}

const PAGE_HELD: u64 = 268_500_992;

fn page_message(n: u32) -> String {
    format!("page {n} decodes more than 268435456 bytes of PDF streams")
}

#[tokio::test]
async fn e2e_0089_sec_007_one_form_drawn_300_times() {
    let _heavy = heavy().await;
    refused_with("/docs/forms.pdf", &forms_pdf(), &page_message(1), PAGE_HELD).await;
}

#[tokio::test]
async fn e2e_0089_sec_008_a_colour_profile_kept_in_300_save() {
    let _heavy = heavy().await;
    refused_with("/docs/states.pdf", &selected_states_pdf(), &page_message(1), PAGE_HELD).await;
}

#[tokio::test]
async fn e2e_0089_013_a_page_decoding_exactly_the_page_tota() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/form.pdf", &form_total_pdf(0)).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/form.pdf", false).await.expect("at the page total");

    let held = decoded_held("/docs/form.pdf");
    assert!(held <= PAGE_HELD, "the extraction held {held} decoded bytes, more than {PAGE_HELD}");
    assert!(v["preview"].as_str().unwrap().contains("Hi"), "{v}");
}

#[tokio::test]
async fn e2e_0089_014_a_page_one_byte_over_the_page_total() {
    let _heavy = heavy().await;
    refused_with("/docs/form.pdf", &form_total_pdf(1), &page_message(1), PAGE_HELD).await;
}

#[tokio::test]
async fn e2e_0089_018_page_2_over_the_page_total_page_1_fin() {
    let _heavy = heavy().await;
    refused_with("/docs/two.pdf", &two_pages_pdf(), &page_message(2), PAGE_HELD).await;
}

#[tokio::test]
async fn e2e_0089_027_load_total_met_before_a_page_bomb() {
    let _heavy = heavy().await;
    refused_with("/docs/order2.pdf", &order2_pdf(), LOAD_MESSAGE, LOAD_HELD).await;
}

/// The page total is met inside the third inflation of a form: decoding stops one
/// chunk past it, not at the end of the form (300 000 000 bytes).
#[tokio::test]
async fn e2e_0089_page_total_met_inside_a_stream() {
    let _heavy = heavy().await;
    let doc = "/docs/big.pdf";
    refused_with(doc, &big_forms_pdf(300_000_000), &page_message(1), PAGE_HELD).await;
    let held = decoded_held(doc);
    assert!(held > PAGE_LIMIT as u64, "the refusal decoded only {held} bytes");
}

/// The page total reached exactly by a first inflation, not by a repeat.
#[tokio::test]
async fn e2e_0089_page_total_exactly_met_by_a_stream() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    let third = PAGE_LIMIT - 2 * STREAM_LIMIT;
    seed(&f, "/docs/big.pdf", &big_forms_pdf(third)).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/big.pdf", false).await.expect("at the page total");

    let held = decoded_held("/docs/big.pdf");
    assert!(held <= PAGE_HELD, "the extraction held {held} decoded bytes, more than {PAGE_HELD}");
    assert!(v["preview"].as_str().unwrap().contains("Hi"), "{v}");
}

const CONTENT_HELD: u64 = 268_500_992;
const CONTENT_MESSAGE: &str = "the extracted content exceeds 268435456 bytes";

/// The content limit refusal of `doc`, the content held between the limit and
/// [`CONTENT_HELD`] (the extraction really built up to the limit), nothing left.
async fn content_refused(doc: &str, bytes: &[u8]) {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, doc, false).await;

    let content = pdf_content_held(doc);
    assert!(content <= CONTENT_HELD, "the extraction held {content} content bytes");
    assert!(
        content >= 268_435_456,
        "the refusal built only {content} content bytes: {:?}",
        v.as_ref().err()
    );
    let Err(e) = &v else { panic!("the extraction must be refused; it held {content} bytes") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, format!("could not extract {doc}: {CONTENT_MESSAGE}"));
    nothing_left(&f, doc, None).await;
}

#[tokio::test]
async fn e2e_0089_sec_011_one_code_mapped_to_a_long_text_sh() {
    let _heavy = heavy().await;
    content_refused("/docs/expand.pdf", &one_code_long_text_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_015_content_of_exactly_the_content_limit() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/text.pdf", &expanding_pdf(240)).await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/text.pdf", false).await.expect("at the limit");

    assert_eq!(pdf_content_held("/docs/text.pdf"), 268_435_456);
    assert_eq!(v["truncated"], json!(true), "{}", v["meta"]);
    assert!(v["preview"].as_str().unwrap().starts_with("---\n*[Page 1]*\n\neee"));
}

#[tokio::test]
async fn e2e_0089_016_content_one_byte_over_the_content_lim() {
    let _heavy = heavy().await;
    content_refused("/docs/text.pdf", &expanding_pdf(241)).await;
}

#[tokio::test]
async fn e2e_0089_028_content_limit_on_page_1_before_a_page() {
    let _heavy = heavy().await;
    content_refused("/docs/order3.pdf", &expanding_then_bomb_pdf()).await;
}

#[tokio::test]
async fn e2e_0089_006_pdf_without_a_text_layer() {
    let f = office_fixture().await;
    seed(&f, "/docs/scan.pdf", &scan_pdf()).await;

    let v = extract_text(&f, OWNER, "/docs/scan.pdf", false).await.expect("answered");

    assert_eq!(v["preview"], json!(""), "{v}");
    assert_eq!(v["meta"]["pages"], json!(1), "{v}");
    assert_eq!(
        v["note"],
        json!("no extractable text layer (scanned PDF?); enable a multimodal OCR provider")
    );
}

/// The answer fields every door gives for the report.
fn report_answer(v: &Value) {
    assert_eq!(v["md_path"], "/docs/report.md", "{v}");
    assert_eq!(v["format"], "pdf", "{v}");
    assert_eq!(v["cached"], false, "{v}");
    assert_eq!(v["meta"]["pages"], json!(2), "{v}");
    assert_eq!(v["meta"]["title"], "Report 2026", "{v}");
    let preview = v["preview"].as_str().unwrap();
    let at = |needle: &str| preview.find(needle).unwrap_or_else(|| panic!("{needle}: {preview}"));
    assert!(
        at("*[Page 1]*") < at("Quarterly results")
            && at("Quarterly results") < at("*[Page 2]*")
            && at("*[Page 2]*") < at("Outlook"),
        "{preview}"
    );
}

#[tokio::test]
async fn e2e_0089_001_pdf_within_the_limits() {
    let f = office_fixture().await;
    seed(&f, "/docs/report.pdf", &report_pdf()).await;

    let v = extract_text(&f, OWNER, "/docs/report.pdf", false).await.expect("within the limits");

    report_answer(&v);
    let companion = read(&f, "/docs/report.md").await.expect("the companion is written");
    let companion = String::from_utf8(companion).unwrap();
    assert!(v["preview"].as_str().unwrap().len() <= companion.len(), "{companion}");
    for needle in ["*[Page 1]*", "Quarterly results", "*[Page 2]*", "Outlook"] {
        assert!(companion.contains(needle), "{companion}");
    }
    let lines: Vec<_> =
        f.state.safety.audit(OWNER, MOUNT).into_iter().filter(|e| e.op == "extract_text").collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].path, "/docs/report.md");
}

#[tokio::test]
async fn e2e_0089_019_a_pdf_that_cannot_be_read() {
    let f = office_fixture().await;
    seed(&f, "/docs/bad.pdf", b"%PDF-1.4\ngarbage").await;

    let v = extract_text(&f, OWNER, "/docs/bad.pdf", false).await;

    let Err(e) = &v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert!(e.message.starts_with("could not extract /docs/bad.pdf: "), "{}", e.message);
    nothing_left(&f, "/docs/bad.pdf", None).await;
}

/// `/docs/bomb.pdf` then, written after it, `/docs/bomb.md` holding `kept text`.
async fn bomb_with_companion() -> crate::tools::admin::test_support::Fixture {
    let f = office_fixture().await;
    seed(&f, "/docs/bomb.pdf", &content_bomb_pdf()).await;
    seed(&f, "/docs/bomb.md", b"kept text").await;
    f
}

#[tokio::test]
async fn e2e_0089_020_up_to_date_companion_answers_without() {
    let _heavy = heavy().await;
    let f = bomb_with_companion().await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/bomb.pdf", false).await.expect("answered from cache");

    assert_eq!(v["cached"], true, "{v}");
    assert_eq!(v["md_path"], "/docs/bomb.md", "{v}");
    assert_eq!(v["preview"], "kept text", "{v}");
    assert_eq!(pdf_content_held("/docs/bomb.pdf"), 0);
    assert_eq!(
        crate::docs::extract::PDF_HELD
            .with(|m| m.borrow().get("/docs/bomb.pdf").map_or(0, |h| h.decoded)),
        0
    );
}

#[tokio::test]
async fn e2e_0089_021_refresh_on_a_pdf_bomb_keeps_the_compa() {
    let _heavy = heavy().await;
    let f = bomb_with_companion().await;
    pdf_held_reset();

    let v = extract_text(&f, OWNER, "/docs/bomb.pdf", true).await;

    stream_refusal(&v, "/docs/bomb.pdf", decoded_held("/docs/bomb.pdf"));
    assert_eq!(read(&f, "/docs/bomb.md").await.as_deref(), Some(&b"kept text"[..]));
    nothing_left(&f, "/docs/bomb.pdf", Some(b"kept text")).await;
}

#[tokio::test]
async fn e2e_0089_022_repeating_a_refused_pdf_call() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/bomb.pdf", &content_bomb_pdf()).await;
    pdf_held_reset();

    let first = extract_text(&f, OWNER, "/docs/bomb.pdf", false).await;
    let second = extract_text(&f, OWNER, "/docs/bomb.pdf", false).await;

    stream_refusal(&first, "/docs/bomb.pdf", 0);
    stream_refusal(&second, "/docs/bomb.pdf", 0);
    assert_eq!(first.unwrap_err().message, second.unwrap_err().message);
    nothing_left(&f, "/docs/bomb.pdf", None).await;
}

#[tokio::test]
async fn e2e_0089_024_not_a_member_pdf_bomb() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/bomb.pdf", &content_bomb_pdf()).await;
    pdf_held_reset();

    let denied = f.state.authorize(MOUNT, "eve@example.com").await.unwrap_err();

    assert_eq!(denied.code, code::FORBIDDEN, "{denied:?}");
    assert_eq!(denied.message, "'eve@example.com' is not a member of 'atlas'");
    let held = crate::docs::extract::PDF_HELD
        .with(|m| m.borrow().get("/docs/bomb.pdf").map_or(0, |h| h.decoded));
    assert_eq!(held, 0);
}

#[tokio::test]
async fn e2e_0089_025_missing_path() {
    let f = office_fixture().await;

    let v = extract_text(&f, OWNER, "/docs/none.pdf", false).await;

    let Err(e) = &v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::NOT_FOUND, "{e:?}");
    assert_eq!(e.message, "not a file: /docs/none.pdf");
}

#[tokio::test]
async fn e2e_0089_029_upper_case_suffix() {
    let _heavy = heavy().await;
    refused_on("/docs/BOMB.PDF", &content_bomb_pdf()).await;
}

/// What the operator reads for a PDF refusal: one informational line naming the
/// code and the limit, no error line, no audit line. The capture records on the
/// thread that took the lock, so the whole body runs on that one thread.
#[test]
fn e2e_0089_030_what_the_operator_sees_for_a_pdf_refu() {
    use crate::mcp::server::{ExtractTextArgs, McpServer};
    use rmcp::handler::server::wrapper::Parameters;

    let _capture = crate::logging::capture::lock_for_test();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let _heavy = heavy().await;
        let f = office_fixture().await;
        seed(&f, "/docs/bomb.pdf", &content_bomb_pdf()).await;
        let server = McpServer::new(f.state.clone(), OWNER.to_string());
        let args = ExtractTextArgs {
            mount_id: MOUNT.into(),
            path: "/docs/bomb.pdf".into(),
            max_chars: 200_000,
            preview_chars: 4_000,
            ocr: true,
            refresh: false,
        };
        crate::logging::capture::clear();

        let out = server.fs_extract_text(Parameters(args)).await.unwrap();

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
            message(info[0]).contains("a PDF stream decodes to more than 104857600 bytes"),
            "{events:?}"
        );
        assert!(!events.iter().any(|e| level(e) == "ERROR"), "{events:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        assert!(!log.iter().any(|e| e.path.contains("bomb")), "{log:?}");
    });
}

#[tokio::test]
async fn e2e_0089_031_two_pdf_bombs_and_a_normal_pdf_at_onc() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    let second = add_second_member(&f).await;
    f.state.admin.add_member(MOUNT, "ola@example.com", OWNER).await.unwrap();
    seed(&f, "/docs/m.pdf", &content_bomb_pdf()).await;
    seed(&f, "/docs/n.pdf", &content_bomb_pdf()).await;
    seed(&f, "/docs/report.pdf", &report_pdf()).await;
    pdf_held_reset();

    let (m, n, report) = tokio::join!(
        extract_text(&f, OWNER, "/docs/m.pdf", false),
        extract_text(&f, second, "/docs/n.pdf", false),
        extract_text(&f, "ola@example.com", "/docs/report.pdf", false),
    );

    stream_refusal(&m, "/docs/m.pdf", 0);
    stream_refusal(&n, "/docs/n.pdf", 0);
    report_answer(&report.expect("the report is answered"));
    for path in ["/docs/m.pdf", "/docs/n.pdf"] {
        let held = decoded_held(path);
        assert!(held <= BOMB_HELD && held > STREAM_LIMIT as u64, "{path} held {held}");
    }
}
