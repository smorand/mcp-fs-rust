---
Id: BL-0050
Title: An index mode change indexes the trash, so a soft deleted file becomes searchable
Nature: BUG
Source: US-0026 implementation (SPEC-0024), 2026-10-13
Created: 2026-10-13
---

## What
`search::indexer::full_index` (`crates/core/src/search/indexer.rs:131`), run in the background
by `admin.set_index_mode` through `ProjectIndexer::on_mode_change`, walks
`fs_ops::iter_files(client, "/", &[])` with no exclusion, so it indexes every file under
`/{safety.trash_dir}` too. A file the user deleted (soft delete into the trash) is unindexed by
the delete hook, then indexed again under its trash path by the next mode change, and
`search.query` returns its content.

Evidence, US-0026 branch, `MCPFS_TEST_PG_DSN` set,
`cargo test -p mcp-fs-core --all-features --lib -- project_removal_e2e`, with a seed that wrote
`/plan.txt` and `/memo.txt`, deleted `/memo.txt` into the trash, THEN called
`admin.set_index_mode`: on every backend (Tantivy, PostgreSQL BM25, sqlite-vec, PostgreSQL
vectors) `search.status` settled at 2 entries, not 1, 5 runs out of 5:
```
atlas: index never reached 1 entries, last saw 2
```
The US-0026 tests now set the mode before writing, so they do not depend on this.

## Proposal
Skip `/{safety.trash_dir}` (and any internal prefix) in `full_index`, the same way the write
and delete hooks never index a trash path. Test: delete a file, change the mode, assert
`search.query` finds nothing of it and `search.status` counts only live files.
