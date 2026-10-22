# US-0076: exact case for copy and recursive reads

> Spec: SPEC-0050 (sdlc/specs/SPEC-0050-case-sensitive-subtree-queries/)
> Nature: BUG
> Priority: P2
> Depends on: US-0074
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Copying a folder, searching it by name or content, exporting it and downloading it cover that folder and what lies below it and nothing else. The shared subtree read and the folder breakdown of SPEC-0049 use the case exact clause.

## Requirements covered
- SPEC-0050/BR-004: WHEN a folder is copied recursively THE product SHALL copy that folder and the files and folders below it, and no other path.
- SPEC-0050/BR-005: The product SHALL limit a search by name, a search in file content, a zip export and a download of a folder to that folder and the files and folders below it, comparing letter case exactly.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::case_paths_e2e`: BT-004 copies `/video` to `/vcopy` and `/vcopy/x.txt` does not exist; BT-005 returns `/video/y.txt` only from the glob and no match from the grep.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0050): the exact test is a substring equality, not a range comparison.
- DEC-005 (SPEC-0050): `Dialect` renders the engine specific parts (`substr_fn`, `char_len`, `exact_equals`); the prefix length is written into the SQL as an integer literal, never as user text.
- DEC-009 (SPEC-0050): `folder_items` (SPEC-0049) switches to the helper and keeps its Rust `strip_prefix`, which buckets the rows anyway.
- DEC-010 (SPEC-0050): tests are functional e2e on the tool methods for user visible operations, an integration test for the purge, plus a dialect unit test and a conformance case as extras.
### Data model
None. No schema change, no migration.
### Contracts
Uses the helpers of US-0074. `subtree` (the shared read behind copy, walk, glob, grep, export and download) selects `WHERE volume_id=?1 AND (path=?2 OR <clause on path>) ORDER BY path` with `SubtreeMatch::new(dialect, root)`; `folder_items` (SPEC-0049) renders its file query with the clause on `path` and keeps its Rust `strip_prefix`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`subtree`, `folder_items`) |
| `crates/core/src/tools/case_paths_e2e.rs` | modify (add tests) |
### Existing patterns
- Escaped prefix today: `descendant_pattern` at `crates/core/src/storage/meta.rs:574`; a query shaped like the permanent removal:
```rust
"SELECT sha256, kind, size FROM nodes WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')"
```
- Test harness: `World`, `seed`, `ok_json`, `err_text` in `crates/core/src/tools/disk_usage_e2e.rs` / `folder_usage_e2e.rs` (copy what you need into `case_paths_e2e.rs`; the modules are private). Delete/trash/restore helpers: `delete`, `trashed` in `folder_usage_e2e.rs`.
- Callers of the shared read: `volume.rs:179` (`walk`, behind `fs.glob`, `fs.grep`, the zip export and the download), `fs_ops.rs:1358` (recursive copy), `volume.rs:278` (`copy_tree`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `P` owned by `o@x.com`, which can delete permanently; files `/Video/x.txt` containing `upper` and `/video/y.txt` containing `lower`, 10 bytes in all. All tests drive the real tool methods (the assistant interface) over real storage.

### SPEC-0050/BT-004: a copy takes only the exact folder
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-004
- Given the shared setup / When the owner copies `/video` to `/vcopy` recursively / Then `/vcopy/y.txt` reads `lower`; `/vcopy/x.txt` does not exist; `/Video/x.txt` and `/video/y.txt` are unchanged

### SPEC-0050/BT-005: recursive reads stay inside the folder
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-005
- Given the shared setup / When the owner runs `fs.glob` with `root` `/video` and pattern `*.txt`, then `fs.grep` with `root` `/video` and pattern `upper`, then `fs.export_zip` with `paths` `["/video"]` and reads the zip names / Then the glob returns `/video/y.txt` only; the grep returns no match; the zip holds `y.txt` only

## Constraints
- Files not to touch: `volume.rs`, `fs_ops.rs`, `export.rs`, `dataplane.rs` (they call the store; the fix is below them).
- Dependencies not to add: none.
- Patterns to avoid: filtering the walked paths in the callers.
- Scope boundary: trash listing and purge (US-0077).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (BT-004 fails because `/vcopy/x.txt` exists; BT-005 fails because the glob returns `/Video/x.txt`, the grep matches `upper` and the zip holds `x.txt`).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: US-0074 and US-0075 tests stay green; `spec_0049_us_0073_folders_differing_only_by_case_do_not_mix` and the 41 folder usage tests stay green.
- [ ] Docs updated: none in this story
- [ ] Self review: full; time and space of the changed query paths stated; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0076: exact case for copy and recursive reads`, body `Spec: SPEC-0050` and the requirement ids.
