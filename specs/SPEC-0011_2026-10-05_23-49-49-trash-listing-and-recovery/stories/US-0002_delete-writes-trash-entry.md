# US-0002: `fs.delete` writes the trash entry, collision-safe

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 2
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective
When `fs.delete` soft-deletes a path, it must now record a `trash_entries` row for it (so it can
later be listed and restored), and the trash-destination `rename` must never fail on a collision
between two different original paths flattening to the same destination — it retries with a `~N`
suffix instead. This story also closes the one case that must stay a prohibition: a **hard** delete
(`trash=false`) must never write a `trash_entries` row.

## Technical Context

### Stack
Rust 2024, tokio async. The engine function this story modifies is the single implementation both
the MCP tool layer and the REST `/api/fs` layer call (AGENTS.md: *"Never reimplement an operation in
the tool layer... `core::fs_ops` is the only place an operation is written"*).

### Relevant File Structure
```
crates/core/src/core/fs_ops.rs   # delete_path lives here; this story's new helper lives here too
```

### Existing Patterns
`delete_path`'s current soft-delete branch (what you are extending):
```rust
// crates/core/src/core/fs_ops.rs, inside delete_path, soft-delete branch
let dst = safety.trash_path(norm);
client.makedirs(&parent_of(&dst), true).await?;
client.rename(norm, &dst).await?;
```
`client.rename(src, dst)` returns `ERR_NO_CLOBBER` (via `ToolError::no_clobber`) when `dst` already
exists (`storage/meta.rs:701`, `storage/meta.rs:716-718`) — it never silently overwrites. This is the
exact error your retry loop watches for.

`MetaBackend::rename` signature, for reference (you are not changing it, only calling it in a loop):
```rust
async fn rename(&self, src: &str, dst: &str) -> Result<()>;
```

### Data Model (excerpt)
Same `trash_entries` row shape as US-0001 (§8 of the spec). This story is the first writer:
`original_path` = the pre-delete path, `deleted_by` = the calling person, `size`/`kind` = the moved
node's own fields, written via US-0001's `insert_trash_entry` trait method inside the **same
transaction** as the `rename` (see NFR below).

### Decisions That Govern This Story
- **DEC-001** (§17): the row is written explicitly rather than reconstructed later — this story is
  one of its two "Implemented by" requirements (FR-NEW-002), together with US-0003 (FR-NEW-003).
- **DEC-002** (§17): "Trash-write collisions retry with `~N`; restore-destination collisions retry
  with `_restoredN` and never fail... the two suffix schemes are kept distinct (one
  internal/write-side, one user-facing/restore-side) rather than unified, since they solve different
  problems at different layers." This story implements the **write-side** (`~N`) half only; the
  restore-side (`_restoredN`) half is US-0005's (FR-NEW-010) — do not implement it here.
- **DEC-008** (§17): "`fs.trash_list`'s collision-exhaustion and unrelated errors reuse
  `ERR_INTERNAL_ERROR`; no new `ERR_*` code is added." Despite the decision's own wording naming
  `fs.trash_list`, its `Implemented by: FR-NEW-005` ties it to the exhaustion case this story
  implements — exhaustion returns `ERR_INTERNAL_ERROR`, not a new code.

### Applicable NFRs
- **7.4 Reliability** (§7.4): the `trash_entries` insert and the `rename` commit or roll back
  together — use the transaction handle, do not call US-0001's insert method after the rename has
  already committed outside a shared transaction.
- **7.5 Observability** (§7.5): "the `trash_entries` write/delete at DEBUG (file mutations), matching
  the existing tracing levels for `fs.delete`." Trace the insert at the same level and in the same
  manner `delete_path` already traces its other mutations.

### Bounded Context
**Trash tracking** and **File lifecycle** (§4.5): this story is where those two contexts meet —
`fs.delete` (File lifecycle, pre-existing) now also populates `trash_entries` (Trash tracking, new).

## Functional Requirements

### FR-NEW-002: `fs.delete` writes a trash entry
- **EARS:** [EARS-E] "WHEN `fs.delete` soft-deletes a path (the `trash=true` branch of
  `core::fs_ops::delete_path`) THE system SHALL insert a `trash_entries` row with `original_path`
  equal to the pre-delete path, `deleted_by` equal to the calling person, `size`/`kind` taken from
  the moved node, in the same transaction as the `rename`."
