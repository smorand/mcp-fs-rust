# US-0070: name the folder and refuse bad paths

> Spec: SPEC-0049 (sdlc/specs/SPEC-0049-project-folder-usage-breakdown/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0069
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The reader can name the folder to read, with the exact path rules: a trailing slash is accepted once, odd forms are refused as typed, a missing folder and a file given as the folder get their own messages, in the order cap, project, folder.

## Requirements covered
- SPEC-0049/FR-NEW-009: IF the folder asked for does not exist, is a file, or does not start with `/` THEN THE Administration context SHALL refuse and show nothing.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::folder_usage_e2e`: E2E-015 returns an error ending `folder '/nothing' not found`; E2E-027 returns identical answers for `/video/` and `/video`; E2E-038 returns, for `//`, an error ending `folder '//' not found`.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0049): `parse_folder(raw)` implements the path rules (empty or no leading `/` gives `path must start with /`; a single `/` is the root; otherwise exactly one trailing `/` is dropped when the text before it does not end with `/`; every segment of the rest must be non empty and not `.` or `..`, else `folder '<path>' not found`; case sensitive); the check runs after the gate; the cap parser `parse_disk_usage_cap` is reused.
- DEC-008 (SPEC-0049): a missing folder is `ERR_NOT_FOUND` `folder '<path>' not found`; a file given as the folder and a bad path text are `ERR_INVALID_ARGUMENT` (`'<path>' is not a folder`, `path must start with /`); `<path>` is the text as the reader gave it.
- DEC-012 (SPEC-0049): acceptance tests are functional e2e through the real tool method or the data plane router.
### Data model
None.
### Contracts
`parse_folder(raw: &str) -> Result<String>` in `tools/admin.rs`, called by `get_project_folder_usage` after `reader_project` and before `quota::folder_usage`. `None` folder is the root. `FolderRead::Missing` becomes `ToolError::not_found(format!("folder '{raw}' not found"))`; `FolderRead::IsFile` becomes `ToolError::invalid_argument(format!("'{raw}' is not a folder"))`; the first parse rule gives `ToolError::invalid_argument("path must start with /")`; a segment `.`, `..` or empty gives the `not found` message. `<raw>` is the text as the reader gave it.
Table of expected results for `parse_folder`: `` (empty) refused `path must start with /`; `video` refused `path must start with /`; `/` is `/`; `/video` is `/video`; `/video/` is `/video`; `/video//` refused `folder '/video//' not found`; `//` refused `folder '//' not found`; `///` refused `folder '///' not found`; `//video` refused `folder '//video' not found`; `/video/../docs` refused `folder '/video/../docs' not found`; `/Video` is accepted by the parser and then not found by the lookup.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/admin.rs` | modify (`parse_folder`, error mapping) |
| `crates/core/src/tools/folder_usage_e2e.rs` | modify (add tests) |
### Existing patterns
- Tool shape, gate and cap: `get_project_disk_usage`, `reader_project`, `parse_disk_usage_cap`, `crates/core/src/tools/admin.rs:672-760`; its `#[tool]` method `admin_get_project_disk_usage` in `crates/core/src/mcp/server.rs` (SPEC-0047, US-0064).
- Engine and lock: `RelationalMetaStore::largest_files` (`crates/core/src/storage/meta.rs:596-690`) and `quota::disk_usage` (`crates/core/src/quota.rs:179-195`).
- Test harness: `World`, `seed`, `read`, `ok_json`, `err_text`, `rest_usage` in `crates/core/src/tools/disk_usage_e2e.rs` (copy what you need into `folder_usage_e2e.rs`: modules are private).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files, all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000, `/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise. The answer fields are named in the design: `project_id`, `max_mb`, `used_bytes`, `folder` (`path`, `size`, `file_count`), `item_count`, `truncated`, `items` (`path`, `kind`, `size`, `file_count` for a folder only). Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0049/E2E-015: A folder that does not exist
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/nothing` of `Atlas`
- **Then** she is refused with `folder '/nothing' not found`

### SPEC-0049/E2E-016: A file given as the folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/readme.md` of `Atlas`
- **Then** she is refused with `'/readme.md' is not a folder`

### SPEC-0049/E2E-017: A folder path without a leading slash
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `video` of `Atlas`
- **Then** she is refused with `path must start with /`

### SPEC-0049/E2E-027: A trailing slash means the same folder
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** Medium
- **Given** the shared setup
- **When** Olivia reads the folder `/video/` and the folder `/video`
- **Then** the two answers are identical, including the folder shown as `/video`

### SPEC-0049/E2E-038: Odd folder names are refused as typed
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_folder_usage` over real storage / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads, one after the other, the folder named by the empty text, `/video/../docs`, `//video`, `/video//`, `//`, `/Video` and `/`
- **Then** the empty name is refused with `path must start with /`; `/video/../docs` with `folder '/video/../docs' not found`; `//video` with `folder '//video' not found`; `/video//` with `folder '/video//' not found`; `//` with `folder '//' not found`; `/Video` with `folder '/Video' not found`; and `/` shows the root folder exactly as with no folder (the answer of E2E-001)

## Constraints
- Files not to touch: the engine (`meta.rs`, `quota.rs`), the golden contract.
- Dependencies not to add: none.
- Patterns to avoid: normalizing the path with `PosixPath` (it resolves `..` and hides the `not found` rule).
- Scope boundary: the route (US-0071).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (E2E-027 may pass before the parser exists for a path that the lookup accepts; it gets a mutation check).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: US-0069 tests stay green; `admin.get_project_disk_usage` untouched.
- [ ] Docs updated: none in this story
- [ ] Self review: the answer shape matches DEC-003 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0070: name the folder and refuse bad paths`, body `Spec: SPEC-0049` and the requirement ids.
