# US-0064: list a project's largest files

> Spec: SPEC-0047 (sdlc/specs/SPEC-0047-project-disk-usage-breakdown/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 12
> Closes spec: no

## Objective
An admin or the project owner can read a project's files from the largest to the smallest through an AI assistant tool: limit, bytes used, file count, a cap (100 by default, 1 to 500) and a sign when the list is cut. Anyone else is refused with the quota read's texts. This adds a tool to the frozen contract, so the contract artifacts and the tool counts move in the same commit: no thinner slice exists (floor check), hence `min_tier` 1 for the file count; the decision density is T2.

## Requirements covered
- SPEC-0047/FR-NEW-001: WHEN a platform admin or the project owner reads a project's breakdown THE Administration context SHALL show the project id, the limit in whole MB (no value when unlimited), the bytes used, the number of files, whether the list is cut, and the list of files.
- SPEC-0047/FR-NEW-002: The Administration context SHALL list the files from the largest to the smallest.
- SPEC-0047/FR-NEW-003 (partial: every file shows state `Active`; the `In trash` state and the original path come in US-0065): The Administration context SHALL show, for each listed file, its full path, its size in bytes, and its state: `Active`, or `In trash`.
- SPEC-0047/FR-NEW-004: IF the reader gives a cap between 1 and 500, or none, THEN THE Administration context SHALL list at most that many files, 100 when none is given.
- SPEC-0047/FR-NEW-005: WHEN the project holds more files than are listed THE Administration context SHALL report the list as cut, and otherwise as complete.
- SPEC-0047/FR-NEW-007: The Administration context SHALL NOT show a project's breakdown to anyone who is neither a platform admin nor the project owner.
- SPEC-0047/FR-NEW-008 (partial: the limit is shown as set; the unlimited and over limit cases are asserted in US-0067): The Administration context SHALL show the limit currently set for the project, no value when it is unlimited, whatever the usage.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::disk_usage_e2e`: E2E-001 returns for `Atlas` the five entries `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 with `used_bytes` 975 500, `file_count` 5, `truncated` false, and E2E-012 (cap 0) returns an error ending `limit must be between 1 and 500`. `cargo test -p mcp-fs-core --lib tool_contract` passes against the regenerated golden file.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0047): a new tool `admin.get_project_disk_usage` with its own route; `admin.get_project_quota` is not changed.
- DEC-003 (SPEC-0047): top N by size in SQL, ties at the cutoff resolved in Rust: `SELECT size FROM nodes WHERE volume_id=?1 AND kind='file' ORDER BY size DESC LIMIT 1 OFFSET <cap-1>` gives the cutoff (none: fewer than `cap` files, take all); fetch every file with `size >=` cutoff; sort by size descending then `path` with Rust `str` order (UTF-8 byte order equals code point order); truncate to cap.
- DEC-004 (SPEC-0047): answer `{project_id, max_mb, used_bytes, file_count, truncated, entries:[{path, size, state, original_path}]}`; `state` is exactly `Active` or `In trash`; `original_path` is `null` for `Active`; `truncated = file_count > entries.len()`.
- DEC-005 (SPEC-0047), the lock part is built in US-0067: here read `used_bytes` with `client.meta.used_bytes()` and do the count and the list in one transaction.
- DEC-006 (SPEC-0047): `limit` is `Option<f64>`; valid when finite, whole, 1..=500; absent is 100; otherwise `ToolError::invalid_argument("limit must be between 1 and 500")`.
- DEC-007 (SPEC-0047): extract the gate of `get_project_quota` into `fn reader_project(ctx, project_id) -> Result<Project>` used by both reads, behavior unchanged.
- DEC-010 (SPEC-0047): no schema change, no new index.
- DEC-011 (SPEC-0047): regenerate the golden contract with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, review the diff; counts move by one.
- DEC-012 (SPEC-0047): acceptance tests are functional e2e through the real `McpServer` method.
### Data model
No schema change. Reads `nodes(volume_id, path, kind, size)` where `kind='file'`.
### Contracts
Tool `admin.get_project_disk_usage`. Parameters: `project_id` (string, required, description `Id of the project whose files are listed by size.`), `limit` (number, optional, description `Optional. Number of files listed, a whole number from 1 to 500. Default 100.`). Tool description (verbatim): `List a project's files from the largest to the smallest (platform admin or project owner), with the limit in MB (null when unlimited), the bytes used, the number of files and whether the list was cut. Trashed files are listed too.` Annotations: read only, idempotent, not destructive, closed world.
Errors, after the `ERR_<CODE>: ` prefix: cap invalid `ERR_INVALID_ARGUMENT` `limit must be between 1 and 500`; not admin nor owner `ERR_FORBIDDEN` `'<person>' is not the owner of '<project>'` (also for a missing project); admin, missing project `ERR_PROJECT_NOT_FOUND` `project '<project>' not found`. The cap is checked before the gate.
Rust:
```rust
// storage/meta.rs, inherent on RelationalMetaStore
pub(crate) struct FileUsageRow { pub path: String, pub size: i64, pub original_path: Option<String> }
pub(crate) async fn largest_files(&self, cap: usize, trash_dir: &str) -> Result<(Vec<FileUsageRow>, i64)>;
// quota.rs
pub(crate) struct DiskUsage { pub used_bytes: i64, pub file_count: i64, pub entries: Vec<FileUsageRow> }
pub(crate) async fn disk_usage(client: &VolumeClient, cap: usize, trash_dir: &str) -> Result<DiskUsage>;
// tools/admin.rs
pub(crate) async fn get_project_disk_usage(ctx: &ToolCtx, project_id: &str, limit: Option<f64>) -> Result<Value>;
// mcp/server.rs
pub struct GetProjectDiskUsageArgs { pub project_id: String, #[serde(default)] pub limit: Option<f64> }
pub(crate) async fn admin_get_project_disk_usage(&self, Parameters(a): Parameters<GetProjectDiskUsageArgs>) -> Result<CallToolResult, ErrorData>;
```
In this story `trash_dir` is accepted and `original_path` is always `None`; every row is `Active`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`FileUsageRow`, `largest_files`) |
| `crates/core/src/quota.rs` | modify (`DiskUsage`, `disk_usage`) |
| `crates/core/src/tools/admin.rs` | modify (`reader_project`, `get_project_disk_usage`, test registry entry next to line 333) |
| `crates/core/src/mcp/server.rs` | modify (args struct near line 1391, `#[tool]` near line 2918, count assertions near line 3897) |
| `crates/core/src/tools/disk_usage_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`mod disk_usage_e2e;`) |
| `tool-contract-golden.json` | regenerate, never hand edit |
| `TOOL_CONTRACT.txt` | modify (one entry plus the example line near 674, same layout as `admin.get_project_quota` at line 21) |
| `crates/core/src/tools/contract_golden.rs` | modify (109 to 110 at lines 145, 252, 308; 15 to 16 admin in the message and comment) |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (60 to 61 at line 241, 113 to 114 at line 256, and any other served count that includes admin) |
| `crates/agent/src/mcp.rs` | modify (109 to 110 at line 319 and the comment at 317) |
| `crates/core/src/tools/all.rs` | modify (comments at 73, 116, 136 and any count assertion they accompany) |
### Existing patterns
- Tool shape and gate: `crates/core/src/tools/admin.rs:647-676` (`get_project_quota`) and its `#[tool]` method `crates/core/src/mcp/server.rs:2918-2929`.
- Test harness: `crates/core/src/tools/quota_e2e.rs:60-139` (`World`, `as_`, `seed`, `ok_json`); a second harness for the router: `crates/core/src/tools/diff_files_e2e.rs:709-750`.
- Query style for the store: `list_trash_entries` and `count_trash_entries`, `crates/core/src/storage/meta.rs:1530-1585` (`tx.query(&Query::new(sql).bind(..))`, `row.i64(0)`, `row.text(0)`). Literal example to follow:
```rust
let row = tx.query_opt(&Query::new("SELECT COUNT(*) FROM nodes WHERE volume_id=?1 AND kind='file'").bind(&self.volume_id)).await?;
let count = row.map_or(Ok(0), |r| r.i64(0))?;
```

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec): project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores five files, all `Active`: `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (975 500 in all). Olivia (`olivia@example.com`) owns Atlas, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Every session budget is 262 144 bytes. "The standard answer" means: project `Atlas`, limit 2, bytes used 975 500, 5 files, list complete, entries in the order of the table above, each `Active`. Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0047/E2E-001: The owner reads the breakdown
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002, 003, 006, 008, 010
- **Given** the shared setup
- **When** Olivia reads the breakdown of `Atlas` with no cap
- **Then** she gets the standard answer: five entries in the order `/video/demo.mp4` 600 000,
  `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500
