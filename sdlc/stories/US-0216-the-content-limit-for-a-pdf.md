# US-0216: the content limit for a PDF

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0215
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A PDF whose extracted text passes 268 435 456 bytes, page markers included, is refused as soon as the text crosses that limit, with SPEC-0088's content limit message: one character code mapped to a long text and shown two million times no longer builds half a gigabyte of text before the member's character cut. The text is built in one buffer, trimmed in place, so the content held stays under 268 500 992 bytes.

## Requirements covered
- SPEC-0089/FR-NEW-005: IF the extracted content of a PDF passes 268 435 456 bytes THEN THE document text extraction SHALL stop building it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the extracted content exceeds 268435456 bytes`.
- SPEC-0089/FR-NEW-006 (partial: the content held): THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- SPEC-0089/FR-NEW-007 (partial: item 4, FR-NEW-005 at the moment the content crosses its limit): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.
- SPEC-0089/FR-NEW-009 (partial: the note for a PDF without a text layer, the page markers): THE document text extraction SHALL answer a PDF within every limit with the same text, page markers, page count, title, note, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading the PDF when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_015 e2e_0089_016 e2e_0089_028 e2e_0089_sec_011 e2e_0089_006`: 5 tests pass. Real assertions: `pdf_content_held` reads exactly 268 435 456 for the document at the limit (it calibrates the fixture), at most 268 500 992 for the refusals, the content limit message wins over a page 2 bomb, and a page that draws nothing answers the no text layer note. The three existing PDF tests of `docs/extract.rs` still pass.

