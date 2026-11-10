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
