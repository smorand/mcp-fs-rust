# US-0005: `fs.trash_restore` tool

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 5
> Depends On: US-0004
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective
Expose `fs.trash_restore`, the second MCP tool: it moves a trashed file (or whole directory subtree)
back to its original location, never failing on a collision — it renames to `_restored`,
`_restored2`, etc. instead. This closes the full trash lifecycle: a file can now be deleted, listed,
and recovered.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0, axum.

### Relevant File Structure
```
crates/core/src/tools/trash.rs   # continues the module US-0004 started
crates/core/src/mcp/server.rs    # the #[tool] method for fs.trash_restore goes here
```

### Existing Patterns
Same handler shape as US-0004 (AGENTS.md Conventions, `state.authorize` first, then normalize,
then call the engine function). `delete_path`'s own ancestor-creation call is the pattern to mirror
for restore's destination:
```rust
// crates/core/src/core/fs_ops.rs:1019-1025, the pattern to mirror symmetrically on restore
client.makedirs(&parent_of(&dst), true).await?;
client.rename(norm, &dst).await?;
```
On restore, `dst` is `original_path` (or a `_restoredN` variant on collision) instead of a trash
path, but the "create missing ancestors before renaming" shape is identical.

Reuse US-0004's backfill helper (FR-NEW-011) for the single-entry case — do not reconstruct
`original_path` a second, different way here. If `trash_path` names a node that exists on disk but
has no `trash_entries` row yet, call the same backfill logic US-0004 uses for its bulk case, just for
this one path, then proceed with the now-tracked row.

### Data Model (excerpt)
Deletes the `trash_entries` row (US-0001's `delete_trash_entry`) on successful restore, in the same
transaction as the `rename` back (same reliability pattern as US-0002/US-0003).

### Decisions That Govern This Story
- **DEC-002** (§17): "restore-destination collisions retry with `_restoredN` and never fail...
  restore should never block on a collision." This story implements the restore-side half of
  DEC-002 (the write-side `~N` half is already done, in US-0002).
- **DEC-003** (§17): the restore-triggered half of the lazy backfill (FR-NEW-011), reusing
  US-0004's helper, not re-implementing it.
- **DEC-004/DEC-005** (§17): same ACL pattern as US-0004 — cited here, not re-decided.

### Applicable NFRs
- **7.4 Reliability**: rename-back and `trash_entries` delete commit or roll back together.
- **7.5 Observability**: `fs.trash_restore` calls at INFO.

### Bounded Context
**Trash tracking** (§4.5).

## Functional Requirements

### FR-NEW-009: `fs.trash_restore`
- **EARS:** [EARS-E] "WHEN `fs.trash_restore(mount_id:string, trash_path:string)` is called THE
  system SHALL resolve the entry's `original_path` (backfilling per FR-NEW-011 if the entry is
  untracked but the node exists), recreate any missing ancestor directories of the restore
  destination, rename the node back, delete the `trash_entries` row, and return
  `{"restored_path": "<final path>", "trash_path": "<input>"}`."
- **Inputs / Outputs:** `mount_id:string`, `trash_path:string` (both required) →
  `{"restored_path": string, "trash_path": string}`.
- **Business Rules:** authorization via `state.authorize(mount_id, person)`. Ancestor directories
  created via `makedirs(parent, true)`, same pattern as `delete_path`'s trash-side.
- **Exact names:** tool `fs.trash_restore`; error codes `ERR_NOT_FOUND`, `ERR_FORBIDDEN`,
  `ERR_INVALID_ARGUMENT`, `ERR_INTERNAL_ERROR`.

### FR-NEW-010: restore destination collision
- **EARS:** [EARS-O] "IF the restore destination (`original_path`, or a prior suffixed form)
  already exists THEN THE system SHALL retry at `original_path + \"_restored\"`, then
  `\"_restored2\"`, `\"_restored3\"`, ..., appending the suffix after the full filename including
  its extension, until a free path is found. The system SHALL NOT fail `fs.trash_restore` due to
  this collision."

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| occupied-destination fixtures | pre-existing files at `original_path`, `original_path_restored`, etc. | new test fixture this story adds | pending |

### E2E-NEW-417: restore to the original path
- **Category:** Happy / State Transition / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- **Preconditions:** project `proj-restore-1`, file `/doc.txt` with content `"hello"`, deleted
  producing `trash_path="/.mcp_trash/{ts}__doc.txt"`.
