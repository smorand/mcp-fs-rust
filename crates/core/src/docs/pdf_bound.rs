//! The PDF text pass under a decode bound (SPEC-0089). Neither lopdf nor
//! pdf-extract can cap a decode: `Stream::decompressed_content` inflates into an
//! unbounded `Vec`, and object streams are inflated inside the load. What they do
//! offer is enough to decide every inflation from here (DEC-001). The load filter
//! runs before an object stream is inflated, so object streams are deferred and
//! handled here, measured first (DEC-004). A stream without a `Filter` key is never
//! inflated, so after the load every `Filter` is moved aside ("defuse") and put
//! back, page by page, only on the streams this module measured within the limits
//! ("arm", DEC-006). `output_doc_page` runs one page of the loaded document, so the
//! page loop is ours, and the walk before each page mirrors the reader's decode
//! sites (DEC-007).
//!
//! Anything the walk does not reach is handed to the reader as its stored bytes,
//! the way an undecodable stream is today: a missed site costs text, never the bound.

use crate::docs::extract::CONTENT_LIMIT;
use crate::errors::{Result, ToolError};
use pdf_extract::content::{Content, Operation};
use pdf_extract::encryption::decrypt_object;
use pdf_extract::xref::XrefEntry;
use pdf_extract::{
    Dictionary, Document, EncryptionState, LoadOptions, Object, ObjectId, ObjectStream, Stream,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// Largest decoded size of one PDF stream (SPEC-0089 FR-NEW-002).
pub(crate) const STREAM_LIMIT: u64 = 104_857_600;

/// Largest total decoded size of the compressed object streams of one PDF
/// (SPEC-0089 FR-NEW-003).
pub(crate) const LOAD_LIMIT: u64 = 268_435_456;

/// Largest total decoded size of the streams one page decodes, repeats counted
/// (SPEC-0089 FR-NEW-004).
pub(crate) const PAGE_LIMIT: u64 = 268_435_456;

/// Most objects one PDF may hold (SPEC-0089 FR-NEW-001).
pub(crate) const OBJECT_LIMIT: u64 = 1_000_000;

/// Bytes the measurer inflates at a time: the only decoded data it holds.
const CHUNK: usize = 65_536;

/// The `Type` a deferred object stream wears during the load, so lopdf keeps it
/// as an ordinary stream instead of inflating it (DEC-004).
const DEFERRED: &[u8] = b"McpFsObjStm";

/// The `Type` a stream the file itself typed [`DEFERRED`] is given instead, so
/// that only the streams this module deferred are expanded as object streams.
const FILE_TYPED: &[u8] = b"McpFsFileType";

/// Longest filter chain this module measures. Real files use one or two; a
/// longer chain is left stored, so a chain of thousands of names never costs a
/// decoder per name.
const MAX_FILTERS: usize = 8;

/// Where a defused stream keeps its `Filter` value (DEC-006).
const DEFUSED: &[u8] = b"McpFsFilter";

/// The refusal of FR-NEW-002, before `prepare_extraction` names the path.
fn over_stream_limit() -> ToolError {
    ToolError::invalid_argument("a PDF stream decodes to more than 104857600 bytes")
}

/// The refusal of FR-NEW-003, before `prepare_extraction` names the path.
fn over_load_limit() -> ToolError {
    ToolError::invalid_argument(
        "the PDF object streams decode to more than 268435456 bytes together",
    )
}

/// The refusal of FR-NEW-004, before `prepare_extraction` names the path.
fn over_page_limit(page_no: u32) -> ToolError {
    ToolError::invalid_argument(format!(
        "page {page_no} decodes more than 268435456 bytes of PDF streams"
    ))
}

/// The refusal of FR-NEW-001, before `prepare_extraction` names the path.
fn over_object_limit() -> ToolError {
    ToolError::invalid_argument("the PDF holds more than 1000000 objects")
}

/// Bytes a page may write past [`CONTENT_LIMIT`] before the sink refuses: room
/// for the whitespace the page trim removes (DEC-010).
const SINK_SLACK: u64 = 65_536;

/// The refusal of FR-NEW-005, before `prepare_extraction` names the path.
fn over_content_limit() -> ToolError {
    ToolError::invalid_argument(format!("the extracted content exceeds {CONTENT_LIMIT} bytes"))
}

/// The one buffer the extracted content is built in. A write that would bring
/// it past `threshold` is refused before it lands, so the content held stays
/// under the limit plus the slack whatever one character code maps to.
struct ContentSink {
    buf: Vec<u8>,
    threshold: u64,
    tripped: bool,
}

impl std::io::Write for ContentSink {
    fn write(&mut self, piece: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() as u64 + piece.len() as u64 > self.threshold {
            self.tripped = true;
            return Err(std::io::Error::other("the extracted content exceeds its limit"));
        }
        self.buf.extend_from_slice(piece);
        Ok(piece.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentSink {
    /// Trim the page written after `text_at` the way `str::trim` does, in place:
    /// leading bytes drained, trailing truncated; an empty page is removed with
    /// its marker (from `marker_at`). Returns the trimmed length. O(page bytes)
    /// time, no extra space.
    fn trim_page(&mut self, marker_at: usize, text_at: usize) -> Result<usize> {
        let page = std::str::from_utf8(&self.buf[text_at..]).map_err(|_| {
            ToolError::invalid_argument("pdf: the extracted text is not valid UTF-8")
        })?;
        let lead = page.len() - page.trim_start().len();
        let end = page.trim_end().len();
        if end == lead {
            self.buf.truncate(marker_at);
            return Ok(0);
        }
        self.buf.truncate(text_at + end);
        self.buf.drain(text_at..text_at + lead);
        Ok(end - lead)
    }
}

/// What the text pass gives back: the trimmed length in bytes of each page's
/// text (in order, empty pages included; only the page count is read, so no
/// second copy of the text is kept), the body assembled as before, and the title.
pub(crate) struct PdfText {
    pub(crate) pages: Vec<usize>,
    pub(crate) body: String,
    pub(crate) title: Option<String>,
}

/// The figures of one call. `page` is the decode count of the page being
/// walked; `held` the largest decoded data held at once (DEC-012), in logical
/// bytes: the measurer's running output, a buffer the walk parses, the forms on
/// the drawing stack, a page's decode count. The transient copy lopdf makes in
/// `ObjectStream::new` is not counted, so the real peak may reach twice it.
#[derive(Default)]
struct PdfBudget {
    held: u64,
    page: u64,
    /// The page being walked, from 1.
    page_no: u32,
    content: u64,
    /// Decoded bytes of the object streams measured so far by the load.
    load: u64,
    /// Objects counted so far by the load.
    objects: u64,
}

impl PdfBudget {
    fn hold(&mut self, bytes: u64) {
        self.held = self.held.max(bytes);
    }
}

/// Owns the figures so that every outcome records them, a panic included:
/// the unwinding drops the guard (DEC-012).
struct Figures(PdfBudget);

#[cfg(test)]
impl Drop for Figures {
    fn drop(&mut self) {
        let held = crate::docs::extract::PdfHeld { decoded: self.0.held, content: self.0.content };
        crate::docs::extract::LAST_PDF_HELD.with(|c| c.set(Some(held)));
    }
}

/// What a measured decode counts against.
#[derive(Clone, Copy)]
enum Scope {
    /// An object stream decoded by the load.
    Load,
    /// A stream the reader may decode for the current page.
    Page,
    /// A cross reference stream lopdf inflates while reading the cross reference.
    Guard,
}

/// The text of `data`, every stream decoded under [`STREAM_LIMIT`]. Order of
/// the refusals (DEC-013): the load and its object streams in increasing number,
/// then the pages in order, then the title.
pub(crate) fn extract(data: &[u8]) -> Result<PdfText> {
    let mut figures = Figures(PdfBudget::default());
    let budget = &mut figures.0;
    let mut doc = load(data, budget)?;
    let stand_ins = install_stand_ins(&mut doc);
    let originals: HashMap<ObjectId, ObjectId> = stand_ins.iter().map(|(o, s)| (*s, *o)).collect();
    defuse(&mut doc);

    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let mut pages = Vec::new();
    let mut sink =
        ContentSink { buf: Vec::new(), threshold: CONTENT_LIMIT + SINK_SLACK, tripped: false };
    for (index, page_id) in page_ids.into_iter().enumerate() {
        let n = index as u32 + 1;
        budget.page_no = n;
        let walked = walk(&doc, page_id, &originals, budget)?;
        budget.hold(budget.page);
        // A tint transform pdf-extract would format from invalid UTF-8 keeps
        // doing so (DEC-009): its stand in holds the one invalid byte.
        for id in &walked.invalid {
            if let Some(Object::Stream(stand_in)) = doc.objects.get_mut(id) {
                stand_in.set_plain_content(vec![0xFF]);
            }
        }
        // The marker goes in before the page so the page is built in place: one
        // buffer holds the content, no second copy is made (DEC-010).
        let marker_at = sink.buf.len();
        sink.buf.extend_from_slice(format!("\n\n---\n*[Page {n}]*\n\n").as_bytes());
        let text_at = sink.buf.len();
        let page = {
            let armed = Armed::new(&mut doc, walked.armed);
            pdf_extract::output_doc_page(
                armed.doc,
                &mut pdf_extract::PlainTextOutput::new(&mut sink as &mut dyn std::io::Write),
                n,
            )
        };
        if sink.tripped {
            budget.content = budget.content.max(sink.buf.len() as u64);
            return Err(over_content_limit());
        }
        // The reader's own loop stops at the first page error, the error itself
        // dropped (`extract_text_from_mem_by_pages`), and so does its text.
        if page.is_err() {
            sink.buf.truncate(marker_at);
            break;
        }
        let kept = sink.trim_page(marker_at, text_at)?;
        pages.push(kept);
        // The final trim drops the two newlines that open the first marker.
        let content = (sink.buf.len() as u64).saturating_sub(2);
        budget.content = budget.content.max(content);
        if content > CONTENT_LIMIT {
            return Err(over_content_limit());
        }
    }

    let mut body = String::from_utf8(sink.buf)
        .map_err(|_| ToolError::invalid_argument("pdf: the extracted text is not valid UTF-8"))?;
    body.drain(..body.len().min(2));
    let title = title(&doc);
    Ok(PdfText { pages, body, title })
}

/// `/Info /Title` of the loaded document: no second load (DEC-011). Absent or
/// unreadable metadata is no error.
fn title(doc: &Document) -> Option<String> {
    let info = doc.trailer.get(b"Info").ok()?;
    let dict = match info {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let raw = dict.get(b"Title").ok()?.as_str().ok()?;
    Some(crate::docs::extract::decode_pdf_text(raw))
}

// ── Cross reference and encryption, before the load (DEC-005) ───────────────

/// The refusal of an encrypted document whose `/Encrypt` key could not be
/// renamed in a copy: it is never handed to lopdf's encrypted loader, which
/// inflates object streams with no filter (`reader.rs:889`).
fn not_isolated() -> ToolError {
    ToolError::invalid_argument("pdf: the encryption dictionary could not be isolated")
}

/// The refusal of a cross reference stream the measurer cannot follow exactly
/// (a predictor between two steps, more than [`MAX_FILTERS`] filters): lopdf
/// would decode it whole while reading the cross reference, and it cannot be
/// left stored as a page stream is.
fn unmeasurable_xref() -> ToolError {
    ToolError::invalid_argument("pdf: the cross reference stream could not be measured")
}

/// lopdf's whitespace (`parser/mod.rs` `is_whitespace`).
fn is_space(c: u8) -> bool {
    b" \t\n\r\0\x0C".contains(&c)
}

fn is_delimiter(c: u8) -> bool {
    b"()<>[]{}/%".contains(&c)
}

/// One token of the minimal tokenizer: enough to find where a dictionary ends
/// and which of its names are `Encrypt`, never to read a value.
enum Token {
    Open,
    Close,
    /// A name, decoded as lopdf decodes `#xx` escapes, and its bytes in the input.
    Name(Vec<u8>, std::ops::Range<usize>),
    /// A run of regular characters: a number, `R`, a keyword.
    Word(std::ops::Range<usize>),
    /// `[`, `]`, a string, a hex string, `{`, `}`.
    Other(u8),
}

/// The tokenizer: skips whitespace and comments, and strings, hex strings and
/// names whole, so a `>>` or a `/Encrypt` inside them is not taken for one.
struct Lexer<'a> {
    data: &'a [u8],
    at: usize,
}

impl Lexer<'_> {
    fn skip_space(&mut self) {
        while let Some(&c) = self.data.get(self.at) {
            if is_space(c) {
                self.at += 1;
            } else if c == b'%' {
                while self.data.get(self.at).is_some_and(|c| !b"\r\n".contains(c)) {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next token, `None` at the end or on an unterminated string.
    /// O(token length) time.
    fn next_token(&mut self) -> Option<Token> {
        self.skip_space();
        let data = self.data;
        let start = self.at;
        let c = *data.get(start)?;
        self.at += 1;
        match c {
            b'<' if data.get(self.at) == Some(&b'<') => {
                self.at += 1;
                Some(Token::Open)
            }
            b'>' if data.get(self.at) == Some(&b'>') => {
                self.at += 1;
                Some(Token::Close)
            }
            b'<' => {
                self.at += data[self.at..].iter().position(|&c| c == b'>')? + 1;
                Some(Token::Other(c))
            }
            b'(' => {
                let mut depth = 1usize;
                while depth > 0 {
                    match *data.get(self.at)? {
                        b'\\' => self.at += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    self.at += 1;
                }
                Some(Token::Other(c))
            }
            b'/' => {
                // As lopdf's `name`: `#` and two hex digits is one byte; a `#`
                // without them ends the name.
                let mut name = Vec::new();
                while let Some(&c) = data.get(self.at) {
                    let hex = |i: usize| data.get(i).copied().filter(u8::is_ascii_hexdigit);
                    if c == b'#' {
                        let (Some(h), Some(l)) = (hex(self.at + 1), hex(self.at + 2)) else {
                            break;
                        };
                        let digit = |d: u8| (d as char).to_digit(16).unwrap_or(0) as u8;
                        name.push(digit(h) << 4 | digit(l));
                        self.at += 3;
                    } else if !is_space(c) && !is_delimiter(c) {
                        name.push(c);
                        self.at += 1;
                    } else {
                        break;
                    }
                }
                Some(Token::Name(name, start..self.at))
            }
            c if is_delimiter(c) => Some(Token::Other(c)),
            _ => {
                while data.get(self.at).is_some_and(|&c| !is_space(c) && !is_delimiter(c)) {
                    self.at += 1;
                }
                Some(Token::Word(start..self.at))
            }
        }
    }
}

/// The end of the dictionary opening at `start` (just past its `>>`), and the
/// names at its first level that decode to `Encrypt`. O(dictionary bytes).
fn dictionary_span(data: &[u8], start: usize) -> Option<(usize, Vec<std::ops::Range<usize>>)> {
    let mut lexer = Lexer { data, at: start };
    if !matches!(lexer.next_token()?, Token::Open) {
        return None;
    }
    let (mut depth, mut arrays) = (1usize, 0usize);
    let mut names = Vec::new();
    while depth > 0 {
        match lexer.next_token()? {
            Token::Open => depth += 1,
            Token::Close => depth -= 1,
            Token::Other(b'[') => arrays += 1,
            Token::Other(b']') => arrays = arrays.saturating_sub(1),
            Token::Name(name, range) if depth == 1 && arrays == 0 && name == b"Encrypt" => {
                names.push(range);
            }
            _ => {}
        }
    }
    Some((lexer.at, names))
}

/// Whether a name decoding to `Encrypt` appears anywhere in `data`, strings and
/// stream bytes included: the fail closed test when the trailer cannot be found.
fn mentions_encrypt(data: &[u8]) -> bool {
    data.iter().enumerate().filter(|(_, c)| **c == b'/').any(|(at, _)| {
        matches!(Lexer { data, at }.next_token(), Some(Token::Name(name, _)) if name == b"Encrypt")
    })
}

/// A reader over `buffer` whose cross reference places object `id` at `offset`
/// and nothing else, as lopdf's own reader is while it reads the cross
/// reference: an indirect `Length` does not resolve, so its stream is read empty.
fn object_at(buffer: &[u8], id: ObjectId, offset: usize) -> Option<Object> {
    let mut reader = pdf_extract::Reader {
        buffer,
        document: Document::new(),
        encryption_state: None,
        raw_objects: BTreeMap::new(),
        password: None,
        strict: false,
    };
    let offset = u32::try_from(offset).ok()?;
    reader.document.reference_table.insert(id.0, XrefEntry::Normal { offset, generation: id.1 });
    reader.get_object(id, &mut HashSet::new()).ok()
}

/// The dictionary `bytes` hold, read by lopdf's own parser.
fn parse_dictionary(bytes: &[u8]) -> Option<Dictionary> {
    let mut object = b"0 0 obj\n".to_vec();
    object.extend_from_slice(bytes);
    object.extend_from_slice(b"\nendobj\n");
    match object_at(&object, (0, 0), 0)? {
        Object::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

/// `N G obj` at `at`, as lopdf's `object_id` reads it; returns the id and
/// where the object's value starts.
fn object_header(data: &[u8], at: usize) -> Option<(ObjectId, usize)> {
    let mut lexer = Lexer { data, at };
    let word = |lexer: &mut Lexer<'_>| match lexer.next_token()? {
        Token::Word(range) => std::str::from_utf8(&data[range]).ok().map(str::to_string),
        _ => None,
    };
    let number = word(&mut lexer)?.parse().ok()?;
    let generation = word(&mut lexer)?.parse().ok()?;
    (word(&mut lexer)? == "obj").then_some(((number, generation), lexer.at))
}

/// One cross reference section: its trailer (or stream dictionary) as lopdf
/// parses it, and that dictionary's span with its `Encrypt` names.
struct Section {
    trailer: Dictionary,
    span: std::ops::Range<usize>,
    names: Vec<std::ops::Range<usize>>,
}

/// The section at `offset` of `buffer`: a classic table and its trailer, or a
/// cross reference stream, measured here as lopdf will inflate it while
/// reading (`parser/mod.rs:547`, `parser_aux.rs:487-490`). `None` when it
/// cannot be read the way lopdf reads it.
fn section(buffer: &[u8], offset: usize, budget: &mut PdfBudget) -> Result<Option<Section>> {
    let Some(at) = buffer.get(offset..) else { return Ok(None) };
    if at.starts_with(b"xref") {
        // Between `xref` and `trailer` lopdf accepts only entries, then
        // whitespace and comments.
        let mut lexer = Lexer { data: buffer, at: offset + 4 };
        loop {
            match lexer.next_token() {
                Some(Token::Word(range)) if &buffer[range.clone()] == b"trailer" => break,
                Some(Token::Word(_)) => {}
                _ => return Ok(None),
            }
        }
        lexer.skip_space();
        let start = lexer.at;
        let Some((end, names)) = dictionary_span(buffer, start) else { return Ok(None) };
        let Some(trailer) = parse_dictionary(&buffer[start..end]) else { return Ok(None) };
        return Ok(Some(Section { trailer, span: start..end, names }));
    }
    let Some((id, value)) = object_header(buffer, offset) else { return Ok(None) };
    let Some(Object::Stream(stream)) = object_at(buffer, id, offset) else { return Ok(None) };
    if !measurable(&stream) {
        return Err(unmeasurable_xref());
    }
    measure(&stream, budget, Scope::Guard)?;
    let mut lexer = Lexer { data: buffer, at: value };
    lexer.skip_space();
    let start = lexer.at;
    let Some((end, names)) = dictionary_span(buffer, start) else { return Ok(None) };
    Ok(Some(Section { trailer: stream.dict, span: start..end, names }))
}

/// Where lopdf starts reading the cross reference (`reader.rs:1262-1286`):
/// `startxref` in the 25 bytes before the last `%%EOF` of the last 512 bytes,
/// then the offset, an end of line and `%%EOF`.
fn xref_start(buffer: &[u8]) -> Option<usize> {
    let rfind = |hay: &[u8], needle: &[u8], from: usize| {
        hay.get(from..)?.windows(needle.len()).rposition(|w| w == needle).map(|p| from + p)
    };
    let eof = rfind(buffer, b"%%EOF", buffer.len() - buffer.len().min(512))?;
    if eof <= 25 {
        return None;
    }
    let at = rfind(&buffer[..eof], b"startxref", eof - 25)? + 9;
    let rest = &buffer[at..];
    let rest = rest.strip_prefix(b" ").unwrap_or(rest);
    let eol = |r: &[u8]| -> Option<usize> {
        [&b"\r\n"[..], b"\n", b"\r"].iter().find(|e| r.starts_with(e)).map(|e| e.len())
    };
    let rest = &rest[eol(rest)?..];
    let spaces = |r: &[u8]| r.iter().take_while(|c| **c == b' ').count();
    let rest = &rest[spaces(rest)..];
    let signed = usize::from(rest.first().is_some_and(|c| b"+-".contains(c)));
    let digits = signed + rest[signed..].iter().take_while(|c| c.is_ascii_digit()).count();
    let value: i64 = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    let rest = &rest[digits..];
    let rest = &rest[spaces(rest)..];
    if !rest[eol(rest)?..].starts_with(b"%%EOF") {
        return None;
    }
    usize::try_from(value).ok().filter(|v| *v <= buffer.len())
}

/// The main trailer of a PDF: its span in the file, the spans of its first
/// level names that decode to `Encrypt`, and its dictionary as lopdf parses it.
struct TrailerSpan {
    span: std::ops::Range<usize>,
    names: Vec<std::ops::Range<usize>>,
    trailer: Dictionary,
}

impl TrailerSpan {
    /// lopdf would load the document with its encrypted loader.
    fn encrypted(&self) -> bool {
        self.trailer.get(b"Encrypt").is_ok()
    }
}

/// Locate the cross reference sections as lopdf does (the main one, then the
/// `Prev` chain with a seen set and the main trailer's `XRefStm`,
/// `reader.rs:765-791`) and measure every cross reference stream under the
/// stream limit before anything is loaded. A section lopdf could not read
/// ends the walk: lopdf then fails as today. When the main trailer cannot be
/// read at all but the file names `Encrypt` past `startxref`, or when no
/// cross reference can be located and the file names `Encrypt` anywhere, the
/// document is refused rather than risk the encrypted loader on a reading of
/// the file this module did not share. O(file) time.
fn xref_guard(data: &[u8], budget: &mut PdfBudget) -> Result<Option<TrailerSpan>> {
    let base = data.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let buffer = &data[base..];
    let Some(start) = xref_start(buffer) else {
        return if mentions_encrypt(buffer) { Err(not_isolated()) } else { Ok(None) };
    };
    let Some(main) = section(buffer, start, budget)? else {
        return if mentions_encrypt(&buffer[start..]) { Err(not_isolated()) } else { Ok(None) };
    };
    let mut seen = HashSet::new();
    let mut stream_section = main.trailer.get(b"XRefStm").and_then(Object::as_i64).ok();
    let mut prev = main.trailer.get(b"Prev").and_then(Object::as_i64).ok();
    while let Some(offset) = prev {
        if !seen.insert(offset) {
            break;
        }
        let Some(offset) = usize::try_from(offset).ok().filter(|o| *o <= buffer.len()) else {
            break;
        };
        let Some(older) = section(buffer, offset, budget)? else { break };
        if let Some(at) = stream_section.take().and_then(|a| usize::try_from(a).ok()) {
            section(buffer, at, budget)?;
        }
        prev = older.trailer.get(b"Prev").and_then(Object::as_i64).ok();
    }
    let shift = |r: std::ops::Range<usize>| r.start + base..r.end + base;
    Ok(Some(TrailerSpan {
        span: shift(main.span),
        names: main.names.into_iter().map(shift).collect(),
        trailer: main.trailer,
    }))
}

/// A copy of `data` whose main trailer no longer names `Encrypt`, so lopdf
/// loads it with the filter: each first level name decoding to `Encrypt` is
/// replaced by `/NoCrypt`, padded to the same length, so no offset moves. The
/// copy's trailer is parsed again by lopdf; when it still holds `Encrypt` the
/// document fails closed. O(file) time and one copy of the file.
fn unlock_encrypted(data: &[u8], span: &TrailerSpan) -> Result<Vec<u8>> {
    let mut copy = data.to_vec();
    for range in &span.names {
        let mut name = b"/NoCrypt".to_vec();
        name.resize(range.len(), b'X');
        copy[range.clone()].copy_from_slice(&name);
    }
    match parse_dictionary(&copy[span.span.clone()]) {
        Some(trailer) if trailer.get(b"Encrypt").is_err() => Ok(copy),
        _ => Err(not_isolated()),
    }
}

/// The trailer lopdf's own load would have left: the main trailer without
/// `Prev`, and without `XRefStm` when `Prev` was followed (`reader.rs:767-778`).
fn loaded_trailer(span: &TrailerSpan) -> Dictionary {
    let mut trailer = span.trailer.clone();
    if trailer.remove(b"Prev").is_some_and(|p| p.as_i64().is_ok()) {
        trailer.remove(b"XRefStm");
    }
    trailer
}

/// The load filter: a non empty object stream is renamed so that lopdf keeps
/// it whole (DEC-004). An empty one is left alone: lopdf reads its content later
/// from an indirect `Length` and treats it as today. lopdf ignores what this
/// returns for a top level object (`reader.rs:960`) and keeps the object it
/// passed; it uses the returned value only for the members of an object stream,
/// never streams, so a stream is not cloned for nothing.
fn defer_object_streams(id: ObjectId, object: &mut Object) -> Option<(ObjectId, Object)> {
    match object {
        Object::Stream(stream) => {
            if stream.dict.has_type(DEFERRED) {
                stream.dict.set("Type", Object::Name(FILE_TYPED.to_vec()));
            } else if stream.dict.has_type(b"ObjStm") && !stream.content.is_empty() {
                stream.dict.set("Type", Object::Name(DEFERRED.to_vec()));
            }
            Some((id, Object::Null))
        }
        other => Some((id, other.clone())),
    }
}

/// The text of a load or decryption failure, as the reader words it today.
fn pdf_error(e: pdf_extract::Error) -> ToolError {
    ToolError::invalid_argument(format!("pdf: {}", pdf_extract::OutputError::PdfError(e)))
}

/// Load `data` with its object streams deferred, then expand them one at a
/// time, in increasing object number, each measured before it is decoded. The
/// cross reference streams are measured first; an encrypted document is loaded
/// from a copy whose trailer does not name `Encrypt`, so the filter applies,
/// then decrypted by lopdf (DEC-005).
fn load(data: &[u8], budget: &mut PdfBudget) -> Result<Document> {
    let main = xref_guard(data, budget)?.filter(TrailerSpan::encrypted);
    let unlocked = main.as_ref().map(|span| unlock_encrypted(data, span)).transpose()?;
    let options = LoadOptions { filter: Some(defer_object_streams), ..Default::default() };
    let mut doc = Document::load_mem_with_options(unlocked.as_deref().unwrap_or(data), options)
        .map_err(pdf_error)?;
    drop(unlocked);
    if let Some(span) = &main {
        doc.trailer = loaded_trailer(span);
        // What lopdf's encrypted loader leaves when the empty password is
        // refused: the encryption dictionary alone (`reader.rs:832-856`).
        // The reader's `maybe_decrypt` then fails as today on the password.
        if doc.authenticate_password("").is_err() {
            let keep = doc.trailer.get(b"Encrypt").and_then(Object::as_reference).ok();
            doc.objects.retain(|id, _| Some(*id) == keep);
            if doc.is_encrypted() {
                doc.decrypt("").map_err(pdf_error)?;
            }
        } else if doc.is_encrypted() {
            // The object streams still deferred are expanded below once decrypted.
            decrypt_each(&mut doc)?;
        }
    }
    let encrypted = main.is_some();

    let containers: BTreeMap<u32, u32> = doc
        .reference_table
        .entries
        .iter()
        .filter_map(|(&n, entry)| match entry {
            XrefEntry::Compressed { container, .. } => Some((n, *container)),
            _ => None,
        })
        .collect();
    let deferred: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, o)| o.as_stream().is_ok_and(|s| s.dict.has_type(DEFERRED)))
        .map(|(id, _)| *id)
        .collect();
    // The objects outside the object streams, before any stream is decoded.
    budget.objects = doc.objects.len() as u64;
    if budget.objects > OBJECT_LIMIT {
        return Err(over_object_limit());
    }
    for id in deferred {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else { continue };
        let mut plain = plain_object_stream(stream, budget)?;
        // Counted from the index block, before `ObjectStream::new` parses a member.
        budget.objects = budget.objects.saturating_add(pair_count(&plain));
        if budget.objects > OBJECT_LIMIT {
            return Err(over_object_limit());
        }
        match ObjectStream::new(&mut plain) {
            Ok(parsed) => {
                drop(plain);
                for (member, object) in parsed.objects {
                    if encrypted {
                        // As the encrypted loader: only the numbers the cross
                        // reference places in this container, replacing
                        // (`reader.rs:887-897`).
                        if id.1 == 0 && member.1 == 0 && containers.get(&member.0) == Some(&id.0) {
                            doc.objects.insert(member, object);
                        }
                    } else if containers.get(&member.0).is_none_or(|c| *c == id.0) {
                        // As lopdf: a member the cross reference places in another
                        // container is a stale copy, and nothing loaded is replaced.
                        doc.objects.entry(member).or_insert(object);
                    }
                }
                if let Some(Object::Stream(container)) = doc.objects.get_mut(&id) {
                    // Freed: the members are loaded, nothing reads the container again.
                    container.set_plain_content(Vec::new());
                    container.dict.set("Type", Object::Name(b"ObjStm".to_vec()));
                }
            }
            // lopdf drops an object stream it cannot parse (`.ok()?`).
            Err(_) => {
                doc.objects.remove(&id);
            }
        }
    }
    Ok(doc)
}

/// Decrypt every object with the empty password as the encrypted loader does
/// (`reader.rs:856-866`): an object that fails to decrypt is kept as it is and
/// the next one is tried, where `Document::decrypt` stops at the first failure
/// (`document.rs:469`) and would refuse a file read today. The encryption
/// dictionary is skipped, then removed with the trailer's `Encrypt`, as both
/// do. O(objects) time.
fn decrypt_each(doc: &mut Document) -> Result<()> {
    let state = EncryptionState::decode(&*doc, "").map_err(pdf_error)?;
    let dictionary = doc.trailer.get(b"Encrypt").and_then(Object::as_reference).ok();
    for (id, object) in doc.objects.iter_mut() {
        if Some(*id) != dictionary {
            // Ignored per object, as the loader ignores it.
            let _ = decrypt_object(&state, *id, object);
        }
    }
    if let Some(id) = dictionary {
        doc.objects.remove(&id);
    }
    doc.trailer.remove(b"Encrypt");
    doc.encryption_state = Some(state);
    Ok(())
}

/// The object stream `stream` decoded as `ObjectStream::new` would decode it,
/// with no `Filter` left so that it does not decode it again. A stream this
/// module cannot measure exactly, or that lopdf fails to decode, keeps its
/// stored bytes, as lopdf keeps them when `decompress` fails.
fn plain_object_stream(stream: &Stream, budget: &mut PdfBudget) -> Result<Stream> {
    let mut dict = stream.dict.clone();
    dict.set("Type", Object::Name(b"ObjStm".to_vec()));
    let decoded = if measurable(stream) {
        let size = measure(stream, budget, Scope::Load)?;
        budget.hold(size);
        stream.decompressed_content().ok()
    } else {
        None
    };
    dict.remove(b"Filter");
    dict.remove(b"DecodeParms");
    Ok(Stream::new(dict, decoded.unwrap_or_else(|| stream.content.clone())))
}

/// The pairs in the index block of the plain object stream `stream`, counted as
/// `ObjectStream::new` splits them (`lopdf-0.42.0/src/object_stream.rs:60-70`):
/// whitespace separated numbers, two to a pair, whatever the numbers are. A
/// block it would refuse (no readable `First`, not UTF-8) holds no pair.
/// O(First) time, no allocation.
fn pair_count(stream: &Stream) -> u64 {
    let first = stream.dict.get(b"First").and_then(Object::as_i64).ok();
    let block = first.and_then(|f| usize::try_from(f).ok()).and_then(|f| stream.content.get(..f));
    let text = block.and_then(|b| std::str::from_utf8(b).ok());
    text.map_or(0, |t| t.split_whitespace().count() as u64 / 2)
}

/// Hand the reader empty stand ins for the colour profiles and tint transforms
/// it keeps (DEC-009): the reader copies a profile into every saved drawing
/// state, so a profile selected once and saved 300 times would be held 300
/// times. Every array opening with `/ICCBased` gets its stream reference
/// replaced, and every `/Separation` its tint transform's. A stand in is a new
/// object: the original dictionary without `Filter`, `DecodeParms` and
/// `Length`, and no content. The walk counts the original on every selection.
/// Returns original to stand in. O(objects) time.
fn install_stand_ins(doc: &mut Document) -> HashMap<ObjectId, ObjectId> {
    let mut wanted = BTreeSet::new();
    for object in doc.objects.values_mut() {
        visit_spaces(object, &mut |id| {
            wanted.insert(*id);
        });
    }
    let mut stand_ins = HashMap::new();
    for id in wanted {
        let Some(Object::Stream(original)) = doc.objects.get(&id) else { continue };
        let mut dict = original.dict.clone();
        for key in [&b"Filter"[..], b"DecodeParms", b"Length"] {
            dict.remove(key);
        }
        stand_ins.insert(id, doc.add_object(Stream::new(dict, Vec::new())));
    }
    for object in doc.objects.values_mut() {
        visit_spaces(object, &mut |id| {
            if let Some(stand_in) = stand_ins.get(id) {
                *id = *stand_in;
            }
        });
    }
    stand_ins
}

/// Call `f` on the stream reference of every `/ICCBased` profile and
/// `/Separation` tint transform inside `object`, however deep.
fn visit_spaces(object: &mut Object, f: &mut dyn FnMut(&mut ObjectId)) {
    match object {
        Object::Array(items) => {
            let at = match items.first().and_then(|o| o.as_name().ok()) {
                Some(b"ICCBased") => Some(1),
                Some(b"Separation") => Some(3),
                _ => None,
            };
            if let Some(Object::Reference(id)) = at.and_then(|i| items.get_mut(i)) {
                f(id);
            }
            for item in items {
                visit_spaces(item, f);
            }
        }
        Object::Dictionary(dict) => dict.iter_mut().for_each(|(_, v)| visit_spaces(v, f)),
        Object::Stream(stream) => stream.dict.iter_mut().for_each(|(_, v)| visit_spaces(v, f)),
        _ => {}
    }
}

/// Move every stream's `Filter` aside, so the reader inflates nothing it was
/// not handed (DEC-006). A `McpFsFilter` key the file itself wrote on a stream
/// without `Filter` is dropped, so arming can never give it a filter it lacked.
/// O(objects) time, no allocation beyond the moved values.
fn defuse(doc: &mut Document) {
    for object in doc.objects.values_mut() {
        if let Object::Stream(stream) = object {
            match stream.dict.remove(b"Filter") {
                Some(filter) => stream.dict.set(DEFUSED, filter),
                None => {
                    stream.dict.remove(DEFUSED);
                }
            }
        }
    }
}

/// Put `Filter` back on `ids`, or take it away again.
fn swap_filter(doc: &mut Document, ids: &BTreeSet<ObjectId>, arm: bool) {
    let (from, to): (&[u8], &[u8]) = if arm { (DEFUSED, b"Filter") } else { (b"Filter", DEFUSED) };
    for id in ids {
        if let Some(Object::Stream(stream)) = doc.objects.get_mut(id)
            && let Some(filter) = stream.dict.remove(from)
        {
            stream.dict.set(to, filter);
        }
    }
}

/// The streams one page may decode, armed for as long as this lives: dropping
/// it defuses them again on every exit path, a panic of the reader included.
struct Armed<'d> {
    doc: &'d mut Document,
    ids: BTreeSet<ObjectId>,
}

impl<'d> Armed<'d> {
    fn new(doc: &'d mut Document, ids: BTreeSet<ObjectId>) -> Self {
        swap_filter(doc, &ids, true);
        Self { doc, ids }
    }
}

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        swap_filter(self.doc, &self.ids, false);
    }
}

/// The filter names of `stream` as `Stream::filters` reads them, from `Filter`
/// or, once defused, from where it was moved. `None` where lopdf's `filters()`
/// fails: the reader then copies the stored bytes.
fn filters(stream: &Stream) -> Option<Vec<&[u8]>> {
    let filter = stream.dict.get(b"Filter").or_else(|_| stream.dict.get(DEFUSED)).ok()?;
    if let Ok(name) = filter.as_name() {
        Some(vec![name])
    } else {
        filter.as_array().ok()?.iter().map(|f| f.as_name().ok()).collect()
    }
}

/// The two row buffers a PNG predictor allocates, when `DecodeParms` asks for
/// one (`object.rs:929-948`, `filters/png.rs:87-95`).
fn predictor_rows(stream: &Stream) -> Option<u64> {
    let params = stream.dict.get(b"DecodeParms").and_then(Object::as_dict).ok()?;
    let int =
        |key: &[u8], default: i64| params.get(key).and_then(Object::as_i64).unwrap_or(default);
    if !(10..=15).contains(&int(b"Predictor", 1)) {
        return None;
    }
    let columns = int(b"Columns", 1).max(1) as u128;
    let colors = int(b"Colors", 1).max(1) as u128;
    let bits = int(b"BitsPerComponent", 8).max(8) as u128;
    let row = (colors.saturating_mul(bits) / 8).saturating_mul(columns);
    Some(u64::try_from(row.saturating_mul(2)).unwrap_or(u64::MAX))
}

/// Whether a filter name is one lopdf decodes (`object.rs:807-812`).
fn known(name: &[u8]) -> bool {
    matches!(name, b"FlateDecode" | b"LZWDecode" | b"ASCII85Decode")
}

/// Whether lopdf runs the predictor of `DecodeParms` after this step: it does
/// after an inflation and after an LZW decode, not after ASCII85.
fn predicting(name: &[u8]) -> bool {
    matches!(name, b"FlateDecode" | b"LZWDecode")
}

/// Whether [`measure`] counts exactly what the reader would produce. A predictor
/// reshapes the bytes between two steps, which the streaming measurer does not
/// reproduce: a stream whose predictor runs before another step, or more than
/// once, is left stored. A chain longer than [`MAX_FILTERS`] is left stored too.
fn measurable(stream: &Stream) -> bool {
    let Some(names) = filters(stream) else { return true };
    if names.len() > MAX_FILTERS {
        return false;
    }
    let chain: Vec<&[u8]> = names.into_iter().take_while(|n| known(n)).collect();
    let predicted = chain.iter().filter(|n| predicting(n)).count();
    predicted == 0
        || predictor_rows(stream).is_none()
        || (predicted == 1 && chain.last().is_some_and(|n| predicting(n)))
}

#[cfg(test)]
thread_local! {
    /// Inflate decoders the measurer created on this thread.
    static DECODERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Largest decoded size of the forms the walk kept parsed at once.
    static FORMS_PEAK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn new_decoder(zlib_header: bool) -> flate2::Decompress {
    #[cfg(test)]
    DECODERS.with(|c| c.set(c.get() + 1));
    flate2::Decompress::new(zlib_header)
}

/// One inflation step of the measurer: the zlib pass lopdf runs, and the raw
/// pass over the input after its first two bytes that lopdf falls back to when
/// the zlib pass yields nothing (`object.rs:849-872`).
struct Inflate {
    /// The decoders exist: they are created when the step first gets input,
    /// so a step never reached costs nothing.
    started: bool,
    zlib: Option<flate2::Decompress>,
    raw: Option<flate2::Decompress>,
    zlib_out: u64,
    /// Bytes of the step's input still to drop before the raw pass.
    skip: usize,
}

/// One ASCII85 step, a streaming copy of `decode_ascii85` (`object.rs:874-920`).
/// A trailing `~>` needs no special case: `~` is outside `!` to `u`, so it ends
/// the input as its removal would, and the partial group before it counts.
struct Ascii85 {
    buffer: u32,
    count: usize,
    /// A byte outside the alphabet ended the input; the partial group still counts.
    ended: bool,
    /// lopdf fails the whole decode: nothing more is output.
    failed: bool,
}

/// One step of the chain, decoding as lopdf decodes it.
enum Step {
    Flate(Box<Inflate>),
    /// The decoder is configured as lopdf configures it (`EarlyChange`,
    /// `object.rs:818-846`); `None` once it ended or failed.
    Lzw(Option<weezl::decode::Decoder>),
    Ascii85(Ascii85),
}

impl Step {
    fn new(name: &[u8], early_change: bool) -> Self {
        use weezl::{BitOrder, decode::Decoder};
        match name {
            b"LZWDecode" if early_change => {
                Step::Lzw(Some(Decoder::with_tiff_size_switch(BitOrder::Msb, 8)))
            }
            b"LZWDecode" => Step::Lzw(Some(Decoder::new(BitOrder::Msb, 8))),
            b"ASCII85Decode" => {
                Step::Ascii85(Ascii85 { buffer: 0, count: 0, ended: false, failed: false })
            }
            _ => Step::Flate(Box::new(Inflate {
                started: false,
                zlib: None,
                raw: None,
                zlib_out: 0,
                skip: 2,
            })),
        }
    }

    /// Decode some of `frame` into `out`: `None` when this step can take
    /// nothing more from the frame, else the bytes produced (0 when it only
    /// consumed input).
    fn feed(&mut self, frame: &mut Frame<'_>, out: &mut [u8]) -> Option<usize> {
        let len = frame.data.len();
        match self {
            Step::Flate(step) => {
                if !step.started {
                    step.started = true;
                    step.zlib = Some(new_decoder(true));
                    step.raw = Some(new_decoder(false));
                }
                if step.zlib.is_some() && (frame.at < len || frame.full) {
                    let (used, made) = inflate_some(&mut step.zlib, &frame.data[frame.at..], out);
                    frame.at += used;
                    frame.full = made == CHUNK;
                    step.zlib_out += made as u64;
                    Some(made)
                } else if step.zlib_out == 0
                    && step.raw.is_some()
                    && (frame.raw_at < len || frame.raw_full)
                {
                    let skip = step.skip.min(len - frame.raw_at);
                    step.skip -= skip;
                    frame.raw_at += skip;
                    let (used, made) =
                        inflate_some(&mut step.raw, &frame.data[frame.raw_at..], out);
                    frame.raw_at += used;
                    frame.raw_full = made == CHUNK;
                    Some(made)
                } else {
                    None
                }
            }
            Step::Lzw(decoder) => {
                let d = decoder.as_mut()?;
                if frame.at >= len && !frame.full {
                    return None;
                }
                let result = d.decode_bytes(&frame.data[frame.at..], out);
                frame.at += result.consumed_in;
                frame.full = result.consumed_out == out.len();
                // lopdf's `decode_all` stops at the end code, an error or no
                // progress; the output of the call that failed is over counted,
                // which is safe. No progress here only means the frame is spent.
                match result.status {
                    Ok(weezl::LzwStatus::Ok) => {}
                    Ok(weezl::LzwStatus::NoProgress) => return None,
                    Ok(weezl::LzwStatus::Done) | Err(_) => *decoder = None,
                }
                if result.consumed_in == 0 && result.consumed_out == 0 {
                    return None;
                }
                Some(result.consumed_out)
            }
            Step::Ascii85(step) => {
                if step.ended || frame.at >= len {
                    return None;
                }
                let mut made = 0;
                while frame.at < len && made + 4 <= out.len() {
                    let ch = frame.data[frame.at];
                    frame.at += 1;
                    if ch == b'z' {
                        if step.count != 0 {
                            // lopdf fails the whole decode here; what was
                            // counted is an over count, which is safe.
                            (step.ended, step.failed) = (true, true);
                            break;
                        }
                        out[made..made + 4].fill(0);
                        made += 4;
                        continue;
                    }
                    if ch.is_ascii_whitespace() {
                        continue;
                    }
                    if !(b'!'..=b'u').contains(&ch) {
                        step.ended = true;
                        break;
                    }
                    let Some(next) = step.buffer.checked_mul(85) else {
                        (step.ended, step.failed) = (true, true);
                        break;
                    };
                    step.buffer = next.wrapping_add(u32::from(ch - b'!'));
                    step.count += 1;
                    if step.count == 5 {
                        out[made..made + 4].copy_from_slice(&step.buffer.to_be_bytes());
                        made += 4;
                        step.buffer = 0;
                        step.count = 0;
                    }
                }
                Some(made)
            }
        }
    }

    /// The bytes this step still outputs once its input is complete: a final
    /// partial ASCII85 group gives `count - 1` bytes, padded as lopdf pads it.
    /// Where lopdf fails instead, nothing (the bytes before were counted).
    fn finish(&mut self) -> Vec<u8> {
        let Step::Ascii85(step) = self else { return Vec::new() };
        if step.failed || step.count == 0 {
            return Vec::new();
        }
        (step.ended, step.failed) = (true, true);
        let mut buffer = step.buffer;
        for _ in step.count..5 {
            let Some(next) = buffer.checked_mul(85) else { return Vec::new() };
            buffer = next.wrapping_add(84);
        }
        buffer.to_be_bytes()[..step.count - 1].to_vec()
    }
}

/// A piece of a step's input: the stored bytes for the first step, a decoded
/// chunk of the step before for the others.
struct Frame<'s> {
    step: usize,
    data: std::borrow::Cow<'s, [u8]>,
    /// Bytes of `data` consumed by the step (the zlib pass for an inflation).
    at: usize,
    /// Bytes of `data` consumed by the raw pass of an inflation.
    raw_at: usize,
    /// The last call filled the chunk, so the decoder may hold more output.
    full: bool,
    raw_full: bool,
}

impl<'s> Frame<'s> {
    fn new(step: usize, data: std::borrow::Cow<'s, [u8]>) -> Self {
        Self { step, data, at: 0, raw_at: 0, full: false, raw_full: false }
    }
}

/// Feed `input` to `decoder`, at most one chunk out; returns the bytes consumed
/// and produced. The decoder is dropped once its stream ended or failed, or
/// when it can make no progress on input it was given.
fn inflate_some(
    decoder: &mut Option<flate2::Decompress>,
    input: &[u8],
    out: &mut [u8],
) -> (usize, usize) {
    let Some(d) = decoder else { return (0, 0) };
    let (before_in, before_out) = (d.total_in(), d.total_out());
    let status = d.decompress(input, out, flate2::FlushDecompress::None);
    let consumed = (d.total_in() - before_in) as usize;
    let produced = (d.total_out() - before_out) as usize;
    let stuck = consumed == 0 && produced == 0 && !input.is_empty();
    if stuck || !matches!(status, Ok(flate2::Status::Ok | flate2::Status::BufError)) {
        *decoder = None;
    }
    (consumed, produced)
}

/// The running count of one measure: what each step output, plus a
/// predictor's row buffers and output, checked against the stream limit.
struct Count<'b> {
    total: u64,
    /// Bytes the load has measured before this stream, when this one is an
    /// object stream: its running sum is the load total.
    load_base: Option<u64>,
    /// Bytes this page has decoded before this stream, when it is a page stream.
    page_base: Option<u64>,
    /// The last step runs the predictor: its output is counted twice.
    predicted_last: Option<usize>,
    budget: &'b mut PdfBudget,
}

impl Count<'_> {
    fn add(&mut self, step: usize, produced: usize) -> Result<()> {
        // The predictor's output, at most its input, is counted as its input.
        let counted = if self.predicted_last == Some(step) { 2 * produced } else { produced };
        self.total = self.total.saturating_add(counted as u64);
        // The bytes really decoded so far, so a refusal records what it cost.
        self.budget.hold(self.total);
        if let Some(base) = self.page_base {
            self.budget.hold(base.saturating_add(self.total));
        }
        // Checked after every chunk, so decoding stops at most one chunk past
        // the limit; equal is accepted.
        if self.total > STREAM_LIMIT {
            return Err(over_stream_limit());
        }
        // The load total, on the running sum, after the stream limit.
        if self.load_base.is_some_and(|base| base.saturating_add(self.total) > LOAD_LIMIT) {
            return Err(over_load_limit());
        }
        // The page total, on the running sum, after the stream limit (DEC-003).
        if self.page_base.is_some_and(|base| base.saturating_add(self.total) > PAGE_LIMIT) {
            return Err(over_page_limit(self.budget.page_no));
        }
        Ok(())
    }
}

/// Run the frames on `stack` depth first until it is empty: each chunk a step
/// outputs is counted, then pushed as the next step's input.
fn drain(
    steps: &mut [Step],
    stack: &mut Vec<Frame<'_>>,
    out: &mut [u8],
    count: &mut Count<'_>,
) -> Result<()> {
    while let Some(frame) = stack.last_mut() {
        let at = frame.step;
        let Some(produced) = steps[at].feed(frame, out) else {
            stack.pop();
            continue;
        };
        if produced == 0 {
            continue;
        }
        count.add(at, produced)?;
        if at + 1 < steps.len() {
            stack.push(Frame::new(at + 1, std::borrow::Cow::Owned(out[..produced].to_vec())));
        }
    }
    Ok(())
}

/// The decoded size of `stream` as the reader would produce it, decoding one
/// [`CHUNK`] at a time and keeping none of it (DEC-003): the sum of what each
/// step outputs, plus a predictor's row buffers and output. A chain whose first
/// filter is unknown counts 0 (the reader copies the stored bytes); an unknown
/// filter later ends the chain, the steps before it counted.
///
/// Steps are fed depth first from an explicit stack, so the held data is one
/// chunk per step and no recursion. Once a step's input is complete its tail
/// (a final ASCII85 group) goes to the next step. O(decoded bytes) time.
fn measure(stream: &Stream, budget: &mut PdfBudget, scope: Scope) -> Result<u64> {
    let Some(names) = filters(stream) else { return Ok(0) };
    if names.len() > MAX_FILTERS {
        return Ok(0);
    }
    let chain: Vec<&[u8]> = names.into_iter().take_while(|n| known(n)).collect();
    let params = stream.dict.get(b"DecodeParms").and_then(Object::as_dict).ok();
    let early_change = params
        .and_then(|p| p.get(b"EarlyChange").and_then(Object::as_i64).ok())
        .is_none_or(|v| v != 0);
    let rows = predictor_rows(stream);
    let predicted_last =
        chain.last().filter(|n| rows.is_some() && predicting(n)).map(|_| chain.len() - 1);
    let mut count = Count {
        total: if predicted_last.is_some() { rows.unwrap_or(0) } else { 0 },
        predicted_last,
        load_base: matches!(scope, Scope::Load).then_some(budget.load),
        page_base: matches!(scope, Scope::Page).then_some(budget.page),
        budget,
    };
    if count.total > STREAM_LIMIT {
        return Err(over_stream_limit());
    }
    let mut steps: Vec<Step> = chain.iter().map(|n| Step::new(n, early_change)).collect();
    let mut out = vec![0u8; CHUNK];
    let mut stack = Vec::new();
    if !steps.is_empty() && !stream.content.is_empty() {
        stack.push(Frame::new(0, std::borrow::Cow::Borrowed(stream.content.as_slice())));
    }
    drain(&mut steps, &mut stack, &mut out, &mut count)?;
    for at in 0..steps.len() {
        let tail = steps[at].finish();
        if tail.is_empty() {
            continue;
        }
        count.add(at, tail.len())?;
        if at + 1 < steps.len() {
            stack.push(Frame::new(at + 1, std::borrow::Cow::Owned(tail)));
            drain(&mut steps, &mut stack, &mut out, &mut count)?;
        }
    }
    let total = count.total;
    match scope {
        Scope::Page => budget.page = budget.page.saturating_add(total),
        Scope::Load => budget.load = budget.load.saturating_add(total),
        Scope::Guard => {}
    }
    Ok(total)
}

/// What the walk of one page found.
#[derive(Default)]
struct Walked {
    /// The streams to arm.
    armed: BTreeSet<ObjectId>,
    /// The stand ins of tint transforms whose original is not valid UTF-8.
    invalid: BTreeSet<ObjectId>,
}

/// The object `obj` leads to, with the id of the last reference followed.
fn resolve<'d>(doc: &'d Document, obj: &'d Object) -> Option<(Option<ObjectId>, &'d Object)> {
    doc.dereference(obj).ok()
}

/// The stream `dict[key]` refers to, by id.
fn stream_at(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<ObjectId> {
    stream_of(doc, dict.get(key).ok()?)
}

fn stream_of(doc: &Document, obj: &Object) -> Option<ObjectId> {
    match resolve(doc, obj)? {
        (Some(id), Object::Stream(_)) => Some(id),
        _ => None,
    }
}

/// The dictionary `dict[key]` leads to.
fn dict_at<'d>(doc: &'d Document, dict: &'d Dictionary, key: &[u8]) -> Option<&'d Dictionary> {
    resolve(doc, dict.get(key).ok()?)?.1.as_dict().ok()
}

/// Deepest chain of page groups above a page the reader is allowed to climb for
/// its resources or its page size (SPEC-0092 DEC-002). The reader climbs by
/// recursion with no memory of the nodes seen, so a loop never ends.
pub(crate) const CLIMB_LIMIT: usize = 256;

/// The value of `key` the reader's `get_inherited` takes for `page`: the first
/// node, the page itself then each `Parent`, whose `key` (resolved one step) is
/// `usable`. A missing, non reference or unresolvable `Parent` ends the climb
/// with `None`. Level `CLIMB_LIMIT + 1` is refused, so a loop counts as passing
/// the limit; there is deliberately no `seen` set. O(CLIMB_LIMIT) time, O(1) space.
fn climb<'d>(
    doc: &'d Document,
    page: &'d Dictionary,
    key: &[u8],
    usable: fn(&Object) -> bool,
    page_no: u32,
) -> Result<Option<&'d Object>> {
    let mut node = page;
    for level in 0..=CLIMB_LIMIT + 1 {
        if level > CLIMB_LIMIT {
            return Err(ToolError::invalid_argument(format!(
                "page {page_no} inherits from more than {CLIMB_LIMIT} levels of the PDF page tree"
            )));
        }
        if let Some(found) = node.get(key).ok().and_then(|o| resolve(doc, o)).map(|(_, o)| o)
            && usable(found)
        {
            return Ok(Some(found));
        }
        let Some(parent) = node.get(b"Parent").and_then(Object::as_reference).ok() else {
            return Ok(None);
        };
        let Ok(next) = doc.get_dictionary(parent) else { return Ok(None) };
        node = next;
    }
    Ok(None)
}

