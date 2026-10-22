# SPEC-0049: Project folder usage breakdown, design

> Spec: sdlc/specs/SPEC-0049-project-folder-usage-breakdown/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-22
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, MCP, REST, persistence | existing Rust workspace: rmcp tools, axum REST, `RelationalDb` meta store | per `Cargo.toml` | DEC-001 |

No dependency added or removed, so no context7 lookup. Skill loaded: `rust` (`make check` is the gate,
`ToolError` in the library, typed `Parameters<T>`, functional e2e on the real entry points, one
implementation shared by tool and route). Project state: existing (`git ls-files crates | wc -l` far above
scaffolding; 114 tools served with git and search). The decisions of SPEC-0047 bind this design: the gate
`reader_project`, the cap parser, the admission lock around a read, the ties in code, no index.

## 2. Approaches

### Chosen: A, a new tool and route that bucket the file rows under the folder by first path component

`admin.get_project_folder_usage(project_id, folder?, limit?)` and `GET
/api/admin/projects/{project_id}/folder-usage?folder=&limit=` call one function in `tools/admin.rs`. It
parses the cap, passes the gate, then reads under the project's admission lock: the usage counter, then in
one transaction every file row below the folder (`path`, `size`) and the direct folder rows (by `parent`).
Rust buckets the files by the first component after the folder, adds the empty direct folders, sorts and
cuts.

- Pros: no schema change, no index, no dependency; one pass, O(n) time and O(n) transient memory for n files
  below the folder, inside NFR 7.1 at 100 000 files; works unchanged on SQLite, PostgreSQL, SQL Server; the
  match is by whole path component, so `/doc` never absorbs `/docs`.
- Cons and risks: all file rows below the folder are read for each call (the root read touches every file);
  for 100 000 files that is a few MB of strings, accepted for an admin read.
- Affected scope: `storage/meta.rs`, `quota.rs`, `tools/admin.rs`, `mcp/server.rs`, `api/dataplane.rs`,
  `api/openapi.rs`, the frozen tool contract and its counts.

### Rejected: B, an optional folder parameter on `admin.get_project_disk_usage`

Rejected (DEC-002): that answer is a pinned shape (SPEC-0047 DEC-004) and E2E-022 requires the file list to
stay as it is; two answer shapes behind one tool would blur the frozen contract.

### Rejected: C, grouping in SQL

`GROUP BY` the first path component needs a per engine substring and position function (SQLite `instr`,
PostgreSQL `strpos`, SQL Server `CHARINDEX`). Rejected (DEC-004): three dialect branches for a read that the
Rust pass already serves.

### Rejected: D, a maintained per folder total

A rollup table kept by the node transactions. Rejected (DEC-013): a migration on three engines and an extra
write per file operation for a rare admin read.

## 3. Architecture

```
 MCP: admin.get_project_folder_usage       REST: GET /api/admin/projects/{id}/folder-usage?folder=&limit=
                \                                   /
                 tools::admin::get_project_folder_usage(ctx, project_id, folder, limit)
                   1. parse the cap                       (FR-NEW-005, SPEC-0047 DEC-006)
                   2. reader_project gate                 (FR-NEW-008, SPEC-0047 DEC-007)
                   3. parse_folder(folder)                (FR-NEW-009, DEC-007)
                   4. quota::folder_usage(client, folder, cap)
                        a. lock client.admission          (FR-NEW-010, DEC-006)
                        b. used = meta.used_bytes()
                        c. meta.get(folder): missing / file / folder
                        d. one tx on RelationalMetaStore::folder_items:
                             file rows under the folder, direct folder rows
                        e. bucket, sort, cut                (FR-NEW-002, 004, 006, DEC-004, DEC-005)
                   5. shape the answer                    (DEC-003)
```

Bounded contexts: Administration owns the gate, the path rules and the shape; Storage owns the rows and the
lock. Existing structure: gate and file list at `crates/core/src/tools/admin.rs:672-760`; lock and counter
at `crates/core/src/quota.rs:164-195`; concrete store through `VolumeClient.trash`
(`crates/core/src/storage/volume.rs:22`); rows `nodes(volume_id, path, parent, name, kind, size)` and the
`parent` index at `crates/core/src/storage/meta.rs:195-260`.

