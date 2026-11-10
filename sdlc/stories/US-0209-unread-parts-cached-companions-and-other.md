# US-0209: unread parts, cached companions and other formats unchanged

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0208
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Parts the extraction never reads (images, media, nested archives) cost nothing and are never opened, an up to date companion still answers without reading a part, and the other refusals and formats are unchanged. This pins, with tests, what the new reader must not touch.

## Requirements covered
- SPEC-0088/FR-NEW-008: THE document text extraction SHALL NOT decode, count or open a part outside the reading order of Section 2.1, nor any archive nested inside a part.
- SPEC-0088/FR-NEW-011 (partial: cached companion, not a zip, missing Word main part, `.docm` read as text): THE document text extraction SHALL answer a Word, Excel or PowerPoint document within every limit with the same text, metadata, tables, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading any part when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_005 e2e_0088_017 e2e_0088_018 e2e_0088_019 e2e_0088_027 e2e_0088_028 e2e_0088_037`: 7 tests pass. Real assertions: `parts_held` 0 or at most 65 536, the cached flag, the refusal texts, the listing of `/docs`.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0088):** one reader, `OfficeBudget::read_part(zip, name) -> Result<Option<String>>`, replaces `entry_text` at every call site: `index_for_name(name)` (absent: `Ok(None)`), declared size from `by_index_raw(i)?.size()` (central directory, `u64`, no decompressor built), part limit then call limit (DEC-003), then `by_index(i)` and `read_capped(file, declared)`; any open error other than a refusal stays `Ok(None)` as today. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-008. **Rationale:** one fix site covers occurrences 1 to 10; the declared size is known before any byte is decoded, so a refusal holds 0 bytes; `index_for_name` is what `by_name` uses (`zip-9.0.5/src/read/zip_archive.rs:545-551`), so the parts found are the same. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:563-568`, `crates/core/src/tools/archive_single.rs:92-108`, `zip-9.0.5/src/read/zip_archive.rs:551`, `:699`.
- **DEC-006 (SPEC-0088):** propagation and reading order: `docx` uses `read_part(..)?` then the missing message on `None`; `pptx_slide_order` returns `Result<Vec<String>>` (presentation part or its relationships absent: numeric fallback as today; refused: the refusal); `pptx_notes_part` returns `Result<Option<String>>`; `xlsx_shared_strings` and `xlsx_sheets` return `Result<..>` (the workbook relationships refusal is no longer swallowed by `unwrap_or_default`); a slide or sheet that reads as absent is still skipped with `continue`. The reading order of spec 2.1 is unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-009 item 4, FR-NEW-011. **Rationale:** a refusal must not degrade into "absent" (E2E-SEC-004 would otherwise fall back to the numeric order and succeed). **Alternatives considered:** none. **Source:** existing code. **Evidence:** `extract.rs:716-717`, `:868-881`, `:894-930`, `:936-942`, `:985-1048`.
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-014 (SPEC-0088):** tests whose rule an earlier story or today's code already holds are green on arrival and each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent SPEC-0085 DEC-013. **Source:** design. **Evidence:** US-0189 to US-0193.
### Data model
None.
### Contracts
None new: the reader of US-0205 to US-0208 is exercised as it is.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/office_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`). Cached companion: `prepare_extraction` (`crates/core/src/docs/extract.rs:318-362`); routing by suffix: `Extractor::extract` (`:151-185`); the existing `companion_md_is_reused_when_up_to_date` (`:1876`).
### Fixtures
Payloads: E2E-005 a bomb document then `/docs/bomb.md` written after it (later modification time) holding `kept text`; E2E-017 `word/media/image1.png` and E2E-019 `xl/media/video.bin` as 200 000 000 byte `deflate_run` parts through `zip_raw`; E2E-018 `word/embeddings/inner.zip`, itself `zip_raw` of one 200 000 000 byte entry, stored in the outer document; E2E-037 a `bomb_part` main part in `/docs/macro.docm`. Take `heavy()` first in 005, 017, 018, 019, 037.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-005: Up to date companion answers without reading a bomb
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-011; **Priority:** High
- **Given** `/docs/bomb.docx`, a bomb part document, and `/docs/bomb.md` holding `kept text`, written after it
- **When** the member asks for the text of `/docs/bomb.docx` without refresh
- **Then** the answer is cached, names `/docs/bomb.md` and previews `kept text`
- **And** the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_005_up_to_date_companion_answers_without`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-017: Huge embedded image is not read
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/docs/photo.docx` holding the paragraph `See photo` and `word/media/image1.png`, declaring and holding 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview reads `See photo`
- **And** the decoded bytes held never exceed 65 536
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_017_huge_embedded_image_is_not_read`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-018: Nested archive is not opened
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/docs/nested.docx` holding the paragraph `Outer` and `word/embeddings/inner.zip`, a zip archive whose entry declares and holds 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds, the preview reads `Outer`, and no file appears under `/docs/nested`
- **And** the decoded bytes held never exceed 65 536
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_018_nested_archive_is_not_opened`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-019: Huge unread part in a workbook
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/media.xlsx` with sheet `S` (cell `fine`) and `xl/media/video.bin`, declaring and holding 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview holds `fine`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_019_huge_unread_part_in_a_workbook`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-027: Not a zip, unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/docs/fake.docx` holding the text `plain words`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `could not extract /docs/fake.docx: not a valid OOXML package:`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_027_not_a_zip_unchanged`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-028: Word document without its main part, unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/docs/empty.docx` holding only `word/styles.xml`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/empty.docx: docx: word/document.xml is missing`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_028_word_document_without_its_main_part_u`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-037: A `.docm` file is not read as a Word document
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Low
- **Given** `/docs/macro.docm`, a zip whose main part is a bomb part
- **When** the member asks for its text
- **Then** the extraction succeeds as today, with format `text` and the note `unknown extension .docm; decoded as text`
- **And** the decoded bytes held are 0 (no part is decoded, the stored bytes are)
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_037_a_docm_file_is_not_read_as_a_word_doc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-005 | in `prepare_extraction` drop the up to date companion branch (always extract): the bomb is refused, the test fails |
| E2E-017 | in `docx`, read every part listed by `entry_names` through `read_part` before the main part: `word/media/image1.png` is refused on the part limit, the test fails |
| E2E-018 | the same mutation: `word/embeddings/inner.zip` is read, `parts_held` passes 65 536 (the inner archive is about 200 KB), the test fails |
| E2E-019 | in `xlsx`, read `xl/media/video.bin` through `read_part`: refused on the part limit, the test fails |
| E2E-027 | in `open_zip` map the zip error to `docx: word/document.xml is missing`: the prefix `not a valid OOXML package:` is gone, the test fails |
| E2E-028 | in `docx` answer an absent main part with an empty text: the call succeeds, the test fails |
| E2E-037 | route `.docm` to `docx` in `Extractor::extract`: the bomb part is refused, the test fails |

## Constraints
- Files not to touch: `crates/core/src/docs/extract.rs` (a test that fails here is a defect of US-0205 to US-0208: report it, do not patch it in this story), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Every mutation is applied, run, then reverted; the commit carries no mutation
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] RED evidence: none: every test here is green on arrival (DEC-014); baseline run recorded, then each mutation.
- [ ] Green on arrival tests (E2E-005, E2E-017, E2E-018, E2E-019, E2E-027, E2E-028, E2E-037): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story US-0211 carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0209: unread parts, cached companions and other formats unchanged` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-008, FR-NEW-011).