/// The bytes the reader gets from `stream`: when `armed`, decoded if its
/// filters are known, else the stored bytes (`get_contents`, `lib.rs:1255-1261`).
/// The stored bytes are borrowed; decoding copies them once, since lopdf decodes
/// only a `Stream` whose dictionary holds `Filter`.
fn reader_bytes(stream: &Stream, armed: bool) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if !armed || stream.dict.get(DEFUSED).is_err() {
        return Cow::Borrowed(&stream.content);
    }
    let mut armed = Stream::new(Dictionary::new(), Vec::new());
    armed.dict = stream.dict.clone();
    if let Some(filter) = armed.dict.remove(DEFUSED) {
        armed.dict.set("Filter", filter);
    }
    armed.content = stream.content.clone();
    if armed.filters().is_ok() {
        Cow::Owned(armed.decompressed_content().unwrap_or_else(|_| armed.content.clone()))
    } else {
        Cow::Owned(armed.content)
    }
}

/// One page's walk state: the streams armed, their sizes measured once.
struct PageWalk<'d> {
    doc: &'d Document,
    armed: BTreeSet<ObjectId>,
    sizes: HashMap<ObjectId, u64>,
}

impl PageWalk<'_> {
    /// Count one decode of stream `id` by the reader: measured the first time on
    /// this page, its size reused for the repeats (DEC-008).
    fn decode(&mut self, id: ObjectId, budget: &mut PdfBudget) -> Result<u64> {
        if let Some(size) = self.sizes.get(&id) {
            budget.page = budget.page.saturating_add(*size);
            // A repeat adds its cached size at once and is checked right after;
            // it decodes nothing, so a refused sum is never recorded as held.
            if budget.page > PAGE_LIMIT {
                return Err(over_page_limit(budget.page_no));
            }
            budget.hold(budget.page);
            return Ok(*size);
        }
        let Ok(Object::Stream(stream)) = self.doc.get_object(id) else { return Ok(0) };
        let size = if measurable(stream) {
            let size = measure(stream, budget, Scope::Page)?;
            self.armed.insert(id);
            size
        } else {
            0
        };
        self.sizes.insert(id, size);
        Ok(size)
    }
}

