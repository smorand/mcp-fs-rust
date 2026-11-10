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

/// The size lie refusal of `part` in `doc`, declaring `declared` bytes.
fn lie_refusal(v: &Result<Value>, doc: &str, part: &str, declared: u64) {
    let Err(e) = v else { panic!("the extraction must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!(
            "could not extract {doc}: document part '{part}' decodes to more bytes than declared {declared}"
        )
    );
}

#[tokio::test]
async fn e2e_0088_013_word_main_part_larger_than_it_declare() {
    let f = office_fixture().await;
    let doc = zip_declaring(&[("word/document.xml", 10, &vec![b'a'; 1_000_000])]);
    seed(&f, "/docs/liar.docx", &doc).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/liar.docx", false).await;

    lie_refusal(&v, "/docs/liar.docx", "word/document.xml", 10);
    let held = parts_held("/docs/liar.docx");
    assert!(held <= 65_546, "the extraction held {held} decoded bytes");
    nothing_left(&f, "/docs/liar.docx", None).await;
}

#[tokio::test]
async fn e2e_0088_015_slide_declared_0_holding_data() {
    let f = office_fixture().await;
    let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>hello</a:t></a:r></a:p></p:sld>"#;
    let doc = zip_declaring(&[("ppt/slides/slide1.xml", 0, slide.as_bytes())]);
    seed(&f, "/docs/zero.pptx", &doc).await;

    let v = extract_text(&f, OWNER, "/docs/zero.pptx", false).await;

    lie_refusal(&v, "/docs/zero.pptx", "ppt/slides/slide1.xml", 0);
}

#[tokio::test]
async fn e2e_0088_016_part_holding_exactly_its_declared_siz() {
    let f = office_fixture().await;
    let (open, close) =
        ("<w:document><w:body><w:p><w:r><w:t>exact", "</w:t></w:r></w:p></w:body></w:document>");
    let xml = format!("{open}{}{close}", " ".repeat(5_000 - open.len() - close.len()));
    assert_eq!(xml.len(), 5_000);
    seed(&f, "/docs/exact.docx", &office_zip(&[("word/document.xml", xml.as_bytes())])).await;

    let v = extract_text(&f, OWNER, "/docs/exact.docx", false).await.expect("declared size held");

    assert_eq!(v["preview"].as_str().unwrap().trim(), "exact", "{v}");
}

#[tokio::test]
async fn e2e_0088_022_header_inconsistency_wins_over_a_bomb() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    let doc = zip_raw(&[bomb_part("word/document.xml")]);
    let doc =
        crate::docs::zip_check::craft::rename_local(&doc, "word/document.xml", "word/documenX.xml");
    seed(&f, "/docs/twice.docx", &doc).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/twice.docx", false).await;

    let Err(e) = &v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert!(
        e.message.starts_with(
            "could not extract /docs/twice.docx: not a valid OOXML package: header inconsistency detected"
        ),
        "{}",
        e.message
    );
    assert_eq!(parts_held("/docs/twice.docx"), 0);
}

#[tokio::test]
async fn e2e_0088_023_shared_strings_named_before_a_sheet() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    let doc = zip_raw(&[
        bomb_part("xl/sharedStrings.xml"),
        raw_part("xl/workbook.xml", WORKBOOK.as_bytes()),
        raw_part("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        bomb_part("xl/worksheets/sheet1.xml"),
    ]);
    seed(&f, "/docs/two.xlsx", &doc).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/two.xlsx", false).await;

    part_refusal(&v, "/docs/two.xlsx", "xl/sharedStrings.xml", 200_000_000, 0);
}

#[tokio::test]
async fn e2e_0088_024_first_slide_lies_second_is_a_bomb() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    let doc = zip_raw(&[
        raw_part("ppt/presentation.xml", PRESENTATION.as_bytes()),
        raw_part("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS.as_bytes()),
        RawPart { declared: 10, ..raw_part("ppt/slides/slide1.xml", &[b' '; 1_000]) },
        bomb_part("ppt/slides/slide2.xml"),
    ]);
    seed(&f, "/docs/order.pptx", &doc).await;

    let v = extract_text(&f, OWNER, "/docs/order.pptx", false).await;

    lie_refusal(&v, "/docs/order.pptx", "ppt/slides/slide1.xml", 10);
}

