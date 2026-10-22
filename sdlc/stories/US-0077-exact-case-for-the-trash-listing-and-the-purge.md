# US-0077: exact case for the trash listing and the purge, spec close

> Spec: SPEC-0050 (sdlc/specs/SPEC-0050-case-sensitive-subtree-queries/)
> Nature: BUG
> Priority: P2
> Depends on: US-0075, US-0076
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
The trash listing by original path prefix and the exemption of the trash folder from the automatic file purge compare letter case exactly. The cross engine conformance suite gets a case collision case, the architecture notes carry the rule, and the spec closes with its whole acceptance set.

## Requirements covered
- SPEC-0050/BR-006: WHEN the trash is listed with an original path prefix THE product SHALL return only the entries whose original path equals the prefix or lies below it, comparing letter case exactly.
- SPEC-0050/BR-007: IF a stale file lies under a folder whose name differs from the trash folder name only by letter case THEN THE product SHALL purge that file like any other stale file.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::case_paths_e2e purge::` and `storage::conformance`: BT-006 lists one entry for each prefix; BT-007 shows `/.MCP_TRASH/old.txt` moved to the trash by the file purge while a file inside `/.mcp_trash` stays.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0050): `stale_files` excludes the trash folder with `NOT (clause)`, so only the exact trash folder is exempt from the file purge.
- DEC-008 (SPEC-0050): the trash entry filter applies the clause to `original_path`; the equality `original_path = ?` of the existing predicate stays.
- DEC-010 (SPEC-0050): tests are functional e2e on the tool methods for user visible operations, an integration test for the purge, plus a dialect unit test and a conformance case as extras.
### Data model
None. No schema change, no migration.
### Contracts
Uses the helpers of US-0074. `list_trash_entries` and `count_trash_entries` render their prefix predicate as `(original_path = ?2 OR <clause on original_path>)`, binding `like` and `prefix` of `SubtreeMatch::new(dialect, prefix)`; `stale_files` renders `path<>?3 AND NOT <clause on path>` for `exclude_root`. The trash entry filter with an empty prefix is unchanged.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify (`list_trash_entries`, `count_trash_entries`, `stale_files`; remove `trash_path_prefix_pattern` if unused) |
| `crates/core/src/purge.rs` | modify (one test in the test module) |
| `crates/core/src/storage/conformance.rs` | modify (case collision case in `meta_like_escaping`) |
| `crates/core/src/tools/case_paths_e2e.rs` | modify (add tests) |
| `.agent_docs/architecture.md`, `AGENTS.md` | modify (the rule below) |
### Existing patterns
- Escaped prefix today: `descendant_pattern` at `crates/core/src/storage/meta.rs:574`; a query shaped like the permanent removal:
```rust
"SELECT sha256, kind, size FROM nodes WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')"
```
- Test harness: `World`, `seed`, `ok_json`, `err_text` in `crates/core/src/tools/disk_usage_e2e.rs` / `folder_usage_e2e.rs` (copy what you need into `case_paths_e2e.rs`; the modules are private). Delete/trash/restore helpers: `delete`, `trashed` in `folder_usage_e2e.rs`.
- Purge test fixture: the test around `crates/core/src/purge.rs:700` (`f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None)`, then `sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None)`); set a file's access time 10 days back through the store, as the neighbouring tests do.
- Conformance case to extend: `meta_like_escaping`, `crates/core/src/storage/conformance.rs:190-220`: add `put_file("/Video/x.txt", ..)` and `put_file("/video/y.txt", ..)`, then assert `subtree("/video")`, `remove_subtree("/video")` and `rename("/video", "/clip")` leave `/Video/x.txt` alone.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `P` owned by `o@x.com`, which can delete permanently; files `/Video/x.txt` containing `upper` and `/video/y.txt` containing `lower`, 10 bytes in all. All tests drive the real tool methods (the assistant interface) over real storage.

### SPEC-0050/BT-006: the trash listing prefix is exact
- Level / Driver / Location: functional e2e / the assistant tool methods over real storage / `crates/core/src/tools/case_paths_e2e.rs`
- **Validates:** BR-006
- Given the shared setup, both `/Video/x.txt` and `/video/y.txt` deleted to the trash one by one / When the owner lists the trash with `path_prefix` `/Video` and then with `/video` / Then the first list has 1 entry with original path `/Video/x.txt`, the second 1 entry with original path `/video/y.txt`

### SPEC-0050/BT-007: a folder named like the trash folder in another case is purged normally
- Level / Driver / Location: integration / `purge::sweep_project_files` over the real stores / `crates/core/src/purge.rs`
- **Validates:** BR-007
- Given automatic purge on with a retention of 1 day, the trash folder `.mcp_trash`, and the stale file `/.MCP_TRASH/old.txt` (last access 10 days ago) / When the file purge runs / Then `/.MCP_TRASH/old.txt` has been moved to the trash like any stale file, and a stale file already inside `/.mcp_trash` is left where it is

## Constraints
- Files not to touch: the tool layer; the git hash lookup (`git/db.rs`).
- Dependencies not to add: none.
- Patterns to avoid: a second prefix helper; case folding anywhere.
- Scope boundary: none left; this story closes SPEC-0050. The SQL Server behavior of the whole key comparison is BL-0073.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (BT-006 fails because each prefix lists 2 entries; BT-007 fails because `/.MCP_TRASH/old.txt` is excluded from the purge by the case insensitive `NOT LIKE`).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every test of US-0074 to US-0076 stays green; the conformance suite stays green on SQLite (PostgreSQL and SQL Server with their opt in DSNs).
- [ ] Docs updated: `.agent_docs/architecture.md` (a dated entry: a path and what lies below it is selected by `SubtreeMatch`, a `LIKE` plus an exact substring equality, case exact on every engine) and the same rule in `AGENTS.md` Conventions next to the `descendant_pattern` rule
- [ ] Self review: full; time and space of the changed query paths stated; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] Closes spec: the 11 acceptance tests of SPEC-0050 (BT-001 to BT-008, BT-020 to BT-022) are green, every BR-001 to BR-008 is cited by at least one story with status done or landed (US-0074 to US-0077), and the SPEC-0049 test `spec_0049_us_0073_folders_differing_only_by_case_do_not_mix` still passes.
- [ ] One commit `US-0077: exact case for the trash listing and the purge, spec close`, body `Spec: SPEC-0050` and the requirement ids.
