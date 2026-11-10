# US-0220: cap the web converter's answer

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0219
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A web converter whose answer body passes the converter limit, as plain Markdown or as a bundle, is abandoned as soon as the body passes the limit and refused with today's bundle message; a failure status keeps today's message, quoting the head of a body read no further than the limit. `.agent_docs/config.md` says that `doc_service.max_input_bytes` now caps every answer channel.

## Requirements covered
- SPEC-0089/FR-NEW-010 (partial: the web answer, whatever its form): IF the converter's answer passes the converter limit THEN THE document conversion SHALL stop reading it, end the converter's work, remove its working folder, and refuse with `ERR_INTERNAL_ERROR` and the message `document service output is over the doc_service.max_input_bytes limit of <cap>`.
- SPEC-0089/FR-NEW-011 (partial: the web answer held, a failure status read no further than the limit): THE document conversion SHALL hold, during one conversion, at most the converter limit plus 65 536 bytes of the converter's answer, at most 73 728 bytes of its diagnostic output, and at most 65 536 bytes of its standard output in bundle mode.
- SPEC-0089/FR-NEW-012 (partial: a web failure status before the body limit): IF several refusals apply to one conversion THEN THE document conversion SHALL report the first met in this order: (1) membership; (2) not a file (documentize); (3) converter not configured or extension not accepted (`ERR_NOT_SUPPORTED`, today's messages); (4) the document over the converter limit (today's message); (5) the SPEC-0041 Office check; (6) the companion exists and overwrite is not asked (documentize, `ERR_NO_CLOBBER`); (7) during the conversion, whichever comes first: the answer passing the converter limit (FR-NEW-010), the timeout; then, once the converter ended within the limit, its failure exit, a web failure status, an empty answer or a bundle without its Markdown, with today's messages.
- SPEC-0089/FR-NEW-013 (partial: a web refusal): THE document conversion SHALL NOT write or change a companion, store an artifact, charge the session write allowance for a companion, or record an audit line for a companion, for a conversion refused under FR-NEW-010.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_005 e2e_0089_035 e2e_0089_036 e2e_0089_043 e2e_0089_sec_015 e2e_0089_sec_016`: 6 tests pass. Real assertions: the `CONVERTER_HELD` records (`answer` at most 1 114 112), the web stand in's count of bytes sent (below 1 073 741 824 when the server abandoned the body), the companion bytes, the listing of `/docs` and the audit trail. The 7 api mode tests of `docs/service.rs` still pass.

## Design excerpt
### Decisions
- **DEC-016 (SPEC-0089):** status and content type are read first (`crates/core/src/docs/service.rs:378-385`); the body through `read_body_capped(&mut response, max_input_bytes)` (`Response::chunk()` until `None`; a chunk that would pass the cap returns `Capped::Over` without being appended; a transport error mid body yields an empty body, as today's `text().await.unwrap_or_default()`); the response is then dropped, which closes the connection. Failure status: `head_of` of the lossy text of what was read, `Over` included (read no further than the cap); success and `Over`: `over_cap(cap)`; success: the text is `String::from_utf8` of the bytes (no copy) or the lossy repair, then `parse_api_bundle`, the plain Markdown or the named field as today. reqwest is built without its `charset` feature, so `text()` was `from_utf8_lossy` (`reqwest-0.12.28/src/async_impl/response.rs:169-174`) and the text is identical.
- **DEC-017 (SPEC-0089):** `ConverterHeld.answer` is the largest body buffer length.
- **DEC-019 (SPEC-0089), this story's part:** `web(status, content_type, prefix, letter, n, suffix) -> (url, sent: Arc<AtomicU64>)`: an axum route `POST /convert` answering `Body::from_stream` of 65 536 byte chunks (`futures::stream::unfold`), adding each chunk's length to `sent` as it is pulled; model `stub_server` (`docs/service.rs:1048-1087`).
- **DEC-023 (SPEC-0089), this story's part:** `.agent_docs/config.md:166` (`max_input_bytes`): also caps the converter's answer on every channel (text stdout, web body, bundle files) and is the limit named by the refusal.
### Contracts
`async fn read_body_capped(response: &mut reqwest::Response, cap: u64) -> std::result::Result<Vec<u8>, Capped>`; messages unchanged: `document service returned HTTP <status>: <head>` (`BODY_CAP` 2 048 characters), `over_cap(cap)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/service.rs` | modify |
| `crates/core/src/tools/converter_bound_e2e.rs` | modify |
| `.agent_docs/config.md` | modify |
### Existing patterns
`ApiDocService::convert` (`docs/service.rs:355-422`), `parse_api_bundle` (`:552-588`), `head_of` (`:598-604`); `api_service` (`:1089-1091`, `ApiDocService::new(&cfg, resolve_extensions(&[]), u64::MAX)`: here `1_048_576`); US-0218's `converter_fixture` and nothing left helper.
### Fixtures
E2E-005: status 200, `application/json`, body `{"markdown":"# Deck"}`. E2E-035: 200, `text/markdown`, `#` then 1 048 575 letters `d`; E2E-036: 1 048 576 letters. E2E-043: 500, `text/plain`, `upstream broke` then 1 073 741 824 letters `x`. E2E-SEC-015: 200, `text/markdown`, 1 073 741 824 letters `a`. E2E-SEC-016: 200, `application/json`, `{"markdown":"` then 1 073 741 824 letters `a` then `"}`. Take `heavy()` first in SEC-015, SEC-016 and E2E-043.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-015: Web answer flood
- **Covers:** occurrence 16
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the 1 073 741 824 byte body is read whole.
- **Given** a web stand in converter answering, with success status, 1 073 741 824 letters `a` as plain Markdown
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; the stand in converter records that the server closed the connection before 1 073 741 824 bytes were sent; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::documentize` with a real `ApiDocService` / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_sec_015_web_answer_flood`
- Red before the change because: step 1 (hook recording today's whole body): `answer` is 1 073 741 824 and `sent` reaches 1 073 741 824
- Cleanup: none

### SPEC-0089/E2E-SEC-016: Web bundle answer flood
- **Covers:** occurrence 17
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the whole JSON body is read before the bundle files are counted.
- **Given** a web stand in converter answering, with success status, a bundle written `{"markdown":"` followed by 1 073 741 824 letters `a` and `"}`
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; the stand in converter records the connection closed early; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_sec_016_web_bundle_answer_flood`
- Red before the change because: after step 1 `answer` is 1 073 741 839
- Cleanup: none

### SPEC-0089/E2E-005: Web endpoint conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a web stand in converter answering a bundle with the Markdown `# Deck` and no file
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds `# Deck`; the answer reports 6 bytes written
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_005_web_endpoint_conversion_within_the_li`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-035: Web answer of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** a web stand in converter answering, as plain Markdown, `#` then 1 048 575 letters `d`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` is written with 1 048 576 bytes
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_035_web_answer_of_exactly_the_converter_l`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-036: Web answer one byte over the converter limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-011; **Priority:** Critical
- **Given** the web stand in converter of E2E-035 answering one more letter
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_036_web_answer_one_byte_over_the_converte`
- Red before the change because: today the companion is written with 1 048 577 bytes
- Cleanup: none

### SPEC-0089/E2E-043: Web failure status with a huge body
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-011, FR-NEW-012; **Priority:** High
- **Given** a web stand in converter answering failure status 500 with a body of `upstream broke` followed by 1 073 741 824 letters `x`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service returned HTTP 500: upstream broke` and quoting at most 2 048 characters of the body
- **And** the answer held never exceeds 1 114 112; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_043_web_failure_status_with_a_huge_body`
- Red before the change because: after step 1 `answer` is 1 073 741 838 (the message itself already holds)
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-005 | treat an `application/json` answer as plain Markdown: `/docs/deck.md` holds the JSON text, the test fails |
| E2E-035 | refuse when the body would reach the cap (`>=` for `>`): the exact limit is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/core/fs_ops.rs`, the api mode tests of `docs/service.rs`
- Dependencies not to add: none (`futures` and `axum` are already dependencies)
- Never call `text()` or `bytes()` on the response; never log a header or the token
- No dash as punctuation in code, comments or docs
- Scope boundary: the HTTP door tests and the closing documentation are US-0221

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-015, E2E-SEC-016, E2E-036, E2E-043 (step 1: the web stand in, the tests and the hook around today's body read, under `heavy()`), before the production code (step 2: `read_body_capped`).
- [ ] Green on arrival tests (E2E-005, E2E-035): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::service` (`api_non_2xx_carries_the_status`, `api_json_answer_is_a_bundle`, `api_reads_the_body_or_the_configured_json_field`) and every `e2e_0089_` test pass.
- [ ] Docs updated: `.agent_docs/config.md` (`max_input_bytes` caps every answer channel).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; failure modes listed in the commit body.
- [ ] One commit `US-0220: cap the web converter's answer` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013).
