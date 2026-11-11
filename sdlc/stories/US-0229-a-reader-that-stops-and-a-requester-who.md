# US-0229: a reader that stops and a requester who gives up

> Spec: SPEC-0091 (sdlc/specs/SPEC-0091-keep-requests-answered-while-documents-are-extracted/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0228
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Define what a member sees when a reader stops before giving a result: one refusal, `ERR_INTERNAL_ERROR` with `could not extract <path>: the document reader stopped unexpectedly`, one error log line, nothing left, the place free, other extractions untouched. And prove what an abandoned extraction leaves: nothing, whether it was decoding (which runs to its end holding its place) or still waiting (which never starts).

## Requirements covered
- SPEC-0091/FR-NEW-004: IF a reader stops before giving a result THEN THE document text extraction SHALL refuse that request with `ERR_INTERNAL_ERROR` and the message `could not extract <path>: the document reader stopped unexpectedly`, leave no companion, artifact, audit line or charge, log one error line naming the code and the path, and free its place.
- SPEC-0091/FR-NEW-005: WHEN a member abandons an extraction request while its document is decoding or waiting for a place THE document text extraction SHALL write no companion, store no artifact, record no audit line and charge nothing for it.
- SPEC-0091/FR-NEW-002 (partial: a stopped or abandoned decoding and its place): The document text extraction SHALL decode at most as many documents at once as the server has request processing threads, and SHALL start a waiting extraction as soon as a decoding ends, in arrival order.
- SPEC-0091/FR-NEW-003 (partial: the same document after the fault, asking again after an abandon): The document text extraction SHALL answer, for the same document, options and member, exactly what it answers today: text, preview, character count, truncation, notes, companion, table and image artifacts, audit line, write charge, and every refusal code and message.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0091_009 e2e_0091_010 e2e_0091_012 e2e_0091_013 e2e_0091_014 e2e_0091_015 e2e_0091_026 e2e_0091_028 --test-threads=1`: 8 tests pass. Real assertions: the code and message, the captured log events, the listing, artifact lists, audit trail and a 52 428 800 byte write, the answer order and times, `started()` of the server's places.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0091):** the permit is acquired on the request side before `spawn_blocking` and moved into the closure, dropped when the closure returns: a waiting extraction whose future is dropped never starts, a started one keeps its place until its decoding ends, success, refusal or panic alike. **Obliged by:** SPEC-0091/FR-NEW-002, FR-NEW-005. **Rationale:** dropping the future cancels the acquire; the closure owns the release.
- **DEC-006 (SPEC-0091):** a panic of the decoding closure (a `JoinError` that `is_panic()`) is answered `ToolError::internal("could not extract <path>: the document reader stopped unexpectedly")` by `prepare_extraction`, which passes this error through instead of re wrapping it as `ERR_INVALID_ARGUMENT`; the closure's place is already free (DEC-003); nothing was written. A `JoinError` that is a cancellation (runtime shutdown) is answered the same way. Until US-0229, US-0228 resumes the panic on the request side (`std::panic::resume_unwind`), today's behaviour. The PDF reader's own `catch_unwind` (`extract.rs:519`) is unchanged: a panic it contains stays `pdf parser failed on this document`. **Obliged by:** SPEC-0091/FR-NEW-004, DI-6. **Rationale:** the message is worded once, at the place that knows the path.
- **DEC-007 (SPEC-0091):** the test fault is per server: `DecodePlaces::inject_fault(path)` (test only) makes the decoding closure for that path panic before calling the reader; `clear_faults()` removes it (E2E-026). **Obliged by:** SPEC-0091/FR-NEW-004 (E2E-009, "the operator's test fault"), DI-6. **Rationale:** the closure runs on a pooled thread, so a thread local set by the test is invisible there; a per server set keeps parallel tests apart.
- **DEC-008 (SPEC-0091):** the probes: the operator's health check is `GET /health` on the router built with the state; noa's listing is `McpServer::fs_list_dir` of `/notes`; noa's read is `McpServer::fs_read` of `/notes/todo.txt`. A probe loop runs as a spawned task sending one probe every 100 ms from 50 ms after the extraction started until a `watch` channel says stop, recording each answer time. "Abandon" is aborting the task that awaits `McpServer::fs_extract_text` (the transport drops that future when a client cancels), and, at the HTTP door, closing the client socket of a served router (hyper drops the handler future when it reads the end of the connection). **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-005, FR-NEW-006. **Rationale:** real handlers, no stand in.
- **DEC-010 (SPEC-0091):** documentation: `.agent_docs/architecture.md` (US-0229) states where decoding runs, the ceiling and the stop; the closing story updates `README.md`, `AGENTS.md` (`docs/extract.rs` bullet), `.agent_docs/tools.md` (`fs.extract_text`: other requests unaffected, the stop message) and `.agent_docs/testing.md` (filter `e2e_0091_`, the multi thread timing tests and `heavy()`). No tool contract change. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing pattern.
### Data model
`PlacesProbe` gains `faults: HashSet<String>` (test only).
### Contracts
- In `extract_owned`, a `JoinError` from the decoding closure (panic or cancellation) returns `Err(ToolError::internal(READER_STOPPED))` with `const READER_STOPPED: &str = "the document reader stopped unexpectedly";` (the `resume_unwind` of US-0228 goes).
- In `prepare_extraction`, before the `ERR_INVALID_ARGUMENT` wrap: `Err(e) if e.code == code::INTERNAL_ERROR && e.message == READER_STOPPED => Err(ToolError::internal(format!("could not extract {path}: {READER_STOPPED}")))`.
- Test only: `DecodePlaces::inject_fault(&self, path: &str)`, `DecodePlaces::clear_faults(&self)`; the request side reads whether `path` has a fault before spawning and moves the boolean into the closure, which panics with `test fault: the document reader stops` before calling the reader.
- Answers: MCP `CallToolResult::error` `An error occurred invoking 'fs.extract_text': ERR_INTERNAL_ERROR: could not extract /docs/report.pdf: the document reader stopped unexpectedly` and one ERROR line from `logging::log_tool_failure` (`crates/core/src/mcp/server.rs:46-62`).
- `.agent_docs/architecture.md`: dated paragraph: the six readers decode on the blocking pool behind `DecodePlaces` (one place per request processing thread, arrival order), the figures carried back, the stop answer, an abandoned decoding running to its end.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/tools/offload_e2e.rs` | modify |
| `.agent_docs/architecture.md` | modify |

Read only (imported, never modified): `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/tool_surface_e2e.rs (its `setup` is the model, not imported)`.
### Existing patterns
US-0228's `extract_owned` and `DecodePlaces`; the log capture of SPEC-0088 E2E-035 (`crates/core/src/tools/office_bound_e2e.rs:939-948`); abandoning: `let task = tokio::spawn(async move { server.fs_extract_text(..).await }); tokio::time::sleep(..).await; task.abort();` (DEC-008); `archive_testkit::nothing_left` for the listing, artifacts, audit and the 52 428 800 byte write.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0091/E2E-009: A reader stops unexpectedly
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** Critical
- **Given** `/docs/report.pdf`, a one page PDF reading `Revenue`, and the operator's test fault that makes the PDF reader stop unexpectedly on it
- **When** mia asks for its text
- **Then** mia is refused `ERR_INTERNAL_ERROR` with `could not extract /docs/report.pdf: the document reader stopped unexpectedly`
- **And** nothing left
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test]` (current thread) / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_009_a_reader_stops_unexpectedly`
- Red before the change because: on US-0228 code `inject_fault` does not exist; once the fault is wired (step 1), the panic is resumed and the request task panics instead of answering
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-010: What the operator sees after an unexpected stop
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** E2E-009 just happened
- **When** the operator reads the log and sends a health check
- **Then** the log holds exactly one error line containing `ERR_INTERNAL_ERROR` and `/docs/report.pdf`, and no audit line names `/docs/report.pdf`
- **And** the health check is answered healthy in time
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` under `crate::logging::capture::lock_for_test` on a current thread runtime built in the test, then `GET /health` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_010_what_the_operator_sees_after_an_unexp`
- Red before the change because: as E2E-009: no answer and no error line
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-012: One reader stops, another extraction runs on
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** High
- **Given** a server with two request processing threads; `/docs/report.pdf` with the fault of E2E-009, the large PDF, and `/docs/memo.docx`
- **When** noa asks for the large PDF, mia for `/docs/report.pdf` 50 ms later, and ola for memo 50 ms after mia's refusal
- **Then** mia gets the refusal of E2E-009; noa gets the answer of E2E-001; ola gets memo's preview while noa's decoding still runs
- **And** most at once is 2
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_012_one_reader_stops_another_extraction_r`
- Red before the change because: as E2E-009: mia's request panics
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-013: Mia gives up mid decoding
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** the large PDF
- **When** mia asks for its text and abandons the request 1 second later
- **Then** after 10 seconds no `/docs/large.md` is in the listing of `/docs`, no artifact is listed for `/docs/large.pdf`, no audit line names it
- **And** a following 52 428 800 byte write by mia succeeds
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs, the extraction task aborted 1 s after it started
- Function: `e2e_0091_013_mia_gives_up_mid_decoding`
- Green on arrival: the result of an abandoned decoding is dropped with its future (US-0228); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-014: An abandoned decoding keeps its place until it ends
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** High
- **Given** the large PDF; `/docs/memo.docx`
- **When** mia asks for the large PDF and abandons it 1 second later, and noa asks for memo 100 ms after the abandon
- **Then** noa's answer (memo's preview) arrives after the large PDF's decoding ended, and no earlier than 4 seconds after mia's request
- **And** most at once is 1; a health check sent with noa's request is answered in time
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs, the extraction task aborted 1 s after it started
- Function: `e2e_0091_014_an_abandoned_decoding_keeps_its_place`
- Green on arrival: the permit lives in the closure (US-0228, DEC-003); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-015: Asking again after giving up
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** E2E-013 just happened and its decoding ended
- **When** mia asks for the text of the large PDF again
- **Then** the answer equals the answer of E2E-001, not served from a companion
- **And** one audit line names `/docs/large.pdf` and `/docs/large.md` now exists
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_015_asking_again_after_giving_up`
- Green on arrival: an abandon leaves nothing to answer from; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-026: The same document after the fault is removed
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** Medium
- **Given** E2E-009 just happened, then the test fault is removed
- **When** mia asks for the text of `/docs/report.pdf`
- **Then** the answer equals the recorded answer of `/docs/report.pdf`, preview containing `Revenue`
- **And** `/docs/report.md` exists and one audit line names it
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test]` (current thread) / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_026_the_same_document_after_the_fault_is`
- Red before the change because: on US-0228 code `clear_faults` does not exist (compile), then E2E-009 part of the test panics
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-028: Giving up while waiting for a place
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** High
- **Given** the large PDF decoding for noa; `/docs/c.pdf` as in E2E-003
- **When** mia asks for `/docs/c.pdf` 50 ms later and abandons it 500 ms later
- **Then** after noa's answer and 10 more seconds, no `/docs/c.md`, no artifact and no audit line for `/docs/c.pdf`
- **And** most at once stayed 1, and `/docs/c.pdf` was never decoded (its decoded bytes measurement is absent)
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs, mia's task aborted 500 ms after it asked
- Function: `e2e_0091_028_giving_up_while_waiting_for_a_place`
- Green on arrival: an abandoned acquire never starts (US-0228, DEC-003); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-013 | in `prepare_extraction`, run the whole extraction and companion write in a detached `tokio::spawn` and await its handle: the abort leaves the detached task, which writes `/docs/large.md`, the test fails |
| E2E-014 | drop the permit on the request side right after `spawn_blocking`: memo starts while the large PDF still decodes, before 4 s, and `most_at_once()` is 2, the test fails |
| E2E-015 | write the companion from the decoding closure when the decoding ends: the second call is answered from the companion (`cached` true), the test fails |
| E2E-028 | acquire the place inside the closure with `blocking_acquire`: `/docs/c.pdf` is decoded after noa, `started()` holds it, the test fails |

