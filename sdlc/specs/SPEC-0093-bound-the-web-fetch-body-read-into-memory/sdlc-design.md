# SPEC-0093: Bound the web fetch body read into memory, design

> Spec: sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding
> Designed on: 2026-11-11
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Capped read | `reqwest::Response::content_length()` then `Response::chunk()` in a loop, the shape of `git/provider/mod.rs:398-448` (`read_capped`) and `docs/service.rs:225-250` (`read_body_capped`) | reqwest 0.12.28 locked (`Cargo.lock:4766`), `default-features = false`, features `json`, `rustls-tls`, `multipart` (`Cargo.toml:110`): no decompression | DEC-001, DEC-006 |
| Host in the message | `reqwest::Url::parse(url)` then `host_str()`, lower case | same | DEC-002 |
| Write limit | `quota::project_remaining_for` (`quota.rs:221-231`), `SafetyManager::project_limit`, `quota::used_bytes`, `SafetyManager::session_remaining` (`safety.rs:153-156`), the session quota | in tree | DEC-004 |
| Stand in site | a raw `tokio::net::TcpListener` answering hand written HTTP/1.1 and recording what it sent | tokio 1.54.1 | DEC-008 |
| Assistant tool door | the MCP route of `crate::app::build_with_state`, called as `tools/tool_surface_e2e.rs` does (`setup` `:37-50`, `app_with_atlas` `:53-57`, `call_tool` `:351`) | in tree | DEC-009 |

Dependency statement: no new crate, no feature change (in particular no `gzip`, `brotli`, `deflate` or `zstd`
feature on reqwest, DEC-006), no `unsafe`.

Skills loaded: `rust` (reqwest async client, no blocking reads, typed `ToolError`, never log a body, clippy and
fmt gates).

## 2. Approaches

### Chosen: one capped reader for every remote answer of the two families

`web.rs` gains `read_answer(resp, host, limit, refusal)`: refuse before the body when `content_length()` is
past the limit, then read `chunk()` by `chunk()`, checking before each extend, dropping the response on
refusal (which closes the connection). Six call sites use it (spec 14.2 occurrences 1 to 6): with the answer
limit and the provider's message for answers handed back to the assistant; with the write limit and the
storage refusals for a download or a saved page. Parsing (HTML split, JSON) then runs on the capped bytes with
today's messages. Pros: one bound, one hook, one place for the order of refusals; the precedent shape. Cons:
JSON parse errors must keep today's text (DEC-006). Scope: `web.rs`, `context7.rs`, `config.rs` (test seam).

### Rejected: a shared client with a body size layer (DEC-001)

reqwest has no body size limit option; a tower layer would need a new dependency.

### Rejected: a fixed ceiling for downloads (DEC-004)

A smaller fixed number than the member's allowance would refuse legitimate downloads (spec D-6).

## 3. Architecture

```
MCP route (only door; web and context7 are optional families, mcp/server.rs:69-77)
  web.search / web.news / web.suggestions          -> endpoints(config) (DuckDuckGo, or the test base) [US-0236]
  web.fetch (no project)                            -> fetch_answer(url, ANSWER_LIMIT)                  [US-0235]
  web.fetch (project + save_path) / web.download    -> volume() membership, norm(path)                  [US-0238]
                                                       write_limit(client, safety, person, mount, path)  [US-0238, US-0239]
                                                       fetch_answer(url, write limit) -> fs_ops::write_bytes (today's checks)
  context7.resolve_library_id / get_library_docs    -> status as today, then read_answer(ANSWER_LIMIT)   [US-0237]

read_answer(resp, host, cap, refusal):
   HeldRecord::start(url)                         (test only, records 0 at once)
   content_length() > cap          -> refusal, body never read, response dropped
   loop chunk(): held = buf + piece (recorded first); buf + piece > cap -> refusal, response dropped
   end -> Ok(buf)
```

## 4. Data model

Nothing persisted. In memory per call: `buf: Vec<u8>` (at most the cap), the piece just received, the cap.
`WriteLimit { cap: u64, side: Side, quota: i64, project: Option<ProjectSide> }` with `ProjectSide { limit: i64,
used: i64, replaced: i64 }`, `Side::{Session, Project}`. Test only: `#[cfg(test)] pub(crate) struct AnswerHeld
{ url: String, held: u64 }`, thread local `ANSWER_HELD: RefCell<Vec<AnswerHeld>>`.

## 5. Contracts and interfaces

