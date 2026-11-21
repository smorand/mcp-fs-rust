# US-0271: delete and move follow the jobs

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0268
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
A delete cancels the waiting rebuild, removes every job of the path or folder, and handles a running rebuild; a move carries the waiting job (with the time already waited) and the finished jobs to the new path, supersedes a running one and starts a fresh wait, and a move over an existing file removes the replaced path jobs.

## Requirements covered
- SPEC-0100/FR-NEW-005: WHEN a file is deleted THE system SHALL cancel its waiting rebuild, end its running rebuild as `superseded` with its result discarded, and remove every job of that path, finished ones included.
- SPEC-0100/FR-NEW-006: WHEN a file is moved THE system SHALL make its waiting rebuild name the new path with the time already waited, end its running rebuild as `superseded` with its result discarded and schedule a new waiting rebuild with the full settle delay, and make its finished jobs name the new path.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_010 e2e_0100_024 e2e_0100_030 e2e_0100_031 e2e_0100_032 e2e_0100_033 e2e_0100_042 e2e_0100_075`. For example: save `/b.md`, advance 10 s, `fs_move` it to `/c.md`, assert one `waiting` job on `/c.md` with about 20 s left and none on `/b.md`.

## Design excerpt
### Decisions
- DEC-010 (SPEC-0100): Delete or project removal during a run sets the key's `removed` flag before the hook deletes the entries, and the rebuild epilogue calls `delete_path` again when it finds the flag. **Obliged by:** SPEC-0100/FR-NEW-005, FR-NEW-018. **Rationale:** closes the window between the guard and the delete. **Alternatives considered:** a per path lock in every engine. **Source:** design. **Evidence:** n/a.
- DEC-018 (SPEC-0100): A move keeps indexing the destination immediately and without a job (as today) and moves jobs by `src`: `after_move` gains the `src` argument. **Obliged by:** SPEC-0100/FR-NEW-006. **Rationale:** the spec says entries follow the move within 5 s without a job. **Alternatives considered:** schedule the destination (contradicts FR-NEW-006). **Source:** existing code. **Evidence:** `search/indexer.rs:322-345`.
### Data model
No schema change.
### Contracts
`Scheduler::forget_path(project, path)` (sets `removed` on a running job, deletes rows, drops the waiting entry); `Scheduler::move_path(project, from, to)`. `indexer::after_move(state, volume_id, src: &str, sources: &[String], destination: &str, client)`: for each source file `p`: if `p == src` or starts with `src/`, `move_path(p, destination + p[src.len()..])`, else `forget_path(p)` (replaced destination); the existing immediate clear and `index_tree` of the destination stay (no job). `after_delete` and `after_delete_many` call `forget_path` first (flag), then the existing immediate entry delete. Epilogue of a removed running job: `delete_path` once more (DEC-010).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/indexer.rs` | modify: `after_move` (new `src` argument), `after_delete`, `after_delete_many` |
| `crates/core/src/search/derived/scheduler.rs` | modify: `forget_path`, `move_path`, epilogue for `removed` |
| `crates/core/src/mcp/server.rs` | modify: pass `src` at the `after_move` call (around line 2234) |
| `crates/core/src/api/dataplane.rs` | modify: pass `src` (around line 595) |
| `crates/core/src/search/derived_e2e.rs` | modify: these tests |
### Existing patterns
Collection of paths before the operation: `paths_under` and `paths_displaced_by_move` in `search/indexer.rs` (keep them, they already return the files of a folder and of an overwritten destination). Call sites: `mcp/server.rs:2194-2234`, `api/dataplane.rs:550-595`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-010: a delete cancels the waiting rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_010_*`)
- **Given** a rebuild of `/a.md` waits
- **When** Alice deletes `/a.md`, then waits past the settle delay
- **Then** no job names `/a.md`; search never finds its content
- **And** no rebuild runs for `/a.md` (the job count of the project stays 0)
- **Priority:** High

### SPEC-0100/E2E-024: a move keeps the wait
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_024_*`)
- **Given** Alice saved `/b.md` 10 s ago; settle delay 30 s
- **When** she moves it to `/c.md`
- **Then** one `waiting` job names `/c.md` with 20 s left (within 2 s), none names `/b.md`
- **And** 40 s after the original save search finds the content under `/c.md`
- **Priority:** High

### SPEC-0100/E2E-030: a delete during a running rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_030_*`)
- **Given** a rebuild of `/a.md` is running (the embedding service answers in 10 s)
- **When** Alice deletes `/a.md`, and the rebuild ends
- **Then** search never finds the content of `/a.md`
- **And** no job names `/a.md` in the job list
- **Priority:** High

### SPEC-0100/E2E-031: deleting a file with nothing pending
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_031_*`)
- **Given** `/a.md` indexed, no job waiting, one finished job
- **When** Alice deletes `/a.md`
- **Then** search no longer finds it
- **And** no job names `/a.md`
- **Priority:** High

### SPEC-0100/E2E-032: moving a file with nothing pending
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_032_*`)
- **Given** `/b.md` indexed, no job waiting, one finished job
- **When** Alice moves it to `/c.md`
- **Then** a search read 2 s after the move answered finds nothing under `/b.md`, and within 5 s it finds the content under `/c.md`; no waiting job is created
- **And** the finished job now names `/c.md`
- **Priority:** Medium

### SPEC-0100/E2E-033: a move onto an existing file with jobs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_033_*`)
- **Given** `/b.md` has a waiting job; `/c.md` exists and has a waiting job and a finished job; a rebuild of `/b.md` is not running
- **When** Alice moves `/b.md` over `/c.md` (the move replaces it)
- **Then** the jobs that named the replaced `/c.md` are removed; one `waiting` job names `/c.md` with the time already waited by `/b.md`
- **And** none names `/b.md`
- **Priority:** Medium

### SPEC-0100/E2E-042: a failed job is removed when the file is deleted
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_042_*`)
- **Given** the rebuild of `/notes.md` ended `failed`
- **When** Alice deletes `/notes.md`
- **Then** no job names `/notes.md`
- **And** listing the project jobs shows no `failed` job for it
- **Priority:** High

### SPEC-0100/E2E-075: a move while the file is being rebuilt
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_075_*`)
- **Given** settle delay 3 s; a rebuild of `/b.md` runs (the embedding service answers in 10 s)
- **When** Alice moves `/b.md` to `/c.md`, and the rebuild ends
- **Then** the running job ends `superseded` and names `/c.md`; one new job names `/c.md` and waits the full delay; a search finds nothing under `/b.md`
- **And** after the delay `/c.md` is searchable
- **Priority:** High

## Constraints
- Keep the immediate entry removal on delete and move (not a job).
- A job removed while running leaves no record when its computation ends.
- `after_restore` (US-0269) does not call `after_move`.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: None beyond US-0273.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0271: delete and move follow the jobs` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