- **And** the five sizes add up to 975 500, the bytes used shown

### SPEC-0047/E2E-002: The admin reads the same breakdown
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-001, 007
- **Given** the shared setup; Ada belongs to no project
- **When** Ada reads the breakdown of `Atlas`
- **Then** she gets the standard answer

### SPEC-0047/E2E-007: A member is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Mia reads the breakdown of `Atlas`
- **Then** she is refused with `'mia@example.com' is not the owner of 'Atlas'`

### SPEC-0047/E2E-008: An outsider is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Otto reads the breakdown of `Atlas`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Atlas'`

### SPEC-0047/E2E-009: An outsider naming a missing project is refused the same way
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup; no project `Ghost` exists
- **When** Otto reads the breakdown of `Ghost`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Ghost'`

### SPEC-0047/E2E-010: The admin naming a missing project
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-007
- **Given** the shared setup; no project `Ghost` exists
- **When** Ada reads the breakdown of `Ghost`
- **Then** she is refused with `project 'Ghost' not found`

### SPEC-0047/E2E-011: The owner of another project is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup, and Bram owns `Borealis`
- **When** Bram reads the breakdown of `Atlas`
- **Then** he is refused with `'bram@example.com' is not the owner of 'Atlas'`

### SPEC-0047/E2E-012: A cap of 0 is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of 0
- **Then** she is refused with `limit must be between 1 and 500` and sees no file

