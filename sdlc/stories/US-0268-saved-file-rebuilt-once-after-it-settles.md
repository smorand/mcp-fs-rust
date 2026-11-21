# US-0268: a saved file is rebuilt once after it settles

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0264, US-0265, US-0266, US-0267
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
Replace the immediate detached indexing of a write by the scheduler: a change of an indexed project schedules one rebuild that starts when the file has been quiet for the settle delay; each change restarts the wait; a delay of 0 starts it right away; a failure never fails the write and never loses the previous entries; a result made stale by a later change is discarded. This is the core of the feature.

## Requirements covered
- SPEC-0100/FR-NEW-001 (partial: write, edit, patch and upload; copy and restore are US-0269): WHEN the content of a file is created or replaced by a write, an edit, a patch, an upload, a copy or a restore from the trash THE system SHALL schedule one rebuild of its search entries, starting after the settle delay with no further change.
- SPEC-0100/FR-NEW-002: WHEN a file changes THE system SHALL restart the wait of its waiting rebuild, keeping one waiting rebuild per file.
- SPEC-0100/FR-NEW-003: WHILE the settle delay is 0 THE system SHALL start the rebuild of a changed file right after the change, leaving no waiting job.
- SPEC-0100/FR-NEW-004: The system SHALL NOT make a write, edit, patch, move, copy or delete wait for a rebuild, a waiting rebuild or a failed rebuild.
- SPEC-0100/FR-NEW-011: WHILE a rebuild of a file waits or runs THE system SHALL keep serving the previous search entries of that file unchanged.
- SPEC-0100/FR-NEW-012: IF a rebuild fails THEN THE system SHALL set its job to `failed` with the reason, keep the previous search entries, leave the write that caused it successful, and SHALL NOT retry it on its own.
- SPEC-0100/FR-NEW-013 (partial: a change during a running rebuild; the manual case is US-0273): WHEN a running rebuild ends THE system SHALL discard its result and set its job to `superseded` if its file changed since it started or a rebuild by hand was asked for during it.
- SPEC-0100/FR-NEW-014 (partial: the default of 30 seconds is used): The system SHALL read the settle delay as a whole number of seconds, 0 or more, from the key `search.settle_delay_seconds` of the server configuration.
- SPEC-0100/FR-NEW-020: WHEN a rebuild succeeds THE system SHALL replace the previous search entries of the file in one step.
- SPEC-0100/FR-MOD-001: WHEN a file of an indexing project is written THE system SHALL make its new content searchable once the file has been unchanged for the settle delay and the rebuild has ended.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_`. With the clock paused, a member saves a file three times 20 s apart, the test advances time and asserts search finds the last content once, the job list shows exactly one `done` job whose id is the one seen after the first save, and `fs_write` answered at once each time.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0100): The cut in point is `search::indexer::after_*`; no caller of a hook changes. **Obliged by:** SPEC-0100/FR-NEW-001, FR-NEW-004. **Rationale:** about 40 call sites reach the index through these hooks. **Alternatives considered:** wrapping each tool (rejected, drift). **Source:** existing code. **Evidence:** `search/indexer.rs:199-376`, `mcp/server.rs:1757-2261`.
- DEC-003 (SPEC-0100): One scheduler with one dispatcher task and an async mutex over its state, not one timer per write. **Obliged by:** SPEC-0100/FR-NEW-002, FR-NEW-003, FR-NEW-013. **Rationale:** restart of a wait is a field update; one rebuild per key at a time is a map check. **Alternatives considered:** timer per write, `tokio_util::DelayQueue` (new dependency). **Source:** design. **Evidence:** n/a.
- DEC-005 (SPEC-0100): Only the job row is persisted, never `due`; after a restart every waiting row restarts with the full delay. Ids are uuid v4, times RFC 3339 UTC seconds (`chrono`). **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-010. **Rationale:** the spec says the full delay again; no clock arithmetic across restarts. **Alternatives considered:** persist due (needs wall clock math, not asked). **Source:** design. **Evidence:** n/a.
- DEC-007 (SPEC-0100): At delay 0 a changed file gets a `hidden` waiting entry (due now, not listed, not counted as `waiting`); a manual rebuild queued behind a running one is a listed entry with `seconds_left` 0. **Obliged by:** SPEC-0100/FR-NEW-003, FR-NEW-008, FR-NEW-013. **Rationale:** the two cases differ by the spec. **Alternatives considered:** one flag for both (contradicts FR-NEW-008). **Source:** design. **Evidence:** n/a.
- DEC-008 (SPEC-0100): Add `SearchBackend::index_path_checked(..., still_current)` as a provided method (default ignores the guard and calls `index_path`); the four engines and Combined override it. `index_path` keeps its signature. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-013, FR-NEW-020. **Rationale:** the guard must run after embedding and before the commit; callers and the test `RecordingBackend` stay untouched. **Alternatives considered:** change `index_path` (touches `search.index`, `full_index`, tests), a prepare/commit split (more surface). **Source:** existing code. **Evidence:** `search/mod.rs:52-62`, `search/indexer.rs:455`.
- DEC-014 (SPEC-0100): The scheduler reaches the hooks through a `ScheduledBackend` decorator built in `app.rs` (`SearchBackend::scheduler()` default `None`); a backend without scheduler keeps today's immediate indexing, used only by tests that build an `AppState` by hand. **Obliged by:** SPEC-0100/FR-NEW-004. **Rationale:** about 25 test sites build `AppState { search: ... }`; adding a field would touch them all. **Alternatives considered:** `AppState.derived` field, a global. **Source:** existing code. **Evidence:** `tools/trash_search_e2e.rs:78`, `tools/project_removal_e2e.rs:159`, `state.rs:16-28`.
- DEC-015 (SPEC-0100): `search.index`, `search.status` and `after_companion` are not changed. **Obliged by:** SPEC-0100/FR-NEW-001 (business rule). **Rationale:** the spec leaves them out. **Alternatives considered:** none. **Source:** spec. **Evidence:** `search/indexer.rs:213-244`.
- DEC-019 (SPEC-0100): Failure reason: an error whose text starts with `embedding request failed` or `embedding endpoint returned` becomes `embedding service unreachable`; any other error is `<ERR_CODE>: <message>`. **Obliged by:** SPEC-0100/FR-NEW-012. **Rationale:** matches the two failure texts of `embed`. **Alternatives considered:** a typed error (touches the whole `ToolError`). **Source:** existing code. **Evidence:** `search/embedding.rs:33-41`.
- DEC-021 (SPEC-0100): Rebuild reads the file at start, not at schedule time; the hooks pass only a path. **Obliged by:** SPEC-0100/FR-NEW-001. **Rationale:** the content of the last save is what must be indexed. **Alternatives considered:** carry the text (stale, large). **Source:** design. **Evidence:** `search/indexer.rs:185-196` (`after_write_reread`).
### Data model
Rows of `derived_job` as designed (US-0266 created the table). Memory per key: `Waiting { job_id, created_at, due, hidden, cause }`, `Running { job_id, stale, removed }` (design section 3.1). One finished row kept per key (DEC-006).
### Contracts
Rust: `Scheduler::new(backend, store, stores, admin, delay_secs)`, `schedule(project, path, Cause)`, `list(project, path_filter, marker)` (memory for waiting and running, store for finished), dispatcher task; `ScheduledBackend` implements `SearchBackend` by delegation and `scheduler()`; `app.rs` builds `ScheduledBackend` only when search is enabled and passes it as `AppState.search`. Hooks `after_write`, `after_write_reread`, `after_patch`: when `backend.scheduler()` is `Some`, call `schedule`; otherwise keep the old detached task. Failure reason: DEC-019. Guard: `!stale.load()`. `fs.list_derived_jobs` now merges memory and table.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/derived/scheduler.rs` | create: `Scheduler`, dispatcher, rebuild task, failure reason |
| `crates/core/src/search/derived/store.rs` | modify: `insert`, `set_state`, `finish` used by the scheduler |
| `crates/core/src/search/mod.rs` | modify: `ScheduledBackend`, trait method `scheduler()` |
| `crates/core/src/search/indexer.rs` | modify: the three hooks call `schedule` when a scheduler exists |
| `crates/core/src/app.rs` | modify: build the store and the scheduler, wrap the backend |
| `crates/core/src/tools/derived.rs` | modify: list merges memory and table |
| `crates/core/src/search/derived_e2e.rs` | modify: stub embedding server (settable delay, down switch), paused clock helpers, these tests |
| `.agent_docs/search.md` | modify: scheduler section |
### Existing patterns
Hook shape to keep: `after_write_reread` reads the file only when `active_backend` says the mode is active (`search/indexer.rs:185-196`); keep that check so a project with indexing off schedules nothing (E2E-021). Detached task style to replace: `search/indexer.rs:113-128`. Dispatcher shutdown beside the purge loop: `app.rs:176-190`. Fixture style: `tools/trash_search_e2e.rs:60-90`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-001: three saves, one rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_001_*`)
- **Given** project `proj` indexing its files, settle delay 30 seconds, `/notes.md` indexed with `alpha`, Alice (`alice@example.com`) a member
- **When** Alice saves `/notes.md` with `beta` at 0 s, with `gamma` at 20 s and with `delta` at 40 s
- **Then** each save answers at once; at 67 s search finds no `delta`; at 80 s search finds `delta` and no `alpha`
- **And** the job list for `/notes.md` shows exactly one job of kind `search_index`, state `done`, cause `change`, whose id is the id seen in the job list after the first save and whose creation time is the time of the first save
- **Priority:** Critical

### SPEC-0100/E2E-002: delay 0 rebuilds at once
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_002_*`)
- **Given** settle delay 0, `/notes.md` indexed with `alpha`
- **When** Alice saves `/notes.md` with `beta`
- **Then** the save answers at once; at 10 s search finds `beta`
- **And** no job of `/notes.md` is `waiting`
- **Priority:** High

