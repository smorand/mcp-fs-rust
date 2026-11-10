# US-0215: the page total, repeats and saved states

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0214
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A page whose streams, repeats counted, decode to more than 268 435 456 bytes is refused naming the page, before the reader runs it: a form drawn 300 times or a colour profile selected 300 times no longer multiplies what the server decodes or holds. Colour profiles and tint transforms are handed to the reader as empty stand ins, so the copies the reader keeps in saved drawing states hold nothing.

## Requirements covered
- SPEC-0089/FR-NEW-004: IF the streams one page decodes total more than 268 435 456 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> decodes more than 268435456 bytes of PDF streams`.
- SPEC-0089/FR-NEW-006 (partial: the page's decoded data and the copies kept in saved states): THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- SPEC-0089/FR-NEW-007 (partial: item 4 for the page total, and the load before the pages): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_013 e2e_0089_014 e2e_0089_018 e2e_0089_027 e2e_0089_sec_007 e2e_0089_sec_008 pdf_one_profile_kept_in_300_saved_states_holds_nothing`: 7 tests pass. Real assertions: the page number in the refusal, the form text `Hi` at the exact total, `decoded_held` at most 268 500 992 for the refusals, and the document handed to the reader pointing its colour space at an empty stand in while the original profile object keeps its stored bytes.

