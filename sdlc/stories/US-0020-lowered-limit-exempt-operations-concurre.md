# US-0020: Lowered limit, exempt operations, concurrency

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0019
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
Closes SPEC-0020: a limit lowered below usage evicts nothing and refuses growing operations, moves, deletes, restores and pushes always pass, a purge frees space, and concurrent writes never overshoot. Updates the docs.

## Requirements covered
- SPEC-0020/FR-MOD-001: WHEN a platform admin sets a limit below a project's usage THE Administration context SHALL accept it, remove nothing, and from then on refuse every growing operation.
- SPEC-0020/FR-NEW-008: The Storage context SHALL NOT let concurrent operations leave a project's usage above its limit.
- SPEC-0020/FR-NEW-012: The Storage context SHALL accept moves, deletes to the trash, trash restores and version control pushes whatever the limit and the usage.

## Independent Test
`make check`; then 20 concurrent pairs of 8 000 byte writes with 8 576 bytes left: exactly one accepted, and the usage read from the store never exceeds 1 048 576.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): Per volume `volume_usage` counter maintained in the node transactions, checked under a per volume in process admission lock held across the operation's writes. **Obliged by:** SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B), conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`, `:1693`, `:1728`.
- DEC-009 (SPEC-0020): Aborts, moves, trash, restore, push never call `admit`. **Obliged by:** FR-NEW-012. **Rationale:** they add no project bytes. **Alternatives considered:** checking with delta 0 (needless lock). **Source:** spec. **Evidence:** `tools/git.rs:1600-1727`.
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
| `crates/core/src/tools/quota_e2e.rs` | create or extend: the story's e2e tests |
| `README.md, AGENTS.md` | modify: project quota enforcement, `admin.get_project_quota`, 108 tools |
| `.agent_docs/tools.md, .agent_docs/architecture.md, .agent_docs/api.md` | modify: tool, counter and admission lock, route |
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

### SPEC-0020/E2E-022
- Category: side effect; scenario: SC-004; requirements: FR-NEW-002, FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/old.txt` is 100 000 bytes. When Mia deletes it. Then the delete is accepted. And `/old.txt` is in the trash, and usage is still 1 000 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-023
- Category: side effect; scenario: SC-004; requirements: FR-NEW-012, FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/old.txt` of 100 000 bytes is in the trash. When it is permanently purged. Then usage is 900 000 bytes. And Mia can then write a new 148 576 byte file.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-024
- Category: edge; scenario: SC-004; requirements: FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 500 000 bytes under a 1 MB limit. When Mia moves `/a.txt` to `/arch/a.txt`. Then the move is accepted. And usage is still 1 500 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-025
- Category: edge; scenario: SC-004; requirements: FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 500 000 bytes under a 1 MB limit and `/t.txt` of 30 000 bytes is in the trash. When Mia restores it. Then the restore is accepted. And `/t.txt` is back, and usage is still 1 500 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-026
- Category: edge; scenario: SC-004; requirements: FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 500 000 bytes under a 1 MB limit. When Mia pushes new commits to Atlas's repository. Then the push succeeds. And no quota message appears.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-044
- Category: happy; scenario: SC-004; requirements: FR-MOD-001
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has no limit and uses 1 500 000 bytes. When Ada sets its limit to 1 MB. Then the change is accepted. And every file is still present, and usage is 1 500 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-045
- Category: failure; scenario: SC-004; requirements: FR-MOD-001, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044. When Mia writes a new 1 byte file. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 1 bytes requested`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-046
- Category: edge; scenario: SC-004; requirements: FR-MOD-001, FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044 and `/r.md` of 100 000 bytes. When Mia overwrites `/r.md` with 40 000 bytes. Then the overwrite is accepted. And usage is 1 440 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-047
- Category: side effect; scenario: SC-004; requirements: FR-MOD-001, FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044. When Mia deletes `/a.txt` (40 000 bytes) and moves `/b.txt` to `/c.txt`. Then both are accepted. And usage is still 1 500 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-048
- Category: edge; scenario: SC-004; requirements: FR-MOD-001, FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas uses 1 048 576 bytes and has no limit. When Ada sets the limit to 1 MB, then Mia writes a 1 byte file. Then the limit change is accepted. And the write is refused with `project storage quota of 1 MB exceeded: 1048576 bytes used, 1 bytes requested`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-049
- Category: failure; scenario: SC-005; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia and Olivia each write a different new 40 000 byte file at the same moment. Then exactly one write is accepted. And the other is refused with `project storage quota of 1 MB exceeded: 1040000 bytes used, 40000 bytes requested`, and usage is 1 040 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-050
- Category: happy; scenario: SC-005; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia and Olivia each write a new 24 000 byte file at the same moment. Then both writes are accepted. And usage is 1 048 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-051
- Category: failure; scenario: SC-005; requirements: FR-NEW-008, FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia extracts an archive whose entries total 30 000 bytes while Olivia writes a 30 000 byte file at the same moment. Then exactly one operation is accepted. And usage is 1 030 000 bytes, and when the refused one is the extraction none of its entries exists.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-052
- Category: edge; scenario: SC-005; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 040 000 bytes. When Mia and Olivia each try 20 times to write a new 8 000 byte file, concurrently. Then exactly one write is accepted. And usage is 1 048 000 bytes and never exceeded 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-061
- Category: failure; scenario: SC-004; requirements: FR-MOD-001, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044 and `/s.bin` of 10 bytes. When Mia copies `/s.bin` to `/s3.bin`. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`. And `/s3.bin` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-063
- Category: side effect; scenario: SC-005; requirements: FR-NEW-008, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the race of E2E-049 has finished and Ada then removes Atlas's limit. When the member whose write was refused writes 262 144 bytes in the same session. Then the write is accepted.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-064
- Category: failure; scenario: SC-005; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 048 000 bytes. When Mia and Olivia each write a new 1 000 byte file at the same moment. Then both are refused with `project storage quota of 1 MB exceeded: 1048000 bytes used, 1000 bytes requested`. And usage is still 1 048 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files; existing acceptance tests except declared count bumps.
- Dependencies not to add: any crate.
- Patterns to avoid: a `WHERE` without `volume_id`; a second implementation in `api/dataplane.rs`; charging the session budget before the project check; sleeps for synchronization; dashes as punctuation.
- Scope boundary: nothing: closes the spec.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes, SPEC-0010's 8 quota tests included; session refusal message unchanged.
- [ ] Docs updated: README.md, AGENTS.md, .agent_docs/tools.md, architecture.md, api.md.
- [ ] Self review: full.
- [ ] Closes spec only: every E2E-001 to E2E-069 green, every requirement of SPEC-0020 cited by a done story.
- [ ] One commit `US-0020: Lowered limit, exempt operations, concurrency`, body `Spec: SPEC-0020` and the requirement ids.
