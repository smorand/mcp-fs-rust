# US-0075: exact case for move, rename and the trash

> Spec: SPEC-0050 (sdlc/specs/SPEC-0050-case-sensitive-subtree-queries/)
> Nature: BUG
> Priority: P1
> Depends on: US-0074
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Moving or renaming a folder, and deleting it to the trash, take that folder and what lies below it and nothing else, even when another folder differs from it only by letter case.

## Requirements covered
- SPEC-0050/BR-002: WHEN a folder is deleted to the trash THE product SHALL move that folder and the files and folders below it, and no other path, and SHALL keep the others where they are.
- SPEC-0050/BR-003: WHEN a folder is moved or renamed THE product SHALL move that folder and the files and folders below it, and no other path.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::case_paths_e2e`: BT-003 moves `/video` to `/clip` and `/Video/x.txt` stays and reads `upper`; BT-002 shows exactly one trash entry for `/video`.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0050): decide the match in SQL, not in Rust (one `DELETE`, exact paged totals, `stale_files` stops hiding rows).
- DEC-005 (SPEC-0050): `Dialect` renders the engine specific parts (`substr_fn`, `char_len`, `exact_equals`); the prefix length is written into the SQL as an integer literal, never as user text.
- DEC-010 (SPEC-0050): tests are functional e2e on the tool methods for user visible operations, an integration test for the purge, plus a dialect unit test and a conformance case as extras.
### Data model
None. No schema change, no migration.
### Contracts
Uses the helpers of US-0074 (`SubtreeMatch`, `clause`). `tx_rename` takes `&SubtreeMatch` instead of the `pattern: &str`; `rename` and `rename_into_trash` build it with `SubtreeMatch::new(dialect, src)`. The `SELECT path FROM nodes WHERE volume_id=?1 AND (path=?2 OR <clause on path>) ORDER BY <len>(path)` binds `src`, `like`, `prefix`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`tx_rename`, `rename`, `rename_into_trash`) |
| `crates/core/src/tools/case_paths_e2e.rs` | modify (add tests) |
### Existing patterns
- Escaped prefix today: `descendant_pattern` at `crates/core/src/storage/meta.rs:574`; a query shaped like the permanent removal:
```rust
"SELECT sha256, kind, size FROM nodes WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')"
```
- Test harness: `World`, `seed`, `ok_json`, `err_text` in `crates/core/src/tools/disk_usage_e2e.rs` / `folder_usage_e2e.rs` (copy what you need into `case_paths_e2e.rs`; the modules are private). Delete/trash/restore helpers: `delete`, `trashed` in `folder_usage_e2e.rs`.
- The rename loop: `crates/core/src/storage/meta.rs:1591-1650` (`tx_rename`, selects the paths then updates one row at a time; the new path is `dst + old[src.len()..]`, so a wrong row selected is renamed with a wrong suffix).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `P` owned by `o@x.com`, which can delete permanently; files `/Video/x.txt` containing `upper` and `/video/y.txt` containing `lower`, 10 bytes in all. All tests drive the real tool methods (the assistant interface) over real storage.

### SPEC-0050/BT-002: a delete to the trash moves only the exact folder
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-002
- Given the shared setup / When the owner deletes `/video` recursively to the trash / Then `/Video/x.txt` is still at its place and reads `upper`; the trash list has exactly 1 entry, with original path `/video` (kind folder); the trash folder holds `y.txt` and no `x.txt`

### SPEC-0050/BT-003: a move takes only the exact folder
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-003
- Given the shared setup / When the owner moves `/video` to `/clip` / Then `/clip/y.txt` reads `lower`; `/clip/x.txt` does not exist; `/Video/x.txt` is still at its place and reads `upper`; `/video` no longer exists

### SPEC-0050/BT-008: nested and accented differences
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-001, BR-003
- Given `/a/Docs/p.txt` (`P`), `/a/docs/q.txt` (`Q`), `/École/e.txt` and `/école/f.txt` / When the owner deletes `/a/docs` permanently and moves `/école` to `/ecole` / Then `/a/Docs/p.txt` reads `P`; `/École/e.txt` is untouched; `/ecole/f.txt` exists and `/ecole/e.txt` does not

## Constraints
- Files not to touch: the permanent removal (done), `subtree`, `stale_files`, trash entries.
- Dependencies not to add: none.
- Patterns to avoid: changing the per row update; the length of `src` used for the suffix stays bytes (`src.len()`), because the rows kept are exact prefixes.
- Scope boundary: copy and reads (US-0076), trash listing and purge (US-0077).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (BT-002 fails because the trash list has 2 entries where 1 is expected; BT-003 fails because `/clip/x.txt` exists; BT-008 fails on its move part: `/ecole/e.txt` exists).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: US-0074 tests stay green; the restore from the trash and `SPEC-0044` atomic trash write tests stay green.
- [ ] Docs updated: none in this story
- [ ] Self review: full; time and space of the changed query paths stated; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0075: exact case for move, rename and the trash`, body `Spec: SPEC-0050` and the requirement ids.
