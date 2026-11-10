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

use crate::errors::{Result, ToolError};
use pdf_extract::content::{Content, Operation};
use pdf_extract::xref::XrefEntry;
use pdf_extract::{Dictionary, Document, LoadOptions, Object, ObjectId, ObjectStream, Stream};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// Largest decoded size of one PDF stream (SPEC-0089 FR-NEW-002).
pub(crate) const STREAM_LIMIT: u64 = 104_857_600;

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

/// What the text pass gives back: page texts (trimmed, in order, empty pages
/// included), the body assembled as before, and the title.
pub(crate) struct PdfText {
    pub(crate) pages: Vec<String>,
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
    content: u64,
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
}

/// The text of `data`, every stream decoded under [`STREAM_LIMIT`]. Order of
/// the refusals (DEC-013): the load and its object streams in increasing number,
/// then the pages in order, then the title.
pub(crate) fn extract(data: &[u8]) -> Result<PdfText> {
    let mut figures = Figures(PdfBudget::default());
    let budget = &mut figures.0;
    let mut doc = load(data, budget)?;
    defuse(&mut doc);

    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let mut texts = Vec::new();
    for (index, page_id) in pages.into_iter().enumerate() {
        let walked = walk(&doc, page_id, budget)?;
        budget.hold(budget.page);
        let n = index as u32 + 1;
        let mut text = String::new();
        let page = {
            let armed = Armed::new(&mut doc, walked);
            pdf_extract::output_doc_page(
                armed.doc,
                &mut pdf_extract::PlainTextOutput::new(&mut text),
                n,
            )
        };
        // The reader's own loop stops at the first page error, the error itself
        // dropped (`extract_text_from_mem_by_pages`).
        if page.is_err() {
            break;
        }
        texts.push(text);
    }

    let mut body = String::new();
    for (i, text) in texts.iter().enumerate() {
        if !text.trim().is_empty() {
            body.push_str(&format!("\n\n---\n*[Page {}]*\n\n{}", i + 1, text.trim()));
        }
    }
    let body = body.trim().to_string();
    budget.content = body.len() as u64;
    let title = title(&doc);
    let pages = texts.iter().map(|t| t.trim().to_string()).collect();
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
/// time, in increasing object number, each measured before it is decoded.
fn load(data: &[u8], budget: &mut PdfBudget) -> Result<Document> {
    let options = LoadOptions { filter: Some(defer_object_streams), ..Default::default() };
    let mut doc = Document::load_mem_with_options(data, options).map_err(pdf_error)?;
    // An encrypted document goes through lopdf's encrypted loader, which ignores
    // the filter; its decryption with the empty password is the reader's
    // `maybe_decrypt` (SPEC-0089 DEC-027, US-0213 bounds it).
    if doc.is_encrypted() {
        doc.decrypt("").map_err(pdf_error)?;
    }

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
    for id in deferred {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else { continue };
        let mut plain = plain_object_stream(stream, budget)?;
        match ObjectStream::new(&mut plain) {
            Ok(parsed) => {
                drop(plain);
                for (member, object) in parsed.objects {
                    // As lopdf: a member the cross reference places in another
                    // container is a stale copy, and nothing loaded is replaced.
                    if containers.get(&member.0).is_none_or(|c| *c == id.0) {
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

/// Whether [`measure`] counts exactly what the reader would produce. A predictor
/// on a chain of several inflations reshapes the bytes between two steps, which
/// the streaming measurer does not reproduce: such a stream is left stored.
/// A chain longer than [`MAX_FILTERS`] is left stored too.
fn measurable(stream: &Stream) -> bool {
    let Some(names) = filters(stream) else { return true };
    if names.len() > MAX_FILTERS {
        return false;
    }
    let flates = names.iter().take_while(|n| *n == b"FlateDecode").count();
    flates <= 1 || predictor_rows(stream).is_none()
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

/// A piece of a step's input: the stored bytes for the first step, an inflated
/// chunk of the step before for the others.
struct Frame<'s> {
    step: usize,
    data: std::borrow::Cow<'s, [u8]>,
    zlib_at: usize,
    raw_at: usize,
    /// The last call filled the chunk, so the decoder may hold more output.
    zlib_full: bool,
    raw_full: bool,
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

/// The decoded size of `stream` as the reader would produce it, inflating one
/// [`CHUNK`] at a time and keeping none of it (DEC-003): the sum of what each
/// step outputs, plus a predictor's row buffers and output. A chain whose first
/// filter is unknown counts 0 (the reader copies the stored bytes); an unknown
/// filter later ends the chain, the steps before it counted. Until US-0213 a
/// chain holding `LZWDecode` or `ASCII85Decode` counts 0 (DEC-027).
///
/// Steps are fed depth first from an explicit stack, so the held data is one
/// chunk per step and no recursion. O(decoded bytes) time.
fn measure(stream: &Stream, budget: &mut PdfBudget, scope: Scope) -> Result<u64> {
    let Some(names) = filters(stream) else { return Ok(0) };
    if names.len() > MAX_FILTERS {
        return Ok(0);
    }
    if names.iter().any(|n| *n == b"LZWDecode" || *n == b"ASCII85Decode") {
        return Ok(0);
    }
    let steps = names.iter().take_while(|n| *n == b"FlateDecode").count();
    let rows = predictor_rows(stream);
    let mut total = if steps > 0 { rows.unwrap_or(0) } else { 0 };
    if total > STREAM_LIMIT {
        return Err(over_stream_limit());
    }
    let last = steps.saturating_sub(1);
    let mut inflates: Vec<Inflate> = (0..steps)
        .map(|_| Inflate { started: false, zlib: None, raw: None, zlib_out: 0, skip: 2 })
        .collect();
    let mut out = vec![0u8; CHUNK];
    let mut stack = Vec::new();
    if steps > 0 && !stream.content.is_empty() {
        stack.push(Frame {
            step: 0,
            data: std::borrow::Cow::Borrowed(stream.content.as_slice()),
            zlib_at: 0,
            raw_at: 0,
            zlib_full: false,
            raw_full: false,
        });
    }
    while let Some(frame) = stack.last_mut() {
        let step = &mut inflates[frame.step];
        if !step.started {
            step.started = true;
            step.zlib = Some(new_decoder(true));
            step.raw = Some(new_decoder(false));
        }
        let len = frame.data.len();
        let produced = if step.zlib.is_some() && (frame.zlib_at < len || frame.zlib_full) {
            let (used, made) = inflate_some(&mut step.zlib, &frame.data[frame.zlib_at..], &mut out);
            frame.zlib_at += used;
            frame.zlib_full = made == CHUNK;
            step.zlib_out += made as u64;
            made
        } else if step.zlib_out == 0 && step.raw.is_some() && (frame.raw_at < len || frame.raw_full)
        {
            let skip = step.skip.min(len - frame.raw_at);
            step.skip -= skip;
            frame.raw_at += skip;
            let (used, made) = inflate_some(&mut step.raw, &frame.data[frame.raw_at..], &mut out);
            frame.raw_at += used;
            frame.raw_full = made == CHUNK;
            made
        } else {
            stack.pop();
            continue;
        };
        if produced == 0 {
            continue;
        }
        let at = frame.step;
        // The predictor's output, at most its input, is counted as its input.
        let counted = if at == last && rows.is_some() { 2 * produced } else { produced };
        total = total.saturating_add(counted as u64);
        // The bytes really decoded so far, so a refusal records what it cost.
        budget.hold(total);
        // Checked after every chunk, so decoding stops at most one chunk past
        // the limit; equal is accepted.
        if total > STREAM_LIMIT {
            return Err(over_stream_limit());
        }
        if at < last {
            stack.push(Frame {
                step: at + 1,
                data: std::borrow::Cow::Owned(out[..produced].to_vec()),
                zlib_at: 0,
                raw_at: 0,
                zlib_full: false,
                raw_full: false,
            });
        }
    }
    if let Scope::Page = scope {
        budget.page = budget.page.saturating_add(total);
    }
    Ok(total)
}

/// What the walk of one page found: the streams to arm.
type Walked = BTreeSet<ObjectId>;

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

/// A page's resources, inherited through `Parent` as the reader looks them up.
fn inherited_resources<'d>(doc: &'d Document, page: &'d Dictionary) -> Option<&'d Dictionary> {
    let mut node = page;
    let mut seen = HashSet::new();
    loop {
        if let Some(found) = dict_at(doc, node, b"Resources") {
            return Some(found);
        }
        let parent = node.get(b"Parent").and_then(Object::as_reference).ok()?;
        if !seen.insert(parent) {
            return None;
        }
        node = doc.get_dictionary(parent).ok()?;
    }
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
    armed: Walked,
    sizes: HashMap<ObjectId, u64>,
}

impl PageWalk<'_> {
    /// Count one decode of stream `id` by the reader: measured the first time on
    /// this page, its size reused for the repeats (DEC-008).
    fn decode(&mut self, id: ObjectId, budget: &mut PdfBudget) -> Result<u64> {
        if let Some(size) = self.sizes.get(&id) {
            budget.page = budget.page.saturating_add(*size);
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

/// A content stream or form being walked.
struct Drawing<'d> {
    ops: Rc<Vec<Operation>>,
    at: usize,
    resources: &'d Dictionary,
    form: Option<ObjectId>,
    /// Decoded size of the form, 0 for the page content.
    size: u64,
}

/// Walk page `page_id` the way pdf-extract will run it (DEC-007), measuring
/// every stream it may decode in drawing order; returns the streams to arm.
/// Where the reader would panic the walk stops without refusing, and the
/// reader then panics as today. O(operations drawn) time; the parsed
/// operations held are the page's and those of the forms on the drawing stack:
/// a form's are dropped when it is popped, and parsed again if drawn again, as
/// the reader does.
fn walk(doc: &Document, page_id: ObjectId, budget: &mut PdfBudget) -> Result<Walked> {
    budget.page = 0;
    let mut page = PageWalk { doc, armed: BTreeSet::new(), sizes: HashMap::new() };
    let Ok(page_dict) = doc.get_dictionary(page_id) else { return Ok(page.armed) };
    let empty = Dictionary::new();
    let resources = inherited_resources(doc, page_dict).unwrap_or(&empty);

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
    let Ok(parsed) = Content::decode(&content) else { return Ok(page.armed) };
    drop(content);

    let mut fonts: HashSet<Vec<u8>> = HashSet::new();
    // Decoded size of the forms on the stack, whose operations are held.
    let mut retained = 0u64;
    let mut stack =
        vec![Drawing { ops: Rc::new(parsed.operations), at: 0, resources, form: None, size: 0 }];
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
                for id in font_streams(doc, font) {
                    page.decode(id, budget)?;
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
                    page.decode(id, budget)?;
                }
            }
            "Do" => {
                let Some(name) = name() else { break };
                let Some(id) =
                    dict_at(doc, resources, b"XObject").and_then(|x| stream_at(doc, x, name))
                else {
                    break;
                };
                let size = page.decode(id, budget)?;
                if stack.iter().any(|d| d.form == Some(id)) {
                    continue;
                }
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
                stack.push(Drawing { ops, at: 0, resources: inner, form: Some(id), size });
            }
            _ => {}
        }
    }
    Ok(page.armed)
}

/// The streams `make_font` decodes for `font`, by its `Subtype` (`lib.rs:322-332`).
fn font_streams(doc: &Document, font: &Dictionary) -> Vec<ObjectId> {
    let subtype =
        font.get(b"Subtype").ok().and_then(|o| resolve(doc, o)).and_then(|o| o.1.as_name().ok());
    let mut ids = Vec::new();
    match subtype {
        Some(b"Type0") => ids.extend(stream_at(doc, font, b"Encoding")),
        // The reader panics on a font without `Subtype`: nothing is decoded.
        None => return ids,
        Some(b"Type3") => {}
        Some(other) => {
            if let Some(descriptor) = dict_at(doc, font, b"FontDescriptor") {
                match other {
                    b"Type1" => ids.extend(stream_at(doc, descriptor, b"FontFile")),
                    b"TrueType" => ids.extend(stream_at(doc, descriptor, b"FontFile2")),
                    _ => {}
                }
                ids.extend(stream_at(doc, descriptor, b"FontFile3"));
            }
        }
    }
    ids.extend(stream_at(doc, font, b"ToUnicode"));
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
}