- `pub(crate) const ANSWER_LIMIT: u64 = 16_777_216` (`web.rs`), equal to `git::provider::MAX_RESPONSE_BYTES`.
- Answer refusal: `ToolError::invalid_argument(format!("response from host '{host}' exceeds the 16 MiB limit"))`,
  `<host>` the lower case host of the URL first asked, no port (DEC-002).
- Write refusals, today's constructors and messages: session `ToolError::write_quota_exceeded(format!("session
  write quota of {quota} bytes exceeded"))` (`safety.rs:142-144`); project
  `ToolError::project_quota_exceeded(limit, used, requested)` (`errors.rs:103-111`), `requested` =
  `limit - used + 1` when passed while reading, `declared - replaced` when declared (DEC-005).
- `pub(crate) async fn read_answer(resp: reqwest::Response, url: &str, cap: u64, over: impl Fn(Option<u64>) ->
  ToolError) -> Result<Vec<u8>>`: `over(Some(declared))` before the body, `over(None)` while reading.
- `fn write_limit(...) -> Result<WriteLimit>`; `WriteLimit::refusal(declared: Option<u64>) -> ToolError`.
- `WebConfig` gains `#[cfg(test)] #[serde(skip)] pub(crate) test_base: Option<String>` (DEC-007);
  `fn endpoints(config: &WebConfig) -> Endpoints { search, news, suggestions }`.
- Unchanged: every schema and description (optional families, outside the golden contract,
  `.agent_docs/tools.md:636-644`), every failure message of spec 2.1 row 7, time limits, the 50 000 character
  cut, the 50 result cap, the download's `content_type`.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/web.rs` | modify: `read_answer`, `ANSWER_LIMIT`, held hook; fetch, search, news, suggestions, download and saved page through it; `write_limit`; `endpoints` | FR-NEW-001 to FR-NEW-009 | US-0235, US-0236, US-0238, US-0239 |
| `crates/core/src/tools/context7.rs` | modify: resolution and documentation through `read_answer` | FR-NEW-001, FR-NEW-002, FR-NEW-009 | US-0237 |
| `crates/core/src/config.rs` | modify: test only `WebConfig::test_base` | test seam (DEC-007) | US-0236 |
| `crates/core/src/tools/web_testkit.rs` | create: the stand in site, its record, the flood, the served app with both families, `nothing_left`, held readers | test harness (DEC-008, DEC-009) | US-0235, US-0236, US-0238, US-0239 |
| `crates/core/src/tools/web_bound_e2e.rs` | create: the `e2e_0093_*` tests | every requirement | US-0235 to US-0240 |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify: `setup`, `app_with_atlas`, `call_tool`, `success_payload`, `assert_tool_not_found` made `pub(crate)` (visibility only) | DEC-009 | US-0235 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] pub(crate) mod web_testkit; #[cfg(test)] mod web_bound_e2e;` | test layout | US-0235 |
| `.agent_docs/config.md` | modify: `web` and `context7` sections: answer limit, write limit | DEC-010 | US-0237 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing (DEC-010) | US-0240 |

Affected existing tests, kept unchanged and green: the 5 tests of `tools/web.rs` (`:468-509`), the 4 of
`tools/context7.rs` (`:189-217`), `bt_012` and `bt_014` of `tools/tool_surface_e2e.rs` (`:487-530`), the
counts of `tools/all.rs:120-121`, the SPEC-0089 converter tests.

## 7. Compatibility and risks

- Declared changes: an answer past 16 MiB is refused; a download or saved page past the write limit is refused
  while reading, with today's storage refusals.
- Undeclared, documented: none; a JSON parse failure keeps its exact text (DEC-006).
- Reachability: both families are off by default (spec D-4); the change matters to deployments that turn them
  on.
- Ordering between stories (each lands on main alone): each story bounds a set of tools completely and leaves
  the others exactly as today, so no intermediate main is worse than today.
- Rollback: revert; no data or contract change.

## 8. Security design

