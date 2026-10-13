# US-0028: Auto purge and restore keep search in step

> Spec: SPEC-0025 (sdlc/specs/SPEC-0025-deleted-files-reenter-search/)
> Nature: BUG
> Priority: P1
> Depends on: US-0027
> min_tier: 2
> Files touched: 6
> Closes spec: yes

## Objective
The auto purge moves stale files to the trash without removing them from search, so search returns paths that no longer exist; `fs.trash_restore` puts a file back without reindexing it, so it cannot be found. After this story both keep the index in step with the volume.

## Requirements covered
- SPEC-0025/BR-004: WHEN the auto purge soft deletes a stale file THE Search context SHALL remove that file's chunks.
- SPEC-0025/BR-005: WHEN `fs.trash_restore` puts a file back THE Search context SHALL index it at the path it was restored to, including a `_restored` collision path.
- SPEC-0025/BR-006: The Search context SHALL NOT return from `search.query` a result whose `path` is under the trash, unless that file was indexed explicitly as in BR-002's assumption.

## Independent Test
`make check` green; the US-0028 tests in `crates/core/src/tools/trash_search_e2e.rs` pass: BT-013 runs the purge cycle and reads `search.query` and `fs.trash_list`; BT-002 restores and reads `search.query`.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0025): the auto purge (`crates/core/src/purge.rs:51-95`, rename at `:83`) removes the original path's chunks with `SearchBackend::delete_path(project_id, original_path)` after a successful move to the trash, only when the project's index mode is active (`admin.get_index_mode(project_id)`, `is_active()`), errors logged not raised (the indexer's failure rule).
- DEC-003 (SPEC-0025): `fs.trash_restore` (`crates/core/src/tools/trash.rs:177`) calls `crate::search::indexer::after_move(state, mount_id, &[trash_path.to_string()], &restored_path, &client)` after the rename (`indexer.rs:327-356`): it clears the trash path and indexes the restored path, `_restored` collisions included (`tools/trash.rs` tests assert `/a.txt_restored2`, `/report.pdf_restored`).
- DEC-004 (SPEC-0025): `purge::run_cycle` and `purge::sweep_project_files` gain `search: Option<&Arc<dyn SearchBackend>>`; `crates/core/src/app.rs:215` passes `state.search.as_ref()`; `crates/core/src/cli.rs:253` passes the backend `cli.rs` already builds for the grace sweep (`cli.rs:224`); test callers (`tools/teardown_e2e.rs:408`, `purge.rs` tests) pass `None` or their backend.
- DEC-005 (SPEC-0025): tests in `crates/core/src/tools/trash_search_e2e.rs` (from US-0027). Harnesses: `crates/core/src/search/e2e.rs` (`make_sqlite_bm25_harness`, `make_sqlite_rag_harness`, `make_pg_bm25_harness`, stub embeddings) and the settle wait used in `crates/core/src/tools/project_removal_e2e.rs`. BM25 on Tantivy runs in plain `make check`; PostgreSQL and vector variants skip with `eprintln!` when `MCPFS_TEST_PG_DSN` is unset.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/purge.rs` | sweep removes chunks, signatures |
| `crates/core/src/tools/trash.rs` | restore reindexes |
| `crates/core/src/app.rs` | pass the backend |
| `crates/core/src/cli.rs` | pass the backend |
| `crates/core/src/tools/trash_search_e2e.rs` | add tests |
| `.agent_docs/search.md` | the trash rule, timestamped 2026-10-13 |

## Acceptance tests
> 100% must pass, through `make check` (plus the DSN variants when set).

### SPEC-0025/BT-002: A restored file is found again
- **Reproduces:** BUG-001 (restore side)
- **Fails today because:** `tools/trash.rs:177` calls no index hook.
- Given `atlas` in mode `bm25`, `/memo.txt` deleted
- When `fs.trash_restore {mount_id: "atlas", trash_path: <its trash_path from fs.trash_list>}` / Then `search.query {query: "forecast"}` returns one result with `path: "/memo.txt"`.

### SPEC-0025/BT-013: Auto purge removes the stale file from search
- **Validates:** BR-004
- Given `atlas` with auto purge on, `file_retention_days: 1`, `/old.txt` = `obsolete roadmap` last accessed 2 days ago
- When the purge cycle runs / Then `search.query {query: "roadmap"}` returns `"results": []`, and `fs.trash_list` lists `/old.txt`.

### SPEC-0025/BT-014: Restore onto a taken path
- **Validates:** BR-005
- Given `/memo.txt` deleted, then a new `/memo.txt` = `fresh notes` written
- When the deleted one is restored / Then `search.query {query: "forecast"}` returns `/memo.txt_restored`, and `search.query {query: "fresh"}` still returns `/memo.txt`.

### SPEC-0025/BT-021: Trash listing and restore unchanged
- **Protects:** SPEC-0013
- `fs.trash_list` and `fs.trash_restore` return exactly what they return today for the BT-002 data.

## Constraints
- Files not to touch: search backend implementations, `crates/core/src/storage/*`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Restore and purge responses and trash rows unchanged (BT-021).
- An index failure never fails the purge or the restore.

## Definition of done
- [ ] RED for BT-002, BT-013, BT-014 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: purge tests, trash tests, `teardown_e2e`, `project_removal_e2e`.
- [ ] Docs updated: `.agent_docs/search.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0025 green; BR-001..BR-006 cited by US-0027 or US-0028.
- [ ] One commit `US-0028: Auto purge and restore keep search in step`, body `Spec: SPEC-0025`.
