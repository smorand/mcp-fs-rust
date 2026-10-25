# US-0094: paused operations and concurrent deletions

> Spec: SPEC-0053 (sdlc/specs/SPEC-0053-delete-a-tag-on-the-remote/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0092
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Prove and lock in that a paused merge, rebase, cherry pick or revert does not block a remote deletion, and that two members deleting the same remote tag get exactly one success and one refusal, with the lease protecting a tag that moved between the look and the push.

## Requirements covered
- SPEC-0053/FR-NEW-001: WHEN a member deletes a named tag on the remote and states the commit the remote tag points at now, THE Remote exchange context SHALL remove that tag from the remote and only that tag.
- SPEC-0053/FR-NEW-003 (partial: moved meanwhile): IF the stated commit is not the commit the remote tag points at now, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.
- SPEC-0053/FR-NEW-004 (partial: loser of a race): IF the remote has no tag of that name, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v9.9' does not exist.`
- SPEC-0053/FR-NEW-010: WHILE a merge, rebase, cherry pick or revert is paused, THE Remote exchange context SHALL still delete a confirmed remote tag, and leave the paused operation as it was.
- SPEC-0053/FR-NEW-011: WHEN two members delete the same remote tag at the same time, THE Remote exchange context SHALL let exactly one succeed and refuse the other.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green; two concurrent deletions of `v1.0` leave one audit entry and the second caller gets `Remote tag 'v1.0' does not exist.`

## Design excerpt
### Decisions
- DEC-008 (SPEC-0053): When the push reports a lost lease (the remote tag moved or was deleted between listing and push), re list: the tag absent gives `Remote tag '<n>' does not exist.`, otherwise the `moved` refusal with the new commit; the push is never retried.
- DEC-009 (SPEC-0053): The whole operation holds the repository `write_lock` (as publication does, `git_tags.rs:551`) so two members of one server serialize; the lease covers other processes. `reject_if_operation_in_progress` is NOT called: a paused merge, rebase, cherry pick or revert does not block the deletion.
- DEC-015 (SPEC-0053): Three `#[cfg(test)]` seams carry the cases a local file remote cannot produce: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or deletes the remote tag between listing and push), `forced_rejection(url)` (the delete push reports a per ref rejection, since the local transport runs no hooks) and `fault_after_push(url)` (a successful delete push is reported as a connection error). `closing_remote` simulates a dropped connection. Production builds contain none of them.

### Data model
No schema change; a remote deletion writes nothing to the database.

### Contracts
No contract change. The lease on the advertised tag object (DEC-004, DEC-008) is exercised through the existing `after_listing` test seam of `git_tags.rs`, extended to fire in the delete path. A lost lease re-lists: tag absent gives `Remote tag '<n>' does not exist.`, otherwise the `moved` refusal with the new commit.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (fire the `after_listing` seam in the delete path; lost lease handling if not already delivered) |
| `crates/core/src/tools/git.rs` | modify (tests) |

### Existing patterns
Existing pattern, a tool built on an engine: `tool_remote_push_tag` and `publish_tag` in `crates/core/src/tools/git_tags.rs:466-611` (`open`, remote lookup via `entry.db.list_remotes()`, `resolve_clone_credential`, `trace_remote_operation`, `record_audit` after success); declaration pattern: `GitRemotePushTagArgs` and `git_remote_push_tag` in `crates/core/src/mcp/server.rs` (~3516) with the `to_call_result(...)` wrapper.

Test helpers (private to the tests module of `crates/core/src/tools/git.rs`): `call_publish_tag`, `publish_as`, `bare_refs`, `publish_audit`, `cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`, `bare_ref_sha`, `closing_remote`, `with_git_hosts_lock`; the seam `after_listing` in `git_tags.rs`; the SPEC-0052 tests `e2e_tag_033` to `e2e_tag_085` show the shapes (audit assertions read the audit log like `publish_audit`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_delete_remote_tag` with a `file://` bare remote (the registered tool for non member and expired credential cases).

### SPEC-0053/E2E-029: A paused operation does not block the deletion
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_029_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** a merge is paused in `proj-a`; then, in three further runs, a rebase, a cherry pick and a revert. **When** Alice deletes the remote tag `v1.0` stating `f6a7b8c` in each run. **Then** each deletion succeeds. **And** the paused operation is still paused afterwards, with its state unchanged.
- **Priority:** High

### SPEC-0053/E2E-030: Two members delete the same remote tag
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_030_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** Alice and Bob both looked at the remote tag `v1.0` at `f6a7b8c`. **When** both delete it at the same moment stating `f6a7b8c`. **Then** exactly one succeeds and the other is refused with `Remote tag 'v1.0' does not exist.`. **And** exactly one audit entry exists.
- **Priority:** Critical

### SPEC-0053/E2E-031: Stale view while another member moves the tag
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_031_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-011, FR-NEW-003
- **Given** Alice looked at `v1.0` at `f6a7b8c`; at the same moment Bob moves the remote tag to `e5f6a7b` by publishing with confirmation. **When** Alice states `f6a7b8c`. **Then** either Alice's deletion came first and succeeds, or she is refused with `Remote tag 'v1.0' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`. **And** the tag Bob placed at `e5f6a7b` is never deleted by Alice's stale confirmation; the trail holds one entry if Alice's deletion came first and none if she was refused.
- **Priority:** High

### SPEC-0053/E2E-038: Paused operation and an unknown tag
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_038_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-004
- **Given** a merge is paused in `proj-a`; the remote has no `v9.9`. **When** Alice deletes `v9.9` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** the merge is still paused with its state unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-039: Paused operation and a wrong commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_039_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-003
- **Given** a merge is paused in `proj-a`; the remote tag `v1.0` points at `f6a7b8c`. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the merge is still paused and the remote tag is unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-040: A retry after a successful deletion
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_040_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-011
- **Given** Alice has just deleted the remote tag `v1.0`. **When** Bob, who looked at it earlier, deletes `v1.0` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v1.0' does not exist.`. **And** the trail holds exactly one entry for `v1.0`.
- **Priority:** High

## Constraints
- The remote deletion must not call `reject_if_operation_in_progress`.
- Concurrency is serialized by the repository write lock; the lease protects against other processes.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git test stays green, in particular the tag publish and branch push tests; `tool_contract_golden_is_current` green after review of the regenerated diff.
- [ ] Docs updated: only the counts the parity tests force; the closing story documents the action.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0094: paused operations and concurrent deletions`, body `Spec: SPEC-0053` and the requirement ids.
