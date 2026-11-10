# US-0221: converter refusals through the HTTP door and documentation

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0220
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Prove that the HTTP API answers a converter flood as the assistant tool does: an upload with the conversion flag keeps the document and reports the refusal, documentize refuses and leaves nothing, no converter process and no working folder remain. Close the spec: the documentation states the PDF limits and the converter answer cap, and the full acceptance set and requirement coverage are checked.

## Requirements covered
- SPEC-0089/FR-NEW-010 (partial: documentize through the HTTP API): IF the converter's answer passes the converter limit THEN THE document conversion SHALL stop reading it, end the converter's work, remove its working folder, and refuse with `ERR_INTERNAL_ERROR` and the message `document service output is over the doc_service.max_input_bytes limit of <cap>`.
- SPEC-0089/FR-NEW-013: THE document conversion SHALL NOT write or change a companion, store an artifact, charge the session write allowance for a companion, or record an audit line for a companion, for a conversion refused under FR-NEW-010.
- SPEC-0089/FR-NEW-014: THE product SHALL answer a PDF extraction or a conversion through the HTTP API with the same code, message and effects as through the assistant tool, and SHALL answer a conversion within the converter limit with the same companion, artifacts, report, charge, audit line and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_039 e2e_0089_041`: 2 tests pass. Real assertions: the HTTP status and body, `exists` answers for `/docs/deck.pptx` and `/docs/deck.md`, the stand in's pid gone and its working folder absent, the audit trail. Then the closing checks: `cargo test -p mcp-fs-core --lib -- e2e_0089_ pdf_` runs every test of the spec green, and the requirement coverage audit below passes.

## Design excerpt
### Decisions
- **DEC-018 (SPEC-0089):** the HTTP tests live in `crates/core/src/api/dataplane.rs` tests, `Harness::with_doc_service(Some(Arc::new(cli)))` (`:1637`), project `proj` and member `owner@test.com` standing for `atlas` and `mia@example.com`, the stand in helpers of `crates/core/src/tools/converter_bound_e2e.rs` (`stand_in`, `cli`, the pid and folder probes) imported as `pub(crate)` test items.
- **DEC-023 (SPEC-0089):** the closing story updates `README.md`, `AGENTS.md` (new `docs/pdf_bound.rs` bullet: measured then decoded, the five limits, the `weezl` dependency; `docs/service.rs`: capped answer on every channel; the `e2e_0089_` filter in key commands), `.agent_docs/tools.md` (`:345` and `:438` known gaps BL-0142 and BL-0143 closed, `fs.extract_text` PDF limits and messages, `fs.documentize` answer cap, a dated SPEC-0089 paragraph) and `.agent_docs/testing.md` (filter `e2e_0089_`, `pdf_testkit` builders, the stand in converters and their `perl` requirement, `heavy()`).
- **DEC-024 (SPEC-0089):** the tool contract is unchanged: no description mentions the limits.
### Contracts
`POST /api/fs/{mount}/upload` (multipart, `documentation=true`): status 200, `documentation[0].error.code` `ERR_INTERNAL_ERROR`, `documentation[0].error.message` the cap message (`fs_ops.rs:659-680`, unchanged). `POST /api/fs/{mount}/documentize` `{"path": "/docs/deck.pptx"}`: status 500, `error` `ERR_INTERNAL_ERROR`, `message` the cap message.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0089_039`, `e2e_0089_041`) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
`upload_keeps_the_files_when_the_conversion_fails` (`api/dataplane.rs:2962-2980`, `upload_body`, `upload`), `e2e_0088_029` (`:3936`), `h.post(&u("documentize"), json!(..))`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-039: Upload with conversion, answer over the limit
- **Category:** side effect; **Scenario:** SC-006; **Requirements:** FR-NEW-013; **Priority:** Critical
- **Given** the stand in converter of E2E-034 (`#` then 1 048 576 letters `d`, `max_input_bytes` 1 048 576)
- **When** a client uploads `deck.pptx` (2 bytes `PK`) into `/docs` through the HTTP API with the conversion flag
- **Then** the upload succeeds and `/docs/deck.pptx` exists with its 2 bytes
- **And** the documentation report of that file carries `ERR_INTERNAL_ERROR` and `document service output is over the doc_service.max_input_bytes limit of 1048576`; `/docs/deck.md` does not exist; no converter process of the call runs; no working folder remains
- Level / Driver / Location: functional e2e / HTTP door (`Harness::with_doc_service`, multipart upload) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0089_039_upload_with_conversion_answer_over_th`
- Green on arrival (US-0218 built the cap); mutation below
- Cleanup: none

### SPEC-0089/E2E-041: Documentize through the HTTP API, answer over the limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-014; **Priority:** High
- **Given** `/docs/deck.pptx` stored and the stand in converter of E2E-034
- **When** a client asks the HTTP API for its companion
- **Then** the refusal carries `ERR_INTERNAL_ERROR` and the message of E2E-034; nothing left (no `/docs/deck.md`, no `doc_service` audit line, no process, no working folder)
- Level / Driver / Location: functional e2e / HTTP door (`POST /api/fs/{mount}/documentize`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0089_041_documentize_through_the_http_api_answ`
- Green on arrival; mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-039 | in `CliDocService::convert`, map `Capped::Over` to `ToolError::invalid_argument`: the report carries `ERR_INVALID_ARGUMENT`, the test fails |
| E2E-041 | same mutation: status 400 and `ERR_INVALID_ARGUMENT`, the test fails |

### Closing checks (spec converge)
- Every acceptance test of SPEC-0089 green, run by `make test`: the 49 `E2E-` and 17 `E2E-SEC-` tests (`rg -c 'fn e2e_0089_' crates/core/src` totals 66) and the design tests of US-0212, US-0213 and US-0215.
- Requirement coverage audit: `comm -23 <(rg -o 'FR-NEW-[0-9]{3}' sdlc/specs/SPEC-0089-*/sdlc-spec.md | sort -u) <(rg -o 'SPEC-0089/FR-NEW-[0-9]{3}' sdlc/stories/US-021[2-9]-*.md sdlc/stories/US-022[01]-*.md -N --no-filename | sed 's#.*/##' | sort -u)` prints nothing, and every story US-0212 to US-0220 is `done` or `landed` in `sdlc/stories/_queue.md`.

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, every production file (no production change in this story)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: the backlog entries BL-0147 to BL-0152 stay open

## Definition of done
- [ ] Green on arrival tests (E2E-039, E2E-041): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` clean; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_ e2e_0087_` pass.
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` (DEC-023), each entry dated.
- [ ] Self review: full; failure modes listed in the commit body.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- and E2E-SEC- of SPEC-0089), and every requirement of SPEC-0089 cited by at least one story with status done or landed (the audit above).
- [ ] One commit `US-0221: converter refusals through the HTTP door and documentation` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-010, FR-NEW-013, FR-NEW-014).
