# US-0230: extraction latency through the HTTP door and documentation

> Spec: SPEC-0091 (sdlc/specs/SPEC-0091-keep-requests-answered-while-documents-are-extracted/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0229
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Prove the HTTP API keeps other requests answered during a large extraction, answers a limit refusal and an unexpected stop as the assistant tool does, and leaves nothing when its client closes the connection; prove the SPEC-0088 and SPEC-0089 suites pass with their files untouched. Close the spec with the documentation.

## Requirements covered
- SPEC-0091/FR-NEW-006: The server SHALL apply FR-NEW-001 to FR-NEW-005 alike to extractions asked through the assistant tool and through the HTTP API.
- SPEC-0091/FR-NEW-001 (partial: through the HTTP API): WHILE one or more documents are being decoded for text extraction THE server SHALL add at most 250 ms to the answer time of every request that decodes no document, compared with the same request when no document is decoding.
- SPEC-0091/FR-NEW-003 (partial: the existing suites unmodified, a refusal through the HTTP API): The document text extraction SHALL answer, for the same document, options and member, exactly what it answers today: text, preview, character count, truncation, notes, companion, table and image artifacts, audit line, write charge, and every refusal code and message.
- SPEC-0091/FR-NEW-004 (partial: through the HTTP API): IF a reader stops before giving a result THEN THE document text extraction SHALL refuse that request with `ERR_INTERNAL_ERROR` and the message `could not extract <path>: the document reader stopped unexpectedly`, leave no companion, artifact, audit line or charge, log one error line naming the code and the path, and free its place.
- SPEC-0091/FR-NEW-005 (partial: through the HTTP API): WHEN a member abandons an extraction request while its document is decoding or waiting for a place THE document text extraction SHALL write no companion, store no artifact, record no audit line and charge nothing for it.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0091_011 e2e_0091_019 e2e_0091_020 e2e_0091_027 --test-threads=1` (4 pass) and the E2E-017 command below. Real assertions: HTTP status and body, probe times, the listing, audit trail and write after a closed socket, and `git diff` of the existing suites' files empty.

## Design excerpt
### Decisions
- **DEC-006 (SPEC-0091):** a panic of the decoding closure (a `JoinError` that `is_panic()`) is answered `ToolError::internal("could not extract <path>: the document reader stopped unexpectedly")` by `prepare_extraction`, which passes this error through instead of re wrapping it as `ERR_INVALID_ARGUMENT`; the closure's place is already free (DEC-003); nothing was written. A `JoinError` that is a cancellation (runtime shutdown) is answered the same way. Until US-0229, US-0228 resumes the panic on the request side (`std::panic::resume_unwind`), today's behaviour. The PDF reader's own `catch_unwind` (`extract.rs:519`) is unchanged: a panic it contains stays `pdf parser failed on this document`. **Obliged by:** SPEC-0091/FR-NEW-004, DI-6. **Rationale:** the message is worded once, at the place that knows the path.
- **DEC-008 (SPEC-0091):** the probes: the operator's health check is `GET /health` on the router built with the state; noa's listing is `McpServer::fs_list_dir` of `/notes`; noa's read is `McpServer::fs_read` of `/notes/todo.txt`. A probe loop runs as a spawned task sending one probe every 100 ms from 50 ms after the extraction started until a `watch` channel says stop, recording each answer time. "Abandon" is aborting the task that awaits `McpServer::fs_extract_text` (the transport drops that future when a client cancels), and, at the HTTP door, closing the client socket of a served router (hyper drops the handler future when it reads the end of the connection). **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-005, FR-NEW-006. **Rationale:** real handlers, no stand in.
- **DEC-010 (SPEC-0091):** documentation: `.agent_docs/architecture.md` (US-0229) states where decoding runs, the ceiling and the stop; the closing story updates `README.md`, `AGENTS.md` (`docs/extract.rs` bullet), `.agent_docs/tools.md` (`fs.extract_text`: other requests unaffected, the stop message) and `.agent_docs/testing.md` (filter `e2e_0091_`, the multi thread timing tests and `heavy()`). No tool contract change. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing pattern.
### Contracts
`POST /api/fs/{mount}/extract-text`: an unexpected stop answers status 500 `{"error": "ERR_INTERNAL_ERROR", "detail": "ERR_INTERNAL_ERROR: could not extract /docs/report.pdf: the document reader stopped unexpectedly"}`; a stream limit refusal answers status 400 with the SPEC-0089 detail (`crates/core/src/api/dataplane.rs:259-266`, unchanged). E2E-027 serves the router on `127.0.0.1:0` with `axum::serve` and sends the request on a raw `tokio::net::TcpStream`, dropped 1 s later.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0091_011`, `019`, `020`, `027`) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
`e2e_0088_029` (`crates/core/src/api/dataplane.rs:3936`), `e2e_0089_002` (`:3961`), `Harness`, `h.post(&u("extract-text"), json!(..))`; US-0228's probes and US-0229's fault seam (`h.state.safety.decode_places().inject_fault(..)`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0091/E2E-011: Unexpected stop through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-006; **Priority:** High
- **Given** as E2E-009
- **When** mia asks for the text through the HTTP API
- **Then** the answer is a server error whose error is `ERR_INTERNAL_ERROR` and whose detail is `ERR_INTERNAL_ERROR: could not extract /docs/report.pdf: the document reader stopped unexpectedly`
- **And** nothing left
- Level / Driver / Location: functional e2e / HTTP door, current thread runtime / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0091_011_unexpected_stop_through_the_http_api`
- Green on arrival: US-0229 maps the stop at the engine both doors share; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-017: Every refusal and limit as recorded
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** the 48 SPEC-0088 tests, the 50 SPEC-0089 tests and the 51 extraction unit tests, unmodified
- **When** they run against this increment
- **Then** all pass, including every decoded bytes and content held figure they assert
- **And** none of their files changed in this increment
- Level / Driver / Location: functional e2e / the existing suites as they are / crates/core/src/tools/office_bound_e2e.rs, pdf_bound_e2e.rs, docs/extract.rs tests
- Function: none: the existing suites are the test (command in the note)
- Green on arrival: US-0228 carried the figures back; then the mutation below
- Note: run `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0088_ e2e_0089_ docs::extract --test-threads=1` (all pass) and `git diff --stat <US-0227 commit> -- crates/core/src/tools/office_bound_e2e.rs crates/core/src/tools/pdf_bound_e2e.rs crates/core/src/tools/archive_testkit.rs crates/core/src/tools/pdf_testkit.rs` (empty), and check that the `mod tests` of `docs/extract.rs` is unchanged since US-0227 (`git diff <US-0227 commit> -- crates/core/src/docs/extract.rs` touches no line after `#[cfg(test)]`).
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-019: The HTTP API door during a large extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-006; **Priority:** High
- **Given** the large PDF
- **When** mia asks for its text through the HTTP API, and noa reads `/notes/todo.txt` through the HTTP API every 100 ms from 50 ms after
- **Then** every read returns `buy milk` in time
- **And** mia's answer equals the answer of E2E-001
- Level / Driver / Location: functional e2e / HTTP door with probe reads every 100 ms, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0091_019_the_http_api_door_during_a_large_extr`
- Green on arrival: US-0228 moved decoding off the request threads for both doors; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-020: A refusal through the HTTP API, as recorded
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/bomb.pdf` (E2E-004)
- **When** mia asks for its text through the HTTP API
- **Then** the answer's error is `ERR_INVALID_ARGUMENT` and its detail `ERR_INVALID_ARGUMENT: could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** nothing left
- Level / Driver / Location: functional e2e / HTTP door / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0091_020_a_refusal_through_the_http_api_as_rec`
- Green on arrival: the refusal is unchanged (FR-NEW-003); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-027: Giving up through the HTTP API
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** Medium
- **Given** the large PDF
- **When** mia asks for its text through the HTTP API and closes the connection 1 second later
- **Then** after 10 seconds no `/docs/large.md`, no artifact, no audit line for it
- **And** a following 52 428 800 byte write by mia succeeds
- Level / Driver / Location: functional e2e / HTTP door on a real loopback socket, raw client closed after 1 s, one worker / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0091_027_giving_up_through_the_http_api`
- Green on arrival: hyper drops the handler future when the connection ends; the decoding result is dropped with it; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-011 | in the REST error mapping, answer `ERR_INTERNAL_ERROR` with status 400: the status assertion fails |
| E2E-017 | drop the carry back of `LAST_PDF_HELD` after the join: `decoded_held` panics on a missing record in the SPEC-0089 suite, the run fails |
| E2E-019 | run the readers inline (no `spawn_blocking`): the reads wait about 5.5 s, the test fails |
| E2E-020 | wrap the stream limit refusal as `ERR_INTERNAL_ERROR` in `prepare_extraction`: the error and detail differ, the test fails |
| E2E-027 | in the REST extract handler, run the engine call in a detached `tokio::spawn`: the companion is written after the close, the test fails |

### Closing checks (spec converge)
- Every acceptance test of SPEC-0091 green, run by `make test`: the 28 `E2E-` tests (`rg -c 'fn e2e_0091_' crates/core/src` totals 27, E2E-017 being the existing suites).
- Requirement coverage audit: `comm -23 <(rg -o 'FR-NEW-[0-9]{3}' sdlc/specs/SPEC-0091-*/sdlc-spec.md | sort -u) <(rg -o 'SPEC-0091/FR-NEW-[0-9]{3}' sdlc/stories/US-022[89]-*.md sdlc/stories/US-0230-*.md -N --no-filename | sed 's#.*/##' | sort -u)` prints nothing, and US-0228, US-0229 are `done` or `landed` in `sdlc/stories/_queue.md`.

## Constraints
- Files not to touch: every production file (no production change in this story), the files of E2E-017, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: symbol search and Markdown to Word rendering on the request thread are side findings, reported, never filed

## Definition of done
- [ ] Green on arrival tests (E2E-011, E2E-017, E2E-019, E2E-020, E2E-027): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` clean; every `e2e_0092_` test
- [ ] Docs updated: `README.md` (document extraction: decoding never holds up other requests; the stop message), `AGENTS.md` (`docs/extract.rs` bullet: blocking pool, `DecodePlaces` in `SafetyManager`, figures carried back), `.agent_docs/tools.md` (`fs.extract_text`: ceiling, arrival order, the stop answer), `.agent_docs/testing.md` (filter `e2e_0091_`, multi thread timing tests under `heavy()`, the fault seam, the recorded literals), each entry dated (DEC-010).
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- and E2E-SEC- of SPEC-0091), and every requirement of SPEC-0091 cited by at least one story with status done or landed (the audit above).
- [ ] One commit `US-0230: extraction latency through the HTTP door and documentation` on its story branch `story/US-0230-extraction-latency-through-the-http-door`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0091-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0091` and the requirement ids (SPEC-0091/FR-NEW-006, FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005).
