# SPEC-0089: Bound PDF stream inflation and the document converter output, design

> Spec: sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding
> Designed on: 2026-11-10
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`); US-0212 and US-0213 at tier 1

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| PDF parsing and text | `pdf-extract` (re exports all of `lopdf` with `pub use lopdf::*`, `pdf-extract-0.12.1/src/lib.rs:6`), public pieces only: `Document::load_mem_with_options` with `LoadOptions::filter`, `ObjectStream::new`, `Document::get_page_contents`, `Document::decrypt`, `output_doc_page`, `PlainTextOutput`, `OutputDev`, `content::Content::decode`, `Reader` and `xref::decode_xref_stream` | `pdf-extract = "0.12"` (`Cargo.toml:143`), locked 0.12.1; lopdf locked 0.42.0, features `wasm_js` only (no `rayon`: `cargo tree -e features -i lopdf`) | DEC-001, DEC-004, DEC-006 |
| Flate measurement | `flate2::Decompress` (zlib, then raw deflate as lopdf falls back) | `flate2 = "1"` (`Cargo.toml:82`) | DEC-003 |
| LZW measurement | `weezl::decode::Decoder` (the decoder lopdf itself uses, `lopdf-0.42.0/Cargo.toml` `weezl = "0.1"`) | locked 0.1.12 (`Cargo.lock:6721`), becomes a direct dependency of `mcp-fs-core` in US-0213 | DEC-021 |
| capped reads of the converter answer | `tokio::io::AsyncReadExt::read` in 65 536 byte chunks; `reqwest::Response::chunk` | tokio 1, reqwest 0.12.28 without its `charset` feature (`text()` is `from_utf8_lossy`, `reqwest-0.12.28/src/async_impl/response.rs:169-174`) | DEC-015, DEC-016 |
| content limit message and number | `CONTENT_LIMIT` 268 435 456 (`crates/core/src/docs/extract.rs:601`) | in tree (SPEC-0088) | DEC-010 |
| test fixtures | hand written PDF bytes; `archive_testkit::deflate_run` cached segments wrapped in zlib framing; `lopdf::Document::encrypt` for the encrypted fixture; stand in converters as shell scripts ending in `exec perl`; axum streaming body | in tree | DEC-019 |

Dependency statement: no new crate. One crate already in `Cargo.lock` becomes a direct dependency of
`mcp-fs-core`: `weezl` 0.1.12, the LZW decoder lopdf already pulls in (DEC-021). Every other crate used is
already a dependency of `mcp-fs-core` (`pdf-extract`, `flate2`, `tokio`, `reqwest`, `futures`, `axum`,
`tempfile`). No vendored or patched PDF library, no git dependency, no `unsafe`.

Skills loaded: `rust` (edition 2024, no `unwrap` or `expect` on fallible production paths, typed
`ToolError`, clippy `--all-targets --all-features -D warnings`, `cargo fmt --check`, functional e2e
through the real entry points, no `unsafe`).

## 2. Approaches

The key question (spec 14.3): neither PDF library lets the caller cap a decode. Evidence:
`Stream::decompressed_content` inflates with `read_to_end` into an unbounded `Vec`
(`lopdf-0.42.0/src/object.rs:794-815`, `:849-872`), `LoadOptions` holds only a password, a filter and
strictness (`lopdf-0.42.0/src/load_options.rs`), and object streams are inflated inside the load
(`src/reader.rs:965`, `:889`, through `src/object_stream.rs:42`). What the libraries do offer, read from
their sources:

1. The load filter runs on each object BEFORE an object stream is inflated, and receives it `&mut`
   (`src/reader.rs:959-965`: `filter_func(object_id, &mut object)?;` then `has_type(b"ObjStm")` then
   `ObjectStream::new`). It is a plain `fn` pointer (`load_options.rs` `FilterFunc`), so it can rename the
   type of an object stream (no state needed) and lopdf then keeps it as an ordinary compressed stream.
   It is ignored on an encrypted document (`src/reader.rs:821`, `_filter_func`), whose object streams are
   inflated at `:889` with no type check.
2. `ObjectStream::new` is public and inflates only when the stream still has a `Filter`
   (`src/object_stream.rs:42`, `stream.decompress()` is a no op without one).
3. A stream without a `Filter` key is never inflated: `get_contents` clones the stored bytes
   (`pdf-extract-0.12.1/src/lib.rs:1255-1261`), `decompressed_content` returns a clone
   (`object.rs:794-799`), `get_page_content` too (`document.rs:589-600`). The document is ours between
   pages, so the `Filter` key of every stream can be moved aside and put back for the streams one page
   may decode.
4. `output_doc_page(doc, output, n)` (`lib.rs:2389-2399`) takes the loaded document and any
   `OutputDev`; `PlainTextOutput<&mut dyn std::io::Write>` (`lib.rs:2117-2122`, `:2131-2190`) writes
   through our own writer, which can refuse a write.
5. Cross reference streams are inflated while the cross reference is read (`src/parser/mod.rs:547`,
   `src/parser_aux.rs:487-490`), before any caller code; one with an indirect `Length` is not
   (`parser/mod.rs:344-359`, the reference cannot resolve yet, so the content is empty).

### Chosen: measure every stream under a cap, then let the reader decode only what was measured
A new module `crates/core/src/docs/pdf_bound.rs` drives the text pass itself. It (a) measures, before the
load, every cross reference stream lopdf will inflate (DEC-005); (b) loads with a filter that defers every
object stream (DEC-004), and for an encrypted document first renames the trailer's `/Encrypt` key in a copy
of the bytes so the filter applies, then decrypts with `Document::decrypt` (DEC-005); (c) counts the
objects, then measures, decodes and parses each deferred object stream in increasing object number
(stream limit, load total, object limit); (d) moves the `Filter` key of every stream aside ("defuse"); (e)
for each page, walks the page the way pdf-extract will (content streams, fonts once per name, colour
spaces per selection, forms and images per draw, forms inside forms) and measures every stream that walk
reaches with a streaming counter that holds one 65 536 byte chunk (stream limit, page total), points colour
space profiles at empty stand ins (DEC-009), puts the `Filter` key back on exactly the streams measured
("arm"), runs `output_doc_page` with a writer that enforces the content limit, then defuses again; (f)
reads the title from the document already loaded. Every inflation lopdf performs during the text pass is
of a stream this module measured within the limits; a stream the walk did not reach is handed over as
stored bytes, the way an undecodable stream is today.
Pros: all 12 PDF occurrences of the class sweep are bounded, plus the cross reference stream the sweep
missed; safe Rust, no new crate (one lock entry promoted); the reader is unchanged for every document
within the limits; a refusal comes before the reader decodes anything of the page or the load.
Cons and risks: each stream reached is inflated twice (measured, then decoded by the reader), content and
forms three times (measured, decoded for the walk, decoded by the reader); the walk must mirror
pdf-extract's decode sites, and a site it misses gets stored bytes (text differs for that document, never
an unbounded decode); the encrypted path mirrors `load_encrypted_document` with documented differences
(DEC-005). Affected scope: `docs/pdf_bound.rs` (new), `docs/extract.rs` (`pdf`, `pdf_title`, hook).
Best suited when, as here, the library offers no decode hook but owns no state the caller cannot edit.

### Rejected: a pre scan of the raw bytes alone, then the unmodified reader
Parse the file ourselves, measure every stream, then call `extract_text_from_mem_by_pages`. Rejected:
repeats are invisible to a scan of the bytes (a form drawn 300 times, a profile selected 300 times,
occurrences 7 and 8), the reader would still inflate object streams of an encrypted file and anything the
scan misjudged, and a refusal could not respect the reading order of FR-NEW-007 (a page 2 bomb found by the
scan would win over page 1's content limit, E2E-028) (DEC-001).

### Rejected: materialize decoded bytes ourselves and hand the reader plain streams
Decode under a cap into the document and remove every `Filter`. Rejected: the reader then copies each plain
stream again (`get_contents` clone, `get_page_content` concatenation), so a page holds our copy plus up to
two of its own: three times the page total, past FR-NEW-006's 536 936 448. Measuring and letting the reader
decode keeps the reader's own two copies at most (DEC-012).

### Rejected: hooks only (`OutputDev`, `LoadOptions::filter`)
The `OutputDev` sees text, not decodes; the filter sees objects at load only, and not on encrypted files.
Both are used, neither suffices (DEC-001).

### Rejected: a bounded child process or thread
A memory cap needs `setrlimit` (no safe API in the tree: `libc` is a dependency of the agent crate only and
`unsafe_code` is forbidden; `RLIMIT_AS` is not enforced on macOS), a thread cannot be memory capped, and an
out of memory kill carries none of the five refusal messages nor their order (DEC-001).

### Rejected: a patched, vendored or other PDF library
Category 2 of ASK ONLY FOR; not needed, since the chosen design meets every requirement in tree (DEC-001).

### Chosen for the converter: capped streaming reads, killing the child on the cap
`docs/service.rs` reads standard output in text mode through a capped reader (stop one chunk past
`max_input_bytes`), keeps a tail ring of 8 192 bytes of the diagnostic output, discards standard output as
it comes in bundle mode, reads the web body chunk by chunk under the same cap, and on the cap kills and
reaps the child before its working folder is dropped (DEC-015, DEC-016). Rejected: reading whole then
checking (the spec's red reasons).

## 3. Architecture

```
fs.extract_text (mcp/server.rs:2402)          POST /api/fs/{mount}/extract-text (api/dataplane.rs:171)
        \  membership, path normalized                   /
         +---- core::fs_ops::extract_document -----------+
                 prepare_extraction (docs/extract.rs:318): not a file; up to date companion -> cached
                 Extractor::extract -> ".pdf" -> pdf(data)                     (extract.rs:510)
                   catch_unwind(pdf_bound::extract(data))                      (one closure, as today)
                     PdfBudget::new()
                     load:  xref_guard(data)               cross reference streams measured   [US-0213]
                            unlock_encrypted(data)         /Encrypt renamed in a copy          [US-0213]
                            load_mem_with_options(filter = defer_object_streams)
                            decrypt mirror                                                     [US-0213]
                            count objects; object streams in increasing number:
                              measure (stream limit, load total) -> decode -> count pairs (object limit)
                              -> ObjectStream::new -> insert as lopdf would -> free the container bytes
                     defuse every stream (Filter -> McpFsFilter)
                     neutralize colour space profiles (empty stand ins)                       [US-0215]
                     for n in 1..: walk(page n): measure every stream the reader will decode
                                   (stream limit, page total), arm them,
                                   output_doc_page(doc, PlainTextOutput(ContentSink), n)  content limit [US-0216]
                                   disarm; trim the page into the content in place
                     title from doc.trailer /Info (no second load)
                     cfg(test): LAST_PDF_HELD = { decoded, content }
                   cfg(test): PDF_HELD[path] = LAST_PDF_HELD.take()
                 refusal -> "could not extract <path>: <reason>"        (extract.rs:373-381, unchanged)

