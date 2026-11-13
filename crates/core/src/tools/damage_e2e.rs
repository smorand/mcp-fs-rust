//! SPEC-0094 tests: a PDF page whose content is damaged or cut is named in the
//! note of the extraction, and the text is exactly what it was before the note
//! existed. Every test goes through `fs_ops::extract_document` on a
//! `nesting_fixture`, the one engine both doors call (DEC-008).

use crate::errors::code;
use crate::tools::archive_testkit::*;
use crate::tools::nesting_testkit::*;
use crate::tools::pdf_testkit::{PdfObj, PdfWriter, pages_writer, plain, small};
use serde_json::Value;

const SCANNED: &str = "no extractable text layer (scanned PDF?); enable a multimodal OCR provider";
const GOOD: &str = "BT /F1 12 Tf 72 700 Td (Quarterly results) Tj ET";
const CUT: &str = "BT /F1 12 Tf 72 700 Td (Revenue rose) Tj ET BT 72 650 Td (Costs fell by";

/// The note of the damage listing `pages`, as the contract words it.
fn note_of(pages: &str, plural: bool) -> String {
    let word = if plural { "pages" } else { "page" };
    format!("pdf: damaged or cut content on {word} {pages}; the text may be incomplete")
}

/// The body the reader gives for `pages`, written by hand from the markers
/// (`pdf_bound.rs`: `\n\n---\n*[Page N]*\n\n`, the first two newlines trimmed).
fn body(pages: &[&str]) -> String {
    let mut out = String::new();
    for (i, text) in pages.iter().enumerate() {
        out.push_str(&format!("\n\n---\n*[Page {}]*\n\n{text}", i + 1));
    }
    out[2..].to_string()
}

/// N pages laid out with `PdfWriter`: catalog 1, page tree 2, font 5, page
/// `10 + 2i` with its content at `11 + 2i`, `/F1` in every page's resources,
/// `extra` objects (forms from 100) set last.
fn pdf_writer(contents: Vec<PdfObj>, resources: &str) -> PdfWriter {
    let mut w = PdfWriter::new(1);
    let kids: Vec<String> = (0..contents.len()).map(|i| format!("{} 0 R", 10 + 2 * i)).collect();
    w.object(1, PdfObj::Body("<< /Type /Catalog /Pages 2 0 R >>".into()));
    w.object(
        2,
        PdfObj::Body(format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        )),
    );
    w.object(5, PdfObj::Body("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()));
    for (i, content) in contents.into_iter().enumerate() {
        let (page, no) = (10 + 2 * i as u32, 11 + 2 * i as u32);
        w.object(
            page,
            PdfObj::Body(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> {resources} >> /Contents {no} 0 R >>"
            )),
        );
        w.object(no, content);
    }
    w
}

fn pdf_pages(contents: Vec<PdfObj>, extra: Vec<(u32, PdfObj)>, resources: &str) -> Vec<u8> {
    let mut w = pdf_writer(contents, resources);
    for (n, obj) in extra {
        w.object(n, obj);
    }
    small(w.finish())
}

/// The three page `report-3p.pdf` of SC-001: page 2 is cut.
fn report_3p() -> Vec<u8> {
    pdf_pages(
        vec![plain(GOOD), plain(CUT), plain("BT /F1 12 Tf 72 700 Td (Outlook stable) Tj ET")],
        vec![],
        "",
    )
}

const FORM_FONT: &str = "/Font << /F1 5 0 R >>";

async fn stored(path: &str, bytes: &[u8]) -> crate::tools::admin::test_support::Fixture {
    let f = nesting_fixture().await;
    seed(&f, path, bytes).await;
    f
}

fn text_of(v: &Value) -> &str {
    v["preview"].as_str().unwrap_or_else(|| panic!("no preview in {v}"))
}

#[tokio::test]
async fn e2e_0094_001_a_cut_page_is_reported_and_the_text_kept() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;

    let v = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");

    assert_eq!(v["note"], note_of("2", false), "{v}");
    assert_eq!(v["meta"]["pages"], 3, "{v}");
    let text = text_of(&v);
    assert_eq!(text, body(&["Quarterly results", "Revenue rose", "Outlook stable"]), "{v}");
    assert!(!v["note"].as_str().unwrap().contains("scanned"), "{v}");
}

