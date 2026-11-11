# US-0237: cap the library documentation answers

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0236
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
The library documentation tools read the documentation service's answer whole. After this story the resolution and the documentation are read through the capped reader after today's status checks, refused past 16 MiB or past a declared length; a failure status is still answered without reading its body, and the documentation family stays off by default.

## Requirements covered
- SPEC-0093/FR-NEW-001 (partial: library resolution and documentation): IF a remote answer handed back to the assistant passes 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL stop reading it and refuse with `ERR_INVALID_ARGUMENT` and the message `response from host '<host>' exceeds the 16 MiB limit`.
- SPEC-0093/FR-NEW-002 (partial: documentation; a failure status keeps today's refusal): IF a remote answer handed back to the assistant declares a length past 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL refuse with the message of FR-NEW-001 without reading any byte of its body.
- SPEC-0093/FR-NEW-005 (partial: library resolution and documentation): THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- SPEC-0093/FR-NEW-009 (partial: documentation within the limit, failure status): THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.
- SPEC-0093/FR-NEW-010 (partial: the documentation family): WHILE a family is off THE product SHALL NOT offer its tools nor make any remote request on their behalf.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_sec_008 e2e_0093_sec_009 e2e_0093_003 e2e_0093_016 e2e_0093_032 e2e_0093_036 e2e_0093_038` and `cargo test -p mcp-fs-core --lib tools::context7`: 7 new and 4 existing tests pass. Real assertions: the documentation text (10 012 bytes), the refusal text, `context7 error: 500 Internal Server Error`, held records (0 for the failure status), the stand in's request count.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0093):** one capped reader, `read_answer`, in `web.rs`, used by the six occurrences: it records the held figure, refuses before the body when `content_length()` passes the cap, then loops on `chunk()`: before extending, `held = buf.len() + piece.len()` is recorded, and when it passes the cap the response is dropped and the refusal returned; a `chunk()` error keeps today's text (`read response: {e}` for fetch and download, `parse ... response: {e}` for the JSON readers, `read docs response: {e}` for the documentation). JSON and HTML parsing then run on the capped bytes. **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009, 7.1, 7.5. **Rationale:** the in tree shape for remote bodies; dropping the response closes the connection without waiting for the site.
- **DEC-003 (SPEC-0093):** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error) leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard is created by a small `start_answer(url)` call right after `send()` returns, before the status check. Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:** SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal path; the tests assert a lower bound on read refusals (at least cap plus 1).
- **DEC-006 (SPEC-0093):** no decompression is added (reqwest stays without its compression features), so the bytes counted are the bytes kept (FR-NEW-006). JSON readers parse the capped bytes with `serde_json::from_slice`; a parse failure keeps today's exact text by formatting reqwest's own wording for a decode error (`parse news response: error decoding response body`, and likewise for suggestions and the context7 resolution: `reqwest-0.12.28/src/error.rs:233`, `async_impl/response.rs:269-273`); search and documentation text use `String::from_utf8_lossy`, as `text()` does without the `charset` feature. **Obliged by:** SPEC-0093/FR-NEW-006, FR-NEW-009. **Rationale:** byte for byte today's answers and messages.
- **DEC-010 (SPEC-0093):** documentation: `.agent_docs/config.md` (web and context7 sections: 16 MiB answer limit, write limit for downloads) in US-0237; the closing story updates `README.md`, `AGENTS.md` (`tools/web.rs` capped reads), `.agent_docs/tools.md` (optional families: the limits and messages) and `.agent_docs/testing.md` (`e2e_0093_`, the stand in, `test_base`). Descriptions unchanged. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing pattern.
### Contracts
- `resolve_library_id`: `start_answer(&url)` (guard) right after `send()`, then `read_answer(resp, &url, ANSWER_LIMIT, "parse context7 response", |_| over_answer_limit(&host_of(&url)))`, then `serde_json::from_slice`, a parse failure answered `parse context7 response: error decoding response body` (today's text).
- `get_library_docs`: the 404 and other failure statuses are answered first as today (body never read, the guard records 0); a success is read with `read_err` `read docs response`, then `String::from_utf8_lossy`.
- `.agent_docs/config.md`: dated lines in the `web` and `context7` sections: answers past 16 777 216 bytes are refused (`response from host '<host>' exceeds the 16 MiB limit`); downloads and saved pages stop at the write limit (from US-0238).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/context7.rs` | modify |
| `crates/core/src/tools/web_bound_e2e.rs` | modify |
| `.agent_docs/config.md` | modify |

Read only (imported, never modified): `crates/core/src/tools/web_testkit.rs`, `crates/core/src/tools/web.rs`.
### Existing patterns
`resolve_library_id` and `get_library_docs` (`crates/core/src/tools/context7.rs:94-159`); US-0235's `read_answer`, `start_answer`, `host_of`, `over_answer_limit` (imported from `crate::tools::web`).
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

The documentation service is the stand in at `http://127.0.0.1:<port>` set as `Context7Config::api_url`; routes `/v1/search` (resolution) and `/v1/tokio-rs/tokio` (documentation).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-SEC-008: Library resolution flood
- **Covers:** occurrence 5
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed.
- **Given** the documentation service answers the resolution with a flood
- **When** the member resolves `tokio`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "context7.resolve_library_id", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_008_library_resolution_flood`
- Red before the change because: with the guard on today's `json()` read the record is the body length
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-009: Library documentation flood
- **Covers:** occurrence 6
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read as text and returned.
- **Given** the documentation service answers the documentation of `/tokio-rs/tokio` with a flood
- **When** the member asks for it
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "context7.get_library_docs", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_009_library_documentation_flood`
- Red before the change because: with the guard on today's `text()` read the record is the body length
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-003: Library documentation within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** the documentation service is a stand in site answering `# Tokio docs` followed by 10 000 letters `t` for `/tokio-rs/tokio`
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the answer is exactly that text, 10 012 bytes
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "context7.get_library_docs", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_003_library_documentation_within_the_limi`
- Green on arrival: read whole today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-016: Library resolution answer over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the documentation service answers the resolution of `tokio` with 16 777 217 bytes, chunked
- **When** the member resolves `tokio`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "context7.resolve_library_id", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_016_library_resolution_answer_over_the_li`
- Red before the change because: on US-0236 code the 16 777 217 bytes are read and parsed as JSON: a parse refusal instead of the limit message
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-032: Documentation failure status with a huge body
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-009; **Priority:** Medium
- **Given** the documentation service answers a server failure status, declaring a length of 1 073 741 824 bytes
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `context7 error: 500 Internal Server Error`
- **And** the answer held is 0
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "context7.get_library_docs", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_032_documentation_failure_status_with_a_h`
- Green on arrival: the status is checked before any read today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-036: Documentation answer declaring one byte over
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** the documentation service answers success declaring 16 777 217 bytes, waiting 10 seconds after its headers
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`, before the first body byte is sent
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "context7.get_library_docs", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_036_documentation_answer_declaring_one_by`
- Red before the change because: on US-0236 code the tool waits 10 s and reads the body
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-038: Documentation family off
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the documentation family off and a stand in documentation service
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the call answers that the tool does not exist; the stand in received no request
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "context7.get_library_docs", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with the family off
- Function: `e2e_0093_038_documentation_family_off`
- Green on arrival: a disabled family adds no route (SPEC-0027); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-003 | drop the last byte of the capped documentation (`buf.pop()`): 10 011 bytes come back, the test fails |
| E2E-032 | read the body before checking the status: the guard records the body read, held is not 0, the test fails |
| E2E-038 | register `context7` whatever `enabled` in `add_optional_families`: the call reaches the stand in, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/git/provider/mod.rs` (its reader is the model, not shared), `crates/core/src/docs/service.rs`, `Cargo.toml` (no reqwest feature)
- Dependencies not to add: none
- Keep the status checks before any body read
- No dash as punctuation in code, comments or docs
- Scope boundary: writes are US-0238 and US-0239

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-008, E2E-SEC-009, E2E-016, E2E-036 (tests and fixtures first, on the previous story code, each failing for the reason stated).
- [ ] Green on arrival tests (E2E-003, E2E-032, E2E-038): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 5 tests of `tools::web`, the 4 of `tools::context7`, `bt_012` and `bt_014` of `tools::tool_surface_e2e`, `tools::all` counts, every earlier `e2e_0093_` test
- [ ] Docs updated: `.agent_docs/config.md` (DEC-010), entry dated.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0237: cap the library documentation answers` on its story branch `story/US-0237-cap-the-library-documentation-answers`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009, FR-NEW-010).