### SPEC-0100/E2E-005: previous entries served while a rebuild waits
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_005_*`)
- **Given** `/notes.md` indexed with `alpha`; settle delay 30 s
- **When** Alice saves it with `beta`, searches `alpha` and `beta` at 5 s, and lists the jobs
- **Then** the search for `alpha` finds `/notes.md`; the search for `beta` finds nothing
- **And** the job list shows one `waiting` job with between 21 and 28 seconds left; after it ends, `alpha` is gone and `beta` is found
- **Priority:** High

### SPEC-0100/E2E-007: index failure keeps the old entries and the write
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_007_*`)
- **Given** `/notes.md` indexed with `alpha`; the embedding service is down; settle delay 2 s
- **When** Alice saves `/notes.md` with `beta`, then waits 12 s
- **Then** the save answered successfully; search for `alpha` still finds `/notes.md`
- **And** the job is `failed` with the reason `embedding service unreachable`; no new job starts by itself during the next 60 s
- **Priority:** Critical

### SPEC-0100/E2E-008: change during a running rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_008_*`)
- **Given** settle delay 3 s; a rebuild of `/notes.md` is running (the embedding service answers in 10 s); the file held `beta`
- **When** Alice saves `/notes.md` with `gamma` while it runs, then the first rebuild ends and the settle delay passes
- **Then** the first job ended `superseded` (visible until the next job of that file and kind ends); search never found `beta` as new content; after the quiet period search finds `gamma`
- **And** a new job was `waiting` between the save and the second rebuild
- **Priority:** High