#[tokio::test]
async fn e2e_0094_003_two_damaged_pages_one_cut_in_a_form() {
    let (c, token) = {
        let d = tempfile::tempdir().unwrap();
        let (c, token) = crate::tools::tool_surface_e2e::setup(d.path());
        std::mem::forget(d);
        (c, token)
    };
    let (app, state) = crate::tools::tool_surface_e2e::app_with_atlas(c).await;
    let bytes = pdf_pages(
        vec![
            plain("BT /F1 12 Tf 72 700 Td (Opening remarks) Tj ET BT 72 650 Td (Agenda i"),
            plain("BT /F1 12 Tf 72 700 Td (Second page body) Tj ET"),
            plain("/Fm1 Do"),
        ],
        vec![(100, form(Some(FORM_FONT), "BT /F1 12 Tf 72 700 Td (Opening) Tj ET (cut"))],
        "/XObject << /Fm1 100 0 R >>",
    );
    let client = state.stores.client("atlas").await.unwrap();
    client.makedirs("/docs", true).await.unwrap();
    client.write_bytes_atomic("/docs/agenda.pdf", &bytes).await.unwrap();

    let v = rest_extract(&app, &token, "/docs/agenda.pdf").await;

    assert_eq!(v["note"], note_of("1, 3", true), "{v}");
    let text = text_of(&v);
    assert!(text.contains("Opening remarks"), "{v}");
    assert!(text.contains("Second page body"), "{v}");
}

