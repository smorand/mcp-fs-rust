//! Document to Markdown extraction. Port of the C# `Core/Extract.cs` plus the
//! `FsOps.ExtractDocument` orchestration (companion `.md` caching).
//!
//! Format support and the crates behind it:
//!
//! | format                          | backend                                  |
//! |---------------------------------|------------------------------------------|
//! | PDF                             | `pdf-extract` (text layer, per page)     |
//! | DOCX / PPTX / XLSX              | `zip` + `quick-xml` scan of the OOXML    |
//! | HTML                            | hand rolled tag stripper (no html crate) |
//! | CSV                             | hand rolled RFC 4180 parser              |
//! | text, markdown, json, yaml, ... | decoded, optionally fenced               |
//! | images                          | the configured `OcrProvider`             |
//!
//! NOT supported, by design and exactly like the C#: audio and video. Those
//! return `ERR_NOT_SUPPORTED`; transcription needs a speech model and belongs
//! outside a filesystem server. Legacy binary Office formats (`.doc`, `.xls`,
//! `.ppt`) are not OOXML, so they fall through to the text decoder with a note,
//! which is also what the C# does.
//!
//! Deviation from the C#: an unsupported format answers `ERR_NOT_SUPPORTED`
//! instead of the C# `ERR_INVALID_ARGUMENT`. The message text is unchanged, and
//! the code now says what actually happened.

use crate::docs::ocr::OcrProvider;
use crate::errors::{Result, ToolError};
use crate::storage::VolumeClient;
use serde_json::{Map, Value, json};
use std::io::Read;

/// Extensions decoded as plain text.
const TEXT_EXTS: &[&str] = &[".txt", ".md", ".markdown", ".rst", ".log", ".text"];
/// Extensions wrapped in a fenced code block, tagged with the extension.
const FENCED_EXTS: &[&str] = &[".json", ".yaml", ".yml", ".xml", ".toml", ".ini", ".env"];
/// Extensions routed to the OCR provider.
const IMAGE_EXTS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp"];
/// Audio and video: out of scope.
const AV_EXTS: &[&str] = &[
    ".mp3", ".wav", ".m4a", ".ogg", ".flac", ".aac", ".mp4", ".mkv", ".mov", ".avi", ".webm", ".wmv",
];
/// Extensions that get a companion `.md` written next to the source. A `.txt` is
/// already readable with `fs.read`, so it gets no companion.
const MD_COMPANION_EXTS: &[&str] = &[
    ".pdf", ".docx", ".pptx", ".pptm", ".potx", ".ppsx", ".xlsx", ".xlsm", ".html", ".htm", ".csv",
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp",
];
/// Row cap for generated Markdown tables, matching the C# `TableRowCap`.
const TABLE_ROW_CAP: usize = 400;

/// Outcome of a document extraction. Mirrors the C# `ExtractResult`.
#[derive(Debug, Clone, Default)]
pub struct ExtractResult {
    pub fmt: String,
    pub text: String,
    pub truncated: bool,
    pub meta: Map<String, Value>,
    pub note: String,
}

impl ExtractResult {
    fn of(fmt: &str, text: String) -> Self {
        Self { fmt: fmt.to_string(), text, ..Default::default() }
    }

    fn with_meta(mut self, key: &str, value: Value) -> Self {
        self.meta.insert(key.to_string(), value);
        self
    }

    fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// The extraction engine. Borrows the OCR provider so the composition root owns
/// it and a single instance serves every request.
pub struct Extractor<'a> {
    ocr: &'a dyn OcrProvider,
}

impl<'a> Extractor<'a> {
    pub fn new(ocr: &'a dyn OcrProvider) -> Self {
        Self { ocr }
    }

    /// Extract `data` named `filename` to Markdown, capped at `max_chars`.
    pub async fn extract(
        &self,
        data: &[u8],
        filename: &str,
        max_chars: usize,
        ocr_enabled: bool,
    ) -> Result<ExtractResult> {
        let ext = extension_of(filename);
        if AV_EXTS.contains(&ext.as_str()) {
            return Err(ToolError::not_supported(format!(
                "audio/video is out of scope for extraction: {ext}"
            )));
        }

        let mut result = match ext.as_str() {
            ".pdf" => pdf(data),
            ".docx" => docx(data),
            ".pptx" | ".pptm" | ".potx" | ".ppsx" => pptx(data),
            ".xlsx" | ".xlsm" => xlsx(data),
            ".html" | ".htm" => Ok(html(data)),
            ".csv" => Ok(csv(data)),
            e if IMAGE_EXTS.contains(&e) => self.image(data, e, ocr_enabled).await,
            e if FENCED_EXTS.contains(&e) => Ok(fenced(data, e)),
            e if TEXT_EXTS.contains(&e) || e.is_empty() => Ok(ExtractResult::of("text", decode(data))),
            e => Ok(ExtractResult::of("text", decode(data))
                .with_note(format!("unknown extension {e}; decoded as text"))),
        }?;

        let (text, truncated) = truncate_chars(&result.text, max_chars);
        result.text = text;
        result.truncated = truncated;
        Ok(result)
    }

    async fn image(&self, data: &[u8], ext: &str, ocr_enabled: bool) -> Result<ExtractResult> {
        if ocr_enabled && self.ocr.enabled() {
            let mime = match ext {
                ".png" => "image/png",
                ".gif" => "image/gif",
                ".bmp" => "image/bmp",
                ".webp" => "image/webp",
                ".tif" | ".tiff" => "image/tiff",
                _ => "image/jpeg",
            };
            let text = self.ocr.extract_text(data, mime).await?.trim().to_string();
            if !text.is_empty() {
                return Ok(ExtractResult::of("image", text)
                    .with_note("text recovered via multimodal OCR provider"));
            }
        }
        let note = if self.ocr.enabled() {
            "image: the OCR provider returned no text"
        } else {
            "image: no OCR text. Configure extract.ocr.provider=multimodal to enable image understanding"
        };
        Ok(ExtractResult::of("image", String::new()).with_note(note))
    }
}

// ── orchestration (port of FsOps.ExtractDocument) ─────────────────────────────

/// Companion Markdown path for a source document: `report.pdf` -> `report.md`.
/// Matches the C# `FsOps.CompanionPath`, including the case of a dot that lives
/// in a parent directory name (then the whole path gets `.md` appended).
pub fn companion_md_path(path: &str) -> String {
    let dot = path.rfind('.');
    let slash = path.rfind('/');
    let stem = match (dot, slash) {
        (Some(d), Some(s)) if d > s => &path[..d],
        (Some(d), None) => &path[..d],
        _ => path,
    };
    format!("{stem}.md")
}

