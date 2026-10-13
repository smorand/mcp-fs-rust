# SPEC-0025: Search does not follow the trash, design

> Spec: sdlc/specs/SPEC-0025-deleted-files-reenter-search/sdlc-spec.md
> Nature: BUG
> Depth: L
> Security: n/a
> Designed on: 2026-10-13
> Target tier: 2 (default)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024, existing workspace | `Cargo.toml` | n/a |
| Search | existing `SearchBackend` and `search::indexer` hooks | `crates/core/src/search/mod.rs:84`, `indexer.rs` | DEC-002, DEC-004 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: one trash predicate, applied at every root cause site
A single function decides whether a path is in the trash: it is when it equals `/{trash_dir}` or starts
with `/{trash_dir}/`, `trash_dir` read from `SafetyConfig` (`crates/core/src/safety.rs:90`, `:221-225`).
The index walkers (`indexer.rs:136`, `mcp/server.rs:2846`, `indexer.rs:371-381` used by `after_move`
at `:355`) drop trash paths; the auto purge and the restore call the same index operations the delete
and move hooks use.
- Pros: fixes each of the five root cause sites; top level only, so `/docs/.mcp_trash` stays indexed (BT-016).
- Cons: three call signatures gain a parameter (DEC-004).
- Scope: `search/indexer.rs`, `mcp/server.rs`, `purge.rs`, `tools/trash.rs`, `app.rs`, `cli.rs`.
### Rejected: reuse `iter_files` exclusions
`fs_ops::iter_files(client, root, &[".mcp_trash"])` excludes the segment at any depth
(`core/fs_ops.rs:303-305`), so it would drop `/docs/.mcp_trash/a.txt` (violates BT-016). DEC-001.
### Rejected: filter results at query time
Hide trash paths in `search.query` only. Leaves `search.status` overcounting (BT-001 asserts
`bm25_docs: 1`) and stale chunks of purged paths (BR-004). DEC-001.

## 3. Architecture
```
trash predicate  is_trash_path(path, trash_dir)          (search/indexer.rs, DEC-001)
   used by: full_index (mode change)          BR-001
            search.index recursive            BR-002
            files_under / after_move          BR-003
auto purge sweep_project_files -> backend.delete_path(original path) when the mode is active   BR-004 (DEC-002)
fs.trash_restore -> indexer::after_move(state, mount, [trash_path], restored_path, client)       BR-005 (DEC-003)
```

## 4. Data model
No change.

## 5. Contracts and interfaces
No tool change. Internal:
- `pub(crate) fn is_trash_path(path: &str, trash_dir: &str) -> bool` in `crates/core/src/search/indexer.rs`.
- `full_index(backend, volume_id, client, trash_dir: &str)` gains the trash dir.
- `purge::sweep_project_files` and `purge::run_cycle` gain `search: Option<&Arc<dyn SearchBackend>>`;
  `app.rs:215` passes `state.search.as_ref()`, `cli.rs:253` passes the backend `cli.rs` already builds
  for the grace sweep (`cli.rs:224`).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/search/indexer.rs` | predicate; `full_index`, `files_under` skip trash | BR-001, BR-003, BR-006 | US-0027 |
| `crates/core/src/mcp/server.rs` | recursive `search.index` skips trash | BR-002 | US-0027 |
| `crates/core/src/tools/trash_search_e2e.rs` | new tests | US-0027, US-0028 tests | US-0027, US-0028 |
| `crates/core/src/tools/mod.rs` | register test module | DEC-005 | US-0027 |
| `crates/core/src/purge.rs` | sweep removes chunks | BR-004 | US-0028 |
| `crates/core/src/tools/trash.rs` | restore reindexes | BR-005 | US-0028 |
| `crates/core/src/app.rs`, `crates/core/src/cli.rs` | pass the backend | BR-004 | US-0028 |
| `.agent_docs/search.md` | trash rule, 2026-10-13 | BR-006 | US-0028 |
Affected existing tests: kept (spec 4.4); call sites of `full_index`, `run_cycle`, `sweep_project_files` in tests gain the new argument.

## 7. Compatibility and risks
Indexes built before the fix keep trash entries until the next mode change, which wipes first
(`indexer.rs:68`). Restore and sweep index work is detached like every hook, so a test waits for the
index to settle. Rollback: revert.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | tool calls, BM25 Tantivy (`make check`), PG and stub vectors when DSN set | `tools/trash_search_e2e.rs` | US-0027 |
| BT-010, BT-011, BT-012, BT-015, BT-016 | functional e2e | tool calls | same | US-0027 |
| BT-020 | functional e2e + existing indexer tests | tool calls | same | US-0027 |
| BT-002, BT-013, BT-014, BT-021 | functional e2e | tool calls, purge cycle | same | US-0028 |
Fixtures: `crates/core/src/search/e2e.rs` harnesses (`make_sqlite_bm25_harness`, `make_sqlite_rag_harness`,
`make_pg_bm25_harness`) and the settle wait of `tools/project_removal_e2e.rs`. Red before: BT-001 (2 docs),
BT-010, BT-012 (trash path returned), BT-015, BT-002 and BT-014 (not found), BT-013 (stale path returned).

## 10. Decisions log
- **DEC-001:** one predicate `is_trash_path(path, trash_dir)`, top level only, applied by the three
  index walkers. **Obliged by:** BR-001, BR-002, BR-003, BR-006. **Rationale:** fixes the root causes
  and keeps nested folders. **Alternatives considered:** `iter_files` exclusions (any depth); query
  time filter. **Source:** design. **Evidence:** `core/fs_ops.rs:303-305`, `safety.rs:221-225`.
- **DEC-002:** the auto purge removes the original path's chunks with `SearchBackend::delete_path`
  when the project's index mode is active, after a successful move to the trash. **Obliged by:** BR-004.
  **Rationale:** the sweep has no `AppState`, only the backend and the admin store. **Alternatives
  considered:** route through `indexer::after_delete` (needs `AppState`). **Source:** existing code
  `purge.rs:51-95`. **Evidence:** `search/mod.rs:84`.
- **DEC-003:** `fs.trash_restore` calls `indexer::after_move(state, mount, &[trash_path], restored_path, client)`
  after the rename. **Obliged by:** BR-005. **Rationale:** the same hook `fs.move` uses; it clears the
  source and indexes the destination, `_restored` paths included. **Alternatives considered:** a new
  hook. **Source:** existing code `indexer.rs:327-356`.
- **DEC-004:** `purge::run_cycle` and `purge::sweep_project_files` gain
  `search: Option<&Arc<dyn SearchBackend>>`; `app.rs:215` and `cli.rs:253` pass it. **Obliged by:**
  BR-004. **Rationale:** same pattern as SPEC-0024's grace sweep. **Source:** existing code `cli.rs:224`.
- **DEC-005:** tests in a new `#[cfg(test)]` module `tools/trash_search_e2e.rs`, BM25 Tantivy runs in
  `make check`, PostgreSQL and vector variants behind `MCPFS_TEST_PG_DSN` and stub embeddings.
  **Obliged by:** spec section 4. **Source:** existing `tools/project_removal_e2e.rs`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0027 | Index walks skip the trash | P1 | BR-001, BR-002, BR-003, BR-006 | main | 2 | no |
| US-0028 | Auto purge and restore keep search in step | P1 | BR-004, BR-005, BR-006 | US-0027 | 2 | yes |