- **Steps:** Given the trashed file / When `fs.trash_restore(mount_id="proj-restore-1",
  trash_path="/.mcp_trash/{ts}__doc.txt")` is called / Then the response is `{"restored_path":
  "/doc.txt", "trash_path": "/.mcp_trash/{ts}__doc.txt"}` / And `/doc.txt` exists again with content
  `"hello"` (filesystem/node check) / And the `trash_entries` row for that `trash_path` no longer
  exists (DB query).
- **Priority:** Critical

### E2E-NEW-418: restore of an unknown trash path
- **Category:** Failure / **Scenario:** SC-002 / EXC-002a / **Requirements:** FR-NEW-009
- **Steps:** Given project `proj-restore-1` / When
  `fs.trash_restore(mount_id="proj-restore-1", trash_path="/.mcp_trash/999__nope.txt")` is called,
  no such node or row exists / Then the call fails with `ERR_NOT_FOUND`.
- **Priority:** Critical

### E2E-NEW-419: restore forbidden for a non-member
- **Category:** Failure / **Scenario:** SC-002 / **Requirements:** FR-NEW-016 (cited, not
  re-decided — same ACL pattern as US-0004)
- **Steps:** Given project `proj-restore-1` with member `alice` only, a trashed entry present / When
  `eve` (non-member) calls `fs.trash_restore(...)` / Then the call fails with `ERR_FORBIDDEN`.
- **Priority:** Critical

### E2E-NEW-420: restore collision renames to `_restored`
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-010
- **Preconditions:** project `proj-restore-1`, `/a.txt` deleted (trashed), then a new `/a.txt`
  written at the original path.
- **Steps:** Given both the trashed entry and the new occupying file / When
  `fs.trash_restore(mount_id="proj-restore-1", trash_path="<a.txt's trash_path>")` is called / Then
  the call succeeds with `{"restored_path": "/a.txt_restored", "trash_path": "..."}` / And both
  `/a.txt` (new) and `/a.txt_restored` (restored) exist distinctly (filesystem/node check) / And the
  `trash_entries` row is deleted (DB query).
- **Priority:** Critical

### E2E-NEW-421: double collision renames to `_restored2`
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-010
- **Preconditions:** state from E2E-NEW-420, plus a second distinct trashed copy of `/a.txt`.
- **Steps:** Given `/a.txt` and `/a.txt_restored` both occupied, and a second trashed `/a.txt` entry
  / When that second entry is restored / Then it lands at `/a.txt_restored2`.
- **Priority:** High

### E2E-NEW-422: collision suffix appended after the extension
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-010
- **Preconditions:** project `proj-restore-1`, `/report.pdf` deleted, then a new `/report.pdf`
  written to occupy the original path.
- **Steps:** Given the collision / When the trashed `/report.pdf` is restored / Then the restored
  path is exactly `/report.pdf_restored`, not `/report_restored.pdf`.
- **Priority:** High

### E2E-NEW-423: restoring a directory restores the whole subtree
- **Category:** Happy / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- **Preconditions:** project `proj-restore-1`, directory `/proj/` containing `/proj/x.txt` and
  `/proj/sub/y.txt`, deleted via `fs.delete(path="/proj", recursive=true, trash=true)`.
- **Steps:** Given the trashed directory subtree / When `fs.trash_restore(mount_id="proj-restore-1",
  trash_path="<proj's trash_path>")` is called / Then `/proj/`, `/proj/x.txt`, `/proj/sub/y.txt` all
  exist again with original content (filesystem/node check on every node in the subtree) / And
  exactly one `trash_entries` row is deleted (DB query).
- **Priority:** Critical

### E2E-NEW-424: restore backfills an untracked node inline
- **Category:** Edge / **Scenario:** cross / **Requirements:** FR-NEW-011
- **Preconditions:** project `proj-backfill-2`, node seeded at
  `/.mcp_trash/1700000000000__orphan.txt`, no prior `fs.trash_list` call.
- **Steps:** Given the untracked node / When `fs.trash_restore(mount_id="proj-backfill-2",
  trash_path="/.mcp_trash/1700000000000__orphan.txt")` is called directly / Then the restore
  succeeds, restoring to `/orphan.txt` (filesystem/node check) / And no leftover `trash_entries` row
  exists afterward (DB query — created then deleted within the same call).
