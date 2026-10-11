# US-0015: Project usage and quota read

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 9
> Closes spec: no

## Objective
Nobody can see how much a project stores. This story keeps an exact per project usage figure, maintained wherever file bytes change, and lets an admin or the owner read the limit and the usage through MCP and REST.

## Requirements covered
- SPEC-0020/FR-NEW-002: The Storage context SHALL count as a project's usage the sum of the declared sizes of all its files, trashed files included, each path counted once even when files share content.
- SPEC-0020/FR-NEW-009: WHEN a platform admin or the project owner reads a project's quota THE Administration context SHALL show the project id, the limit in MB, and the bytes used computed at that moment.
- SPEC-0020/FR-NEW-010: The Administration context SHALL NOT show a project's quota to anyone who is neither a platform admin nor the project owner.
- SPEC-0020/FR-NEW-011: WHILE a project has no limit THE Storage context SHALL refuse no operation for the project limit.

## Independent Test
`make check`; then write three files of 1 000, 2 000 and 3 000 bytes, delete the second to the trash, purge it, and read `admin.get_project_quota`: `used_bytes` 4 000, compared with an independent `SUM` of file sizes read from the store.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): Per volume `volume_usage` counter maintained in the node transactions, checked under a per volume in process admission lock held across the operation's writes. **Obliged by:** SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B), conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`, `:1693`, `:1728`.
- DEC-006 (SPEC-0020): Read tool `admin.get_project_quota` returning `project_id`, `max_mb` (null when unlimited), `used_bytes`, plus REST `GET /api/admin/projects/{project_id}/quota` on the same function. **Obliged by:** FR-NEW-009, constraint 3.3. **Rationale:** mirrors `admin.set_project_quota` labels. **Alternatives considered:** adding usage to `admin.list_projects` (wider read, other gate). **Source:** existing code. **Evidence:** `mcp/server.rs:2789-2792`.
- DEC-007 (SPEC-0020): Read gate: admin then existence; non admin gets the owner refusal whether the project exists or not. **Obliged by:** FR-NEW-010. **Rationale:** no probing of project ids. **Alternatives considered:** reuse `require_owner_or_admin` (checks existence first for non admins, `state.rs:47-54`). **Source:** spec. **Evidence:** `storage/admin.rs:403-414`.
### Data model
**`volume_usage`** (new, meta schema, PK `volume_id`): `volume_id TextKey`, `used_bytes BigInt`.

- `put_file` adds `new_size - old_size` (it already reads the old size, `meta.rs:1615`); a new path
  adds `new_size`. `delete_file` subtracts the node size; `remove_subtree` subtracts the sum of the
  removed files; `rename` changes nothing (trash is a rename, so trashed files stay counted). All in
  the existing transactions, `volume_id` in every `WHERE`.
- Backfill: when the row is missing, `INSERT ... SELECT SUM(size) FROM nodes WHERE volume_id=? AND
  mode=file` under the admission lock, once.
- Migration additive; rollback: drop the table (it is derived data).

No change to the project table: `quota_bytes` exists (`storage/traits.rs:148`).
### Contracts
### 5.1 Refusal

New code `ERR_PROJECT_QUOTA_EXCEEDED` in `errors.rs` (16 codes), HTTP 507, constructor
`ToolError::project_quota_exceeded(limit_mb, used, requested)` producing exactly
`project storage quota of <N> MB exceeded: <U> bytes used, <W> bytes requested` (N = `quota_bytes /
1 048 576`, a whole number since it is set in MB).

### 5.2 Admission

```rust
// crates/core/src/quota.rs
pub(crate) struct Admission { _guard: tokio::sync::OwnedMutexGuard<()> }
pub(crate) async fn admit(state: &AppState, mount_id: &str, person: &str,
                          session_bytes: i64, delta: i64) -> Result<Admission>;
```

`delta` is the net change of FR-NEW-003 (new minus previous size per written path, minus the size of
each path a version control operation removes). `delta <= 0` always passes the project check
(FR-NEW-004). The session budget keeps today's amount (`session_bytes`, unchanged per path).

### 5.3 Read

MCP tool `admin.get_project_quota` (params `project_id`; read only, idempotent, not open world) returns
`{"project_id": "<id>", "max_mb": <int|null>, "used_bytes": <int>}`. REST:
`GET /api/admin/projects/{project_id}/quota`, same function, same body. Gate: an admin gets
`project '<id>' not found` for a missing project; anyone else gets
`'<person>' is not the owner of '<id>'` (`ERR_FORBIDDEN`) when the project is missing or not theirs.
Tool count 107 to 108.

### 5.4 Entry points switched to `admit`