/// Extract a document to Markdown, store it as a companion `.md` next to the
/// source and return `md_path` plus a bounded preview.
///
/// The companion is reused when its mtime is at least the source mtime, unless
/// `refresh` is set. The caller (the tool layer) owns the safety accounting:
/// charge the write, record the read on `md_path` and audit, exactly like the C#
/// `FsOps.ExtractDocument` does around this engine.
pub async fn extract_text(
    client: &VolumeClient,
    ocr: &dyn OcrProvider,
    path: &str,
    max_chars: usize,
    preview_chars: usize,
    ocr_enabled: bool,
    refresh: bool,
) -> Result<Value> {
    if !client.is_file(path).await? {
        return Err(ToolError::not_found(format!("not a file: {path}")));
    }
    let ext = extension_of(path);
    let mut md_path: Option<String> = if MD_COMPANION_EXTS.contains(&ext.as_str()) {
        Some(companion_md_path(path))
    } else {
        None
    };

    if let Some(md) = md_path.as_deref()
        && !refresh
        && client.exists(md).await?
        && client.stat(md).await?.mtime >= client.stat(path).await?.mtime
    {
        let cached = client.read_text(md).await?;
        return Ok(doc_payload(path, Some(md), "md", &cached, preview_chars, true));
    }

    let data = client.read_bytes(path).await?;
    let extractor = Extractor::new(ocr);
    let result = match extractor.extract(&data, path, max_chars, ocr_enabled).await {
        Ok(r) => r,
        Err(e) if e.code == crate::errors::code::NOT_SUPPORTED => return Err(e),
        Err(e) => {
            return Err(ToolError::invalid_argument(format!(
                "could not extract {path}: {}",
                e.message
            )));
        }
    };

    // An empty extraction (a scanned PDF with no OCR, for instance) must not
    // create an empty companion: there would be nothing to read and the stale
    // file would then satisfy the mtime check forever.
    if let Some(md) = md_path.as_deref() {
        if result.text.trim().is_empty() {
            md_path = None;
        } else {
            client.write_bytes_atomic(md, result.text.as_bytes()).await?;
        }
    }

    let mut payload = doc_payload(path, md_path.as_deref(), &result.fmt, &result.text, preview_chars, false);
    if let Value::Object(map) = &mut payload {
        map.insert("truncated".into(), Value::Bool(result.truncated));
        map.insert("meta".into(), Value::Object(result.meta));
        map.insert("note".into(), Value::String(result.note));
    }
    Ok(payload)
}

fn doc_payload(
    source: &str,
    md_path: Option<&str>,
    fmt: &str,
    text: &str,
    preview_chars: usize,
    cached: bool,
) -> Value {
    json!({
        "path": source,
        "md_path": md_path,
        "format": fmt,
        "chars": text.chars().count(),
        "cached": cached,
        "preview": take_chars(text, preview_chars),
    })
}

// ── shared helpers ───────────────────────────────────────────────────────────

/// Lowercased extension including the dot, or "" when the path has no dot.
/// Deliberately scans the WHOLE path like the C# does, so behaviour matches even
/// for the odd `/dir.d/README` case.
fn extension_of(filename: &str) -> String {
    match filename.rfind('.') {
        None => String::new(),
        Some(i) => filename[i..].to_lowercase(),
    }
}

/// UTF-8 decode that never fails, replacing invalid sequences. Same contract as
/// the C# `UTF8Encoding(false, false)`.
fn decode(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

/// Truncate to `max` characters, reporting whether anything was cut. Character
/// based (not byte based) so a cut never lands mid codepoint.
fn truncate_chars(text: &str, max: usize) -> (String, bool) {
    // The byte offset of character number `max` is exactly the cut point; no such
    // character means the text was already short enough.
    match text.char_indices().nth(max) {
        Some((i, _)) => (text[..i].to_string(), true),
        None => (text.to_string(), false),
    }
}

fn take_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Render rows as a GitHub Markdown table. Port of the C# `MdTable`: newlines
/// collapse to spaces, pipes are escaped, short rows are padded.
fn md_table(rows: &[Vec<String>]) -> String {
    let mut cleaned: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| c.replace('\n', " ").replace('|', "\\|").trim().to_string()).collect())
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    let width = cleaned.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut cleaned {
        while row.len() < width {
            row.push(String::new());
        }
    }
    let mut out = format!("| {} |\n", cleaned[0].join(" | "));
    out.push_str(&format!("| {} |", vec!["---"; width].join(" | ")));
    for row in cleaned.iter().skip(1) {
        out.push_str(&format!("\n| {} |", row.join(" | ")));
    }
    out
}

// ── PDF ──────────────────────────────────────────────────────────────────────

fn pdf(data: &[u8]) -> Result<ExtractResult> {
    // pdf-extract panics on some malformed content streams, and a panic in a
    // request handler would take down the whole worker, so it is contained here.
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(data))
        .map_err(|_| ToolError::invalid_argument("pdf parser failed on this document"))?
        .map_err(|e| ToolError::invalid_argument(format!("pdf: {e}")))?;

    let mut parts = String::new();
    for (i, text) in pages.iter().enumerate() {
        if !text.trim().is_empty() {
            parts.push_str(&format!("\n\n---\n*[Page {}]*\n\n{}", i + 1, text.trim()));
        }
    }
    let body = parts.trim().to_string();
    let mut result = ExtractResult::of("pdf", body).with_meta("pages", json!(pages.len()));
    if let Some(title) = pdf_title(data).filter(|t| !t.is_empty()) {
        result = result.with_meta("title", json!(title));
    }
    if result.text.is_empty() {
        result = result
            .with_note("no extractable text layer (scanned PDF?); enable a multimodal OCR provider");
    }
    Ok(result)
}

/// Best effort `/Info /Title`. Absent or unreadable metadata is not an error:
/// the C# also just omits the key.
fn pdf_title(data: &[u8]) -> Option<String> {
    let doc = std::panic::catch_unwind(|| pdf_extract::Document::load_mem(data)).ok()?.ok()?;
    let info = doc.trailer.get(b"Info").ok()?;
    let dict = match info {
        pdf_extract::Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        pdf_extract::Object::Dictionary(d) => d,
        _ => return None,
    };
    let raw = dict.get(b"Title").ok()?.as_str().ok()?;
    Some(decode_pdf_text(raw))
}

