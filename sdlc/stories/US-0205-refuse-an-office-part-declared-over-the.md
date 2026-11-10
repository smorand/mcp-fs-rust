# US-0205: refuse an Office part declared over the part limit

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0204
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
A Word, Excel or PowerPoint document whose part declares more than 104 857 600 bytes is refused before that part is decoded, with a message naming the part and its exact declared size, whatever that size, for every part the readers read. This builds the one bounded part reader, the per call budget, the measurement hook and the fixture builders the rest of the spec uses, with every exploit test of the decoding sites red first.

## Requirements covered
- SPEC-0088/FR-NEW-001: THE document text extraction SHALL decode no part of a Word, Excel or PowerPoint document beyond 104 857 600 bytes.
- SPEC-0088/FR-NEW-002: IF a part about to be read declares more than 104 857 600 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: document part '<part>' is <D> bytes once decoded, over the limit of 104857600 bytes`, before decoding that part.
- SPEC-0088/FR-NEW-006 (partial: the bound on any one decoded part and 0 held for a part refused on its declared size): THE document text extraction SHALL hold, during one call, at most 104 857 600 plus 65 536 bytes of any one decoded part, at most 268 435 456 plus 65 536 bytes of decoded part data in total, and at most 373 293 056 bytes of extracted content.
- SPEC-0088/FR-NEW-007 (partial: nothing left after a part limit refusal): THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-002 to FR-NEW-005.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_sec_001 e2e_0088_sec_002 e2e_0088_sec_003 e2e_0088_sec_004 e2e_0088_sec_005 e2e_0088_sec_006 e2e_0088_sec_007 e2e_0088_sec_008 e2e_0088_sec_009 e2e_0088_sec_010 e2e_0088_sec_012 e2e_0088_sec_013 e2e_0088_006 e2e_0088_007`: 14 tests pass. Real assertions: `parts_held` reads 0 for a refusal on the first part read (no decoder ran) and at most 65 536 otherwise; the listing of `/docs`, the artifact lists, the audit trail and a following 52 428 800 byte write show nothing was written or charged.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0088):** one reader, `OfficeBudget::read_part(zip, name) -> Result<Option<String>>`, replaces `entry_text` at every call site: `index_for_name(name)` (absent: `Ok(None)`), declared size from `by_index_raw(i)?.size()` (central directory, `u64`, no decompressor built), part limit then call limit (DEC-003), then `by_index(i)` and `read_capped(file, declared)`; any open error other than a refusal stays `Ok(None)` as today. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-008. **Rationale:** one fix site covers occurrences 1 to 10; the declared size is known before any byte is decoded, so a refusal holds 0 bytes; `index_for_name` is what `by_name` uses (`zip-9.0.5/src/read/zip_archive.rs:545-551`), so the parts found are the same. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:563-568`, `crates/core/src/tools/archive_single.rs:92-108`, `zip-9.0.5/src/read/zip_archive.rs:551`, `:699`.
- **DEC-002 (SPEC-0088):** the limits are `pub(crate) const` in `extract.rs` (`PART_LIMIT` 104 857 600, `CALL_LIMIT` 268 435 456, `CONTENT_LIMIT` 268 435 456), not configuration, and there is no test seam that scales them: every acceptance test runs against the real limits with payloads as small as the limits permit. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-005, spec D-4. **Rationale:** the refusal messages and the held figures the spec asserts are literal (`104857600`, `268435456`, `104 923 136`, `268 500 992`, `373 293 056`); a scaled limit would assert a different message and prove nothing about the shipped one. The cost is bounded by `heavy()` and the cheap fixture builder (DEC-011). **Alternatives considered:** a `#[cfg(test)]` limits struct passed down (rejected: changes the asserted messages, adds a parameter to every reader). **Source:** spec. **Evidence:** spec section 9.
- **DEC-003 (SPEC-0088):** arithmetic: declared sizes are `u64` compared unsigned with `>` (equal accepted); the call total is a `u128` (`declared + size as u128 > CALL_LIMIT as u128`), so `18446744073709551615` never wraps; the part limit is checked before the call limit for the part about to be read; the size enters the total at its check, before decoding (a part read twice counts twice; a part that then reads as absent still counted). Messages print the `u64` with `{}`. **Obliged by:** SPEC-0088/FR-NEW-002 business rule, FR-NEW-003 business rule, FR-NEW-009 item 4. **Rationale:** exact digits and no wrap for the two largest declarable sizes (E2E-SEC-012, E2E-SEC-013). **Alternatives considered:** `checked_add` on `u64` (equivalent, one more branch). **Source:** spec 14.3. **Evidence:** n/a.
- **DEC-005 (SPEC-0088):** refusals are `ToolError::invalid_argument` with the messages of section 5 and no prefix; `prepare_extraction` keeps adding `could not extract <path>: ` (`extract.rs:366-374`), so both doors and every format share the text; the existing `docx: word/document.xml is missing` and not a zip messages are unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-010, spec D-5. **Rationale:** one wrapper, no new code. **Alternatives considered:** a new error code (spec D-5 rules it out). **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:366-374`.
- **DEC-006 (SPEC-0088):** propagation and reading order: `docx` uses `read_part(..)?` then the missing message on `None`; `pptx_slide_order` returns `Result<Vec<String>>` (presentation part or its relationships absent: numeric fallback as today; refused: the refusal); `pptx_notes_part` returns `Result<Option<String>>`; `xlsx_shared_strings` and `xlsx_sheets` return `Result<..>` (the workbook relationships refusal is no longer swallowed by `unwrap_or_default`); a slide or sheet that reads as absent is still skipped with `continue`. The reading order of spec 2.1 is unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-009 item 4, FR-NEW-011. **Rationale:** a refusal must not degrade into "absent" (E2E-SEC-004 would otherwise fall back to the numeric order and succeed). **Alternatives considered:** none. **Source:** existing code. **Evidence:** `extract.rs:716-717`, `:868-881`, `:894-930`, `:936-942`, `:985-1048`.
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-009 (SPEC-0088):** a decoded part becomes its `String` with `String::from_utf8(bytes)` (no copy), and only an invalid UTF-8 part goes through `String::from_utf8_lossy` as today. **Obliged by:** SPEC-0088/FR-NEW-006 (spec 14.3 asks whether the copy counts), FR-NEW-011. **Rationale:** a valid part is then held once, so the held figure is what is held; the lossy repair copy of an invalid part is not counted and is filed (BL-0145). **Alternatives considered:** counting the copy (the figure would double for every valid part, which never copies); refusing invalid UTF-8 (changes today's text, FR-NEW-011). **Source:** design. **Evidence:** `extract.rs:418-420`.
- **DEC-011 (SPEC-0088):** fixtures, all in `crates/core/src/tools/archive_testkit.rs`: `office_fixture()` and `office_fixture_allowance(n)` (project `atlas`, member `mia@example.com`, folder `/docs`); `THIRD` = `ola@example.com` with `add_third_member`; `extract_text(f, person, path, refresh)`; `office_zip(parts: &[(&str, &[u8])])` (zip writer, deflate); `zip_raw(entries: &[RawPart])` writing given compressed bytes, checksum and declared size (`zip_declaring` becomes a thin wrapper over it, same output); `deflate_run(prefix, letter, n, suffix) -> (Vec<u8>, flate2::Crc)` building the deflate stream of `prefix + letter * n + suffix` from one per process cached segment of 1 000 000 letters (each segment compressed by its own `flate2::Compress` with `FlushCompress::Full`, so segments end on a byte boundary, reference nothing before them and concatenate into one valid stream; the checksum is combined with `Crc::combine`), so a 200 000 000 byte part costs about 200 KB and no 200 MB compression; `bomb_part(name) -> RawPart` (exactly 200 000 000 bytes, markup included: the letter `a` as text for `word/document.xml`, slides, notes and `xl/sharedStrings.xml`; as the value of a root attribute `x` for `ppt/presentation.xml`, every `.rels` part, `xl/workbook.xml` and worksheets); `padded_part(name, xml_head, total)` (a part of an exact size padded through a root attribute, for E2E-006 to E2E-011); `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when no entry); `nothing_left(f, doc, companion_before)`; `heavy()` returning the guard of a process wide `tokio::sync::Mutex<()>`, taken first by every test building or decoding a part of 10 MB or more (each can hold about 1 GB; in parallel they would exhaust memory). **Obliged by:** SPEC-0088 section 9 (fixtures built by the design), FR-NEW-002, FR-NEW-006. **Rationale:** no library writes a lying or zip64 header; compressing 200 MB per test in an unoptimized debug build would cost seconds each. **Alternatives considered:** committed fixture files (binary blobs for 40 cases, unreadable); `flate2::write::DeflateEncoder` per test (slow); a dev profile that optimizes dependencies (BL-0137). **Source:** design. **Evidence:** `crates/core/src/tools/archive_testkit.rs:236-305` (`zip_declaring`), `crates/core/src/docs/zip_check.rs:44-120` (`craft::rename_local`, reused by E2E-022).
- **DEC-017 (SPEC-0088):** the ASSUMED zip64 short part (spec 14.3, E2E-SEC-012, E2E-SEC-013): the first step of US-0205 builds both fixtures with `zip_declaring` and runs today's code. If the extraction succeeds on the 5 real bytes, the spec's red reason holds. If the zip reader refuses the short read, the part reads as absent: still red (the message differs), and the fix makes it green because the refusal comes before decoding. If `ZipArchive::new` or the SPEC-0041 header check refuses the package, that refusal is the control: record it in the commit body, keep the test as a guard asserting the observed refusal (`not a valid OOXML package:` prefix) and 0 held, and report it in the story's DONE_WITH_CONCERNS. **Obliged by:** SPEC-0088/FR-NEW-002 (E2E-SEC-012, E2E-SEC-013). **Rationale:** the spec's own rule for an ASSUMED fixture. **Alternatives considered:** none. **Source:** spec 14.3. **Evidence:** `zip-9.0.5/src/crc32.rs:43-51` (only the excess is checked).
### Data model
In memory only: `OfficeBudget { declared: u128, decoded: u64, content: u64, content_limit: Option<u64> }` (this story uses `declared` only for the part being read and `decoded`; the call total is US-0206, the content counter US-0208); `#[cfg(test)] OfficeHeld { parts: u64, content: u64 }`, thread locals `LAST_OFFICE_HELD: Cell<Option<OfficeHeld>>` and `OFFICE_HELD: RefCell<HashMap<String, OfficeHeld>>`.
### Contracts
`crates/core/src/docs/extract.rs`: `pub(crate) const PART_LIMIT: u64 = 104_857_600;` (`CALL_LIMIT` and `CONTENT_LIMIT` arrive with their stories); `struct OfficeBudget` with `fn office() -> Self` and `fn read_part(&mut self, zip: &mut Zip, name: &str) -> Result<Option<String>>`: `index_for_name` (absent: `Ok(None)`), `by_index_raw(i)` then `size()` (error: `Ok(None)`), `size > PART_LIMIT` refuses with `ToolError::invalid_argument(format!("document part '{name}' is {size} bytes once decoded, over the limit of 104857600 bytes"))`, then `by_index(i)` (error: `Ok(None)`) and `archive_single::read_capped(file, size)`: `Ok` adds `bytes.len()` to `decoded` and returns `String::from_utf8(bytes)` (lossy fallback on invalid UTF-8, DEC-009); `CapExceeded` and every `Io` error read as absent, `Ok(None)` (the size lie refusal is US-0207). `entry_text` is removed; every caller uses `budget.read_part(..)?` with the propagation of DEC-006. `fn office(data: &[u8], read: impl FnOnce(&mut Zip, &mut OfficeBudget) -> Result<ExtractResult>) -> Result<ExtractResult>`; `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` and call it. Hook (DEC-008): `office` stores `OfficeHeld { parts: decoded, content }` in `LAST_OFFICE_HELD` on every outcome; `Extractor::extract` moves it into `OFFICE_HELD[filename]` right after the reader returns. Messages are wrapped by `prepare_extraction` into `could not extract <path>: ...` (unchanged code).
Testkit (`crates/core/src/tools/archive_testkit.rs`, DEC-011): `office_fixture`, `office_fixture_allowance`, `THIRD`, `add_third_member`, `extract_text`, `office_zip`, `RawPart { name, declared: u64, crc: u32, data: Vec<u8> }` and `zip_raw` (with `zip_declaring` rewritten as a wrapper giving the same bytes), `deflate_run`, `bomb_part`, `padded_part`, `office_held_reset`, `parts_held`, `content_held`, `nothing_left`, `heavy`. Module `#[cfg(test)] mod office_bound_e2e;` in `crates/core/src/tools/mod.rs`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/office_bound_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`). Bounded read and its hook: `read_capped` and `LAST_HIGH_WATER` (`crates/core/src/tools/archive_single.rs:35-108`); the zip size lie text matched by `zip_read_error` (`crates/core/src/tools/archive.rs:571-578`). `craft::rename_local` (`crates/core/src/docs/zip_check.rs:100`) for later stories.
### Fixtures
Payloads (DEC-011): each bomb part is `bomb_part(<name>)`, declaring and holding exactly 200 000 000 bytes, markup included; the other parts are well formed and under 2 000 bytes (`office_zip`); the whole document stays under 1 000 000 bytes (assert it once per builder in a testkit unit test `bomb_documents_are_small`). E2E-SEC-012: `zip_declaring(&[("word/document.xml", 9_223_372_036_854_775_808, b"hello")])`; E2E-SEC-013: `zip_declaring` with `xl/sharedStrings.xml` declaring `u64::MAX` and holding `hello`, plus a workbook, its relationships and one sheet declaring their real sizes. E2E-006: a text run, `deflate_run("<w:document><w:body><w:p><w:r><w:t>", b'b', n, "</w:t></w:r></w:p></w:body></w:document>")` with `n` chosen so the part is exactly 104 857 600 bytes; E2E-007 the same with one more letter. Take `heavy()` first in all 14 tests.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-SEC-001: Word main part bomb
- **Covers:** class sweep occurrence 1 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the whole part, 200 000 000 bytes, is decoded before anything is checked.
- **Given** `/docs/bomb.docx` whose `word/document.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.docx: document part 'word/document.xml' is <D> bytes once decoded, over the limit of 104857600 bytes` with `<D>` = `200000000`
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_001_word_main_part_bomb`
- Red before the change because: step 1 (hook counting today's `entry_text` output, no limit): `parts_held` is 200 000 000, not 0, and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-002: PowerPoint slide bomb
- **Covers:** occurrence 2
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today slide 2, 200 000 000 bytes, is decoded whole.
- **Given** `/docs/bomb.pptx` with slide 1 `Intro` and slide 2 `ppt/slides/slide2.xml` the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `/docs/bomb.pptx` and `ppt/slides/slide2.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_002_powerpoint_slide_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-003: PowerPoint notes bomb
- **Covers:** occurrence 3
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the notes part is decoded whole.
- **Given** `/docs/notes.pptx` with slide 1 `Intro` whose notes part `ppt/notesSlides/notesSlide1.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/notesSlides/notesSlide1.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_003_powerpoint_notes_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-004: Presentation part bomb
- **Covers:** occurrence 4
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the presentation part is decoded whole to find the slide order.
- **Given** `/docs/pres.pptx` whose `ppt/presentation.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/presentation.xml`
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_004_presentation_part_bomb`
- Red before the change because: after step 1, `parts_held` is 200 000 000, not 0, and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-005: Presentation relationships bomb
- **Covers:** occurrence 5
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the relationships part is decoded whole.
- **Given** `/docs/prels.pptx` whose `ppt/_rels/presentation.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/_rels/presentation.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_005_presentation_relationships_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-006: Slide relationships bomb
- **Covers:** occurrence 6
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the slide's relationships part is decoded whole while looking for its notes.
- **Given** `/docs/srels.pptx` with slide 1 `Intro` whose `ppt/slides/_rels/slide1.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/slides/_rels/slide1.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_006_slide_relationships_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-007: Excel sheet bomb
- **Covers:** occurrence 7
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the sheet is decoded whole.
- **Given** `/docs/sheet.xlsx` whose only sheet `xl/worksheets/sheet1.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/worksheets/sheet1.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_007_excel_sheet_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-008: Shared strings bomb
- **Covers:** occurrence 8
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the shared strings part is decoded whole, first of all parts.
- **Given** `/docs/shared.xlsx` whose `xl/sharedStrings.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/sharedStrings.xml`
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_008_shared_strings_bomb`
- Red before the change because: after step 1, `parts_held` is 200 000 000, not 0, and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-009: Workbook part bomb
- **Covers:** occurrence 9
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the workbook part is decoded whole.
- **Given** `/docs/book.xlsx` whose `xl/workbook.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/workbook.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_009_workbook_part_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-010: Workbook relationships bomb
- **Covers:** occurrence 10
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the relationships part is decoded whole.
- **Given** `/docs/brels.xlsx` whose `xl/_rels/workbook.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/_rels/workbook.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_010_workbook_relationships_bomb`
- Red before the change because: after step 1, `parts_held` is above 200 000 000 and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-012: Part declaring 2^63 bytes
- **Covers:** occurrence 12
- **Red before the fix because:** `Then the refusal is ...` fails: today the part reads its 5 real bytes and the extraction succeeds.
- **Given** `/docs/huge.docx` whose `word/document.xml` declares 9 223 372 036 854 775 808 bytes in its large size record and holds a 5 byte content, its checksum matching
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/huge.docx: document part 'word/document.xml' is 9223372036854775808 bytes once decoded, over the limit of 104857600 bytes`
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_012_part_declaring_2_63_bytes`
- Red before the change because: today the extraction succeeds on the 5 real bytes (the spec's reason); if the zip reader refuses the short read the main part reads as absent and the refusal is `docx: word/document.xml is missing` (still red); if the package itself is refused, apply DEC-017 (control, guard, concern)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-SEC-013: Part declaring the largest size
- **Covers:** occurrence 13
- **Red before the fix because:** `Then the refusal is ...` fails: today the extraction succeeds on the 5 real bytes.
- **Given** `/docs/max.xlsx` whose `xl/sharedStrings.xml` declares 18 446 744 073 709 551 615 bytes and holds 5 bytes, checksum matching
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/max.xlsx: document part 'xl/sharedStrings.xml' is 18446744073709551615 bytes once decoded, over the limit of 104857600 bytes`, the digits exact
- **And** nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_013_part_declaring_the_largest_size`
- Red before the change because: today the extraction succeeds (the spec's reason), or the shared strings read as absent and it still succeeds; a refused package follows DEC-017
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-006: Part of exactly the part limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-006; **Priority:** Critical
- **Given** `/docs/large.docx` whose main part declares and holds exactly 104 857 600 bytes, one paragraph of the letter `b`
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated, its preview made of the letter `b`
- **And** the decoded bytes held never exceed 104 923 136
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_006_part_of_exactly_the_part_limit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-007: Part one byte over the part limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/large.docx` as in E2E-006 but its main part is 104 857 601 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/large.docx: document part 'word/document.xml' is 104857601 bytes once decoded, over the limit of 104857600 bytes`
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_007_part_one_byte_over_the_part_limit`
- Red before the change because: after step 1 the extraction succeeds: nothing bounds a part today
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-006 | in `read_part` refuse a part whose declared size equals the limit (`>=` for `>`): the extraction is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, the 14 Office reader tests of `docs/extract.rs` (their code stays as it is)
- Dependencies not to add: none (no `Cargo.toml` change; BL-0137 owns the dev profile)
- The hook is test only (`#[cfg(test)]`) and is set on the calling thread; keep every test on the default single threaded `#[tokio::test]`
- A part that is absent, unreadable or damaged still reads as absent (BL-0144); only the part limit refuses in this story
- No dash as punctuation in code, comments or docs
- Scope boundary: the call total is US-0206, the size lie refusal US-0207, the content counter US-0208

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-004, E2E-SEC-005, E2E-SEC-006, E2E-SEC-007, E2E-SEC-008, E2E-SEC-009, E2E-SEC-010, E2E-SEC-012, E2E-SEC-013, E2E-007), before the production code (step 1: testkit builders, the test module, and the hook (`office()` wrapper and `OFFICE_HELD`) counting the bytes today's `entry_text` decodes, no limit; run the 14 tests: the 12 exploit tests and E2E-007 are red as stated (run them under `heavy()`, one at a time: each red run decodes its 200 000 000 byte part whole, about 1 GB peak); step 2: `read_part` with the part limit, the propagation of DEC-006).
- [ ] Green on arrival tests (E2E-006): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story US-0211 carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0205: refuse an Office part declared over the part limit` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-006, FR-NEW-007).
