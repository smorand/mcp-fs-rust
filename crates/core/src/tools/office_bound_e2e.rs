//! SPEC-0088 tests: an Office document part is refused on the size it declares
//! before it is decoded. Each exploit test carries its literal payload (a bomb
//! part of exactly 200 000 000 bytes, or a declared 2^63 or 2^64 minus 1) and
//! checks the second channels: the decoded bytes held through the hook, the
//! listing of `/docs`, the artifact lists, the audit trail and a following
//! 52 428 800 byte write. Every test takes `heavy()` first: before the fix a
//! bomb part is decoded whole.

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use serde_json::Value;

const PRESENTATION: &str = r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="256" r:id="rId1"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
const PRESENTATION_RELS: &str = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/slide" Target="slides/slide1.xml"/><Relationship Id="rId2" Type="http://x/slide" Target="slides/slide2.xml"/></Relationships>"#;
const INTRO: &str =
    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Intro</a:t></a:r></a:p></p:sld>"#;
const SLIDE_RELS: &str = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#;
const WORKBOOK: &str = r#"<workbook xmlns:r="r"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const WORKBOOK_RELS: &str = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
const SHEET: &str = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>cell</t></is></c></row></sheetData></worksheet>"#;

/// 2^63: the first size a signed cast turns negative.
const TWO_63: u64 = 9_223_372_036_854_775_808;

/// The part limit of SPEC-0088 FR-NEW-001.
const PART_LIMIT: usize = 104_857_600;

/// A document of small well formed parts and, at its place in `parts`, the bomb
/// part named `bomb`.
fn with_bomb(parts: &[(&str, &str)], bomb: &str) -> Vec<u8> {
    let raw: Vec<RawPart> = parts
        .iter()
        .map(
            |(name, xml)| {
                if *name == bomb { bomb_part(name) } else { raw_part(name, xml.as_bytes()) }
            },
        )
        .collect();
    let doc = zip_raw(&raw);
    assert!(
        doc.len() < 1_000_000,
        "a bomb document is stored in under 1 000 000 bytes: {}",
        doc.len()
    );
    doc
}

/// A presentation of slide 1 `Intro` (with the relationships of its notes) and
/// slide 2, with `bomb` as the bomb part.
fn deck(bomb: &str) -> Vec<u8> {
    with_bomb(
        &[
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", INTRO),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
            ("ppt/notesSlides/notesSlide1.xml", "<p:notes/>"),
            ("ppt/slides/slide2.xml", INTRO),
        ],
        bomb,
    )
}

/// A workbook of one sheet and its shared strings, with `bomb` as the bomb part.
fn book(bomb: &str) -> Vec<u8> {
    with_bomb(
        &[
            ("xl/sharedStrings.xml", "<sst><si><t>s</t></si></sst>"),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", SHEET),
        ],
        bomb,
    )
}

/// The part limit refusal of `part`, declaring `declared` bytes, in `doc`.
fn part_refusal(v: &Result<Value>, doc: &str, part: &str, declared: u64, held: u64) {
    let Err(e) = v else { panic!("the extraction must be refused; it decoded {held} part bytes") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!(
            "could not extract {doc}: document part '{part}' is {declared} bytes once decoded, over the limit of 104857600 bytes"
        )
    );
}

/// Store `bytes` at `doc`, extract it as the member, then check the part limit
/// refusal of `part`, at most `max_held` decoded bytes held, and nothing left.
async fn refused_on(doc: &str, bytes: &[u8], part: &str, declared: u64, max_held: u64) {
    let f = office_fixture().await;
    seed(&f, doc, bytes).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, doc, false).await;

    let held = parts_held(doc);
    part_refusal(&v, doc, part, declared, held);
    assert!(held <= max_held, "the extraction held {held} decoded bytes, more than {max_held}");
    nothing_left(&f, doc, None).await;
}

#[tokio::test]
async fn e2e_0088_sec_001_word_main_part_bomb() {
    let _heavy = heavy().await;
    let doc = with_bomb(&[("word/document.xml", "")], "word/document.xml");
    refused_on("/docs/bomb.docx", &doc, "word/document.xml", BOMB, 0).await;
}