### SPEC-0047/E2E-013: A cap of 501 is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of 501
- **Then** she is refused with `limit must be between 1 and 500` and sees no file

### SPEC-0047/E2E-014: A negative or fractional cap is refused
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of -5, then with a cap of 2.5
- **Then** each time she is refused with `limit must be between 1 and 500` and sees no file

### SPEC-0047/E2E-016: A refusal reveals nothing about the files
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Otto reads the breakdown of `Atlas`
- **Then** the refusal contains none of the paths, none of the sizes and no file count of `Atlas`

### SPEC-0047/E2E-022: An empty project
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, 003, 005
- **Given** `Atlas` stores no file
- **When** Olivia reads the breakdown
- **Then** 0 files, bytes used 0, an empty list reported as complete

### SPEC-0047/E2E-023: The default cap is 100
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads the breakdown with no cap
- **Then** 100 entries are shown, 120 files are counted, bytes used 120 000, the list is reported as cut

### SPEC-0047/E2E-027: Equal sizes are ordered by path
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** `Atlas` stores `/b.txt`, `/a.txt` and `/C.txt`, each 10 bytes
- **When** Olivia reads the breakdown
- **Then** the order is `/C.txt`, `/a.txt`, `/b.txt`

## Constraints
- Files not to touch: `get_project_quota`'s behavior and answer (only its gate moves into `reader_project`); any `fs.*` tool; the schema in `storage/meta.rs:195-260`.
- Dependencies not to add: none.
- Patterns to avoid: ordering ties in SQL; hand editing `tool-contract-golden.json`; a second copy of the owner check.
- Scope boundary: the `In trash` state and original paths (US-0065), the route (US-0066), the admission lock and the docs (US-0067).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (add the tool method first as a stub that answers an internal error, so each test fails on its assertion and not on a missing symbol).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `admin.get_project_quota` answers byte for byte as before (its 97 tests in `quota_e2e.rs` stay green); the frozen contract differs from before by exactly the one new tool.
- [ ] Docs updated: none in this story (the docs are written in US-0067)
- [ ] Self review: the answer shape matches DEC-004 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment, not a WHAT comment.
- [ ] One commit `US-0064: list a project's largest files`, body `Spec: SPEC-0047` and the requirement ids.
