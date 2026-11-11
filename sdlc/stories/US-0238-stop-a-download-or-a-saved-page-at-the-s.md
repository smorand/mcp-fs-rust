# US-0238: stop a download or a saved page at the session allowance

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0237
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A download or a page saved into a project is read whole before the write allowance is checked, so a flood costs the server its whole size before the refusal it would get anyway. After this story the write limit (the session allowance, and the project storage limit where one applies) is computed after membership and before the request, and the answer is refused once it passes that limit, or before its body when its declared length does, with today's storage refusals; within the limit the write is as today.

## Requirements covered
- SPEC-0093/FR-NEW-003 (partial: the session side): IF the answer of a download or of a page fetch saved into a project passes the write limit THEN THE web lookup SHALL stop reading it and refuse with the refusal of the side that sets the write limit.
- SPEC-0093/FR-NEW-004 (partial: the session side): IF the answer of a download or of a saved page declares a length past the write limit THEN THE web lookup SHALL refuse without reading any byte of its body, with the refusal of the side that sets the write limit.
- SPEC-0093/FR-NEW-005 (partial: downloads and saved pages): THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- SPEC-0093/FR-NEW-006 (partial: a compressed download kept as received): THE web lookup and the library documentation lookup SHALL count against every limit of this specification each byte of the answer as the server keeps it, an answer marked compressed being counted and kept as received.
- SPEC-0093/FR-NEW-008: THE web lookup SHALL NOT write or change a file, charge the session write allowance or the project storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-004.
- SPEC-0093/FR-NEW-009 (partial: a download and a saved page within the allowance): THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_sec_005 e2e_0093_sec_006 e2e_0093_sec_007 e2e_0093_sec_010 e2e_0093_004 e2e_0093_005 e2e_0093_008 e2e_0093_017 e2e_0093_018 e2e_0093_037`: 10 tests pass. Real assertions: the file bytes, `bytes_written`, `content_type`, the refusal code and message, `nothing_left`, held records, the stand in's record, the time to refuse.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0093):** `write_limit` runs after `volume()` (membership) and `norm()` (path), before the request: `replaced` = the size of the file at the destination when it exists (`client.stat`), else 0; project side = `project_remaining_for(client, safety, replaced)` (`None` without a limit), with `limit` and `used` kept for the message; session side = `session_remaining(person, mount)`; the cap is the lower, a tie goes to the project. The whole answer then goes to `fs_ops::write_bytes`, whose checks (read before write, then project, then session) run as today on the whole answer. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007. **Rationale:** SPEC-0084's choice of the lower side, tie to the project (`archive_single.rs:246-253`).
- **DEC-005 (SPEC-0093):** `WriteLimit::refusal(declared)`: session side -> the session message with the configured quota; project side -> `project_quota_exceeded(limit, used, requested)` with `requested = declared - replaced` when a declared length triggered it, `limit - used + 1` when passed while reading. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004 (E2E-019 to E2E-022). **Rationale:** spec D-7 (net growth).
- **DEC-003 (SPEC-0093):** the held hook: `#[cfg(test)] thread_local! ANSWER_HELD: RefCell<Vec<AnswerHeld>>`; a guard created in `read_answer` before anything else pushes `{url, held: 0}` and updates `held` to the largest `buf + piece` seen, in `Drop`, so every return path (declared refusal 0, read refusal, success, read error) leaves exactly one record; a failure status answered before the body (context7) records 0 because the guard is created by a small `start_answer(url)` call right after `send()` returns, before the status check. Readers: `answer_held_reset()`, `answer_held(url) -> Vec<u64>` (panics when empty). **Obliged by:** SPEC-0093/FR-NEW-005, 7.6. **Rationale:** a hook that counted only successful reads would hide the refusal path; the tests assert a lower bound on read refusals (at least cap plus 1).
### Data model
`struct WriteLimit { cap: u64, side: Side, quota: i64, project: Option<ProjectSide> }`, `enum Side { Session, Project }`, `struct ProjectSide { limit: i64, used: i64, replaced: i64 }` in `web.rs`.
### Contracts
- `async fn write_limit(client: &VolumeClient, safety: &SafetyManager, person: &str, mount: &str, path: &str) -> Result<WriteLimit>`: `replaced` from `client.stat(path)` when the file exists, else 0; project side from `quota::project_remaining_for(client, safety, replaced)` with `safety.project_limit(&client.project_id)` and `quota::used_bytes(client)` kept for the message; session side `safety.session_remaining(person, mount)`; cap = the lower, a tie to the project (in this story the project side is computed but only the session refusal is exercised; US-0239 pins the project side).
- `WriteLimit::refusal(&self, declared: Option<u64>) -> ToolError`: session: `ToolError::write_quota_exceeded(format!("session write quota of {quota} bytes exceeded"))`; project: `ToolError::project_quota_exceeded(limit, used, requested)`, `requested` = `declared - replaced` or `limit - used + 1`.
- Save mode and `web.download`: `volume()`, `norm()`, then `write_limit(..)`, then `fetch_raw(&url, timeout, limit.cap, |d| limit.refusal(d))`, then `fs_ops::write_bytes` as today.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/web.rs` | modify |
| `crates/core/src/tools/web_testkit.rs` | modify (`nothing_left` for writes) |
| `crates/core/src/tools/web_bound_e2e.rs` | modify |
### Existing patterns
SPEC-0084's choice of sides (`crates/core/src/tools/archive_single.rs:185-253`, `pick_caps` `:246-253`); `quota::project_remaining_for` (`crates/core/src/quota.rs:221-231`); `SafetyManager::session_remaining` (`crates/core/src/safety.rs:153-156`); the save and download handlers (`crates/core/src/tools/web.rs:96-164`).
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-SEC-005: Saved page flood
- **Covers:** occurrence 3, page saved into a project
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the whole body is read before the write allowance is checked.
- **Given** a stand in site answering a flood at `/flood`
- **When** the member fetches it into `/pages/flood.html` of `atlas`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; the stand in site records the connection closed early; nothing left
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.fetch", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs with `mount_id` and `save_path`
- Function: `e2e_0093_sec_005_saved_page_flood`
- Red before the change because: today the flood is read whole before the allowance check: the record is 1 073 741 824
- Note: take `archive_testkit::heavy()` first; the RED run holds about 1 GiB.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-006: Download flood (originating occurrence)
- **Covers:** occurrence 3, download
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the whole body is read before the write allowance is checked.
- **Given** a stand in site answering a flood at `/flood`, then continuing endlessly
- **When** the member downloads it into `/downloads/flood.bin` with a time limit of 120 seconds
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`, within 30 seconds of the call
- **And** the answer held never exceeds 53 477 376; the stand in site records the connection closed early; nothing left
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.download", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_006_download_flood_originating_occurr`
- Red before the change because: today the flood is read whole before the allowance check: the record is the body length
- Note: take `heavy()` first. The RED run uses the stand in without its endless tail (the 1 073 741 824 byte flood only): today's code would otherwise read until the 120 s time limit and exhaust memory; the committed test uses the endless flood.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-007: Download declaring a huge length
- **Covers:** occurrence 3, declared length
- **Red before the fix because:** `Then the refusal ... before the first body byte is sent` fails: today the server waits for the body and reads it.
- **Given** a stand in site answering `HTTP/1.1 200 OK`, `Content-Length: 1073741824`, then waiting 10 seconds before a flood of the same length
- **When** the member downloads it into `/downloads/flood.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`, before the first body byte is sent
- **And** the answer held is 0; the stand in site records 0 body bytes sent; nothing left
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.download", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_007_download_declaring_a_huge_length`
- Red before the change because: today the server waits 10 s and reads the declared gigabyte
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-SEC-010: Compressed marked download flood
- **Covers:** occurrence 3, compressed answer
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the 1 073 741 824 bytes are read whole.
- **Given** a stand in site answering a flood marked `Content-Encoding: gzip`
- **When** the member downloads it into `/downloads/flood.gz`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; nothing left
- Level / Driver / Location: exploit (functional e2e) / MCP route, `call_tool(&app, &token, "web.download", ..)`, stand in flood / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_sec_010_compressed_marked_download_flood`
- Red before the change because: today the flood is read whole
- Note: take `heavy()` first.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-004: Download within the allowance
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/report.pdf` with 1 000 000 bytes, declared, marked `application/pdf`
- **When** the member downloads it into `/downloads/report.pdf` of `atlas`
- **Then** the answer names `/downloads/report.pdf`, 1 000 000 bytes written, not overwritten, content type `application/pdf`
- **And** the file holds the 1 000 000 bytes sent; the audit trail has one write line for it; a following write of 51 428 801 bytes by the member is refused with `session write quota of 52428800 bytes exceeded` (the download was charged)
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_004_download_within_the_allowance`
- Green on arrival: today's download within the allowance; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-005: Page saved into a project within the allowance
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/page` with `<p>Saved</p>`, chunked
- **When** the member fetches it into `/pages/saved.html` of `atlas`
- **Then** the answer names `/pages/saved.html` with 12 bytes written
- **And** the file holds `<p>Saved</p>`; the audit trail has one write line for it
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.fetch", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs with `mount_id` and `save_path`
- Function: `e2e_0093_005_page_saved_into_a_project_within_the`
- Green on arrival: today's save within the allowance; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-008: Download of exactly the session side
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** a stand in site answering 52 428 800 bytes, chunked
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the answer reports 52 428 800 bytes written
- **And** the answer held never exceeds 53 477 376
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_008_download_of_exactly_the_session_side`
- Green on arrival: today 52 428 800 bytes are read and written; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-017: Download one byte over the session side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-005, FR-NEW-008; **Priority:** Critical
- **Given** a stand in site answering 52 428 801 bytes, chunked
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; nothing left
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_017_download_one_byte_over_the_session_si`
- Green on arrival: today the 52 428 801 bytes are read whole and the write is refused with the same message, within the same bound: the boundary is pinned, not changed; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-018: Download declaring one byte over the session side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site declaring 52 428 801 bytes, waiting 10 seconds after its headers
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the refusal of E2E-017 arrives before the first body byte is sent
- **And** the stand in site records 0 body bytes sent; nothing left
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_018_download_declaring_one_byte_over_the`
- Red before the change because: today the server waits 10 s and reads the body: `body_sent` is 52 428 801
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-037: Compressed download saved as received
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a stand in site answering 1 000 000 bytes marked as compressed, declared
- **When** the member downloads it into `/downloads/packed.gz`
- **Then** the answer reports 1 000 000 bytes written; the file holds the bytes exactly as sent
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_037_compressed_download_saved_as_received`
- Green on arrival: no decompression today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-004 | write `content_type` from the request's `Accept` header: `*/*` instead of `application/pdf`, the test fails |
| E2E-005 | strip the HTML before saving: the file holds `Saved`, the test fails |
| E2E-008 | refuse when `held >= cap`: the download of exactly the allowance is refused, the test fails |
| E2E-017 | word the session refusal with the remaining bytes instead of the configured quota in `WriteLimit::refusal`: the message reads 0, the test fails |
| E2E-037 | as US-0235 E2E-011 (decode `gzip`): the file differs from the bytes sent, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/git/provider/mod.rs` (its reader is the model, not shared), `crates/core/src/docs/service.rs`, `Cargo.toml` (no reqwest feature)
- Dependencies not to add: none
- Compute the write limit after `volume()` and `norm()`, before the request (FR-NEW-007 items 2 and 3)
- Leave `fs_ops::write_bytes` and its checks unchanged; the read before write check stays after the read (spec D-12)
- No dash as punctuation in code, comments or docs
- Scope boundary: the project side tests are US-0239; order and trace tests US-0240

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-005, E2E-SEC-006, E2E-SEC-007, E2E-SEC-010, E2E-018 (tests and fixtures first, on the previous story code, each failing for the reason stated).
- [ ] Green on arrival tests (E2E-004, E2E-005, E2E-008, E2E-017, E2E-037): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 5 tests of `tools::web`, the 4 of `tools::context7`, `bt_012` and `bt_014` of `tools::tool_surface_e2e`, `tools::all` counts, every earlier `e2e_0093_` test; every `fs.write_bytes` and quota test (`cargo test -p mcp-fs-core --lib quota`)
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0238: stop a download or a saved page at the session allowance` on its story branch `story/US-0238-stop-a-download-or-a-saved-page-at-the-s`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-008, FR-NEW-009).
