# SPEC-0047: Project disk usage breakdown, design

> Spec: sdlc/specs/SPEC-0047-project-disk-usage-breakdown/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-22
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, MCP, REST, persistence | existing Rust workspace: rmcp tools, axum REST, `RelationalDb` meta store | per `Cargo.toml` | DEC-001 |

No dependency added or removed, so no context7 lookup. Skill loaded: `rust` (`make check` gate,
`ToolError` in the library, typed `Parameters<T>`, functional e2e on the real entry points, one
implementation shared by the tool and the REST route). Project state: existing (`git ls-files
crates | wc -l` is far above scaffolding; 113 tools registered).

## 2. Approaches

### Chosen: A, a new tool and REST route reading a bounded top N from the nodes table

`admin.get_project_disk_usage(project_id, limit)` and `GET
/api/admin/projects/{project_id}/disk-usage` call one function in `tools/admin.rs`. It reuses the
owner or admin gate of `get_project_quota` (extracted so both share it), takes the volume's admission
lock, reads the usage counter, then reads the file count and the top N files in one relational
transaction. Ties at the cutoff are resolved in Rust in character code order.

- Pros: no schema change, no new index, no new dependency; O(n log k) for n files and cap k, inside
  NFR 7.1 for 100 000 files; the usage and the list come from one moment (DEC-005).
- Cons and risks: a full scan of a volume's file rows per read (acceptable for an admin read, not a
  hot path); the tie group at the cutoff is loaded whole (bounded by the file count).
- Affected scope: `storage/meta.rs`, `quota.rs`, `tools/admin.rs`, `mcp/server.rs`,
  `api/dataplane.rs`, `api/openapi.rs`, the frozen tool contract and its counts.

### Rejected: B, extend `admin.get_project_quota` with an optional list

Rejected (DEC-002): the quota read has a frozen three value shape that SPEC-0020 FR-NEW-009 states and
its REST route returns byte for byte; adding fields changes a contract that tests and clients pin.

### Rejected: C, a maintained size index or per file ranking table

A table or index on `(volume_id, kind, size)` kept by the node transactions. Rejected (DEC-010):
needs a migration on three engines and an extra write per file operation to serve a rare admin read.

### Rejected: D, `ORDER BY size DESC, path` entirely in SQL

Rejected (DEC-003): the text order of `path` is engine specific (PostgreSQL follows the database
collation, SQL Server compares UTF-16 units), so FR-NEW-002's character code order would hold on
SQLite only.

## 3. Architecture

```
 MCP: admin.get_project_disk_usage      REST: GET /api/admin/projects/{id}/disk-usage?limit=
                \                              /
                 tools::admin::get_project_disk_usage(ctx, project_id, limit)
                   1. parse the cap            (FR-NEW-004, DEC-006)
                   2. owner or admin gate      (FR-NEW-007, DEC-007)
                   3. backfill trash rows      (tools::trash::backfill_trash_entries)
                   4. quota::disk_usage(client, cap, trash_dir)
                        a. lock client.admission          (FR-NEW-010, DEC-005)
                        b. used = meta.used_bytes()       (FR-NEW-001, FR-NEW-006)
                        c. one tx on RelationalMetaStore::largest_files(cap):
                             COUNT(*) of files, cutoff size, rows >= cutoff,
                             Rust sort + truncate, trash original paths
                   5. shape the answer         (DEC-004)
```

Bounded contexts: Administration owns the gate and the shape; Storage owns the rows and the lock.
The existing structure: gate and shape at `crates/core/src/tools/admin.rs:647-676`; lock and counter
at `crates/core/src/quota.rs:164-167`; concrete store reached through `VolumeClient.trash`
(`crates/core/src/storage/volume.rs:22`); trash rows at `crates/core/src/storage/meta.rs:216-231`,
trash lookup `get_trash_entry` at `meta.rs:658`.

## 4. Data model

No schema change. Rows read: `nodes(volume_id, path, kind, size)` where `kind='file'`, and
`trash_entries(volume_id, trash_path, original_path)`. A file is `In trash` when its path lies under
the configured trash directory (`SafetyConfig.trash_dir`, default `.bin`); its trash path is flat, so
its top level trash entry is the first two path components (`/<trash_dir>/<stamp>__<flat>`), and its
original path is that entry's `original_path` plus the remainder of the path.

## 5. Contracts and interfaces

### 5.1 MCP tool `admin.get_project_disk_usage`

Parameters: `project_id` (string, required: id of the project), `limit` (number, optional: files to
list, whole number from 1 to 500, default 100). Read only, idempotent, closed world.

Answer (DEC-004):