impl PageWalk<'_> {
    /// The walk's result: the streams to arm and the stand ins to fill.
    fn into_walked(self, invalid: BTreeSet<ObjectId>) -> Walked {
        Walked { armed: self.armed, invalid }
    }

    /// Whether tint transform `id`, of `FunctionType` 4, decodes to bytes that
    /// are not valid UTF-8, which pdf-extract would format with an `unwrap`
    /// (DEC-009). O(decoded bytes) time; holds the decoded transform, counted
    /// by the caller.
    fn invalid_text(&self, id: ObjectId) -> bool {
        let Ok(Object::Stream(stream)) = self.doc.get_object(id) else { return false };
        let typed = stream.dict.get(b"FunctionType").and_then(Object::as_i64);
        typed.is_ok_and(|t| t == 4)
            && std::str::from_utf8(&reader_bytes(stream, self.armed.contains(&id))).is_err()
    }
}

/// A content stream or form being walked.
struct Drawing<'d> {
    ops: Rc<Vec<Operation>>,
    at: usize,
    resources: &'d Dictionary,
    /// Decoded size of the form, 0 for the page content.
    size: u64,
}

/// Deepest chain of forms the reader is allowed to recurse through; the page
/// content is level 0. Measured: 200 nested forms still run on a 2 MiB stack,
/// about 400 abort the process (SPEC-0092 DEC-002).
pub(crate) const FORM_DEPTH_LIMIT: usize = 64;

