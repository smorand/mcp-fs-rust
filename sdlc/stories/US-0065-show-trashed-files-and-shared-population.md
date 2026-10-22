# US-0065: show trashed files and the shared population

> Spec: SPEC-0047 (sdlc/specs/SPEC-0047-project-disk-usage-breakdown/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0064
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
The breakdown now tells apart files in the trash: they carry the state `In trash` and the path they had before they were deleted, including files inside a trashed folder and legacy trashed files with no tracking row. The population is exactly the one usage counts, so a delete moves a file to `In trash` without changing the bytes used and a purge removes it.

## Requirements covered
- SPEC-0047/FR-NEW-003: The Administration context SHALL show, for each listed file, its full path, its size in bytes, and its state: `Active`, or `In trash`.
- SPEC-0047/FR-NEW-006: The Administration context SHALL base the breakdown on the files that usage counts: all files of the project, trashed files included, each path once even when files share content.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::disk_usage_e2e`: E2E-003 returns, after Mia deletes `/docs/notes.txt`, an entry of size 4 000 with `state` `In trash` and `original_path` `/docs/notes.txt`, while `used_bytes` stays 975 500 and `file_count` stays 5.

## Design excerpt
### Decisions
- DEC-008 (SPEC-0047): `In trash` means the path lies under the trash directory (`SafetyConfig.trash_dir`, default `.bin`); the original path is the `original_path` of the entry row for the first two path components (`/<trash_dir>/<stamp>__<flat>`) plus the remainder of the path; the rows are first backfilled with `tools::trash::backfill_trash_entries(client, trash_dir)` and read with `get_trash_entry`.
- DEC-004 (SPEC-0047): `state` is exactly `Active` or `In trash`; `original_path` is `null` for `Active`.
- DEC-005 (SPEC-0047): the count and the list stay in one transaction; the trash rows are read in that same transaction.
### Data model
No schema change. Reads `trash_entries(volume_id, trash_path, original_path)`.
### Contracts
No new surface. `largest_files(&self, cap, trash_dir)` now sets, for each returned row, `original_path: Some(..)` when the row is a trashed file and the answer's `state` becomes `In trash`. `get_project_disk_usage` calls `backfill_trash_entries(&client, &state.safety.config().trash_dir)` before `quota::disk_usage`. Trash directory comparison is by whole component: `/.bin/x` is trashed, `/.binary/x` is not.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`largest_files`: trashed detection, original path) |
| `crates/core/src/tools/admin.rs` | modify (backfill call, `state` label) |
| `crates/core/src/tools/disk_usage_e2e.rs` | modify (add tests) |
### Existing patterns
- Tool shape and gate: `crates/core/src/tools/admin.rs:647-676` (`get_project_quota`) and its `#[tool]` method `crates/core/src/mcp/server.rs:2918-2929`.
- Test harness: `crates/core/src/tools/quota_e2e.rs:60-139` (`World`, `as_`, `seed`, `ok_json`); a second harness for the router: `crates/core/src/tools/diff_files_e2e.rs:709-750`.
- Backfill and trash entry lookup: `crates/core/src/tools/trash.rs:135` (`backfill_trash_entries`), `crates/core/src/storage/meta.rs:1510` (`get_trash_entry(tx, volume, trash_path)`).
- Test helpers for deletes: `delete_to_trash` and `trashed(name)` in `crates/core/src/tools/quota_e2e.rs` (copy them into `disk_usage_e2e.rs`; example of a permanent purge: `quota_e2e.rs:1863-1880`, `fs_delete` with `trash: false` on the trashed path, needs `c.safety.allow_hard_delete = true`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec): project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores five files, all `Active`: `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (975 500 in all). Olivia (`olivia@example.com`) owns Atlas, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Every session budget is 262 144 bytes. "The standard answer" means: project `Atlas`, limit 2, bytes used 975 500, 5 files, list complete, entries in the order of the table above, each `Active`. Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0047/E2E-003: A trashed file is listed as in the trash
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-003, 006
- **Given** the shared setup, and Mia has deleted `/docs/notes.txt`
- **When** Olivia reads the breakdown
- **Then** the entry of size 4 000 is `In trash`, shows the path where it is stored in the trash and
  the original path `/docs/notes.txt`; the four other entries are `Active`
- **And** 5 files, bytes used 975 500, list complete

### SPEC-0047/E2E-005: A purged file disappears
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-001, 006
- **Given** the shared setup, Mia has deleted `/docs/notes.txt`, and the purge has removed it
- **When** Olivia reads the breakdown
- **Then** 4 files, bytes used 971 500, no entry of size 4 000, list complete

### SPEC-0047/E2E-006: The sizes add up to the usage
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-006, 010
- **Given** `Atlas` stores 120 files of 1 000 bytes, and Olivia reads with a cap of 20
- **When** the answer is returned
- **Then** bytes used is 120 000, files 120, 20 entries shown, list cut
- **And** reading again with a cap of 500 gives 120 entries whose sizes add up to 120 000

### SPEC-0047/E2E-019: Deleting moves a file to the trash
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-003, 006
- **Given** the shared setup
- **When** Mia deletes `/data/export.csv`, then Olivia reads the breakdown
- **Then** 5 files, bytes used 975 500; the second entry has size 250 000, is `In trash` and shows the
  original path `/data/export.csv`; its position among the entries is unchanged

### SPEC-0047/E2E-020: Restoring brings a file back to Active
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-003
- **Given** the shared setup, and `/data/export.csv` deleted then restored by Mia
- **When** Olivia reads the breakdown
- **Then** the standard answer, every entry `Active`

### SPEC-0047/E2E-029: Files sharing content each show their full size
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** `Atlas` stores `/a/copy1.bin` and `/a/copy2.bin` with identical content of 100 000 bytes
- **When** Olivia reads the breakdown
- **Then** both entries show 100 000 and bytes used is 200 000

### SPEC-0047/E2E-031: History, exports and artifacts are not counted
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` plus `fs_delete` / `fs_trash_restore` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** `Atlas` stores one file `/a.txt` of 10 000 bytes, has version control history of earlier
  versions, one zip export, and two images kept from a converted document
- **When** Olivia reads the breakdown
- **Then** 1 file, bytes used 10 000, one entry `/a.txt` 10 000

## Constraints
- Files not to touch: `tools/trash.rs` and `fs.trash_list` behavior; `quota.rs`.
- Dependencies not to add: none.
- Patterns to avoid: joining only on `trash_path = path` (misses files inside a trashed folder); string prefix test without a component boundary.
- Scope boundary: the route (US-0066), the lock and docs (US-0067).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code for E2E-003, E2E-019 and the nested case (they fail while every file reads `Active`); E2E-005, E2E-006, E2E-020, E2E-029 and E2E-031 may be green on first run, and each gets a mutation check instead (change the code to break the rule, see the test fail, revert).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: US-0064 tests stay green; `fs.trash_list` and `fs.trash_restore` tests unchanged.
- [ ] Docs updated: none in this story (US-0067 writes the docs)
- [ ] Self review: the answer shape matches DEC-004 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment, not a WHAT comment.
- [ ] One commit `US-0065: show trashed files and the shared population`, body `Spec: SPEC-0047` and the requirement ids.
