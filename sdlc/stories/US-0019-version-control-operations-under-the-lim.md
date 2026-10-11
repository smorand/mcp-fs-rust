# US-0019: Version control operations under the limit

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0018
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Clone, pull, merge, cherry pick, rebase, revert, reset, branch operations, stash and file checkout all write project files; this story checks their net change, removed paths included, once before applying, and keeps aborts always accepted.

## Requirements covered
- SPEC-0020/FR-NEW-003: The Storage context SHALL check the limit on writes, appends, uploads, every kind of edit, copies, Word document creation, a conversion's Markdown file, archive extraction, and every version control operation that changes project files: clone, pull, merge, merge conflict resolution, cherry pick and its continuation, rebase and its continuation, revert and its continuation, reset, branch creation, branch switch, branch reset, checkout of a version or of a file, stash save, stash apply, stash pop, and a pull request merge into the project.
- SPEC-0020/FR-NEW-012: The Storage context SHALL accept moves, deletes to the trash, trash restores and version control pushes whatever the limit and the usage.

## Independent Test
`make check`; then with 48 576 bytes left, a merge adding 49 000 bytes is refused and the branch files are byte identical to before.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): Per volume `volume_usage` counter maintained in the node transactions, checked under a per volume in process admission lock held across the operation's writes. **Obliged by:** SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B), conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`, `:1693`, `:1728`.
- DEC-003 (SPEC-0020): Net change computed by the caller (it knows the previous sizes and removed paths) and passed to `admit`. **Obliged by:** FR-NEW-003, FR-NEW-004. **Rationale:** one admission per operation. **Alternatives considered:** per file checks inside `put_file` (breaks FR-NEW-007). **Source:** design. **Evidence:** `git/merge.rs:691-707`.
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
| `crates/core/src/git/merge.rs` | modify: `charge_quota` computes the net change and calls `admit` |
| `crates/core/src/tools/git.rs` | modify: `git.checkout_file` and clone call `admit` |
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

### SPEC-0020/E2E-031
- Category: failure; scenario: SC-002; requirements: FR-NEW-007, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given a remote repository whose files total 60 000 bytes (two files of 30 000). When Mia clones it into Atlas. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 60000 bytes requested`. And no cloned file exists in Atlas.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-032
- Category: failure; scenario: SC-002; requirements: FR-NEW-007, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas is a clone and the remote has new commits adding 50 000 bytes. When Mia pulls. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And the files stay at their previous commit.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-033
- Category: failure; scenario: SC-002; requirements: FR-NEW-007, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given branch `feature` adds 49 000 bytes over `main`. When Mia merges `feature` into `main`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 49000 bytes requested`. And the files of `main` are unchanged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-034
- Category: failure; scenario: SC-002; requirements: FR-NEW-007, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given branch `big` holds `/r.md` 50 000 bytes larger than on the current branch. When Mia checks out `big`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And the current branch and its files stay as they were.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-035
- Category: edge; scenario: SC-002; requirements: FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given usage is 1 500 000 bytes under a 1 MB limit and branch `slim` is 200 000 bytes smaller. When Mia checks out `slim`. Then the checkout is accepted. And usage is 1 300 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-036
- Category: happy; scenario: SC-002; requirements: FR-NEW-003, FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given a remote repository whose files total 40 000 bytes and whose history is 500 000 bytes. When Mia clones it into Atlas. Then the clone is accepted. And usage is 1 040 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-065
- Category: edge; scenario: SC-002; requirements: FR-NEW-003, FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044, and branch `feature` holds a commit adding `/k.txt` of 10 bytes. When Mia cherry picks that commit onto the current branch. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`. And `/k.txt` does not exist, and usage is still 1 500 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-066
- Category: failure; scenario: SC-002; requirements: FR-NEW-003, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the shared setup and a stash holding `/st.txt` of 50 000 bytes. When Mia pops the stash. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/st.txt` does not exist, and the stash still exists.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-067
- Category: edge; scenario: SC-002; requirements: FR-NEW-003, FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given the state after E2E-044, and branch `swap` differs from the current branch by deleting `/r.md` (100 000 bytes) and adding `/n2.txt` (50 000 bytes). When Mia checks out `swap`. Then the checkout is accepted. And `/r.md` is not in the trash, `/n2.txt` exists, and usage is 1 450 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-068
- Category: edge; scenario: SC-004; requirements: FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given Atlas has no limit and uses 1 400 000 bytes, Mia starts a merge that stops on a conflict after removing `/big.bin` (100 000 bytes), and Ada then sets the limit to 1 MB. When Mia aborts the merge. Then the abort is accepted. And `/big.bin` exists again, and usage is 1 400 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-069
- Category: failure; scenario: SC-001; requirements: FR-NEW-003, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/r.md` is 100 000 bytes on the current branch and 150 000 bytes in the last commit. When Mia restores `/r.md` from the last commit. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/r.md` is still 100 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files; existing acceptance tests except declared count bumps.
- Dependencies not to add: any crate.
- Patterns to avoid: a `WHERE` without `volume_id`; a second implementation in `api/dataplane.rs`; charging the session budget before the project check; sleeps for synchronization; dashes as punctuation.
- Scope boundary: lowered limit and concurrency (US-0020).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes, SPEC-0010's 8 quota tests included; session refusal message unchanged.
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0019: Version control operations under the limit`, body `Spec: SPEC-0020` and the requirement ids.
