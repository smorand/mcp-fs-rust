# US-0272: index mode change and project removal cancel jobs

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0268
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A change of a project index mode cancels its waiting jobs and supersedes its running ones; a deleted project loses all its jobs (soft delete included) and a restored project lists none.

## Requirements covered
- SPEC-0100/FR-NEW-015: WHEN the index mode of a project changes THE system SHALL cancel and remove its waiting jobs and end its running jobs as `superseded` with their results discarded.
- SPEC-0100/FR-NEW-018: WHEN a project is deleted THE system SHALL cancel and remove all its jobs.
- SPEC-0100/FR-NEW-019: WHEN a deleted project is restored THE system SHALL list no job for it.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_046 e2e_0100_047 e2e_0100_048 e2e_0100_060 e2e_0100_061 e2e_0100_062 e2e_0100_065 e2e_0100_066`. For example: a waiting job exists, the admin sets the mode to `none`, the job list is empty and no rebuild runs after the delay.

## Design excerpt
### Decisions
- DEC-010 (SPEC-0100): Delete or project removal during a run sets the key's `removed` flag before the hook deletes the entries, and the rebuild epilogue calls `delete_path` again when it finds the flag. **Obliged by:** SPEC-0100/FR-NEW-005, FR-NEW-018. **Rationale:** closes the window between the guard and the delete. **Alternatives considered:** a per path lock in every engine. **Source:** design. **Evidence:** n/a.
- DEC-016 (SPEC-0100): A soft delete and an undelete of a project call `Scheduler::forget_project`; a hard delete relies on the cascade. **Obliged by:** SPEC-0100/FR-NEW-018, FR-NEW-019. **Rationale:** the soft delete keeps the `project` row, so no cascade fires. **Alternatives considered:** listing filters deleted projects only. **Source:** existing code. **Evidence:** `storage/admin.rs:510-540`.
### Data model
No schema change; the foreign key cascade covers a hard delete.
### Contracts
`Scheduler::forget_project(project)`: drop waiting entries, set `removed` on running ones, delete rows of the project. Called by `admin.set_index_mode` (after the wipe, before the full index starts), by the soft delete and the undelete of a project (tool and background paths). The full rebuild started by `on_mode_change` is not a job.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/derived/scheduler.rs` | modify: `forget_project` |
| `crates/core/src/mcp/server.rs` | modify: `admin_set_index_mode` (around line 2904) |
| `crates/core/src/tools/admin.rs` | modify: soft delete and undelete call `forget_project` |
| `crates/core/src/search/derived_e2e.rs` | modify: these tests |
### Existing patterns
Mode change wipe: `search/indexer.rs:65-100` and `mcp/server.rs:2877-2920`. Soft delete and undelete: `storage/admin.rs:510-540`, `tools/admin.rs` (find `soft_delete_project` and `undelete_project` with `rg`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-046: the index mode becomes none
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_046_*`)
- **Given** a `search_index` rebuild of `/notes.md` waits in `proj`
- **When** the platform admin sets the index mode of `proj` to `none`
- **Then** the waiting job is removed
- **And** after the settle delay no job exists for `proj`
- **Priority:** High

### SPEC-0100/E2E-047: a project is deleted, then restored
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_047_*`)
- **Given** `proj` has one waiting, one running and one finished job
- **When** the platform admin deletes the project, then restores it
- **Then** while `proj` is deleted, listing its jobs answers `ERR_PROJECT_NOT_FOUND`; after the restore the list is empty
- **And** no rebuild runs for it after the settle delay
- **Priority:** High

### SPEC-0100/E2E-048: the index mode changes between two active modes
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_048_*`)
- **Given** a `search_index` rebuild of `/notes.md` waits; the mode is `rag`
- **When** the platform admin changes the mode to `both`
- **Then** the waiting job is removed
- **And** within 20 s of the mode change (a project of 1 file) the project is rebuilt in full in the mode `both` and search finds the content of `/notes.md` there
- **Priority:** High

### SPEC-0100/E2E-060: a running rebuild when the index mode becomes none
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_060_*`)
- **Given** a `search_index` rebuild of `/notes.md` is running (the embedding service answers in 10 s)
- **When** the platform admin sets the index mode of `proj` to `none`, and the rebuild ends
- **Then** the job ends `superseded` and its entries are not kept
- **And** search finds nothing in `proj`
- **Priority:** High

### SPEC-0100/E2E-061: a project is deleted while a rebuild runs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_061_*`)
- **Given** a rebuild of `/notes.md` is running in `proj`
- **When** the platform admin deletes `proj`, and the rebuild ends
- **Then** while `proj` is deleted, listing its jobs answers `ERR_PROJECT_NOT_FOUND` and the result of the rebuild is not kept; after the restore nothing of the rebuild is searchable
- **And** restoring `proj` later lists no job
- **Priority:** High

### SPEC-0100/E2E-062: jobs of a restored project
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_062_*`)
- **Given** `proj` deleted with 3 waiting jobs
- **When** the platform admin restores `proj`, then waits past the settle delay
- **Then** the job list of `proj` is empty
- **And** the files of `proj` are back and a later save schedules a new rebuild
- **Priority:** Medium

### SPEC-0100/E2E-065: a restored project lists only new jobs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_065_*`)
- **Given** `proj` was deleted with 3 jobs and restored
- **When** Alice saves `/notes.md` once the project is back
- **Then** the job list holds exactly one job, for `/notes.md`
- **And** none of the old jobs reappears
- **Priority:** Medium

### SPEC-0100/E2E-066: no rebuild runs for a deleted project
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_066_*`)
- **Given** `proj` has 3 waiting jobs
- **When** the platform admin deletes `proj` and the settle delay passes
- **Then** no rebuild ran for any file of `proj`
- **And** the embedding service received no call for `proj`
- **Priority:** High

## Constraints
- Do not change the order of the wipe and the full index.
- A deleted project answers `ERR_PROJECT_NOT_FOUND` on listing (already so through `require_member`).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: None beyond US-0273.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0272: index mode change and project removal cancel jobs` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
