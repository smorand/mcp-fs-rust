# US-0016: Single file writes refused over the limit

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0015
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
A project's limit has no effect today. This story refuses every single file write, append, upload, edit, copy and Word document that would push usage over the limit, with the exact message, before the session budget is charged.

## Requirements covered
- SPEC-0020/FR-NEW-001: IF an operation would make a project's usage exceed its limit THEN THE Storage context SHALL refuse it with `project storage quota of <N> MB exceeded: <U> bytes used, <W> bytes requested`.
- SPEC-0020/FR-NEW-003: The Storage context SHALL check the limit on writes, appends, uploads, every kind of edit, copies, Word document creation, a conversion's Markdown file, archive extraction, and every version control operation that changes project files: clone, pull, merge, merge conflict resolution, cherry pick and its continuation, rebase and its continuation, revert and its continuation, reset, branch creation, branch switch, branch reset, checkout of a version or of a file, stash save, stash apply, stash pop, and a pull request merge into the project.
- SPEC-0020/FR-NEW-004: IF an operation requests zero or fewer bytes THEN THE Storage context SHALL accept it whatever the limit and the usage.
- SPEC-0020/FR-NEW-005: The Storage context SHALL accept an operation that brings usage exactly to the limit.
- SPEC-0020/FR-NEW-006: The Storage context SHALL check the project limit before the session budget, and charge nothing to the session budget for an operation the limit refuses.
- SPEC-0020/FR-NEW-013: The Storage context SHALL NOT change any file or the usage when it refuses an operation for the limit.

## Independent Test
`make check`; then with a 1 MB limit and 1 000 000 bytes used, a 50 000 byte write answers `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`, and the store still holds no node at that path.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): Per volume `volume_usage` counter maintained in the node transactions, checked under a per volume in process admission lock held across the operation's writes. **Obliged by:** SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B), conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`, `:1693`, `:1728`.
- DEC-003 (SPEC-0020): Net change computed by the caller (it knows the previous sizes and removed paths) and passed to `admit`. **Obliged by:** FR-NEW-003, FR-NEW-004. **Rationale:** one admission per operation. **Alternatives considered:** per file checks inside `put_file` (breaks FR-NEW-007). **Source:** design. **Evidence:** `git/merge.rs:691-707`.
- DEC-004 (SPEC-0020): Order inside `admit`: project check first, session `charge_write` only after it passes. **Obliged by:** FR-NEW-006. **Rationale:** a refusal charges nothing. **Alternatives considered:** charge then refund (racy with other session writes). **Source:** spec. **Evidence:** `safety.rs:130-136`.
- DEC-005 (SPEC-0020): Error code `ERR_PROJECT_QUOTA_EXCEEDED`, HTTP 507 Insufficient Storage, message per FR-NEW-001. **Obliged by:** FR-NEW-001 and spec 14.3 (distinct category). **Rationale:** 507 names storage exhaustion, distinct from the 429 rate style session refusal. **Alternatives considered:** reuse 429 (indistinguishable for HTTP consumers by status), 413 (about one request body). **Source:** design. **Evidence:** `errors.rs:111-134`.
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
| `crates/core/src/errors.rs` | modify: `ERR_PROJECT_QUOTA_EXCEEDED`, 507, constructor, code count tests 15 to 16 |
| `crates/core/src/quota.rs` | modify: `admit` check and session charge order |
| `crates/core/src/core/fs_ops.rs` | modify: write, write_bytes, append, edit kinds, write_docx, copy file call `admit` |
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

