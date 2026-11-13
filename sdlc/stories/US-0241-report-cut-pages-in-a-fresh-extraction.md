# US-0241: report cut pages in a fresh PDF extraction

> Spec: SPEC-0094 (sdlc/specs/SPEC-0094-mark-damaged-pdf-page-content/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A member who extracts the text of a PDF with a damaged or cut page content, in the page or in a form it draws, gets the same text as today and a note naming the damaged pages. The note is built and composed with the scanned PDF message; this story covers the first extraction of a PDF (the stored text copy is the next story).

## Requirements covered
- SPEC-0094/FR-NEW-001: WHEN a member extracts the text of a PDF in which the content of one or more pages is damaged or cut, THE text extraction SHALL return the text of every page exactly as it returns it today AND a note naming the damaged pages.
- SPEC-0094/FR-NEW-002 (partial: the wording and the cap, unit tested here; the 25 page journey is E2E-004 in US-0242): THE text extraction SHALL write the damage note as `pdf: damaged or cut content on page <N>; the text may be incomplete` for one damaged page, and `pdf: damaged or cut content on pages <N1>, <N2>; the text may be incomplete` for several, listing page numbers in ascending order, numbered as in the `*[Page N]*` markers, separated by a comma and a space.
- SPEC-0094/FR-NEW-003 (partial: a complete PDF, first extraction; other formats and replaced files in US-0243): THE text extraction SHALL NOT write a damage note for a PDF in which every page content is complete, nor for any document that is not a PDF.
- SPEC-0094/FR-NEW-004: WHEN any part of the content of a page, or the content of a form the page draws, is damaged or cut, THE text extraction SHALL count that page as damaged once.
- SPEC-0094/FR-NEW-007: IF the extracted text of a PDF is empty AND one or more pages are damaged, THEN THE text extraction SHALL write the damage note first, then one space, then `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`.
- SPEC-0094/FR-NEW-008: THE text extraction SHALL NOT refuse, shorten or alter the extraction of any PDF because a page is damaged or cut.
- SPEC-0094/FR-NEW-009: THE text extraction SHALL give the same damage note through the assistant tool interface and through the web API.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0094_001 e2e_0094_003 e2e_0094_005 e2e_0094_006 e2e_0094_010 e2e_0094_011 e2e_0094_012 e2e_0094_014 e2e_0094_015 e2e_0094_016 e2e_0094_017 e2e_0094_018 e2e_0094_019 e2e_0094_021 damage_note`: 14 e2e tests and the `damage_note` unit tests pass. Real assertions: the exact `note` strings of a first extraction, the extracted text unchanged, the refusal message, equal notes on the two doors. The existing PDF suites must stay green: `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0089_ e2e_0092_ --test-threads=1`.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0094):** detect damage with `Content::decode_strict` first and fall back to the lenient `Content::decode` on error, at both parse sites (`pdf_bound.rs:1514` the page, `:1604` a drawn form). One grammar for both calls (`lopdf-0.42.0/src/parser/mod.rs:721-742`), so a clean page yields the same operations in one parse and the walk is unchanged. Rejected: a second strict pass on every page, an own validator.
- **DEC-002 (SPEC-0094):** carry the finding as `damaged: bool` per page (`PageWalk` to `Walked`) and `damaged: Vec<u32>` on `PdfText`, page numbers from 1, ascending, one entry per damaged page, so a page damaged twice is named once.
- **DEC-003 (SPEC-0094):** pages the reader never reaches (after a reader page error, `pdf_bound.rs:240-244`) are not walked and not listed: walking them could raise a refusal that is not raised today.
- **DEC-004 (SPEC-0094):** `damage_note(pages: &[u32]) -> String` and `DAMAGE_LIST_CAP: usize = 20` in `docs/extract.rs` next to the scanned message; `pdf()` composes the final note once: damage note, then one space, then the scanned message when the text is empty (`with_note` overwrites, `extract.rs:89-92`).
- **DEC-008 (SPEC-0094):** every `E2E-` is a functional e2e through `fs_ops::extract_document` on a `nesting_fixture`, in `tools/damage_e2e.rs`, no `in_child`.
- **DEC-009 (SPEC-0094):** the form depth refusal stays first: `walk` returns `Err` before any note is built, so no note and no stored copy accompany a refusal.
### Data model
None in this story.
### Contracts
- `PdfText { pages: Vec<usize>, body: String, title: Option<String>, damaged: Vec<u32> }` (`pdf_bound.rs:141`).
- Note wording, verbatim (singular, plural, cap, with no text):
  - `pdf: damaged or cut content on page 2; the text may be incomplete`
  - `pdf: damaged or cut content on pages 1, 3; the text may be incomplete`
  - `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 and 5 more; the text may be incomplete` (M = count minus 20; exactly 20 has no ` and ... more`)
  - `pdf: damaged or cut content on page 1; the text may be incomplete no extractable text layer (scanned PDF?); enable a multimodal OCR provider`
- Refusals unchanged: `could not extract <path>: <message>` (`extract.rs:657-662`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/damage_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`mod damage_e2e;` in alphabetical place, near `mod case_paths_e2e;`) |

### Existing patterns
- The two parse sites, `crates/core/src/docs/pdf_bound.rs:1514` (`let Ok(parsed) = Content::decode(&content) else { return Ok(page.into_walked(invalid)) };`) and `:1604` (`let Ok(parsed) = Content::decode(&bytes) else { break };`). Shape of the change: `let strict = Content::decode_strict(&content); let damaged = strict.is_err();` then use the strict value when `Ok`, else the lenient `Content::decode` exactly as today; set `page.damaged = true` on `Err`. A clean page parses once.
- `Walked` and `PageWalk` (`pdf_bound.rs:1319`, `:1409`, `into_walked` `:1444`); `extract` pushes the page number when `walked.damaged` (`pdf_bound.rs:209-213`).
- The scanned message site `crates/core/src/docs/extract.rs:793-810` (`pdf()`): build `damage_note(&text.damaged)`, then `with_note` once.
- Test shape: `crates/core/src/tools/nesting_e2e.rs:94-104` (`nesting_fixture`, `seed`, `extract_text`, assertions on `v["preview"]`, `v["cached"]`); MCP door `nesting_e2e.rs:780-795` (`McpServer::fs_extract_text`); the form builder `nesting_testkit.rs:156` (`form`), `forms_chain_pdf` `:179`; the page writer `pdf_testkit.rs:285` (`pages_writer`, two pages at most).
### Fixtures
In `damage_e2e.rs`: `fn pdf_pages(contents: Vec<PdfObj>, extra: Vec<(u32, PdfObj)>, resources: &str) -> Vec<u8>` lays out N pages with `PdfWriter` (object 1 catalog, 2 page tree with `/Kids`, 5 font `Helvetica`, page `10 + 2i`, its content `11 + 2i`, `/F1` in every page's resources, forms at objects 100 and above through `extra`), `finish()`, then `small(...)`. Content strings, byte exact: page text `BT /F1 12 Tf 72 700 Td (Quarterly results) Tj ET`; a cut page `BT /F1 12 Tf 72 700 Td (Revenue rose) Tj ET BT 72 650 Td (Costs fell by` (no closing parenthesis, no `ET`). The expected body is written by hand from the `*[Page N]*` markers (`pdf_bound.rs:224`: `\n\n---\n*[Page N]*\n\n`, first two newlines trimmed).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `./test.sh`). The ad hoc `cargo test` filter of the Independent Test is for the red green loop only.

### SPEC-0094/E2E-001: Cut page is reported and the text is kept
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-008, FR-NEW-009
- **Given** the PDF `report-3p.pdf` of SC-001 stored in a project, no stored text copy, and the member extracts through the assistant tool interface
- **When** the member extracts its text
- **Then** the text holds, in order, `*[Page 1]*` and `Quarterly results`, `*[Page 2]*` and `Revenue rose`, `*[Page 3]*` and `Outlook stable`, the page count is 3, and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- **And** the text is identical to what the same file gave before this change; the scanned PDF message is absent
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_001_a_cut_page_is_reported_and_the_text_kept`
- Red before the code: the `note` is `""`, the assertion on the damage note fails.

### SPEC-0094/E2E-003: Two damaged pages through the web API, one cut in a form
- **Category:** failure
- **Scenario:** SC-005
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-009
- **Given** a 3 page PDF: page 1 cut in its own content (`Opening remarks` then `Agenda i` with no closing), page 2 complete, page 3 draws a form whose content is cut, and the member uses the web API
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on pages 1, 3; the text may be incomplete`
- **And** the text holds `Opening remarks` on page 1 and the complete text of page 2
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_003_two_damaged_pages_one_cut_in_a_form`
- Red before the code: the `note` is `""`. Page 3 content: `/Fm1 Do`, form object 100 content `BT /F1 12 Tf 72 700 Td (Opening) Tj ET (cut` (the string never closes).

### SPEC-0094/E2E-005: Exactly twenty damaged pages
- **Category:** edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-002
- **Given** a 22 page PDF where pages 1 to 20 are cut and pages 21 and 22 are complete
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20; the text may be incomplete`, with no `more`
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_005_exactly_twenty_damaged_pages`
- Red before the code: the `note` is `""`. 22 pages built by `pdf_pages`, pages 1 to 20 cut, 21 and 22 complete.

### SPEC-0094/E2E-006: A page damaged twice is named once
- **Category:** edge
- **Scenario:** SC-005
- **Requirements:** FR-NEW-002, FR-NEW-004, FR-NEW-009
- **Given** a 3 page PDF where page 2 is cut in its own content AND draws a form whose content is cut; pages 1 and 3 are complete
- **When** the member extracts its text through the assistant tool interface
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- Level / Driver / Location: functional e2e / `extract_text` helper, MCP door `fs_extract_text` / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_006_a_page_damaged_twice_is_named_once`
- Red before the code: the `note` is `""`.

### SPEC-0094/E2E-010: Damaged page and no text
- **Category:** failure
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005, FR-NEW-007
- **Given** a 1 page PDF whose only content is a text string opened and never closed (`(`), so no text is extractable
- **When** the member extracts its text, then extracts again without the refresh option
- **Then** both notes are exactly `pdf: damaged or cut content on page 1; the text may be incomplete no extractable text layer (scanned PDF?); enable a multimodal OCR provider`
- **And** neither answer comes from a stored text copy, and no stored text copy exists next to the PDF
- Level / Driver / Location: functional e2e / `extract_text` helper, twice / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_010_damaged_page_and_no_text`
- Red before the code: the `note` is only the scanned message. The page content is exactly `(` .

### SPEC-0094/E2E-011: Text before the cut is kept verbatim, accents included
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-008
- **Given** a 1 page PDF with the content `Bilan` then `Coût en baisse de` cut with no closing
- **When** the member extracts its text
- **Then** the text equals what the same file gave before this change (`Bilan` and nothing from the cut part), and the note is exactly `pdf: damaged or cut content on page 1; the text may be incomplete`
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_011_text_before_the_cut_is_kept_verbatim`
- Red before the code: the `note` is `""`. Content: `BT /F1 12 Tf 72 700 Td (Bilan) Tj ET BT 72 650 Td (Co\xc3\xbbt en baisse de` as UTF-8 bytes of the accents, no closing parenthesis.

### SPEC-0094/E2E-012: Existing refusal wins
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Given** a 2 page PDF stored as `/nested-and-cut.pdf` where page 1 draws forms nested 65 deep and page 2 is cut
- **When** the member extracts its text
- **Then** the extraction is refused with exactly `could not extract /nested-and-cut.pdf: page 1 draws PDF forms nested more than 64 deep`, the same as before this change
- **And** no stored text copy is written and no damage note appears
- Level / Driver / Location: functional e2e / `extract_text` helper, refusal / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_012_an_existing_refusal_wins`
- Green on arrival: the refusal exists today (`pdf_bound.rs:1593-1603`). Mutation proof: in `walk` replace that `Err(...)` by `break`, the test must fail on the message. Build: page 1 draws a chain of 66 nested forms (`forms_chain_pdf` style, `nesting_testkit.rs:179`), page 2 is a cut page; stored as `/nested-and-cut.pdf`.

### SPEC-0094/E2E-014: Both ways in give the same note
- **Category:** cross scenario
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-009
- **Given** `report-3p.pdf` and a second project copy of it with no stored text copy
- **When** the member extracts one through the assistant tool interface and the other through the web API
- **Then** the two notes are both exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the two texts are equal
- Level / Driver / Location: functional e2e / MCP `fs_extract_text` (as `nesting_e2e.rs:780-795`) and the REST route `POST /api/fs/{mount_id}/extract-text` of the served app (`tool_surface_e2e::setup`, `app_with_atlas`, a token for `mia`) / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_014_both_doors_give_the_same_note`
- Red before the code: both notes are `""`. Two files: `/docs/report-3p.pdf` and `/docs/second.pdf`, same bytes.

### SPEC-0094/E2E-015: Cut in the second part of a page
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** a 2 page PDF where page 1 is complete and page 2 is made of two parts, the first complete (`First part`) and the second cut (`Second pa`, no closing)
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the text of page 2 holds `First part`
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_015_cut_in_the_second_part_of_a_page`
- Red before the code: the `note` is `""`. Page 2 `/Contents [6 0 R 20 0 R]`: part 1 `BT /F1 12 Tf 72 700 Td (First part) Tj ET`, part 2 `BT 72 650 Td (Second pa`; the parts are joined by a line break before parsing (`pdf_bound.rs:1508-1513`).

### SPEC-0094/E2E-016: Empty page and trailing blanks are not damage
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003
- **Given** a 3 page PDF: page 1 complete, page 2 with no content at all, page 3 complete and followed by two line breaks and a comment line
- **When** the member extracts its text
- **Then** the note is empty and the text equals what the same file gave before this change
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_016_empty_page_and_trailing_blanks_are_clean`
- Green on arrival (no note on a clean file). Mutation proof: set `page.damaged = true` unconditionally, the test must fail. Page 3 content ends with `\n\n% end of page\n`.

### SPEC-0094/E2E-017: Unknown operator and unclosed block are not damage
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001, FR-NEW-003
- **Given** a 2 page PDF: page 1 content is `BT (Known) Tj ET` followed by an instruction with the operator word `Zz` the reader does not know; page 2 opens a block with `q` and never closes it
- **When** the member extracts its text
- **Then** the note is empty and the text equals what the same file gave before this change
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_017_unknown_operator_and_unclosed_block_are_clean`
- Green on arrival. Mutation proof as E2E-016. Page 1 content `BT (Known) Tj ET Zz`, page 2 content `q BT (Open) Tj ET`; ASSUMED accepted by the strict grammar (spec 14.3), this test is the signal: if it is red on arrival, stop and report a spec issue.

### SPEC-0094/E2E-018: Stray closing marks in a form
- **Category:** edge
- **Scenario:** SC-005
- **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** a 2 page PDF where page 2 draws a form whose content is `BT (Fine) Tj ET` followed by `>>`, and page 1 is complete
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the text of page 2 holds `Fine`
- Level / Driver / Location: functional e2e / `extract_text` helper / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_018_stray_closing_marks_in_a_form`
- Red before the code: the `note` is `""`. Form object 100 content `BT /F1 12 Tf 72 700 Td (Fine) Tj ET >>`; ASSUMED rejected by the strict grammar (spec 14.3), this test is the signal.

### SPEC-0094/E2E-019: Damaged page beyond the text asked for
- **Category:** failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** `report-3p.pdf` and a maximum length of 20 characters
- **When** the member extracts its text
- **Then** the returned text is cut at 20 characters and flagged as cut, and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- Level / Driver / Location: functional e2e / `fs_ops::extract_document` called directly with `max_chars` 20 and `preview_chars` 4000, as `archive_testkit.rs:489-500` does with 200000 / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_019_a_damaged_page_beyond_the_text_asked_for`
- Red before the code: the `note` is `""`. Expected `v["truncated"] == true`. Page 2 of `report-3p.pdf` is where the text is cut at 20 characters, the damage still listed.

### SPEC-0094/E2E-021: A scan with no cut page keeps its own message
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003, FR-NEW-007
- **Given** a 1 page PDF with no text at all and complete content (an empty page)
- **When** the member extracts its text twice
- **Then** both notes are exactly `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`, and neither contains `damaged`
- Level / Driver / Location: functional e2e / `extract_text` helper, twice / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_021_a_scan_keeps_its_own_message`
- Green on arrival. Mutation proof: make `pdf()` always emit `damage_note(&[1])` before the scanned message, the test must fail. A 1 page PDF whose page content is empty.

Unit tests (`docs/extract.rs`, `#[cfg(test)]`), test name prefix `damage_note_`: `[]` gives `""`; `[2]` gives the singular; `[1, 3]` the plural; 20 numbers have no `more`; 21 numbers end `20 and 1 more`; 25 numbers end `20 and 5 more`; every case ends `; the text may be incomplete`.

## Constraints
- Files not to touch: `mcp/server.rs`, `api/dataplane.rs`, `core/fs_ops.rs`, `storage/*` (the stored copy is US-0242), `tool-contract-golden.json`.
- Dependencies not to add: none; `Content::decode_strict` is in the locked lopdf.
- Patterns to avoid: a second parse of clean pages; changing what the walk or the reader receives; any new refusal; dashes as punctuation in code, comments or messages.
- Scope boundary: cached answers, the `note` column, documentation and the other formats belong to US-0242 and US-0243.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (the green on arrival ones carry their mutation proof).
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit.
- [ ] Non regression: `e2e_0089_`, `e2e_0092_` and the PDF unit tests of `extract.rs` and `pdf_bound.rs` pass unmodified; a clean PDF answer is byte identical (`offload_e2e.rs:1109` still holds); `tool-contract-golden.json` unchanged.
- [ ] Docs: none in this story (US-0243).
- [ ] Self review: the walk receives the same operations as before in every case (clean: strict value, damaged: lenient value); no early return skips the flag; the damage note is never built when `walk` refused.
- [ ] One commit `US-0241: report cut pages in a fresh PDF extraction` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0094` and the requirement ids.
