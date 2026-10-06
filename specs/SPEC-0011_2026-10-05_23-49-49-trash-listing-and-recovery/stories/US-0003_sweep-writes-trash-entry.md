# US-0003: Sweep writes the trash entry

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 3
> Depends On: US-0001, US-0002
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
The existing background purge sweep (`purge::sweep_project_files`) already soft-deletes stale files
on a schedule. This story makes it record a `trash_entries` row for each one it trashes, exactly like
`fs.delete` now does (US-0002), so system-initiated deletes are just as listable and restorable as
user-initiated ones — distinguished only by `deleted_by: null`.

## Technical Context

### Stack
Rust 2024, tokio async.

### Relevant File Structure
```
crates/core/src/purge.rs   # sweep_project_files lives here
```

### Existing Patterns
`sweep_project_files`'s current loop (what you are extending):
```rust
// crates/core/src/purge.rs, inside sweep_project_files
for node in client.meta.stale_files(threshold, &trash_root).await? {
    let dst = safety.trash_path(&node.path);
    client.makedirs(&parent_of(&dst), true).await?;
    client.rename(&node.path, &dst).await?;   // <- replace this call
    purged += 1;
}
```
Replace the direct `client.rename(...)` call with US-0002's shared helper:
```rust
let final_dst = rename_with_collision_retry(&client, &node.path, &dst).await?;
```
Then write the `trash_entries` row using US-0001's `insert_trash_entry` trait method, with
`deleted_by: None`, in the same transaction as the rename (same reliability requirement as
US-0002). Each file still commits independently — `purge.rs:26`'s existing doc comment ("Each file
commits independently... no cascading rollback across files") is unchanged by this story; the
atomicity requirement is per-file (rename + its own trash_entries row), not across files.

### Data Model (excerpt)
Same `trash_entries` row as US-0001/US-0002, with `deleted_by: None` (SQL `NULL`) always.

### Decisions That Govern This Story
- **DEC-001** (§17): this story is the second of FR-NEW-003's two co-implementers alongside
  FR-NEW-002 (US-0002) — same row shape, different caller.
- **DEC-002/DEC-008**: the collision-retry mechanics and the `ERR_INTERNAL_ERROR` exhaustion code
  are already decided and already implemented by US-0002; this story only calls the existing
  `rename_with_collision_retry` helper, it does not re-decide or re-implement either.

### Applicable NFRs
- **7.4 Reliability**: per-file atomicity (rename + trash_entries insert commit or roll back
  together), matching US-0002 exactly, and matching the existing "each file commits independently"
  sweep semantics — a failure on one file must not roll back or block any other file in the same
  sweep cycle.

### Bounded Context
**Trash tracking** and **File lifecycle** (§4.5) — same intersection as US-0002, for the sweep's
caller instead of `fs.delete`'s.

## Functional Requirements

### FR-NEW-003: `sweep_project_files` writes a trash entry
- **EARS:** [EARS-E] "WHEN `purge::sweep_project_files` soft-deletes a stale file THE system SHALL
  insert a `trash_entries` row identical in shape to FR-NEW-002's, with `deleted_by` set to `null`."

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| aged fixture files | files with `atime` set N days in the past | existing `purge.rs` fixture helpers | ready |
| forced-failure insert seam | a way to make one `trash_entries` insert fail once | new test-only instrumentation this story adds | pending |

### E2E-NEW-403: sweep writes a system-initiated trash entry
- **Category:** Happy
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Preconditions:** project `proj-trash-1`, `PurgeConfig{autopurge_enabled: true,
  use_internal_purge: true, file_retention_days: Some(1)}`, file `/old.txt` with `atime` 3 days old.
- **Steps:** Given the stale file above / When `sweep_project_files(client, admin, safety,
  "proj-trash-1")` runs / Then it returns `Ok(1)` / And a `trash_entries` row exists for the trashed
  path with `original_path="/old.txt", deleted_by=null` (DB query, asserting SQL `NULL`, not empty
  string).
- **Priority:** Critical

