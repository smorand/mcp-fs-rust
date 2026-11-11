# US-0236: cap search, news and suggestion answers

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0235
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
The search, news and suggestion tools read the search service's answer whole before parsing it. After this story they read it through the same capped reader, refused past 16 MiB or past a declared length, with today's results and messages otherwise. A test only base address lets the tests point the three fixed hosts at a stand in.

## Requirements covered
- SPEC-0093/FR-NEW-001 (partial: search, news, suggestions): IF a remote answer handed back to the assistant passes 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL stop reading it and refuse with `ERR_INVALID_ARGUMENT` and the message `response from host '<host>' exceeds the 16 MiB limit`.
- SPEC-0093/FR-NEW-002 (partial: search): IF a remote answer handed back to the assistant declares a length past 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL refuse with the message of FR-NEW-001 without reading any byte of its body.
- SPEC-0093/FR-NEW-005 (partial: search, news, suggestions): THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- SPEC-0093/FR-NEW-009 (partial: search results and news within the limit): THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_sec_001 e2e_0093_sec_002 e2e_0093_sec_003 e2e_0093_002 e2e_0093_007 e2e_0093_015 e2e_0093_035` and `cargo test -p mcp-fs-core --lib tools::web`: 7 new and 5 existing tests pass. Real assertions: exact results, refusal text with host `127.0.0.1`, held records, the stand in's record.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0093):** one capped reader, `read_answer`, in `web.rs`, used by the six occurrences: it records the held figure, refuses before the body when `content_length()` passes the cap, then loops on `chunk()`: before extending, `held = buf.len() + piece.len()` is recorded, and when it passes the cap the response is dropped and the refusal returned; a `chunk()` error keeps today's text (`read response: {e}` for fetch and download, `parse ... response: {e}` for the JSON readers, `read docs response: {e}` for the documentation). JSON and HTML parsing then run on the capped bytes. **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009, 7.1, 7.5. **Rationale:** the in tree shape for remote bodies; dropping the response closes the connection without waiting for the site.
- **DEC-003 (SPEC-0093):** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error) leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard is created by a small `start_answer(url)` call right after `send()` returns, before the status check. Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:** SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal path; the tests assert a lower bound on read refusals (at least cap plus 1).
- **DEC-006 (SPEC-0093):** no decompression is added (reqwest stays without its compression features), so the bytes counted are the bytes kept (FR-NEW-006). JSON readers parse the capped bytes with `serde_json::from_slice`; a parse failure keeps today's exact text by formatting reqwest's own wording for a decode error (`parse news response: error decoding response body`, and likewise for suggestions and the context7 resolution: `reqwest-0.12.28/src/error.rs:233`, `async_impl/response.rs:269-273`); search and documentation text use `String::from_utf8_lossy`, as `text()` does without the `charset` feature. **Obliged by:** SPEC-0093/FR-NEW-006, FR-NEW-009. **Rationale:** byte for byte today's answers and messages.
- **DEC-007 (SPEC-0093):** the fixed DuckDuckGo hosts get a test only seam: `WebConfig` gains `#[cfg(test)] #[serde(skip)] pub(crate) test_base: Option<String>` (included in its `Default` under `cfg(test)`); `register` clones the config into the three handlers, which build their URL from `endpoints(&config)`: `https://html.duckduckgo.com` and `https://duckduckgo.com` in production, the test base (`http://127.0.0.1:<port>`) for all three in tests. **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-009 (E2E-002, E2E-007, E2E-015, E2E-035, E2E-SEC-001 to E2E-SEC-003, spec 9 header). **Rationale:** per state, so parallel tests never share it; absent from the release build; same idea as the device flow's `with_github_urls` (`git/oauth/device_flow.rs:172-180`).
### Contracts
- `WebConfig` (`crates/core/src/config.rs:623-637`) gains `#[cfg(test)] #[serde(skip)] pub(crate) test_base: Option<String>`, `None` in its `Default` (the field and its default line both `#[cfg(test)]`).
- `struct Endpoints { search: String, news: String, suggestions: String }`; `fn endpoints(config: &WebConfig) -> Endpoints`: `https://html.duckduckgo.com/html/`, `https://duckduckgo.com/news.js`, `https://duckduckgo.com/ac/` in production; in tests, when `test_base` is set, `{base}/html/`, `{base}/news.js`, `{base}/ac/`. `register(reg, config)` clones the config into the three handlers (the parameter `_config` becomes used).
- `web_search` reads with `read_answer(.., ANSWER_LIMIT, "read response", |_| over_answer_limit(&host_of(&url)))` then `String::from_utf8_lossy`; `web_news` and `web_suggestions` read with `read_err` `parse news response` and `parse suggestions response`, then `serde_json::from_slice`, mapping a parse failure to `parse news response: error decoding response body` (resp. suggestions), today's text (DEC-006).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/web.rs` | modify |
| `crates/core/src/config.rs` | modify (test only field) |
| `crates/core/src/tools/web_bound_e2e.rs` | modify |