### SPEC-0100/E2E-009: the boundary of the delay
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_009_*`)
- **Given** settle delay 30 s; instant services
- **When** case A: Alice saves at 0 s and at 29 s and at 58 s; case B: she saves at 0 s and at 45 s
- **Then** case A: nothing starts before 86 s, one rebuild runs and ends by 97 s and the job list shows one `done` job for the three saves; case B: the first rebuild starts between 30 s and 32 s and ends by 37 s, and a second starts between 75 s and 77 s
- **And** in case B the job list shows one `done` job (the last finished job per file and kind)
- **Priority:** High

### SPEC-0100/E2E-018: two files, independent jobs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_018_*`)
- **Given** settle delay 30 s; instant services
- **When** Alice saves `/a.md` at 0 s and `/b.md` at 20 s
- **Then** `/a.md` is rebuilt about 30 s after its save and `/b.md` about 50 s, each within 2 s
- **And** two `done` jobs, one per file
- **Priority:** Medium

### SPEC-0100/E2E-019: names with accents and spaces
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_019_*`)
- **Given** a file `/notes été.md` indexed
- **When** Alice rebuilds it by hand and lists its jobs
- **Then** the job names `/notes été.md` exactly
- **And** the list without a path shows it too
- **Priority:** Medium

### SPEC-0100/E2E-021: a project that does not index schedules nothing
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_021_*`)
- **Given** project `plain` whose index mode is `none`
- **When** Alice saves `/x.md` and waits past the settle delay
- **Then** the list of jobs of `plain` is empty
- **And** the save answered successfully
- **Priority:** High