## 4. Data model

No schema change. Rows read: files `nodes WHERE kind='file' AND path LIKE '<folder>/%'`, folders
`nodes WHERE kind='dir' AND parent='<folder>'`. The root folder `/` is virtual (always a folder, no row
needed). The trash folder `/<trash_dir>` is an ordinary `dir` row created by the first soft delete
(`crates/core/src/core/fs_ops.rs:1272-1274`) and never removed by the purge (no directory removal in
`crates/core/src/purge.rs` outside test wrappers), so FR-NEW-003's "stays once created" needs no code.

## 5. Contracts and interfaces

### 5.1 MCP tool `admin.get_project_folder_usage`

Parameters: `project_id` (string, required: `Id of the project whose folder is read.`), `folder` (string,
optional: `Optional. Absolute path of the folder read, default /.`), `limit` (number, optional:
`Optional. Number of items listed, a whole number from 1 to 500. Default 100.`). Description (frozen):
`List one folder of a project (platform admin or project owner): its size and file count, and its direct folders and files from the largest to the smallest, a folder counting everything below it. Trashed files are counted.` Read only, idempotent, closed world.

Answer (DEC-003):

```json
{
  "project_id": "Atlas", "max_mb": 2, "used_bytes": 1225500,
  "folder": {"path": "/", "size": 1225500, "file_count": 7},
  "item_count": 4, "truncated": false,
  "items": [
    {"path": "/video", "kind": "Folder", "size": 800000, "file_count": 2},
    {"path": "/readme.md", "kind": "File", "size": 1500}
  ]
}
```

`max_mb` is `null` when unlimited; a `File` item has no `file_count` key; `item_count` is the number of items
listed; `truncated` is true when the folder has more direct items than listed. The folder is shown without a
trailing `/` (root as `/`).

### 5.2 Errors (after the `ERR_<CODE>: ` prefix), checked in this order

| Order | Case | Code | Message |
|---|---|---|---|
| 1 | cap not a whole number from 1 to 500 | `ERR_INVALID_ARGUMENT` | `limit must be between 1 and 500` |
| 2 | caller neither admin nor owner | `ERR_FORBIDDEN` | `'<person>' is not the owner of '<project>'` |
| 2 | admin, missing project | `ERR_PROJECT_NOT_FOUND` | `project '<project>' not found` |
| 3 | empty folder text, or not starting with `/` | `ERR_INVALID_ARGUMENT` | `path must start with /` |
| 3 | segment `.`, `..` or empty, or folder row absent | `ERR_NOT_FOUND` | `folder '<path>' not found` |
| 3 | the folder is a file | `ERR_INVALID_ARGUMENT` | `'<path>' is not a folder` |

`<path>` is the text as the reader gave it. `parse_folder` (DEC-007): empty or no leading `/` gives the first
message; the single text `/` is the root; otherwise exactly one trailing `/` is dropped when the text before
it does not end with `/`; then every segment of the rest must be non empty and not `.` or `..`, else the
`not found` message. Case sensitive.

### 5.3 REST route

`GET /api/admin/projects/{project_id}/folder-usage?folder=<path>&limit=<n>`, bearer only, same body, error
body (`{"error": <code>, "detail": "<code>: <message>"}`) and status as the tool
(`crates/core/src/api/dataplane.rs:251`). `folder=` present but empty is the empty folder text (refused);
absent means the root. An unparsable `limit` goes on as NaN (SPEC-0047 DEC-009). Added to the route table
at `dataplane.rs:98` and to `api/openapi.rs` next to the disk usage `Op`.

### 5.4 Internal signatures