Read only (imported, never modified): `crates/core/src/tools/web_testkit.rs`.
### Existing patterns
US-0235's `read_answer`; the URL building of `web_search` (`web.rs:277`), `web_news` (`:346-348`), `web_suggestions` (`:421`); `with_github_urls` (`crates/core/src/git/oauth/device_flow.rs:172-180`) as the precedent of a test base.
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

E2E-002's result page: `<a class="result__a" href="https://www.rust-lang.org/">Rust</a><a class="result__snippet">A language</a>` then the same for `Tokio`, `https://tokio.rs/`, `A runtime`, as the parser splits on `result__a` (`web.rs:291-330`). E2E-007's news: `{"results":[{"title":"Launch","url":"https://x.test/","excerpt":"","date":""}]}` padded with spaces after the closing brace to exactly 16 777 216 bytes.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-SEC-001: Search answer flood
- **Covers:** occurrence 1
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole 1 073 741 824 bytes are read as text before the results are parsed.
- **Given** the search service answers a flood
- **When** the member searches `rust`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed before 1 073 741 824 bytes were sent
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.search", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_001_search_answer_flood`
- Red before the change because: with the guard on today's `text()` read (step 1) the record is 1 073 741 824
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-002: News answer flood
- **Covers:** occurrence 2
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed as a list.
- **Given** the news service answers `{"results":[` followed by the letter `a` repeated 1 073 741 824 times, chunked
- **When** the member asks for news about `launch`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.news", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_002_news_answer_flood`
- Red before the change because: today's `json()` reads the whole body first: the record is the body length
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-003: Suggestions answer flood
- **Covers:** occurrence 4
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed.
- **Given** the suggestion service answers a flood
- **When** the member asks for suggestions for `ru`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.suggestions", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_003_suggestions_answer_flood`
- Red before the change because: as E2E-SEC-002
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-002: Search within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** the search service is a stand in site answering a result page with two results, `Rust` at `https://www.rust-lang.org/` with snippet `A language`, and `Tokio` at `https://tokio.rs/` with snippet `A runtime`
- **When** the member searches `rust` with at most 10 results
- **Then** the answer lists exactly those two results, title, address and snippet, in that order
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.search", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with `max_results` 10
- Function: `e2e_0093_002_search_within_the_limit`
- Green on arrival: once the test base reaches the stand in (step 1), today's parser answers the two results; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-007: News answer of exactly the answer limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the news service answers a list with one item titled `Launch`, padded with spaces to exactly 16 777 216 bytes
- **When** the member asks for news about `launch`
- **Then** the answer lists one item titled `Launch`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.news", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_007_news_answer_of_exactly_the_answer_lim`
- Green on arrival: read whole today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-015: Suggestions answer over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the suggestion service answers a list padded with spaces to 16 777 217 bytes, chunked
- **When** the member asks for suggestions for `ru`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.suggestions", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_015_suggestions_answer_over_the_limit`
- Red before the change because: on US-0235 code (test base wired) the padded answer is read and parsed: no refusal
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-035: Search answer declaring a length over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** the search service declares 20 000 000 bytes and waits 10 seconds after its headers
- **When** the member searches `rust`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`, before the first body byte is sent
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.search", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_035_search_answer_declaring_a_length_over`
- Red before the change because: on US-0235 code the search waits 10 s and reads the body
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-002 | drop the snippet extraction: the snippets are empty, the test fails |
| E2E-007 | refuse when `held >= cap`: the answer at the limit is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/git/provider/mod.rs` (its reader is the model, not shared), `crates/core/src/docs/service.rs`, `Cargo.toml` (no reqwest feature)
- Dependencies not to add: none
- The test base exists only under `cfg(test)`: no configuration key, no environment variable, no behaviour in a release build
- Keep every failure message of the three tools byte for byte
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-015, E2E-035 (step 1: the test base and the guard on today's reads, then the tests on US-0235 code; the three exploit tests, E2E-015 and E2E-035 fail by assertion; step 2: the capped reads).
- [ ] Green on arrival tests (E2E-002, E2E-007): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 5 tests of `tools::web`, the 4 of `tools::context7`, `bt_012` and `bt_014` of `tools::tool_surface_e2e`, `tools::all` counts, every earlier `e2e_0093_` test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0236: cap search, news and suggestion answers` on its story branch `story/US-0236-cap-search-news-and-suggestion-answers`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009).