```json
{
  "project_id": "Atlas",
  "max_mb": 2,
  "used_bytes": 975500,
  "file_count": 5,
  "truncated": false,
  "entries": [
    {"path": "/video/demo.mp4", "size": 600000, "state": "Active", "original_path": null},
    {"path": "/.bin/20261022T101500__docs_notes.txt", "size": 4000, "state": "In trash",
     "original_path": "/docs/notes.txt"}
  ]
}
```

`max_mb` is `null` when unlimited. `original_path` is `null` for `Active` entries.

### 5.2 Errors (exact text, after the `ERR_<CODE>: ` prefix every tool error carries)

| Case | Code | Message |
|---|---|---|
| cap not a whole number from 1 to 500 | `ERR_INVALID_ARGUMENT` | `limit must be between 1 and 500` |
| caller neither admin nor owner (missing project included) | `ERR_FORBIDDEN` | `'<person>' is not the owner of '<project>'` |
| admin names a missing project | `ERR_PROJECT_NOT_FOUND` | `project '<project>' not found` |

The cap is checked first, then the gate: a refused caller learns nothing either way, because the cap
message names no project data. Tests assert the message with `ends_with`, as the quota tests do.

### 5.3 REST route

`GET /api/admin/projects/{project_id}/disk-usage?limit=<n>`, bearer only, same body and same error
body (`{"error": <code>, "detail": <code>: <message>}`) and status as the tool
(`crates/core/src/api/dataplane.rs:251`). A missing `limit` is the default; an unparsable `limit` is
refused with the cap message. Listed in `api/openapi.rs` next to the quota `Op` (line 1178) and in the
`ADMIN_SUBS`-style route table at `dataplane.rs:98` (`("GET", "disk-usage")`).

### 5.4 Internal signatures