```rust
// storage/meta.rs, inherent on RelationalMetaStore
pub(crate) struct FolderItemRow { pub path: String, pub is_folder: bool, pub size: i64, pub file_count: i64 }
pub(crate) struct FolderItems { pub total_size: i64, pub total_files: i64, pub item_total: i64, pub items: Vec<FolderItemRow> }
pub(crate) async fn folder_items(&self, folder: &str, cap: usize) -> Result<FolderItems>;
// quota.rs
pub(crate) enum FolderRead { Missing, IsFile, Found { used_bytes: i64, items: FolderItems } }
pub(crate) async fn folder_usage(client: &VolumeClient, folder: &str, cap: usize) -> Result<FolderRead>;
// tools/admin.rs
fn parse_folder(raw: &str) -> Result<String>;
pub(crate) async fn get_project_folder_usage(ctx: &ToolCtx, project_id: &str, folder: Option<&str>, limit: Option<f64>) -> Result<Value>;
```

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | modify: `FolderItemRow`, `FolderItems`, `folder_items` | FR-NEW-002, 003, 004, 006, 007 | US-0069 |
| `crates/core/src/quota.rs` | modify: `FolderRead`, `folder_usage` | FR-NEW-001, 010 | US-0069, US-0072 |
| `crates/core/src/tools/admin.rs` | modify: `parse_folder`, `get_project_folder_usage`, test registry entry | FR-NEW-001, 005, 008, 009 | US-0069, US-0070 |
| `crates/core/src/mcp/server.rs` | modify: args struct, `#[tool]`, count asserts | FR-NEW-001 | US-0069 |
| `crates/core/src/api/dataplane.rs`, `crates/core/src/api/openapi.rs` | modify: route, `Op` | FR-NEW-012 | US-0071 |
| `crates/core/src/tools/folder_usage_e2e.rs`, `crates/core/src/tools/mod.rs` | create / modify | all E2E | US-0069 to US-0073 |
| `tool-contract-golden.json`, `TOOL_CONTRACT.txt` | regenerate / edit | contract change | US-0069 |
| `crates/core/src/tools/contract_golden.rs`, `tool_surface_e2e.rs`, `all.rs`, `crates/agent/src/mcp.rs`, `AGENTS.md`, `.agent_docs/tools.md` | modify: tool counts (admin 16 to 17, frozen 110 to 111, default 61 to 62, git 110 to 111, git and search 114 to 115, every flag 136 to 137) | contract change | US-0069 |
| `README.md`, `AGENTS.md`, `.agent_docs/api.md`, `.agent_docs/architecture.md`, `.agent_docs/tools.md` | modify: the tool, the route | docs | US-0073 |

Affected existing tests: keep all; the count assertions above are modified (one number each). The disk usage
tests (`disk_usage_e2e.rs`) stay unchanged and green.

## 7. Compatibility and risks

- Consumers: additive (a new tool and route). The file list is untouched (E2E-022).
- The frozen tool contract grows by one tool (DEC-011).
- Risk: the root read loads every file row of the project. Mitigation: O(n) once, under the admission lock
  that delays writers of that project for its duration; at 100 000 files about 100k short rows. Revisit with
  SQL grouping only if E2E-039 fails.
- Ordering between stories: US-0069 lands the read with the folder argument taken as a canonical absolute path
  (no check beyond what the lookup does); US-0070 adds the path rules. Until then an odd path answers as the
  lookup does (no release between them).
- Rollback: revert the commits; nothing is persisted.

## 9. Test strategy

Every acceptance test is a functional e2e through the real `McpServer` method (as `disk_usage_e2e.rs` does)
or the real data plane router (`oneshot`), over real storage. Location: `crates/core/src/tools/folder_usage_e2e.rs`
(its own `World`, copied from `disk_usage_e2e.rs`, the modules are private).