#[tokio::test]
async fn e2e_0088_025_not_a_member() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/bomb.docx", &zip_raw(&[bomb_part("word/document.xml")])).await;
    office_held_reset();

    let denied = f.state.authorize(MOUNT, "eve@example.com").await;

    let Err(e) = &denied else { panic!("a non member must be refused") };
    assert_eq!(e.code, code::FORBIDDEN, "{e:?}");
    assert_eq!(e.message, "'eve@example.com' is not a member of 'atlas'");
    assert_eq!(parts_held("/docs/bomb.docx"), 0);
}

#[tokio::test]
async fn e2e_0088_026_missing_path() {
    let f = office_fixture().await;

    let v = extract_text(&f, OWNER, "/docs/none.docx", false).await;

    let Err(e) = &v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::NOT_FOUND, "{e:?}");
    assert_eq!(e.message, "not a file: /docs/none.docx");
}

/// The content limit of SPEC-0088 FR-NEW-005.
const CONTENT_LIMIT: u64 = 268_435_456;

/// Most content a call may hold: the content limit plus one part.
const CONTENT_HELD_MAX: u64 = 373_293_056;

const WORKBOOK_S: &str = r#"<workbook xmlns:r="r"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

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

/// The content limit refusal of `doc`.
fn content_refusal(v: &Result<Value>, doc: &str) {
    let Err(e) = v else { panic!("the extraction must be refused") };
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(
        e.message,
        format!("could not extract {doc}: the extracted content exceeds 268435456 bytes")
    );
}

#[tokio::test]
async fn e2e_0088_sec_011_one_shared_text_multiplied_by_400() {
    let _heavy = heavy().await;
    let f = office_fixture().await;
    seed(&f, "/docs/mult.xlsx", &repeated_text(b'a', 400)).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/mult.xlsx", false).await;

    let held = content_held("/docs/mult.xlsx");
    assert!(held <= CONTENT_HELD_MAX, "the extraction held {held} content bytes");
    content_refusal(&v, "/docs/mult.xlsx");
    assert!(held > CONTENT_LIMIT, "the counter saw the limit pass: {held}");
    nothing_left(&f, "/docs/mult.xlsx", None).await;
}

