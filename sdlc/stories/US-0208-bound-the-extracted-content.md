# US-0208: bound the extracted content

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0207
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A workbook that repeats one long shared text in many cells, or any Office document whose Markdown and kept table cells pass 268 435 456 bytes, is refused while the content is built, never holding more than one piece past the limit. Documents within the limit keep exactly the same text, tables and artifacts.

## Requirements covered
- SPEC-0088/FR-NEW-005: IF the extracted content built from a document passes 268 435 456 bytes THEN THE document text extraction SHALL stop building it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the extracted content exceeds 268435456 bytes`.
- SPEC-0088/FR-NEW-006 (partial: at most 373 293 056 bytes of extracted content): THE document text extraction SHALL hold, during one call, at most 104 857 600 plus 65 536 bytes of any one decoded part, at most 268 435 456 plus 65 536 bytes of decoded part data in total, and at most 373 293 056 bytes of extracted content.
- SPEC-0088/FR-NEW-011 (partial: Word, Excel and PowerPoint documents within every limit: same text and tables): THE document text extraction SHALL answer a Word, Excel or PowerPoint document within every limit with the same text, metadata, tables, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading any part when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_sec_011 e2e_0088_020 e2e_0088_021 e2e_0088_001 e2e_0088_002 e2e_0088_003`, then `cargo test -p mcp-fs-core --lib docs::extract`: every test passes (CSV and the 14 Office reader tests unchanged). Real assertions: `content_held` at most 373 293 056, the table artifact listing, the companion text.

