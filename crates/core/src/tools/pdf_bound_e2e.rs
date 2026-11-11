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