| Test | Level | Driver | Story |
|---|---|---|---|
| E2E-001, 002, 003, 004, 007, 008, 009, 010, 011, 012, 013, 014, 018 | functional e2e | `McpServer::admin_get_project_folder_usage` | US-0069 |
| E2E-015, 016, 017, 027, 038 | functional e2e | tool | US-0070 |
| E2E-006, 019, 020 | functional e2e | tool and router `oneshot` | US-0071 |
| E2E-005, 021, 022, 023, 024, 025, 033, 034, 037 | functional e2e | tool, `fs_delete`, `fs_trash_restore`, the file list tool, the admission lock | US-0072 |
| E2E-026, 028, 029, 030, 031, 032, 035, 036, 039 | functional e2e | tool | US-0073 |

Test data: generated through the volume client. E2E-039 seeds 100 000 files through the store's `put_file` with
one shared content blob so the seed does not write 100 000 blobs. E2E-037 holds `client.admission` and spawns
the read (no sleeps, as SPEC-0047 E2E-032). Tests green on the first run after their story's code (the rollup
makes them pass) carry no RED evidence; each story requires a mutation check instead.

## 10. Decisions log

- **DEC-001:** No stack change. **Obliged by:** n/a (no code impact): nothing new is chosen. **Rationale:**
  conventions fix the stack; no library needed. **Alternatives considered:** none viable. **Source:** existing
  code `Cargo.toml`, `AGENTS.md`. **Evidence:** n/a
- **DEC-002:** A new tool `admin.get_project_folder_usage` with its own route. **Obliged by:**
  SPEC-0049/FR-NEW-001, FR-NEW-010 (E2E-022 pins the file list). **Rationale:** the file list answer is pinned
  (SPEC-0047 DEC-004). **Alternatives considered:** a `folder` parameter on the file list tool; an `fs.*` tool
  (member audience, wrong gate). **Source:** SPEC-0047 design. **Evidence:** `crates/core/src/tools/admin.rs:720-760`
- **DEC-003:** The answer is `{project_id, max_mb, used_bytes, folder{path,size,file_count}, item_count,
  truncated, items[{path,kind,size,file_count?}]}`, `kind` exactly `Folder` or `File`, no `file_count` on a
  file. **Obliged by:** SPEC-0049/FR-NEW-001, FR-NEW-003, FR-NEW-006. **Rationale:** mirrors SPEC-0047's first
  keys (`project_id`, `max_mb`, `used_bytes`, `truncated`) as 14.3 asks. **Alternatives considered:** reuse
  `entries` and `state` (an item is not a file with a state). **Source:** spec 14.3. **Evidence:**
  `crates/core/src/tools/admin.rs:735-760`
- **DEC-004:** One query reads the file rows under the folder (`path LIKE '<folder>/%'`, built with
  `descendant_pattern`, never by hand), a second reads the direct folder rows (`parent = '<folder>'`); Rust
  buckets the files by the first component after the folder (a name with a further `/` is a folder item, else
  a file item) and adds direct folders that hold no file with 0 bytes and 0 files. **Obliged by:**
  SPEC-0049/FR-NEW-002, FR-NEW-007, NFR 7.1. **Rationale:** whole component match (`/doc` vs `/docs`), one
  code path for every engine. **Alternatives considered:** SQL `GROUP BY` on a substring (three dialect
  branches); one aggregate query per child (N queries). **Source:** design. **Evidence:**
  `crates/core/src/storage/meta.rs:554` (`descendant_pattern`)
- **DEC-005:** Items are sorted by size descending then path ascending as Rust strings, then truncated to the
  cap; 0 byte items fall last by the same rule. **Obliged by:** SPEC-0049/FR-NEW-004. **Rationale:** engine
  independent code point order (SPEC-0047 DEC-003). **Alternatives considered:** `ORDER BY` in SQL.
  **Source:** SPEC-0047 DEC-003. **Evidence:** `crates/core/src/storage/meta.rs:660`
- **DEC-006:** `quota::folder_usage` holds `client.admission` while it reads the counter with
  `client.meta.used_bytes()`, looks the folder up with `client.meta.get`, and runs `folder_items` in one
  transaction. **Obliged by:** SPEC-0049/FR-NEW-010, FR-NEW-007. **Rationale:** writers hold the same lock for
  the whole operation. **Alternatives considered:** no lock. **Source:** SPEC-0047 DEC-005. **Evidence:**
  `crates/core/src/quota.rs:179-195`