## Design excerpt
### Decisions
- **DEC-010 (SPEC-0089):** each page runs `output_doc_page(&doc, &mut PlainTextOutput::new(&mut sink as &mut dyn std::io::Write), n)` (pdf-extract's own page text, `ConvertToFmt` for `&mut dyn io::Write`, `pdf-extract-0.12.1/src/lib.rs:2117-2122`); `ContentSink` appends into the one content buffer and refuses (an `io::Error`, seen as `OutputError::FormatError`, then recognized by its flag) a write that would bring the buffer past `CONTENT_LIMIT + 65 536`; after the page, its text is trimmed in place (leading bytes drained, trailing truncated, with `str::trim`'s notion of whitespace) behind the marker `\n\n---\n*[Page <n>]*\n\n` pushed before it (removed with the page when empty), and the content (the buffer minus the two leading newlines the final trim drops) is checked against `CONTENT_LIMIT`: over, the refusal, before the next page's walk. The final body is the buffer trimmed as today (`parts.trim()`, `extract.rs:518-522`). A page error other than the sink's ends the page loop as today (`while let Ok(..)`, `lib.rs:2327-2331`). Rationale: the refusal comes before the crossing piece is appended, one buffer holds the content (no `format!` copy), so the content held stays under 268 500 992; the count after each page makes E2E-028 refuse before page 2. Rejected: an `OutputDev` checking after each character (the piece, up to a whole character map entry, lands first). Edge accepted: a page whose untrimmed text passes the threshold by more than 65 536 bytes of surrounding whitespace is refused although its trimmed text would fit.
- **DEC-012 (SPEC-0089):** `content` in the hook is the content buffer's largest length.
### Data model
`ContentSink { buf: Vec<u8>, threshold: u64, tripped: bool }` (implements `std::io::Write`); the body is `String::from_utf8(buf)` (every piece written is a `&str`, so it is valid).
### Contracts
`PdfText::body` becomes the assembled, trimmed content; `PdfText::pages` keeps each page's trimmed text for the page count. Refusal (no prefix): `the extracted content exceeds 268435456 bytes` (`extract::CONTENT_LIMIT`, `crates/core/src/docs/extract.rs:601`, SPEC-0088's message verbatim). `extract.rs::pdf` takes `body` instead of joining pages itself.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/pdf_testkit.rs` | modify |
| `crates/core/src/tools/pdf_bound_e2e.rs` | modify |
### Existing patterns
US-0212's page loop; `PlainTextOutput` (`lib.rs:2131-2190`); SPEC-0088's content counter (`OfficeBudget::add_content`, `extract.rs:656`).
### Fixtures
`expanding_font(map: &[(u8, &str)])`: `/F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 1 /LastChar 3 /Widths [0 0 0] /ToUnicode 9 0 R >>`, object 9 a `ToUnicode` CMap (`beginbfchar <01> <00650065...> endbfchar`) mapping each code to its text in UTF-16BE; content `BT /F1 12 Tf 72 700 Td <0101...0102> Tj ET` (hex codes). E2E-015: one page, marker `---\n*[Page 1]*\n\n` (16 bytes once the leading newlines are trimmed), then 1 048 575 codes `<01>` mapped to 256 letters `e` and one code `<02>` mapped to 240 letters `e`: 16 + 268 435 200 + 240 = 268 435 456 (the zero widths keep `PlainTextOutput` from inserting spaces; the test's exact `pdf_content_held` assertion is the calibration: adjust the recipe, never the assertion). E2E-016: `<02>` mapped to 241 letters. E2E-028: page 1 as E2E-016, page 2 content a PDF bomb stream. E2E-SEC-011: `<01>` mapped to 128 letters `é` (256 bytes), 2 000 000 codes on page 1. E2E-006: one page whose content is `q Q`, no object stream. Take `heavy()` first in E2E-015, 016, 028 and SEC-011.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-011: One code mapped to a long text, shown 2 000 000 times
- **Covers:** occurrence 11
- **Red before the fix because:** `And the content held never exceeds 268 500 992` fails: today the page text, 512 000 000 bytes, is built whole before the character cut.
- **Given** `/docs/expand.pdf` whose font `/F1` maps code `<01>` to 128 letters `é` (256 bytes) and whose page 1 shows `<0101...01> Tj` with 2 000 000 codes
- **When** the member asks for its text
- **Then** the refusal is the content limit message for `/docs/expand.pdf`
- **And** the content held never exceeds 268 500 992; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_011_one_code_mapped_to_a_long_text_sh`
- Red before the change because: on US-0215's code the page text is built whole, `pdf_content_held` is 512 000 016 and the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-015: Content of exactly the content limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/docs/text.pdf` whose pages, through a character map turning one code into 256 letters `e`, produce exactly 268 435 456 bytes of extracted content, page markers included
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated at 200 000 characters
- **And** `pdf_content_held` is exactly 268 435 456 (fixture calibration)
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_015_content_of_exactly_the_content_limit`
- Green on arrival: US-0215's code accepts it; then the mutation below
- Cleanup: none

### SPEC-0089/E2E-016: Content one byte over the content limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** High
- **Given** `/docs/text.pdf` as in E2E-015 producing one more byte
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/text.pdf: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 268 500 992; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_016_content_one_byte_over_the_content_lim`
- Red before the change because: on US-0215's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-028: Content limit on page 1 before a page 2 bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order3.pdf` whose page 1 produces 268 435 457 bytes of content through the character map of E2E-015 and whose page 2 content is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the content limit message for `/docs/order3.pdf`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_028_content_limit_on_page_1_before_a_page`
- Red before the change because: on US-0215's code page 1 passes and page 2 is refused with the stream limit message
- Cleanup: none

### SPEC-0089/E2E-006: PDF without a text layer
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/scan.pdf`, one page whose content draws nothing, and no compressed object stream
- **When** the member asks for its text
- **Then** the text is empty, 1 page, with the note `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_006_pdf_without_a_text_layer`
- Green on arrival: today's behaviour, kept by US-0212; then the mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-015 | refuse when the content equals the limit (`>=` for `>` in the after page check): the extraction is refused, the test fails |
| E2E-006 | drop an empty page's marker removal (keep `\n\n---\n*[Page 1]*\n\n` for an empty page): the text is not empty and the note is missing, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs`, the three PDF tests of `docs/extract.rs`
- Dependencies not to add: none
- Never build a page's text with `format!` or a second buffer; trim in place
- No dash as punctuation in code, comments or docs
- Scope boundary: order, trace, both doors are US-0217

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-011, E2E-016, E2E-028 (step 1: fixtures and tests on US-0215's code, under `heavy()`; the SEC-011 red run builds 512 000 016 bytes of text), before the production code (step 2: `ContentSink` and the in place assembly).
- [ ] Green on arrival tests (E2E-015, E2E-006): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion` and every earlier `e2e_0089_` and design test pass.
- [ ] Docs updated: none in this story.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; failure modes listed in the commit body.
- [ ] One commit `US-0216: the content limit for a PDF` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-009).
