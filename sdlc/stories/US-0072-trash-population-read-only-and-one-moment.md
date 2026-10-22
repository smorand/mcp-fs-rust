# US-0072: trash, population, read only and one moment

> Spec: SPEC-0049 (sdlc/specs/SPEC-0049-project-folder-usage-breakdown/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0069
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A read never shows half of a write: it holds the project's admission lock for its duration. The tests also pin that a delete moves bytes into the trash folder, a restore and a purge follow, shared content counts per path, a name that starts another's name never absorbs it, and reading changes nothing.

## Requirements covered
- SPEC-0049/FR-NEW-003 (partial: the trash folder listed like any other folder, and kept (empty) after restore and purge): The Administration context SHALL show, for each item, its full path, its kind (`Folder` or `File`), its size in bytes, and for a folder its number of files.
- SPEC-0049/FR-NEW-007: The Storage context SHALL count the same files as usage counts: all files of the project, trashed files included, each path once even when files share content.
- SPEC-0049/FR-NEW-010: The Administration context SHALL NOT change any file, the bytes used, the trash, the reader's session write budget or the answer of the file list of SPEC-0047 when it shows a folder breakdown, and SHALL take every figure of one answer from the same moment.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::folder_usage_e2e`: E2E-037 holds the project's admission lock, starts a read, asserts the read has not finished, writes `/docs/new.txt` (30 000 bytes), releases the lock, and the read answers `/docs` of 154 000 bytes (3 files) with `used_bytes` 1 255 500.

## Design excerpt
### Decisions
- DEC-006 (SPEC-0049): `quota::folder_usage` holds `client.admission` while it reads the counter with `client.meta.used_bytes()` (never `quota::used_bytes`, which locks again), looks the folder up with `client.meta.get`, and runs `folder_items` in one transaction.
- DEC-010 (SPEC-0049): no special case for the trash folder: it is a stored directory listed like any other and it stays after restore and purge because nothing removes it.
- DEC-012 (SPEC-0049): acceptance tests are functional e2e through the real tool method or the data plane router.
### Data model
None.
### Contracts
No new surface. In `quota::folder_usage` take `let _admission = client.admission.lock().await;` before `client.meta.used_bytes()` (a read through `meta`, not `quota::used_bytes`, which would lock a second time and deadlock). The trash folder needs no code (DEC-010): E2E-005, 023, 024, 025 assert the existing behavior.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/quota.rs` | modify (lock in `folder_usage`) |
| `crates/core/src/tools/folder_usage_e2e.rs` | modify (add tests) |
### Existing patterns
- Tool shape, gate and cap: `get_project_disk_usage`, `reader_project`, `parse_disk_usage_cap`, `crates/core/src/tools/admin.rs:672-760`; its `#[tool]` method `admin_get_project_disk_usage` in `crates/core/src/mcp/server.rs` (SPEC-0047, US-0064).
- Engine and lock: `RelationalMetaStore::largest_files` (`crates/core/src/storage/meta.rs:596-690`) and `quota::disk_usage` (`crates/core/src/quota.rs:179-195`).
- Test harness: `World`, `seed`, `read`, `ok_json`, `err_text`, `rest_usage` in `crates/core/src/tools/disk_usage_e2e.rs` (copy what you need into `folder_usage_e2e.rs`: modules are private).
- Lock test without sleeps: `crates/core/src/tools/disk_usage_e2e.rs`, `spec_0047_e2e_032_*` (hold `client.admission.clone().lock_owned()`, `tokio::spawn` the read, `yield_now` 50 times, assert `!handle.is_finished()`, write through `client.write_bytes_atomic`, drop the guard).
- Delete, restore, purge helpers: `delete`, `trashed` and the restore call in the same file (`TrashRestoreArgs`; purge is `fs_delete` with `trash: false` on the trashed path; the fixture sets `allow_hard_delete`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files, all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000, `/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise. The answer fields are named in the design: `project_id`, `max_mb`, `used_bytes`, `folder` (`path`, `size`, `file_count`), `item_count`, `truncated`, `items` (`path`, `kind`, `size`, `file_count` for a folder only). Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0049/E2E-005: The trash folder shows what the trash weighs
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-007. **Priority:** High
- **Given** the shared setup, and Mia has deleted `/docs/notes.txt`
- **When** Olivia reads the folder breakdown of `Atlas` with no folder
- **Then** bytes used is still 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 120 000 (1 file), `/.mcp_trash` Folder 4 000 (1 file), `/readme.md` File 1 500
- **And** the item sizes add up to 1 225 500

### SPEC-0049/E2E-021: Reading changes nothing
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** Critical
- **Given** the shared setup, and Olivia has written `/session.bin` of 100 000 bytes this session (budget 262 144)
- **When** Olivia reads the folder breakdown of `Atlas` three times with no folder and once for `/video`
- **Then** the three answers for no folder are identical: bytes used 1 325 500, items in order `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/session.bin` File 100 000, `/readme.md` File 1 500
- **And** the usage read still shows 1 325 500 bytes used, the files are unchanged, and Olivia can still write 162 144 bytes in this session (a write of 162 145 bytes is refused with `session write quota of 262144 bytes exceeded`)

### SPEC-0049/E2E-022: The file list of the previous breakdown is unchanged
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` before and after `admin_get_project_folder_usage` / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the file list (largest files first) before and after reading the folder breakdown twice
- **Then** both file lists are identical: bytes used 1 225 500, 7 files, in order `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/video/clips/a.mp4` 200 000, `/docs/spec.docx` 120 000, `/data/old/x.csv` 50 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500, list complete

### SPEC-0049/E2E-023: Deleting moves bytes into the trash folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-003. **Priority:** High
- **Given** the shared setup
- **When** Mia deletes `/data/export.csv`, then Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/.mcp_trash` Folder 250 000 (1 file), `/docs` Folder 124 000 (2 files), `/data` Folder 50 000 (1 file), `/readme.md` File 1 500

### SPEC-0049/E2E-024: Restoring brings the bytes back
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the shared setup, `/data/export.csv` deleted then restored by Mia
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/readme.md` File 1 500, followed by `/.mcp_trash` Folder 0 bytes (0 files); 5 items, list complete
- **And** the item sizes add up to 1 225 500

### SPEC-0049/E2E-025: A purge removes the bytes
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the shared setup, `/docs/notes.txt` deleted by Mia and then removed for good by the purge
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 221 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 120 000 (1 file), `/readme.md` File 1 500, followed by `/.mcp_trash` Folder 0 bytes (0 files); 5 items, list complete
- **And** the item sizes add up to 1 221 500

### SPEC-0049/E2E-033: A folder name that starts another folder's name
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-007. **Priority:** High
- **Given** `Atlas` stores `/doc/a.bin` of 1 000 bytes and `/docs/b.bin` of 2 000 bytes
- **When** Olivia reads the folder breakdown with no folder, then the folder `/doc`
- **Then** the first answer lists `/docs` Folder 2 000 bytes (1 file) then `/doc` Folder 1 000 bytes (1 file); the second shows folder `/doc` as 1 000 bytes holding 1 file with the single item `/doc/a.bin` File 1 000 bytes

### SPEC-0049/E2E-034: Files sharing content count once per path
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** `Atlas` stores `/a/copy1.bin` and `/a/copy2.bin` with identical content of 100 000 bytes
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 200 000 and `/a` shows as Folder 200 000 bytes (2 files)

### SPEC-0049/E2E-037: A write during the read is seen entirely or not at all
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` plus `fs_delete`, `fs_trash_restore`, `admin_get_project_disk_usage` (E2E-022) and the admission lock (E2E-037) / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the shared setup
- **When** Mia writes `/docs/new.txt` of 30 000 bytes while Olivia reads the folder breakdown with no folder
- **Then** the answer is either the answer of E2E-001 or the same answer with bytes used 1 255 500 and `/docs` Folder 154 000 bytes (3 files); in both cases the item sizes add up to the folder total
- **And** a second read afterwards shows the second form

## Constraints
- Files not to touch: `meta.rs`, the route, the tool function.
- Dependencies not to add: none.
- Patterns to avoid: `sleep` to order the lock test; calling `quota::used_bytes` inside the lock.
- Scope boundary: edge cases, scale and docs (US-0073).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code for E2E-037 only (it fails while the read does not wait for the lock); E2E-005, 021, 022, 023, 024, 025, 033 and 034 describe behavior US-0069 already provides, so each gets a mutation check instead (break the behavior it asserts, see the test fail, revert), recorded in the commit body.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every test of US-0069 to US-0071 stays green.
- [ ] Docs updated: none in this story
- [ ] Self review: the answer shape matches DEC-003 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0072: trash, population, read only and one moment`, body `Spec: SPEC-0049` and the requirement ids.
