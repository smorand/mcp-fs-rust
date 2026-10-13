# US-0027: Index walks skip the trash

> Spec: SPEC-0025 (sdlc/specs/SPEC-0025-deleted-files-reenter-search/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A mode change backfill, a recursive `search.index` and a move into the trash folder all index files under the trash, so deleted content comes back in `search.query` and `search.status` overcounts. After this story every index walk skips the top level trash folder.

## Requirements covered
- SPEC-0025/BR-001: WHEN an index mode change backfills a project THE Search context SHALL index every live file and no file under the trash.
- SPEC-0025/BR-002: WHEN `search.index` is called with `recursive: true` on a path that contains the trash THE Search context SHALL index no file under the trash, and SHALL report only the files it indexed.
- SPEC-0025/BR-003: WHEN `fs.move` moves a file into the trash THE Search context SHALL remove the source's chunks and SHALL NOT index the destination.
- SPEC-0025/BR-006: The Search context SHALL NOT return from `search.query` a result whose `path` is under the trash, unless that file was indexed explicitly as in BR-002's assumption.
(BR-006 partial: the walks; the purge path is US-0028.)

## Independent Test
`make check` green; the US-0027 tests in `crates/core/src/tools/trash_search_e2e.rs` pass, BT-001 reading `search.query` and `search.status` after the mode change backfill settles.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0025): one predicate `pub(crate) fn is_trash_path(path: &str, trash_dir: &str) -> bool` in `crates/core/src/search/indexer.rs`, true when `path == "/{trash_dir}"` or starts with `"/{trash_dir}/"` (top level only; `trash_dir` from `SafetyConfig`, `crates/core/src/safety.rs:90`, `:221-225`, default `.mcp_trash`). Do NOT use `fs_ops::iter_files` exclusions: they match the segment at any depth (`core/fs_ops.rs:303-305`) and would drop `/docs/.mcp_trash/a.txt`.
- DEC-001 applied at: `full_index` (`crates/core/src/search/indexer.rs:131-150`, walk at `:136`; signature gains `trash_dir: &str`, callers `indexer.rs:84` and the test at `:572`), recursive `search.index` (`crates/core/src/mcp/server.rs:2845-2850`), `files_under` used by `after_move` (`indexer.rs:371-381`, `:355`) so a move into the trash indexes nothing there while the source is still cleared (`indexer.rs:336-353`).
- DEC-005 (SPEC-0025): tests in a new `#[cfg(test)]` module `crates/core/src/tools/trash_search_e2e.rs`. Harnesses: `crates/core/src/search/e2e.rs` (`make_sqlite_bm25_harness`, `make_sqlite_rag_harness`, `make_pg_bm25_harness`, stub embeddings) and the settle wait used in `crates/core/src/tools/project_removal_e2e.rs`. BM25 on Tantivy runs in plain `make check`; PostgreSQL and vector variants skip with `eprintln!` when `MCPFS_TEST_PG_DSN` is unset.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/search/indexer.rs` | predicate, `full_index`, `files_under` |
| `crates/core/src/mcp/server.rs` | recursive `search.index` filter |
| `crates/core/src/tools/trash_search_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod trash_search_e2e;` |

## Acceptance tests
> 100% must pass, through `make check` (plus the DSN variants when set).

### SPEC-0025/BT-001: A deleted file does not come back after a mode change
- **Reproduces:** BUG-001
- **Driver:** tool calls `fs.write_bytes`, `fs.delete`, `admin.set_index_mode`, `search.query`, `search.status`.
- **Fails today because:** `indexer.rs:136` walks the trash.
- Given `atlas` in mode `bm25` with `/memo.txt` = `quarterly forecast draft` and `/plan.txt` = `merger with Borealis`
- When `/memo.txt` is deleted, then `admin.set_index_mode {project_id: "atlas", mode: "both"}` runs and its backfill completes
- Then `search.query {mount_id: "atlas", query: "forecast"}` returns `"results": []`
- And `search.status` reports `bm25_docs: 1`

### SPEC-0025/BT-010: Recursive index of the root skips the trash
- **Validates:** BR-002
- Given `/memo.txt` deleted and `/plan.txt` live / When `search.index {path: "/", recursive: true}` / Then `search.query {query: "forecast"}` returns `"results": []`, `search.query {query: "Borealis"}` returns `/plan.txt`.

### SPEC-0025/BT-011: Explicit index of one trash file still works
- **Validates:** BR-002 assumption, BR-006 exception
- When `search.index {path: <trash path of /memo.txt>}` / Then `search.query {query: "forecast"}` returns that trash path.

### SPEC-0025/BT-012: Move into the trash folder
- **Validates:** BR-003
- When `fs.move {source: "/plan.txt", destination: "/.mcp_trash/manual.txt"}` / Then `search.query {query: "Borealis"}` returns `"results": []`.

### SPEC-0025/BT-015: Custom trash folder
- **Validates:** BR-001, edge case
- Given `safety.trash_dir: ".bin"` and `/memo.txt` deleted into `/.bin/` / When the mode changes / Then `"results": []` for `forecast`.

### SPEC-0025/BT-016: Nested folder named like the trash is indexed
- **Validates:** edge case
- Given `/docs/.mcp_trash/a.txt` = `nested keep` (a live file) / When the mode changes / Then `search.query {query: "nested"}` returns `/docs/.mcp_trash/a.txt`.

### SPEC-0025/BT-020: Delete still clears, write still indexes
- **Protects:** SPEC-0008/SC-004 hooks
- `fs.delete` of an indexed file empties its results; `fs.write_bytes` makes a new file findable; existing indexer tests stay green.

## Constraints
- Files not to touch: `crates/core/src/purge.rs`, `crates/core/src/tools/trash.rs` (US-0028), search backend implementations, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Patterns to avoid: query time filtering; segment exclusions at any depth.
- Explicit non recursive `search.index` of a trash file keeps working (BT-011).

## Definition of done
- [ ] RED for BT-001, BT-010, BT-012, BT-015 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: indexer tests (`indexer.rs:520-580`), `project_removal_e2e`.
- [ ] Self review: full.
- [ ] One commit `US-0027: Index walks skip the trash`, body `Spec: SPEC-0025`.