## Design excerpt
### Decisions
- **DEC-008 (SPEC-0089):** the page total counts, in drawing order, each content stream once, each font's streams on the first selection of its name, each colour profile and tint transform on every selection, each form or image on every draw (nested draws included); a stream's size is measured once per page and reused for its repeats (`sizes`), a form's parsed operations are kept per page (`forms`) and replayed with the current resources; the count restarts at 0 on each page; the refusal names the page number from 1. Rationale: repeats are what the reader decodes again; reuse keeps the walk's own cost to one decode per stream per page. Rejected: measuring every repeat (300 inflations of the same form before the refusal).
- **DEC-009 (SPEC-0089):** at the end of the load, every array whose first element is `/ICCBased` gets its stream reference replaced by a stand in (a new object: the original dictionary without `Filter`, `DecodeParms` and `Length`, empty content), and every `/Separation` array's tint transform stream reference by a stand in with empty content (`FunctionType` 0) or, for `FunctionType` 4, a content set by the walk on first selection: empty when the measured original decodes to valid UTF-8, the single byte `0xFF` otherwise (pdf-extract formats a type 4 function with `String::from_utf8(..).unwrap()` inside a `warn!`, `pdf-extract-0.12.1/src/lib.rs:1215-1220`, enabled by the `log` bridge of `tracing-subscriber`, so validity decides whether it panics today). The walk counts the original's measured size on every selection. The stand ins are what saved states copy (`q` clones the graphics state, `lib.rs:1783`), so a copy holds nothing. pdf-extract keeps the decoded profile in `ColorSpace::ICCBased(Vec<u8>)` (`lib.rs:1425`, `:1446`) and never reads it for text. Rejected: counting `q` copies in the page total (refuses where the spec accepts).
- **DEC-003 (SPEC-0089), this story's part:** the page total is checked after every chunk of a measurement, after the stream limit; a repeat adds its cached size at once and is checked right after.
- **DEC-012 (SPEC-0089):** while the reader runs a page, the hook's `decoded` candidate is that page's decode count.
### Data model
`PdfBudget` gains `page: u64` and `page_no: u32`; per document `stand_ins: HashMap<ObjectId, ObjectId>` (original profile or tint transform to its stand in).
### Contracts
Refusal (no prefix): `page <N> decodes more than 268435456 bytes of PDF streams`, `<N>` in plain digits from 1. `pub(crate) const PAGE_LIMIT: u64 = 268_435_456;` in `pdf_bound.rs`. `fn install_stand_ins(doc: &mut Document) -> HashMap<ObjectId, ObjectId>` (test visible within the crate).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify (and its `#[cfg(test)] mod tests` for the stand in unit test) |
| `crates/core/src/tools/pdf_testkit.rs` | modify |
| `crates/core/src/tools/pdf_bound_e2e.rs` | modify |
### Existing patterns
US-0212's walk (`Tf`, `cs`/`CS`, `Do`), `sizes` and `forms`; US-0214's object stream fixtures for E2E-027.
### Fixtures
`form_pdf(content, form_content, extra)`: page 1 with `/XObject << /Fm0 7 0 R >>`, object 7 `<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Filter /FlateDecode /Length <stored> >>`. E2E-013: form content `BT /F1 12 Tf 72 700 Td (Hi) Tj ET` padded with spaces to 67 108 000 bytes, page content `/Fm0 Do` 4 times padded with spaces to 3 456 bytes (3 456 + 4 x 67 108 000 = 268 435 456); E2E-014 the page content at 3 457 bytes. E2E-018: page 1 `BT /F1 12 Tf 72 700 Td (One) Tj ET`, page 2 `/Fm0 Do` 300 times over a form of 1 000 000 decoded bytes. E2E-027: the object streams of E2E-010 plus page 1 `/Fm0 Do` 300 times over a 1 000 000 byte form. E2E-SEC-007: `/docs/forms.pdf`, page 1 content `/Fm0 Do` written 300 times, form content 1 000 000 spaces, stored in under 100 000 bytes. E2E-SEC-008: `/docs/states.pdf`, page 1 content `/CS0 cs q` written 300 times, `/ColorSpace << /CS0 [/ICCBased 7 0 R] >>`, object 7 `<< /N 1 /Filter /FlateDecode /Length <stored> >>` over 1 000 000 bytes. Take `heavy()` first in every e2e test of this story.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-007: One form drawn 300 times
- **Covers:** occurrence 7
- **Red before the fix because:** `Then the refusal is ...` fails: today the extraction inflates the form 300 times, 300 000 000 bytes, and succeeds.
- **Given** `/docs/forms.pdf`, stored in under 100 000 bytes, whose page 1 content is `/Fm0 Do` written 300 times, `/Fm0` being a form whose compressed content inflates to 1 000 000 spaces
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/forms.pdf: page 1 decodes more than 268435456 bytes of PDF streams`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_007_one_form_drawn_300_times`
- Red before the change because: on US-0214's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-SEC-008: A colour profile kept in 300 saved states
- **Covers:** occurrence 8
- **Red before the fix because:** `And the decoded bytes held never exceed 268 500 992` fails: today each `q` keeps a copy of the inflated profile, 300 000 000 bytes held at once.
- **Given** `/docs/states.pdf` whose page 1 content is `/CS0 cs q` written 300 times, `/CS0` being `[/ICCBased 7 0 R]` and object 7 a compressed profile inflating to 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is the page total message for page 1 of `/docs/states.pdf`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_008_a_colour_profile_kept_in_300_save`
- Red before the change because: on US-0214's code the walk counts the 300 selections without a limit, the reader runs page 1 with a decode count of 300 000 000 (`decoded_held` 300 000 000) and the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-013: A page decoding exactly the page total
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/docs/form.pdf` whose page 1 draws one form 4 times, the page content and the four draws of the form decoding exactly 268 435 456 bytes together, the form showing `Hi`
- **When** the member asks for its text
- **Then** the extraction succeeds and the text holds `Hi`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_013_a_page_decoding_exactly_the_page_tota`
- Green on arrival: US-0214's code accepts it; then the mutation below
- Cleanup: none

### SPEC-0089/E2E-014: A page one byte over the page total
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-006; **Priority:** High
- **Given** `/docs/form.pdf` as in E2E-013 with one more byte in the page content
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/form.pdf: page 1 decodes more than 268435456 bytes of PDF streams`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_014_a_page_one_byte_over_the_page_total`
- Red before the change because: on US-0214's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-018: Page 2 over the page total, page 1 fine
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/two.pdf` whose page 1 reads `One` and whose page 2 draws a form decoding 1 000 000 bytes 300 times
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/two.pdf: page 2 decodes more than 268435456 bytes of PDF streams`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_018_page_2_over_the_page_total_page_1_fin`
- Red before the change because: on US-0214's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-027: Load total met before a page bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order2.pdf` with the object streams of E2E-010 and a page 1 drawing a form decoding 1 000 000 bytes 300 times
- **When** the member asks for its text
- **Then** the refusal is the load total message for `/docs/order2.pdf`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_027_load_total_met_before_a_page_bomb`
- Green on arrival: US-0214 refuses the load first; then the mutation below
- Cleanup: none

### Design test: one profile kept in 300 saved states holds nothing
- Level / Driver / Location: unit / direct call on the loaded document / crates/core/src/docs/pdf_bound.rs (`mod tests`)
- Function: `pdf_one_profile_kept_in_300_saved_states_holds_nothing`
- Given the bytes of a page whose content is `/CS0 cs` then `q` written 300 times, `/CS0 [/ICCBased 7 0 R]`, object 7 a 1 000 000 byte profile / When `pdf_bound::extract` runs on them / Then it succeeds with a page decode count of 1 000 000 (`decoded_held`), and `install_stand_ins` on the loaded document maps object 7 to a stand in whose content is empty and whose dictionary has no `Filter`, while object 7 keeps its stored bytes and `Filter`
- Red before the change because: `install_stand_ins` does not exist
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-013 | refuse when the page count equals the limit (`>=` for `>`): the extraction is refused, the test fails |
| E2E-027 | disable the load total check: the load passes and page 1 is refused with the page total message, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs`
- Dependencies not to add: none
- Never count a `q` as a decode; the stand ins are what makes the copies hold nothing
- No dash as punctuation in code, comments or docs
- Scope boundary: the content limit is US-0216

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-007, E2E-SEC-008, E2E-014, E2E-018 and the stand in unit test (step 1: fixtures and tests on US-0214's code, under `heavy()`), before the production code (step 2: the page total; step 3: the stand ins).
- [ ] Green on arrival tests (E2E-013, E2E-027): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: every earlier `e2e_0089_` and design test, `cargo test -p mcp-fs-core --lib docs::extract tools::artifacts` pass.
- [ ] Docs updated: none in this story.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; failure modes listed in the commit body.
- [ ] One commit `US-0215: the page total, repeats and saved states` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-004, FR-NEW-006, FR-NEW-007).