| Entry | Today | delta | session_bytes |
|---|---|---|---|
| write, write_bytes, upload, append, edit kinds, write_docx (`fs_ops.rs:575`, `:611`, `:841`, `:905`, `:934`, `:1295`, `:1749`) | `charge_write(len)` | new size minus previous size | unchanged |
| copy file / folder (`fs_ops.rs` `copy_path`, `volume.rs:209`, `:222`) | file copy charged, folder copy not | size of every copied file minus previous size of overwritten targets | unchanged (0 for folder copy) |
| archive extraction (`tools/archive.rs:183-187`) | one charge of declared total | sum of entry sizes minus previous sizes | unchanged |
| conversion sibling (`core/fs_ops.rs` `land_conversion`) | one charge of sibling + set + text | sibling bytes minus previous sibling size (artifacts excluded, FR-NEW-002) | unchanged |
| git apply (`git/merge.rs:691` `charge_quota`, callers listed in spec 14.3) | sum of written blobs | written blobs minus previous sizes minus removed paths | unchanged |
| `git.checkout_file` (`tools/git.rs:1403`), clone (`tools/git.rs:3121`) | direct charge | as above | unchanged |
| aborts (`tools/git.rs:1600`, `:1643`, `:1685`, `:1727`) | none | not checked (FR-NEW-012) | unchanged |
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify: `volume_usage` table, counter in `put_file`/`delete_file`/`remove_subtree` transactions, backfill, read |
| `crates/core/src/storage/conformance.rs` | modify: usage counter case |
| `crates/core/src/quota.rs` | create: usage read with backfill, per volume admission lock skeleton |
| `crates/core/src/tools/admin.rs` | modify: `get_project_quota` with the DEC-007 gate |
| `crates/core/src/mcp/server.rs, crates/core/src/api/dataplane.rs, crates/core/src/api/openapi.rs` | modify: tool and `GET /api/admin/projects/{project_id}/quota` |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | regenerate (`MCPFS_REWRITE_TOOL_CONTRACT=1`), bump counts 107 to 108; one mechanical unit |
| `crates/core/src/tools/quota_e2e.rs` | create or extend: the story's e2e tests |
### Existing patterns
- Tool method and admin gate: `admin_set_project_quota` (`crates/core/src/mcp/server.rs:2774-2796`), `require_owner` (`crates/core/src/storage/admin.rs:403-414`).
- Session charge: `SafetyManager::charge_write` (`crates/core/src/safety.rs:130-136`).
- Node transaction with old size: `put_file` (`crates/core/src/storage/meta.rs:1615`); meta table pattern `trash_entries` (`meta.rs` schema).
- Version control apply: `merge::charge_and_apply` (`crates/core/src/git/merge.rs:669-707`).
- Tool e2e harness: `crates/core/src/tools/artifacts.rs` `mod e2e` (SPEC-0019).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

Shared setup unless stated: project `Atlas` has a limit of 1 MB (1 048 576 bytes) and uses
1 000 000 bytes (48 576 bytes left). Mia (`mia@example.com`) and Olivia (`olivia@example.com`,
owner) are members with no session budget used; Ada (`ada@example.com`) is a platform admin; Otto
(`otto@example.com`) belongs to no project. Each member's session budget is 262 144 bytes.

### SPEC-0020/E2E-001
- Category: happy; scenario: SC-001; requirements: FR-NEW-001, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia writes a new file `/a.txt` of 40 000 bytes. Then the write is accepted. And Ada reads Atlas's usage as 1 040 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-016
- Category: edge; scenario: SC-001; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/s.bin` is 40 000 bytes. When Mia copies it to `/s2.bin`. Then the copy is accepted. And usage is 1 040 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-020
- Category: side effect; scenario: SC-003; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia exports the whole project as a zip of 900 000 bytes. Then the export is accepted. And usage is still 1 000 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-021
- Category: side effect; scenario: SC-003; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia commits all of Atlas's files to version control history. Then the commit is accepted. And usage is still 1 000 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-037
- Category: happy; scenario: SC-003; requirements: FR-NEW-009
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Ada reads Atlas's quota. Then she sees project `Atlas`, limit 1 MB and 1 000 000 bytes used.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-038
- Category: happy; scenario: SC-003; requirements: FR-NEW-009
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Olivia reads Atlas's quota. Then she sees project `Atlas`, limit 1 MB and 1 000 000 bytes used.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-039
- Category: failure; scenario: SC-003; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Mia is a member of Atlas but not its owner. When she reads Atlas's quota. Then she is refused with `'mia@example.com' is not the owner of 'Atlas'`. And no limit or usage is shown.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-040
- Category: failure; scenario: SC-003; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Otto belongs to no project. When he reads Atlas's quota. Then he is refused with `'otto@example.com' is not the owner of 'Atlas'`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-041
- Category: failure; scenario: SC-003; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given project `Zephyr` does not exist. When Otto reads Zephyr's quota, then Ada does. Then Otto is refused with `'otto@example.com' is not the owner of 'Zephyr'`. And Ada is refused with `project 'Zephyr' not found`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-042
- Category: edge; scenario: SC-003; requirements: FR-NEW-009, FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has no limit and uses 5 000 000 bytes. When Ada reads Atlas's quota. Then she sees project `Atlas`, no limit value (not 0, not text), and 5 000 000 bytes used.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-043
- Category: edge; scenario: SC-003; requirements: FR-NEW-009
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given project `Atlas` is empty. When Ada reads its quota, Mia writes a 1 234 byte file, and Ada reads it again. Then the first read shows 0 bytes used. And the second shows 1 234 bytes used.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-053
- Category: happy; scenario: SC-001; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has no limit and uses 5 000 000 bytes. When Mia writes 262 144 bytes. Then the write is accepted. And usage is 5 262 144 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files; existing acceptance tests except declared count bumps.
- Dependencies not to add: any crate.
- Patterns to avoid: a `WHERE` without `volume_id`; a second implementation in `api/dataplane.rs`; charging the session budget before the project check; sleeps for synchronization; dashes as punctuation.
- Scope boundary: enforcement (US-0016 onward).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes, SPEC-0010's 8 quota tests included; session refusal message unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0015: Project usage and quota read`, body `Spec: SPEC-0020` and the requirement ids.
