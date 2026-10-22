# US-0074: exact case for the permanent delete

> Spec: SPEC-0050 (sdlc/specs/SPEC-0050-case-sensitive-subtree-queries/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A permanent delete of a folder removes that folder and what lies below it and nothing else, even when another folder differs from it only by letter case. This story introduces the case exact clause (`Dialect` helpers and `SubtreeMatch`) and applies it to the permanent removal; it also pins that wildcard characters and prefix names stay untouched.

## Requirements covered
- SPEC-0050/BR-001: WHEN a folder is deleted permanently THE product SHALL remove that folder and the files and folders below it, and no other path, comparing letter case exactly.
- SPEC-0050/BR-008: The product SHALL NOT widen the set of paths of a folder operation through the characters `%` and `_` in a name.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::case_paths_e2e`: BT-001 deletes `/video` and reads `/Video/x.txt` back as `upper`; `cargo test -p mcp-fs-core --lib storage::rel::dialect` shows the three rendered clauses.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0050): do not use `PRAGMA case_sensitive_like` (deprecated, connection wide, SQLite only, would change the git hash lookup).
- DEC-003 (SPEC-0050): decide the match in SQL, not in Rust (one `DELETE`, exact paged totals, `stale_files` stops hiding rows).
- DEC-004 (SPEC-0050): the exact test is a substring equality, not a range comparison.
- DEC-005 (SPEC-0050): `Dialect` renders the engine specific parts (`substr_fn`, `char_len`, `exact_equals`); the prefix length is written into the SQL as an integer literal, never as user text.
- DEC-006 (SPEC-0050): on SQL Server the equality carries `COLLATE Latin1_General_100_BIN2` and the length counts UTF-16 units; unverified on a live server (BL-0073).
- DEC-010 (SPEC-0050): tests are functional e2e on the tool methods for user visible operations, an integration test for the purge, plus a dialect unit test and a conformance case as extras.
### Data model
None. No schema change, no migration.
### Contracts
Internal only, no tool, route or answer changes.
```rust
// storage/rel/dialect.rs
impl Dialect {
    pub fn substr_fn(self) -> &'static str;                  // "substr" | "SUBSTRING" (SQL Server)
    pub fn char_len(self, s: &str) -> usize;                 // chars (SQLite, PostgreSQL) | UTF-16 units (SQL Server)
    pub fn exact_equals(self, lhs: &str, rhs: &str) -> String; // "lhs = rhs" | "lhs = rhs COLLATE Latin1_General_100_BIN2" (SQL Server)
}
// storage/meta.rs
struct SubtreeMatch { like: String, prefix: String, prefix_len: usize }
impl SubtreeMatch {
    fn new(dialect: Dialect, path: &str) -> Self;   // like = descendant_pattern(path); prefix = path.trim_end_matches('/') + "/" (root: "/")
    fn clause(&self, dialect: Dialect, col: &str, like_ph: usize, prefix_ph: usize) -> String;
    // e.g. SQLite/PostgreSQL: (path LIKE ?3 ESCAPE '\' AND substr(path, 1, 7) = ?4)
    //      SQL Server:        (path LIKE ?3 ESCAPE '\' AND SUBSTRING(path, 1, 7) = ?4 COLLATE Latin1_General_100_BIN2)
}
```
The `LIKE` stays (it narrows rows); the equality decides. `prefix_len` is `dialect.char_len(&prefix)`. A site binds `like` at `like_ph` and `prefix` at `prefix_ph`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/rel/dialect.rs` | modify (`substr_fn`, `char_len`, `exact_equals`, unit tests of the three rendered clauses) |
| `crates/core/src/storage/meta.rs` | modify (`SubtreeMatch`; `remove_subtree`: both the `SELECT` and the `DELETE` use the clause) |
| `crates/core/src/tools/case_paths_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] mod case_paths_e2e;`) |
### Existing patterns
- Escaped prefix today: `descendant_pattern` at `crates/core/src/storage/meta.rs:574`; a query shaped like the permanent removal:
```rust
"SELECT sha256, kind, size FROM nodes WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')"
```
- Test harness: `World`, `seed`, `ok_json`, `err_text` in `crates/core/src/tools/disk_usage_e2e.rs` / `folder_usage_e2e.rs` (copy what you need into `case_paths_e2e.rs`; the modules are private). Delete/trash/restore helpers: `delete`, `trashed` in `folder_usage_e2e.rs`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `P` owned by `o@x.com`, which can delete permanently; files `/Video/x.txt` containing `upper` and `/video/y.txt` containing `lower`, 10 bytes in all. All tests drive the real tool methods (the assistant interface) over real storage.

### SPEC-0050/BT-001: a permanent delete leaves the other case alone
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Reproduces:** BUG-001
- **Driver:** the assistant tool methods `fs.delete`, `fs.read`, `fs.list_dir` and the usage read, through the project's functional e2e harness.
- **Fails today because:** the removal selects `/Video/x.txt` through the case insensitive `LIKE` (`crates/core/src/storage/meta.rs:2139`).
- **Given** the shared setup
- **When** the owner deletes `/video` recursively and permanently
- **Then** listing `/` shows the single folder `/Video`; reading `/Video/x.txt` returns `upper`; reading `/video/y.txt` fails with `ERR_NOT_FOUND`
- **And** the project's bytes used is 5

### SPEC-0050/BT-020: the folder and everything below it still go
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Protects:** the delete, move and copy of a folder (BR-001, BR-003, BR-004)
- Given `/video/sub/z.txt`, `/video/y.txt` and an unrelated `/other/o.txt` / When the owner deletes `/video` recursively and permanently / Then `/video` and both files below it are gone; `/other/o.txt` is untouched; the same files moved to `/clip` appear under `/clip` with the same sub folder

### SPEC-0050/BT-021: a name that only starts like the folder is not touched
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Protects:** BR-008, the prefix rule
- Given `/video/y.txt` and `/videos/q.txt` / When the owner deletes `/video` recursively / Then `/videos/q.txt` is untouched

### SPEC-0050/BT-022: wildcard characters
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Protects:** BR-008
- Given `/a_b/x.txt`, `/axb/y.txt`, `/50%/p.txt` and `/50x/q.txt` / When the owner deletes `/a_b` and then `/50%` / Then `/axb/y.txt` and `/50x/q.txt` are untouched

## Constraints
- Files not to touch: `fs_ops.rs`, `volume.rs`, `purge.rs`, the tool layer, any other `LIKE` site (later stories); `git/db.rs:319` (hash abbreviation) is never touched.
- Dependencies not to add: none.
- Patterns to avoid: building the clause or the pattern by hand at a site; putting a user value into the SQL text (the length is the only literal and it is an integer).
- Scope boundary: rename and trash (US-0075), subtree reads (US-0076), trash listing and purge (US-0077).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (BT-001 fails on its assertion: reading `/Video/x.txt` answers `ERR_NOT_FOUND` where `upper` is expected; BT-020, BT-021 and BT-022 pass today, so each gets a mutation check instead: break the behavior it protects, see the test fail, revert; the dialect unit test is written first and fails to compile until the helpers exist, so write the helpers as empty bodies returning an empty string first).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing wildcard tests `subtree_does_not_match_a_sibling_through_an_underscore` and `remove_subtree_does_not_delete_a_sibling_through_a_percent` stay green; the conformance suite and the 36 + 41 usage tests stay green.
- [ ] Docs updated: none in this story (US-0077 writes the docs)
- [ ] Self review: full; time and space of the changed query paths stated; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0074: exact case for the permanent delete`, body `Spec: SPEC-0050` and the requirement ids.