fs.documentize / fs.write_bytes(flag) / POST documentize, upload, write-bytes
  -> core::fs_ops::{documentize, write_bytes_documented} -> write_companion -> DocService::convert
       CliDocService: stdout  text mode -> read_capped_async(cap)  -> Over => kill + reap, refuse
                              bundle mode -> drain (discard)
                      stderr  -> tail ring 8 192
                      dir     -> TempDir dropped after the child is reaped
       ApiDocService: status first; body -> read_body_capped(cap); failure -> head of what was read
       cfg(test): CONVERTER_HELD.push({ answer, diagnostic, bundle_stdout })
```

- Bounded context "document text extraction" owns `pdf_bound.rs` (the budget, the measurer, the load, the
  walk, the content sink) and the five PDF refusals; `extract.rs::pdf` becomes a thin adapter that keeps
  today's result shape (`pages`, `title`, note). Project storage is untouched: a refusal returns before
  `land_conversion` (`crates/core/src/core/fs_ops.rs`, the success path only charges and audits).
- Bounded context "document conversion" stays in `docs/service.rs`; `core::fs_ops` is unchanged (the
  refusal travels the existing error paths: documentize answers it, the upload and binary write put it in
  the documentation report, `fs_ops.rs:659-680`).

## 4. Data model

Nothing persisted. In memory, per call:

- `PdfBudget { load: u64, objects: u64, page: u64, page_no: u32, content: u64, held: u64, live: u64 }`:
  object stream bytes decoded so far (load total), objects counted, decode count of the current page,
  extracted content bytes, and the hook figure (largest decoded data held at once) with the live figure.
- `Measured { size: u64 }` or the refusal: the decoded size of one stream as the reader would produce it
  (DEC-003).
- Per page (dropped at the page end): `armed: BTreeSet<ObjectId>`, `sizes: HashMap<ObjectId, u64>`
  (measured once per page), `forms: HashMap<ObjectId, Rc<Vec<Operation>>>` (parsed form operations, for
  repeats), `fonts: HashSet<Vec<u8>>` (font names selected on this page).
- Per document: `stand_ins: HashMap<ObjectId, ObjectId>` (colour profile stream to its empty stand in,
  DEC-009).
- `#[cfg(test)] PdfHeld { decoded: u64, content: u64 }`, thread locals `LAST_PDF_HELD` and `PDF_HELD`
  (map keyed by path, as `OFFICE_HELD`); `#[cfg(test)] ConverterHeld { answer: u64, diagnostic: u64,
  bundle_stdout: u64 }`, thread local `CONVERTER_HELD: RefCell<Vec<ConverterHeld>>` (one record per
  conversion).

## 5. Contracts and interfaces

`crates/core/src/docs/pdf_bound.rs` (all `pub(crate)` or private; no public API change):

```rust
pub(crate) const STREAM_LIMIT: u64 = 104_857_600;   // FR-NEW-002
pub(crate) const LOAD_LIMIT: u64 = 268_435_456;     // FR-NEW-003
pub(crate) const PAGE_LIMIT: u64 = 268_435_456;     // FR-NEW-004
pub(crate) const OBJECT_LIMIT: u64 = 1_000_000;     // FR-NEW-001
const CHUNK: usize = 65_536;
/// The text pass: page texts (trimmed, in order, empty pages included) and the title.
pub(crate) struct PdfText { pub(crate) pages: Vec<String>, pub(crate) body: String, pub(crate) title: Option<String> }
pub(crate) fn extract(data: &[u8]) -> Result<PdfText>;
fn defer_object_streams(id: ObjectId, object: &mut Object) -> Option<(ObjectId, Object)>; // FilterFunc
fn measure(stream: &Stream, budget: &mut PdfBudget, scope: Scope) -> Result<u64>;      // Scope::Load | Page | Guard
```

`extract.rs::pdf(data)` keeps `fn(&[u8]) -> Result<ExtractResult>`; it builds the same `ExtractResult`
(`format` `pdf`, meta `pages`, meta `title`, the no text layer note) from `PdfText`.

Refusals, `ToolError::invalid_argument` (`ERR_INVALID_ARGUMENT`), message without prefix; the existing
wrapper adds `could not extract <path>: ` (`extract.rs:373-381`):

| Rule | Message after the prefix |
|---|---|
| FR-NEW-001 | `the PDF holds more than 1000000 objects` |
| FR-NEW-002 | `a PDF stream decodes to more than 104857600 bytes` |
| FR-NEW-003 | `the PDF object streams decode to more than 268435456 bytes together` |
| FR-NEW-004 | `page <N> decodes more than 268435456 bytes of PDF streams` |
| FR-NEW-005 | `the extracted content exceeds 268435456 bytes` |

Unchanged: a load failure is `pdf: PDF error: <lopdf error>` (today `OutputError::PdfError` Display,
`pdf-extract-0.12.1/src/lib.rs:43-52`, reached through `format!("pdf: {e}")` at `extract.rs:515`), a panic
is `pdf parser failed on this document` (`extract.rs:514`).

`crates/core/src/docs/service.rs`:

```rust
const CHUNK: usize = 65_536;
/// Read `reader` to its end, refusing once the bytes would pass `cap` (equal accepted).
async fn read_capped_async(reader: impl AsyncRead + Unpin, cap: u64) -> std::result::Result<Vec<u8>, Capped>;
/// Keep only the last `STDERR_CAP` bytes, reading in chunks.
async fn read_tail(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>>;
/// Read and discard.
async fn drain(reader: impl AsyncRead + Unpin) -> std::io::Result<()>;
/// The web body under the same cap.
async fn read_body_capped(response: &mut reqwest::Response, cap: u64) -> std::result::Result<Vec<u8>, Capped>;
enum Capped { Over, Io(std::io::Error) }
```