- Threat surface: the body of every remote answer is controlled by the remote site (a member chosen URL, a page
  a search returns, the operator's documentation service answering from the internet). Sink: an unbounded
  `Vec` or `String` (`resp.text()`, `bytes()`, `json()`).
- Occurrences of the class sweep (spec 14.2), each fixed in this lot:

| # | Fix site | Mechanism | DEC | Exploit tests | Story |
|---|---|---|---|---|---|
| 1 | `web.rs:288` (`web_search`, `resp.text()`) | `read_answer(ANSWER_LIMIT)`, then `String::from_utf8_lossy` | DEC-001 | E2E-SEC-001 | US-0236 |
| 2 | `web.rs:360-361` (`web_news`, `resp.json()`) | `read_answer(ANSWER_LIMIT)`, then `serde_json::from_slice` | DEC-001, DEC-006 | E2E-SEC-002 | US-0236 |
| 3 | `web.rs:400-401` (`fetch_raw`), from fetch `:410`, saved page `:105`, download `:147` | `read_answer` with `ANSWER_LIMIT` (fetch) or the write limit (saved page, download), declared length first | DEC-001, DEC-004, DEC-005 | E2E-SEC-004 to E2E-SEC-007, E2E-SEC-010 | US-0235, US-0238 |
| 4 | `web.rs:432-435` (`web_suggestions`, `resp.json()`) | as 2 | DEC-001, DEC-006 | E2E-SEC-003 | US-0236 |
| 5 | `context7.rs:111-114` (`resolve_library_id`, `resp.json()`) | as 2 | DEC-001, DEC-006 | E2E-SEC-008 | US-0237 |
| 6 | `context7.rs:157-158` (`get_library_docs`, `resp.text()`), after today's status checks | `read_answer(ANSWER_LIMIT)` | DEC-001 | E2E-SEC-009 | US-0237 |

- Exploit channel: each exploit test asserts the refusal and reads the absence of effect through two channels
  other than the answer: the stand in site's record (bytes sent, connection closed before 1 073 741 824 bytes,
  `web_testkit::Record`, DEC-008) and the held hook (`answer_held`, at most the bound and, for a refusal while
  reading, at least the cap plus one byte, so a hook that skips the refusal path fails, DEC-003); for a write,
  `nothing_left` (the listing of the folder, no write audit line, a following 52 428 800 byte write succeeds).
  The payload is the spec's literal flood (`HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter
  `a` 1 073 741 824 times).
- Red before the fix: every exploit test's held assertion fails on today's code (the whole body is read).
- Secrets: none; a body is never logged; the refusal names the host only.
- Out of scope, reported as side findings (spec 3.2, 14.2 rows 9 to 11): address filtering, the device flow
  without timeout, operator endpoints, remote clone size.

## 9. Test strategy

Functional e2e through the assistant tool's real route (the MCP endpoint of the served app, `call_tool`), in
`crates/core/src/tools/web_bound_e2e.rs`, `#[tokio::test]` (current thread: the stand in, the client and the
hook share one thread). "Answer held" is read with `answer_held(url)` (every record for that URL; panics when
none, so a missing record never reads as 0). The stand in site of DEC-008 is the remote site; the search, news
and suggestion services are pointed at it through `WebConfig::test_base`, the documentation service through
`Context7Config::api_url`. Names `e2e_0093_<id>_<title in snake case>`, cut at 50 characters; filter
`e2e_0093_`. Log assertions (E2E-033) run under `crate::logging::capture::lock_for_test` on a current thread
runtime built in the test, as SPEC-0088 E2E-035 does (`office_bound_e2e.rs:939-948`).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, 006, 011, 013, 014, 030, SEC-004 | functional e2e / exploit | MCP route `web.fetch` | web_bound_e2e.rs | US-0235 |
| E2E-002, 007, 015, 035, SEC-001, SEC-002, SEC-003 | functional e2e / exploit | MCP route `web.search`, `web.news`, `web.suggestions` | web_bound_e2e.rs | US-0236 |
| E2E-003, 016, 032, 036, 038, SEC-008, SEC-009 | functional e2e / exploit | MCP route `context7.*` | web_bound_e2e.rs | US-0237 |
| E2E-004, 005, 008, 017, 018, 037, SEC-005, SEC-006, SEC-007, SEC-010 | functional e2e / exploit | MCP route `web.download`, `web.fetch` with a project | web_bound_e2e.rs | US-0238 |
| E2E-009, 010, 019, 020, 021, 022, 023 | functional e2e | MCP route `web.download` | web_bound_e2e.rs | US-0239 |
| E2E-012, 024 to 029, 031, 033, 034, 039 | functional e2e | MCP route | web_bound_e2e.rs | US-0240 |

Test data: generated by the stand in at run time; projects, limits and allowances set through `state.admin`
and `state.safety` as the SPEC-0084 tests do (`archive_testkit::limit_remaining`, `fixture_allowance` patterns).

## 10. Decisions log