/// The web API door: `POST /api/fs/atlas/extract-text`.
async fn rest_extract(app: &axum::Router, token: &str, path: &str) -> Value {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;
    let req = Request::builder()
        .method("POST")
        .uri("/api/fs/atlas/extract-text")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::json!({ "path": path }).to_string()))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), axum::http::StatusCode::OK);
    serde_json::from_slice(&to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn e2e_0094_005_exactly_twenty_damaged_pages() {
    let contents: Vec<PdfObj> = (1..=22).map(|n| plain(if n <= 20 { CUT } else { GOOD })).collect();
    let f = stored("/docs/twenty.pdf", &pdf_pages(contents, vec![], "")).await;

    let v = extract_text(&f, OWNER, "/docs/twenty.pdf", false).await.expect("extracts");

    let list: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
    assert_eq!(v["note"], note_of(&list.join(", "), true), "{v}");
    assert!(!v["note"].as_str().unwrap().contains("more"), "{v}");
}

#[tokio::test]
async fn e2e_0094_006_a_page_damaged_twice_is_named_once() {
    let bytes = pdf_pages(
        vec![
            plain(GOOD),
            plain(
                "BT /F1 12 Tf 72 700 Td (Revenue rose) Tj ET /Fm1 Do BT 72 650 Td (Costs fell by",
            ),
            plain("BT /F1 12 Tf 72 700 Td (Outlook stable) Tj ET"),
        ],
        vec![(100, form(Some(FORM_FONT), "BT /F1 12 Tf 72 700 Td (Inside) Tj ET (cut"))],
        "/XObject << /Fm1 100 0 R >>",
    );
    let f = stored("/docs/twice.pdf", &bytes).await;

    let v = mcp_extract(&f, "/docs/twice.pdf").await;

    assert_eq!(v["note"], note_of("2", false), "{v}");
}

/// The assistant tool door: `McpServer::fs_extract_text`.
async fn mcp_extract(f: &crate::tools::admin::test_support::Fixture, path: &str) -> Value {
    use crate::mcp::server::{ExtractTextArgs, McpServer};
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ContentBlock;
    let server = McpServer::new(f.state.clone(), OWNER.to_string());
    let args = ExtractTextArgs {
        mount_id: MOUNT.into(),
        path: path.into(),
        max_chars: 200_000,
        preview_chars: 4_000,
        ocr: true,
        refresh: false,
    };
    let r = server.fs_extract_text(Parameters(args)).await.expect("a tool result");
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text block");
    assert_ne!(r.is_error, Some(true), "{text}");
    serde_json::from_str(&text).expect("a json answer")
}

#[tokio::test]
async fn e2e_0094_010_damaged_page_and_no_text() {
    let f = stored("/docs/lone.pdf", &pdf_pages(vec![plain("(")], vec![], "")).await;
    let want = format!("{} {SCANNED}", note_of("1", false));

    let first = extract_text(&f, OWNER, "/docs/lone.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/lone.pdf", false).await.expect("extracts");

    assert_eq!(first["note"], want, "{first}");
    assert_eq!(second["note"], want, "{second}");
    assert_eq!(first["cached"], false, "{first}");
    assert_eq!(second["cached"], false, "{second}");
    let c = f.state.stores.client(MOUNT).await.unwrap();
    assert!(!c.exists("/docs/lone.md").await.unwrap());
}

#[tokio::test]
async fn e2e_0094_011_text_before_the_cut_is_kept_verbatim() {
    let mut content = b"BT /F1 12 Tf 72 700 Td (Bilan) Tj ET BT 72 650 Td (Co".to_vec();
    content.extend_from_slice("ût en baisse de".as_bytes());
    let obj = PdfObj::Stream(format!("<< /Length {} >>", content.len()), content);
    let f = stored("/docs/bilan.pdf", &pdf_pages(vec![obj], vec![], "")).await;

    let v = extract_text(&f, OWNER, "/docs/bilan.pdf", false).await.expect("extracts");

    assert_eq!(text_of(&v), body(&["Bilan"]), "{v}");
    assert_eq!(v["note"], note_of("1", false), "{v}");
}

#[tokio::test]
async fn e2e_0094_012_an_existing_refusal_wins() {
    let mut w = pages_writer(vec![plain("/A Do"), plain(CUT)], "/XObject << /A 10 0 R >>");
    for n in 10..75 {
        w.object(n, form(Some(&format!("/XObject << /A {} 0 R >>", n + 1)), "/A Do"));
    }
    w.object(75, form(Some(FORM_FONT), "BT /F1 12 Tf 72 700 Td (Bottom) Tj ET"));
    let f = stored("/nested-and-cut.pdf", &small(w.finish())).await;

    let v = extract_text(&f, OWNER, "/nested-and-cut.pdf", false).await;

    let Err(e) = &v else { panic!("the extraction must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        "could not extract /nested-and-cut.pdf: page 1 draws PDF forms nested more than 64 deep"
    );
    let c = f.state.stores.client(MOUNT).await.unwrap();
    assert!(!c.exists("/nested-and-cut.md").await.unwrap());
}

#[tokio::test]
async fn e2e_0094_014_both_doors_give_the_same_note() {
    let bytes = report_3p();
    let f = stored("/docs/report-3p.pdf", &bytes).await;
    let mcp = mcp_extract(&f, "/docs/report-3p.pdf").await;

    let d = tempfile::tempdir().unwrap();
    let (c, token) = crate::tools::tool_surface_e2e::setup(d.path());
    let (app, state) = crate::tools::tool_surface_e2e::app_with_atlas(c).await;
    let client = state.stores.client("atlas").await.unwrap();
    client.makedirs("/docs", true).await.unwrap();
    client.write_bytes_atomic("/docs/second.pdf", &bytes).await.unwrap();
    let rest = rest_extract(&app, &token, "/docs/second.pdf").await;

    assert_eq!(mcp["note"], note_of("2", false), "{mcp}");
    assert_eq!(rest["note"], note_of("2", false), "{rest}");
    assert_eq!(mcp["preview"], rest["preview"]);
}

#[tokio::test]
async fn e2e_0094_015_cut_in_the_second_part_of_a_page() {
    let mut w =
        pdf_writer(vec![plain(GOOD), plain("BT /F1 12 Tf 72 700 Td (First part) Tj ET")], "");
    w.object(
        12,
        PdfObj::Body(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [13 0 R 20 0 R] >>"
                .into(),
        ),
    );
    w.object(20, plain("BT 72 650 Td (Second pa"));
    let f = stored("/docs/parts.pdf", &small(w.finish())).await;

    let v = extract_text(&f, OWNER, "/docs/parts.pdf", false).await.expect("extracts");

    assert_eq!(v["note"], note_of("2", false), "{v}");
    assert!(text_of(&v).contains("First part"), "{v}");
}

#[tokio::test]
async fn e2e_0094_016_empty_page_and_trailing_blanks_are_clean() {
    let bytes = pdf_pages(
        vec![
            plain(GOOD),
            plain(""),
            plain("BT /F1 12 Tf 72 700 Td (Third page) Tj ET\n\n% end of page\n"),
        ],
        vec![],
        "",
    );
    let f = stored("/docs/blank.pdf", &bytes).await;

    let v = extract_text(&f, OWNER, "/docs/blank.pdf", false).await.expect("extracts");

    assert_eq!(v["note"], "", "{v}");
    // An empty page leaves no marker, but is still counted as a page.
    assert_eq!(
        text_of(&v),
        "---\n*[Page 1]*\n\nQuarterly results\n\n---\n*[Page 3]*\n\nThird page",
        "{v}"
    );
    assert_eq!(v["meta"]["pages"], 3, "{v}");
}

#[tokio::test]
async fn e2e_0094_017_unknown_operator_and_unclosed_block_are_clean() {
    let bytes = pdf_pages(
        vec![plain("BT /F1 12 Tf (Known) Tj ET Zz"), plain("q BT /F1 12 Tf (Open) Tj ET")],
        vec![],
        "",
    );
    let f = stored("/docs/loose.pdf", &bytes).await;

    let v = extract_text(&f, OWNER, "/docs/loose.pdf", false).await.expect("extracts");

    assert_eq!(v["note"], "", "{v}");
    assert_eq!(text_of(&v), body(&["Known", "Open"]), "{v}");
}

#[tokio::test]
async fn e2e_0094_018_stray_closing_marks_in_a_form() {
    let bytes = pdf_pages(
        vec![plain(GOOD), plain("/Fm1 Do")],
        vec![(100, form(Some(FORM_FONT), "BT /F1 12 Tf 72 700 Td (Fine) Tj ET >>"))],
        "/XObject << /Fm1 100 0 R >>",
    );
    let f = stored("/docs/stray.pdf", &bytes).await;

    let v = extract_text(&f, OWNER, "/docs/stray.pdf", false).await.expect("extracts");

    assert_eq!(v["note"], note_of("2", false), "{v}");
    assert!(text_of(&v).contains("Fine"), "{v}");
}

#[tokio::test]
async fn e2e_0094_019_a_damaged_page_beyond_the_text_asked_for() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;
    f.state.authorize(MOUNT, OWNER).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();

    let v = crate::core::fs_ops::extract_document(
        &c,
        &f.state.safety,
        &f.state.config.extract.ocr,
        OWNER,
        MOUNT,
        "/docs/report-3p.pdf",
        20,
        4_000,
        true,
        false,
    )
    .await
    .expect("extracts");

    assert_eq!(v["truncated"], true, "{v}");
    assert_eq!(v["note"], note_of("2", false), "{v}");
    assert_eq!(v["chars"], 20, "{v}");
}

#[tokio::test]
async fn e2e_0094_002_a_clean_pdf_is_untouched() {
    let bytes = pdf_pages(
        vec![
            plain("BT /F1 12 Tf 72 700 Td (Alpha) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Beta) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Gamma) Tj ET"),
        ],
        vec![],
        "",
    );
    let f = stored("/docs/clean-3p.pdf", &bytes).await;

    let first = extract_text(&f, OWNER, "/docs/clean-3p.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/clean-3p.pdf", false).await.expect("extracts");

    let want = body(&["Alpha", "Beta", "Gamma"]);
    assert_eq!(text_of(&first), want, "{first}");
    assert_eq!(text_of(&second), want, "{second}");
    assert_eq!(first["cached"], false, "{first}");
    assert_eq!(second["cached"], true, "{second}");
    assert_eq!(first["note"], "", "{first}");
    assert_eq!(second["note"], "", "{second}");
    for v in [&first, &second] {
        let note = v["note"].as_str().unwrap_or_else(|| panic!("no note in {v}"));
        assert!(!note.contains("damaged") && !note.contains("scanned"), "{v}");
    }
}

#[tokio::test]
async fn e2e_0094_004_twenty_five_damaged_pages() {
    let contents: Vec<PdfObj> = (0..25).map(|_| plain(CUT)).collect();
    let f = stored("/docs/many.pdf", &pdf_pages(contents, vec![], "")).await;

    let first = extract_text(&f, OWNER, "/docs/many.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/many.pdf", false).await.expect("extracts");

    let want = "pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, \
                14, 15, 16, 17, 18, 19, 20 and 5 more; the text may be incomplete";
    assert_eq!(first["note"], want, "{first}");
    assert_eq!(second["note"], want, "{second}");
    assert_eq!(second["cached"], true, "{second}");
}

#[tokio::test]
async fn e2e_0094_007_repeating_keeps_the_note() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;
    let want = note_of("2", false);

    let first = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");
    let third = extract_text(&f, OWNER, "/docs/report-3p.pdf", true).await.expect("extracts");

    assert_eq!(first["note"], want, "{first}");
    assert_eq!(second["cached"], true, "{second}");
    assert_eq!(second["note"], want, "{second}");
    assert_eq!(third["cached"], false, "{third}");
    assert_eq!(third["note"], want, "{third}");
}