#[tokio::test]
async fn e2e_0088_020_content_just_under_the_content_limit() {
    let _heavy = heavy().await;
    let f = office_fixture_allowance(1_000_000_000).await;
    seed(&f, "/docs/long.xlsx", &repeated_text(b'd', 100)).await;
    office_held_reset();

    // The default 200 000 character cut would drop the table from the artifact
    // set (SPEC-0019 FR-NEW-034), so the whole text is asked for.
    f.state.authorize(MOUNT, OWNER).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let v = crate::core::fs_ops::extract_document(
        &c,
        &f.state.safety,
        &f.state.config.extract.ocr,
        OWNER,
        MOUNT,
        "/docs/long.xlsx",
        300_000_000,
        4_000,
        true,
        false,
    )
    .await
    .expect("within the limit");

    assert_eq!(v["format"], "xlsx", "{v}");
    assert!(content_held("/docs/long.xlsx") <= CONTENT_HELD_MAX);
    let tables =
        crate::tools::artifacts::list_tables(&f.state, MOUNT, "/docs/long.xlsx", None).await;
    let tables = tables.unwrap();
    let tables = tables["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 1, "{tables:?}");
    // The artifact counts the rows under the header row.
    assert_eq!(tables[0]["rows"], 99, "{tables:?}");
}

#[tokio::test]
async fn e2e_0088_021_content_over_the_content_limit() {
    let _heavy = heavy().await;
    let f = office_fixture_allowance(1_000_000_000).await;
    seed(&f, "/docs/long.xlsx", &repeated_text(b'd', 140)).await;
    office_held_reset();

    let v = extract_text(&f, OWNER, "/docs/long.xlsx", false).await;

    let held = content_held("/docs/long.xlsx");
    assert!(held <= CONTENT_HELD_MAX, "the extraction held {held} content bytes");
    content_refusal(&v, "/docs/long.xlsx");
    nothing_left(&f, "/docs/long.xlsx", None).await;
}

#[tokio::test]
async fn e2e_0088_001_word_document_within_the_limits() {
    let f = office_fixture().await;
    let body = r#"<w:document xmlns:w="w"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Results</w:t></w:r></w:p><w:p><w:r><w:t>Quarterly results</w:t></w:r></w:p></w:body></w:document>"#;
    seed(&f, "/docs/memo.docx", &office_zip(&[("word/document.xml", body.as_bytes())])).await;

    let v = extract_text(&f, OWNER, "/docs/memo.docx", false).await.unwrap();

    assert_eq!(v["md_path"], "/docs/memo.md", "{v}");
    assert_eq!(v["format"], "docx", "{v}");
    assert_eq!(v["cached"], false, "{v}");
    assert_eq!(v["preview"], "# Results\n\nQuarterly results", "{v}");
    assert_eq!(read(&f, "/docs/memo.md").await.unwrap(), b"# Results\n\nQuarterly results");
    let log = f.state.safety.audit(OWNER, MOUNT);
    let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_text").collect();
    assert_eq!(lines.len(), 1, "{log:?}");
    assert_eq!(lines[0].path, "/docs/memo.md");
}

#[tokio::test]
async fn e2e_0088_002_excel_workbook_within_the_limits_acce() {
    let f = office_fixture().await;
    let sheet = r#"<worksheet><sheetData><row><c t="inlineStr"><is><t>Poste</t></is></c><c t="inlineStr"><is><t>Montant</t></is></c></row><row><c t="s"><v>0</v></c><c><v>12</v></c></row></sheetData></worksheet>"#;
    let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="Q1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let doc = office_zip(&[
        ("xl/sharedStrings.xml", "<sst><si><t>Caf\u{e9}</t></si></sst>".as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    seed(&f, "/docs/budget.xlsx", &doc).await;

    let v = extract_text(&f, OWNER, "/docs/budget.xlsx", false).await.unwrap();

    let preview = v["preview"].as_str().unwrap();
    assert_eq!(
        preview, "## Sheet: Q1\n\n| Poste | Montant |\n| --- | --- |\n| Caf\u{e9} | 12 |",
        "{v}"
    );
    let tables = crate::tools::artifacts::list_tables(&f.state, MOUNT, "/docs/budget.xlsx", None)
        .await
        .unwrap();
    let tables = tables["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 1, "{tables:?}");
    assert_eq!(tables[0]["caption"], "Q1", "{tables:?}");
}

#[tokio::test]
async fn e2e_0088_003_powerpoint_deck_within_the_limits_wit() {
    let f = office_fixture().await;
    let slide = |t: &str| {
        format!(r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#)
    };
    let notes =
        r#"<p:notes xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Say hello</a:t></a:r></a:p></p:notes>"#;
    let doc = office_zip(&[
        ("ppt/presentation.xml", PRESENTATION.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS.as_bytes()),
        ("ppt/slides/slide1.xml", slide("Welcome").as_bytes()),
        ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS.as_bytes()),
        ("ppt/notesSlides/notesSlide1.xml", notes.as_bytes()),
        ("ppt/slides/slide2.xml", slide("Plan").as_bytes()),
    ]);
    seed(&f, "/docs/deck.pptx", &doc).await;

    let v = extract_text(&f, OWNER, "/docs/deck.pptx", false).await.unwrap();

    assert_eq!(
        v["preview"], "## Slide 1\n\nWelcome\n\n> Notes: Say hello\n\n## Slide 2\n\nPlan",
        "{v}"
    );
}