- **DEC-001:** one capped reader, `read_answer`, in `web.rs`, used by the six occurrences: it records the held
  figure, refuses before the body when `content_length()` passes the cap, then loops on `chunk()`: before
  extending, `held = buf.len() + piece.len()` is recorded, and when it passes the cap the response is dropped
  and the refusal returned; a `chunk()` error keeps today's text (`read response: {e}` for fetch and download,
  `parse ... response: {e}` for the JSON readers, `read docs response: {e}` for the documentation). JSON and
  HTML parsing then run on the capped bytes. **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005,
  FR-NEW-009, 7.1, 7.5. **Rationale:** the in tree shape for remote bodies; dropping the response closes the
  connection without waiting for the site. **Alternatives considered:** per site copies (six places to keep in
  step); a body limit layer (new dependency). **Source:** existing code. **Evidence:**
  `git/provider/mod.rs:398-448`; `docs/service.rs:225-250`.
- **DEC-002:** `<host>` is `reqwest::Url::parse(url)?.host_str()` lower cased, read from the URL the tool
  builds or receives before sending, so a redirect never changes it and the port never appears. **Obliged by:**
  SPEC-0093/FR-NEW-001. **Rationale:** spec wording. **Alternatives considered:** `resp.url()` (the final URL
  after redirects, spec forbids). **Source:** spec. **Evidence:** spec FR-NEW-001 rules.
- **DEC-003:** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard
  created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest
  `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error)
  leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard
  is created by a small `start_answer(url)` call right after `send()` returns, before the status check.
  Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:**
  SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal
  path; the tests assert a lower bound on read refusals (at least cap plus 1). **Alternatives considered:** a
  per URL map (two concurrent floods on one URL, E2E-034, would overwrite). **Source:** design, SPEC-0089
  converter hook. **Evidence:** `docs/extract.rs:575-605`.
- **DEC-004:** `write_limit` runs after `volume()` (membership) and `norm()` (path), before the request:
  `replaced` = the size of the file at the destination when it exists (`client.stat`), else 0; project side =
  `project_remaining_for(client, safety, replaced)` (`None` without a limit), with `limit` and `used` kept for the
  message; session side = `session_remaining(person, mount)`; the cap is the lower, a tie goes to the project.
  The whole answer then goes to `fs_ops::write_bytes`, whose checks (read before write, then project, then
  session) run as today on the whole answer. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007.
  **Rationale:** SPEC-0084's choice of the lower side, tie to the project (`archive_single.rs:246-253`).
  **Alternatives considered:** reusing `archive_single::Caps::refusal` (reports the cap plus one, not net
  growth, spec 14.3 says on purpose). **Source:** existing code. **Evidence:** `quota.rs:221-231`;
  `safety.rs:153-156`; `fs_ops.rs:594-622`.
- **DEC-005:** `WriteLimit::refusal(declared)`: session side -> the session message with the configured quota;
  project side -> `project_quota_exceeded(limit, used, requested)` with `requested = declared - replaced` when
  a declared length triggered it, `limit - used + 1` when passed while reading. **Obliged by:**
  SPEC-0093/FR-NEW-003, FR-NEW-004 (E2E-019 to E2E-022). **Rationale:** spec D-7 (net growth). **Alternatives
  considered:** none. **Source:** spec. **Evidence:** `errors.rs:103-111`; `safety.rs:142-144`.
- **DEC-006:** no decompression is added (reqwest stays without its compression features), so the bytes counted
  are the bytes kept (FR-NEW-006). JSON readers parse the capped bytes with `serde_json::from_slice`; a parse
  failure keeps today's exact text by formatting reqwest's own wording for a decode error (`parse news
  response: error decoding response body`, and likewise for suggestions and the context7 resolution:
  `reqwest-0.12.28/src/error.rs:233`, `async_impl/response.rs:269-273`); search and documentation text use
  `String::from_utf8_lossy`, as `text()` does without the `charset` feature. **Obliged by:**
  SPEC-0093/FR-NEW-006, FR-NEW-009. **Rationale:** byte for byte today's answers and messages. **Alternatives
  considered:** passing serde's message through (changes today's message). **Source:** library source.
  **Evidence:** reqwest sources cited.
- **DEC-007:** the fixed DuckDuckGo hosts get a test only seam: `WebConfig` gains `#[cfg(test)] #[serde(skip)]
  pub(crate) test_base: Option<String>` (included in its `Default` under `cfg(test)`); `register` clones the
  config into the three handlers, which build their URL from `endpoints(&config)`: `https://html.duckduckgo.com`
  and `https://duckduckgo.com` in production, the test base (`http://127.0.0.1:<port>`) for all three in tests.
  **Obliged by:** SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-009 (E2E-002, E2E-007, E2E-015, E2E-035,
  E2E-SEC-001 to E2E-SEC-003, spec 9 header). **Rationale:** per state, so parallel tests never share it; absent
  from the release build; same idea as the device flow's `with_github_urls` (`git/oauth/device_flow.rs:172-180`).
  **Alternatives considered:** a configuration key (a production surface nobody asked for); a global override
  (parallel tests race). **Source:** design. **Evidence:** `config.rs:623-637`; `web.rs:25`, `:277`, `:346`, `:421`.