#[tokio::test]
async fn e2e_0088_sec_002_powerpoint_slide_bomb() {
    let _heavy = heavy().await;
    refused_on(
        "/docs/bomb.pptx",
        &deck("ppt/slides/slide2.xml"),
        "ppt/slides/slide2.xml",
        BOMB,
        65_536,
    )
    .await;
}

#[tokio::test]
async fn e2e_0088_sec_003_powerpoint_notes_bomb() {
    let _heavy = heavy().await;
    let doc = deck("ppt/notesSlides/notesSlide1.xml");
    refused_on("/docs/notes.pptx", &doc, "ppt/notesSlides/notesSlide1.xml", BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_004_presentation_part_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/pres.pptx", &deck("ppt/presentation.xml"), "ppt/presentation.xml", BOMB, 0)
        .await;
}

#[tokio::test]
async fn e2e_0088_sec_005_presentation_relationships_bomb() {
    let _heavy = heavy().await;
    let part = "ppt/_rels/presentation.xml.rels";
    refused_on("/docs/prels.pptx", &deck(part), part, BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_006_slide_relationships_bomb() {
    let _heavy = heavy().await;
    let part = "ppt/slides/_rels/slide1.xml.rels";
    refused_on("/docs/srels.pptx", &deck(part), part, BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_007_excel_sheet_bomb() {
    let _heavy = heavy().await;
    let part = "xl/worksheets/sheet1.xml";
    refused_on("/docs/sheet.xlsx", &book(part), part, BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_008_shared_strings_bomb() {
    let _heavy = heavy().await;
    let part = "xl/sharedStrings.xml";
    refused_on("/docs/shared.xlsx", &book(part), part, BOMB, 0).await;
}

#[tokio::test]
async fn e2e_0088_sec_009_workbook_part_bomb() {
    let _heavy = heavy().await;
    refused_on("/docs/book.xlsx", &book("xl/workbook.xml"), "xl/workbook.xml", BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_010_workbook_relationships_bomb() {
    let _heavy = heavy().await;
    let part = "xl/_rels/workbook.xml.rels";
    refused_on("/docs/brels.xlsx", &book(part), part, BOMB, 65_536).await;
}

#[tokio::test]
async fn e2e_0088_sec_012_part_declaring_2_63_bytes() {
    let _heavy = heavy().await;
    let doc = zip_declaring(&[("word/document.xml", TWO_63, b"hello")]);
    refused_on("/docs/huge.docx", &doc, "word/document.xml", TWO_63, 0).await;
}

#[tokio::test]
async fn e2e_0088_sec_013_part_declaring_the_largest_size() {
    let _heavy = heavy().await;
    let doc = zip_declaring(&[
        ("xl/sharedStrings.xml", u64::MAX, b"hello"),
        ("xl/workbook.xml", WORKBOOK.len() as u64, WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.len() as u64, WORKBOOK_RELS.as_bytes()),
        ("xl/worksheets/sheet1.xml", SHEET.len() as u64, SHEET.as_bytes()),
    ]);
    refused_on("/docs/max.xlsx", &doc, "xl/sharedStrings.xml", u64::MAX, 0).await;
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

#[tokio::test]
async fn e2e_0088_006_part_of_exactly_the_part_limit() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/large.docx", &letters_docx(PART_LIMIT)).await;
    office_held_reset();

    let v =
        extract_text(&f, OWNER, "/docs/large.docx", false).await.expect("within the part limit");

    assert_eq!(v["truncated"], Value::Bool(true), "{}", v["note"]);
    let preview = v["preview"].as_str().unwrap();
    assert!(!preview.is_empty() && preview.bytes().all(|b| b == b'b'), "{preview:.80}");
    let held = parts_held("/docs/large.docx");
    assert!(held <= 104_923_136, "the extraction held {held} decoded bytes");
}

#[tokio::test]
async fn e2e_0088_007_part_one_byte_over_the_part_limit() {
    let _heavy = heavy().await;
    let doc = letters_docx(PART_LIMIT + 1);
    refused_on("/docs/large.docx", &doc, "word/document.xml", PART_LIMIT as u64 + 1, 0).await;
}

/// The builders: a bomb part decodes to exactly its declared size, checksum
/// verified by the zip reader, so a refusal is about the real payload.
#[tokio::test]
async fn bomb_part_decodes_to_exactly_its_declared_size() {
    use std::io::Read as _;
    let _heavy = heavy().await;
    let doc = deck("ppt/slides/slide2.xml");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(doc)).unwrap();
    let mut file = zip.by_name("ppt/slides/slide2.xml").unwrap();
    let mut out = Vec::new();
    file.read_to_end(&mut out).expect("valid stream and checksum");
    assert_eq!(out.len() as u64, BOMB);
    assert!(out.starts_with(b"<p:sld") && out.ends_with(b"</p:sld>"));
}

/// The call limit of SPEC-0088 FR-NEW-003.
const CALL_LIMIT: u64 = 268_435_456;

/// What the call limit refusal of `doc` says.
fn call_refusal(v: &Result<Value>, doc: &str) {
    let Err(e) = v else { panic!("the extraction must be refused") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!(
            "could not extract {doc}: the document parts read total more than 268435456 bytes once decoded"
        )
    );
}

/// One worksheet row of 10 numeric cells.
const TEN_CELLS: &str = r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c><c r="C1"><v>3</v></c><c r="D1"><v>4</v></c><c r="E1"><v>5</v></c><c r="F1"><v>6</v></c><c r="G1"><v>7</v></c><c r="H1"><v>8</v></c><c r="I1"><v>9</v></c><c r="J1"><v>10</v></c></row></sheetData></worksheet>"#;

/// A workbook of three sheets whose six parts declare `CALL_LIMIT + extra`
/// bytes together, the extra byte on the third sheet.
fn wide(extra: usize) -> Vec<u8> {
    let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="A" sheetId="1" r:id="rId1"/><sheet name="B" sheetId="2" r:id="rId2"/><sheet name="C" sheetId="3" r:id="rId3"/></sheets></workbook>"#;
    let rels = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://x/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://x/worksheet" Target="worksheets/sheet3.xml"/></Relationships>"#;
    let strings = "<sst><si><t>s</t></si></sst>";
    let small = strings.len() + workbook.len() + rels.len();
    let rest = CALL_LIMIT as usize - small;
    let each = rest / 3;
    let parts = vec![
        raw_part("xl/sharedStrings.xml", strings.as_bytes()),
        raw_part("xl/workbook.xml", workbook.as_bytes()),
        raw_part("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        padded_part("xl/worksheets/sheet1.xml", TEN_CELLS, each),
        padded_part("xl/worksheets/sheet2.xml", TEN_CELLS, each),
        padded_part("xl/worksheets/sheet3.xml", TEN_CELLS, rest - 2 * each + extra),
    ];
    let total: u64 = parts.iter().map(|p| p.declared).sum();
    assert_eq!(total, CALL_LIMIT + extra as u64);
    assert!(parts.iter().all(|p| p.declared < 100_000_000));
    zip_raw(&parts)
}

#[tokio::test]
async fn e2e_0088_008_parts_read_totalling_exactly_the_call() {
    let _heavy = heavy().await;
    let f = office_fixture_allowance(1_000_000_000).await;
    seed(&f, "/docs/wide.xlsx", &wide(0)).await;

    let v = extract_text(&f, OWNER, "/docs/wide.xlsx", false).await.expect("within the call limit");

    assert_eq!(v["meta"]["sheets"], serde_json::json!(3), "{v}");
}

#[tokio::test]
async fn e2e_0088_009_parts_read_one_byte_over_the_call_lim() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/wide.xlsx", &wide(1)).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/wide.xlsx", false).await;

    call_refusal(&v, "/docs/wide.xlsx");
    let held = parts_held("/docs/wide.xlsx");
    assert!(held <= CALL_LIMIT + 65_536, "the extraction held {held} decoded bytes");
    nothing_left(&f, "/docs/wide.xlsx", None).await;
}

#[tokio::test]
async fn e2e_0088_010_many_slides_together_over_the_call_li() {
    let _heavy = heavy().await;
    let ids: String =
        (1..=30).map(|n| format!(r#"<p:sldId id="{}" r:id="rId{n}"/>"#, 255 + n)).collect();
    let rels: String = (1..=30)
        .map(|n| {
            format!(
                r#"<Relationship Id="rId{n}" Type="http://x/slide" Target="slides/slide{n}.xml"/>"#
            )
        })
        .collect();
    let mut parts = vec![
        raw_part(
            "ppt/presentation.xml",
            format!(r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"#)
                .as_bytes(),
        ),
        raw_part(
            "ppt/_rels/presentation.xml.rels",
            format!(r#"<Relationships xmlns="x">{rels}</Relationships>"#).as_bytes(),
        ),
    ];
    for n in 1..=30 {
        parts.push(padded_part(
            &format!("ppt/slides/slide{n}.xml"),
            r#"<p:sld xmlns:p="p" xmlns:a="a"/>"#,
            10_000_000,
        ));
    }
    let f = office_fixture().await;
    seed(&f, "/docs/many.pptx", &zip_raw(&parts)).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/many.pptx", false).await;

    call_refusal(&v, "/docs/many.pptx");
    let held = parts_held("/docs/many.pptx");
    assert!(held <= CALL_LIMIT + 65_536, "the extraction held {held} decoded bytes");
    nothing_left(&f, "/docs/many.pptx", None).await;
}

#[tokio::test]
async fn e2e_0088_011_one_sheet_listed_three_times() {
    let _heavy = heavy().await;
    let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="A" sheetId="1" r:id="rId1"/><sheet name="B" sheetId="2" r:id="rId2"/><sheet name="C" sheetId="3" r:id="rId3"/></sheets></workbook>"#;
    let target = r#"Type="http://x/worksheet" Target="worksheets/sheet1.xml""#;
    let rels = format!(
        r#"<Relationships xmlns="x"><Relationship Id="rId1" {target}/><Relationship Id="rId2" {target}/><Relationship Id="rId3" {target}/></Relationships>"#
    );
    let doc = zip_raw(&[
        raw_part("xl/workbook.xml", workbook.as_bytes()),
        raw_part("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        padded_part("xl/worksheets/sheet1.xml", TEN_CELLS, 100_000_000),
    ]);
    let f = office_fixture().await;
    seed(&f, "/docs/repeat.xlsx", &doc).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/repeat.xlsx", false).await;

    call_refusal(&v, "/docs/repeat.xlsx");
    nothing_left(&f, "/docs/repeat.xlsx", None).await;
}

#[tokio::test]
async fn e2e_0088_012_many_small_slides_within_the_limits() {
    let ids: String =
        (1..=500).map(|n| format!(r#"<p:sldId id="{}" r:id="rId{n}"/>"#, 255 + n)).collect();
    let rels: String = (1..=500)
        .map(|n| {
            format!(
                r#"<Relationship Id="rId{n}" Type="http://x/slide" Target="slides/slide{n}.xml"/>"#
            )
        })
        .collect();
    let presentation = format!(
        r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"#
    );
    let rels = format!(r#"<Relationships xmlns="x">{rels}</Relationships>"#);
    let slides: Vec<(String, String)> = (1..=500)
        .map(|n| {
            (
                format!("ppt/slides/slide{n}.xml"),
                format!(
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Slide text {n}</a:t></a:r></a:p></p:sld>"#
                ),
            )
        })
        .collect();
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("ppt/presentation.xml", presentation.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", rels.as_bytes()),
    ];
    entries.extend(slides.iter().map(|(n, x)| (n.as_str(), x.as_bytes())));
    let f = office_fixture().await;
    seed(&f, "/docs/talk.pptx", &office_zip(&entries)).await;

    let v = extract_text(&f, OWNER, "/docs/talk.pptx", false).await.expect("within the limits");

    assert_eq!(v["meta"]["slides"], serde_json::json!(500), "{v}");
    assert!(v["preview"].as_str().unwrap().starts_with("## Slide 1\n\nSlide text 1"), "{v}");
}