/// PDF text strings are either UTF-16BE with a BOM or PDFDocEncoding, which
/// coincides with Latin-1 for everything we care about.
fn decode_pdf_text(raw: &[u8]) -> String {
    if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
        let units: Vec<u16> = raw[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    raw.iter().map(|b| *b as char).collect()
}

// ── OOXML shared plumbing ────────────────────────────────────────────────────

type Zip = zip::ZipArchive<std::io::Cursor<Vec<u8>>>;

fn open_zip(data: &[u8]) -> Result<Zip> {
    zip::ZipArchive::new(std::io::Cursor::new(data.to_vec()))
        .map_err(|e| ToolError::invalid_argument(format!("not a valid OOXML package: {e}")))
}

/// Read one zip entry as text, or `None` when the entry is absent.
fn entry_text(zip: &mut Zip, name: &str) -> Option<String> {
    let mut file = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(decode(&buf))
}

fn entry_names(zip: &Zip) -> Vec<String> {
    zip.file_names().map(str::to_string).collect()
}

/// XML text content. No unescaping here: quick-xml surfaces entity and character
/// references as separate `GeneralRef` events (see `ref_text`), so a text event
/// is already literal content.
fn event_text(e: &quick_xml::events::BytesText<'_>) -> String {
    e.decode().unwrap_or_default().into_owned()
}

/// Text carried by an entity or character reference event. quick-xml reports
/// `&amp;` and `&#233;` as their own events rather than folding them into the
/// surrounding text, so every reader has to reassemble them.
fn ref_text(r: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    let name = String::from_utf8_lossy(r.as_ref()).into_owned();
    match name.as_str() {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        // An entity we cannot resolve is kept verbatim: losing it silently would
        // corrupt the extracted text without any trace.
        other => format!("&{other};"),
    }
}

fn local_name(e: &quick_xml::events::BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn local_name_end(e: &quick_xml::events::BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

/// Attribute value by local name (`w:val` matches `val`), unescaped.
fn attr(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.local_name().as_ref() == name.as_bytes() {
            return a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|c| c.into_owned());
        }
    }
    None
}

/// Value of the namespace prefixed relationship id (`r:id`). Matched on the
/// PREFIXED key on purpose: `p:sldId` also carries a plain `id` attribute, so a
/// local name comparison would pick the wrong one.
fn rel_id_attr(e: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    for a in e.attributes().flatten() {
        let key = a.key.as_ref();
        if key.ends_with(b":id")
            && let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
        {
            return Some(v.into_owned());
        }
    }
    None
}

/// One OPC relationship.
struct Rel {
    id: String,
    rel_type: String,
    target: String,
}

fn parse_rels(xml: &str) -> Vec<Rel> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if local_name(&e) == "Relationship" {
                    out.push(Rel {
                        id: attr(&e, "Id").unwrap_or_default(),
                        rel_type: attr(&e, "Type").unwrap_or_default(),
                        target: attr(&e, "Target").unwrap_or_default(),
                    });
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

/// Resolve a relationship target against the part directory, collapsing `..`.
fn resolve_target(base_dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut segments: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}

/// Concatenated text of every element with the given local name.
fn all_text_of(xml: &str, tag: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) if local_name(&e) == tag => depth += 1,
            Ok(quick_xml::events::Event::End(e)) if local_name_end(&e) == tag => {
                depth = depth.saturating_sub(1)
            }
            Ok(quick_xml::events::Event::Text(t)) if depth > 0 => out.push_str(&event_text(&t)),
            Ok(quick_xml::events::Event::GeneralRef(r)) if depth > 0 => out.push_str(&ref_text(&r)),
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

// ── DOCX ─────────────────────────────────────────────────────────────────────

/// Where the current text events go while walking `word/document.xml`.
enum DocxMode {
    Idle,
    Paragraph { style: String, text: String },
    Table { depth: usize, rows: Vec<Vec<String>>, row: Vec<String>, cell: String, in_cell: bool },
}

fn docx(data: &[u8]) -> Result<ExtractResult> {
    let mut zip = open_zip(data)?;
    let xml = entry_text(&mut zip, "word/document.xml")
        .ok_or_else(|| ToolError::invalid_argument("docx: word/document.xml is missing"))?;

    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut lines: Vec<String> = Vec::new();
    let mut para_count = 0usize;
    let mut mode = DocxMode::Idle;
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = local_name(&e);
                match (&mut mode, name.as_str()) {
                    (DocxMode::Idle, "p") => {
                        mode = DocxMode::Paragraph { style: String::new(), text: String::new() }
                    }
                    (DocxMode::Idle, "tbl") => {
                        mode = DocxMode::Table {
                            depth: 1,
                            rows: Vec::new(),
                            row: Vec::new(),
                            cell: String::new(),
                            in_cell: false,
                        }
                    }
                    (DocxMode::Paragraph { style, .. }, "pStyle") => {
                        if let Some(v) = attr(&e, "val") {
                            *style = v;
                        }
                    }
                    (DocxMode::Table { depth, .. }, "tbl") => *depth += 1,
                    (DocxMode::Table { depth, row, .. }, "tr") if *depth == 1 => row.clear(),
                    (DocxMode::Table { depth, cell, in_cell, .. }, "tc") if *depth == 1 => {
                        cell.clear();
                        *in_cell = true;
                    }
                    (_, "t") => in_text = true,
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Empty(e)) => match local_name(&e).as_str() {
                // A self closing pStyle is the common shape produced by Word.
                "pStyle" => {
                    if let DocxMode::Paragraph { style, .. } = &mut mode
                        && let Some(v) = attr(&e, "val")
                    {
                        *style = v;
                    }
                }
                // `<w:p/>` is an empty paragraph: it contributes nothing to the
                // text but the C# counts it, so the meta stays comparable.
                "p" => {
                    if matches!(mode, DocxMode::Idle) {
                        para_count += 1;
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                docx_push_text(&mut mode, &event_text(&t))
            }
            Ok(quick_xml::events::Event::GeneralRef(r)) if in_text => {
                docx_push_text(&mut mode, &ref_text(&r))
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let name = local_name_end(&e);
                match name.as_str() {
                    "t" => in_text = false,
                    "p" => {
                        if let DocxMode::Paragraph { style, text } = &mode {
                            para_count += 1;
                            if let Some(line) = docx_paragraph_line(style, text.trim()) {
                                lines.push(line);
                            }
                            mode = DocxMode::Idle;
                        }
                    }
                    "tc" => {
                        if let DocxMode::Table { depth, row, cell, in_cell, .. } = &mut mode
                            && *depth == 1
                        {
                            row.push(std::mem::take(cell));
                            *in_cell = false;
                        }
                    }
                    "tr" => {
                        if let DocxMode::Table { depth, rows, row, .. } = &mut mode
                            && *depth == 1
                            && rows.len() < TABLE_ROW_CAP
                        {
                            rows.push(std::mem::take(row));
                        }
                    }
                    "tbl" => {
                        if let DocxMode::Table { depth, rows, .. } = &mut mode {
                            *depth -= 1;
                            if *depth == 0 {
                                let rows = std::mem::take(rows);
                                if !rows.is_empty() {
                                    lines.push(String::new());
                                    lines.push(md_table(&rows));
                                }
                                mode = DocxMode::Idle;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }

    Ok(ExtractResult::of("docx", lines.join("\n\n").trim().to_string())
        .with_meta("paragraphs", json!(para_count)))
}

/// Route a text chunk to the paragraph or the table cell being built.
fn docx_push_text(mode: &mut DocxMode, chunk: &str) {
    match mode {
        DocxMode::Paragraph { text, .. } => text.push_str(chunk),
        DocxMode::Table { cell, in_cell, .. } if *in_cell => cell.push_str(chunk),
        _ => {}
    }
}

/// Map a Word paragraph style to Markdown, like the C#: `Heading{n}` becomes an
/// ATX heading, `List*` becomes a bullet, anything else stays plain.
fn docx_paragraph_line(style: &str, text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let lower = style.to_ascii_lowercase();
    if lower.starts_with("heading") {
        let digits: String = style.chars().filter(char::is_ascii_digit).collect();
        let level = digits.parse::<usize>().unwrap_or(1).clamp(1, 6);
        return Some(format!("{} {text}", "#".repeat(level)));
    }
    if lower.starts_with("list") {
        return Some(format!("- {text}"));
    }
    Some(text.to_string())
}

// ── PPTX ─────────────────────────────────────────────────────────────────────

fn pptx(data: &[u8]) -> Result<ExtractResult> {
    let mut zip = open_zip(data)?;
    let slides = pptx_slide_order(&mut zip);
    let mut lines: Vec<String> = Vec::new();
    for (index, slide_part) in slides.iter().enumerate() {
        let Some(xml) = entry_text(&mut zip, slide_part) else { continue };
        lines.push(format!("## Slide {}", index + 1));
        for para in drawing_paragraphs(&xml) {
            if !para.is_empty() {
                lines.push(para);
            }
        }
        if let Some(notes_part) = pptx_notes_part(&mut zip, slide_part)
            && let Some(notes_xml) = entry_text(&mut zip, &notes_part)
        {
            let notes = all_text_of(&notes_xml, "t").trim().to_string();
            if !notes.is_empty() {
                lines.push(format!("> Notes: {notes}"));
            }
        }
    }
    Ok(ExtractResult::of("pptx", lines.join("\n\n").trim().to_string())
        .with_meta("slides", json!(slides.len())))
}

/// Presentation order of the slide parts. Read from `p:sldIdLst` plus the
/// presentation relationships, because zip entry order and file numbering both
/// lie about the real order after slides are reordered in PowerPoint.
fn pptx_slide_order(zip: &mut Zip) -> Vec<String> {
    let ordered = (|| {
        let presentation = entry_text(zip, "ppt/presentation.xml")?;
        let rels = parse_rels(&entry_text(zip, "ppt/_rels/presentation.xml.rels")?);
        let mut reader = quick_xml::Reader::from_str(&presentation);
        let mut ids = Vec::new();
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                    if local_name(&e) == "sldId"
                        && let Some(id) = rel_id_attr(&e)
                    {
                        ids.push(id);
                    }
                }
                Ok(quick_xml::events::Event::Eof) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let parts: Vec<String> = ids
            .iter()
            .filter_map(|id| rels.iter().find(|r| &r.id == id))
            .map(|r| resolve_target("ppt", &r.target))
            .collect();
        if parts.is_empty() { None } else { Some(parts) }
    })();
    if let Some(parts) = ordered {
        return parts;
    }
    // Fallback: numeric order of slideN.xml.
    let mut names: Vec<String> = entry_names(zip)
        .into_iter()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    names.sort_by_key(|n| slide_number(n));
    names
}

fn slide_number(name: &str) -> u32 {
    name.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)
}

fn pptx_notes_part(zip: &mut Zip, slide_part: &str) -> Option<String> {
    let (dir, file) = slide_part.rsplit_once('/')?;
    let rels_path = format!("{dir}/_rels/{file}.rels");
    let rels = parse_rels(&entry_text(zip, &rels_path)?);
    let rel = rels.iter().find(|r| r.rel_type.ends_with("notesSlide"))?;
    Some(resolve_target(dir, &rel.target))
}

/// Text of each `a:p` in a DrawingML part, one string per paragraph.
fn drawing_paragraphs(xml: &str) -> Vec<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => match local_name(&e).as_str() {
                "p" => current = Some(String::new()),
                "t" => in_text = true,
                _ => {}
            },
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                if let Some(buf) = current.as_mut() {
                    buf.push_str(&event_text(&t));
                }
            }
            Ok(quick_xml::events::Event::GeneralRef(r)) if in_text => {
                if let Some(buf) = current.as_mut() {
                    buf.push_str(&ref_text(&r));
                }
            }
            Ok(quick_xml::events::Event::End(e)) => match local_name_end(&e).as_str() {
                "t" => in_text = false,
                "p" => {
                    if let Some(buf) = current.take() {
                        out.push(buf.trim().to_string());
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

// ── XLSX ─────────────────────────────────────────────────────────────────────

fn xlsx(data: &[u8]) -> Result<ExtractResult> {
    let mut zip = open_zip(data)?;
    let shared = xlsx_shared_strings(&mut zip);
    let sheets = xlsx_sheets(&mut zip);
    let mut lines: Vec<String> = Vec::new();
    for (name, part) in &sheets {
        let Some(xml) = entry_text(&mut zip, part) else { continue };
        let rows = xlsx_rows(&xml, &shared);
        if rows.is_empty() {
            continue;
        }
        lines.push(format!("## Sheet: {name}"));
        lines.push(md_table(&rows));
    }
    Ok(ExtractResult::of("xlsx", lines.join("\n\n").trim().to_string())
        .with_meta("sheets", json!(sheets.len())))
}

/// The shared string table, one entry per `si` (concatenating its rich text runs).
fn xlsx_shared_strings(zip: &mut Zip) -> Vec<String> {
    let Some(xml) = entry_text(zip, "xl/sharedStrings.xml") else { return Vec::new() };
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => match local_name(&e).as_str() {
                "si" => current = Some(String::new()),
                "t" => in_text = true,
                _ => {}
            },
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                if let Some(buf) = current.as_mut() {
                    buf.push_str(&event_text(&t));
                }
            }
            Ok(quick_xml::events::Event::GeneralRef(r)) if in_text => {
                if let Some(buf) = current.as_mut() {
                    buf.push_str(&ref_text(&r));
                }
            }
            Ok(quick_xml::events::Event::End(e)) => match local_name_end(&e).as_str() {
                "t" => in_text = false,
                "si" => {
                    if let Some(buf) = current.take() {
                        out.push(buf);
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

/// `(sheet name, part path)` in workbook order.
fn xlsx_sheets(zip: &mut Zip) -> Vec<(String, String)> {
    let Some(workbook) = entry_text(zip, "xl/workbook.xml") else { return Vec::new() };
    let rels = entry_text(zip, "xl/_rels/workbook.xml.rels").map(|x| parse_rels(&x)).unwrap_or_default();
    let mut reader = quick_xml::Reader::from_str(&workbook);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if local_name(&e) == "sheet" {
                    let name = attr(&e, "name").unwrap_or_default();
                    let target = rel_id_attr(&e)
                        .and_then(|id| rels.iter().find(|r| r.id == id).map(|r| r.target.clone()));
                    if let Some(target) = target {
                        out.push((name, resolve_target("xl", &target)));
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

/// Non empty rows of a worksheet, capped like the C#.
fn xlsx_rows(xml: &str, shared: &[String]) -> Vec<Vec<String>> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut in_cell = false;
    let mut capture = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => match local_name(&e).as_str() {
                "row" => row.clear(),
                "c" => {
                    in_cell = true;
                    cell_type = attr(&e, "t").unwrap_or_default();
                    value.clear();
                }
                // `v` is the stored value, `t` inside `is` is an inline string.
                "v" | "t" if in_cell => capture = true,
                _ => {}
            },
            Ok(quick_xml::events::Event::Text(t)) if capture => value.push_str(&event_text(&t)),
            Ok(quick_xml::events::Event::GeneralRef(r)) if capture => value.push_str(&ref_text(&r)),
            Ok(quick_xml::events::Event::End(e)) => match local_name_end(&e).as_str() {
                "v" | "t" => capture = false,
                "c" => {
                    in_cell = false;
                    row.push(xlsx_cell_text(&cell_type, &value, shared));
                }
                "row" => {
                    if row.iter().any(|c| !c.trim().is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    }
                    if rows.len() >= TABLE_ROW_CAP {
                        return rows;
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    rows
}

fn xlsx_cell_text(cell_type: &str, value: &str, shared: &[String]) -> String {
    if cell_type == "s"
        && let Ok(index) = value.trim().parse::<usize>()
    {
        return shared.get(index).cloned().unwrap_or_default();
    }
    value.to_string()
}

// ── HTML ─────────────────────────────────────────────────────────────────────

/// Strip tags (dropping `script` and `style` bodies), unescape entities, then
/// trim and drop blank lines.
///
/// Like the C# `HtmlAgilityPack.InnerText`, adjacent elements are concatenated
/// with NO inserted separator: the line structure comes from the source markup
/// only. Minified HTML therefore extracts as one long line, in both ports.
fn html(data: &[u8]) -> ExtractResult {
    let src = decode(data);
    let mut text = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < src.len() {
        if bytes[i] != b'<' {
            let ch = src[i..].chars().next().expect("valid char boundary");
            text.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if let Some(rest) = src[i..].strip_prefix("<!--") {
            i += 4 + rest.find("-->").map(|p| p + 3).unwrap_or(rest.len());
            continue;
        }
        let Some(close) = src[i..].find('>') else { break };
        let tag = &src[i + 1..i + close];
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '!')
            .collect::<String>()
            .to_ascii_lowercase();
        i += close + 1;
        if matches!(name.as_str(), "script" | "style") && !tag.ends_with('/') {
            let end = format!("</{name}");
            match src[i..].to_ascii_lowercase().find(&end) {
                Some(p) => {
                    i += p;
                }
                None => break,
            }
        }
    }
    let unescaped = unescape_html(&text);
    let normalized: Vec<&str> =
        unescaped.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
    ExtractResult::of("html", normalized.join("\n").trim().to_string())
}

/// The named entities that matter for text extraction, plus numeric references.
fn unescape_html(text: &str) -> String {
    const NAMED: &[(&str, char)] = &[
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("nbsp", ' '),
        ("copy", '\u{a9}'),
        ("reg", '\u{ae}'),
        ("hellip", '\u{2026}'),
        ("mdash", '\u{2014}'),
        ("ndash", '\u{2013}'),
        ("rsquo", '\u{2019}'),
        ("lsquo", '\u{2018}'),
        ("ldquo", '\u{201c}'),
        ("rdquo", '\u{201d}'),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // An entity is short; anything longer is a stray ampersand.
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let body = &rest[1..semi];
        let replacement = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = body.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            NAMED.iter().find(|(n, _)| *n == body).map(|(_, c)| *c)
        };
        match replacement {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ── CSV and text ─────────────────────────────────────────────────────────────

fn csv(data: &[u8]) -> ExtractResult {
    let mut rows = parse_csv(&decode(data));
    rows.truncate(TABLE_ROW_CAP);
    let count = rows.len();
    ExtractResult::of("csv", md_table(&rows)).with_meta("rows", json!(count))
}

/// Minimal RFC 4180 parser (quotes, escaped quotes, embedded newlines). Direct
/// port of the C# `CsvParser.Parse`, including its trailing row handling.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut any = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    field.push('"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
                i += 1;
                continue;
            }
            field.push(c);
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_quotes = true;
                any = true;
                i += 1;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                any = true;
                i += 1;
            }
            '\r' => i += 1,
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
                i += 1;
            }
            _ => {
                field.push(c);
                any = true;
                i += 1;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() || any {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn fenced(data: &[u8], ext: &str) -> ExtractResult {
    let lang = ext.trim_start_matches('.');
    ExtractResult::of(lang, format!("```{lang}\n{}\n```", decode(data)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::ocr::{NullOcrProvider, OcrProvider};
    use crate::storage::blob::local::LocalBlobStore;
    use crate::storage::meta::SqliteMetaStore;
    use async_trait::async_trait;
    use std::sync::Arc;

    fn vol() -> (tempfile::TempDir, VolumeClient) {
        let d = tempfile::tempdir().unwrap();
        let meta = Arc::new(SqliteMetaStore::in_memory().unwrap());
        let blob = Arc::new(LocalBlobStore::new(d.path(), "mcpfs-docs-test"));
        (d, VolumeClient::new("test", meta, blob))
    }

    /// An OCR stub returning a fixed transcription.
    struct StubOcr(&'static str);

    #[async_trait]
    impl OcrProvider for StubOcr {
        fn enabled(&self) -> bool {
            true
        }
        async fn extract_text(&self, _image: &[u8], _mime: &str) -> Result<String> {
            Ok(self.0.to_string())
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    #[test]
    fn extension_of_scans_the_whole_path() {
        assert_eq!(extension_of("/a/b.PDF"), ".pdf");
        assert_eq!(extension_of("/a/Makefile"), "");
        assert_eq!(extension_of("/dir.d/README"), ".d/readme");
    }

    #[test]
    fn companion_path_replaces_the_extension() {
        assert_eq!(companion_md_path("/docs/report.pdf"), "/docs/report.md");
        assert_eq!(companion_md_path("/a/b.tar.gz"), "/a/b.tar.md");
        // no dot in the file name: append rather than eat a directory name
        assert_eq!(companion_md_path("/dir.d/README"), "/dir.d/README.md");
        assert_eq!(companion_md_path("/plain"), "/plain.md");
    }

    #[test]
    fn md_table_pads_escapes_and_flattens() {
        let rows = vec![
            vec!["a".into(), "b|c".into(), "d\ne".into()],
            vec!["1".into()],
        ];
        let table = md_table(&rows);
        assert_eq!(
            table,
            "| a | b\\|c | d e |\n| --- | --- | --- |\n| 1 |  |  |"
        );
        assert_eq!(md_table(&[]), "");
    }

    #[test]
    fn truncate_chars_is_codepoint_safe() {
        let (t, cut) = truncate_chars("caf\u{e9}s", 4);
        assert_eq!(t, "caf\u{e9}");
        assert!(cut);
        let (t2, cut2) = truncate_chars("abc", 10);
        assert_eq!(t2, "abc");
        assert!(!cut2);
    }

    #[test]
    fn resolve_target_collapses_dot_dot() {
        assert_eq!(resolve_target("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve_target("ppt/slides", "../notesSlides/notesSlide1.xml"), "ppt/notesSlides/notesSlide1.xml");
        assert_eq!(resolve_target("xl", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
    }

    // ── CSV ──────────────────────────────────────────────────────────────────

    #[test]
    fn csv_parser_handles_quotes_and_embedded_newlines() {
        let rows = parse_csv("a,b\n\"x,1\",\"y\"\"z\"\n\"multi\nline\",2\n");
        assert_eq!(rows[0], vec!["a", "b"]);
        assert_eq!(rows[1], vec!["x,1", "y\"z"]);
        assert_eq!(rows[2], vec!["multi\nline", "2"]);
    }

    #[test]
    fn csv_parser_keeps_a_trailing_row_without_newline() {
        let rows = parse_csv("a,b\nc,d");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], vec!["c", "d"]);
    }

    #[test]
    fn csv_extraction_produces_a_markdown_table() {
        let r = csv(b"name,qty\nbolt,3\n");
        assert_eq!(r.fmt, "csv");
        assert_eq!(r.text, "| name | qty |\n| --- | --- |\n| bolt | 3 |");
        assert_eq!(r.meta["rows"], json!(2));
    }

    // ── HTML ─────────────────────────────────────────────────────────────────

    #[test]
    fn html_drops_scripts_styles_and_entities() {
        let src = b"<html><head><style>p{color:red}</style><script>var x=1;</script></head>\n<body>\n<p>Hello &amp; welcome</p>\n<p>caf&#233;</p>\n</body></html>";
        let r = html(src);
        assert_eq!(r.fmt, "html");
        assert!(!r.text.contains("color:red"));
        assert!(!r.text.contains("var x"));
        assert!(r.text.contains("Hello & welcome"));
        assert!(r.text.contains("caf\u{e9}"));
    }

    #[test]
    fn html_strips_comments_and_blank_lines() {
        let r = html(b"<div>\n<!-- hidden -->\n\n<span>kept</span>\n</div>");
        assert!(!r.text.contains("hidden"));
        assert_eq!(r.text, "kept");
    }

    #[test]
    fn html_unescape_leaves_stray_ampersands_alone() {
        assert_eq!(unescape_html("a & b &amp; c &#x41;"), "a & b & c A");
        assert_eq!(unescape_html("&notanentity;"), "&notanentity;");
    }

    // ── fenced / text ────────────────────────────────────────────────────────

    #[test]
    fn json_is_wrapped_in_a_fence() {
        let r = fenced(b"{\"a\":1}", ".json");
        assert_eq!(r.fmt, "json");
        assert_eq!(r.text, "```json\n{\"a\":1}\n```");
    }

    #[tokio::test]
    async fn plain_text_extraction_is_verbatim() {
        let null = NullOcrProvider;
        let e = Extractor::new(&null);
        let r = e.extract(b"line one\nline two\n", "/a/notes.txt", 1000, true).await.unwrap();
        assert_eq!(r.fmt, "text");
        assert_eq!(r.text, "line one\nline two\n");
        assert!(!r.truncated);
        assert_eq!(r.note, "");
    }

    #[tokio::test]
    async fn unknown_extension_decodes_as_text_with_a_note() {
        let null = NullOcrProvider;
        let e = Extractor::new(&null);
        let r = e.extract(b"payload", "/a/thing.qqq", 1000, true).await.unwrap();
        assert_eq!(r.fmt, "text");
        assert_eq!(r.note, "unknown extension .qqq; decoded as text");
    }

    #[tokio::test]
    async fn max_chars_truncates_and_flags() {
        let null = NullOcrProvider;
        let e = Extractor::new(&null);
        let r = e.extract(b"0123456789", "/a/x.txt", 4, true).await.unwrap();
        assert_eq!(r.text, "0123");
        assert!(r.truncated);
    }

    // ── audio / video ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn audio_and_video_are_not_supported() {
        let null = NullOcrProvider;
        let e = Extractor::new(&null);
        for name in ["/a/song.mp3", "/a/clip.mp4", "/a/x.mkv", "/a/x.flac"] {
            let err = e.extract(b"binary", name, 1000, true).await.unwrap_err();
            assert_eq!(err.code, crate::errors::code::NOT_SUPPORTED, "{name}");
            assert!(err.message.starts_with("audio/video is out of scope for extraction: ."), "{}", err.message);
        }
    }

    // ── images / OCR ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn image_without_ocr_returns_the_configuration_hint() {
        let null = NullOcrProvider;
        let e = Extractor::new(&null);
        let r = e.extract(b"\x89PNG", "/a/scan.png", 1000, true).await.unwrap();
        assert_eq!(r.fmt, "image");
        assert_eq!(r.text, "");
        assert!(r.note.contains("extract.ocr.provider=multimodal"));
    }

    #[tokio::test]
    async fn image_with_ocr_returns_the_transcription() {
        let stub = StubOcr("  RECOVERED TEXT  ");
        let e = Extractor::new(&stub);
        let r = e.extract(b"\x89PNG", "/a/scan.png", 1000, true).await.unwrap();
        assert_eq!(r.text, "RECOVERED TEXT");
        assert_eq!(r.note, "text recovered via multimodal OCR provider");
    }

    #[tokio::test]
    async fn ocr_can_be_disabled_per_call() {
        let stub = StubOcr("SHOULD NOT APPEAR");
        let e = Extractor::new(&stub);
        let r = e.extract(b"\x89PNG", "/a/scan.png", 1000, false).await.unwrap();
        assert_eq!(r.text, "");
        assert_eq!(r.note, "image: the OCR provider returned no text");
    }

    #[tokio::test]
    async fn empty_ocr_result_yields_the_provider_note() {
        let stub = StubOcr("   ");
        let e = Extractor::new(&stub);
        let r = e.extract(b"x", "/a/scan.jpg", 1000, true).await.unwrap();
        assert_eq!(r.note, "image: the OCR provider returned no text");
    }

    // ── OOXML ────────────────────────────────────────────────────────────────

    /// Build a minimal OOXML package from `(name, xml)` pairs.
    fn make_zip(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, xml) in parts {
            zip.start_file(*name, opts).unwrap();
            std::io::Write::write_all(&mut zip, xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn docx_maps_styles_to_markdown() {
        let doc = r#"<w:document xmlns:w="x"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title Here</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading3"/></w:pPr><w:r><w:t>Sub</w:t></w:r></w:p>
<w:p><w:r><w:t>Plain </w:t></w:r><w:r><w:t>text.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="ListParagraph"/></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
<w:p/>
</w:body></w:document>"#;
        let bytes = make_zip(&[("word/document.xml", doc)]);
        let r = docx(&bytes).unwrap();
        assert_eq!(r.fmt, "docx");
        assert_eq!(r.text, "# Title Here\n\n### Sub\n\nPlain text.\n\n- item");
        assert_eq!(r.meta["paragraphs"], json!(5), "empty paragraphs still count");
    }

    #[test]
    fn docx_renders_tables() {
        let doc = r#"<w:document xmlns:w="x"><w:body>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        let r = docx(&make_zip(&[("word/document.xml", doc)])).unwrap();
        assert!(r.text.contains("| A | B |"));
        assert!(r.text.contains("| 1 | 2 |"));
        // paragraphs inside table cells are not emitted separately
        assert_eq!(r.meta["paragraphs"], json!(0));
    }

    #[test]
    fn docx_resolves_entities_and_character_references() {
        let doc = r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>a &amp; b &lt;c&gt; &quot;q&quot; caf&#233;</w:t></w:r></w:p></w:body></w:document>"#;
        let r = docx(&make_zip(&[("word/document.xml", doc)])).unwrap();
        assert_eq!(r.text, "a & b <c> \"q\" caf\u{e9}");
    }

    #[test]
    fn docx_round_trips_our_own_writer() {
        let md = "# Heading One\n\nSome body text.\n\n- bullet item\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let bytes = crate::docs::docx::render_markdown_to_docx(md, Some("Doc Title")).unwrap();
        let r = docx(&bytes).unwrap();
        assert!(r.text.contains("# Heading One"), "{}", r.text);
        assert!(r.text.contains("Some body text."));
        assert!(r.text.contains("- \u{2022} bullet item"), "list style detected: {}", r.text);
        assert!(r.text.contains("| A | B |"));
        assert!(r.text.contains("| 1 | 2 |"));
    }

    #[test]
    fn docx_without_document_part_is_an_error() {
        let err = docx(&make_zip(&[("other.xml", "<a/>")])).unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(err.message.contains("word/document.xml"));
    }

    #[test]
    fn non_zip_data_is_rejected_as_invalid() {
        let err = docx(b"definitely not a zip").unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[test]
    fn pptx_uses_presentation_order_and_notes() {
        let presentation = r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="256" r:id="rId9"/><p:sldId id="257" r:id="rId8"/></p:sldIdLst></p:presentation>"#;
        let rels = r#"<Relationships xmlns="x"><Relationship Id="rId9" Type="http://x/slide" Target="slides/slide2.xml"/><Relationship Id="rId8" Type="http://x/slide" Target="slides/slide1.xml"/></Relationships>"#;
        let slide = |t: &str| format!(r#"<p:sld xmlns:a="a"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#);
        let slide_rels = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#;
        let notes = r#"<p:notes xmlns:a="a"><a:p><a:r><a:t>speaker hint</a:t></a:r></a:p></p:notes>"#;
        let bytes = make_zip(&[
            ("ppt/presentation.xml", presentation),
            ("ppt/_rels/presentation.xml.rels", rels),
            ("ppt/slides/slide1.xml", &slide("second in order")),
            ("ppt/slides/slide2.xml", &slide("first in order")),
            ("ppt/slides/_rels/slide2.xml.rels", slide_rels),
            ("ppt/notesSlides/notesSlide1.xml", notes),
        ]);
        let r = pptx(&bytes).unwrap();
        assert_eq!(r.fmt, "pptx");
        assert_eq!(r.meta["slides"], json!(2));
        assert!(r.text.starts_with("## Slide 1\n\nfirst in order"), "{}", r.text);
        assert!(r.text.contains("> Notes: speaker hint"));
        assert!(r.text.contains("## Slide 2\n\nsecond in order"));
    }

    #[test]
    fn pptx_resolves_entities_in_slide_text() {
        let slide = r#"<p:sld xmlns:a="a"><a:p><a:r><a:t>Q&amp;A &#8212; done</a:t></a:r></a:p></p:sld>"#;
        let r = pptx(&make_zip(&[("ppt/slides/slide1.xml", slide)])).unwrap();
        assert!(r.text.contains("Q&A \u{2014} done"), "{}", r.text);
    }

    #[test]
    fn pptx_falls_back_to_numeric_slide_order() {
        let slide = |t: &str| format!(r#"<p:sld xmlns:a="a"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#);
        let bytes = make_zip(&[
            ("ppt/slides/slide10.xml", &slide("ten")),
            ("ppt/slides/slide2.xml", &slide("two")),
        ]);
        let r = pptx(&bytes).unwrap();
        // numeric, not lexicographic: slide2 comes before slide10
        assert!(r.text.find("two").unwrap() < r.text.find("ten").unwrap(), "{}", r.text);
    }

    #[test]
    fn xlsx_resolves_shared_strings_and_sheet_names() {
        let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let rels = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let shared = r#"<sst><si><t>Header</t></si><si><r><t>Rich </t></r><r><t>Text</t></r></si></sst>"#;
        let sheet = r#"<worksheet><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2"><v>42</v></c><c r="B2" t="inlineStr"><is><t>inline</t></is></c></row>
<row r="3"><c r="A3"><v> </v></c></row>
</sheetData></worksheet>"#;
        let bytes = make_zip(&[
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet),
        ]);
        let r = xlsx(&bytes).unwrap();
        assert_eq!(r.fmt, "xlsx");
        assert_eq!(r.meta["sheets"], json!(1));
        assert!(r.text.starts_with("## Sheet: Data"));
        assert!(r.text.contains("| Header | Rich Text |"), "{}", r.text);
        assert!(r.text.contains("| 42 | inline |"), "{}", r.text);
        assert!(!r.text.contains("| |"), "blank rows are skipped: {}", r.text);
    }

    #[test]
    fn xlsx_resolves_entities_in_shared_strings_and_values() {
        let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="S &amp; T" r:id="rId1"/></sheets></workbook>"#;
        let rels = r#"<Relationships xmlns="x"><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let shared = r#"<sst><si><t>R&amp;D</t></si></sst>"#;
        let sheet = r#"<worksheet><sheetData><row><c t="s"><v>0</v></c><c t="inlineStr"><is><t>1 &lt; 2</t></is></c></row></sheetData></worksheet>"#;
        let r = xlsx(&make_zip(&[
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet),
        ]))
        .unwrap();
        assert!(r.text.contains("## Sheet: S & T"), "{}", r.text);
        assert!(r.text.contains("| R&D | 1 < 2 |"), "{}", r.text);
    }

    #[test]
    fn xlsx_without_sheets_yields_empty_text() {
        let r = xlsx(&make_zip(&[("xl/workbook.xml", "<workbook><sheets/></workbook>")])).unwrap();
        assert_eq!(r.text, "");
        assert_eq!(r.meta["sheets"], json!(0));
    }

    // ── PDF ──────────────────────────────────────────────────────────────────

    /// Smallest PDF with a real text object, built by hand so the test has no
    /// binary fixture to check in.
    fn tiny_pdf() -> Vec<u8> {
        let content = b"BT /F1 24 Tf 72 700 Td (Hello PDF) Tj ET";
        let mut pdf = Vec::new();
        let mut offsets = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n");
        let push = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: String| {
            offsets.push(pdf.len());
            pdf.extend_from_slice(body.as_bytes());
        };
        push(&mut pdf, &mut offsets, "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".into());
        push(
            &mut pdf,
            &mut offsets,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".into(),
        );
        push(
            &mut pdf,
            &mut offsets,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n".into(),
        );
        push(
            &mut pdf,
            &mut offsets,
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                content.len(),
                String::from_utf8_lossy(content)
            ),
        );
        push(
            &mut pdf,
            &mut offsets,
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".into(),
        );
        push(&mut pdf, &mut offsets, "6 0 obj\n<< /Title (Unit Test Doc) >>\nendobj\n".into());
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for off in &offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{}\n%%EOF\n",
                offsets.len() + 1,
                xref
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn pdf_extracts_text_pages_and_metadata() {
        let r = pdf(&tiny_pdf()).unwrap();
        assert_eq!(r.fmt, "pdf");
        assert_eq!(r.meta["pages"], json!(1));
        assert_eq!(r.meta.get("title"), Some(&json!("Unit Test Doc")));
        assert!(r.text.contains("*[Page 1]*"), "{}", r.text);
        assert!(r.text.contains("Hello PDF"), "{}", r.text);
    }

    #[test]
    fn corrupt_pdf_is_an_invalid_argument_not_a_panic() {
        let err = pdf(b"%PDF-1.4\ngarbage").unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[test]
    fn pdf_utf16_title_is_decoded() {
        let raw = [0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69];
        assert_eq!(decode_pdf_text(&raw), "Hi");
        assert_eq!(decode_pdf_text(b"Plain"), "Plain");
    }

    // ── orchestration: companion md, cache, previews ─────────────────────────

    #[tokio::test]
    async fn extract_text_on_a_text_file_writes_no_companion() {
        let (_d, v) = vol();
        v.write_text_atomic("/notes.txt", "hello world").await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/notes.txt", 1000, 5, true, false).await.unwrap();
        assert_eq!(out["path"], json!("/notes.txt"));
        assert_eq!(out["md_path"], Value::Null);
        assert_eq!(out["format"], json!("text"));
        assert_eq!(out["chars"], json!(11));
        assert_eq!(out["cached"], json!(false));
        assert_eq!(out["preview"], json!("hello"));
        assert_eq!(out["truncated"], json!(false));
        assert!(!v.exists("/notes.md").await.unwrap());
    }

    #[tokio::test]
    async fn extract_text_on_a_csv_writes_the_companion_md() {
        let (_d, v) = vol();
        v.write_text_atomic("/data.csv", "a,b\n1,2\n").await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        assert_eq!(out["md_path"], json!("/data.md"));
        assert_eq!(out["format"], json!("csv"));
        assert_eq!(out["cached"], json!(false));
        let stored = v.read_text("/data.md").await.unwrap();
        assert_eq!(stored, "| a | b |\n| --- | --- |\n| 1 | 2 |");
        assert_eq!(out["preview"], json!(stored));
        assert_eq!(out["meta"]["rows"], json!(2));
    }

    #[tokio::test]
    async fn companion_md_is_reused_when_up_to_date() {
        let (_d, v) = vol();
        v.write_text_atomic("/data.csv", "a,b\n1,2\n").await.unwrap();
        let null = NullOcrProvider;
        extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        // hand edit the companion: a cached hit must return the edited content
        v.write_text_atomic("/data.md", "EDITED BY HAND").await.unwrap();

        let out = extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        assert_eq!(out["cached"], json!(true));
        assert_eq!(out["format"], json!("md"), "a cache hit reports the md format");
        assert_eq!(out["preview"], json!("EDITED BY HAND"));
        // the cached payload carries no truncated/meta/note keys, like the C#
        assert!(out.get("truncated").is_none());
        assert!(out.get("meta").is_none());
        assert!(out.get("note").is_none());
    }

    #[tokio::test]
    async fn refresh_forces_re_extraction_over_the_companion() {
        let (_d, v) = vol();
        v.write_text_atomic("/data.csv", "a,b\n1,2\n").await.unwrap();
        let null = NullOcrProvider;
        extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        v.write_text_atomic("/data.md", "STALE").await.unwrap();

        let out = extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, true).await.unwrap();
        assert_eq!(out["cached"], json!(false));
        assert_eq!(out["format"], json!("csv"));
        assert_ne!(v.read_text("/data.md").await.unwrap(), "STALE");
    }

    #[tokio::test]
    async fn a_stale_companion_is_regenerated() {
        let (_d, v) = vol();
        v.write_text_atomic("/data.csv", "a,b\n1,2\n").await.unwrap();
        let null = NullOcrProvider;
        extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        // rewriting the source bumps its mtime past the companion
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        v.write_text_atomic("/data.csv", "a,b\n9,9\n").await.unwrap();

        let out = extract_text(&v, &null, "/data.csv", 100_000, 4_000, true, false).await.unwrap();
        assert_eq!(out["cached"], json!(false));
        assert!(v.read_text("/data.md").await.unwrap().contains("| 9 | 9 |"));
    }

    #[tokio::test]
    async fn max_chars_bounds_the_stored_markdown_and_preview_bounds_the_reply() {
        let (_d, v) = vol();
        let long: String = (0..300).map(|i| format!("row{i},value{i}\n")).collect();
        v.write_text_atomic("/big.csv", &long).await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/big.csv", 120, 20, true, false).await.unwrap();
        assert_eq!(out["truncated"], json!(true));
        assert_eq!(out["chars"], json!(120));
        assert_eq!(out["preview"].as_str().unwrap().chars().count(), 20);
        assert_eq!(v.read_text("/big.md").await.unwrap().chars().count(), 120);
    }

    #[tokio::test]
    async fn preview_larger_than_the_text_returns_everything() {
        let (_d, v) = vol();
        v.write_text_atomic("/s.txt", "short").await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/s.txt", 1000, 10_000, true, false).await.unwrap();
        assert_eq!(out["preview"], json!("short"));
    }

    #[tokio::test]
    async fn empty_extraction_writes_no_companion_and_reports_null_md_path() {
        let (_d, v) = vol();
        v.write_bytes_atomic("/scan.png", b"\x89PNG not a real image").await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/scan.png", 1000, 100, true, false).await.unwrap();
        assert_eq!(out["md_path"], Value::Null);
        assert_eq!(out["format"], json!("image"));
        assert!(!v.exists("/scan.md").await.unwrap());
        assert!(out["note"].as_str().unwrap().contains("multimodal"));
    }

    #[tokio::test]
    async fn unsupported_format_returns_err_not_supported() {
        let (_d, v) = vol();
        v.write_bytes_atomic("/talk.mp3", b"ID3fake").await.unwrap();
        let null = NullOcrProvider;
        let err = extract_text(&v, &null, "/talk.mp3", 1000, 100, true, false).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_SUPPORTED);
        assert_eq!(err.message, "audio/video is out of scope for extraction: .mp3");
    }

    #[tokio::test]
    async fn missing_source_is_not_found() {
        let (_d, v) = vol();
        let null = NullOcrProvider;
        let err = extract_text(&v, &null, "/nope.pdf", 1000, 100, true, false).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_FOUND);
        assert_eq!(err.message, "not a file: /nope.pdf");
    }

    #[tokio::test]
    async fn a_directory_is_not_a_file() {
        let (_d, v) = vol();
        v.makedirs("/dir", true).await.unwrap();
        let null = NullOcrProvider;
        let err = extract_text(&v, &null, "/dir", 1000, 100, true, false).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_FOUND);
    }

    #[tokio::test]
    async fn broken_docx_reports_could_not_extract() {
        let (_d, v) = vol();
        v.write_bytes_atomic("/bad.docx", b"not a zip at all").await.unwrap();
        let null = NullOcrProvider;
        let err = extract_text(&v, &null, "/bad.docx", 1000, 100, true, false).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(err.message.starts_with("could not extract /bad.docx: "), "{}", err.message);
    }

    #[tokio::test]
    async fn html_source_gets_a_companion_md() {
        let (_d, v) = vol();
        v.write_text_atomic("/page.html", "<h1>Title</h1>\n<p>Body</p>").await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/page.html", 1000, 100, true, false).await.unwrap();
        assert_eq!(out["md_path"], json!("/page.md"));
        assert_eq!(v.read_text("/page.md").await.unwrap(), "Title\nBody");
    }

    #[tokio::test]
    async fn pdf_end_to_end_writes_the_companion() {
        let (_d, v) = vol();
        v.write_bytes_atomic("/doc.pdf", &tiny_pdf()).await.unwrap();
        let null = NullOcrProvider;
        let out = extract_text(&v, &null, "/doc.pdf", 100_000, 4_000, true, false).await.unwrap();
        assert_eq!(out["format"], json!("pdf"));
        assert_eq!(out["md_path"], json!("/doc.md"));
        assert!(v.read_text("/doc.md").await.unwrap().contains("Hello PDF"));
    }
}