/// Walk page `page_id` the way pdf-extract will run it (DEC-007), measuring
/// every stream it may decode in drawing order; returns the streams to arm.
/// Where the reader would panic the walk stops without refusing, and the
/// reader then panics as today. O(operations drawn) time; the parsed
/// operations held are the page's and those of the forms on the drawing stack:
/// a form's are dropped when it is popped, and parsed again if drawn again, as
/// the reader does.
fn walk(
    doc: &Document,
    page_id: ObjectId,
    originals: &HashMap<ObjectId, ObjectId>,
    budget: &mut PdfBudget,
) -> Result<Walked> {
    budget.page = 0;
    let mut page = PageWalk { doc, armed: BTreeSet::new(), sizes: HashMap::new() };
    let mut invalid = BTreeSet::new();
    let Ok(page_dict) = doc.get_dictionary(page_id) else { return Ok(page.into_walked(invalid)) };
    let empty = Dictionary::new();
    let resources = climb(doc, page_dict, b"Resources", |o| o.as_dict().is_ok(), budget.page_no)?
        .and_then(|o| o.as_dict().ok())
        .unwrap_or(&empty);
    // The reader climbs again for the page size; its value is not used here.
    climb(doc, page_dict, b"MediaBox", |o| o.as_array().is_ok(), budget.page_no)?;

    let mut size = 0u64;
    let mut ids = Vec::new();
    for id in doc.get_page_contents(page_id) {
        if let Some(id) = stream_of(doc, &Object::Reference(id)) {
            size = size.saturating_add(page.decode(id, budget)?);
            ids.push(id);
        }
    }
    budget.hold(size);
    let mut content = Vec::new();
    for id in ids {
        if let Ok(Object::Stream(stream)) = doc.get_object(id) {
            content.extend_from_slice(&reader_bytes(stream, page.armed.contains(&id)));
            content.push(b'\n');
        }
    }
    let Ok(parsed) = Content::decode(&content) else { return Ok(page.into_walked(invalid)) };
    drop(content);

    let mut fonts: HashSet<Vec<u8>> = HashSet::new();
    // Decoded size of the forms on the stack, whose operations are held.
    let mut retained = 0u64;
    let mut stack = vec![Drawing { ops: Rc::new(parsed.operations), at: 0, resources, size: 0 }];
    while let Some(top) = stack.last_mut() {
        // A shared handle, so an operation is borrowed in place, never copied.
        let ops = Rc::clone(&top.ops);
        let Some(op) = ops.get(top.at) else {
            if let Some(done) = stack.pop() {
                retained = retained.saturating_sub(done.size);
            }
            continue;
        };
        top.at += 1;
        let resources = top.resources;
        let name = || op.operands.first().and_then(|o| o.as_name().ok());
        match op.operator.as_str() {
            "Tf" => {
                let Some(name) = name() else { break };
                if !fonts.insert(name.to_vec()) {
                    continue;
                }
                let Some(font) =
                    dict_at(doc, resources, b"Font").and_then(|f| dict_at(doc, f, name))
                else {
                    break;
                };
                for (id, is_map) in font_streams(doc, font) {
                    page.decode(id, budget)?;
                    // Scanned after the stream limit and the page total (both in `decode`),
                    // so a map that is too big is refused for its size first (DEC-005).
                    if is_map && let Ok(Object::Stream(map)) = doc.get_object(id) {
                        let bytes = reader_bytes(map, page.armed.contains(&id));
                        budget.hold(bytes.len() as u64);
                        if map_nesting(&bytes) > MAP_NESTING_LIMIT {
                            return Err(ToolError::invalid_argument(format!(
                                "a PDF character map is nested more than {MAP_NESTING_LIMIT} deep"
                            )));
                        }
                    }
                }
            }
            "cs" | "CS" => {
                let Some(name) = name() else { break };
                if matches!(name, b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"Pattern") {
                    continue;
                }
                let Some(space) = dict_at(doc, resources, b"ColorSpace")
                    .and_then(|c| c.get(name).ok())
                    .and_then(|o| resolve(doc, o))
                else {
                    break;
                };
                for id in colour_streams(doc, space.1) {
                    // The reader holds the stand in; the original is what it would decode.
                    let original = originals.get(&id).copied().unwrap_or(id);
                    let size = page.decode(original, budget)?;
                    if original != id && !invalid.contains(&id) {
                        budget.hold(size);
                        if page.invalid_text(original) {
                            invalid.insert(id);
                        }
                    }
                }
            }
            "Do" => {
                let Some(name) = name() else { break };
                let Some(id) =
                    dict_at(doc, resources, b"XObject").and_then(|x| stream_at(doc, x, name))
                else {
                    break;
                };
                // The stack is the reader's recursion (one `process_stream` per level),
                // so its length is the depth of the form about to be drawn. Refused
                // before the form is measured, and a self draw is walked, not skipped:
                // it reaches the limit like any chain (SPEC-0092 DEC-003).
                if stack.len() > FORM_DEPTH_LIMIT {
                    return Err(ToolError::invalid_argument(format!(
                        "page {} draws PDF forms nested more than {FORM_DEPTH_LIMIT} deep",
                        budget.page_no
                    )));
                }
                let size = page.decode(id, budget)?;
                let Ok(Object::Stream(xobject)) = doc.get_object(id) else { break };
                let inner = dict_at(doc, &xobject.dict, b"Resources").unwrap_or(resources);
                budget.hold(size);
                let bytes = reader_bytes(xobject, page.armed.contains(&id));
                let Ok(parsed) = Content::decode(&bytes) else { break };
                drop(bytes);
                retained = retained.saturating_add(size);
                budget.hold(retained);
                #[cfg(test)]
                FORMS_PEAK.with(|c| c.set(c.get().max(retained)));
                let ops = Rc::new(parsed.operations);
                stack.push(Drawing { ops, at: 0, resources: inner, size });
            }
            _ => {}
        }
    }
    Ok(page.into_walked(invalid))
}

