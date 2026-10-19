# US-0052: add the joint rename and trash entry operation

> Spec: SPEC-0044 (sdlc/specs/SPEC-0044-atomic-trash-entry-write/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
Add `MetaBackend::rename_into_trash`, a single relational transaction that moves a path and inserts its `trash_entries` row. Nothing calls it yet in production; this story builds and proves the capability.

## Requirements covered
- SPEC-0044/DR-001: The metadata store SHALL expose `MetaBackend::rename_into_trash(&self, src: &str, dst: &str, entry: &TrashEntry) -> Result<()>` that renames `src` to `dst` and inserts the `trash_entries` row for `dst` in ONE relational transaction, implemented once in the relational `impl` and not per dialect.
- SPEC-0044/DR-007: The system SHALL make a failure of the row insert undo the rename, and a failure of the rename (including `ensure_storable_path` on a descendant) leave no row, within one transaction.

## Independent Test
`make test` runs DT-003 to DT-006 and DT-013 green; each fails to compile or fails before the production code exists. DT-003 reads the node tree and `trash_entries` through the store after a faulted call: the rename did not happen and no row exists.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0044): new trait method rather than a `VolumeClient` transaction; a store never speaks a driver.
- DEC-004 (SPEC-0044): the trait method is required, no default, so a store cannot silently skip the row.
### Contracts
```rust
// crates/core/src/storage/traits.rs
pub struct TrashEntry {
    pub original_path: String,
    pub size: i64,
    pub kind: String,
    pub deleted_at: String,
    pub deleted_by: Option<String>,
}
// in trait MetaBackend, after `rename`:
async fn rename_into_trash(&self, src: &str, dst: &str, entry: &TrashEntry) -> Result<()>;
```
The row's `trash_path` is `dst`. In `meta.rs` the body of `rename` (inside `run_retrying`, `meta.rs:1984-2037`) moves into `async fn tx_rename(tx: &mut dyn RelationalTx, dialect: Dialect, volume: &str, src: &str, dst: &str, pattern: &str) -> Result<()>`; `rename` calls it, `rename_into_trash` calls it then `insert_trash_entry(tx, &volume, dst, &entry.original_path, entry.size, &entry.kind, &entry.deleted_at, entry.deleted_by.as_deref())` (`meta.rs:1406`), in the same `run_retrying` closure. `run_retrying` re runs the closure, so the work is idempotent by construction (the tx rolls back on `Err`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/traits.rs` | add `TrashEntry` and the method |
| `crates/core/src/storage/meta.rs` | factor `tx_rename`, implement the method in `impl MetaBackend for RelationalMetaStore` (`:1644`) |
| `crates/core/src/storage/conformance.rs` | DT-013 |
| `crates/core/src/purge.rs` | doubles `AlwaysCollideMeta` (`:937`) and `FailingRenameMeta` (`:1578`) implement the method: same interception as their `rename`, else `inner.rename_into_trash`; DT-003, DT-004, DT-005, DT-006 (next to `FailTrashInsertOnceDb`, `:1016`) |
| `crates/core/src/core/fs_ops.rs` | double `ScriptedRenameMeta` (`:3170`) implements the method the same way; update its doc comment `:3160` |
### Existing patterns
`FailTrashInsertOnceDb` / `FailTrashInsertOnceTx` (`purge.rs:1016-1060`) wrap a `RelationalDb` and fail a statement containing `INSERT INTO trash_entries` once. Generalize this wrapper so DT-004 can arm a failure on `UPDATE nodes` instead (a second armed substring), without changing its current behavior.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0044/DT-003: insert failure undoes the rename
- Level / Driver / Location: integration, direct call, `purge.rs` tests
- Given `/a.txt` exists, `meta` opened over a `FailTrashInsertOnceDb`-wrapped db (armed)
- When `meta.rename_into_trash("/a.txt", "/.mcp_trash/x", &entry)` runs
- Then `Err`; `/a.txt` exists; `/.mcp_trash/x` does not; `count_trash_entries_for_test()` is 0
### SPEC-0044/DT-004: rename failure leaves no row
- Given `/d` with 3 files, db wrapper failing the `UPDATE nodes` statement on its second execution
- When `rename_into_trash("/d", "/.mcp_trash/d", &entry)` runs
- Then `Err`; all four paths still under `/d`; nothing under `/.mcp_trash`; 0 rows. (The `ensure_storable_path` ceiling variant, `meta.rs:154`, is SQL Server only and runs in its opt in suite.)
### SPEC-0044/DT-005: success is visible together
- Given `/a.txt` (size 1) and entry `{original_path: "/a.txt", size: 1, kind: "file", deleted_at: "2026-10-19T00:00:00Z", deleted_by: Some("ada")}`
- When the call succeeds Then `get(src)` is none, `get(dst)` is some, `get_trash_entry_for_test(dst)` returns that exact entry.
### SPEC-0044/DT-006: directory move
- Given `/d` with 3 files; When moved to `/.mcp_trash/d` with `kind: "dir"` Then all four paths exist under `dst`, exactly 1 row, `kind == "dir"`.
### SPEC-0044/DT-013: conformance on every engine
- Given the shared suite in `conformance.rs`, When `rename_into_trash` runs on success Then node and row present; When `dst` is occupied Then `ERR_NO_CLOBBER`, `src` present, `dst` unchanged, no row for `dst`. SQLite always; PostgreSQL and SQL Server via their opt in suites.

## Constraints
- Files not to touch: `volume.rs`, production code of `delete_path_at` and `sweep_project_files`.
- Dependencies not to add: none.
- Patterns to avoid: a default body on the trait method; a driver type outside `storage/rel/`; `unwrap` in production.
- Scope boundary: no caller migration (US-0053), no `cfg(test)` on `record_trash_entry` (US-0054).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: the three test doubles gain a method (helper code, no assertion change), 0 test assertions edited.
- [ ] Docs updated: none in this story.
- [ ] Self review: `volume_id` in every `WHERE`; `tx_rename` behavior byte identical for `rename`; no dash punctuation in comments.
- [ ] One commit `US-0052: add the joint rename and trash entry operation`, body `Spec: SPEC-0044` and DR-001, DR-007.
