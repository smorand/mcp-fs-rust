# US-0217: PDF refusals in order, without trace, at both doors

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0216
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
Prove, through both doors, that the bounded PDF text pass answers a reasonable PDF exactly as before, that membership and a missing path still come first, that a refusal leaves no companion, no artifact, no charge and no audit line and is logged once at the informational level, that repeating or refreshing behaves as specified, and that concurrent calls are measured separately. Records the PDF bound in `.agent_docs/architecture.md`.

## Requirements covered
- SPEC-0089/FR-NEW-002 (partial: the upper case suffix and the HTTP door): IF a stream of a PDF decodes to more than 104 857 600 bytes THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF stream decodes to more than 104857600 bytes`.
- SPEC-0089/FR-NEW-006 (partial: three concurrent calls): THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- SPEC-0089/FR-NEW-007 (partial: items 1 and 2): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.
- SPEC-0089/FR-NEW-008: THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-005.
- SPEC-0089/FR-NEW-009 (partial: a PDF within the limits, an unreadable PDF, the cached companion): THE document text extraction SHALL answer a PDF within every limit with the same text, page markers, page count, title, note, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading the PDF when no refresh is asked, and SHALL extract every other format exactly as before.
- SPEC-0089/FR-NEW-014 (partial: PDF extraction through the HTTP API): THE product SHALL answer a PDF extraction or a conversion through the HTTP API with the same code, message and effects as through the assistant tool, and SHALL answer a conversion within the converter limit with the same companion, artifacts, report, charge, audit line and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_001 e2e_0089_002 e2e_0089_019 e2e_0089_020 e2e_0089_021 e2e_0089_022 e2e_0089_023 e2e_0089_024 e2e_0089_025 e2e_0089_029 e2e_0089_030 e2e_0089_031`: 12 tests pass. Real assertions: previews, page counts and titles; the companion bytes; the listing of `/docs`, the artifact lists, the audit trail and a following 52 428 800 byte write; the captured log lines; the HTTP status and body; `decoded_held` per path for three joined calls.

## Design excerpt
### Decisions
- **DEC-013 (SPEC-0089):** refusals are `ToolError::invalid_argument` with the five messages without prefix; a load error is `pdf: PDF error: <e>`, a panic `pdf parser failed on this document`; `prepare_extraction` (membership checked by the doors before, `not a file`, the cached companion) is unchanged and runs before `pdf_bound` (`crates/core/src/docs/extract.rs:318-381`).
- **DEC-012 (SPEC-0089):** the hook map `PDF_HELD` is keyed by path, so three joined calls on one thread keep three figures; a cached call leaves no entry (reads 0).
- **DEC-025 (SPEC-0089):** the log lines come from the existing wiring (`to_call_result` and `error_response` call `log_tool_failure`, SPEC-0088 DEC-012): INFO for the `ERR_INVALID_ARGUMENT` PDF refusal; no new log call.
- **DEC-018 (SPEC-0089):** E2E-030 is a plain `#[test]` on a dedicated current thread runtime under `crate::logging::capture::lock_for_test`, calling `McpServer::new(state, person).fs_extract_text(Parameters(ExtractTextArgs { .. }))` (model `crates/core/src/tools/office_bound_e2e.rs:943-979`); E2E-002 and E2E-023 run in the tests of `crates/core/src/api/dataplane.rs` (`Harness`, project `proj` and member `owner@test.com` stand for `atlas` and `mia@example.com`, model `e2e_0088_004` at `:3876`).
- **DEC-023 (SPEC-0089), this story's part:** a dated "PDF extraction bound (SPEC-0089)" paragraph in `.agent_docs/architecture.md`: measured then decoded, the five limits and their order, defuse and arm, the colour profile stand ins, the cross reference guard and the encrypted path, the hook.
### Contracts
No new contract: this story exercises the ones of US-0212 to US-0216.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/pdf_bound_e2e.rs` | modify |
| `crates/core/src/tools/pdf_testkit.rs` | modify (`report_pdf()`: two pages `Quarterly results` and `Outlook`, title `Report 2026`, every stream compressed, an object stream holding the `/Info` dictionary) |
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0089_002`, `e2e_0089_023`) |
| `.agent_docs/architecture.md` | modify |
### Existing patterns
`office_bound_e2e.rs`: E2E-035 (`:943-979`, log capture), E2E-036 (`:985-1008`, three joined calls with `add_second_member` and `THIRD` added by `f.state.admin.add_member(MOUNT, THIRD, OWNER)`), the cached and refresh tests (`:869-920`). `api/dataplane.rs` `e2e_0088_004` and `e2e_0088_029` (`:3876-3960`). `archive_testkit::nothing_left` (companion byte for byte when one existed).
### Fixtures
`/docs/bomb.pdf` is E2E-SEC-001's document (`pdf_testkit`); `/docs/report.pdf` is `report_pdf()`; `/docs/bad.pdf` is `%PDF-1.4` then the word `garbage`; `/docs/bomb.md` holding `kept text` is written after the PDF with `seed` (its modification time later). Take `heavy()` first in every test that stores a bomb.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