/// Deepest nesting of lists, dictionaries and bracketed strings the cmap parser
/// is allowed to recurse through; it goes one stack frame deeper per level
/// (SPEC-0092 DEC-002).
pub(crate) const MAP_NESTING_LIMIT: u32 = 64;

/// The deepest level the cmap parser would recurse to on `bytes`, lexed as it
/// lexes (`adobe-cmap-parser` `lib.rs:28-150`): outside a string `[` and `<<`
/// open a level, `]` and `>>` close one, a lone `<` starts a hexadecimal string
/// skipped to `>`, `%` a comment skipped to the end of the line, `(` a string
/// level; inside a string `\` skips the next byte, `(` opens and `)` closes a
/// level, every other byte is text. Levels of the three kinds are counted
/// together, unclosed ones included. Returns as soon as the depth passes
/// [`MAP_NESTING_LIMIT`]. Iterative, O(n) time, O(1) space.
pub(crate) fn map_nesting(bytes: &[u8]) -> u32 {
    let (mut depth, mut max, mut string) = (0u32, 0u32, 0u32);
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        at += 1;
        if string > 0 {
            match byte {
                b'\\' => at += 1,
                b'(' => {
                    string += 1;
                    depth += 1;
                }
                b')' => {
                    string -= 1;
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        } else {
            match byte {
                b'[' | b'(' => {
                    depth += 1;
                    string = u32::from(byte == b'(');
                }
                b']' => depth = depth.saturating_sub(1),
                b'<' if bytes.get(at) == Some(&b'<') => {
                    at += 1;
                    depth += 1;
                }
                b'>' if bytes.get(at) == Some(&b'>') => {
                    at += 1;
                    depth = depth.saturating_sub(1);
                }
                b'<' => {
                    while at < bytes.len() && bytes[at] != b'>' {
                        at += 1;
                    }
                    at += 1;
                }
                b'%' => {
                    while at < bytes.len() && !matches!(bytes[at], b'\n' | b'\r') {
                        at += 1;
                    }
                }
                _ => {}
            }
        }
        max = max.max(depth);
        if max > MAP_NESTING_LIMIT {
            return max;
        }
    }
    max
}

/// The streams `make_font` decodes for `font`, by its `Subtype` (`lib.rs:322-332`),
/// each with whether it is a character map or code map the reader parses
/// (`ToUnicode`, a composite font's `Encoding`); font files are not.
fn font_streams(doc: &Document, font: &Dictionary) -> Vec<(ObjectId, bool)> {
    let subtype =
        font.get(b"Subtype").ok().and_then(|o| resolve(doc, o)).and_then(|o| o.1.as_name().ok());
    let mut ids = Vec::new();
    match subtype {
        Some(b"Type0") => ids.extend(stream_at(doc, font, b"Encoding").map(|id| (id, true))),
        // The reader panics on a font without `Subtype`: nothing is decoded.
        None => return ids,
        Some(b"Type3") => {}
        Some(other) => {
            if let Some(descriptor) = dict_at(doc, font, b"FontDescriptor") {
                match other {
                    b"Type1" => {
                        ids.extend(stream_at(doc, descriptor, b"FontFile").map(|i| (i, false)))
                    }
                    b"TrueType" => {
                        ids.extend(stream_at(doc, descriptor, b"FontFile2").map(|i| (i, false)))
                    }
                    _ => {}
                }
                ids.extend(stream_at(doc, descriptor, b"FontFile3").map(|i| (i, false)));
            }
        }
    }
    ids.extend(stream_at(doc, font, b"ToUnicode").map(|id| (id, true)));
    ids
}

/// The streams `make_colorspace` decodes for the colour space `space`
/// (`lib.rs:1449-1525`): an `ICCBased` profile, or a `Separation`'s alternate
/// profile and its tint transform of `FunctionType` 0 or 4 (`lib.rs:1168-1225`).
fn colour_streams(doc: &Document, space: &Object) -> Vec<ObjectId> {
    let Ok(array) = space.as_array() else { return Vec::new() };
    let first = |a: &Vec<Object>| a.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec);
    let mut ids = Vec::new();
    match first(array).as_deref() {
        Some(b"ICCBased") => ids.extend(array.get(1).and_then(|o| stream_of(doc, o))),
        Some(b"Separation") => {
            if let Some((_, Object::Array(alternate))) = array.get(2).and_then(|o| resolve(doc, o))
                && first(alternate).as_deref() == Some(b"ICCBased")
            {
                ids.extend(alternate.get(1).and_then(|o| stream_of(doc, o)));
            }
            if let Some(id) = array.get(3).and_then(|o| stream_of(doc, o))
                && let Ok(Object::Stream(f)) = doc.get_object(id)
                && matches!(f.dict.get(b"FunctionType").and_then(Object::as_i64), Ok(0 | 4))
            {
                ids.push(id);
            }
        }
        _ => {}
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::pdf_testkit::*;

    const HELLO: &str = "BT /F1 12 Tf 72 700 Td (Hello) Tj ET";

    fn decoders() -> usize {
        DECODERS.with(std::cell::Cell::get)
    }

    /// A page content of [`HELLO`] stored as is, its dictionary naming `filter`.
    fn filtered_page(filter: &str) -> Vec<u8> {
        let dict = format!("<< /Length {} /Filter {filter} >>", HELLO.len());
        page_pdf(PdfObj::Stream(dict, HELLO.as_bytes().to_vec()), "", vec![])
    }

    #[test]
    fn a_long_filter_chain_creates_no_decoder_and_keeps_the_text() {
        let chain = format!("[{}]", vec!["/FlateDecode"; 10_000].join(" "));
        let long = filtered_page(&chain);
        let stored = page_pdf(plain(HELLO), "", vec![]);
        let before = decoders();

        let text = extract(&long).expect("answered").body;

        assert_eq!(decoders() - before, 0, "a 10 000 step chain created decoders");
        // Left stored: the text of the same content without any filter.
        assert_eq!(text, extract(&stored).expect("answered").body);
        assert!(text.contains("Hello"), "{text}");
    }

    #[test]
    fn an_empty_stream_creates_no_decoder() {
        let stream = Stream::new(
            Dictionary::from_iter(vec![("Filter", Object::Name(b"FlateDecode".to_vec()))]),
            Vec::new(),
        );
        let before = decoders();

        let size = measure(&stream, &mut PdfBudget::default(), Scope::Page).expect("measured");

        assert_eq!(size, 0);
        assert_eq!(decoders() - before, 0, "decoders created for an empty stream");
    }

    #[test]
    fn a_form_drawn_and_done_is_not_kept_parsed() {
        const FORMS: usize = 20;
        const FORM_SIZE: usize = 1_000_000;
        let names: Vec<String> = (0..FORMS).map(|i| format!("/X{i} {} 0 R", 10 + i)).collect();
        let draws: Vec<String> = (0..FORMS).map(|i| format!("/X{i} Do")).collect();
        let content = format!("{HELLO} {}", draws.join(" "));
        let extra = (0..FORMS)
            .map(|i| {
                let stored = zlib_run("q Q", b' ', FORM_SIZE - 3, "");
                let form = flate("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", stored);
                (10 + i as u32, form)
            })
            .collect();
        let doc = page_pdf(plain(&content), &format!("/XObject << {} >>", names.join(" ")), extra);
        FORMS_PEAK.with(|c| c.set(0));

        let text = extract(&doc).expect("answered").body;

        assert!(text.contains("Hello"), "{text}");
        let peak = FORMS_PEAK.with(std::cell::Cell::get);
        assert_eq!(peak, FORM_SIZE as u64, "forms kept parsed at once: {peak} decoded bytes");
    }

    /// An object stream of member 10, typed `type_name`, outside the cross reference.
    fn typed_stream_page(type_name: &str) -> Vec<u8> {
        let content = "10 0 (Injected) ";
        let dict = format!("<< /Type /{type_name} /N 1 /First 5 /Length {} >>", content.len());
        page_pdf(plain(HELLO), "", vec![(8, PdfObj::Stream(dict, content.as_bytes().to_vec()))])
    }

    #[test]
    fn a_stream_the_file_typed_as_deferred_is_not_expanded() {
        let marked = typed_stream_page("McpFsObjStm");
        let neutral = typed_stream_page("Other");

        let doc = load(&marked, &mut PdfBudget::default()).expect("loaded");

        assert!(!doc.objects.contains_key(&(10, 0)), "member 10 was injected");
        assert_eq!(
            extract(&marked).expect("answered").body,
            extract(&neutral).expect("answered").body
        );
    }

    /// The sum over the steps of what lopdf outputs for `stream`: each prefix
    /// of the chain decoded by lopdf itself.
    fn lopdf_steps(stream: &Stream) -> u64 {
        let names: Vec<Object> = match stream.dict.get(b"Filter").unwrap() {
            Object::Array(a) => a.clone(),
            name => vec![name.clone()],
        };
        (1..=names.len())
            .map(|n| {
                let mut prefix = stream.clone();
                prefix.dict.set("Filter", Object::Array(names[..n].to_vec()));
                prefix.decompressed_content().unwrap().len() as u64
            })
            .sum()
    }

    fn filtered(filters: &[&str], params: Option<Dictionary>, content: Vec<u8>) -> Stream {
        let names = filters.iter().map(|f| Object::Name(f.as_bytes().to_vec())).collect();
        let mut dict = Dictionary::from_iter(vec![("Filter", Object::Array(names))]);
        if let Some(params) = params {
            dict.set("DecodeParms", Object::Dictionary(params));
        }
        Stream::new(dict, content)
    }

    #[test]
    fn measure_counts_what_lopdf_decodes_at_every_step() {
        let text: Vec<u8> = (0..300_000u32).map(|i| b"abcz \x00"[(i % 6) as usize]).collect();
        let mut zeros = vec![0u8; 10];
        zeros.extend_from_slice(b"tail");
        let late = Dictionary::from_iter(vec![("EarlyChange", Object::Integer(0))]);
        let late_code = weezl::encode::Encoder::new(weezl::BitOrder::Msb, 8).encode(&text).unwrap();
        let cases = [
            filtered(&["LZWDecode"], None, lzw(&text)),
            filtered(&["LZWDecode"], Some(late), late_code),
            filtered(&["ASCII85Decode"], None, ascii85(&zeros)),
            filtered(&["ASCII85Decode"], None, ascii85(b"abcdefg")),
            filtered(&["ASCII85Decode", "LZWDecode"], None, ascii85(&lzw(&text))),
            filtered(
                &["ASCII85Decode", "FlateDecode"],
                None,
                ascii85(&zlib_run("x", b' ', 1_000_000, "y")),
            ),
            filtered(
                &["FlateDecode", "ASCII85Decode"],
                None,
                zlib_run(&String::from_utf8(ascii85(&text)).unwrap(), b' ', 0, ""),
            ),
        ];
        for (i, stream) in cases.iter().enumerate() {
            let measured = measure(stream, &mut PdfBudget::default(), Scope::Load).unwrap();
            assert_eq!(measured, lopdf_steps(stream), "case {i}");
        }
    }

    #[test]
    fn a_wrong_empty_password_gives_the_error_of_today() {
        let bytes = passworded_pdf();
        let today = pdf_extract::extract_text_from_mem_by_pages(&bytes).unwrap_err();

        let Err(e) = extract(&bytes) else { panic!("an empty password must not open it") };

        assert_eq!(e.message, format!("pdf: {today}"));
    }

    #[test]
    fn an_encrypt_key_left_in_the_copy_fails_closed() {
        let bytes = secret_pdf("/Encrypt");
        let mut span = xref_guard(&bytes, &mut PdfBudget::default()).unwrap().unwrap();
        assert!(span.encrypted() && span.names.len() == 1);
        span.names.clear();

        let e = unlock_encrypted(&bytes, &span).unwrap_err();

        assert_eq!(e.message, "pdf: the encryption dictionary could not be isolated");
    }

    #[test]
    fn an_unreadable_main_trailer_naming_encrypt_fails_closed() {
        let mut bytes = secret_pdf("/Encrypt");
        // The last section now starts with a word lopdf cannot read as an object.
        let at = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
        let xref: usize = std::str::from_utf8(&bytes[at + 10..])
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        bytes[xref] = b'x';

        let e = extract(&bytes).map(|_| ()).unwrap_err();

        assert_eq!(e.message, "pdf: the encryption dictionary could not be isolated");
    }

    /// What the reader of today answers for `bytes`, as `extract` words it.
    fn today(bytes: &[u8]) -> std::result::Result<String, String> {
        pdf_extract::extract_text_from_mem_by_pages(bytes)
            .map(|pages| pages.concat())
            .map_err(|e| format!("pdf: {e}"))
    }

    /// The number of pages of `bytes` today and now, when both read it.
    fn page_counts(bytes: &[u8]) -> Option<(usize, usize)> {
        let today = pdf_extract::extract_text_from_mem_by_pages(bytes).ok()?.len();
        Some((today, extract(bytes).ok()?.pages.len()))
    }

    #[test]
    fn an_object_aes_cannot_decrypt_is_skipped_as_today() {
        let bytes = aes_secret_pdf(true);
        let clean = extract(&aes_secret_pdf(false)).expect("answered").body;
        assert!(today(&bytes).unwrap().contains("Quarterly results"), "today extracts it");

        let text = extract(&bytes).expect("answered").body;

        assert!(text.contains("Quarterly results") && text.contains("Outlook"), "{text}");
        assert_eq!(text, clean);
    }

    /// The main trailer's `/Encrypt N 0 R` of `bytes` and object N's dictionary.
    fn encrypt_reference(bytes: &[u8]) -> (std::ops::Range<usize>, Vec<u8>) {
        let at = bytes.windows(9).rposition(|w| w == b"/Encrypt ").unwrap();
        let tail = String::from_utf8_lossy(&bytes[at..at + 40]);
        let words: Vec<&str> = tail.split_whitespace().take(4).collect();
        let reference = words[..4].join(" ");
        let header = format!("\n{} {} obj\n", words[1], words[2]);
        let start = bytes.windows(header.len()).position(|w| w == header.as_bytes()).unwrap();
        let body = &bytes[start + header.len()..];
        let end = body.windows(7).position(|w| w == b"\nendobj").unwrap();
        (at..at + reference.len(), body[..end].to_vec())
    }

    #[test]
    fn an_inline_encryption_dictionary_answers_as_today() {
        let mut bytes = secret_pdf("/Encrypt");
        let (reference, dict) = encrypt_reference(&bytes);
        let mut inline = b"/Encrypt ".to_vec();
        inline.extend_from_slice(&dict);
        // The main trailer is the last object: its offset does not move.
        bytes.splice(reference, inline);
        let today = today(&bytes);
        // lopdf cannot authenticate an inline dictionary: its loader keeps no object.
        assert_eq!(today, Ok(String::new()));

        let now = extract(&bytes).map(|e| e.body).map_err(|e| e.message);

        assert_eq!(now, today);
        let (today, now) = page_counts(&bytes).unwrap();
        assert_eq!(now, today);
    }

    #[test]
    fn an_unreadable_start_naming_encrypt_fails_closed() {
        let mut bytes = secret_pdf("/Encrypt");
        // `startxref` followed by a tab: no cross reference can be located.
        let at = bytes.windows(10).rposition(|w| w == b"startxref\n").unwrap();
        bytes[at + 9] = b'\t';

        let e = extract(&bytes).map(|_| ()).unwrap_err();

        assert_eq!(e.message, "pdf: the encryption dictionary could not be isolated");
    }

    /// The secret document whose main trailer reads `key` where `/Encrypt` was:
    /// its guard span, its unlocked copy and its text.
    fn unlocked_with(key: &str) -> (TrailerSpan, Vec<u8>, String) {
        let bytes = secret_pdf(key);
        let span = xref_guard(&bytes, &mut PdfBudget::default()).unwrap().unwrap();
        let copy = unlock_encrypted(&bytes, &span).unwrap();
        (span, copy, extract(&bytes).expect("answered").body)
    }

    #[test]
    fn an_encrypt_name_in_a_trailer_string_is_kept() {
        let clean = unlocked_with("/Encrypt").2;
        let (span, copy, text) = unlocked_with("/Note (/Encrypt) /Encrypt");

        assert_eq!(span.names.len(), 1);
        assert!(copy.windows(10).any(|w| w == b"(/Encrypt)"), "the string was renamed");
        assert_eq!(text, clean);
    }

    #[test]
    fn an_encrypt_name_in_a_trailer_comment_is_kept() {
        let clean = unlocked_with("/Encrypt").2;
        let (span, copy, text) = unlocked_with("% /Encrypt\n/Encrypt");

        assert_eq!(span.names.len(), 1);
        assert!(copy.windows(10).any(|w| w == b"% /Encrypt"), "the comment was renamed");
        assert_eq!(text, clean);
    }

    #[test]
    fn an_encrypt_name_nested_in_the_trailer_is_kept() {
        let clean = unlocked_with("/Encrypt").2;
        let (span, copy, text) = unlocked_with("/Extra << /Encrypt 1 0 R >> /Encrypt");

        assert_eq!(span.names.len(), 1);
        assert!(copy.windows(17).any(|w| w == b"<< /Encrypt 1 0 R"), "the nested name was renamed");
        assert_eq!(text, clean);
    }

    #[test]
    fn an_encrypt_key_only_in_a_previous_trailer_answers_as_today() {
        // The classic section lopdf wrote names `Encrypt`; the main one does not.
        let bytes = secret_pdf("/Unused");
        assert!(!xref_guard(&bytes, &mut PdfBudget::default()).unwrap().unwrap().encrypted());

        let now = extract(&bytes).map(|e| e.body).map_err(|e| e.message);

        assert_eq!(now, today(&bytes));
        let (today, now) = page_counts(&bytes).unwrap();
        assert_eq!(now, today);
    }

    #[test]
    fn a_hybrid_cross_reference_stream_is_measured() {
        let mut doc = page_pdf(plain(HELLO), "", vec![]);
        let classic = doc.windows(6).rposition(|w| w == b"\nxref\n").unwrap() + 1;
        let stored = zlib_run("", b' ', PDF_BOMB, "");
        let stream = doc.len();
        doc.extend_from_slice(
            format!(
                "9 0 obj\n<< /Type /XRef /Size 10 /W [1 4 2] /Filter /FlateDecode /Length {} >>\nstream\n",
                stored.len()
            )
            .as_bytes(),
        );
        doc.extend_from_slice(&stored);
        doc.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = doc.len();
        doc.extend_from_slice(
            format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 10 /Root 1 0 R /Prev {classic} /XRefStm {stream} >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );

        let e = extract(&small(doc)).map(|_| ()).unwrap_err();

        assert_eq!(e.message, "a PDF stream decodes to more than 104857600 bytes");
    }

    #[test]
    fn a_cross_reference_stream_with_a_predictor_between_two_steps_is_refused() {
        let mut doc = page_pdf(plain(HELLO), "", vec![]);
        // The classic cross reference gives way to a stream whose two
        // inflations have a predictor between them.
        let classic = doc.windows(5).rposition(|w| w == b"xref\n").unwrap();
        doc.truncate(classic);
        let dict = "<< /Type /XRef /Size 6 /W [1 4 2] /Filter [/FlateDecode /FlateDecode] /DecodeParms << /Predictor 12 /Columns 7 >> /Length 0 >>";
        doc.extend_from_slice(format!("9 0 obj\n{dict}\nstream\n\nendstream\nendobj\n").as_bytes());
        doc.extend_from_slice(format!("startxref\n{classic}\n%%EOF\n").as_bytes());

        let e = extract(&doc).map(|_| ()).unwrap_err();

        assert_eq!(e.message, "pdf: the cross reference stream could not be measured");
    }

    #[test]
    fn pdf_one_profile_kept_in_300_saved_states_holds_nothing() {
        let bytes = states_pdf(300);
        let mut figures = Figures(PdfBudget::default());
        let mut doc = load(&bytes, &mut figures.0).expect("loaded");
        let profile = ObjectId::from((7, 0));
        let stored = doc.get_object(profile).and_then(Object::as_stream).expect("stream").clone();

        let stand_ins = install_stand_ins(&mut doc);

        let stand_in = *stand_ins.get(&profile).expect("object 7 has a stand in");
        let stand = doc.get_object(stand_in).and_then(Object::as_stream).expect("stream");
        assert!(stand.content.is_empty());
        assert!(stand.dict.get(b"Filter").is_err());
        // `Stream::new` writes the length of the empty content.
        assert_eq!(stand.dict.get(b"Length").and_then(Object::as_i64).ok(), Some(0));
        // The colour space array now points at the stand in, the only reference left.
        let mut seen = Vec::new();
        for object in doc.objects.values_mut() {
            visit_spaces(object, &mut |id| seen.push(*id));
        }
        assert_eq!(seen, vec![stand_in]);
        let original = doc.get_object(profile).and_then(Object::as_stream).expect("stream");
        assert_eq!(original.content, stored.content);
        assert!(original.dict.get(b"Filter").is_ok());

        extract(&bytes).expect("extracted");
        let held = crate::docs::extract::LAST_PDF_HELD.with(std::cell::Cell::take);
        assert_eq!(held.expect("recorded").decoded, 1_000_000);
    }

    /// Walk page 1 of a document selecting a `Separation` whose tint transform
    /// is object 7, `FunctionType` `kind`, holding `body`; the ids marked invalid.
    fn invalid_tints(kind: u8, body: &[u8]) -> (usize, HashMap<ObjectId, ObjectId>) {
        let function = PdfObj::Stream(
            format!("<< /FunctionType {kind} /Domain [0 1] /Range [0 1] /Length {} >>", body.len()),
            body.to_vec(),
        );
        let bytes = page_pdf(
            plain("/CS0 cs"),
            "/ColorSpace << /CS0 [/Separation /Spot /DeviceGray 7 0 R] >>",
            vec![(7, function)],
        );
        let mut budget = PdfBudget::default();
        let mut doc = load(&bytes, &mut budget).expect("loaded");
        let stand_ins = install_stand_ins(&mut doc);
        let originals = stand_ins.iter().map(|(o, s)| (*s, *o)).collect();
        defuse(&mut doc);
        let page = doc.get_pages().into_values().next().expect("a page");
        let walked = walk(&doc, page, &originals, &mut budget).expect("walked");
        (walked.invalid.len(), stand_ins)
    }

    #[test]
    fn a_type_4_tint_transform_that_is_not_utf_8_is_flagged_for_its_stand_in() {
        let (flagged, stand_ins) = invalid_tints(4, b"{ \xFF\xFE }");
        assert_eq!((flagged, stand_ins.len()), (1, 1));
        assert_eq!(invalid_tints(4, b"{ 0.5 mul }").0, 0);
        assert_eq!(invalid_tints(0, b"\xFF\xFE").0, 0);
    }

    #[test]
    fn map_nesting_counts_lists_dictionaries_and_strings() {
        assert_eq!(map_nesting(b"[[<< /K (()) >>]]"), 5);
        assert_eq!(map_nesting(&b"[".repeat(64)), 64);
        assert_eq!(map_nesting(&b"[".repeat(65)), 65);
        assert_eq!(map_nesting(b"((("), 3);
    }

    #[test]
    fn map_nesting_skips_escapes_hex_strings_and_comments() {
        assert_eq!(map_nesting(b"(\\( \\( [)"), 1);
        assert_eq!(map_nesting(b"<0A0B> [ ]"), 1);
        assert_eq!(map_nesting(b"% [[[[\n[ ]"), 1);
        assert_eq!(map_nesting(b"<< >>"), 1);
    }
}
