# US-0073: edge cases, scale, docs, spec close

> Spec: SPEC-0049 (sdlc/specs/SPEC-0049-project-folder-usage-breakdown/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0071, US-0072
> min_tier: 1
> Files touched: 6
> Closes spec: yes

## Objective
The remaining behaviors are pinned (empty folders and empty items, the cut list and the cap boundaries, ties by name, accents and emoji, deep folders, unlimited and over limit projects) together with the 5 second target at 100 000 files, the docs carry the new tool and route, and the spec closes with its whole acceptance set.

## Requirements covered
- SPEC-0049/FR-NEW-001: WHEN a platform admin or the project owner reads a project's folder breakdown THE Administration context SHALL show the project id, the limit in whole MB (no value when unlimited), the bytes used, the folder read with its size in bytes and its number of files, the number of items listed, whether the list is cut, and the items.
- SPEC-0049/FR-NEW-002: The Storage context SHALL list as items the direct children of the folder, folders and files together, and SHALL give a folder item the total size and the number of files of everything below it, at any depth.
- SPEC-0049/FR-NEW-004: The Administration context SHALL list the items from the largest to the smallest.
- SPEC-0049/FR-NEW-005: IF the reader gives a cap between 1 and 500, or none, THEN THE Administration context SHALL list at most that many items, 100 when none is given.
- SPEC-0049/FR-NEW-006: WHEN the folder holds more items than are listed THE Administration context SHALL report the list as cut, and otherwise as complete.
- SPEC-0049/FR-NEW-011: The Administration context SHALL show the limit currently set for the project, no value when it is unlimited, whatever the usage.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::folder_usage_e2e`: all 39 tests of SPEC-0049 pass; E2E-039 reads the root of a project of 100 000 files in under 5 seconds and returns `/d0000` first with 10 000 bytes (100 files).

## Design excerpt
### Decisions
- DEC-004 (SPEC-0049): one query reads the file rows under the folder (`path LIKE '<folder>/%'`, pattern from `descendant_pattern`, never by hand), a second reads the direct folder rows (`parent = '<folder>'`); Rust buckets files by the first component after the folder (a name with a further `/` makes a folder item, else a file item) and adds direct folders holding no file with 0 bytes and 0 files.
- DEC-005 (SPEC-0049): items sorted by size descending then path ascending as Rust strings, then truncated to the cap; 0 byte items fall last by the same rule.
- DEC-012 (SPEC-0049): acceptance tests are functional e2e through the real tool method or the data plane router.
- DEC-013 (SPEC-0049): no index and no schema change.
### Data model
None.
### Contracts
No new surface. If E2E-039 is red, fix the engine inside `folder_items` (DEC-004, DEC-013): no index and no schema change; the comparison budget is one pass over the file rows. Docs to write: `README.md` (a paragraph after the disk usage one: the tool, the route, drill down from `/`), `AGENTS.md` (Overview and the `quota.rs` structure line: `folder_usage`), `.agent_docs/tools.md` (one table row after `admin.get_project_disk_usage`: parameters `project_id`, `folder`?, `limit`?, returns, auth), `.agent_docs/api.md` (one route row), `.agent_docs/architecture.md` (a dated entry: bucket by first component, whole component match, lock, trash folder persistence).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/folder_usage_e2e.rs` | modify (add tests) |
| `README.md`, `AGENTS.md` | modify |
| `.agent_docs/tools.md`, `.agent_docs/api.md`, `.agent_docs/architecture.md` | modify |
### Existing patterns
- Tool shape, gate and cap: `get_project_disk_usage`, `reader_project`, `parse_disk_usage_cap`, `crates/core/src/tools/admin.rs:672-760`; its `#[tool]` method `admin_get_project_disk_usage` in `crates/core/src/mcp/server.rs` (SPEC-0047, US-0064).
- Engine and lock: `RelationalMetaStore::largest_files` (`crates/core/src/storage/meta.rs:596-690`) and `quota::disk_usage` (`crates/core/src/quota.rs:179-195`).
- Test harness: `World`, `seed`, `read`, `ok_json`, `err_text`, `rest_usage` in `crates/core/src/tools/disk_usage_e2e.rs` (copy what you need into `folder_usage_e2e.rs`: modules are private).
- Large seed for E2E-039: write the 100 000 file rows through `RelationalMetaStore::put_file` (via `client.meta.put_file(path, Some(sha), 100, MODE_FILE)`) with one shared content blob put once, so the seed does not write 100 000 blobs; create the 1 000 folders with `makedirs`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files, all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000, `/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise. The answer fields are named in the design: `project_id`, `max_mb`, `used_bytes`, `folder` (`path`, `size`, `file_count`), `item_count`, `truncated`, `items` (`path`, `kind`, `size`, `file_count` for a folder only). Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0049/E2E-026: An empty folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-006. **Priority:** High
- **Given** the shared setup, and Olivia has created the empty folder `/empty`
- **When** Olivia reads the folder `/empty`, then the folder breakdown with no folder
- **Then** `/empty` shows 0 bytes, 0 files, 0 items, list complete
- **And** with no folder, `/empty` is listed as a Folder of 0 bytes (0 files) after the four non empty items

### SPEC-0049/E2E-028: Cap boundaries
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-006. **Priority:** High
- **Given** `Atlas` stores the folder `/many` holding `/many/f000.bin` to `/many/f119.bin` of 1 000 bytes each (120 files)
- **When** Olivia reads the folder `/many` with no cap, with a cap of 120, with a cap of 119 and with a cap of 1
- **Then** no cap shows 100 items and the list is cut; a cap of 120 shows 120 items and the list is complete; a cap of 119 shows 119 items and the list is cut; a cap of 1 shows only `/many/f000.bin` and the list is cut
- **And** each answer reports the folder as 120 000 bytes holding 120 files

### SPEC-0049/E2E-029: Equal sizes are ordered by name
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004. **Priority:** High
- **Given** `Atlas` stores `/t/b.txt`, `/t/a.txt` and `/t/C.txt` of 10 bytes each and the folder `/t/sub` holding one file of 10 bytes
- **When** Olivia reads the folder `/t`
- **Then** the order is `/t/C.txt`, `/t/a.txt`, `/t/b.txt`, `/t/sub` (names compared character by character in character code order)

### SPEC-0049/E2E-030: Empty items come last
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004. **Priority:** Medium
- **Given** `Atlas` stores `/z/a.txt` of 10 bytes, `/z/empty.txt` of 0 bytes and the empty folder `/z/e`
- **When** Olivia reads the folder `/z`
- **Then** the order is `/z/a.txt` File 10 bytes, `/z/e` Folder 0 bytes (0 files), `/z/empty.txt` File 0 bytes

### SPEC-0049/E2E-031: Accents and emoji in a folder name
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Medium
- **Given** `Atlas` stores `/rapports été 📊/a.xlsx` of 5 000 bytes
- **When** Olivia reads the folder breakdown with no folder
- **Then** the item shows the path `/rapports été 📊` exactly, Folder 5 000 bytes (1 file)

### SPEC-0049/E2E-032: Five levels deep
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-002. **Priority:** Medium
- **Given** `Atlas` stores `/l1/l2/l3/l4/l5/deep.bin` of 7 000 bytes
- **When** Olivia reads the folder breakdown with no folder, then the folder `/l1/l2/l3`
- **Then** the first answer lists `/l1` Folder 7 000 bytes (1 file); the second lists `/l1/l2/l3/l4` Folder 7 000 bytes (1 file) and shows the folder `/l1/l2/l3` as 7 000 bytes holding 1 file

### SPEC-0049/E2E-035: An unlimited project
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the shared setup with the project limit removed
- **When** Ada reads the folder breakdown with no folder
- **Then** the limit is shown as no value (neither 0 nor text) and the items are those of E2E-001

### SPEC-0049/E2E-036: A project over its limit is readable
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** `Atlas` stores `/big/a.bin` 700 000, `/big/b.bin` 500 000 and `/big/c.bin` 300 000 bytes, and Ada has lowered its limit to 1 MB
- **When** Olivia reads the folder breakdown with no folder
- **Then** limit 1 MB, bytes used 1 500 000, and one item `/big` Folder 1 500 000 bytes (3 files)
- **And** nothing was removed from the project

### SPEC-0049/E2E-039: The default read of a very large project is fast
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002. **Priority:** High
- **Given** `Atlas` stores 100 000 files of 100 bytes spread over 1 000 folders `/d0000` to `/d0999`, 100 files each, and its limit is 20 MB
- **When** Olivia reads the folder breakdown with no folder and no cap
- **Then** the answer arrives within 5 seconds: bytes used 10 000 000, folder `/` of 10 000 000 bytes holding 100 000 files, 100 items shown and the list cut, the first item `/d0000` Folder 10 000 bytes (100 files)

## Constraints
- Files not to touch: the route and the tool function unless E2E-039 forces a change in `folder_items`.
- Dependencies not to add: none.
- Patterns to avoid: an index or a rollup table (DEC-013).
- Scope boundary: none left; this story closes SPEC-0049.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code for E2E-039 only if the read is slower than 5 seconds; E2E-026, 028, 029, 030, 031, 032, 035 and 036 describe behavior US-0069 already provides, so each gets a mutation check instead, recorded in the commit body.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every test of US-0069 to US-0072 stays green; `admin.get_project_disk_usage` and `admin.get_project_quota` unchanged.
- [ ] Docs updated: README.md, AGENTS.md, .agent_docs/tools.md, .agent_docs/api.md, .agent_docs/architecture.md
- [ ] Self review: the answer shape matches DEC-003 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] Closes spec: the 39 acceptance tests of SPEC-0049 are green (E2E-001 to E2E-039), every FR-NEW-001 to FR-NEW-012 is cited by at least one story with status done or landed (US-0069 to US-0073), and the tool counts in code and docs agree.
- [ ] One commit `US-0073: edge cases, scale, docs, spec close`, body `Spec: SPEC-0049` and the requirement ids.