- **Priority:** Critical

### E2E-NEW-425: restoring twice fails the second time
- **Category:** Idempotency / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- **Preconditions:** state immediately after E2E-NEW-417 succeeds.
- **Steps:** Given the row and node are both gone / When
  `fs.trash_restore(mount_id="proj-restore-1", trash_path="/.mcp_trash/{ts}__doc.txt")` is called
  again with the identical path / Then the call fails with `ERR_NOT_FOUND`.
- **Priority:** Critical

### E2E-NEW-426: restore deletes the row, verified independently
- **Category:** Side Effect / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- **Preconditions:** project `proj-restore-1`, `/z.txt` deleted, `trash_entries` row confirmed
  present via direct DB query before restore.
- **Steps:** Given the confirmed row / When `fs.trash_restore` is called on it / Then `SELECT
  COUNT(*) FROM trash_entries WHERE volume_id=? AND trash_path=?` returns `0` (DB query, independent
  of the tool's response).
- **Priority:** Critical

### E2E-NEW-427: any member can list/restore, not just the deleter
- **Category:** Happy / ACL / **Scenario:** ACL / **Requirements:** FR-NEW-016
- **Preconditions:** project `proj-acl`, members `alice` (deleter) and `bob`, `/shared.txt` deleted
  by `alice`.
- **Steps:** Given `/shared.txt` trashed by `alice` / When `bob` calls
  `fs.trash_list(mount_id="proj-acl", ...)` then `fs.trash_restore(mount_id="proj-acl",
  trash_path="<shared.txt's trash_path>")` / Then both calls succeed for `bob`; the list entry
  showed `deleted_by="alice"` but did not block `bob`'s restore.
- **Priority:** Critical

### E2E-NEW-436: full lifecycle, live → trashed → listed → restored → live
- **Category:** State Transition / **Scenario:** cross / **Requirements:** FR-NEW-007, FR-NEW-009
- **Preconditions:** project `proj-lifecycle`, file `/cycle.txt` with content `"v1"`.
- **Steps:** Given `/cycle.txt` exists / When, in order: `fs.delete(trash=true)` trashes it;
  `fs.trash_list` is called; `fs.trash_restore` restores it; `fs.read("/cycle.txt")` is called / Then
  after delete, the file is absent from `fs.list("/")` but present in `fs.trash_list` / And after
  restore, the `trash_entries` row is gone and the file reappears in `fs.list("/")` / And `fs.read`
  returns `"v1"` unchanged.
- **Priority:** Critical

### E2E-NEW-437: restore with an empty `trash_path`
- **Category:** Failure / **Scenario:** SC-002 / EXC-002c / **Requirements:** FR-NEW-009
- **Steps:** Given project `proj-restore-1` / When `fs.trash_restore(mount_id="proj-restore-1",
  trash_path="")` is called / Then the call fails with `ERR_INVALID_ARGUMENT`, not `ERR_NOT_FOUND`.
- **Priority:** Low

### E2E-NEW-439: restore recreates a missing ancestor directory
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- **Preconditions:** project `proj-restore-1`, `/parent/child/` trashed as a unit, then `/parent`
  itself hard-deleted (test fixture, `allow_hard_delete` on) after trashing.
- **Steps:** Given `/parent` no longer exists live / When `fs.trash_restore` restores
  `/parent/child/` / Then `/parent/` and `/parent/child/` both exist post-restore (ancestors
  recreated via `makedirs`).
- **Priority:** High

## Constraints

### Files Not to Touch
- `crates/core/src/trash_screen.rs` does not exist yet (US-0006).

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not reconstruct `original_path` with a second, different backfill algorithm — reuse US-0004's.
- Do not add a "force overwrite" flag that skips the `_restoredN` renaming — FR-NEW-010 is explicit
  that restore never fails and never silently overwrites.

### Scope Boundary
- GUI is US-0006's. This story is the MCP tool only.

## Non Regression

### Existing Tests That Must Pass
- `cargo test --workspace` stays green.

### Behaviors That Must Not Change
- `fs.trash_list`'s behavior from US-0004 is unaffected by this story's additions.

### API Contracts to Preserve
- n/a — new tool, added to `TOOL_CONTRACT.txt` only in US-0008.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