### E2E-NEW-440: sweep-created entries are listable and restorable like any other
- **Category:** Side Effect
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Preconditions:** project `proj-sweep`, same `PurgeConfig` as above, `/stale.txt` with `atime` 5
  days old. **Note:** the "listable and restorable" half of this test needs US-0004/US-0005 to
  exist to actually call `fs.trash_list`/`fs.trash_restore`; at this story's point in the dependency
  order, verify the row directly via DB query instead, and leave a one-line TODO citing this test for
  US-0004/US-0005 to extend into the full round trip once those tools exist. This story's own
  obligation is: the row is written correctly and is structurally identical (`deleted_by: null`
  aside) to a user-initiated one.
- **Steps:** Given the stale file / When `sweep_project_files` trashes it / Then a `trash_entries`
  row exists with `deleted_by: null`, and every other field (`trash_path`, `original_path`, `size`,
  `kind`) matches the shape US-0002's tests already assert for a user-initiated delete (DB query
  comparison, not a tool-level round trip yet).
- **Priority:** High

### E2E-NEW-445: a failed `trash_entries` insert rolls back the sweep's rename
- **Category:** Failure
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003 (and FR-NEW-001's persistence target)
- **Preconditions:** project `proj-sweep-fail`, stale file `/stale2.txt` eligible for sweep, the
  `trash_entries` insert step instrumented to fail once.
- **Steps:** Given the stale file and the forced insert failure / When `sweep_project_files`
  processes it / Then the per-file failure is reported (matching the existing "each file commits
  independently" semantics, `purge.rs:26`) and `/stale2.txt` remains live, NOT trashed
  (filesystem/node check) — the rename and the `trash_entries` insert commit or roll back together.
- **Priority:** High

### E2E-NEW-447: collision exhaustion inside the sweep surfaces the same error
- **Category:** Failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005 (second caller of US-0002's helper, not a new decision)
- **Preconditions:** same 50-destination collision setup as US-0002's E2E-NEW-405, but triggered via
  `sweep_project_files` instead of `fs.delete`.
- **Steps:** Given the 51 pre-occupied destinations and a stale file that would collide / When
  `sweep_project_files` processes that file / Then the per-file failure surfaces the same
  `ERR_INTERNAL_ERROR` condition, the sweep continues to the next file (per existing per-file
  isolation), and the stale file remains live.
- **Priority:** Low

### E2E-NEW-457: sweep with no stale files writes no trash entries
- **Category:** Edge
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Preconditions:** project `proj-sweep-empty`, `PurgeConfig{autopurge_enabled: true,
  use_internal_purge: true, file_retention_days: Some(7)}`, no files with `atime` older than the
  threshold.
- **Steps:** Given no stale files exist / When `sweep_project_files` runs / Then it returns `Ok(0)`
  (function return check) / And `SELECT COUNT(*) FROM trash_entries WHERE volume_id=?` is unchanged
  (DB query) — the sweep being a no-op writes nothing.
- **Priority:** Low

## Constraints

### Files Not to Touch
- `crates/core/src/core/fs_ops.rs` — already built in US-0002; call its `rename_with_collision_retry`
  helper, don't duplicate it.

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not reimplement collision retry locally in `purge.rs` — it must be the same helper US-0002
  built, called from here.

### Scope Boundary
- Only `sweep_project_files`'s file-sweep loop changes. `sweep_project` (project-level soft-delete)
  and `sweep_grace_period` are untouched — they soft-delete *projects*, a different feature this spec
  explicitly excludes (§3.2).

## Non Regression

### Existing Tests That Must Pass
All 26 existing tests in `purge.rs` (recounted directly: `e2e_new_024` through `e2e_new_039` and
`unconfigured_project_is_a_no_op` at `purge.rs:293-410`,
`run_cycle_sweeps_every_project_and_aggregates_counts` and
`run_cycle_on_zero_projects_is_a_zero_count_no_op` at `purge.rs:538-573`, `e2e_new_079` at
`purge.rs:585`, `e2e_new_053` through `e2e_new_057` at `purge.rs:658-703`, and `e2e_new_080` at
`purge.rs:857`) stay green unmodified.

### Behaviors That Must Not Change
- `sweep_project_files`'s return value (`Ok(usize)` count of purged files) and its gating on
  `autopurge_enabled`/`use_internal_purge`/`file_retention_days` are unchanged.

### API Contracts to Preserve
- n/a — `sweep_project_files` is not a public tool surface.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