Every test of this story is green on arrival: the behaviour is built by US-0212 to US-0216 or is today's; each is proven by its mutation below (DEC-014).

### SPEC-0089/E2E-001: PDF within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/report.pdf`, two pages reading `Quarterly results` and `Outlook`, title `Report 2026`, every stream compressed
- **When** the member asks for its text
- **Then** the answer names `/docs/report.md`, format `pdf`, 2 pages, title `Report 2026`, not cached; its preview reads `*[Page 1]*`, `Quarterly results`, `*[Page 2]*`, `Outlook`, in that order
- **And** `/docs/report.md` holds that text; the audit trail has one extraction line for `/docs/report.md`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_001_pdf_within_the_limits`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-002: PDF through the HTTP API
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-014; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text (`POST /api/fs/{mount}/extract-text`, `{"path": "/docs/report.pdf"}`)
- **Then** the answer carries the same companion path, format, page count, title and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/report.md`
- Level / Driver / Location: functional e2e / HTTP door (`Harness`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0089_002_pdf_through_the_http_api`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-019: A PDF that cannot be read
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/bad.pdf` holding `%PDF-1.4` then the word `garbage`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `could not extract /docs/bad.pdf: `, as today
- **And** nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_019_a_pdf_that_cannot_be_read`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-020: Up to date companion answers without reading a PDF bomb
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/bomb.pdf` whose page 1 content is a PDF bomb stream, and `/docs/bomb.md` holding `kept text`, written after it
- **When** the member asks for the text of `/docs/bomb.pdf` without refresh
- **Then** the answer is cached, names `/docs/bomb.md` and previews `kept text`
- **And** the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_020_up_to_date_companion_answers_without`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-021: Refresh on a PDF bomb keeps the companion
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** High
- **Given** the situation of E2E-020
- **When** the member asks for the text of `/docs/bomb.pdf` with refresh
- **Then** the refusal is the stream limit message for `/docs/bomb.pdf`
- **And** `/docs/bomb.md` still reads `kept text`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_021_refresh_on_a_pdf_bomb_keeps_the_compa`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-022: Repeating a refused PDF call
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-020 without companion
- **When** the member asks for its text twice
- **Then** both answers are the same stream limit refusal; nothing left after the second
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_022_repeating_a_refused_pdf_call`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-023: PDF bomb through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-014; **Priority:** Critical
- **Given** `/docs/bomb.pdf` of E2E-022
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` (status 400) and `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; no `/docs/bomb.md`, no audit line for it
- Level / Driver / Location: functional e2e / HTTP door (`Harness`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0089_023_pdf_bomb_through_the_http_api`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-024: Not a member, PDF bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-022 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text (the doors call `f.state.authorize(MOUNT, "eve@example.com")` first)
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the decoded bytes held are 0
- Level / Driver / Location: functional e2e / `AppState::authorize` then, never reached, the typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_024_not_a_member_pdf_bomb`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-025: Missing path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** no file `/docs/none.pdf`
- **When** the member asks for its text
- **Then** the refusal is `ERR_NOT_FOUND` with `not a file: /docs/none.pdf`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_025_missing_path`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-029: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/BOMB.PDF`, the bytes of `/docs/bomb.pdf` of E2E-022
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/BOMB.PDF`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_029_upper_case_suffix`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-030: What the operator sees for a PDF refusal
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-022
- **When** the member asks for its text through the assistant tool
- **Then** the server log has exactly one informational line for the call naming `ERR_INVALID_ARGUMENT` and `a PDF stream decodes to more than 104857600 bytes`, and no error level line
- **And** the audit trail has no line for `/docs/bomb.pdf` or `/docs/bomb.md`
- Level / Driver / Location: functional e2e / assistant tool method `McpServer::fs_extract_text`, log capture / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_030_what_the_operator_sees_for_a_pdf_refu`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-031: Two PDF bombs and a normal PDF at once
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006, FR-NEW-009; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with a copy of `/docs/bomb.pdf` of E2E-022 named `/docs/m.pdf` and `/docs/n.pdf`, and `ola@example.com` with `/docs/report.pdf` of E2E-001
- **When** the three ask for their texts at the same time (`tokio::join!`)
- **Then** the two bombs are refused with the stream limit message and the report succeeds as in E2E-001
- **And** each bomb call's decoded bytes held never exceed 104 933 136
- Level / Driver / Location: functional e2e / typed engine call, three calls joined / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `e2e_0089_031_two_pdf_bombs_and_a_normal_pdf_at_onc`
- Green on arrival; mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-001 | skip arming the page's content streams: the reader parses their compressed bytes, the preview no longer reads `Quarterly results`, the test fails |
| E2E-002 | drop the `title` meta in `extract.rs::pdf`: the HTTP answer has no title, the test fails |
| E2E-019 | answer a load error with an empty text instead of the refusal: the call succeeds, the test fails |
| E2E-020 | in `prepare_extraction`, treat every call as a refresh: the bomb is read and refused, the test fails |
| E2E-021 | in `prepare_extraction`, ignore `refresh`: the cached text answers, the test fails |
| E2E-022 | on a refusal, write the companion before returning the error: `/docs/bomb.md` appears, the test fails |
| E2E-023 | refuse the stream limit with `ToolError::internal`: status 500 and `ERR_INTERNAL_ERROR`, the test fails |
| E2E-024 | make `AppState::authorize` admit any person: the test's `unwrap_err` panics, the test fails |
| E2E-025 | in `prepare_extraction`, skip the `is_file` check: the missing path fails later with another message, the test fails |
| E2E-029 | route only the lower case `.pdf` to `pdf`: `/docs/BOMB.PDF` is decoded as text and succeeds, the test fails |
| E2E-030 | remove the `log_tool_failure` call in `to_call_result`: no informational line, the test fails |
| E2E-031 | skip arming the page's content streams: the report's preview fails, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/docs/pdf_bound.rs` (no production change in this story)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: the converter is US-0218 to US-0221

## Definition of done
- [ ] Green on arrival tests (all 12): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: every `e2e_0088_` and `e2e_0089_` test, `cargo test -p mcp-fs-core --lib api::dataplane docs::extract tools::artifacts` pass.
- [ ] Docs updated: `.agent_docs/architecture.md` (dated PDF extraction bound paragraph, DEC-023).
- [ ] Self review: every new branch has a test; failure modes listed in the commit body.
- [ ] One commit `US-0217: PDF refusals in order, without trace, at both doors` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-014).