#[tokio::test]
async fn e2e_0094_008_a_stored_copy_with_no_damage_record() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;
    let text = body(&["Quarterly results", "Revenue rose", "Outlook stable"]);
    seed(&f, "/docs/report-3p.md", text.as_bytes()).await;

    let first = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/report-3p.pdf", true).await.expect("extracts");

    assert_eq!(first["cached"], true, "{first}");
    assert_eq!(first.get("note"), Some(&Value::String(String::new())), "{first}");
    assert_eq!(second["cached"], false, "{second}");
    assert_eq!(second["note"], note_of("2", false), "{second}");
}

#[tokio::test]
async fn e2e_0094_020_the_note_does_not_depend_on_the_length_asked_for() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;
    f.state.authorize(MOUNT, OWNER).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let run = |max_chars: usize| {
        crate::core::fs_ops::extract_document(
            &c,
            &f.state.safety,
            &f.state.config.extract.ocr,
            OWNER,
            MOUNT,
            "/docs/report-3p.pdf",
            max_chars,
            4_000,
            true,
            false,
        )
    };

    let first = run(20).await.expect("extracts");
    let second = run(1000).await.expect("extracts");

    assert_eq!(first["cached"], false, "{first}");
    assert_eq!(second["cached"], true, "{second}");
    assert_eq!(second["chars"], 20, "{second}");
    assert_eq!(second["note"], note_of("2", false), "{second}");
}

