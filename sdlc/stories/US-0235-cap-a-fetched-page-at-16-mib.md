# US-0235: cap a fetched page at 16 MiB

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0234
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
With the web tools on, a page fetch reads the remote answer whole before cutting its text: a site answering a gigabyte costs the server a gigabyte, twice. After this story the answer is read piece by piece and refused once it passes 16 777 216 bytes, or before its body when its declared length is past that, with the message the pull request tools already use; within the limit the fetch answers as today. The story builds the capped reader, the held hook and the stand in site every later story reuses.

## Requirements covered
- SPEC-0093/FR-NEW-001 (partial: a page fetch returned to the assistant): IF a remote answer handed back to the assistant passes 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL stop reading it and refuse with `ERR_INVALID_ARGUMENT` and the message `response from host '<host>' exceeds the 16 MiB limit`.
- SPEC-0093/FR-NEW-002 (partial: a page fetch returned to the assistant): IF a remote answer handed back to the assistant declares a length past 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL refuse with the message of FR-NEW-001 without reading any byte of its body.
- SPEC-0093/FR-NEW-005 (partial: a page fetch): THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- SPEC-0093/FR-NEW-006 (partial: a compressed answer fetched): THE web lookup and the library documentation lookup SHALL count against every limit of this specification each byte of the answer as the server keeps it, an answer marked compressed being counted and kept as received.
- SPEC-0093/FR-NEW-009 (partial: the page fetch within the limit, a trickling answer): THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_sec_004 e2e_0093_001 e2e_0093_006 e2e_0093_011 e2e_0093_013 e2e_0093_014 e2e_0093_030`: 7 tests pass, plus `bt_014_web_fetch_is_callable_through_mcp`. Real assertions: the MCP answer and refusal text, `answer_held` records (upper bound, and a lower bound of 16 777 217 on read refusals), the stand in's `body_sent` and `closed_early`.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0093):** one capped reader, `read_answer`, in `web.rs`, used by the six occurrences: it records the held figure, refuses before the body when `content_length()` passes the cap, then loops on `chunk()`: before extending, `held = buf.len() + piece.len()` is recorded, and when it passes the cap the response is dropped and the refusal returned; a `chunk()` error keeps today's text (`read response: {e}` for fetch and download, `parse ... response: {e}` for the JSON readers, `read docs response: {e}` for the documentation). JSON and HTML parsing then run on the capped bytes. **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009, 7.1, 7.5. **Rationale:** the in tree shape for remote bodies; dropping the response closes the connection without waiting for the site.
- **DEC-002 (SPEC-0093):** `<host>` is `reqwest::Url::parse(url)?.host_str()` lower cased, read from the URL the tool builds or receives before sending, so a redirect never changes it and the port never appears. **Obliged by:** SPEC-0093/FR-NEW-001. **Rationale:** spec wording.
- **DEC-003 (SPEC-0093):** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error) leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard is created by a small `start_answer(url)` call right after `send()` returns, before the status check. Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:** SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal path; the tests assert a lower bound on read refusals (at least cap plus 1).
- **DEC-006 (SPEC-0093):** no decompression is added (reqwest stays without its compression features), so the bytes counted are the bytes kept (FR-NEW-006). JSON readers parse the capped bytes with `serde_json::from_slice`; a parse failure keeps today's exact text by formatting reqwest's own wording for a decode error (`parse news response: error decoding response body`, and likewise for suggestions and the context7 resolution: `reqwest-0.12.28/src/error.rs:233`, `async_impl/response.rs:269-273`); search and documentation text use `String::from_utf8_lossy`, as `text()` does without the `charset` feature. **Obliged by:** SPEC-0093/FR-NEW-006, FR-NEW-009. **Rationale:** byte for byte today's answers and messages.
- **DEC-008 (SPEC-0093):** `web_testkit::StandIn::start(script: Script) -> (u16, Arc<Record>)`: a raw `TcpListener` on `127.0.0.1:0`; per connection it reads the request head, counts the request in `Record::requests`, then writes the scripted answer: a status line, headers (`Content-Length: n` or `Transfer-Encoding: chunked`, `Content-Type`, `Content-Encoding`), an optional pause after the headers, then the body in 65 536 byte pieces (chunked framing when chunked), adding the body bytes written to `Record::body_sent`; a write error before the scripted end sets `Record::closed_early`. Scripts: `Fixed(bytes)`, `Letters { letter, n }`, `Flood` (the letter `a` 1 073 741 824 times, endless when asked), `Trickle` (one letter per second), `Status(code)` with a declared length and no body. **Obliged by:** SPEC-0093 section 9 ("stand in site ... records how many body bytes it sent and whether the connection was closed before it finished"). **Rationale:** a raw writer controls the pause between headers and body (E2E-014) and the framing exactly.
- **DEC-009 (SPEC-0093):** the door: the tests call the MCP route of the served app with `call_tool`, reused from `tools/tool_surface_e2e.rs` by making `setup`, `app_with_atlas`, `call_tool`, `success_payload` and `assert_tool_not_found` `pub(crate)` (visibility only, no behaviour change). `web_testkit::served(web: bool, context7: Option<String>)` sets the families, the test base, and seeds `/downloads`, `/pages`, `/docs` in `atlas`. **Obliged by:** SPEC-0093/FR-NEW-001 to FR-NEW-010 (spec 9: through the assistant tool). **Rationale:** the real route, including `add_optional_families` (`mcp/server.rs:69-77`).
### Data model
Test only: `pub(crate) struct AnswerHeld { pub(crate) url: String, pub(crate) held: u64 }`; `thread_local! { pub(crate) static ANSWER_HELD: RefCell<Vec<AnswerHeld>> }` in `web.rs`; a guard `HeldRecord` that pushes `{url, 0}` when created and writes its largest `held` in `Drop`.
### Contracts
- `pub(crate) const ANSWER_LIMIT: u64 = 16_777_216;` in `crates/core/src/tools/web.rs`.
- `pub(crate) fn host_of(url: &str) -> String`: `reqwest::Url::parse(url)` then `host_str()` lower cased (an unparsable URL never reaches here: `send()` already failed).
- `pub(crate) fn over_answer_limit(host: &str) -> ToolError`: `ToolError::invalid_argument(format!("response from host '{host}' exceeds the 16 MiB limit"))`.
- `pub(crate) async fn read_answer(mut resp: reqwest::Response, url: &str, cap: u64, read_err: &str, over: impl Fn(Option<u64>) -> ToolError) -> Result<Vec<u8>>`: guard first; `if let Some(n) = resp.content_length() && n > cap { return Err(over(Some(n))); }` (the response is dropped, nothing read); loop `resp.chunk().await.map_err(|e| ToolError::internal(format!("{read_err}: {e}")))?`; for `Some(piece)`: `held = buf.len() + piece.len()` recorded; `if held > cap { return Err(over(None)); }`; `buf.extend_from_slice(&piece)`.
- `fetch_raw(url, timeout_secs, cap, over)` returns `(Vec<u8>, String)` through `read_answer` with `read_err` `read response`; `web_fetch` passes `ANSWER_LIMIT` and `over_answer_limit(&host_of(url))`. Save mode and download keep the whole answer read in this story (`cap` `u64::MAX`, `over` unreachable) until US-0238.
- Unchanged: `url must start with http:// or https://`, `fetch request failed: ...`, `request timed out`, the 50 000 character cut, HTML stripping when the content type holds `text/html`.
- `tools/tool_surface_e2e.rs`: `setup`, `app_with_atlas`, `call_tool`, `success_payload`, `assert_tool_not_found` become `pub(crate)` (no other change).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/web.rs` | modify |
| `crates/core/src/tools/web_testkit.rs` | create |
| `crates/core/src/tools/web_bound_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] pub(crate) mod web_testkit;`, `#[cfg(test)] mod web_bound_e2e;`) |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (visibility of five helpers) |
### Existing patterns
`read_capped` (`crates/core/src/git/provider/mod.rs:398-448`) and `read_body_capped` (`crates/core/src/docs/service.rs:225-250`) for the loop; `fetch_raw` and `web_fetch` (`crates/core/src/tools/web.rs:383-417`); `bt_014_web_fetch_is_callable_through_mcp` (`crates/core/src/tools/tool_surface_e2e.rs:499-530`) for the door.
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-SEC-004: Page fetch flood (originating occurrence)
- **Covers:** occurrence 3, page returned to the assistant
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read into memory, then copied as text, before the 50 000 character cut.
- **Given** a stand in site answering a flood at `/flood`
- **When** the member fetches `http://127.0.0.1:<port>/flood`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.fetch", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_004_page_fetch_flood_originating_occu`
- Red before the change because: step 1 puts the guard on today's `bytes()` read (it records the body length): the record is 1 073 741 824 and the bound assertion fails; the stand in records the whole flood sent
- Note: the RED run holds about 2 GiB (body plus the text copy); take `archive_testkit::heavy()` first and run it alone.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-001: Page fetch within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/page` with `<p>Hello &amp; welcome</p>`, marked as HTML
- **When** the member asks to fetch `http://127.0.0.1:<port>/page`
- **Then** the answer is the text `Hello & welcome`
- **And** the answer held never exceeds 1 048 576
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_001_page_fetch_within_the_limit`
- Green on arrival: today's fetch answers the small page; step 1's guard records 25 bytes; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-006: Fetch answer of exactly the answer limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** a stand in site answering 16 777 216 letters `b`, chunked, not marked as HTML
- **When** the member fetches it
- **Then** the answer is 50 000 letters `b`
- **And** the answer held never exceeds 17 825 792
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_006_fetch_answer_of_exactly_the_answer_li`
- Green on arrival: 16 777 216 bytes read today; step 1's guard records them; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-011: Compressed answer counted as received
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a stand in site answering 100 000 bytes marked as compressed, which expands to 1 073 741 824 bytes once decompressed
- **When** the member fetches it
- **Then** the fetch succeeds, the answer is built from the 100 000 bytes as received (at most 50 000 characters)
- **And** the answer held never exceeds 1 148 576
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_011_compressed_answer_counted_as_received`
- Green on arrival: no decompression today (`Cargo.toml:110`); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-013: Fetch answer one byte over, chunked
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site answering 16 777 217 letters `b`, chunked
- **When** the member fetches it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- **And** the answer held never exceeds 17 825 792
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_013_fetch_answer_one_byte_over_chunked`
- Red before the change because: on US-0234 code the fetch succeeds with 50 000 letters
- Note: also asserts the record is at least 16 777 217: the refusal path counts what it read (DEC-003).
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-014: Fetch answer declaring one byte over
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site that declares a length of 16 777 217, sends its headers, then waits 10 seconds before sending the body
- **When** the member fetches it
- **Then** the refusal of E2E-013 arrives before the stand in site sends its first body byte
- **And** the stand in site records the connection closed with 0 body bytes sent; the answer held is 0
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_014_fetch_answer_declaring_one_byte_over`
- Red before the change because: on US-0234 code the fetch waits 10 s and reads the body: the time and `body_sent` assertions fail
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-030: Answer trickling until the time limit
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** a stand in site sending its headers then one letter per second for 60 seconds, chunked
- **When** the member fetches it with a time limit of 2 seconds
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `read response: `, within 5 seconds of the call
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with `timeout_secs` 2
- Function: `e2e_0093_030_answer_trickling_until_the_time_limit`
- Green on arrival: the client timeout covers the body read today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-001 | record `cap` instead of `buf.len() + piece.len()` in the guard: the record is 16 777 216, the bound fails |
| E2E-006 | refuse when `held >= cap`: the answer at the limit is refused, the test fails |
| E2E-011 | decode a body marked `gzip` with `flate2` before counting: the 100 000 bytes expand and the bound fails, the test fails |
| E2E-030 | map a `chunk()` error to `fetch request failed: ...`: the prefix assertion fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/git/provider/mod.rs` (its reader is the model, not shared), `crates/core/src/docs/service.rs`, `Cargo.toml` (no reqwest feature)
- Dependencies not to add: none (no reqwest compression feature)
- Check before extending: never append a piece and then compare
- Drop the response on refusal (return from `read_answer`); never drain the rest of the body
- No dash as punctuation in code, comments or docs
- Scope boundary: search, news and suggestions US-0236; context7 US-0237; writes US-0238 and US-0239

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-004, E2E-013, E2E-014 (step 1: the stand in, the served app, the guard on today's `bytes()` read, then the tests on US-0234 code; E2E-SEC-004, E2E-013, E2E-014 fail by assertion; step 2: `read_answer`).
- [ ] Green on arrival tests (E2E-001, E2E-006, E2E-011, E2E-030): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 5 tests of `tools::web`, the 4 of `tools::context7`, `bt_012` and `bt_014` of `tools::tool_surface_e2e`, `tools::all` counts, every earlier `e2e_0093_` test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0235: cap a fetched page at 16 MiB` on its story branch `story/US-0235-cap-a-fetched-page-at-16-mib`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-006, FR-NEW-009).
