# US-0212: bound every PDF stream the text pass decodes

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0211
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
A PDF one of whose streams (page content, object stream, font file, character map, colour profile, form or image) decodes to more than 104 857 600 bytes is refused before the reader inflates it, and the title is no longer read by loading the document a second time. This builds the module that drives the PDF text pass (deferred object streams, a streaming measurer, defuse and arm, the page walk), the measurement hook and the PDF fixture builders the rest of the spec uses, with the six stream bombs and the title bomb red first.

## Requirements covered
- SPEC-0089/FR-NEW-002: IF a stream of a PDF decodes to more than 104 857 600 bytes THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF stream decodes to more than 104857600 bytes`.
- SPEC-0089/FR-NEW-006 (partial: the bound on any one decoded stream): THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- SPEC-0089/FR-NEW-007 (partial: item 5, the title read under the same limits): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.
- SPEC-0089/FR-NEW-008 (partial: nothing left after a stream limit refusal): THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-005.
- SPEC-0089/FR-NEW-009 (partial: a stream in an encoding the extraction does not decode, answered as before): THE document text extraction SHALL answer a PDF within every limit with the same text, page markers, page count, title, note, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading the PDF when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_sec_001 e2e_0089_sec_002 e2e_0089_sec_003 e2e_0089_sec_004 e2e_0089_sec_005 e2e_0089_sec_006 e2e_0089_sec_012 e2e_0089_007 e2e_0089_008 e2e_0089_017 e2e_0089_032 zlib_run_matches_flate2`: 12 tests pass. Real assertions: `decoded_held` reads at most 104 933 136 for each bomb (the measurer holds one chunk, the walk never decodes a bomb for parsing), the listing of `/docs`, the artifact lists, the audit trail and a following 52 428 800 byte write show nothing was written or charged, and E2E-017 matches the answer today's code gave, recorded before the change. The existing PDF tests (`cargo test -p mcp-fs-core --lib docs::extract tools::artifacts`) still pass.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0089):** the PDF bound is built by measuring then letting the reader decode, in a new module `docs/pdf_bound.rs` that drives the load and the page loop itself with lopdf's and pdf-extract's public pieces. **Obliged by:** SPEC-0089/FR-NEW-001 to FR-NEW-007, spec 3.3 and 14.3. **Rationale:** the libraries expose no decode cap, but expose the load filter before object stream inflation, `ObjectStream::new`, the no `Filter` copy path and the page by page entry with any `OutputDev`; together they let the caller decide every inflation, in safe Rust, with no new crate. **Evidence:** `lopdf-0.42.0/src/reader.rs:821`, `:889`, `:959-965`; `src/object_stream.rs:40-42`; `src/object.rs:794-815`, `:849-872`; `src/load_options.rs`; `pdf-extract-0.12.1/src/lib.rs:6`, `:1255-1261`, `:2117-2122`, `:2389-2421`.
- **DEC-002 (SPEC-0089):** the limits are `pub(crate) const` in `pdf_bound.rs` (`STREAM_LIMIT` 104 857 600, `LOAD_LIMIT` 268 435 456, `PAGE_LIMIT` 268 435 456, `OBJECT_LIMIT` 1 000 000), the content limit is `extract::CONTENT_LIMIT`, the chunk 65 536; no configuration, no test seam that scales them.
- **DEC-003 (SPEC-0089):** `measure(stream)` mirrors `Stream::decompressed_content` (`object.rs:794-815`) step by step and counts, streaming, what each step outputs, holding one 65 536 byte chunk: filters in the order of `filters()`; `FlateDecode` through `flate2::Decompress::new(true)`, and when that zlib pass yields no byte the raw pass `Decompress::new(false)` over `input[2..]` is measured too, whether or not the zlib pass reported an error (an over count is safe, an under count is not); an unknown filter ends the chain: the steps before it count, and a chain whose first filter is unknown counts 0; a PNG predictor (`Predictor` 10 to 15 in `DecodeParms`) adds its two row buffers, `2 * Colors * max(8, BitsPerComponent) / 8 * Columns`, and its output counted as its input size (`object.rs:929-948`, `filters/png.rs:87-95`). The decoded size of a stream is the sum over its steps; for one `FlateDecode` and no predictor it is exactly the inflated length. The stream limit is checked after every chunk on the running size, then the load or page total, so decoding stops at most one chunk past a limit. Equal is accepted. In this story a stream with `LZWDecode` or `ASCII85Decode` in its chain is armed unmeasured and counts 0 (DEC-027).
- **DEC-004 (SPEC-0089), this story's part:** the load is `Document::load_mem_with_options(data, LoadOptions { filter: Some(defer_object_streams), ..Default::default() })`; `defer_object_streams` renames the `Type` of a stream typed `ObjStm` with a non empty content to `McpFsObjStm` and keeps it (an empty one is left alone, lopdf reads its content later from an indirect `Length`, `reader.rs:985-990`, `:1025-1027`). Then, in increasing object number, each deferred stream is measured (stream limit), decoded with `decompressed_content()`, parsed with `ObjectStream::new` on the decoded bytes set as plain content, and its objects inserted as lopdf inserts them: kept only when the cross reference does not place that number in another container, never replacing an object already loaded (`reader.rs:968-984`, `:1020-1022`); a stream that fails to parse is removed from the document (`.ok()?`, `:965`); the container's content is then emptied and its `Type` restored. A load error keeps today's message. The load total and the object limit are US-0214.
- **DEC-006 (SPEC-0089):** after the load every stream object's `Filter` key is renamed `McpFsFilter` (defuse); per page the walk's measured set gets `Filter` back (arm) before `output_doc_page` and loses it after (disarm), on every exit path. Anything the walk did not reach yields its stored bytes, as an undecodable stream does today (`object.rs:794-799`, `lib.rs:1256-1260`, `document.rs:593-596`).
- **DEC-007 (SPEC-0089):** the walk mirrors pdf-extract's decode sites for one page, iteratively (an explicit stack, no recursion in our code): resources inherited through `Parent` (`lib.rs:2352-2362`); the page's content streams (`Document::get_page_contents`) measured, decoded, parsed with `Content::decode` (the decoded bytes dropped once parsed, the operations kept); `Tf`: the first selection of a font name on the page measures, by the font's `Subtype` as `make_font` dispatches (`lib.rs:322-332`): `Type0` its `Encoding` when a stream (`:990`) and `ToUnicode` (`:1002`, `:926`); `Type3` its `ToUnicode` (`:695`); any other its descriptor's `FontFile` when `Type1`, `FontFile2` when `TrueType`, `FontFile3` when present (`:386-411`), and `ToUnicode` (`:454`, `:458`); `cs`/`CS` on a name other than the four device names: the colour space array's `ICCBased` stream, a `Separation`'s alternate `ICCBased` stream and its tint transform when it is a stream of `FunctionType` 0 or 4 (`lib.rs:1449-1525`, `:1168-1225`); `Do`: the XObject stream, whatever its subtype, measured, decoded and parsed, its own `Resources` or the caller's (`:1860-1867`). Where pdf-extract would panic (a missing resource, an operation that does not parse), the walk stops that branch without refusing and the reader then panics as today. A form already on the walk stack stops that branch.
- **DEC-011 (SPEC-0089):** the title is read from the loaded document's trailer `/Info` once the pages are done, by the same code as today's `pdf_title` minus its second load (`extract.rs:538-548`).
- **DEC-012 (SPEC-0089):** measurement hook, test only: `pdf_bound::extract` stores `PdfHeld { decoded, content }` in the thread local `LAST_PDF_HELD` on every outcome, a panic included (a drop guard owns the figures); `Extractor::extract` moves it into `PDF_HELD[path]` beside the Office move (`extract.rs:183-185`). `decoded` is the largest, over the call, of: the decoded object stream being parsed (load); the decoded content or form buffer the walk holds while parsing it (one at a time); while the reader runs a page, that page's decode count (the sum of the measured sizes of what it may decode). `content` is the content buffer's largest length. The measuring chunk (65 536) is outside the figure. A call that decodes nothing reads 0; a cached call leaves no entry.
- **DEC-013 (SPEC-0089):** refusals are `ToolError::invalid_argument` with the messages without prefix; a load error is `pdf: PDF error: <e>`, a panic anywhere in the closure is `pdf parser failed on this document` (one `catch_unwind(AssertUnwindSafe(..))`, as today). Order: load, object streams, then pages in order (walk: stream limit per decode in drawing order; then the reader), then the title.
- **DEC-018 (SPEC-0089):** test layout: `crates/core/src/tools/pdf_bound_e2e.rs`, single threaded `#[tokio::test]` through `core::fs_ops::extract_document`; every test that builds or decodes 10 MB or more takes `archive_testkit::heavy()` first.
- **DEC-019 (SPEC-0089), this story's part:** fixtures in `crates/core/src/tools/pdf_testkit.rs`: `zlib_run(prefix, letter, n, suffix) -> Vec<u8>` (bytes `0x78 0x9C`, then `archive_testkit::deflate_run`'s raw deflate, then the Adler 32 of the content computed with the closed form for a run of one byte, big endian); `bomb_stream(dict_extra) -> (String, Vec<u8>)` (dictionary text and stored bytes of a stream of exactly 200 000 000 spaces, `/Length` its real stored length, `/Filter /FlateDecode`); `PdfWriter` (objects by number, `stream` helper, a classic cross reference, or a cross reference stream with uncompressed `/W [1 4 2]` entries when compressed objects are present; `object_stream(number, objects: &[(u32, &str)], padding: usize)` writing `n off` pairs, `/N`, `/First`, the objects, then `padding` spaces, deflated through `zlib_run`); `page_pdf(content, resources, extra)` for one page documents with a `Helvetica` `/F1`; `pdf_held_reset()`, `decoded_held(path)`, `pdf_content_held(path)`.
- **DEC-020 (SPEC-0089):** E2E-017's expected answer is recorded before any production change: step 0 builds `/docs/photo.pdf`, runs today's `extract_document`, and writes the observed outcome (success with its exact preview, or the code and message) as a literal in the test, quoted in the commit body.
- **DEC-027 (SPEC-0089):** until US-0213 a stream whose chain holds `LZWDecode` or `ASCII85Decode` is armed unmeasured and counts 0, the cross reference streams are not measured, and an encrypted document is loaded by lopdf's encrypted loader as today.
### Data model
In memory only: `PdfBudget { held: u64, live: u64, page: u64, page_no: u32, content: u64 }` (this story uses `held`, `live` and the per page set; the load total, object count and page total checks are US-0214 and US-0215); per page `armed: BTreeSet<ObjectId>`, `sizes: HashMap<ObjectId, u64>`, `forms: HashMap<ObjectId, Rc<Vec<Operation>>>`, `fonts: HashSet<Vec<u8>>`; `#[cfg(test)] PdfHeld { decoded: u64, content: u64 }`, thread locals `LAST_PDF_HELD: Cell<Option<PdfHeld>>` and `PDF_HELD: RefCell<HashMap<String, PdfHeld>>` in `docs/extract.rs`.
### Contracts
`crates/core/src/docs/pdf_bound.rs` (new, `pub(crate) mod pdf_bound;` in `docs/mod.rs`): `pub(crate) const STREAM_LIMIT: u64 = 104_857_600;`, `pub(crate) struct PdfText { pages: Vec<String>, body: String, title: Option<String> }`, `pub(crate) fn extract(data: &[u8]) -> Result<PdfText>`, `fn defer_object_streams(id: ObjectId, object: &mut Object) -> Option<(ObjectId, Object)>`, `fn measure(stream: &Stream, budget: &mut PdfBudget, scope: Scope) -> Result<u64>`. Refusal: `ToolError::invalid_argument("a PDF stream decodes to more than 104857600 bytes")`; `prepare_extraction` adds `could not extract <path>: ` (`extract.rs:373-381`, unchanged).
`crates/core/src/docs/extract.rs`: `fn pdf(data: &[u8]) -> Result<ExtractResult>` keeps its signature and result shape (`format` `pdf`, meta `pages` = `PdfText::pages.len()`, meta `title` when not empty, the note `no extractable text layer (scanned PDF?); enable a multimodal OCR provider` when the body is empty); its page text assembly in this story is today's (`\n\n---\n*[Page N]*\n\n` plus the trimmed page, the whole trimmed, `extract.rs:518-522`); `pdf_title` is removed (DEC-011). Each page runs `pdf_extract::output_doc_page(&doc, &mut PlainTextOutput::new(&mut page_text), n)` exactly as `extract_text_by_page` does (`lib.rs:2284-2291`), and the loop stops at the first page error as `extract_text_from_mem_by_pages` does (`lib.rs:2323-2333`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | create |
| `crates/core/src/docs/mod.rs` | modify |
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/pdf_testkit.rs` | create |
| `crates/core/src/tools/pdf_bound_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] mod pdf_testkit; #[cfg(test)] mod pdf_bound_e2e;`) |