- **DEC-007:** `parse_folder` implements the path rules in the error table (5.2) in one function; the folder
  check runs after the gate; the existing cap parser `parse_disk_usage_cap` is reused as is. **Obliged by:**
  SPEC-0049/FR-NEW-005, FR-NEW-008, FR-NEW-009. **Rationale:** the spec fixes the order cap, project, folder.
  **Alternatives considered:** reuse `PosixPath` normalization (it would silently resolve `..`, against the
  `not found` rule). **Source:** spec. **Evidence:** `crates/core/src/tools/admin.rs:708-718`
- **DEC-008:** A missing folder is `ERR_NOT_FOUND`, a file given as the folder and a bad path text are
  `ERR_INVALID_ARGUMENT`. **Obliged by:** SPEC-0049/FR-NEW-009. **Rationale:** the codes the REST plane maps
  to 404 and 400 (`crates/core/src/errors.rs:125-128`). **Alternatives considered:** one code for all three.
  **Source:** existing code. **Evidence:** `crates/core/src/errors.rs:77-89`
- **DEC-009:** REST route `GET /api/admin/projects/{project_id}/folder-usage?folder=&limit=`; the query is read
  raw so `folder=` (present, empty) is refused, absent is the root. **Obliged by:** SPEC-0049/FR-NEW-012,
  FR-NEW-009. **Rationale:** the `Q::opt` helper drops empty values (`dataplane.rs:276`), which would turn the
  empty folder into the root. **Alternatives considered:** `Q::opt` (wrong for E2E-038). **Source:** SPEC-0047
  DEC-009. **Evidence:** `crates/core/src/api/dataplane.rs:269-278`
- **DEC-010:** No special case for the trash folder: it is a stored directory that appears like any other, and
  it stays after restore and purge because nothing removes it. **Obliged by:** SPEC-0049/FR-NEW-003. **Rationale:**
  the existing lifecycle already satisfies the spec. **Alternatives considered:** hide an empty trash folder
  (contradicts E2E-024, 025). **Source:** existing code. **Evidence:** `crates/core/src/core/fs_ops.rs:1272-1274`
- **DEC-011:** The frozen contract is regenerated with
  `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the tool
  counts move by one. **Obliged by:** SPEC-0049/FR-NEW-001. **Rationale:** project rule. **Alternatives
  considered:** none. **Source:** `AGENTS.md`. **Evidence:** `crates/core/src/tools/contract_golden.rs:143-146`
- **DEC-012:** All acceptance tests are functional e2e. **Obliged by:** SPEC-0049/all E2E. **Rationale:** skill
  `rust` and the SPEC-0047 pattern. **Alternatives considered:** unit tests of `folder_items` (extra only).
  **Source:** skill. **Evidence:** `crates/core/src/tools/disk_usage_e2e.rs`
- **DEC-013:** No index and no schema change. **Obliged by:** SPEC-0049/NFR 7.1. **Rationale:** one pass over
  at most the project's file rows meets 5 seconds at 100 000 files. **Alternatives considered:** a rollup
  table; an index on `(volume_id, parent)` already exists. **Source:** SPEC-0047 DEC-010. **Evidence:**
  `crates/core/src/storage/meta.rs:195-260`

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0069 | read a project's root folder by size | P1 | FR-NEW-001, 002, 003, 004, 005, 006, 008, 011 | main | 1 | no |
| US-0070 | name the folder and refuse bad paths | P1 | FR-NEW-009 | US-0069 | 2 | no |
| US-0071 | the same folder read on the web interface | P2 | FR-NEW-012 | US-0070 | 2 | no |
| US-0072 | trash, population, read only and one moment | P2 | FR-NEW-003, 007, 010 | US-0069 | 2 | no |
| US-0073 | edge cases, scale, docs, spec close | P2 | FR-NEW-001, 002, 004, 005, 006, 011 | US-0071, US-0072 | 1 | yes |
