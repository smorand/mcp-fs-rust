# US-0240: web refusals in order, without trace, families off, and documentation

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0239
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Pin the order of refusals (argument checks and membership before any request, the size before the read before write check), that a refused download leaves nothing and one log line, repeated or not, that two floods and a normal fetch at once are each bounded, that unreachable and trickling sites keep today's messages, and that a family off makes no request. Close the spec with the documentation.

## Requirements covered
- SPEC-0093/FR-NEW-007: IF several refusals apply to one call THEN THE web lookup SHALL report the first met in this order: (1) today's argument checks of the tool (a download address not starting with `http://` or `https://`: `url must start with http:// or https://`; a page fetch given only one of the project and the destination: `mount_id and save_path must both be provided, or neither`); (2) for a download or a saved page, membership (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`) and the destination path checks, as today; (3) while asking and reading, whichever comes first: a transport failure, the time limit, a declared length past the limit (FR-NEW-002, FR-NEW-004), the limit passed while reading (FR-NEW-001, FR-NEW-003); (4) for a download or a saved page, today's write checks on the whole answer: a file at the destination not read first in the session (`ERR_EDIT_WITHOUT_PRIOR_READ`, `edit '<path>' requires reading it first in this session`), then the project storage limit, then the session allowance.
- SPEC-0093/FR-NEW-008 (partial: repeat, log line, file kept): THE web lookup SHALL NOT write or change a file, charge the session write allowance or the project storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-004.
- SPEC-0093/FR-NEW-010: WHILE a family is off THE product SHALL NOT offer its tools nor make any remote request on their behalf.
- SPEC-0093/FR-NEW-001 (partial: two floods at once): IF a remote answer handed back to the assistant passes 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL stop reading it and refuse with `ERR_INVALID_ARGUMENT` and the message `response from host '<host>' exceeds the 16 MiB limit`.
- SPEC-0093/FR-NEW-005 (partial: two calls at once): THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- SPEC-0093/FR-NEW-009 (partial: unreachable site, read before write within the limit): THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_012 e2e_0093_024 e2e_0093_025 e2e_0093_026 e2e_0093_027 e2e_0093_028 e2e_0093_029 e2e_0093_031 e2e_0093_033 e2e_0093_034 e2e_0093_039`: 11 tests pass. Real assertions: refusal codes and messages, the stand in's request count, file contents, captured log events, held records per call. Then the closing checks below.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0093):** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error) leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard is created by a small `start_answer(url)` call right after `send()` returns, before the status check. Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:** SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal path; the tests assert a lower bound on read refusals (at least cap plus 1).
- **DEC-004 (SPEC-0093):** `write_limit` runs after `volume()` (membership) and `norm()` (path), before the request: `replaced` = the size of the file at the destination when it exists (`client.stat`), else 0; project side = `project_remaining_for(client, safety, replaced)` (`None` without a limit), with `limit` and `used` kept for the message; session side = `session_remaining(person, mount)`; the cap is the lower, a tie goes to the project. The whole answer then goes to `fs_ops::write_bytes`, whose checks (read before write, then project, then session) run as today on the whole answer. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007. **Rationale:** SPEC-0084's choice of the lower side, tie to the project (`archive_single.rs:246-253`).
- **DEC-009 (SPEC-0093):** the door: the tests call the MCP route of the served app with `call_tool`, reused from `tools/tool_surface_e2e.rs` by making `setup`, `app_with_atlas`, `call_tool`, `success_payload` and `assert_tool_not_found` `pub(crate)` (visibility only, no behaviour change). `web_testkit::served(web: bool, context7: Option<String>)` sets the families, the test base, and seeds `/downloads`, `/pages`, `/docs` in `atlas`. **Obliged by:** SPEC-0093/FR-NEW-001 to FR-NEW-010 (spec 9: through the assistant tool). **Rationale:** the real route, including `add_optional_families` (`mcp/server.rs:69-77`).
- **DEC-010 (SPEC-0093):** documentation: `.agent_docs/config.md` (web and context7 sections: 16 MiB answer limit, write limit for downloads) in US-0237; the closing story updates `README.md`, `AGENTS.md` (`tools/web.rs` capped reads), `.agent_docs/tools.md` (optional families: the limits and messages) and `.agent_docs/testing.md` (`e2e_0093_`, the stand in, `test_base`). Descriptions unchanged. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing pattern.
### Contracts
No production change. E2E-033 runs under `crate::logging::capture::lock_for_test` on a current thread runtime built in the test (the capture records on the locking thread); E2E-034 joins the three calls with `tokio::join!` and reads two `answer_held` records for the flood URL; E2E-024 and E2E-025 call as `eve@example.com` with a token minted for her (`keys::mint_token_from_file`, as `setup` does for mia).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/web_bound_e2e.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |

Read only (imported, never modified): `crates/core/src/tools/web_testkit.rs`, `crates/core/src/tools/tool_surface_e2e.rs`.
### Existing patterns
`bt_012_a_disabled_family_call_is_unknown` (`crates/core/src/tools/tool_surface_e2e.rs:487-497`), the log capture of SPEC-0088 E2E-035 (`crates/core/src/tools/office_bound_e2e.rs:939-948`).
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-012: Web family off
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the web family off and a stand in site
- **When** the member calls the page fetch tool on it
- **Then** the call answers that the tool does not exist
- **And** the stand in site received no request
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with the web family off
- Function: `e2e_0093_012_web_family_off`
- Green on arrival: a disabled family adds no route; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-024: Not a member
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `eve@example.com`, not a member of `atlas`, and the stand in site of E2E-SEC-006
- **When** `eve@example.com` downloads it into `/downloads/x.bin` of `atlas`
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the stand in site received no request
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs as eve
- Function: `e2e_0093_024_not_a_member`
- Green on arrival: membership is checked before the request (US-0238 kept the order); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-025: Download address that is not a web address
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` downloads `ftp://127.0.0.1/x` into `/downloads/x.bin` of `atlas`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `url must start with http:// or https://` (the address check comes before membership)
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs as eve
- Function: `e2e_0093_025_download_address_that_is_not_a_web_ad`
- Green on arrival: the address check precedes membership today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-026: Half given destination
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** the stand in site of E2E-SEC-004
- **When** the member fetches it naming project `atlas` and no destination
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `mount_id and save_path must both be provided, or neither`
- **And** the stand in site received no request
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with `mount_id` only
- Function: `e2e_0093_026_half_given_destination`
- Green on arrival: the pair check precedes any request; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-027: Unread file at the destination, answer over the limit
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** High
- **Given** `/downloads/old.bin` holding `old`, never read by the member; a stand in site answering 60 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** `/downloads/old.bin` still reads `old`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_027_unread_file_at_the_destination_answer`
- Green on arrival: the size refusal comes while reading, before the write checks (spec D-12); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-028: Unread file at the destination, answer within the limit
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** Medium
- **Given** `/downloads/old.bin` holding `old`, never read; a stand in site answering `new`
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the refusal is `ERR_EDIT_WITHOUT_PRIOR_READ` with `edit '/downloads/old.bin' requires reading it first in this session`
- **And** `/downloads/old.bin` still reads `old`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_028_unread_file_at_the_destination_answer`
- Green on arrival: within the limit the write checks run as today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-029: Site unreachable
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** no site listens on `127.0.0.1:<closed port>`
- **When** the member downloads `http://127.0.0.1:<closed port>/x` into `/downloads/x.bin`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `fetch request failed: `; nothing left
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs on a closed port
- Function: `e2e_0093_029_site_unreachable`
- Green on arrival: transport failures keep today's message; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-031: Refused download repeated
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** the stand in site of E2E-017
- **When** the member downloads it twice into `/downloads/big.bin`
- **Then** both refusals are the refusal of E2E-017
- **And** nothing left after the second
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs, twice
- Function: `e2e_0093_031_refused_download_repeated`
- Green on arrival: a refusal leaves no state; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-033: What a refused download leaves
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/downloads/keep.bin` holding `keep`, read by the member, and the stand in site of E2E-017
- **When** the member downloads it into `/downloads/keep.bin`
- **Then** the refusal is the refusal of E2E-017
- **And** `/downloads/keep.bin` still reads `keep`; nothing left; the server log holds exactly one line for the refused call, naming `ERR_WRITE_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / MCP route under the log capture / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_033_what_a_refused_download_leaves`
- Green on arrival: one log line per refused call (`log_tool_failure`); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-034: Two floods and a normal fetch at once
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** High
- **Given** members `mia@example.com` and `noa@example.com`, the stand in site of E2E-SEC-004, and the page of E2E-001
- **When** both members fetch the flood and a third call fetches the page, at the same time
- **Then** both floods are refused with the message of E2E-013; the page fetch answers `Hello & welcome`
- **And** each flood call's answer held never exceeds 17 825 792; the stand in site records both connections closed early
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs, three calls joined
- Function: `e2e_0093_034_two_floods_and_a_normal_fetch_at_once`
- Green on arrival: each call has its own reader and record; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-039: Download while the web family is off
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the web family off and the stand in site of E2E-004
- **When** the member calls the download tool on it
- **Then** the call answers that the tool does not exist; the stand in site received no request; no file appears under `/downloads`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with the web family off
- Function: `e2e_0093_039_download_while_the_web_family_is_off`
- Green on arrival: as E2E-012; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-012 | register `web` whatever `enabled`: the stand in receives a request, the test fails |
| E2E-024 | compute the write limit and fetch before `volume()`: the stand in receives a request, the test fails |
| E2E-025 | call `volume()` before the address check: eve gets the membership refusal, the test fails |
| E2E-026 | fetch before checking the pair: the stand in receives a request, the test fails |
| E2E-027 | run the read before write check before the request: `ERR_EDIT_WITHOUT_PRIOR_READ` comes, the test fails |
| E2E-028 | skip `fs_ops::write_bytes`' read before write check for downloads: `old.bin` is replaced, the test fails |
| E2E-029 | map a `send()` error to `read response: ...`: the prefix differs, the test fails |
| E2E-031 | charge the session with the bytes read before refusing: the second call meets a smaller allowance and `nothing_left` fails, the test fails |
| E2E-033 | log the refusal again inside `read_answer` with `tracing::info!`: two lines, the test fails |
| E2E-034 | share one buffer across calls (a `static` capacity reserved at the limit): the records exceed the bound, the test fails |
| E2E-039 | as E2E-012 |

### Closing checks (spec converge)
- Every acceptance test of SPEC-0093 green, run by `make test`: 39 `E2E-` and 10 `E2E-SEC-` tests (`rg -c 'fn e2e_0093_' crates/core/src` totals 49).
- Requirement coverage audit: `comm -23 <(rg -o 'FR-NEW-[0-9]{3}' sdlc/specs/SPEC-0093-*/sdlc-spec.md | sort -u) <(rg -o 'SPEC-0093/FR-NEW-[0-9]{3}' sdlc/stories/US-023[5-9]-*.md sdlc/stories/US-0240-*.md -N --no-filename | sed 's#.*/##' | sort -u)` prints nothing, and US-0235 to US-0239 are `done` or `landed` in `sdlc/stories/_queue.md`.

## Constraints
- Files not to touch: every production file, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: address filtering, the device flow timeout, operator endpoints and remote clone size are side findings, reported, never filed

## Definition of done
- [ ] Green on arrival tests (E2E-012, E2E-024, E2E-025, E2E-026, E2E-027, E2E-028, E2E-029, E2E-031, E2E-033, E2E-034, E2E-039): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` clean
- [ ] Docs updated: `README.md` (optional web and context7 families: the 16 MiB answer limit, downloads stop at the write limit), `AGENTS.md` (`tools/web.rs` and `tools/context7.rs`: capped reads, `test_base`), `.agent_docs/tools.md` (optional families: limits, messages, order), `.agent_docs/testing.md` (filter `e2e_0093_`, the stand in site, `answer_held`, floods under `heavy()`), each entry dated (DEC-010).
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- and E2E-SEC- of SPEC-0093), and every requirement of SPEC-0093 cited by at least one story with status done or landed (the audit above).
- [ ] One commit `US-0240: web refusals in order, without trace, families off, and documentation` on its story branch `story/US-0240-web-refusals-in-order-without-trace-fami`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-007, FR-NEW-008, FR-NEW-010, FR-NEW-001, FR-NEW-005, FR-NEW-009).
