# US-0269: a copy and a restore schedule their destination

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0268
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A copy (file or folder) and a restore from the trash schedule a rebuild for each destination file, with the settle delay and the same rules as a write, instead of indexing at once.

## Requirements covered
- SPEC-0100/FR-NEW-001 (partial: copy and restore from the trash): WHEN the content of a file is created or replaced by a write, an edit, a patch, an upload, a copy or a restore from the trash THE system SHALL schedule one rebuild of its search entries, starting after the settle delay with no further change.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_049 e2e_0100_063 e2e_0100_064 e2e_0100_068`. After `fs_copy` of a folder of two files, the job list shows two `waiting` jobs for the destination paths and none for the source; after the delay both destinations are searchable.

## Design excerpt
### Decisions
- DEC-017 (SPEC-0100): A restore from the trash calls a new `after_restore` (clear the trash path, schedule the destination) instead of `after_move`. **Obliged by:** SPEC-0100/FR-NEW-001, FR-NEW-006. **Rationale:** a move moves jobs and re-indexes at once; a restore is a change. **Alternatives considered:** a flag on `after_move`. **Source:** existing code. **Evidence:** `tools/trash.rs:206-218`.
### Data model
None.
### Contracts
`indexer::after_copy(state, volume_id, destination, client)` schedules every file under `destination` (skipping the trash folder) and returns without indexing. New `indexer::after_restore(state, volume_id, trash_path, restored_path, client)`: delete the trash path entries (as `after_move` clears its sources) then schedule `restored_path`. A backend without scheduler keeps the old behavior.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/indexer.rs` | modify: `after_copy`, new `after_restore` |
| `crates/core/src/tools/trash.rs` | modify: call `after_restore` instead of `after_move` (line 206-218) |
| `crates/core/src/search/derived_e2e.rs` | modify: these tests |
### Existing patterns
Folder walk for a destination: `index_tree` and `files_under` in `search/indexer.rs` (the same walk, scheduling instead of indexing). Trash restore call: `tools/trash.rs:206-218`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-049: a copy and a restore schedule the destination only
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_049_*`)
- **Given** `/a.md` indexed; `/trash` holds a deleted `/d.md`
- **When** Alice copies `/a.md` to `/a2.md` and restores `/d.md`
- **Then** one waiting job names `/a2.md` and one names `/d.md`
- **And** none names `/a.md`
- **Priority:** Medium

### SPEC-0100/E2E-063: a copy schedules its destination
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_063_*`)
- **Given** `/a.md` indexed, settle delay 30 s
- **When** Alice copies `/a.md` to `/a2.md`
- **Then** the copy answers at once; one `waiting` job names `/a2.md` and none names `/a.md`
- **And** after 45 s search finds the content under both `/a.md` and `/a2.md`
- **Priority:** High

### SPEC-0100/E2E-064: a copy in a project that does not index
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_064_*`)
- **Given** project `plain` (index mode `none`) holds `/a.md`
- **When** Alice copies `/a.md` to `/a2.md` and waits past the settle delay
- **Then** no job is listed for `plain`
- **And** the copy answered successfully
- **Priority:** High

### SPEC-0100/E2E-068: a copy of a folder schedules every file under it
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_068_*`)
- **Given** folder `/docs` holds `/docs/a.md` and `/docs/b.md`, both indexed; settle delay 30 s
- **When** Alice copies `/docs` to `/docs2`
- **Then** two `waiting` jobs name `/docs2/a.md` and `/docs2/b.md`, none names a path under `/docs`
- **And** after 45 s search finds the content under `/docs2`
- **Priority:** Medium

## Constraints
- Do not change `after_move` (US-0271).
- A project with indexing off schedules nothing.
- No new tool or route.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: None beyond US-0273.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0269: a copy and a restore schedule their destination` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