`crates/core/src/tools/archive_testkit.rs` is used, not modified: `MOUNT`, `OWNER`, `seed`, `read`, `office_fixture` (project `atlas`, member `mia@example.com`, folder `/docs`), `extract_text(f, person, path, refresh)`, `deflate_run` (`:379`), `heavy` (`:451`), `nothing_left` (`:522`).
### Existing patterns
Test module model: `crates/core/src/tools/office_bound_e2e.rs` (layout, `part_refusal` style helper, `heavy()` first). Hook model: `OfficeHeld`, `LAST_OFFICE_HELD`, `OFFICE_HELD` (`extract.rs:580-596`) and their readers in `archive_testkit.rs:505-520`. Fixture model: `tiny_pdf` (`extract.rs:1975-2025`, offsets and classic cross reference by hand). Segmented deflate: `deflate_run` (`archive_testkit.rs:379-400`). Library entry points: `output_doc_page` (`pdf-extract-0.12.1/src/lib.rs:2389-2399`), `PlainTextOutput` (`:2131-2190`), `Document::get_page_contents` (`lopdf-0.42.0/src/document.rs:533`), `ObjectStream::new` (`src/object_stream.rs:40`).
### Fixtures
A PDF bomb stream is `bomb_stream(..)`: exactly 200 000 000 spaces, zlib, stored in under 300 000 bytes; each bomb document is asserted under 1 000 000 bytes once in a `pdf_testkit` unit test `bomb_documents_are_small`. SEC-001: page 1 `/Contents` is the bomb stream, dictionary `<< /Length <stored> /Filter /FlateDecode >>`. SEC-002: object 5 `<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length <stored> >>` holding `1 0 ` then `<< /Pad true >>` then spaces to 200 000 000 bytes (`object_stream` with `padding`), object 1 listed as compressed in a cross reference stream. SEC-003: `/F1 << /Type /Font /Subtype /Type1 /BaseFont /Bomb /FontDescriptor 6 0 R >>`, object 6 `<< /Type /FontDescriptor /FontFile 7 0 R >>`, object 7 the bomb stream; content `BT /F1 12 Tf 72 700 Td (Hi) Tj ET`. SEC-004: `/F1` with `/ToUnicode 7 0 R`, object 7 the bomb stream. SEC-005: content `/CS0 cs`, resources `/ColorSpace << /CS0 [/ICCBased 7 0 R] >>`, object 7 the bomb stream with `/N 1`. SEC-006: content `/Im0 Do`, `/XObject << /Im0 7 0 R >>`, object 7 `<< /Subtype /Image /Width 10000 /Height 20000 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode /Length <stored> >>` over the bomb. SEC-012 and E2E-032: `/docs/titled.pdf`, the two page document of E2E-001 (page 1 `Quarterly results`, page 2 `Outlook`) whose `/Info` dictionary `<< /Title (Report 2026) >>` is object 8 inside object stream 9, a bomb (`object_stream(9, &[(8, "<< /Title (Report 2026) >>")], 200_000_000 - head)`). E2E-007: page 1 content `BT /F1 12 Tf 72 700 Td (` + 250 000 letters `b` + `) Tj ET` + spaces, exactly 104 857 600 bytes, through `zlib_run`; E2E-008 the same, 104 857 601 bytes. E2E-017: page 1 content `BT /F1 12 Tf 72 700 Td (Caption) Tj ET` then `/Im0 Do` 200 times, `/Im0` a 900 000 byte image whose `/Filter` is `/DCTDecode` (bytes: `0xFF 0xD8` then 899 998 letters `x`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-001: Page content stream bomb
- **Covers:** class sweep occurrence 2 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the whole page content, 200 000 000 bytes, is inflated before anything is checked.
- **Given** `/docs/bomb.pdf` whose page 1 content is `<< /Length 194000 /Filter /FlateDecode >> stream` followed by the compressed form of 200 000 000 spaces, then `endstream` (the `/Length` is the built stream's real stored length)
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_001_page_content_stream_bomb`
- Red before the change because: step 1 (pipeline and hook, no limit): the walk decodes the content to parse it, `decoded_held` is 200 000 000 and the call succeeds with the no text layer note
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-002: Compressed object stream bomb
- **Covers:** occurrence 1
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the object stream is inflated whole when the document is loaded.
- **Given** `/docs/objstm.pdf` whose object stream `<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode >>` holds `1 0 ` then a dictionary padded to 200 000 000 bytes with spaces, compressed
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/objstm.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_002_compressed_object_stream_bomb`
- Red before the change because: after step 1 the load decodes the object stream, `decoded_held` is 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-003: Font file bomb
- **Covers:** occurrence 3
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the font file is inflated whole when page 1 selects the font.
- **Given** `/docs/font.pdf` whose page 1 shows `(Hi) Tj` with font `/F1`, whose font descriptor's `/FontFile` is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/font.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_003_font_file_bomb`
- Red before the change because: after step 1 the walk arms the font file, the page's decode count is 200 000 000 when the reader runs (the reader may then panic parsing 200 000 000 spaces as a Type 1 font: the drop guard still records the figure), so `decoded_held` is 200 000 000 and the answer is not the stream limit refusal
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-004: Character map bomb
- **Covers:** occurrence 4
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the character map is inflated whole.
- **Given** `/docs/cmap.pdf` whose font `/F1` used on page 1 has a `/ToUnicode` that is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/cmap.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_004_character_map_bomb`
- Red before the change because: after step 1 `decoded_held` is 200 000 000 (the page's decode count when the reader runs) and the answer is not the stream limit refusal
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-005: Colour profile bomb
- **Covers:** occurrence 5
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the colour profile is inflated whole when page 1 runs `/CS0 cs`.
- **Given** `/docs/icc.pdf` whose page 1 runs `/CS0 cs` with `/CS0 [/ICCBased 7 0 R]`, object 7 being a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/icc.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_005_colour_profile_bomb`
- Red before the change because: after step 1 `decoded_held` is 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-006: Image bomb drawn once
- **Covers:** occurrence 6
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the image is inflated whole when page 1 runs `/Im0 Do`.
- **Given** `/docs/image.pdf` whose page 1 runs `/Im0 Do`, `/Im0` being `<< /Subtype /Image /Width 10000 /Height 20000 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode >>` over a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/image.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_006_image_bomb_drawn_once`
- Red before the change because: after step 1 the walk decodes the image to parse it as the reader does, `decoded_held` is 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-012: Title read decoding again
- **Covers:** occurrence 12
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the title read loads the document again and inflates its object streams whole.
- **Given** `/docs/titled.pdf` of E2E-032
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/titled.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_012_title_read_decoding_again`
- Red before the change because: after step 1 the load decodes object stream 9, `decoded_held` is 200 000 000 and the call succeeds with title `Report 2026`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-007: Stream of exactly the stream limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** Critical
- **Given** `/docs/large.pdf` whose page 1 content stream inflates to exactly 104 857 600 bytes, showing the letter `b` repeated, then spaces
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated, its preview made of the letter `b`
- **And** the decoded bytes held never exceed 104 923 136 plus the load's decoded object streams (under 10 000 bytes): `decoded_held` at most 104 933 136
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_007_stream_of_exactly_the_stream_limit`
- Green on arrival: after step 1 nothing refuses yet, so the test passes; then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-008: Stream one byte over the stream limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-006, FR-NEW-008; **Priority:** Critical
- **Given** `/docs/large.pdf` as in E2E-007 but inflating to 104 857 601 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/large.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_008_stream_one_byte_over_the_stream_limit`
- Red before the change because: after step 1 the extraction succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-017: A stream the extraction does not decode
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-009; **Priority:** Medium
- **Given** `/docs/photo.pdf` whose page 1 shows `Caption` and draws, 200 times, a 900 000 byte image in an encoding the extraction does not decode (`/DCTDecode`)
- **When** the member asks for its text
- **Then** the answer, success or refusal, is identical to the one the server gives before this change for the same file, recorded in step 0 (DEC-020); it is not one of the five limit refusals of this specification
- **And** the decoded bytes held never exceed 10 000 (the image is not decoded, so it is not counted)
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_017_a_stream_the_extraction_does_not_deco`
- Green on arrival: the literal is today's answer, recorded in step 0 on main's code; then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-032: Title read under the same limits
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/titled.pdf` of E2E-001 whose information dictionary, holding the title, sits in a compressed object stream that is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message for `/docs/titled.pdf`; no title is reported
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_032_title_read_under_the_same_limits`
- Red before the change because: after step 1 the extraction succeeds and reports title `Report 2026`
- Cleanup: none, the fixture is dropped with the in memory store

### Design test: zlib framing of the cached segments
- Level / Driver / Location: unit / direct call / crates/core/src/tools/pdf_testkit.rs
- Function: `zlib_run_matches_flate2`
- Given `zlib_run("<<", b' ', 2_500_000, ">>")` / When it is inflated with `flate2::read::ZlibDecoder` / Then the output is `<<`, 2 500 000 spaces, `>>`, and no error is reported (the Adler 32 checks)
- Red before the change because: the builder does not exist (compile failure), then fails until the checksum is right

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-007 | in `measure`, refuse when the running size equals the limit (`>=` for `>`): the extraction is refused, the test fails |
| E2E-017 | in `measure`, count a chain whose first filter is unknown at its stored length instead of 0: the page's decode count is 180 000 000 when the reader runs, `decoded_held` passes 10 000, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs` (its helpers are used as they are), the three PDF tests of `docs/extract.rs` and `tiny_pdf` (their code stays as it is)
- Dependencies not to add: none (no `Cargo.toml` change; `weezl` is US-0213)
- The hook is test only (`#[cfg(test)]`) and set on the calling thread; keep every test on the default single threaded `#[tokio::test]`
- No `unsafe`; the load filter is a plain `fn` (no captured state); no recursion in the walk
- No dash as punctuation in code, comments or docs
- Scope boundary: LZW, ASCII85, the cross reference guard and encrypted files are US-0213; the load total and object limit US-0214; the page total and colour profile stand ins US-0215; the content limit US-0216

## Definition of done
- [ ] Step 0 recorded before any production code: E2E-017's fixture run on main's code, its outcome written as the test's literal and quoted in the commit body (DEC-020).
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-004, E2E-SEC-005, E2E-SEC-006, E2E-SEC-012, E2E-008, E2E-032, `zlib_run_matches_flate2`), before the limit (step 1: `pdf_testkit`, the test module, `pdf_bound` with deferral, measurer, defuse and arm, walk, title from the loaded document and the hook, `measure` never refusing; run the tests under `heavy()`, one at a time: each red run decodes 200 000 000 bytes; step 2: the stream limit in `measure`).
- [ ] Green on arrival tests (E2E-007, E2E-017): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::artifacts tools::document` pass (`pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion` unchanged); every `e2e_0088_` test passes; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (US-0217 and the closing story US-0221 carry them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; arm and defuse restored on every exit path (a guard); failure modes of the new code listed in the commit body.
- [ ] One commit `US-0212: bound every PDF stream the text pass decodes` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009).
