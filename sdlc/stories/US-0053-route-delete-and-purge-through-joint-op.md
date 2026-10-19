# US-0053: route soft delete and purge through the joint operation

> Spec: SPEC-0044 (sdlc/specs/SPEC-0044-atomic-trash-entry-write/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0052
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
`fs.delete` (soft) and the auto purge sweep stop doing rename then insert and use `rename_into_trash`, so a crash can no longer leave a trashed file without its row. The purge rename back is deleted.

## Requirements covered
- SPEC-0044/DR-002: `rename_with_collision_retry` SHALL take `trash: Option<&TrashEntry>`; WHEN it is `Some` each attempt (`dst`, `dst~1` ... `dst~50`) SHALL call `VolumeClient::rename_into_trash`, otherwise `VolumeClient::rename`; `delete_path_at` and `sweep_project_files` SHALL pass `Some` exactly when `client.trash.is_some()` and SHALL NOT call `record_trash_entry`.
- SPEC-0044/DR-003: `sweep_project_files` SHALL NOT issue a `rename` back to the original path after a failed trash entry insert; the transaction rollback leaves the file at its original path.
- SPEC-0044/DR-006: IF the destination is occupied THEN the transaction of that attempt SHALL roll back with `ERR_NO_CLOBBER`, leaving no node and no row at that `dst`, and `rename_with_collision_retry` SHALL retry at the next suffix up to `~50`; any other error SHALL propagate with no retry; exhaustion SHALL return `ERR_INTERNAL_ERROR` with `src` untouched and zero rows.

## Independent Test
`make test`: DT-002, DT-007 to DT-012 green, and the unmodified E2E-NEW-401 to 458 stay green. DT-010 checks the file is live with its original `rev` and nothing sits under the trash directory after a faulted sweep, read through the node tree rather than the return value.

## Design excerpt
### Decisions
- DEC-001, DEC-002 (SPEC-0044): `VolumeClient::rename_into_trash` delegates to `self.meta.rename_into_trash`, never `self.trash`; `self.trash` is only the presence flag deciding whether `Some(&TrashEntry)` is passed (behavior unchanged for clients built without a trash store).
### Contracts
```rust
// volume.rs, next to rename (:208)
pub async fn rename_into_trash(&self, src: &str, dst: &str, entry: &TrashEntry) -> Result<()> {
    self.meta.rename_into_trash(src, dst, entry).await
}
// fs_ops.rs (:1171)
pub(crate) async fn rename_with_collision_retry(
    client: &VolumeClient, src: &str, dst: &str, trash: Option<&TrashEntry>,
) -> Result<String>
```
Inside the function, replace both `client.rename(src, X)` calls with a local helper that picks `client.rename_into_trash(src, X, e)` when `trash` is `Some(e)`. `NO_CLOBBER`-only retry logic stays as is.
Callers: `delete_path_at` (`fs_ops.rs:1264-1279`) builds `TrashEntry { original_path: norm.into(), size: node.size, kind: node.kind.clone(), deleted_at: crate::util::now_iso(), deleted_by: Some(person.into()) }` before the move and passes `client.trash.as_ref().map(|_| &entry)`; delete the `record_trash_entry` block. `sweep_project_files` (`purge.rs:89-107`) does the same with `deleted_by: None` and deletes the `record_trash_entry` call and the `client.rename(&final_dst, &node.path)` rename back.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/volume.rs` | add `rename_into_trash` |
| `crates/core/src/core/fs_ops.rs` | signature, callers; test callers `:3306`, `:3393`, `:3409` add `None`; DT-002, DT-007, DT-008, DT-009 |
| `crates/core/src/purge.rs` | migrate sweep; rewrite E2E-NEW-445 as DT-010; DT-011 |
| `crates/core/src/tools/trash.rs` | DT-012 (assertion on backfill) |
### Existing patterns
E2E-NEW-404 (`fs_ops.rs`, `delete_path_at(NOW, ...)` with a pinned `NOW`) forces a deterministic collision; E2E-NEW-405 pre occupies 51 slots; E2E-NEW-449 uses `ScriptedRenameMeta`'s script.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0044/DT-002: one transaction per joint call
- Integration, `fs_ops.rs` tests. A counting `RelationalDb` wraps `meta`'s db; reset the counter right before `VolumeClient::rename_into_trash`, read right after. Then exactly 1 `begin()`, and both `UPDATE nodes` and `INSERT INTO trash_entries` ran inside it.
### SPEC-0044/DT-007: collision rolls back then retries
- Given `NOW` pinned, `/d/a.txt` and `/d__a.txt` deleted at the same instant. Then the second lands at `{dst1}~1` with one row keyed `{dst1}~1`, and no row or node was left at `dst1` by the failed attempt other than the first file's own.
### SPEC-0044/DT-008: exhaustion
- 51 occupied slots (as E2E-NEW-405): `ERR_INTERNAL_ERROR`, `/x` live, `trash_entries` count unchanged.
### SPEC-0044/DT-009: non collision error aborts the retry loop
- `ScriptedRenameMeta` scripted `[Some(NO_CLOBBER), Some(INTERNAL_ERROR)]` with an entry: `ERR_INTERNAL_ERROR`, exactly 2 calls, 0 rows.
### SPEC-0044/DT-010: purge atomicity
- Given the failing db behind the real `meta` (armed on `INSERT INTO trash_entries`) and a stale `/stale2.txt`. Then `sweep_project_files` returns 0, the file exists with its original `rev`, 0 rows, no node under the trash directory.
### SPEC-0044/DT-011: purge row shape
- A sweep of a stale file writes a row with `deleted_by == None`, same fields as E2E-NEW-403/440.
### SPEC-0044/DT-012: legacy backfill unchanged
- A trash node with no row (created with `meta.rename` directly) is listed by `trash_list` and gets a row.

## Constraints
- Files not to touch: `meta.rs`, tool schemas, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: any `record_trash_entry` call in production; a second transaction around a move.
- Scope boundary: `cfg(test)` on `record_trash_entry` and docs belong to US-0054.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test green; `make check` green; main stays green.
- [ ] Non regression: the existing suite passes unmodified; exceptions (3 mechanical `None` arguments at `fs_ops.rs:3306,3393,3409` and E2E-NEW-445 rewritten as DT-010 with the same asserted outcome) are the only edits; 4 total.
- [ ] Docs updated: none in this story.
- [ ] Self review: no `unwrap` in production; client without trash store still uses plain `rename`; no dash punctuation.
- [ ] One commit `US-0053: route soft delete and purge through the joint operation`, body `Spec: SPEC-0044`, DR-002, DR-003, DR-006.