Refusal on the cap: `ToolError::internal(format!("document service output is over the
doc_service.max_input_bytes limit of {cap}"))` (today's bundle message, `service.rs:515-517`, now one
`fn over_cap(cap) -> ToolError` shared by the four sites). Every other message is unchanged.

`TOOL_CONTRACT.txt` and `tool-contract-golden.json`: unchanged (DEC-024).

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/pdf_bound.rs` | create: budget, measurer, load, walk, arm and defuse, content sink, title, hook | FR-NEW-001 to FR-NEW-009 | US-0212 to US-0216 |
| `crates/core/src/docs/mod.rs` | modify: `pub(crate) mod pdf_bound;` | layout | US-0212 |
| `crates/core/src/docs/extract.rs` | modify: `pdf` adapts `PdfText`, `pdf_title` removed, `PdfHeld` hook and its move in `Extractor::extract` | FR-NEW-006, FR-NEW-009 | US-0212, US-0216 |
| `crates/core/src/tools/pdf_testkit.rs` | create: PDF writer, `zlib_run`, bomb streams, object streams, xref stream, encrypted fixture, hook readers | fixtures (DEC-019) | US-0212, US-0213, US-0214, US-0215, US-0216 |
| `crates/core/src/tools/pdf_bound_e2e.rs` | create: the `e2e_0089_*` PDF tests | every PDF requirement | US-0212 to US-0217 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod pdf_testkit; mod pdf_bound_e2e; mod converter_bound_e2e;` | test layout | US-0212, US-0218 |
| `Cargo.toml`, `crates/core/Cargo.toml`, `Cargo.lock` | modify: `weezl = "0.1"` | DEC-021 | US-0213 |
| `crates/core/src/docs/service.rs` | modify: capped stdout, tail ring, drain, capped web body, kill on cap, hook | FR-NEW-010 to FR-NEW-013 | US-0218, US-0219, US-0220 |
| `crates/core/src/tools/converter_bound_e2e.rs` | create: the `e2e_0089_*` converter tests and stand ins | FR-NEW-010 to FR-NEW-014 | US-0218, US-0219, US-0220 |
| `crates/core/src/api/dataplane.rs` | modify (tests only): `e2e_0089_002`, `023`, `039`, `041` | FR-NEW-014 | US-0217, US-0221 |
| `.agent_docs/architecture.md` | modify: PDF extraction bound paragraph | DEC-023 | US-0217 |
| `.agent_docs/config.md` | modify: `max_input_bytes` caps every answer channel | DEC-023 | US-0220 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-023) | US-0221 |

`crates/core/src/tools/archive_testkit.rs` is read, not modified: its `pub(crate)` helpers `MOUNT`,
`OWNER`, `SECOND`, `add_second_member`, `seed`, `read`, `exists`, `office_fixture`,
`office_fixture_allowance`, `extract_text`, `deflate_run`, `heavy`, `nothing_left` and the `Fixture` it
re exports are used as they are (`deflate_run` is already `pub(crate)`, `:379`).

Affected existing tests, kept unchanged and green: `pdf_extracts_text_pages_and_metadata`,
`pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion` (`extract.rs:2029`, `:2045`, `:2215`,
`tiny_pdf` at `:1975`), the `tools/artifacts.rs` users of `tiny_pdf`, the 22 tests of `docs/service.rs`
(among them `cli_outdir_mode_caps_the_bundle_size` `:978`, `bundle_size_cap_is_checked_before_reading`
`:1021`, `cli_timeout_kills_the_child_and_removes_the_tempdir` `:845`,
`cli_failure_carries_the_exit_code_and_only_the_stderr_tail` `:886`), the documentize tests of
`tools/document.rs`, the upload tests of `api/dataplane.rs` (`:2944-2980`), every `e2e_0088_` test. Any
other existing test that changes is a story failure: stop and report it.

Affected documentation: `README.md` (document extraction and `doc_service` paragraphs, `:180-215`),
`AGENTS.md` (`docs/extract.rs` bullet, new `docs/pdf_bound.rs` bullet, `docs/service.rs` mention, key
commands filter), `.agent_docs/tools.md` (`:345` and `:438` known gaps BL-0142 and BL-0143 closed,
`fs.extract_text` PDF limits, `fs.documentize` answer cap), `.agent_docs/testing.md` (filter `e2e_0089_`,
the PDF builders, the stand in converters, `perl`), `.agent_docs/config.md:166`,
`.agent_docs/architecture.md`.

## 7. Compatibility and risks

- Declared changes (spec): a PDF past a limit is refused; a converter answer past `max_input_bytes` is
  refused on every channel, the converter stopped.
- Undeclared, documented: (1) the cross reference stream of a PDF is now also bound by the stream limit and
  refused with the FR-NEW-002 message before the load (DEC-005); (2) a decoded size counts every decoding
  step lopdf performs on a stream and the two row buffers of a PNG predictor (DEC-003), so a stream with a
  predictor is refused slightly before its final bytes reach the limit; (3) an encrypted PDF is decrypted
  after an ordinary load instead of by lopdf's encrypted loader, with the differences listed in DEC-005;
  (4) a stream the walk does not reach is given to the reader as stored bytes (DEC-007).
- Cost: measured streams are inflated two or three times (section 2); the PDF tests decode up to
  268 435 456 bytes in an unoptimized debug build, serialized by `heavy()` (DEC-018).
- Memory not in the spec's definitions, filed: parsed structures (content operations, cross reference
  entries, the object table) can be larger than the bytes they come from (BL-0151); a converter's own child
  processes survive the kill of the converter (BL-0152).
- Between US-0212 and US-0213 (spec branch only): encrypted files, cross reference streams and LZW or
  ASCII85 streams are decoded as today, unmeasured (DEC-027).
- Rollback: revert the story commits; no data or contract change.

## 8. Security design

- Threat surface: a stored PDF is untrusted (its streams, filters, decode parameters, cross reference,
  object streams, encryption dictionary, resources and content operators are chosen by whoever uploaded
  it); the converter's answer is untrusted (an operator chosen program or endpoint fed an untrusted
  document). Sinks: lopdf's inflation into an unbounded `Vec` (`object.rs:794-872`, `parser_aux.rs:489`,
  `object_stream.rs:42`), pdf-extract's copies kept in saved states (`lib.rs:1783`), the page text built
  whole (`extract.rs:513-521`), `read_to_end` on stdout and stderr (`service.rs:268-279`) and `text()` on
  the web body (`service.rs:387`).
- Per class sweep occurrence (spec 14.2), fix site and mechanism:

| # | Occurrence | Fix site | Mechanism | DEC | Exploit test | Story |
|---|---|---|---|---|---|---|
| 1 | object streams at load (`reader.rs:965`, encrypted `:889`) | `pdf_bound::load` | deferred by the load filter, measured then decoded in increasing number; encrypted: `/Encrypt` renamed so the filter applies | DEC-004, DEC-005 | E2E-SEC-002 (plain); design test `encrypted_object_stream_bomb_is_refused` | US-0212, US-0213 |
| 2 | page content (`document.rs:589-600` via `lib.rs:2418`) | `pdf_bound::walk` | measured before the page runs; armed only within limits | DEC-006, DEC-007 | E2E-SEC-001 | US-0212 |
| 3 | font files (`lib.rs:386-411`) | walk, `Tf` | measured on the first selection of the name | DEC-007 | E2E-SEC-003 | US-0212 |
| 4 | character maps (`lib.rs:926`, `:990`) | walk, `Tf` | same | DEC-007 | E2E-SEC-004 | US-0212 |
| 5 | colour profiles (`lib.rs:1476-1520`) | walk, `cs`/`CS` | measured per selection | DEC-007, DEC-009 | E2E-SEC-005 | US-0212 |
| 6 | forms and images on `Do` (`lib.rs:1860-1867`) | walk, `Do` | measured per draw | DEC-007 | E2E-SEC-006 | US-0212 |
| 7 | the same form per draw (`lib.rs:1866`) | walk | every draw counted in the page total | DEC-008 | E2E-SEC-007 | US-0215 |
| 8 | colour profile copies in saved states (`lib.rs:1633-1639`, `:1783`) | load end, colour spaces | profiles and tint transforms replaced by empty stand ins, so a copy holds nothing; selections counted | DEC-009 | E2E-SEC-008 | US-0215 |
| 9 | object streams kept for the call (`document.rs:473-496`) | `pdf_bound::load` | load total; container bytes freed after parsing | DEC-004 | E2E-SEC-009 | US-0214 |
| 10 | object table without bound (`reader.rs:831-1022`) | `pdf_bound::load` | objects counted before parsing each object stream | DEC-004 | E2E-SEC-010 | US-0214 |
| 11 | page text built whole (`extract.rs:518-521`) | `ContentSink` | the writer refuses a piece that would pass the threshold | DEC-010 | E2E-SEC-011 | US-0216 |
| 12 | title load (`extract.rs:539`) | `pdf_bound::extract` | the title is read from the document already loaded: no second load | DEC-011 | E2E-SEC-012 | US-0212 |
| 14 | converter stdout, text mode (`service.rs:272`) | `CliDocService::convert` | capped read; kill and reap on the cap | DEC-015 | E2E-SEC-013 | US-0218 |
| 15 | converter stderr (`service.rs:273`) | same | tail ring of 8 192 bytes | DEC-015 | E2E-SEC-014 | US-0219 |
| 16 | web body (`service.rs:387`) | `ApiDocService::convert` | chunked read under the cap; response dropped | DEC-016 | E2E-SEC-015 | US-0220 |
| 17 | web bundle body (`service.rs:552-579`) | same | parsed only from the capped bytes | DEC-016 | E2E-SEC-016 | US-0220 |
| 18 | stdout in bundle mode (`service.rs:272`, `:305-308`) | `CliDocService::convert` | drained, never kept | DEC-015 | E2E-SEC-017 | US-0219 |
| new | cross reference stream (`parser_aux.rs:487-490`), missed by the sweep, in FR-NEW-002's "every decode" | `pdf_bound::xref_guard` | located as lopdf locates it, measured before the load | DEC-005 | design test `cross_reference_stream_bomb_is_refused` | US-0213 |

  Occurrences 13 and 19 to 26 are safe or filed (spec 14.2: BL-0147, BL-0148, BL-0149, BL-0150).
- Where the exploit tests run and the second channel: `crates/core/src/tools/pdf_bound_e2e.rs` through
  `core::fs_ops::extract_document` after `f.state.authorize(MOUNT, person)` (the function both doors call);
  second channels: the decoded bytes held and the content held read through the `PDF_HELD` hook (DEC-012),
  the listing of `/docs`, the artifact lists, the audit trail and a following 52 428 800 byte write
  (`archive_testkit::nothing_left`). Converter: `crates/core/src/tools/converter_bound_e2e.rs` through
  `core::fs_ops::documentize` (and `McpServer::fs_documentize`, `fs_write_bytes`, the HTTP door for
  E2E-039, 041); second channels: the `CONVERTER_HELD` hook (DEC-017), the stand in's pid probed with
  `kill -0` after the call, the working folder path the stand in wrote probed with `Path::exists`, the
  web stand in's count of bytes sent, the stand in's start marker for "never started".
- Literal payloads: a PDF bomb stream is a zlib stream of exactly 200 000 000 spaces built from cached
  segments (stored in under 300 000 bytes), in a file under 1 000 000 bytes; the dictionaries and operators
  of spec 9.3 are written verbatim (`<< /Length 194000 /Filter /FlateDecode >>` style, the real `Length`
  of the built stream; `/CS0 cs q` 300 times; `/Fm0 Do` 300 times; `<0101...01> Tj` with 2 000 000 codes);
  the converter floods are 1 073 741 824 letters.
- Red before the fix: in each story the tests are written first with the hook wired and the limit about to
  be built absent, and fail for the reason the spec names (the held figure, or the missing refusal).
- Red run cost: a red PDF bomb decodes 200 000 000 bytes (up to 268 435 456 for the load and page
  totals) and holds it; a red converter flood holds 1 073 741 824 bytes. Every such test takes `heavy()`
  first (DEC-018).
- Secrets: none involved; nothing new is traced but the refusal messages (path and numbers). The web
  stand in records no header.

## 9. Test strategy

Functional e2e through the typed engine call, the way SPEC-0088 does. PDF: `core::fs_ops::extract_document`
after `f.state.authorize(MOUNT, person)` in `crates/core/src/tools/pdf_bound_e2e.rs`; E2E-030 through
`McpServer::fs_extract_text` under `crate::logging::capture::lock_for_test`; E2E-002 and E2E-023 through
the HTTP door in the tests of `crates/core/src/api/dataplane.rs` (`Harness`, project `proj` and member
`owner@test.com` stand for `atlas` and `mia@example.com`). Converter: `core::fs_ops::documentize` with a
real `CliDocService` or `ApiDocService` over a stand in, in `crates/core/src/tools/converter_bound_e2e.rs`;
E2E-040 through `McpServer::fs_write_bytes`, E2E-046 through `McpServer::fs_documentize` under the log
capture; E2E-039 and E2E-041 through the HTTP door (`Harness::with_doc_service`). Names
`e2e_0089_<id>_<title in snake case>`, cut at 50 characters, trailing underscore dropped; filter
`e2e_0089_`. No cargo feature (DEC-018).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-002 | functional e2e | HTTP door | api/dataplane.rs tests | US-0217 |
| E2E-003 | functional e2e | typed engine call (documentize) | converter_bound_e2e.rs | US-0218 |
| E2E-004 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0219 |
| E2E-005 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-006 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0216 |
| E2E-007 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-008 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-009 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-010 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-011 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-012 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-013 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-014 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-015 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0216 |
| E2E-016 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0216 |
| E2E-017 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-018 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-019 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-020 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-021 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-022 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-023 | functional e2e | HTTP door | api/dataplane.rs tests | US-0217 |
| E2E-024 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-025 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-026 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-027 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-028 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0216 |
| E2E-029 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0217 |
| E2E-030 | functional e2e | assistant tool method, log capture | pdf_bound_e2e.rs | US-0217 |
| E2E-031 | functional e2e | typed engine call, three calls joined | pdf_bound_e2e.rs | US-0217 |
| E2E-032 | functional e2e | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-033 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-034 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-035 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-036 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-037 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0219 |
| E2E-038 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0219 |
| E2E-039 | functional e2e | HTTP door (upload) | api/dataplane.rs tests | US-0221 |
| E2E-040 | functional e2e | assistant tool method `fs_write_bytes` | converter_bound_e2e.rs | US-0218 |
| E2E-041 | functional e2e | HTTP door (documentize) | api/dataplane.rs tests | US-0221 |
| E2E-042 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-043 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-044 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-045 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-046 | functional e2e | assistant tool method `fs_documentize` x2 joined, log capture | converter_bound_e2e.rs | US-0218 |
| E2E-047 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-048 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-049 | functional e2e | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-SEC-001 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-002 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-003 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-004 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-005 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-006 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-007 | exploit | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-SEC-008 | exploit | typed engine call | pdf_bound_e2e.rs | US-0215 |
| E2E-SEC-009 | exploit | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-SEC-010 | exploit | typed engine call | pdf_bound_e2e.rs | US-0214 |
| E2E-SEC-011 | exploit | typed engine call | pdf_bound_e2e.rs | US-0216 |
| E2E-SEC-012 | exploit | typed engine call | pdf_bound_e2e.rs | US-0212 |
| E2E-SEC-013 | exploit | typed engine call | converter_bound_e2e.rs | US-0218 |
| E2E-SEC-014 | exploit | typed engine call | converter_bound_e2e.rs | US-0219 |
| E2E-SEC-015 | exploit | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-SEC-016 | exploit | typed engine call | converter_bound_e2e.rs | US-0220 |
| E2E-SEC-017 | exploit | typed engine call | converter_bound_e2e.rs | US-0219 |