- **DEC-008:** `web_testkit::StandIn::start(script: Script) -> (u16, Arc<Record>)`: a raw `TcpListener` on
  `127.0.0.1:0`; per connection it reads the request head, counts the request in `Record::requests`, then
  writes the scripted answer: a status line, headers (`Content-Length: n` or `Transfer-Encoding: chunked`,
  `Content-Type`, `Content-Encoding`), an optional pause after the headers, then the body in 65 536 byte pieces
  (chunked framing when chunked), adding the body bytes written to `Record::body_sent`; a write error before the
  scripted end sets `Record::closed_early`. Scripts: `Fixed(bytes)`, `Letters { letter, n }`, `Flood` (the
  letter `a` 1 073 741 824 times, endless when asked), `Trickle` (one letter per second), `Status(code)` with a
  declared length and no body. **Obliged by:** SPEC-0093 section 9 ("stand in site ... records how many body bytes
  it sent and whether the connection was closed before it finished"). **Rationale:** a raw writer controls the
  pause between headers and body (E2E-014) and the framing exactly. **Alternatives considered:** the axum stand
  in of `converter_bound_e2e.rs` (cannot pause after the headers nor count bytes written to the socket).
  **Source:** design. **Evidence:** spec 9 header.
- **DEC-009:** the door: the tests call the MCP route of the served app with `call_tool`, reused from
  `tools/tool_surface_e2e.rs` by making `setup`, `app_with_atlas`, `call_tool`, `success_payload` and
  `assert_tool_not_found` `pub(crate)` (visibility only, no behaviour change). `web_testkit::served(web: bool,
  context7: Option<String>)` sets the families, the test base, and seeds `/downloads`, `/pages`, `/docs` in
  `atlas`. **Obliged by:** SPEC-0093/FR-NEW-001 to FR-NEW-010 (spec 9: through the assistant tool).
  **Rationale:** the real route, including `add_optional_families` (`mcp/server.rs:69-77`). **Alternatives
  considered:** calling the registry handler directly (skips the served routing that FR-NEW-010 tests).
  **Source:** existing code. **Evidence:** `tools/tool_surface_e2e.rs:37-57`, `:351`, `:499-530`.
- **DEC-010:** documentation: `.agent_docs/config.md` (web and context7 sections: 16 MiB answer limit, write
  limit for downloads) in US-0237; the closing story updates `README.md`, `AGENTS.md` (`tools/web.rs` capped
  reads), `.agent_docs/tools.md` (optional families: the limits and messages) and `.agent_docs/testing.md`
  (`e2e_0093_`, the stand in, `test_base`). Descriptions unchanged. **Obliged by:** n/a (no code impact):
  project documentation rule. **Rationale:** house closing pattern. **Alternatives considered:** mention 16 MiB
  in `web.fetch`'s description (no requirement asks). **Source:** constitution. **Evidence:** `AGENTS.md`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0235 | cap a fetched page at 16 MiB | P1 | SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-006, FR-NEW-009 | US-0234 | 2 | no |
| US-0236 | cap search, news and suggestion answers | P1 | SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009 | US-0235 | 2 | no |
| US-0237 | cap the library documentation answers | P1 | SPEC-0093/FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-009, FR-NEW-010 | US-0236 | 2 | no |
| US-0238 | stop a download or a saved page at the session allowance | P1 | SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-008, FR-NEW-009 | US-0237 | 2 | no |
| US-0239 | stop a download at the project storage limit | P1 | SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007 | US-0238 | 2 | no |
| US-0240 | web refusals in order, without trace, families off, and documentation | P2 | SPEC-0093/FR-NEW-001, FR-NEW-005, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-010 | US-0239 | 2 | yes |
