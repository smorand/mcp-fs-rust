# US-0069: read a project's root folder by size

> Spec: SPEC-0049 (sdlc/specs/SPEC-0049-project-folder-usage-breakdown/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 14
> Closes spec: no

## Objective
An admin or the project owner can read a folder of a project through an AI assistant tool: the folder's size and file count and its direct folders and files, largest first, a folder counting everything below it, with the cap (100 by default, 1 to 500) and the access refusals of the file list. This adds a tool to the frozen contract, so the contract artifacts and the tool counts move in the same commit; no thinner slice exists (floor check), hence `min_tier` 1 for the file count; the decision density is T2.

## Requirements covered
- SPEC-0049/FR-NEW-001 (partial: the `folder` argument is taken as a canonical absolute path here; path rules come in US-0070): WHEN a platform admin or the project owner reads a project's folder breakdown THE Administration context SHALL show the project id, the limit in whole MB (no value when unlimited), the bytes used, the folder read with its size in bytes and its number of files, the number of items listed, whether the list is cut, and the items.
- SPEC-0049/FR-NEW-002: The Storage context SHALL list as items the direct children of the folder, folders and files together, and SHALL give a folder item the total size and the number of files of everything below it, at any depth.
- SPEC-0049/FR-NEW-003 (partial: path, kind, size and file count of each item; the trash folder is asserted in US-0072): The Administration context SHALL show, for each item, its full path, its kind (`Folder` or `File`), its size in bytes, and for a folder its number of files.
- SPEC-0049/FR-NEW-004: The Administration context SHALL list the items from the largest to the smallest.
- SPEC-0049/FR-NEW-005: IF the reader gives a cap between 1 and 500, or none, THEN THE Administration context SHALL list at most that many items, 100 when none is given.
- SPEC-0049/FR-NEW-006 (partial: complete lists here; the cut case is asserted in US-0073): WHEN the folder holds more items than are listed THE Administration context SHALL report the list as cut, and otherwise as complete.
- SPEC-0049/FR-NEW-008: The Administration context SHALL NOT show a project's folder breakdown to anyone who is neither a platform admin nor the project owner.
- SPEC-0049/FR-NEW-011 (partial: the limit is shown as set; the unlimited and over limit cases are asserted in US-0073): The Administration context SHALL show the limit currently set for the project, no value when it is unlimited, whatever the usage.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::folder_usage_e2e`: E2E-001 returns for `Atlas` the root `/` of 1 225 500 bytes and 7 files with the four items `/video` 800 000, `/data` 300 000, `/docs` 124 000, `/readme.md` 1 500, and E2E-012 (cap 0) returns an error ending `limit must be between 1 and 500`. `cargo test -p mcp-fs-core --lib tool_contract` passes against the regenerated golden file.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0049): a new tool `admin.get_project_folder_usage` with its own route; the file list tool and its answer are not changed.
- DEC-003 (SPEC-0049): answer `{project_id, max_mb, used_bytes, folder{path,size,file_count}, item_count, truncated, items[{path,kind,size,file_count?}]}`; `kind` is exactly `Folder` or `File`; a `File` item has no `file_count` key; `item_count` is the number of items listed; `truncated` is true when the folder has more direct items than listed; the folder path is shown without a trailing `/` (root as `/`).
- DEC-004 (SPEC-0049): one query reads the file rows under the folder (`path LIKE '<folder>/%'`, pattern from `descendant_pattern`, never by hand), a second reads the direct folder rows (`parent = '<folder>'`); Rust buckets files by the first component after the folder (a name with a further `/` makes a folder item, else a file item) and adds direct folders holding no file with 0 bytes and 0 files.
- DEC-005 (SPEC-0049): items sorted by size descending then path ascending as Rust strings, then truncated to the cap; 0 byte items fall last by the same rule.
- DEC-006 (SPEC-0049): `quota::folder_usage` holds `client.admission` while it reads the counter with `client.meta.used_bytes()` (never `quota::used_bytes`, which locks again), looks the folder up with `client.meta.get`, and runs `folder_items` in one transaction.
- DEC-011 (SPEC-0049): regenerate the golden contract with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, review the diff; counts move by one.
- DEC-012 (SPEC-0049): acceptance tests are functional e2e through the real tool method or the data plane router.
- DEC-013 (SPEC-0049): no index and no schema change.
### Data model
No schema change. Reads `nodes(volume_id, path, parent, kind, size)`.
### Contracts
Tool `admin.get_project_folder_usage`. Parameters: `project_id` (string, required, description `Id of the project whose folder is read.`), `folder` (string, optional, description `Optional. Absolute path of the folder read, default /.`), `limit` (number, optional, description `Optional. Number of items listed, a whole number from 1 to 500. Default 100.`). Tool description (verbatim): `List one folder of a project (platform admin or project owner): its size and file count, and its direct folders and files from the largest to the smallest, a folder counting everything below it. Trashed files are counted.` Annotations: read only, idempotent, closed world. Registry glue for the golden contract: `.req_str("project_id", ..)`, `.opt_nullable_str`-style for `folder` (use the helper that renders `type ["string","null"]` and `default null`, check `schema.rs`), `.opt_nullable_num("limit", ..)`.
Errors, after the `ERR_<CODE>: ` prefix, in this order: cap invalid `ERR_INVALID_ARGUMENT` `limit must be between 1 and 500`; not admin nor owner `ERR_FORBIDDEN` `'<person>' is not the owner of '<project>'` (also for a missing project); admin, missing project `ERR_PROJECT_NOT_FOUND` `project '<project>' not found`.
Rust:
```rust
// storage/meta.rs, inherent on RelationalMetaStore
pub(crate) struct FolderItemRow { pub path: String, pub is_folder: bool, pub size: i64, pub file_count: i64 }
pub(crate) struct FolderItems { pub total_size: i64, pub total_files: i64, pub item_total: i64, pub items: Vec<FolderItemRow> }
pub(crate) async fn folder_items(&self, folder: &str, cap: usize) -> Result<FolderItems>;
// quota.rs
pub(crate) enum FolderRead { Missing, IsFile, Found { used_bytes: i64, items: FolderItems } }
pub(crate) async fn folder_usage(client: &VolumeClient, folder: &str, cap: usize) -> Result<FolderRead>;
// tools/admin.rs
pub(crate) async fn get_project_folder_usage(ctx: &ToolCtx, project_id: &str, folder: Option<&str>, limit: Option<f64>) -> Result<Value>;
// mcp/server.rs
pub struct GetProjectFolderUsageArgs { pub project_id: String, #[serde(default)] pub folder: Option<String>, #[serde(default)] pub limit: Option<f64> }
pub(crate) async fn admin_get_project_folder_usage(&self, Parameters(a): Parameters<GetProjectFolderUsageArgs>) -> Result<CallToolResult, ErrorData>;
```
In this story `folder_usage` may answer `Missing` or `IsFile` and the tool maps both to `ToolError::internal("folder check comes with US-0070")`; the lock is taken in US-0072 (here only the transaction). The root `/` is always a folder.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`FolderItemRow`, `FolderItems`, `folder_items`) |
| `crates/core/src/quota.rs` | modify (`FolderRead`, `folder_usage`) |
| `crates/core/src/tools/admin.rs` | modify (`get_project_folder_usage`, test registry entry next to the disk usage one) |
| `crates/core/src/mcp/server.rs` | modify (args struct, `#[tool]`, count assertions near the SPEC-0047 comment) |
| `crates/core/src/tools/folder_usage_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`mod folder_usage_e2e;`) |
| `tool-contract-golden.json` | regenerate, never hand edit |
| `TOOL_CONTRACT.txt` | modify (one entry after `admin.get_project_disk_usage`, plus the example line) |
| `crates/core/src/tools/contract_golden.rs` | modify (110 to 111 at the four asserts and the comment, 16 to 17 admin in messages) |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (61 to 62, 114 to 115, `{ 136 } else { 134 }` to `{ 137 } else { 135 }`) |
| `crates/core/src/tools/all.rs` | modify (16 to 17 admin, 68 to 69, 130 to 131 and their comments) |
| `crates/agent/src/mcp.rs` | modify (110 to 111 and the comment) |
| `AGENTS.md`, `.agent_docs/tools.md` | modify (tool totals only: 110 to 111 in every place the totals test reads, the `## admin (16)` heading to 17 and the served table) |
### Existing patterns
- Tool shape, gate and cap: `get_project_disk_usage`, `reader_project`, `parse_disk_usage_cap`, `crates/core/src/tools/admin.rs:672-760`; its `#[tool]` method `admin_get_project_disk_usage` in `crates/core/src/mcp/server.rs` (SPEC-0047, US-0064).
- Engine and lock: `RelationalMetaStore::largest_files` (`crates/core/src/storage/meta.rs:596-690`) and `quota::disk_usage` (`crates/core/src/quota.rs:179-195`).
- Test harness: `World`, `seed`, `read`, `ok_json`, `err_text`, `rest_usage` in `crates/core/src/tools/disk_usage_e2e.rs` (copy what you need into `folder_usage_e2e.rs`: modules are private).
- Row reading: `r.text(0)`, `r.i64(1)`, `tx.query(&Query::new(sql).bind(..))` as in `largest_files`. Literal example:
```rust
let rows = tx.query(&Query::new("SELECT path, size FROM nodes WHERE volume_id=?1 AND kind='file' AND path LIKE ?2 ESCAPE '\\'").bind(&self.volume_id).bind(self.descendant_pattern(folder))).await?;
```

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files, all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000, `/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise. The answer fields are named in the design: `project_id`, `max_mb`, `used_bytes`, `folder` (`path`, `size`, `file_count`), `item_count`, `truncated`, `items` (`path`, `kind`, `size`, `file_count` for a folder only). Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0049/E2E-001: The owner reads the root folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-011. **Priority:** Critical
- **Given** the shared setup
- **When** Olivia reads the folder breakdown of `Atlas` with no folder and no cap
- **Then** she sees project `Atlas`, limit 2 MB, bytes used 1 225 500, folder `/` of 1 225 500 bytes holding 7 files, 4 items, list complete: `/video` Folder 800 000 bytes (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/readme.md` File 1 500
- **And** the four item sizes add up to 1 225 500, the folder total

### SPEC-0049/E2E-002: The admin reads the same root
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Ada reads the folder breakdown of `Atlas` with no folder
- **Then** she gets exactly the answer of E2E-001

### SPEC-0049/E2E-003: Drilling into a folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003. **Priority:** Critical
- **Given** the shared setup
- **When** Olivia reads the folder `/video` of `Atlas`
- **Then** she sees folder `/video` of 800 000 bytes holding 2 files, 2 items, list complete: `/video/demo.mp4` File 600 000 bytes, `/video/clips` Folder 200 000 bytes (1 file)

### SPEC-0049/E2E-004: A folder of many small files outranks one bigger file
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-004, FR-NEW-006. **Priority:** Critical
- **Given** `Atlas` stores `/logs/l0001.txt` to `/logs/l1000.txt` of 100 bytes each and `/big.bin` of 90 000 bytes, nothing else
- **When** Olivia reads the folder breakdown of `Atlas` with no folder
- **Then** bytes used is 190 000 and the items are, in order: `/logs` Folder 100 000 bytes (1 000 files), `/big.bin` File 90 000 bytes

### SPEC-0049/E2E-007: A member is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Mia reads the folder breakdown of `Atlas`
- **Then** she is refused with `'mia@example.com' is not the owner of 'Atlas'`

### SPEC-0049/E2E-008: An outsider is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Otto reads the folder breakdown of `Atlas`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Atlas'`

### SPEC-0049/E2E-009: An outsider naming a missing project is refused the same way
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup; no project `Ghost` exists
- **When** Otto reads the folder breakdown of `Ghost`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Ghost'`

### SPEC-0049/E2E-010: The admin naming a missing project
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup; no project `Ghost` exists
- **When** Ada reads the folder breakdown of `Ghost`
- **Then** she is refused with `project 'Ghost' not found`

### SPEC-0049/E2E-011: The owner of another project is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup, and Bram owns `Borealis`
- **When** Bram reads the folder breakdown of `Atlas`
- **Then** he is refused with `'bram@example.com' is not the owner of 'Atlas'`

### SPEC-0049/E2E-012: A cap of 0 is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of 0
- **Then** she is refused with `limit must be between 1 and 500` and sees no item

### SPEC-0049/E2E-013: A cap of 501 is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of 501
- **Then** she is refused with `limit must be between 1 and 500` and sees no item

### SPEC-0049/E2E-014: A negative or fractional cap is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** Medium
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of -5, then with a cap of 2.5
- **Then** each time she is refused with `limit must be between 1 and 500` and sees no item

### SPEC-0049/E2E-018: A refusal reveals nothing about the files
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup
- **When** Otto reads the folder breakdown of `Atlas`
- **Then** the refusal contains none of the names `video`, `readme`, none of the sizes `800000`, `1225500`, and no file count of `Atlas`

## Constraints
- Files not to touch: `get_project_disk_usage` and its tests; `reader_project` and `parse_disk_usage_cap` (reuse them as they are).
- Dependencies not to add: none.
- Patterns to avoid: building a `LIKE` pattern by hand; ordering ties in SQL; hand editing `tool-contract-golden.json`; a second copy of the owner check.
- Scope boundary: path rules and folder refusals (US-0070), the route (US-0071), the lock, trash and read only assertions (US-0072), edge cases, scale and docs (US-0073).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (add the tool method first as a stub answering an internal error, so each test fails on its assertion and not on a missing symbol; E2E-007 to E2E-011 and E2E-018 may pass on a refusal-free stub).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `admin.get_project_disk_usage` and `admin.get_project_quota` answer exactly as before (the 36 tests of `disk_usage_e2e.rs` and the 97 of `quota_e2e.rs` stay green); the frozen contract differs from before by exactly the one new tool.
- [ ] Docs updated: AGENTS.md and .agent_docs/tools.md totals only (the rest in US-0073)
- [ ] Self review: the answer shape matches DEC-003 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0069: read a project's root folder by size`, body `Spec: SPEC-0049` and the requirement ids.