### SPEC-0100/E2E-023: a write never waits
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_023_*`)
- **Given** the embedding service answers in 30 s; settle delay 0
- **When** Alice saves `/notes.md`, edits it, patches it, moves it and deletes it
- **Then** each call answers in under 1 s
- **And** the rebuild of `/notes.md` starts right after the first save and ends in the background, after the calls answered
- **Priority:** Critical

### SPEC-0100/E2E-029: a burst of saves with a delay of 0
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_029_*`)
- **Given** settle delay 0; the embedding service answers in 3 s
- **When** Alice saves `/notes.md` with `v1`, `v2` and `v3`, one second apart
- **Then** every save answers at once; at most one job of that file and kind is `running` at any time; at 20 s search finds `v3` and neither `v1` nor `v2`
- **And** no job is `waiting` at any time; only a rebuild that had started can have ended `superseded`; the job list ends with one finished job, `done`
- **Priority:** High

### SPEC-0100/E2E-036: recovery after a failure
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_036_*`)
- **Given** the rebuild of `/notes.md` failed (embedding service down)
- **When** the service is back and Alice saves `/notes.md` again
- **Then** a new waiting job starts and ends `done`; search finds the latest content
- **And** the earlier failed job is replaced in the list by the new finished job (one finished job per file and kind)
- **Priority:** High

### SPEC-0100/E2E-040: new content is not searchable before its rebuild ends
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_040_*`)
- **Given** `/notes.md` indexed with `alpha`; settle delay 30 s
- **When** Alice saves it with `beta` and searches `beta` at 15 s
- **Then** search finds nothing for `beta`, and `alpha` is still found
- **And** the job list says `waiting` with about 15 s left
- **Priority:** High

### SPEC-0100/E2E-041: entries are replaced in one step
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_041_*`)
- **Given** `/big.md` indexed as 4 chunks with `old`; a rebuild produces 6 chunks with `new` and takes 10 s
- **When** Alice searches `old` and `new` repeatedly every second during the rebuild
- **Then** every answer shows only old chunks or only new chunks, never both
- **And** after the rebuild, `old` is gone and `new` finds 6 chunks
- **Priority:** High

### SPEC-0100/E2E-043: two failures in a row keep one finished job
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_043_*`)
- **Given** the embedding service is down; settle delay 2 s
- **When** Alice saves `/notes.md`, waits 10 s, saves again and waits 10 s
- **Then** the job list shows one finished job for `/notes.md` and kind `search_index`, `failed`
- **And** its end time is later than the first failure
- **Priority:** High

### SPEC-0100/E2E-044: a discarded result is never visible
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_044_*`)
- **Given** settle delay 3 s; a rebuild of `/notes.md` is running with the content `beta` (the embedding service answers in 10 s for the first rebuild and at once for the next ones); the previous content was `alpha`
- **When** Alice saves `gamma`, and the first rebuild ends
- **Then** search finds `alpha` (the previous entries) and not `beta`, until the second rebuild ends (by 25 s after the start of the first)
- **And** the second rebuild then makes `gamma` searchable
- **Priority:** High

### SPEC-0100/E2E-045: two saves during one running rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_045_*`)
- **Given** a rebuild of `/notes.md` is running (10 s); settle delay 30 s
- **When** Alice saves twice during it, 2 s apart
- **Then** after the running job ends `superseded`, exactly one job of that file and kind is `waiting`
- **And** after the quiet period one rebuild runs and ends `done`
- **Priority:** High

