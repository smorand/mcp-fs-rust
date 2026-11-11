# US-0239: stop a download at the project storage limit

> Spec: SPEC-0093 (sdlc/specs/SPEC-0093-bound-the-web-fetch-body-read-into-memory/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0238
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Where a project has a storage limit, a download may be stopped by the project side of the write limit before the session side: the storage left plus the file it replaces. Pin that side: a download of exactly what the project can take, an overwrite counting the replaced file, a chunked or declared answer past it refused with today's project message and its net growth figure, a tie answered by the project, and a session already partly spent.

## Requirements covered
- SPEC-0093/FR-NEW-003 (partial: the project side, the tie, a session partly spent): IF the answer of a download or of a page fetch saved into a project passes the write limit THEN THE web lookup SHALL stop reading it and refuse with the refusal of the side that sets the write limit.
- SPEC-0093/FR-NEW-004 (partial: the project side): IF the answer of a download or of a saved page declares a length past the write limit THEN THE web lookup SHALL refuse without reading any byte of its body, with the refusal of the side that sets the write limit.
- SPEC-0093/FR-NEW-007 (partial: item 3: a tie between the sides): IF several refusals apply to one call THEN THE web lookup SHALL report the first met in this order: (1) today's argument checks of the tool (a download address not starting with `http://` or `https://`: `url must start with http:// or https://`; a page fetch given only one of the project and the destination: `mount_id and save_path must both be provided, or neither`); (2) for a download or a saved page, membership (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`) and the destination path checks, as today; (3) while asking and reading, whichever comes first: a transport failure, the time limit, a declared length past the limit (FR-NEW-002, FR-NEW-004), the limit passed while reading (FR-NEW-001, FR-NEW-003); (4) for a download or a saved page, today's write checks on the whole answer: a file at the destination not read first in the session (`ERR_EDIT_WITHOUT_PRIOR_READ`, `edit '<path>' requires reading it first in this session`), then the project storage limit, then the session allowance.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0093_009 e2e_0093_010 e2e_0093_019 e2e_0093_020 e2e_0093_021 e2e_0093_022 e2e_0093_023`: 7 tests pass. Real assertions: the project's used bytes, `bytes_written`, `overwritten`, the exact project message figures, the replaced file's bytes, held records.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0093):** `write_limit` runs after `volume()` (membership) and `norm()` (path), before the request: `replaced` = the size of the file at the destination when it exists (`client.stat`), else 0; project side = `project_remaining_for(client, safety, replaced)` (`None` without a limit), with `limit` and `used` kept for the message; session side = `session_remaining(person, mount)`; the cap is the lower, a tie goes to the project. The whole answer then goes to `fs_ops::write_bytes`, whose checks (read before write, then project, then session) run as today on the whole answer. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007. **Rationale:** SPEC-0084's choice of the lower side, tie to the project (`archive_single.rs:246-253`).
- **DEC-005 (SPEC-0093):** `WriteLimit::refusal(declared)`: session side -> the session message with the configured quota; project side -> `project_quota_exceeded(limit, used, requested)` with `requested = declared - replaced` when a declared length triggered it, `limit - used + 1` when passed while reading. **Obliged by:** SPEC-0093/FR-NEW-003, FR-NEW-004 (E2E-019 to E2E-022). **Rationale:** spec D-7 (net growth).
### Contracts
As US-0238 (`write_limit`, `WriteLimit::refusal`). Limits are set with `state.admin.set_quota("atlas", Some(limit))` after seeding the used bytes; the session is pre spent by a write of 50 000 000 bytes through `core::fs_ops::write_bytes` (E2E-023).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/web_bound_e2e.rs` | modify |
| `crates/core/src/tools/web.rs` | modify only if a test is red after US-0238 (the project side was built there) |

Read only (imported, never modified): `crates/core/src/tools/web_testkit.rs`.
### Existing patterns
`archive_testkit::limit_remaining` (`crates/core/src/tools/archive_testkit.rs:66-71`) and SPEC-0084's project side tests (`crates/core/src/tools/archive_single.rs` tests around `pick_caps`).
### Fixtures
`web_testkit.rs` (DEC-008, DEC-009): `pub(crate) struct Record { pub(crate) requests: AtomicUsize, pub(crate) body_sent: AtomicU64, pub(crate) closed_early: AtomicBool }`; `pub(crate) enum Script { Fixed { body: Vec<u8>, content_type: Option<String>, chunked: bool, encoding: Option<String> }, Letters { letter: u8, n: u64, chunked: bool }, Flood { endless: bool, encoding: Option<String> }, Declared { length: u64, pause: Duration, then: Box<Script> }, Trickle { per_second: u8, seconds: u32 }, Status { code: u16, declared: u64 } }`; `pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>)` (raw `tokio::net::TcpListener` on `127.0.0.1:0`, reads the request line and headers, picks the script by path prefix, writes the status line and headers, then the body in 65 536 byte pieces with chunked framing when chunked, counting body bytes written; a write error before the scripted end sets `closed_early`); `pub(crate) async fn served(web: bool, context7_base: Option<String>, test_base: Option<String>) -> (Router, String, Arc<AppState>, TempDir)` (`tool_surface_e2e::setup` then `c.web.enabled`, `c.web.test_base`, `c.context7.enabled` and `api_url`, `app_with_atlas`, `/downloads`, `/pages`, `/docs` created); `pub(crate) fn answer_held_reset()`, `pub(crate) fn answer_held(url: &str) -> Vec<u64>` (panics when no record); `pub(crate) async fn nothing_left(state: &AppState, folder: &str, path: &str, before: Option<&[u8]>)` (the folder listing without `path`, or `path` holding `before`; no write audit line for `path`; a 52 428 800 byte write of `/docs/fill.bin` by `mia@example.com` succeeds).
The flood literal (spec 9.3): `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` 1 073 741 824 times as fast as the socket takes it.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0093/E2E-009: Download of exactly the project side
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes and 1 000 000 bytes used; a stand in site answering 1 097 152 bytes, chunked
- **When** the member downloads it into `/downloads/fit.bin`
- **Then** the answer reports 1 097 152 bytes written; the project uses 2 097 152 bytes
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_009_download_of_exactly_the_project_side`
- Green on arrival: the project side admits exactly what is left (US-0238); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-010: Overwrite counts the replaced file
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes, 1 000 000 bytes used of which `/downloads/old.bin` holds 500 000 and was read by the member; a stand in site answering 1 597 152 bytes, chunked
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the answer reports 1 597 152 bytes written, overwritten
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_010_overwrite_counts_the_replaced_file`
- Green on arrival: the replaced file counts (US-0238); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-019: Download over the project side, chunked
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes and 1 000 000 bytes used; a stand in site answering 5 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/over.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 1097153 bytes requested`
- **And** the answer held never exceeds 2 145 728; nothing left
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_019_download_over_the_project_side_chunke`
- Green on arrival: US-0238 built the project refusal; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-020: Download declaring a length over the project side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the project of E2E-019 and a stand in site declaring 5 000 000 bytes, waiting 10 seconds after its headers
- **When** the member downloads it into `/downloads/over.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 5000000 bytes requested`, before the first body byte is sent
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_020_download_declaring_a_length_over_the`
- Green on arrival: as E2E-019; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-021: Overwrite over the project side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** High
- **Given** the project and file of E2E-010; first a stand in site answering 1 597 153 bytes, chunked; then one declaring 1 597 153 bytes
- **When** the member downloads each into `/downloads/old.bin`
- **Then** both refusals are `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 1097153 bytes requested`
- **And** `/downloads/old.bin` still holds its 500 000 bytes
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_021_overwrite_over_the_project_side`
- Green on arrival: as E2E-010; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-022: Tie between the two sides
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Medium
- **Given** `atlas` with a storage limit of 62 914 560 bytes and 10 485 760 bytes used (project side 52 428 800, equal to the session side); a stand in site answering 52 428 801 bytes, chunked
- **When** the member downloads it into `/downloads/tie.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 60 MB exceeded: 10485760 bytes used, 52428801 bytes requested`
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_022_tie_between_the_two_sides`
- Green on arrival: a tie goes to the project (US-0238); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0093/E2E-023: Session partly spent
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the member already wrote 50 000 000 bytes in the session; a stand in site answering 3 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/late.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 3 477 376
- Level / Driver / Location: functional e2e / MCP route of the served app, `call_tool(&app, &token, "web.download", ..)`, `#[tokio::test]` / crates/core/src/tools/web_bound_e2e.rs
- Function: `e2e_0093_023_session_partly_spent`
- Green on arrival: the session side is what the session can still write (US-0238); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-009 | compute the project side without the replaced file and refuse when `held >= cap`: the exact fit is refused, the test fails |
| E2E-010 | set `replaced` to 0: the project side is 1 097 152 and the download is refused, the test fails |
| E2E-019 | report `requested` as the cap (no `+ 1`): the message reads 1097152, the test fails |
| E2E-020 | report `requested` as `limit - used + 1` for a declared length too: the message reads 1097153, the test fails |
| E2E-021 | as E2E-010: the refusal figures differ, the test fails |
| E2E-022 | break a tie toward the session: the session message comes, the test fails |
| E2E-023 | word the session refusal with the remaining bytes instead of the configured quota: the message differs, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/git/provider/mod.rs` (its reader is the model, not shared), `crates/core/src/docs/service.rs`, `Cargo.toml` (no reqwest feature)
- Dependencies not to add: none
- A production change here means US-0238 was incomplete: say so in the commit body
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] Green on arrival tests (E2E-009, E2E-010, E2E-019, E2E-020, E2E-021, E2E-022, E2E-023): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 5 tests of `tools::web`, the 4 of `tools::context7`, `bt_012` and `bt_014` of `tools::tool_surface_e2e`, `tools::all` counts, every earlier `e2e_0093_` test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0239: stop a download at the project storage limit` on its story branch `story/US-0239-stop-a-download-at-the-project-storage-l`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0093-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0093` and the requirement ids (SPEC-0093/FR-NEW-003, FR-NEW-004, FR-NEW-007).