## Constraints
- Files not to touch (E2E-017): `crates/core/src/tools/office_bound_e2e.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/pdf_testkit.rs`, the `#[cfg(test)] mod tests` of `crates/core/src/docs/extract.rs`; also `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Never wrap the Office readers in `catch_unwind` for this (DEC-006); the PDF reader's own `catch_unwind` stays
- The stop error is logged by the request side only (`log_tool_failure`); the decoding closure logs nothing
- No dash as punctuation in code, comments or docs
- Scope boundary: the HTTP door and the closing docs are US-0230

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-009, E2E-010, E2E-012, E2E-026 (step 1: the fault seam and the tests on US-0228 code; E2E-009, E2E-010, E2E-012, E2E-026 fail (a panicking request, no answer); step 2: the stop mapping).
- [ ] Green on arrival tests (E2E-013, E2E-014, E2E-015, E2E-028): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 48 `e2e_0088_` tests, the 50 `e2e_0089_` PDF tests and the 51 extraction unit tests of `docs::extract`, unmodified (`env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0088_ e2e_0089_ docs::extract --test-threads=1`), with every decoded bytes and content held figure they assert; every `e2e_0092_` test; every `api::dataplane` extraction test; every test of US-0228
- [ ] Docs updated: `.agent_docs/architecture.md` (DEC-010), entry dated.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0229: a reader that stops and a requester who gives up` on its story branch `story/US-0229-a-reader-that-stops-and-a-requester-who`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0091-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0091` and the requirement ids (SPEC-0091/FR-NEW-004, FR-NEW-005, FR-NEW-002, FR-NEW-003).