### SPEC-0100/E2E-051: an index failure never fails the write
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_051_*`)
- **Given** the embedding service is down; settle delay 0
- **When** Alice writes `/notes.md` with `beta`
- **Then** the write answers successfully with its byte count; reading the file returns `beta`
- **And** search status shows no new chunks and the job is `failed`
- **Priority:** High

### SPEC-0100/E2E-052: an empty settle delay setting means the default
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_052_*`)
- **Given** a server configuration with the settle delay present but empty
- **When** the operator starts the server
- **Then** the server starts and uses 30 s
- **And** the job list of a saved file shows `waiting` with about 30 s left
- **Priority:** Medium

### SPEC-0100/E2E-053: a configured delay is used
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_053_*`)
- **Given** settle delay 5 s, `/notes.md` indexed with `alpha`
- **When** Alice saves `/notes.md` with `beta`
- **Then** search finds no `beta` at 1 s and finds `beta` at 14 s
- **And** one `done` job
- **Priority:** Medium

### SPEC-0100/E2E-073: a rebuild that fails while Alice reads
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_073_*`)
- **Given** `/notes.md` indexed with `alpha`; Alice saved `beta`; the embedding service is down; settle delay 2 s
- **When** Alice searches `alpha` and `beta` before the rebuild, while it fails, and after it failed
- **Then** every search finds `alpha` and none finds `beta`
- **And** the job is `failed` with the reason `embedding service unreachable`
- **Priority:** High

### SPEC-0100/E2E-077: no mixed entries during a replacement
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_077_*`)
- **Given** `/big.md` indexed as 4 chunks of `old`; settle delay 0; the rebuild produces 6 chunks of `new` and takes 8 s
- **When** Alice searches `new` every second during the rebuild
- **Then** no answer holds both old and new chunks of `/big.md`
- **And** after the rebuild the search finds 6 chunks of `new` and none of `old`
- **Priority:** High

### SPEC-0100/E2E-MOD-001: content searchable after the settle delay (was SPEC-0008/E2E-023)
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_mod001_*`)
- **Validated before:** after a write, search status did not yet count the new chunks, and counted them after 5 seconds; the write answered well under the embedding delay of 2 seconds.
- **Validates now:** with settle delay 3 s and an embedding delay of 2 s, the write answers at once, search status does not yet count the new chunks at 1 s, and counts them at 14 s.
- **Given** an indexing project, settle delay 3 s, embedding service answering in 2 s
- **When** Alice writes a file of 2 chunks
- **Then** the write answers in under 1 s; at 1 s the status counts 0 new chunks; at 14 s it counts 2
- **And** the job list shows one `done` job
- **Priority:** Critical

### SPEC-0100/E2E-MOD-002: a delay of 0 keeps the old timing (was SPEC-0008/E2E-023)
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_mod002_*`)
- **Validated before:** as above, with no delay.
- **Validates now:** with settle delay 0, the behavior of SPEC-0008/E2E-023 holds.
- **Given** an indexing project, settle delay 0, embedding service answering in 4 s
- **When** Alice writes a file of 2 chunks
- **Then** the write answers in under 1 s; at 1 s search status does not yet count the new chunks; at 14 s it counts 2
- **And** no job is `waiting`
- **Priority:** High

## Constraints
- Do not change the delete, move or copy hooks (US-0269, US-0270, US-0271).
- Do not hold the scheduler mutex while a rebuild runs.
- Use `tokio::time` only, so tests can pause the clock.
- The fallback for a backend without scheduler stays for tests that build `AppState` by hand.
- `search.index` and `after_companion` are not changed.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: `.agent_docs/search.md` (scheduler, settle delay, jobs) and `.agent_docs/architecture.md` (one paragraph); full doc pass is US-0273.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0268: a saved file is rebuilt once after it settles` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