```rust
// storage/meta.rs, inherent on RelationalMetaStore
pub(crate) struct FileUsageRow { pub path: String, pub size: i64, pub original_path: Option<String> }
pub(crate) async fn largest_files(&self, cap: usize, trash_dir: &str)
    -> Result<(Vec<FileUsageRow>, i64)>;           // rows already ordered, total file count

// quota.rs
pub(crate) struct DiskUsage { pub used_bytes: i64, pub file_count: i64, pub entries: Vec<FileUsageRow> }
pub(crate) async fn disk_usage(client: &VolumeClient, cap: usize, trash_dir: &str) -> Result<DiskUsage>;

// tools/admin.rs
pub(crate) async fn get_project_disk_usage(ctx: &ToolCtx, project_id: &str, limit: Option<f64>)
    -> Result<Value>;
```

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | modify: `FileUsageRow`, `largest_files` | FR-NEW-002, 003, 004, 005, 006 | US-0064, US-0065 |
| `crates/core/src/quota.rs` | modify: `DiskUsage`, `disk_usage` | FR-NEW-001, 010 | US-0064, US-0067 |
| `crates/core/src/tools/admin.rs` | modify: gate extracted, `get_project_disk_usage`, test registry entry | FR-NEW-001, 004, 007, 008 | US-0064 |
| `crates/core/src/mcp/server.rs` | modify: `GetProjectDiskUsageArgs`, `#[tool]`, count asserts | FR-NEW-001 | US-0064 |
| `crates/core/src/api/dataplane.rs` | modify: route, handler | FR-NEW-011 | US-0066 |
| `crates/core/src/api/openapi.rs` | modify: one `Op` | FR-NEW-011 | US-0066 |
| `crates/core/src/tools/disk_usage_e2e.rs` | create | all E2E | US-0064 to US-0067 |
| `crates/core/src/tools/mod.rs` | modify: `mod disk_usage_e2e;` | tests | US-0064 |
| `tool-contract-golden.json`, `TOOL_CONTRACT.txt` | regenerate / edit | contract change | US-0064 |
| `crates/core/src/tools/contract_golden.rs`, `crates/core/src/tools/tool_surface_e2e.rs`, `crates/agent/src/mcp.rs`, `crates/core/src/tools/all.rs` | modify: tool counts and comments | contract change | US-0064 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/api.md`, `.agent_docs/architecture.md` | modify: counts, the tool, the route | docs | US-0067 |

Affected existing tests: keep all; the count assertions listed above are modified (one number each)
because a tool is added (60 to 61 default, 109 to 110 frozen contract, 113 to 114 with git and search).
No existing behavior test changes. The documented figure "109" in `AGENTS.md` and `README.md` is
already stale against the code (113 served); US-0067 corrects the docs to the real counts.

## 7. Compatibility and risks

- Consumers: additive. A new tool and a new route; no existing answer changes.
- The frozen tool contract grows by one tool (DEC-011); an AI client with a cached tool list sees the
  new tool at its next listing.
- Risk: a full scan per read on a very large volume. Mitigation: the read holds the admission lock for
  its duration, which delays writers of that project; with 100 000 files the scan is milliseconds on
  SQLite. Revisit with an index (rejected approach C) if measured otherwise.
- Risk: the usage counter and the file rows disagree (counter drift). The read reports both as they
  are; E2E-006 and E2E-031 assert equality, and the quota tests already compare the counter to an
  independent sum.
- Ordering constraint between stories: US-0064 lands the tool with every file listed as `Active`;
  US-0065 distinguishes the trash. Until US-0065 lands, a trashed file shows as `Active` (no release
  between the two).
- Rollback: revert the commits; nothing is persisted.

## 9. Test strategy

All acceptance tests are functional e2e: they drive the real `McpServer` tool method (as
`quota_e2e.rs` does) or the real data plane router (as `diff_files_e2e.rs:709` does) over real
storage (SQLite metadata, local blobs). Nothing is mocked. Location: `crates/core/src/tools/disk_usage_e2e.rs`.

| Test | Level | Driver | Story |
|---|---|---|---|
| E2E-001, 002, 007 to 014, 016, 022, 023, 027 | functional e2e | `McpServer::admin_get_project_disk_usage` | US-0064 |
| E2E-003, 005, 006, 019, 020, 029, 031 | functional e2e | tool, plus `fs_delete`, `fs_trash_restore`, purge | US-0065 |
| E2E-004, 015, 034, 035 | functional e2e | tool and data plane router `oneshot` | US-0066 |
| E2E-017, 018, 021, 024, 025, 026, 028, 030, 032, 033 | functional e2e | tool; E2E-032 holds the admission lock | US-0067 |

Test data: generated (files written through the volume client, noise bytes). E2E-031 needs version
history, a zip export and two document artifacts: written through the existing helpers of
`quota_e2e.rs` (`put_text`, `list_images` setup) and `tools::export`. E2E-032 is made deterministic
without timing: the test holds `client.admission`, starts the read, asserts it has not completed,
performs the write and releases the lock, then asserts the answer is coherent (DEC-005). Tests that
are green as soon as US-0064 lands (E2E-017, 018, 021, 024 to 026, 028, 030, 033) carry no RED
evidence; US-0067 requires a mutation check instead (break the behavior, see the test fail).

## 10. Decisions log

- **DEC-001:** No stack change: Rust workspace, rmcp tool, axum route, existing relational layer.
  **Obliged by:** n/a (no code impact): choice of nothing new. **Rationale:** the project conventions fix
  the stack and the capability needs no new library. **Alternatives considered:** none viable.
  **Source:** existing code `Cargo.toml`, `AGENTS.md`. **Evidence:** n/a
- **DEC-002:** A new tool `admin.get_project_disk_usage` with its own route, not an extension of
  `admin.get_project_quota`. **Obliged by:** SPEC-0047/FR-NEW-001, FR-NEW-011. **Rationale:** the quota
  answer is a pinned three value shape (SPEC-0020 FR-NEW-009); the name is the one SPEC-0010 chose.
  **Alternatives considered:** extend the quota tool; an `fs.*` tool (needs `mount_id` and member
  access, the wrong audience). **Source:** SPEC-0010 design finding 3. **Evidence:**
  `crates/core/src/tools/admin.rs:647-676`
- **DEC-003:** Top N by size in SQL, ties at the cutoff resolved in Rust: query the cutoff size with
  `ORDER BY size DESC LIMIT 1 OFFSET <cap-1>`, fetch every file with `size >=` cutoff (all files when
  there are fewer than cap), sort by size descending then path in `str` order (UTF-8 byte order equals
  code point order), truncate to cap. **Obliged by:** SPEC-0047/FR-NEW-002, FR-NEW-004, FR-NEW-005,
  NFR 7.1. **Rationale:** engine independent order (approach D fails it) and O(n log k) selection.
  **Alternatives considered:** `ORDER BY size DESC, path` in SQL; load every file and sort in Rust (O(n)
  memory). **Source:** design. **Evidence:** `crates/core/src/storage/rel/dialect.rs` has no collation hook
- **DEC-004:** The answer is `{project_id, max_mb, used_bytes, file_count, truncated, entries[]}`, each
  entry `{path, size, state, original_path}` with `state` exactly `Active` or `In trash` and
  `original_path` `null` for `Active`; `truncated` is `file_count > entries.len()`. **Obliged by:**
  SPEC-0047/FR-NEW-001, FR-NEW-003, FR-NEW-005. **Rationale:** mirrors the quota answer keys for the
  first three values; a uniform entry shape keeps the schema simple. **Alternatives considered:**
  omit `original_path` for active entries; a separate trash list. **Source:** design. **Evidence:**
  `crates/core/src/tools/admin.rs:668-673`
- **DEC-005:** One moment: `disk_usage` takes `client.admission` for the whole read, reads the counter
  with `meta.used_bytes()` (not `quota::used_bytes`, which would lock again), then does the count and
  the list in one relational transaction. **Obliged by:** SPEC-0047/FR-NEW-006, FR-NEW-010.
  **Rationale:** writers hold the same lock across their writes (`quota.rs:14-18`), so a read under it
  never sees half an operation; the transaction covers engines whose statements see different snapshots.
  **Alternatives considered:** derive `used_bytes` from `SUM(size)` in the same transaction (loses
  the byte for byte match with the quota read); no lock (a half applied multi file write is visible).
  **Source:** existing code `crates/core/src/quota.rs:164-167`. **Evidence:** same
- **DEC-006:** The cap is taken as an optional number (`f64`); it is valid when finite, whole and in
  1..=500, else `ToolError::invalid_argument("limit must be between 1 and 500")`; absent means 100.
  **Obliged by:** SPEC-0047/FR-NEW-004 (2.5 and -5 refused with the cap message). **Rationale:** an
  integer typed argument would turn 2.5 into a deserialization error with another text.
  **Alternatives considered:** `i64` argument (wrong message for a fractional cap). **Source:** spec
  E2E-014. **Evidence:** `crates/core/src/tools/trash.rs:87` (existing `invalid_argument` use)
- **DEC-007:** Extract the access check of `get_project_quota` into a private
  `fn reader_project(ctx, project_id) -> Result<Project>` used by both reads, behavior unchanged.
  **Obliged by:** SPEC-0047/FR-NEW-007. **Rationale:** one implementation of SPEC-0020 DEC-007 (admin
  learns a project is missing; anyone else gets the owner refusal). **Alternatives considered:** copy
  the block (two places to keep in step). **Source:** SPEC-0020 DEC-007. **Evidence:**
  `crates/core/src/tools/admin.rs:649-665`
- **DEC-008:** `In trash` means the path lies under the trash directory; `original_path` is the entry
  row of the first two components plus the remainder; `trash_entries` is backfilled first with
  `backfill_trash_entries`, then rows are read with `get_trash_entry`. **Obliged by:**
  SPEC-0047/FR-NEW-003, FR-NEW-006. **Rationale:** a file inside a trashed folder has no row of its
  own; legacy trashed nodes have no row until backfilled. **Alternatives considered:** join on
  `trash_path = path` (misses nested files). **Source:** existing code. **Evidence:**
  `crates/core/src/safety.rs:225`, `crates/core/src/tools/trash.rs:135`
- **DEC-009:** REST route `GET /api/admin/projects/{project_id}/disk-usage?limit=`, bearer only, calling
  the same function and mapping errors with `error_response`, mirroring the quota route.
  **Obliged by:** SPEC-0047/FR-NEW-011. **Rationale:** project convention that MCP and REST share one
  implementation. **Alternatives considered:** a path under `/api/fs/{mount_id}` (member audience).
  **Source:** SPEC-0020 DEC-006. **Evidence:** `crates/core/src/api/dataplane.rs:398-412`
- **DEC-010:** No new index and no schema change. **Obliged by:** SPEC-0047/NFR 7.1. **Rationale:** a
  bounded top N scan meets 5 seconds at 100 000 files; an index costs a migration on three engines.
  **Alternatives considered:** index `(volume_id, kind, size)`; a ranking table. **Source:** design.
  **Evidence:** `crates/core/src/storage/meta.rs:195-204`
- **DEC-011:** The frozen tool contract is regenerated with
  `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`
  and reviewed; the tool counts in assertions and docs move by one. **Obliged by:**
  SPEC-0047/FR-NEW-001. **Rationale:** project rule, the golden file is never hand edited.
  **Alternatives considered:** none. **Source:** `AGENTS.md`. **Evidence:**
  `crates/core/src/tools/contract_golden.rs:143-145`
- **DEC-012:** All acceptance tests are functional e2e through the real tool method or router.
  **Obliged by:** SPEC-0047/all E2E. **Rationale:** `rust` skill and the quota tests' pattern; the
  behaviors are observable at the tool answer. **Alternatives considered:** unit tests of
  `largest_files` (kept as extra, not as acceptance). **Source:** skill `rust`. **Evidence:**
  `crates/core/src/tools/quota_e2e.rs:60-139`

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0064 | list a project's largest files | P1 | FR-NEW-001, 002, 003 (partial), 004, 005, 007, 008 (partial) | main | 1 | no |
| US-0065 | show trashed files and the shared population | P1 | FR-NEW-003, 006 | US-0064 | 2 | no |
| US-0066 | the same breakdown on the web interface | P2 | FR-NEW-011 | US-0064 | 2 | no |
| US-0067 | one moment, read only, hardening, docs, spec close | P2 | FR-NEW-002, 005, 008, 009, 010 | US-0065, US-0066 | 1 | yes |