- **Business Rules:** the hard-delete branch (`trash=false`) SHALL NOT write a row (FR-NEW-013,
  below, states this as its own prohibition).

### FR-NEW-004: trash-path write collision retry
- **EARS:** [EARS-E] "WHEN a `rename` into a computed trash destination returns `ERR_NO_CLOBBER` THE
  system SHALL retry the same operation at the destination suffixed with `~1`, then `~2`,
  incrementing up to `~50`, until the rename succeeds."
- **Business Rules:** the retry loop SHALL only trigger on `ERR_NO_CLOBBER`; any other error from
  `rename` SHALL propagate immediately without retry (FR-NEW-006).
- **Exact names:** `pub(crate) async fn rename_with_collision_retry(client: &VolumeClient, src: &str,
  dst: &str) -> Result<String>` (returning the final, possibly-suffixed destination path), in
  `crates/core/src/core/fs_ops.rs`, called directly by `delete_path` and by
  `purge::sweep_project_files` (US-0003) — one implementation, two callers.

### FR-NEW-005: collision retries exhausted
- **EARS:** [EARS-O] "IF 50 consecutive `~N` retries all still return `ERR_NO_CLOBBER` THEN THE
  system SHALL return `ERR_INTERNAL_ERROR` and leave the original file live and untouched."

### FR-NEW-006: no retry on unrelated errors
- **EARS:** [EARS-UB] "The system SHALL NOT retry a trash-destination rename when the underlying
  error is anything other than `ERR_NO_CLOBBER`."

### FR-NEW-013: hard delete never writes a trash entry
- **EARS:** [EARS-UB] "The system SHALL NOT write a `trash_entries` row when `fs.delete` is called
  with `trash=false` (the hard-delete branch)."
- **Business Rules:** keeps the existing `hard_delete_works_when_allowed` test (`fs_ops.rs:2711`)
  passing unmodified.

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| mocked/instrumented volume client | for call-count and injected-error assertions (E2E-441/449/450) | test double, same pattern as existing `fs_ops.rs` tests | ready |
| pinned clock seam | equivalent to `sweep_project_files_at`'s explicit `now` param | existing project pattern | ready |

