# US-0207: refuse a part larger than it declares, in reading order

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0206
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A part that decodes to more bytes than it declares stops one chunk past its declared size and refuses the call naming the part, where today a Word document reports its main part missing and a lying sheet or slide is silently skipped. The order of every refusal is pinned: membership, path, header check, then parts in reading order.

## Requirements covered
- SPEC-0088/FR-NEW-004: IF a part decodes to more bytes than it declares THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: document part '<part>' decodes to more bytes than declared <D>`.
- SPEC-0088/FR-NEW-009: IF several refusals apply to one text extraction of a Word, Excel or PowerPoint document THEN THE document text extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the file is not a zip or fails the SPEC-0041 header check (`could not extract <path>: not a valid OOXML package: <reason>`); (4) parts in reading order, and for each part FR-NEW-002, then FR-NEW-003, then FR-NEW-004 while it decodes; (5) FR-NEW-005 at the moment the content crosses its limit, which can fall between two parts.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_013 e2e_0088_015 e2e_0088_016 e2e_0088_022 e2e_0088_023 e2e_0088_024 e2e_0088_025 e2e_0088_026`: 8 tests pass. Real assertion: `parts_held` at most 65 546 for a size lie, and the code and message of the first refusal of FR-NEW-009.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0088):** a size lie is `ReadError::CapExceeded` from `read_capped`, or a zip `InvalidData` error whose text contains `larger than its declared` (zip enforces the declared size itself, `zip-9.0.5/src/crc32.rs:43-51`, `src/types.rs` `size_limit`), mapped to `document part '<part>' decodes to more bytes than declared <D>`; every other read error (bad checksum, damaged stream, encrypted part) keeps reading as absent. **Obliged by:** SPEC-0088/FR-NEW-004, FR-NEW-011 business rule (BL-0144). **Rationale:** the same mapping as `zip_read_error` (`crates/core/src/tools/archive.rs:571-578`); at most the declared size plus one 65 536 byte chunk is held. **Alternatives considered:** trusting zip's own limit alone (true today, not guaranteed). **Source:** SPEC-0085 DEC-005. **Evidence:** as cited.
- **DEC-005 (SPEC-0088):** refusals are `ToolError::invalid_argument` with the messages of section 5 and no prefix; `prepare_extraction` keeps adding `could not extract <path>: ` (`extract.rs:366-374`), so both doors and every format share the text; the existing `docx: word/document.xml is missing` and not a zip messages are unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-010, spec D-5. **Rationale:** one wrapper, no new code. **Alternatives considered:** a new error code (spec D-5 rules it out). **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:366-374`.
- **DEC-006 (SPEC-0088):** propagation and reading order: `docx` uses `read_part(..)?` then the missing message on `None`; `pptx_slide_order` returns `Result<Vec<String>>` (presentation part or its relationships absent: numeric fallback as today; refused: the refusal); `pptx_notes_part` returns `Result<Option<String>>`; `xlsx_shared_strings` and `xlsx_sheets` return `Result<..>` (the workbook relationships refusal is no longer swallowed by `unwrap_or_default`); a slide or sheet that reads as absent is still skipped with `continue`. The reading order of spec 2.1 is unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-009 item 4, FR-NEW-011. **Rationale:** a refusal must not degrade into "absent" (E2E-SEC-004 would otherwise fall back to the numeric order and succeed). **Alternatives considered:** none. **Source:** existing code. **Evidence:** `extract.rs:716-717`, `:868-881`, `:894-930`, `:936-942`, `:985-1048`.
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
### Data model
None new.
### Contracts
`crates/core/src/docs/extract.rs` `read_part`: `ReadError::CapExceeded`, and an `Io` error of kind `InvalidData` whose text contains `larger than its declared`, refuse with `ToolError::invalid_argument(format!("document part '{name}' decodes to more bytes than declared {size}"))`; any other `Io` error keeps `Ok(None)`. Order inside `read_part`: part limit, call total, then decoding (size lie).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/office_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`). `zip_read_error` (`crates/core/src/tools/archive.rs:571-578`) for the text match; `craft::rename_local` (`crates/core/src/docs/zip_check.rs:100`) for E2E-022 (rename the main part's local header to `word/documenX.xml`); membership refusal: `AppState::authorize` (`crates/core/src/state.rs:59`).
### Fixtures
Payloads: liars with `zip_declaring` (declared 10 holding 1 000 000 bytes for E2E-013; `xl/worksheets/sheet2.xml` declared 100 holding 500 000 bytes is E2E-014, US-0211; declared 0 holding a slide with `hello` for E2E-015; slide 1 declared 10 holding 1 000 bytes then a `bomb_part` slide 2 for E2E-024, which needs `zip_raw` to mix both); E2E-016 a main part of exactly 5 000 bytes, `exact` padded with spaces inside `w:t`; E2E-022 a bomb document passed through `craft::rename_local`; E2E-025 member `eve@example.com` never added; E2E-026 nothing seeded. Take `heavy()` first in 022, 023, 024, 025.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-013: Word main part larger than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-006, FR-NEW-007; **Priority:** High
- **Given** `/docs/liar.docx` whose main part declares 10 bytes and holds 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/liar.docx: document part 'word/document.xml' decodes to more bytes than declared 10`, not `docx: word/document.xml is missing`
- **And** the decoded bytes held never exceed 65 546; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_013_word_main_part_larger_than_it_declare`
- Red before the change because: after US-0206 the refusal is `could not extract /docs/liar.docx: docx: word/document.xml is missing` (the zip size error reads as absent)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-015: Slide declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/zero.pptx` whose part `ppt/slides/slide1.xml` declares 0 bytes and holds a slide with the text `hello`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/zero.pptx: document part 'ppt/slides/slide1.xml' decodes to more bytes than declared 0`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_015_slide_declared_0_holding_data`
- Red before the change because: after US-0206 the extraction succeeds without the slide
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-016: Part holding exactly its declared size
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/exact.docx` whose main part declares and holds 5 000 bytes, a paragraph `exact` padded with spaces
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview reads `exact`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_016_part_holding_exactly_its_declared_siz`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-022: Header inconsistency wins over a bomb
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/twice.docx`, a bomb part document whose main part's own header names it `word/documenX.xml`
- **When** the member asks for its text
- **Then** the refusal starts `could not extract /docs/twice.docx: not a valid OOXML package: header inconsistency detected`
- **And** the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_022_header_inconsistency_wins_over_a_bomb`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-023: Shared strings named before a sheet
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/two.xlsx` whose shared strings part and only sheet are both bomb parts
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/two.xlsx: document part 'xl/sharedStrings.xml' is 200000000 bytes once decoded, over the limit of 104857600 bytes`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_023_shared_strings_named_before_a_sheet`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-024: First slide lies, second is a bomb
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-009; **Priority:** Medium
- **Given** `/docs/order.pptx` whose slide 1 declares 10 bytes and holds 1 000 bytes and whose slide 2 is a bomb part
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/order.pptx: document part 'ppt/slides/slide1.xml' decodes to more bytes than declared 10`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_024_first_slide_lies_second_is_a_bomb`
- Red before the change because: after US-0206 slide 1 reads as absent and the refusal names `ppt/slides/slide2.xml` with the part limit message
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-025: Not a member
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document, and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_025_not_a_member`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-026: Missing path
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** nothing at `/docs/none.docx`
- **When** the member asks for its text
- **Then** the refusal is `ERR_NOT_FOUND` with `not a file: /docs/none.docx`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_026_missing_path`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-016 | call `read_capped(file, size - 1)` in `read_part`: the part is refused as a size lie, the test fails |
| E2E-022 | skip `header_inconsistency` in `open_zip`: the refusal becomes the part limit message, the test fails |
| E2E-023 | in `xlsx`, read the workbook and the sheets before `xlsx_shared_strings`: the refusal names `xl/worksheets/sheet1.xml`, the test fails |
| E2E-025 | make `AppState::authorize` accept a non member: the refusal becomes the part limit message, the test fails |
| E2E-026 | drop the `is_file` check of `prepare_extraction`: the message is no longer `not a file: /docs/none.docx`, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `archive_testkit.rs`
- Dependencies not to add: none
- Only a size error is a refusal: a bad checksum, a damaged stream or an encrypted part keep reading as absent (BL-0144)
- No dash as punctuation in code, comments or docs
- Scope boundary: the content limit is US-0208

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-013, E2E-015, E2E-024), before the production code (step 1: write the 8 tests and run them on US-0206's code: 013, 015, 024 red as stated; step 2: the size lie mapping in `read_part`).
- [ ] Green on arrival tests (E2E-016, E2E-022, E2E-023, E2E-025, E2E-026): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story US-0211 carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0207: refuse a part larger than it declares, in reading order` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-004, FR-NEW-009).