#[tokio::test]
async fn e2e_0094_021_a_scan_keeps_its_own_message() {
    let f = stored("/docs/scan.pdf", &pdf_pages(vec![plain("")], vec![], "")).await;

    let first = extract_text(&f, OWNER, "/docs/scan.pdf", false).await.expect("extracts");
    let second = extract_text(&f, OWNER, "/docs/scan.pdf", false).await.expect("extracts");

    assert_eq!(first["note"], SCANNED, "{first}");
    assert_eq!(second["note"], SCANNED, "{second}");
}

#[tokio::test]
async fn e2e_0094_009_a_file_replaced_by_a_clean_one() {
    let f = stored("/docs/report-3p.pdf", &report_3p()).await;
    let first = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");
    assert_eq!(first["note"], note_of("2", false), "{first}");

    // The copy is served while its mtime is not older than the source's, so the
    // replacement must be strictly newer than the stored copy (extract.rs).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let clean = pdf_pages(
        vec![
            plain("BT /F1 12 Tf 72 700 Td (Alpha) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Beta) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Gamma) Tj ET"),
        ],
        vec![],
        "",
    );
    seed(&f, "/docs/report-3p.pdf", &clean).await;

    let plain_call = extract_text(&f, OWNER, "/docs/report-3p.pdf", false).await.expect("extracts");
    let refreshed = extract_text(&f, OWNER, "/docs/report-3p.pdf", true).await.expect("extracts");

    let want = body(&["Alpha", "Beta", "Gamma"]);
    for v in [&plain_call, &refreshed] {
        assert_eq!(text_of(v), want, "{v}");
        assert_eq!(v["note"], "", "{v}");
        assert_eq!(v["cached"], false, "{v}");
    }
}

#[tokio::test]
async fn e2e_0094_013_other_formats_are_untouched() {
    let f = nesting_fixture().await;
    seed(&f, "/docs/notes.txt", b"(unclosed").await;
    let memo = r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#;
    seed(&f, "/docs/memo.docx", &office_zip(&[("word/document.xml", memo.as_bytes())])).await;
    let sheet = r#"<worksheet><sheetData><row><c><v>42</v></c></row></sheetData></worksheet>"#;
    let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="Q1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let book = office_zip(&[
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    seed(&f, "/docs/sheet.xlsx", &book).await;

    let txt = extract_text(&f, OWNER, "/docs/notes.txt", false).await.expect("extracts");
    let docx = extract_text(&f, OWNER, "/docs/memo.docx", false).await.expect("extracts");
    let xlsx = extract_text(&f, OWNER, "/docs/sheet.xlsx", false).await.expect("extracts");
    let again = extract_text(&f, OWNER, "/docs/memo.docx", false).await.expect("extracts");

    for v in [&txt, &docx, &xlsx] {
        assert_eq!(v["note"], "", "{v}");
    }
    assert_eq!(text_of(&txt), "(unclosed", "{txt}");
    assert!(text_of(&docx).contains("Hello") && text_of(&xlsx).contains("42"), "{docx} {xlsx}");
    // A cached answer of a non PDF has the fixed cached shape, with no key
    // beyond those six and no `note`.
    let want = serde_json::json!({
        "path": docx["path"],
        "md_path": docx["md_path"],
        "format": "md",
        "chars": docx["chars"],
        "cached": true,
        "preview": docx["preview"],
    });
    assert_eq!(again, want, "{again}");
    assert!(again.get("note").is_none(), "{again}");
}