Design tests (no spec id, guarding decisions the spec's tests do not reach), in `pdf_bound_e2e.rs` unless
said: `pdf_cross_reference_stream_bomb_is_refused`, `pdf_encrypted_object_stream_bomb_is_refused`,
`pdf_encrypted_document_text_is_unchanged`, `pdf_lzw_content_bomb_is_refused`,
`pdf_escaped_encrypt_key_is_found`, `pdf_ascii85_then_flate_bomb_is_refused`, `pdf_lzw_and_ascii85_text_is_unchanged` (US-0213);
`pdf_one_profile_kept_in_300_saved_states_holds_nothing` (US-0215, in `docs/pdf_bound.rs` tests);
`zlib_run_matches_flate2` (US-0212, in `pdf_testkit.rs`).

Test data: generated in the tests (DEC-019), no committed fixture, nothing user provided. One recorded
literal: today's answer for E2E-017, captured in US-0212 before any production change (DEC-020).

## 10. Decisions log

- **DEC-001:** the PDF bound is built by measuring then letting the reader decode, in a new module
  `docs/pdf_bound.rs` that drives the load and the page loop itself with lopdf's and pdf-extract's public
  pieces (section 2, chosen approach). **Obliged by:** SPEC-0089/FR-NEW-001 to FR-NEW-007, spec 3.3 and
  14.3. **Rationale:** the libraries expose no decode cap, but expose the load filter before object stream
  inflation, `ObjectStream::new`, the no `Filter` copy path and the page by page entry with any
  `OutputDev`; together they let the caller decide every inflation, in safe Rust, with no new crate; the
  four rejected approaches each fail a requirement (section 2). **Alternatives considered:** byte pre scan
  plus unmodified reader; materializing decoded streams; hooks only; a bounded child or thread; a patched,
  vendored or other library. **Source:** design, library sources. **Evidence:**
  `lopdf-0.42.0/src/reader.rs:821`, `:889`, `:959-965`; `src/object_stream.rs:40-42`;
  `src/object.rs:794-815`, `:849-872`; `src/load_options.rs`; `pdf-extract-0.12.1/src/lib.rs:6`,
  `:1255-1261`, `:2117-2122`, `:2389-2421`.
- **DEC-002:** the limits are `pub(crate) const` in `pdf_bound.rs` (`STREAM_LIMIT` 104 857 600,
  `LOAD_LIMIT` 268 435 456, `PAGE_LIMIT` 268 435 456, `OBJECT_LIMIT` 1 000 000), the content limit is
  `extract::CONTENT_LIMIT`, the chunk 65 536; no configuration, no test seam that scales them. **Obliged
  by:** SPEC-0089/FR-NEW-001 to FR-NEW-006, spec D-4, D-5. **Rationale:** the messages and held figures the
  spec asserts are literal; scaled limits prove nothing about the shipped ones (SPEC-0088 DEC-002).
  **Alternatives considered:** a `#[cfg(test)]` limits struct. **Source:** spec. **Evidence:** spec 9.
- **DEC-003:** `measure(stream)` mirrors `Stream::decompressed_content` (`object.rs:794-815`) step by step
  and counts, streaming, what each step outputs, holding one 65 536 byte chunk: filters in the order of
  `filters()`; `FlateDecode` through `flate2::Decompress::new(true)`, and when that zlib pass yields no byte
  the raw pass `Decompress::new(false)` over `input[2..]` is measured too, whether or not the zlib pass
  reported an error (lopdf falls back only on error with empty output, `:858-866`; an over count is safe, an
  under count is not); `LZWDecode` through `weezl` configured as lopdf configures it (`EarlyChange`,
  `:818-846`) (US-0213); `ASCII85Decode` by a streaming copy of `decode_ascii85` (`:874-920`) (US-0213); an
  unknown filter ends the chain: the steps before it count (lopdf decoded them before failing), and a
  chain whose first filter is unknown counts 0 (spec D-9); a PNG predictor (`Predictor` 10 to 15 in
  `DecodeParms`) adds its two row buffers, `2 * Colors * max(8, BitsPerComponent) / 8 * Columns`, and its
  output, at most its input, counted as the input size (`object.rs:929-948`, `filters/png.rs:87-95`). The
  decoded size of a stream is the sum over its steps; for one `FlateDecode` and no predictor it is exactly
  the inflated length. The stream limit is checked after every chunk on the running size, then the load or
  page total (FR-NEW-002 first), so decoding stops at most one chunk past a limit. Equal is accepted.
  Until US-0213, a stream with `LZWDecode` or `ASCII85Decode` in its chain is armed unmeasured and counts 0
  (DEC-027). **Obliged by:** SPEC-0089/FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006. **Rationale:** a
  measurement below what lopdf will produce would let a bomb through; the same crates (flate2, weezl) give
  the same output; counting the predictor's row buffers bounds an allocation driven by `Columns` alone.
  **Alternatives considered:** measuring with lopdf's own decoder (unbounded); counting only the final
  output (misses the intermediate buffers lopdf holds). **Source:** library sources. **Evidence:** as cited.
- **DEC-004:** the load (non encrypted): `Document::load_mem_with_options(data, LoadOptions { filter:
  Some(defer_object_streams), ..Default::default() })`; `defer_object_streams` renames the `Type` of a
  stream typed `ObjStm` with a non empty content to `McpFsObjStm` and keeps it (an empty one, whose content
  lopdf reads later from an indirect `Length`, is left alone so lopdf treats it as today,
  `reader.rs:985-990`, `:1025-1027`). Then: objects outside object streams are counted
  (`doc.objects.len()`, FR-NEW-001); then, in increasing object number, each deferred stream is measured
  (stream limit, load total), decoded with `decompressed_content()` (now known to be within the limits),
  its objects counted as the number of number pairs in `content[..First]` split on whitespace, as
  `ObjectStream::new` splits them (`object_stream.rs:60-70`) (FR-NEW-001, a pair is an object whatever its
  number), then parsed with `ObjectStream::new` on the decoded bytes set as plain content and its objects
  inserted as lopdf inserts them: kept only when the cross reference does not place that number in another
  container, never replacing an object already loaded (`reader.rs:968-984`, `:1020-1022`); a stream that
  fails to parse is removed from the document, as lopdf drops it (`.ok()?`, `:965`); the container's content
  is then emptied and its `Type` restored. A load error keeps today's message. **Obliged by:**
  SPEC-0089/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006, FR-NEW-007 item 3, FR-NEW-009. **Rationale:** the
  filter runs before the inflation and needs no state, so a fn pointer suffices; counting pairs before
  parsing refuses 2 000 000 objects without parsing them; emptying the container keeps one object stream's
  bytes held at a time. **Alternatives considered:** counting `/N` (the attacker's claim); counting distinct
  object numbers (spec E2E-026 and E2E-SEC-010 fixtures cannot hold 1 000 001 distinct numbers in their
  sizes). **Source:** library sources. **Evidence:** as cited.
- **DEC-005:** before the load (US-0213): `xref_guard` locates the cross reference sections as lopdf does
  (the `%PDF-` offset, `startxref` searched before the last `%%EOF` of the last 512 bytes,
  `reader.rs:1262-1278`; then `Prev` and `XRefStm` chains with a seen set, `:765-791`), parses each object
  section with `Reader::get_object` over a cross reference holding that one offset (the public parser, so the
  dictionary is the one lopdf will see), and measures its stream when its `Length` is direct (an indirect one
  makes lopdf read it empty, `parser/mod.rs:344-359`); over the stream limit: the FR-NEW-002 refusal before
  any object is read. Any discovery failure is ignored: lopdf then fails or succeeds as today. A classic
  section's trailer is parsed the same way from a synthetic `0 0 obj` holding its dictionary bytes, located
  by a minimal tokenizer that skips strings, hex strings, names and comments. When the main trailer holds an
  `/Encrypt` key (at depth 1, found by that tokenizer, which decodes `#xx` name escapes), the bytes are
  copied and that key renamed in place to a name of the same length (`/NoCrypt` for the plain spelling,
  offsets unchanged); the copy is checked by re parsing its trailer with lopdf's parser, and when that
  parse still holds `/Encrypt` the document fails closed as unreadable (`could not extract <path>: pdf:
  the encryption dictionary could not be isolated`, the FR-NEW-009 family of unreadable PDFs), never
  handed to the encrypted loader; else the copy is loaded
  with the filter of DEC-004 (lopdf's encrypted loader, which ignores the filter, is never entered); then
  the key is renamed back in `doc.trailer` and today's states are reproduced: if
  `doc.authenticate_password("")` fails, every object but the encryption dictionary is dropped (what
  `load_encrypted_document` leaves, `reader.rs:832-856`, `:1147-1166`) and `doc.decrypt("")` gives today's
  error; else `doc.decrypt("")` decrypts every object (it skips our renamed object streams,
  `document.rs:477-482`), and the deferred object streams are then handled as in DEC-004, inserted the way
  the encrypted loader does (only numbers the cross reference places in that container, replacing,
  `reader.rs:887-897`). Known differences with today, accepted: an object whose encrypted bytes contain
  `endobj` is now read whole (the encrypted loader cuts it, `extract_raw_object`); trailer key order.
  **Obliged by:** SPEC-0089/FR-NEW-002 ("every decode, whatever its purpose": the cross reference stream;
  occurrence 1 cites `reader.rs:889`), FR-NEW-003, FR-NEW-009. **Rationale:** the encrypted loader inflates
  object streams with no type check and no filter; renaming the key routes it through the filtered loader,
  and decryption is lopdf's own. **Alternatives considered:** measuring encrypted object streams before the
  load (needs our own cross reference merge and key derivation); refusing encrypted PDFs (changes today's
  text, FR-NEW-009). **Source:** library sources. **Evidence:** as cited.
- **DEC-006:** after the load every stream object's `Filter` key is renamed `McpFsFilter` (defuse); per
  page the walk's measured set gets `Filter` back (arm) before `output_doc_page` and loses it after
  (disarm), on every exit path. **Obliged by:** SPEC-0089/FR-NEW-002, FR-NEW-004, FR-NEW-006. **Rationale:**
  lopdf and pdf-extract never inflate a stream without `Filter` (`object.rs:794-799`, `lib.rs:1256-1260`,
  `document.rs:593-596`), so anything the walk did not reach yields its stored bytes, bounded by the stored
  file, as an undecodable stream does today. **Alternatives considered:** arming everything (no bound on
  what the walk misses). **Source:** library sources. **Evidence:** as cited.
- **DEC-007:** the walk mirrors pdf-extract's decode sites for one page, iteratively (an explicit stack, no
  recursion in our code): resources inherited through `Parent` (`lib.rs:2352-2362`); the page's content
  streams (`Document::get_page_contents`) measured, decoded, parsed with `Content::decode` (the decoded bytes
  dropped once parsed, the operations kept); `Tf`: the first selection of a font name on the page (the
  reader's per page cache by name, `lib.rs:1712`, `:2408`) measures, by the font's `Subtype` as `make_font`
  dispatches (`lib.rs:322-332`): `Type0` its `Encoding` when a stream (`:990`) and `ToUnicode` (`:1002`,
  `:926`); `Type3` its `ToUnicode` (`:695`); any other its descriptor's `FontFile` when `Type1`,
  `FontFile2` when `TrueType`, `FontFile3` when present (`:386-411`), and `ToUnicode` (`:454`, `:458`);
  `cs`/`CS` on a name other than the four device names: the colour space array's `ICCBased` stream, a
  `Separation`'s alternate `ICCBased` stream and its tint transform when it is a stream of `FunctionType` 0
  or 4 (`lib.rs:1449-1525`, `:1168-1225`); `Do`: the XObject stream, whatever its subtype, measured,
  decoded and parsed, its own `Resources` or the caller's (`:1860-1867`). Where pdf-extract would panic
  (a missing resource, an operation that does not parse), the walk stops that branch without refusing and
  the reader then panics as today (`pdf parser failed on this document`). A form already on the walk stack
  stops that branch (the reader's own recursion is BL-0147). **Obliged by:** SPEC-0089/FR-NEW-002,
  FR-NEW-004, FR-NEW-007 item 4, FR-NEW-009. **Rationale:** every stream the reader decodes for text is
  measured in drawing order; a site missed costs text fidelity, never a bound (DEC-006). **Alternatives
  considered:** recursion (a deep form chain would overflow our stack). **Source:** library sources.
  **Evidence:** as cited.
- **DEC-008:** the page total counts, in drawing order, each content stream once, each font's streams on
  the first selection of its name, each colour profile and tint transform on every selection, each form or
  image on every draw (nested draws included); a stream's size is measured once per page and reused for
  its repeats (`sizes`), a form's parsed operations are kept per page (`forms`) and replayed with the
  current resources; the count restarts at 0 on each page; the refusal names the page number from 1.
  **Obliged by:** SPEC-0089/FR-NEW-004, spec D-7, D-8. **Rationale:** repeats are what the reader decodes
  again; reuse keeps the walk's own cost to one decode per stream per page. **Alternatives considered:**
  measuring every repeat (300 inflations of the same form before the refusal). **Source:** spec.
  **Evidence:** spec FR-NEW-004 business rules.
- **DEC-009:** colour profile copies (US-0215): at the end of the load, every array whose first element is
  `/ICCBased` gets its stream reference replaced by a stand in (a new object: the original dictionary
  without `Filter`, `DecodeParms` and `Length`, empty content), and every `/Separation` array's tint
  transform stream reference by a stand in with empty content (`FunctionType` 0) or, for `FunctionType` 4,
  a content set by the walk on first selection: empty when the measured original decodes to valid UTF-8,
  the single byte `0xFF` otherwise (pdf-extract formats a type 4 function with `String::from_utf8(..)
  .unwrap()` inside a `warn!`, `lib.rs:1215-1220`, enabled by the `log` bridge of
  `tracing-subscriber`, so validity decides whether it panics today). The walk counts the original's
  measured size on every selection. The stand ins are what saved states copy (`q` clones the graphics
  state, `lib.rs:1783`), so a copy holds nothing. The spec's "colour profile" (glossary: "a stream
  describing a colour space") covers both. **Obliged by:** SPEC-0089/FR-NEW-004, FR-NEW-006 ("a colour space
  kept in a saved drawing state ... counts as held"), FR-NEW-009. **Rationale:** pdf-extract keeps the
  decoded profile in `ColorSpace::ICCBased(Vec<u8>)` (`lib.rs:1425`, `:1446`) and never reads it for text;
  without stand ins, one selection and 300 `q` would hold 300 copies within the page total.
  **Alternatives considered:** counting `q` copies in the page total (refuses where the spec accepts and
  names no such refusal). **Source:** library sources. **Evidence:** as cited.
- **DEC-010:** the content limit (US-0216): each page runs `output_doc_page(&doc, &mut PlainTextOutput::new(
  &mut sink as &mut dyn std::io::Write), n)` (pdf-extract's own page text, `ConvertToFmt` for
  `&mut dyn io::Write`, `lib.rs:2117-2122`); `ContentSink` appends into the one content buffer and refuses
  (an `io::Error`, seen as `OutputError::FormatError`, then recognized by its flag) a write that would bring
  the buffer past `CONTENT_LIMIT + 65 536`; after the page, its text is trimmed in place (leading bytes
  drained, trailing truncated) behind the marker `\n\n---\n*[Page <n>]*\n\n` pushed before it (removed with
  the page when empty), and the content (the buffer minus the two leading newlines the final trim drops) is
  checked against `CONTENT_LIMIT`: over, the refusal, before the next page's walk. The final body is the
  buffer trimmed as today (`parts.trim()`, `extract.rs:518-522`). A page error other than the sink's ends
  the page loop as today (`while let Ok(..)`, `lib.rs:2327-2331`). **Obliged by:** SPEC-0089/FR-NEW-005,
  FR-NEW-006, FR-NEW-007 item 4, FR-NEW-009. **Rationale:** the refusal comes before the crossing piece is
  appended, one buffer holds the content (no `format!` copy), so the content held stays under 268 500 992;
  the byte count after each page makes E2E-028 refuse before page 2. **Alternatives considered:** an
  `OutputDev` checking after each character (the piece, up to a whole character map entry, lands first).
  Edge accepted: a page whose untrimmed text passes the threshold by more than 65 536 bytes of surrounding
  whitespace is refused although its trimmed text would fit. **Source:** library sources. **Evidence:** as cited.
- **DEC-011:** the title is read from the loaded document's trailer `/Info` once the pages are done, by the
  same code as today's `pdf_title` minus its second load (`extract.rs:538-548`). **Obliged by:**
  SPEC-0089/FR-NEW-007 item 5, FR-NEW-009 (occurrence 12). **Rationale:** the second load inflated every
  object stream again; the loaded document holds the same `/Info` (decrypted, object streams expanded).
  **Alternatives considered:** a second bounded load (double cost, same answer). **Source:** existing code.
  **Evidence:** `extract.rs:513`, `:539`.
- **DEC-012:** measurement hook, test only: `pdf_bound::extract` stores `PdfHeld { decoded, content }` in
  the thread local `LAST_PDF_HELD` on every outcome, a panic included (a drop guard owns the figures, so
  the unwinding stores them too); `Extractor::extract` moves it into
  `PDF_HELD[path]` beside the Office move (`extract.rs:183-185`). `decoded` is the largest, over the call,
  of: the decoded object stream being parsed (load); the decoded content or form buffer the walk holds while
  parsing it (one at a time, DEC-007); while the reader runs a page, that page's decode count (what the
  reader may hold: it decodes only armed streams, each counted). `content` is the content buffer's largest
  length. The measuring chunk (65 536) is outside the figure (SPEC-0084 DEC-006). A call that decodes
  nothing reads 0; a cached call leaves no entry. The reader's own transient copies (`get_page_content`
  concatenation) are inside FR-NEW-006's bound: the page decode count plus the content concatenation stays
  under twice the page total, 536 870 912, below 536 936 448, the load having freed its buffers.
  **Obliged by:** SPEC-0089/FR-NEW-006, NFR 7.6. **Rationale:** the figure counts real buffers of our code
  and the decodes the reader is allowed; with the limits absent (each story's red step) it reports the
  full decode. **Alternatives considered:** a counting global allocator (needs `unsafe`). **Source:**
  SPEC-0088 DEC-008. **Evidence:** `extract.rs:181-185`, `:589-596`.
- **DEC-013:** refusals and order: `pdf_bound` returns `ToolError::invalid_argument` with the five messages
  of section 5 (no prefix); a load error is `pdf: PDF error: <e>`, a panic anywhere in the closure is
  `pdf parser failed on this document` (one `catch_unwind(AssertUnwindSafe(..))`, as today). Order: cross
  reference guard, load, objects outside, object streams (stream, load total, objects), then pages in order
  (walk: stream then page total per decode in drawing order; then the reader with the content sink), then
  the title. **Obliged by:** SPEC-0089/FR-NEW-001 to FR-NEW-005, FR-NEW-007, FR-NEW-009. **Rationale:** the
  order of FR-NEW-007 is the order the code runs. **Alternatives considered:** none. **Source:** spec.
  **Evidence:** `extract.rs:513-516`, `:373-381`.
- **DEC-014:** tests a story's earlier stories or today's code already satisfy are green on arrival and
  each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged
  by:** n/a (no code impact): story planning. **Rationale:** SPEC-0085 DEC-013, SPEC-0088 DEC-014.
  **Source:** design. **Evidence:** n/a.
- **DEC-015:** CLI converter (US-0218, US-0219): the three pipes are read concurrently with the wait,
  inside the existing timeout (`service.rs:262-279`): text mode stdout through `read_capped_async(stdout,
  max_input_bytes)` (65 536 byte reads; a read that would bring the buffer past the cap returns `Over` with
  the buffer unextended, so at most the cap plus one chunk is held); bundle mode stdout through `drain`
  (one chunk buffer, nothing kept); stderr through `read_tail` (append a chunk, then keep the last 8 192
  bytes: at most 8 192 + 65 536 = 73 728 held). The reads and the wait run under `tokio::try_join!`, so
  `Over` returns at once; then `child.kill().await` (kill and reap, as the timeout path,
  `service.rs:287-295`) and `over_cap(cap)`, before `dir` drops. Order: cap or timeout first, whichever
  comes first; then exit status, empty answer, bundle (FR-NEW-012). Text mode markdown is
  `String::from_utf8_lossy` of the capped bytes as today. **Obliged by:** SPEC-0089/FR-NEW-010, FR-NEW-011,
  FR-NEW-012, NFR 7.1, 7.5. **Rationale:** the same capped read convention as SPEC-0085 (`read_capped`,
  `archive_single.rs:92-108`), async. **Alternatives considered:** polling the child's size (none exists
  for a pipe). **Source:** existing code. **Evidence:** as cited.
- **DEC-016:** web converter (US-0220): status and content type read first (`service.rs:378-385`); the body
  through `read_body_capped(&mut response, max_input_bytes)` (`Response::chunk()` until `None`; a chunk that
  would pass the cap returns `Over` without being appended; a transport error mid body yields an empty body,
  as today's `text().await.unwrap_or_default()`); the response is then dropped, which closes the
  connection. Failure status: `head_of` of the lossy text of what was read, `Over` included (read no further
  than the cap); success and `Over`: `over_cap(cap)`; success: the text is `String::from_utf8` of the bytes
  (no copy) or the lossy repair, then `parse_api_bundle`, the plain Markdown or the named field as today.
  **Obliged by:** SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-014. **Rationale:** the status is known
  before the body (spec D-13); reqwest without `charset` decodes lossily, so the text is identical.
  **Alternatives considered:** `bytes_stream` (same, one more adapter). **Source:** existing code.
  **Evidence:** `reqwest-0.12.28/src/async_impl/response.rs:169-174`, `:315`.
- **DEC-017:** converter hook, test only: each `convert` (both modes, every outcome) pushes
  `ConverterHeld { answer, diagnostic, bundle_stdout }` onto the thread local `CONVERTER_HELD`; `answer`
  is the largest answer buffer length (text stdout or web body), `diagnostic` the largest tail buffer length
  (73 728 at most), `bundle_stdout` the largest chunk read and discarded. **Obliged by:**
  SPEC-0089/FR-NEW-011, NFR 7.6. **Rationale:** one record per call lets E2E-046 read two concurrent calls
  (same file name in two projects); the reads run on the test's thread in a current thread runtime.
  **Alternatives considered:** a map keyed by file name (both calls convert `deck.pptx`). **Source:**
  SPEC-0088 DEC-008. **Evidence:** n/a.
- **DEC-018:** test layout: `crates/core/src/tools/pdf_bound_e2e.rs` and
  `crates/core/src/tools/converter_bound_e2e.rs` (`#[cfg(test)]` in `tools/mod.rs`), each test a single
  threaded `#[tokio::test]` through the typed engine call, except E2E-030 and E2E-046 (plain `#[test]` on a
  dedicated current thread runtime under `crate::logging::capture::lock_for_test`, model
  `office_bound_e2e.rs:943-979`) and the four HTTP tests in `api/dataplane.rs`. Every test that builds or
  decodes 10 MB or more, or floods a converter, takes `archive_testkit::heavy()` first. No feature.
  **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0088 DEC-010 and DEC-011.
  **Source:** existing code. **Evidence:** `crates/core/src/tools/office_bound_e2e.rs`.
- **DEC-019:** fixtures in `crates/core/src/tools/pdf_testkit.rs`: `zlib_run(prefix, letter, n, suffix) ->
  Vec<u8>` (bytes `0x78 0x9C`, then `archive_testkit::deflate_run`'s raw deflate, then the Adler 32 of the
  content computed with the closed form for a run of one byte, big endian); `bomb_stream(dict_extra) ->
  Vec<u8>` (an indirect stream object of exactly 200 000 000 spaces, `/Length` its real stored length,
  `/Filter /FlateDecode`); `PdfWriter` (objects by number, `stream` helper, a classic cross reference, or a
  cross reference stream with uncompressed `/W [1 4 2]` entries when compressed objects are present;
  `object_stream(number, objects: &[(u32, &str)], padding: usize)` writing `n off` pairs, `/N`, `/First`,
  the objects, then `padding` spaces, deflated through `zlib_run`); `page_pdf(content, resources, extra
  objects)` for one or two page documents with a `Helvetica` `/F1`; `cmap_expanding(code, text)` (a
  `ToUnicode` mapping one code to a text); `encrypted(doc)` (lopdf `Document::encrypt` with an
  `EncryptionVersion::V1` state, user password empty, then an appended cross reference stream listing the
  objects of the object stream as compressed, `/Prev` the classic section; ASSUMED: if lopdf refuses that
  shape, the implementer writes the RC4 encryption with `md-5` 0.10.6 from `Cargo.lock` as a dev
  dependency and reports it); `pdf_held_reset()`, `decoded_held(path)`, `pdf_content_held(path)`. Converter
  stand ins in `converter_bound_e2e.rs`: `stand_in(body) -> PathBuf` (an executable `sh` script in a temp
  folder that first writes `started` and `$$` to a marker, then runs the body, floods ending in `exec perl
  -e '...'` so the flooding process is the one killed); `cli(command, cap, timeout)`; `web(status,
  content_type, prefix, letter, n, suffix) -> (url, sent: Arc<AtomicU64>)` (an axum route answering a
  `Body::from_stream` of 65 536 byte chunks, counting bytes as they are pulled). **Obliged by:**
  SPEC-0089 section 9 (fixtures built by the design), FR-NEW-002 to FR-NEW-006, FR-NEW-010, FR-NEW-011.
  **Rationale:** a bomb stream costs about 200 KB and no 200 MB compression (SPEC-0088 DEC-011); `exec`
  makes the pid the stand in reports the process the server kills; `perl` ships with macOS and every
  Debian based image (`git` depends on it). **Alternatives considered:** lopdf's writer alone (cannot write
  a crafted object stream or a compressed cross reference entry); committed binary fixtures. **Source:**
  design. **Evidence:** `archive_testkit.rs:334-400`, `docs/service.rs:771-779` (`script`),
  `:1048-1087` (`stub_server`).
- **DEC-020:** E2E-017's expected answer is recorded before any production change: US-0212's step 0 builds
  `/docs/photo.pdf` (page 1 shows `Caption` and draws 200 times a 900 000 byte image whose `/Filter` is
  `/DCTDecode`), runs today's `extract_document`, and writes the observed outcome (success with its exact
  preview, or the code and message) as a literal in the test, quoted in the commit body. **Obliged by:**
  SPEC-0089/FR-NEW-009, E2E-017 (spec 14.3). **Rationale:** what a drawn image does today is unverified
  (`lib.rs:1582` unwraps the parse). **Alternatives considered:** none. **Source:** spec 14.3.
  **Evidence:** spec A-3.
- **DEC-021:** `weezl = "0.1"` is added to `[workspace.dependencies]` and to `crates/core/Cargo.toml`
  (US-0213): it resolves to the 0.1.12 already in `Cargo.lock` for lopdf, so no new crate is downloaded.
  **Obliged by:** SPEC-0089/FR-NEW-002 (an LZW stream is a decode), spec 3.3. **Rationale:** LZW expands
  up to thousands of times; a measurement must produce what weezl produces, so it is weezl. **Alternatives
  considered:** our own LZW decoder (any divergence is an under count); leaving LZW streams stored (changes
  today's text). **Source:** `Cargo.lock`. **Evidence:** `Cargo.lock:6721-6723`, `lopdf-0.42.0/Cargo.toml`.
- **DEC-022:** story order: the PDF core and the six stream bombs red first (US-0212), the remaining decode
  paths (US-0213), the load limits (US-0214), the page total and saved states (US-0215), the content limit
  (US-0216), order, trace, both doors and the PDF documentation paragraph (US-0217); then the converter's
  text answer (US-0218), diagnostic and bundle outputs (US-0219), web answer (US-0220), both doors and the
  documentation (US-0221). After US-0211, the last queued row. **Obliged by:** n/a (no code impact): story
  order. **Rationale:** exploit tests red before their fix; one writer at a time on `pdf_bound.rs` and on
  `service.rs`. **Source:** section 8. **Evidence:** n/a.
- **DEC-023:** documentation: US-0217 adds a dated "PDF extraction bound" paragraph to
  `.agent_docs/architecture.md`; US-0220 updates `.agent_docs/config.md` (`max_input_bytes` caps every
  answer channel); the closing story US-0221 updates `README.md`, `AGENTS.md` (new `docs/pdf_bound.rs`
  bullet, `docs/service.rs`, the `e2e_0089_` filter), `.agent_docs/tools.md` (`:345` and `:438` gaps
  closed, `fs.extract_text` PDF limits, `fs.documentize` answer cap, dated SPEC-0089 paragraph) and
  `.agent_docs/testing.md`. **Obliged by:** n/a (no code impact): project convention (`AGENTS.md`, spec
  14.3 docs list). **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-024:** the tool contract is unchanged: no description mentions the limits. **Obliged by:**
  SPEC-0089/FR-NEW-009, FR-NEW-014. **Rationale:** spec 14.3 leaves it to the design; SPEC-0088 DEC-016.
  **Source:** design. **Evidence:** `TOOL_CONTRACT.txt`.
- **DEC-025:** the log lines of E2E-030 and E2E-046 come from the existing wiring (`to_call_result` and
  `error_response` call `log_tool_failure`, SPEC-0088 DEC-012): INFO for the `ERR_INVALID_ARGUMENT` PDF
  refusal, ERROR for the `ERR_INTERNAL_ERROR` documentize refusal; no new log call. **Obliged by:**
  SPEC-0089/FR-NEW-008, FR-NEW-013 business rules, NFR 7.6. **Rationale:** a second log site would log
  twice. **Source:** existing code. **Evidence:** `crates/core/src/logging.rs:79-85`.
- **DEC-026:** the converter tests run the real `CliDocService` and `ApiDocService` (the real reads), over
  stand ins, through `core::fs_ops::documentize`; the stand in limit is `max_input_bytes` 1 048 576 and
  the timeout 30 seconds (E2E-044: 1). **Obliged by:** SPEC-0089 section 9 header. **Rationale:** a stub
  service would skip the reads under test. **Source:** spec. **Evidence:** `tools/artifacts.rs:2124-2136`.
- **DEC-027:** between US-0212 and US-0213 a stream whose chain holds `LZWDecode` or `ASCII85Decode` is
  armed unmeasured and counts 0, the cross reference streams are not measured, and an encrypted document is
  loaded by lopdf's encrypted loader as today; US-0213 removes the three gaps. **Obliged by:**
  SPEC-0089/FR-NEW-009 (no regression on the spec branch). **Rationale:** no story may leave today's text
  worse; the gaps exist only on the spec branch. **Source:** design. **Evidence:** n/a.

- **DEC-028:** E2E-003's stand in answers `# Deck` followed by 1 994 letters `d` (2 000 bytes in total):
  the spec's Given says "followed by 1 990 bytes" (1 996 in total) while its Then asserts 2 000 bytes
  written; the asserted figure is kept, the fixture (built by the design, spec section 9) is sized to it.
  Recorded, not blocking, in `design-issues.md` for the next amendment of the spec. **Obliged by:**
  SPEC-0089/FR-NEW-014 (E2E-003). **Rationale:** the observable behaviour (bytes written equal the
  answer's length) admits one reading; only the fixture arithmetic disagrees. **Alternatives considered:**
  holding the spec for a fixture typo; asserting 1 996 (changes an asserted value). **Source:** spec 9.2.
  **Evidence:** spec E2E-003.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0212 | bound every PDF stream the text pass decodes | P1 | SPEC-0089/FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009 | US-0211 | 1 | no |
| US-0213 | encrypted PDFs, cross reference, LZW and ASCII85 streams under the same limit | P1 | SPEC-0089/FR-NEW-002, FR-NEW-009 | US-0212 | 1 | no |
| US-0214 | the load total and the object limit | P1 | SPEC-0089/FR-NEW-001, FR-NEW-003, FR-NEW-006, FR-NEW-007 | US-0213 | 2 | no |
| US-0215 | the page total, repeats and saved states | P1 | SPEC-0089/FR-NEW-004, FR-NEW-006, FR-NEW-007 | US-0214 | 2 | no |
| US-0216 | the content limit for a PDF | P1 | SPEC-0089/FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-009 | US-0215 | 2 | no |
| US-0217 | PDF refusals in order, without trace, at both doors | P2 | SPEC-0089/FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-014 | US-0216 | 2 | no |
| US-0218 | cap the converter's text answer and stop it | P1 | SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013, FR-NEW-014 | US-0217 | 2 | no |
| US-0219 | diagnostic and bundle mode outputs held small | P1 | SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-014 | US-0218 | 2 | no |
| US-0220 | cap the web converter's answer | P1 | SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013 | US-0219 | 2 | no |
| US-0221 | converter refusals through the HTTP door and documentation | P2 | SPEC-0089/FR-NEW-010, FR-NEW-013, FR-NEW-014 | US-0220 | 2 | yes |

US-0212 is `min_tier` 1: the load deferral, the measurer, arm and defuse and the walk must all exist
before any exploit test is meaningful, the walk mirrors a library of 2 400 lines, and no thinner slice is
verifiable on its own. US-0213 is `min_tier` 1: the encrypted path mirrors lopdf's loader with
documented differences and the cross reference guard reproduces lopdf's discovery.