## Design excerpt
### Decisions
- **DEC-007 (SPEC-0088):** content counting: `OfficeBudget::add_content(n)` adds `n` bytes, then refuses with `the extracted content exceeds 268435456 bytes` when the total is over `CONTENT_LIMIT` (equal accepted). Charged where content grows: each Markdown line pushed to `lines` (Word paragraph line, slide heading, slide paragraph, notes line, sheet heading), each grid cell when it is captured (Excel `xlsx_cell_text` result before the copy is kept; Word table cell at `</w:tc>`), and the Markdown table rendered row by row by `md_table`, which charges each cell plus its ` | ` separator before appending it and no longer builds a full `cleaned` copy of the grid. The peak is at most one piece over the limit, a piece being at most one part, so the content held stays under `CONTENT_LIMIT + PART_LIMIT` = 373 293 056. CSV calls the same `md_table` with `OfficeBudget::unbounded()` (counts, never refuses). **Obliged by:** SPEC-0088/FR-NEW-005, FR-NEW-006, FR-NEW-009 item 5. **Rationale:** a table row can join many shared texts, so a per row or per block charge would overshoot by more than one part; per cell charging keeps the bound. **Alternatives considered:** counting only the final text (the grid multiplication of occurrence 11 would land before any check); capping the grid (changes SPEC-0019's artifacts). **Source:** spec 14.3. **Evidence:** `extract.rs:462-490`, `:1071-1118`, `:768-812`.
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-011 (SPEC-0088):** fixtures, all in `crates/core/src/tools/archive_testkit.rs`: `office_fixture()` and `office_fixture_allowance(n)` (project `atlas`, member `mia@example.com`, folder `/docs`); `THIRD` = `ola@example.com` with `add_third_member`; `extract_text(f, person, path, refresh)`; `office_zip(parts: &[(&str, &[u8])])` (zip writer, deflate); `zip_raw(entries: &[RawPart])` writing given compressed bytes, checksum and declared size (`zip_declaring` becomes a thin wrapper over it, same output); `deflate_run(prefix, letter, n, suffix) -> (Vec<u8>, flate2::Crc)` building the deflate stream of `prefix + letter * n + suffix` from one per process cached segment of 1 000 000 letters (each segment compressed by its own `flate2::Compress` with `FlushCompress::Full`, so segments end on a byte boundary, reference nothing before them and concatenate into one valid stream; the checksum is combined with `Crc::combine`), so a 200 000 000 byte part costs about 200 KB and no 200 MB compression; `bomb_part(name) -> RawPart` (exactly 200 000 000 bytes, markup included: the letter `a` as text for `word/document.xml`, slides, notes and `xl/sharedStrings.xml`; as the value of a root attribute `x` for `ppt/presentation.xml`, every `.rels` part, `xl/workbook.xml` and worksheets); `padded_part(name, xml_head, total)` (a part of an exact size padded through a root attribute, for E2E-006 to E2E-011); `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when no entry); `nothing_left(f, doc, companion_before)`; `heavy()` returning the guard of a process wide `tokio::sync::Mutex<()>`, taken first by every test building or decoding a part of 10 MB or more (each can hold about 1 GB; in parallel they would exhaust memory). **Obliged by:** SPEC-0088 section 9 (fixtures built by the design), FR-NEW-002, FR-NEW-006. **Rationale:** no library writes a lying or zip64 header; compressing 200 MB per test in an unoptimized debug build would cost seconds each. **Alternatives considered:** committed fixture files (binary blobs for 40 cases, unreadable); `flate2::write::DeflateEncoder` per test (slow); a dev profile that optimizes dependencies (BL-0137). **Source:** design. **Evidence:** `crates/core/src/tools/archive_testkit.rs:236-305` (`zip_declaring`), `crates/core/src/docs/zip_check.rs:44-120` (`craft::rename_local`, reused by E2E-022).
### Data model
`OfficeBudget::content: u64` and `content_limit: Option<u64>` (`Some(CONTENT_LIMIT)` for Office, `None` for CSV).
### Contracts
`crates/core/src/docs/extract.rs`: `pub(crate) const CONTENT_LIMIT: u64 = 268_435_456;`; `fn unbounded() -> Self`; `fn add_content(&mut self, bytes: usize) -> Result<()>` (add, then `content > limit` refuses with `ToolError::invalid_argument("the extracted content exceeds 268435456 bytes")`); charge sites of DEC-007: Word paragraph line and table cell, slide heading, slide paragraph, notes line, sheet heading, Excel cell before it is kept, and `fn md_table(rows: &[Vec<String>], budget: &mut OfficeBudget) -> Result<String>` rendering row by row and charging each cell plus its ` | ` separator before appending it, output byte for byte the same as today; `csv` calls it with `OfficeBudget::unbounded()` and keeps its output. The hook's `content` is the counter's final value.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/office_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`). Table artifacts: `crate::tools::artifacts::list_tables` (`crates/core/src/tools/artifacts.rs:238`); grid positions: `ExtractResult::of_blocks` (`crates/core/src/docs/extract.rs:96-138`); `md_table` today (`:466-490`) and its test `md_table_pads_escapes_and_flattens` (`:1349`).
### Fixtures
Payloads: E2E-SEC-011, E2E-020, E2E-021 a workbook whose `xl/sharedStrings.xml` holds one `si` of 1 000 000 letters (`a` for SEC-011, `d` for 020 and 021) and whose sheet `S` has 400, 100 or 140 rows, each `<row><c t="s"><v>0</v></c></row>`; SEC-011 stored under 100 000 bytes (assert it). E2E-020 and E2E-021 use `office_fixture_allowance(1_000_000_000)`. E2E-001 to 003 small `office_zip` documents. Take `heavy()` first in SEC-011, 020, 021.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-SEC-011: One shared text multiplied by 400 cells
- **Covers:** occurrence 11
- **Red before the fix because:** `And the content held never exceeds 373 293 056` fails: today 400 copies of the text, 400 000 000 bytes, go into the grid and as much into the Markdown table.
- **Given** `/docs/mult.xlsx`, stored in under 100 000 bytes, whose shared strings hold one text of 1 000 000 letters `a` and whose sheet `S` has 400 rows, each a single cell referring to that text
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/mult.xlsx: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 373 293 056; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_sec_011_one_shared_text_multiplied_by_400`
- Red before the change because: step 1 (counter wired at every charge site, no limit): the extraction succeeds and `content_held` is above 800 000 000 (400 grid cells plus 400 Markdown cells of 1 000 000 bytes), not at most 373 293 056; run alone under `heavy()`: about 2 GB peak
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-020: Content just under the content limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-011; **Priority:** High
- **Given** session allowance 1 000 000 000 and `/docs/long.xlsx` whose shared strings hold one text of 1 000 000 letters `d` and whose sheet `S` has 100 rows, each a single cell referring to that text
- **When** the member asks for its text
- **Then** the extraction succeeds; one table artifact of 100 rows is listed for the document
- **And** the content held never exceeds 373 293 056
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_020_content_just_under_the_content_limit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-021: Content over the content limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** High
- **Given** `/docs/long.xlsx` as in E2E-020 with 140 rows
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/long.xlsx: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 373 293 056; nothing left
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_021_content_over_the_content_limit`
- Red before the change because: after step 1 the extraction succeeds with 140 rows
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-001: Word document within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-011; **Priority:** High
- **Given** `/docs/memo.docx` whose main part holds a heading `Results` (style Heading 1) and the paragraph `Quarterly results`
- **When** the member asks for its text
- **Then** the answer names `/docs/memo.md`, format `docx`, not cached, and its preview reads `# Results` then `Quarterly results`
- **And** `/docs/memo.md` holds that text; the audit trail has one extraction line for `/docs/memo.md`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_001_word_document_within_the_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-002: Excel workbook within the limits, accented shared text
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/docs/budget.xlsx` with one sheet `Q1` whose cells A1 `Poste`, B1 `Montant`, A2 shared text `Café`, B2 `12`
- **When** the member asks for its text
- **Then** the preview holds `## Sheet: Q1` and a table whose header is `Poste | Montant` and whose row is `Café | 12`
- **And** one table artifact captioned `Q1` is listed for `/docs/budget.xlsx`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_002_excel_workbook_within_the_limits_acce`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-003: PowerPoint deck within the limits, with notes
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/docs/deck.pptx` with slide 1 `Welcome` (notes `Say hello`) and slide 2 `Plan`
- **When** the member asks for its text
- **Then** the preview reads `## Slide 1`, `Welcome`, `> Notes: Say hello`, `## Slide 2`, `Plan`, in that order
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_003_powerpoint_deck_within_the_limits_wit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-020 | charge every Excel grid cell twice: 300 000 000 counted bytes pass the limit and the call is refused, the test fails |
| E2E-001 | in `docx_paragraph_line` ignore the style: the preview reads `Results` without `# `, the test fails |
| E2E-002 | make `xlsx_cell_text` return the index instead of the shared text: the row reads `0 | 12`, the test fails |
| E2E-003 | skip the notes line in `pptx`: `> Notes: Say hello` is missing, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `archive_testkit.rs`, the SPEC-0019 artifact code (`docs/artifacts/`)
- Dependencies not to add: none
- `md_table` keeps its output byte for byte (CSV and Office); one implementation, the CSV budget never refuses
- No dash as punctuation in code, comments or docs
- Scope boundary: the transient assembly copies (join, trim, character cut) are not counted (BL-0145)

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-SEC-011, E2E-021), before the production code (step 1: the counter (`add_content` without a limit check) at every charge site and the hook's `content`; write the 6 tests: SEC-011 and 021 red as stated; step 2: the limit check).
- [ ] Green on arrival tests (E2E-020, E2E-001, E2E-002, E2E-003): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story US-0211 carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0208: bound the extracted content` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-005, FR-NEW-006, FR-NEW-011).
