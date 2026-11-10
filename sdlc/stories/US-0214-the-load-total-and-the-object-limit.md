# US-0214: the load total and the object limit

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0213
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A PDF whose compressed object streams decode to more than 268 435 456 bytes together, or which holds more than 1 000 000 objects, is refused during the load, in the order the load reads them, before any page is read. Two million objects packed in a small file are refused on their count, before they are parsed.

## Requirements covered
- SPEC-0089/FR-NEW-001: IF a PDF holds more than 1 000 000 objects THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the PDF holds more than 1000000 objects`.
- SPEC-0089/FR-NEW-003: IF the compressed object streams of a PDF decode to more than 268 435 456 bytes together THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the PDF object streams decode to more than 268435456 bytes together`.
- SPEC-0089/FR-NEW-006 (partial: the load's decoded data): THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- SPEC-0089/FR-NEW-007 (partial: item 3, the load): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_009 e2e_0089_010 e2e_0089_011 e2e_0089_012 e2e_0089_026 e2e_0089_sec_009 e2e_0089_sec_010`: 7 tests pass. Real assertions: the refusal messages name the load total or the object limit, the documents at the limits succeed with their page text, `decoded_held` stays at or under 268 500 992, and nothing is left after a refusal.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0089), this story's part:** after the load, objects outside object streams are counted (`doc.objects.len()`, FR-NEW-001); then, in increasing object number, each deferred stream is measured (stream limit, then the load total on the running sum `budget.load`), decoded with `decompressed_content()`, its objects counted as the number of number pairs in `content[..First]` split on whitespace, as `ObjectStream::new` splits them (`lopdf-0.42.0/src/object_stream.rs:60-70`) (a pair is an object whatever its number), checked against `OBJECT_LIMIT` before `ObjectStream::new` parses them; equal is accepted for both limits. Counting `/N` (the attacker's claim) and counting distinct numbers (the spec's E2E-026 and E2E-SEC-010 fixtures cannot hold 1 000 001 distinct numbers in their sizes) were rejected. The container's bytes are emptied once parsed, so one object stream's decoded bytes are held at a time.
- **DEC-002 (SPEC-0089):** `LOAD_LIMIT` 268 435 456 and `OBJECT_LIMIT` 1 000 000 are `pub(crate) const` in `pdf_bound.rs`; no scaling.
- **DEC-003 (SPEC-0089), this story's part:** the load total is checked after every 65 536 byte chunk of the measurement, after the stream limit, so decoding stops at most one chunk past it.
- **DEC-013 (SPEC-0089):** order: cross reference guard, load, objects outside, object streams (stream, load total, objects), then pages.
- **DEC-019 (SPEC-0089), this story's part:** `PdfWriter::object_stream(number, objects: &[(u32, &str)], padding: usize)` and a pair repeating variant `object_stream_pairs(number, pair: (u32, u32), count: usize, object: &str)` writing `count` copies of the same `n off` pair (all at offset 0, one `null`), deflated through `zlib_run` from cached segments where the text repeats.
### Data model
`PdfBudget` gains `load: u64` (object stream bytes measured so far) and `objects: u64`.
### Contracts
Refusals (no prefix): `the PDF object streams decode to more than 268435456 bytes together`; `the PDF holds more than 1000000 objects`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/tools/pdf_testkit.rs` | modify |
| `crates/core/src/tools/pdf_bound_e2e.rs` | modify |
### Existing patterns
US-0212's `load`, `measure`, `defer_object_streams`; `part_refusal` style helper in `pdf_bound_e2e.rs`; `archive_testkit::nothing_left`, `heavy`.
### Fixtures
E2E-009: three object streams numbered 20, 21, 22 holding one dictionary each (`<< /Pad 1 >>`, objects 30, 31, 32) padded with spaces to 100 000 000, 100 000 000 and 68 435 456 decoded bytes, one page reading `Fine` (`BT /F1 12 Tf 72 700 Td (Fine) Tj ET`); E2E-010 the third at 68 435 457. E2E-011: one page reading `Many`, its outside objects counted, and object streams holding `null` objects with distinct numbers so that outside plus pairs is exactly 1 000 000; E2E-012 one pair more. E2E-026: object stream 5 with 1 000 001 pairs `1 0` (within 10 000 000 decoded bytes) and object stream 9 a PDF bomb stream. E2E-SEC-009: three object streams of 100 000 000 decoded bytes each, file under 1 000 000 bytes. E2E-SEC-010: object streams holding 2 000 000 pairs `1 0` over one `null`, file under 1 000 000 bytes. Take `heavy()` first in all 7 tests.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-009: Three object streams over the load total
- **Covers:** occurrence 9
- **Red before the fix because:** `And the decoded bytes held never exceed 268 500 992` fails: today the three object streams, 300 000 000 bytes, are inflated and kept for the whole call.
- **Given** `/docs/objs3.pdf` with three object streams each inflating to 100 000 000 bytes, stored in under 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is the load total message for `/docs/objs3.pdf`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_009_three_object_streams_over_the_loa`
- Red before the change because: on US-0213's code the three streams are each under the stream limit, the extraction succeeds (`Then` fails); the held figure already stays at 100 000 000, since US-0212 frees each object stream once parsed (recorded in the commit body)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0089/E2E-SEC-010: Two million objects
- **Covers:** occurrence 10
- **Red before the fix because:** `Then the refusal is ...` fails: today the 2 000 000 objects are parsed into the server's object table and the extraction succeeds.
- **Given** `/docs/nulls.pdf`, stored in under 1 000 000 bytes, whose object streams hold 2 000 000 objects written `null`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/nulls.pdf: the PDF holds more than 1000000 objects`
- **And** nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_sec_010_two_million_objects`
- Red before the change because: on US-0213's code the pairs are parsed and the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-009: Object streams totalling exactly the load total
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/objs.pdf` with three compressed object streams inflating to 100 000 000, 100 000 000 and 68 435 456 bytes (objects padded with spaces), one page reading `Fine`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview reads `*[Page 1]*` then `Fine`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_009_object_streams_totalling_exactly_the`
- Green on arrival: US-0213's code accepts it; then the mutation below
- Cleanup: none

### SPEC-0089/E2E-010: Object streams one byte over the load total
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/objs.pdf` as in E2E-009 with its third object stream inflating to 68 435 457 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/objs.pdf: the PDF object streams decode to more than 268435456 bytes together`
- **And** the decoded bytes held never exceed 268 500 992; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_010_object_streams_one_byte_over_the_load`
- Red before the change because: on US-0213's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-011: Exactly 1 000 000 objects
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/docs/many.pdf` holding exactly 1 000 000 objects in total, the padding objects being `null` objects in compressed object streams, one page reading `Many`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview reads `*[Page 1]*` then `Many`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_011_exactly_1_000_000_objects`
- Green on arrival: US-0213's code accepts it; then the mutation below
- Cleanup: none

### SPEC-0089/E2E-012: 1 000 001 objects
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/docs/many.pdf` as in E2E-011 with one more `null` object
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/many.pdf: the PDF holds more than 1000000 objects`
- **And** nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_012_1_000_001_objects`
- Red before the change because: on US-0213's code the extraction succeeds
- Cleanup: none

### SPEC-0089/E2E-026: Object count met before a later object stream bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order1.pdf` whose object stream number 5 holds 1 000 001 `null` objects within 10 000 000 decoded bytes and whose object stream number 9 is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the object limit message for `/docs/order1.pdf`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_026_object_count_met_before_a_later_objec`
- Red before the change because: on US-0213's code object stream 5 passes and object stream 9 is refused with the stream limit message
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-009 | refuse when the load total equals the limit (`>=` for `>`): the extraction is refused, the test fails |
| E2E-011 | refuse when the object count equals the limit (`>=` for `>`): the extraction is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs`
- Dependencies not to add: none
- Count pairs before `ObjectStream::new` parses them; never count `/N`
- No dash as punctuation in code, comments or docs
- Scope boundary: the page total is US-0215, the content limit US-0216

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-009, E2E-SEC-010, E2E-010, E2E-012, E2E-026 (step 1: fixtures and tests on US-0213's code, under `heavy()`), before the production code (step 2: the load total and the object count).
- [ ] Green on arrival tests (E2E-009, E2E-011): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: every earlier `e2e_0089_` and design test, `cargo test -p mcp-fs-core --lib docs::extract tools::artifacts` pass.
- [ ] Docs updated: none in this story.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; counts in `u64` with checked or saturating addition; failure modes listed in the commit body.
- [ ] One commit `US-0214: the load total and the object limit` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-001, FR-NEW-003, FR-NEW-006, FR-NEW-007).
