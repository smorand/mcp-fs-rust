# US-0206: count the parts read together

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0205
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A document whose parts read in one call declare more than 268 435 456 bytes together is refused before the part that crosses the total is decoded, so many slides, many sheets or one sheet listed several times cannot add up past the bound. A document of many small parts within the total is read as before.

## Requirements covered
- SPEC-0088/FR-NEW-003: IF the declared sizes of the parts already read in this call plus the declared size of the part about to be read exceed 268 435 456 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the document parts read total more than 268435456 bytes once decoded`, before decoding that part.
- SPEC-0088/FR-NEW-006 (partial: at most 268 435 456 plus 65 536 bytes of decoded part data in total): THE document text extraction SHALL hold, during one call, at most 104 857 600 plus 65 536 bytes of any one decoded part, at most 268 435 456 plus 65 536 bytes of decoded part data in total, and at most 373 293 056 bytes of extracted content.
- SPEC-0088/FR-NEW-011 (partial: many small parts within the limits: E2E-012): THE document text extraction SHALL answer a Word, Excel or PowerPoint document within every limit with the same text, metadata, tables, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading any part when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_008 e2e_0088_009 e2e_0088_010 e2e_0088_011 e2e_0088_012`: 5 tests pass. Real assertions: the refusal before the crossing part (`parts_held` at most 268 500 992) and nothing left (listing, artifacts, audit, a following write).

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0088):** one reader, `OfficeBudget::read_part(zip, name) -> Result<Option<String>>`, replaces `entry_text` at every call site: `index_for_name(name)` (absent: `Ok(None)`), declared size from `by_index_raw(i)?.size()` (central directory, `u64`, no decompressor built), part limit then call limit (DEC-003), then `by_index(i)` and `read_capped(file, declared)`; any open error other than a refusal stays `Ok(None)` as today. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-008. **Rationale:** one fix site covers occurrences 1 to 10; the declared size is known before any byte is decoded, so a refusal holds 0 bytes; `index_for_name` is what `by_name` uses (`zip-9.0.5/src/read/zip_archive.rs:545-551`), so the parts found are the same. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:563-568`, `crates/core/src/tools/archive_single.rs:92-108`, `zip-9.0.5/src/read/zip_archive.rs:551`, `:699`.
- **DEC-003 (SPEC-0088):** arithmetic: declared sizes are `u64` compared unsigned with `>` (equal accepted); the call total is a `u128` (`declared + size as u128 > CALL_LIMIT as u128`), so `18446744073709551615` never wraps; the part limit is checked before the call limit for the part about to be read; the size enters the total at its check, before decoding (a part read twice counts twice; a part that then reads as absent still counted). Messages print the `u64` with `{}`. **Obliged by:** SPEC-0088/FR-NEW-002 business rule, FR-NEW-003 business rule, FR-NEW-009 item 4. **Rationale:** exact digits and no wrap for the two largest declarable sizes (E2E-SEC-012, E2E-SEC-013). **Alternatives considered:** `checked_add` on `u64` (equivalent, one more branch). **Source:** spec 14.3. **Evidence:** n/a.
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-011 (SPEC-0088):** fixtures, all in `crates/core/src/tools/archive_testkit.rs`: `office_fixture()` and `office_fixture_allowance(n)` (project `atlas`, member `mia@example.com`, folder `/docs`); `THIRD` = `ola@example.com` with `add_third_member`; `extract_text(f, person, path, refresh)`; `office_zip(parts: &[(&str, &[u8])])` (zip writer, deflate); `zip_raw(entries: &[RawPart])` writing given compressed bytes, checksum and declared size (`zip_declaring` becomes a thin wrapper over it, same output); `deflate_run(prefix, letter, n, suffix) -> (Vec<u8>, flate2::Crc)` building the deflate stream of `prefix + letter * n + suffix` from one per process cached segment of 1 000 000 letters (each segment compressed by its own `flate2::Compress` with `FlushCompress::Full`, so segments end on a byte boundary, reference nothing before them and concatenate into one valid stream; the checksum is combined with `Crc::combine`), so a 200 000 000 byte part costs about 200 KB and no 200 MB compression; `bomb_part(name) -> RawPart` (exactly 200 000 000 bytes, markup included: the letter `a` as text for `word/document.xml`, slides, notes and `xl/sharedStrings.xml`; as the value of a root attribute `x` for `ppt/presentation.xml`, every `.rels` part, `xl/workbook.xml` and worksheets); `padded_part(name, xml_head, total)` (a part of an exact size padded through a root attribute, for E2E-006 to E2E-011); `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when no entry); `nothing_left(f, doc, companion_before)`; `heavy()` returning the guard of a process wide `tokio::sync::Mutex<()>`, taken first by every test building or decoding a part of 10 MB or more (each can hold about 1 GB; in parallel they would exhaust memory). **Obliged by:** SPEC-0088 section 9 (fixtures built by the design), FR-NEW-002, FR-NEW-006. **Rationale:** no library writes a lying or zip64 header; compressing 200 MB per test in an unoptimized debug build would cost seconds each. **Alternatives considered:** committed fixture files (binary blobs for 40 cases, unreadable); `flate2::write::DeflateEncoder` per test (slow); a dev profile that optimizes dependencies (BL-0137). **Source:** design. **Evidence:** `crates/core/src/tools/archive_testkit.rs:236-305` (`zip_declaring`), `crates/core/src/docs/zip_check.rs:44-120` (`craft::rename_local`, reused by E2E-022).
### Data model
`OfficeBudget::declared: u128`, the declared sizes of the parts read so far in the call, added at each part's check.
### Contracts
`crates/core/src/docs/extract.rs`: `pub(crate) const CALL_LIMIT: u64 = 268_435_456;`. In `read_part`, after the part limit: `if self.declared + u128::from(size) > u128::from(CALL_LIMIT)` refuse with `ToolError::invalid_argument("the document parts read total more than 268435456 bytes once decoded")`, else `self.declared += u128::from(size)`; then decode. A sheet listed twice is read twice and counts twice (the reading order is unchanged).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/office_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`).
### Fixtures
Payloads (DEC-011): E2E-008 and E2E-009 sheets are `padded_part` worksheets of one row of 10 numeric cells padded through a root attribute to their exact sizes, each under 100 000 000 bytes, the sizes computed so the six parts declare exactly 268 435 456 bytes (E2E-009: the third sheet one byte longer); E2E-010 slides are `padded_part` slides of 10 000 000 bytes whose letter `c` run is the root attribute (no text), so the refusal is the call total; E2E-011 one `padded_part` sheet of 100 000 000 bytes behind three `sheet` entries of the workbook pointing to the same relationship target. Take `heavy()` first in 008 to 011.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-008: Parts read totalling exactly the call limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** session allowance 1 000 000 000 and `/docs/wide.xlsx` whose shared strings, workbook, workbook relationships and three sheets, each sheet under 100 000 000 bytes, declare 268 435 456 bytes together, every sheet holding numbers only in rows of 10 cells
- **When** the member asks for its text
- **Then** the extraction succeeds with 3 sheets in its metadata
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_008_parts_read_totalling_exactly_the_call`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-009: Parts read one byte over the call limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/wide.xlsx` as in E2E-008 with its third sheet one byte longer
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/wide.xlsx: the document parts read total more than 268435456 bytes once decoded`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_009_parts_read_one_byte_over_the_call_lim`
- Red before the change because: after US-0205 the extraction succeeds: no total is kept
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-010: Many slides together over the call limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/many.pptx` with 30 slides, each declaring and holding 10 000 000 bytes of the letter `c`
- **When** the member asks for its text
- **Then** the refusal is the call limit message of E2E-009 naming `/docs/many.pptx`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_010_many_slides_together_over_the_call_li`
- Red before the change because: after US-0205 the extraction succeeds with 30 slides and `parts_held` is above 300 000 000
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-011: One sheet listed three times
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/repeat.xlsx` whose workbook lists sheets `A`, `B`, `C`, all three pointing to the same sheet part of 100 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/repeat.xlsx: the document parts read total more than 268435456 bytes once decoded`; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_011_one_sheet_listed_three_times`
- Red before the change because: after US-0205 the extraction succeeds, the sheet decoded three times
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-012: Many small slides within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-011; **Priority:** Medium
- **Given** `/docs/talk.pptx` with 500 slides, slide N holding the text `Slide text N`
- **When** the member asks for its text
- **Then** the extraction succeeds with 500 slides in its metadata and the preview starts `## Slide 1` then `Slide text 1`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_012_many_small_slides_within_the_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-008 | compare the call total with `>=` instead of `>`: the third sheet is refused, the test fails |
| E2E-012 | count every part read as 1 048 576 bytes in the call total: 500 slides pass 268 435 456 and the call is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `archive_testkit.rs` (US-0205 built every builder; a missing helper is a US-0205 defect: report it)
- Dependencies not to add: none
- Exact arithmetic: the total is a `u128`; equal is accepted
- No dash as punctuation in code, comments or docs
- Scope boundary: the size lie is US-0207, the content counter US-0208

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-009, E2E-010, E2E-011), before the production code (step 1: write the 5 tests and run them on US-0205's code: 009, 010, 011 red as stated (each decodes up to 300 MB, run under `heavy()`); step 2: the call total in `read_part`).
- [ ] Green on arrival tests (E2E-008, E2E-012): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story US-0211 carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0206: count the parts read together` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-003, FR-NEW-006, FR-NEW-011).