### SPEC-0020/E2E-002
- Category: failure; scenario: SC-001; requirements: FR-NEW-001, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia writes a new file `/b.txt` of 50 000 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/b.txt` does not exist, and usage is still 1 000 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-003
- Category: edge; scenario: SC-001; requirements: FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia writes a new file of 48 576 bytes. Then the write is accepted. And usage is 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-004
- Category: failure; scenario: SC-001; requirements: FR-NEW-005, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 048 576 bytes. When Mia writes a new file of 1 byte. Then she is refused with `project storage quota of 1 MB exceeded: 1048576 bytes used, 1 bytes requested`. And usage is still 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-005
- Category: failure; scenario: SC-001; requirements: FR-NEW-006, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia writes a 300 000 byte file, over both the project limit and her session budget. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 300000 bytes requested`. And she does not receive `session write quota of 262144 bytes exceeded`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-006
- Category: side effect; scenario: SC-001; requirements: FR-NEW-006, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Mia was refused a 50 000 byte write as in E2E-002 and Ada then removes Atlas's limit. When Mia writes 262 144 bytes in the same session, then 1 more byte. Then the 262 144 byte write is accepted. And the 1 byte write is refused with `session write quota of 262144 bytes exceeded`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-007
- Category: failure; scenario: SC-001; requirements: FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has a limit of 2 MB and uses 0 bytes. When Mia writes 262 145 bytes. Then she is refused with `session write quota of 262144 bytes exceeded`. And the file does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-008
- Category: failure; scenario: SC-001; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/n.md` is 10 000 bytes. When Mia appends 48 577 bytes to it. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/n.md` is still 10 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-009
- Category: happy; scenario: SC-001; requirements: FR-NEW-003, FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/n.md` is 10 000 bytes. When Mia appends 48 576 bytes to it. Then the append is accepted. And `/n.md` is 58 576 bytes, and usage is 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-010
- Category: failure; scenario: SC-001; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/r.md` is 100 000 bytes. When Mia overwrites it with 148 577 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/r.md` keeps its previous content.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-011
- Category: edge; scenario: SC-001; requirements: FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 048 576 bytes and `/r.md` is 100 000 bytes. When Mia overwrites `/r.md` with different content of 100 000 bytes. Then the overwrite is accepted. And usage is still 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-012
- Category: edge; scenario: SC-001; requirements: FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 048 576 bytes. When Mia edits `/r.md` (one of the project's files, 100 000 bytes), replacing 5 000 bytes of text with 1 000 bytes. Then the edit is accepted. And usage is 1 044 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-013
- Category: failure; scenario: SC-001; requirements: FR-NEW-003, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/r.md` contains the single character `X`. When Mia edits it to replace `X` with 48 578 bytes of text. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/r.md` still contains `X`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-014
- Category: failure; scenario: SC-001; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia uploads `photo.png` of 60 000 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 60000 bytes requested`. And `photo.png` is not in Atlas.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-015
- Category: failure; scenario: SC-001; requirements: FR-NEW-002, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/big.bin` is 50 000 bytes. When Mia copies it to `/big2.bin`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/big2.bin` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-017
- Category: failure; scenario: SC-001; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup. When Mia creates a Word document `/w.docx` whose content makes a document of N bytes, N being the size the same content produces in a project with no limit and N greater than 48 576. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, <N> bytes requested`, N written out. And `/w.docx` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-054
- Category: failure; scenario: SC-001; requirements: FR-NEW-011, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has no limit. When Mia writes 262 145 bytes. Then she is refused with `session write quota of 262144 bytes exceeded`. And no project quota message appears.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-055
- Category: edge; scenario: SC-001; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Mia was refused a 50 000 byte write as in E2E-002. When Ada removes the limit and Mia retries the same write. Then the write is accepted. And usage is 1 050 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-056
- Category: edge; scenario: SC-001; requirements: FR-NEW-005, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given an empty Atlas with a 1 MB limit. When Mia writes a 1 048 577 byte file. Then she is refused with `project storage quota of 1 MB exceeded: 0 bytes used, 1048577 bytes requested`. And she does not receive `session write quota of 262144 bytes exceeded`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-057
- Category: edge; scenario: SC-001; requirements: FR-NEW-004, FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 048 576 bytes. When Mia creates an empty file `/empty.txt`. Then the write is accepted. And usage is still 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files; existing acceptance tests except declared count bumps.
- Dependencies not to add: any crate.
- Patterns to avoid: a `WHERE` without `volume_id`; a second implementation in `api/dataplane.rs`; charging the session budget before the project check; sleeps for synchronization; dashes as punctuation.
- Scope boundary: conversion (US-0017), multi file (US-0018), version control (US-0019).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes, SPEC-0010's 8 quota tests included; session refusal message unchanged.
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0016: Single file writes refused over the limit`, body `Spec: SPEC-0020` and the requirement ids.