### E2E-NEW-401: `fs.delete` creates a trash entry
- **Category:** Happy
- **Scenario:** SC-003 (write path shared with SC-002's delete step)
- **Requirements:** FR-NEW-001 (persistence target), FR-NEW-002
- **Preconditions:** project `proj-trash-1` exists, member `alice`, file `/a.txt` written with
  content `"x"`.
- **Steps:** Given the file above exists / When `alice` calls `fs.delete(mount_id="proj-trash-1",
  path="/a.txt", recursive=false, trash=true)` / Then the response is `{"path": "/a.txt", "trashed":
  true, "trash_path": "/.mcp_trash/{epoch_ms}__a.txt"}` / And a DB query `SELECT * FROM
  trash_entries WHERE volume_id=? AND trash_path=?` returns exactly one row with
  `original_path="/a.txt", size=1, kind="file", deleted_by="alice"`.
- **Priority:** Critical

### E2E-NEW-402: `deleted_by` reflects the actual caller
- **Category:** Side Effect
- **Scenario:** cross
- **Requirements:** FR-NEW-002
- **Preconditions:** same as E2E-NEW-401, delete performed by `bob`.
- **Steps:** Given file `/a.txt` exists / When `bob` calls `fs.delete(mount_id="proj-trash-1",
  path="/a.txt", trash=true)` / Then the `trash_entries` row has `deleted_by="bob"` exactly (DB
  query).
- **Priority:** High

### E2E-NEW-404: flatten/timestamp collision retries at `~1`
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Preconditions:** project `proj-trash-1`, files `/d/a.txt` and `/d__a.txt` both exist; both
  deletes forced to compute the identical trash destination `/.mcp_trash/{same_ms}__d__a.txt` via
  the pinned clock seam.
- **Steps:** Given both files exist and the clock is pinned / When `/d/a.txt` is deleted, then
  `/d__a.txt` is deleted at the same instant / Then the first succeeds at
  `/.mcp_trash/{ts}__d__a.txt`; the second, after `ERR_NO_CLOBBER`, succeeds at
  `/.mcp_trash/{ts}__d__a.txt~1` / And both have correct, distinct `trash_entries` rows with correct
  `original_path` (DB query).
- **Priority:** Critical

### E2E-NEW-405: collision retries exhausted
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005
- **Preconditions:** `/.mcp_trash/{ts}__x` through `/.mcp_trash/{ts}__x~50` all pre-exist as dummy
  nodes (seeded directly via the volume client).
- **Steps:** Given the 51 pre-occupied destinations / When a delete that would collide at
  `/.mcp_trash/{ts}__x` is attempted / Then the call returns `ERR_INTERNAL_ERROR`, completing within
  1 second (bounding an infinite loop) / And the original file `/x` remains live, untouched
  (filesystem/node check).
- **Priority:** Low

### E2E-NEW-441: retry fires only on `ERR_NO_CLOBBER`
- **Category:** Failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-006
- **Preconditions:** a `rename` call mocked to return a non-collision error (e.g. a simulated volume
  I/O error) on the first attempt at the trash destination.
- **Steps:** Given the mocked non-collision failure / When a delete triggers the trash-path rename /
  Then the error surfaces immediately with no `~1` retry attempted (mock call-count assertion: the
  rename function is called exactly once).
- **Priority:** High

### E2E-NEW-443: the retry counter advances past `~1` when more than one slot is occupied
- **Category:** Happy
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Preconditions:** the real file `/f.txt` exists. Both `/.mcp_trash/{ts}__f.txt` (bare destination)
  and `/.mcp_trash/{ts}__f.txt~1` are pre-seeded as occupied dummy nodes, at pinned instant `{ts}`.
- **Steps:** Given `/f.txt` exists and both the bare destination and its `~1` slot are already
  occupied / When `/f.txt` is deleted with the clock pinned to `{ts}` / Then the delete succeeds at
  `/.mcp_trash/{ts}__f.txt~2` (tool response `trash_path` field), skipping both occupied slots in
  order / And the `trash_entries` row for `/f.txt` records `trash_path` ending in `~2` (DB query).
- **Priority:** High

### E2E-NEW-444: collision retry succeeds on the 49th attempt (one before exhaustion)
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Preconditions:** `/.mcp_trash/{ts}__y` through `/.mcp_trash/{ts}__y~48` pre-exist (49 occupied
  destinations, one fewer than E2E-NEW-405's 50).
- **Steps:** Given the 49 pre-occupied destinations / When a delete that would collide at
  `/.mcp_trash/{ts}__y` is attempted / Then it succeeds at `/.mcp_trash/{ts}__y~49` (tool response
  check), distinguishing the last-successful boundary from E2E-NEW-405's first-exhausted boundary.
- **Priority:** Low

### E2E-NEW-446: trashing a directory writes exactly one row
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-001 (verifying the persistence target's granularity)
- **Preconditions:** project `proj-restore-1`, directory `/big/` containing 10 descendant files.
- **Steps:** Given the 10-file directory / When `fs.delete(path="/big", recursive=true, trash=true)`
  is called / Then exactly one `trash_entries` row exists for `/big`'s trash path (DB query:
  `SELECT COUNT(*)` = 1), not one per descendant.
- **Priority:** Low

### E2E-NEW-448: no permanent poison after exhaustion
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005
- **Preconditions:** state immediately after E2E-NEW-405's exhaustion (50 colliding destinations
  still occupied).
- **Steps:** Given the exhausted state / When one of the 50 occupying dummy nodes is removed, then
  the same delete is retried / Then it now succeeds at the freed `~N` slot — confirming exhaustion is
  not a permanent failure mode for that path.
- **Priority:** Low

### E2E-NEW-449: unrelated error on a later retry attempt still aborts without further retry
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-006
- **Preconditions:** the first `~1` retry attempt is mocked to return a non-collision error, after an
  initial genuine `ERR_NO_CLOBBER` on the bare destination.
- **Steps:** Given a real collision on attempt 0 and a mocked unrelated failure on attempt 1 (`~1`) /
  When the delete is attempted / Then the unrelated error surfaces immediately after exactly one
  retry (`~1`), with no `~2` attempt made (mock call-count assertion).
- **Priority:** Low

### E2E-NEW-450: a non-colliding delete never invokes the retry path
- **Category:** Happy
- **Scenario:** cross
- **Requirements:** FR-NEW-006
- **Preconditions:** a normal delete with no pre-existing trash destination collision.
- **Steps:** Given no collision exists / When the file is deleted / Then `rename` into the trash
  destination is called exactly once (mock call-count assertion), confirming the retry path is
  dormant on the ordinary path.
- **Priority:** Low

### E2E-NEW-453: hard-deleting a file writes no trash entry
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Preconditions:** server configured with `allow_hard_delete=true`, file `/h.txt` exists.
- **Steps:** Given the file and hard-delete allowed / When `fs.delete(mount_id, path="/h.txt",
  trash=false)` is called / Then the file is gone (`hard_delete_works_when_allowed` behavior,
  unchanged) / And a DB query for any `trash_entries` row referencing `/h.txt` or its would-be
  trash path returns zero rows.
- **Priority:** Critical

### E2E-NEW-454: hard-deleting a directory subtree writes no trash entries
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Preconditions:** server configured with `allow_hard_delete=true`, directory `/hd/` with 3
  descendant files.
- **Steps:** Given the directory and hard-delete allowed / When `fs.delete(mount_id, path="/hd",
  recursive=true, trash=false)` is called / Then the whole subtree is gone / And `SELECT COUNT(*)
  FROM trash_entries WHERE volume_id=?` is unchanged from before the call (DB query) — zero rows
  added.
- **Priority:** High

### E2E-NEW-456: hard-deleting a file does not disturb an unrelated prior trash entry
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Preconditions:** server configured with `allow_hard_delete=true`. File `/k.txt` soft-deleted
  then restored once (leaving no `trash_entries` row — this precondition is only fully realizable
  once US-0005 exists; until then, simulate it by inserting then deleting a `trash_entries` row
  directly via US-0001's trait methods), then a new unrelated file `/m.txt` is created and
  hard-deleted.
- **Steps:** Given the prior soft-delete/restore cycle on `/k.txt` left zero rows, and `/m.txt` is
  now hard-deleted / When `fs.delete(mount_id, path="/m.txt", trash=false)` is called / Then
  `SELECT COUNT(*) FROM trash_entries WHERE volume_id=?` is `0` before and after (DB query) — the
  hard delete neither creates a row for `/m.txt` nor resurrects/affects any row related to `/k.txt`'s
  earlier, already-closed lifecycle.
- **Priority:** Low

### E2E-NEW-458: soft-deleting a directory records `kind` and `size` correctly
- **Category:** Happy
- **Scenario:** cross
- **Requirements:** FR-NEW-002
- **Preconditions:** project `proj-trash-1`, directory `/dirx/` with known aggregate node size.
- **Steps:** Given the directory / When `fs.delete(path="/dirx", recursive=true, trash=true)` is
  called / Then the resulting `trash_entries` row has `kind="dir"` and `size` matching the directory
  node's own recorded size field (DB query) — distinct from E2E-NEW-446, which only checks the row
  count, not these field values.
- **Priority:** Low

## Constraints

### Files Not to Touch
- `crates/core/src/purge.rs` — US-0003.
- `crates/core/src/storage/traits.rs` / `rel/*` — already built in US-0001; call its methods, don't
  redefine them.

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not duplicate the collision-retry loop inside `delete_path` itself — it must be the shared
  `rename_with_collision_retry` free function, callable by US-0003 unchanged.
- Do not catch and swallow a non-`ERR_NO_CLOBBER` error inside the retry loop "just in case" — FR-006
  is a strict prohibition.

### Scope Boundary
- No MCP tool changes in this story (that's US-0004/US-0005). `delete_path` is the only call site
  modified.

## Non Regression

### Existing Tests That Must Pass
- `delete_soft_moves_into_the_trash` (`fs_ops.rs:2689`), `hard_delete_is_refused_unless_configured`
  (`fs_ops.rs:2702`), `hard_delete_works_when_allowed` (`fs_ops.rs:2711`),
  `delete_a_directory_needs_recursive` (`fs_ops.rs:2726`), `delete_a_missing_path_is_not_found`
  (`fs_ops.rs:2735`), `soft_delete_of_a_directory_moves_the_whole_subtree` (`fs_ops.rs:2815`) — all
  stay green unmodified.

### Behaviors That Must Not Change
- `fs.delete`'s existing response shape (`{"path", "trashed", "trash_path"}`) is unchanged — the
  `trash_entries` write is a side effect, not a response-shape change.

### API Contracts to Preserve
- `TOOL_CONTRACT.txt:101-105`'s `fs.delete` schema and its sample output (`TOOL_CONTRACT.txt:600`)
  are unchanged by this story.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
