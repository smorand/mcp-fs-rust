---
id: BL-0011
title: Make the trash_entries write truly atomic with the rename it accompanies
kind: debt
suggested_command: /spec-debt
created: 2026-10-06
origin: SPEC-0011 US-0002, Phase 4.6 CONVERGE gap, accepted by user decision 2026-10-06
---

## What
`fs.delete`'s soft-delete path (and `purge::sweep_project_files`) renames the file into the
trash directory via `rename_with_collision_retry`, then writes its `trash_entries` row via a
**separate** transaction (`VolumeClient::trash`'s `record_trash_entry`,
`crates/core/src/storage/meta.rs:270-314`). `MetaBackend::rename` commits on its own inside the
trait-object boundary (`Arc<dyn MetaBackend>`), so the rename and the trash-entry insert cannot
currently share one transaction without either widening `MetaBackend` with a new dialect-agnostic
method that performs both operations atomically, or giving `VolumeClient` dialect-specific
knowledge it was deliberately designed not to have.

Add a `MetaBackend` method (implemented once in the generic relational `impl`, per this project's
"a store never speaks a driver" convention) that performs the rename and the trash-entry insert as
one transactional unit, and have `rename_with_collision_retry` call it instead of the current
two-call sequence.

## Why
`SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md`'s `FR-NEW-002` requires the
`trash_entries` insert to run "in the same transaction as the `rename`" (NFR 7.4 states the same:
"a failed rename never leaves an orphaned `trash_entries` row, and a failed insert never leaves an
untracked rename"). The shipped implementation does not meet that literal text: if the process
crashes between the rename's commit and the trash-entry insert's commit, the file is live in the
trash directory but untracked in `trash_entries` until FR-NEW-011's lazy backfill (built for
exactly the "legacy untracked trash node" case) picks it up on next `fs.trash_list`/
`fs.trash_restore` access, with a best-effort `deleted_at` instead of the exact original time.

Caught by the Phase 4.6 CONVERGE gate when implementing SPEC-0011 (every other requirement,
FR-NEW-001/003-018, passed cleanly; this was the one gap). Discovered and documented in detail,
with code citations, at
`specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/drift/2026-10-06_11-29-34.md`.
Decision: accept the current best-effort-sequential behavior as shipped (the blast radius is a
sub-millisecond crash window, self-healed by the backfill mechanism), and track the proper fix
here rather than block the lot on it.

## Notes
- The fix touches `MetaBackend` (`crates/core/src/storage/traits.rs`) for every dialect
  (sqlite/postgres/sqlserver), since the new method must be implemented once in the generic
  relational `impl`, not per-dialect.
- `purge::sweep_project_files` already compensates for a failed trash-entries insert by renaming
  the file back (added in US-0003, see `e2e_new_445_a_failed_trash_entries_insert_leaves_the_file_live`)
  — that is a different, already-correct behavior (insert failure, not crash-mid-window) and is not
  what this entry is about.
- Evidence: `crates/core/src/storage/meta.rs:270-314` (`record_trash_entry`'s own doc comment
  admits the sequential-not-joint shape), `crates/core/src/core/fs_ops.rs:1006-1031`
  (`rename_with_collision_retry`), the drift file above.
