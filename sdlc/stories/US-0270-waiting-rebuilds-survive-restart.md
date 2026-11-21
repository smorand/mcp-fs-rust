# US-0270: waiting rebuilds survive a restart

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0268
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
At boot, put every waiting job back in the scheduler with the full settle delay, and end every job that was running as `failed` with `interrupted by a server restart` and schedule a new rebuild for its file.

## Requirements covered
- SPEC-0100/FR-NEW-007: The system SHALL keep every waiting rebuild across a server restart, each restarting with the full settle delay.
- SPEC-0100/FR-NEW-017: WHEN the server starts THE system SHALL end each rebuild that was running at its last stop as `failed` with the reason `interrupted by a server restart` and schedule a new waiting rebuild for its file with the full settle delay.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_011 e2e_0100_034 e2e_0100_035 e2e_0100_054 e2e_0100_067 e2e_0100_072`. The test drops the first `Scheduler`, builds a second one over the same store, calls `recover()` and asserts the job list and the rebuilds that follow.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0100): Only the job row is persisted, never `due`; after a restart every waiting row restarts with the full delay. Ids are uuid v4, times RFC 3339 UTC seconds (`chrono`). **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-010. **Rationale:** the spec says the full delay again; no clock arithmetic across restarts. **Alternatives considered:** persist due (needs wall clock math, not asked). **Source:** design. **Evidence:** n/a.
- DEC-020 (SPEC-0100): Boot recovery runs in `app.rs` after the stores are built, before the router serves: `Scheduler::recover()` ends `running` rows as `failed` (`interrupted by a server restart`) and re queues each, and re queues `waiting` rows with the full delay. Shutdown stops the dispatcher and leaves rows as they are. **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-017. **Rationale:** rows are the truth; the dispatcher is stateless. **Alternatives considered:** lazy recovery on first request. **Source:** existing code. **Evidence:** `app.rs:160-190`.
### Data model
No schema change. `recover()` reads rows with state `waiting` or `running` through `JobStore::load_unfinished`.
### Contracts
`Scheduler::recover(&self) -> Result<()>`: for each `running` row `finish(failed, "interrupted by a server restart")` then `schedule(change)`; for each `waiting` row re queue with `due = now + delay` (hidden when delay is 0). Called in `app.rs` right after the scheduler is built and before the server accepts requests; if recovery fails the boot fails with the error. Shutdown drops the dispatcher; rows stay as they are.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/derived/scheduler.rs` | modify: `recover` |
| `crates/core/src/search/derived/store.rs` | modify: `load_unfinished` if not present |
| `crates/core/src/app.rs` | modify: call `recover` |
| `crates/core/src/search/derived_e2e.rs` | modify: these tests |
### Existing patterns
Boot order and error propagation: `app.rs:60-160` (the admin store is connected before anything else; the search backend failure is mapped with `anyhow!`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-011: a restart keeps the waiting rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_011_*`)
- **Given** Alice saved `/notes.md` with `beta`; the rebuild waits with 20 s left
- **When** the operator restarts the server
- **Then** the job list shows `/notes.md` `waiting` with between 28 and 30 s left (the full delay)
- **And** after 40 quiet seconds search finds `beta` and the job is `done`
- **Priority:** Critical

### SPEC-0100/E2E-034: a restart while a rebuild runs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_034_*`)
- **Given** a rebuild of `/notes.md` is running (the embedding service answers in 60 s)
- **When** the operator restarts the server
- **Then** the interrupted job is `failed` with the reason `interrupted by a server restart`
- **And** a new job for the same file and kind is `waiting` with the full settle delay left, and after it ends the content is searchable
- **Priority:** High

### SPEC-0100/E2E-035: a restart with nothing pending
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_035_*`)
- **Given** all jobs `done`
- **When** the operator restarts the server
- **Then** no job is `waiting` or `running`
- **And** the finished jobs are still listed
- **Priority:** Medium

### SPEC-0100/E2E-054: two running rebuilds at a restart
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_054_*`)
- **Given** a rebuild of `/a.md` runs and one of `/b.md` waits with 10 s left
- **When** the operator restarts the server
- **Then** the job of `/a.md` is `failed` with `interrupted by a server restart` and a new waiting job exists for it; the job of `/b.md` is `waiting` with the full delay
- **And** after the delay both files are rebuilt; with a settle delay of 0 instead, the new rebuild of `/a.md` starts right after the start and is not listed as `waiting`
- **Priority:** Medium

### SPEC-0100/E2E-067: two restarts in a row
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_067_*`)
- **Given** Alice saved `/notes.md`; the rebuild waits
- **When** the operator restarts the server twice, 5 s apart
- **Then** the job list shows `/notes.md` `waiting` with the full delay left after each restart
- **And** after the delay one rebuild runs and ends `done`
- **Priority:** Medium

### SPEC-0100/E2E-072: a restart interrupts a rebuild while the embedding service is down
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_072_*`)
- **Given** a rebuild of `/a.md` runs; the embedding service goes down; settle delay 3 s
- **When** the operator restarts the server and waits 15 s
- **Then** the first job is `failed` with `interrupted by a server restart` and the new job is `failed` with `embedding service unreachable`
- **And** the previous entries of `/a.md` are still searchable
- **Priority:** High

## Constraints
- Never restore `due` from the database.
- A rebuild that was running is not retried before the new waiting job is due.
- No new tool.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: None beyond US-0273.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0270: waiting rebuilds survive a restart` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
