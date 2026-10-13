# US-0026: Project removal wipes the search index

> Spec: SPEC-0024 (sdlc/specs/SPEC-0024-deleted-project-data-survives/)
> Nature: BUG
> Priority: P1
> Depends on: US-0025
> min_tier: 1
> Files touched: 7
> Closes spec: yes

## Objective
Neither `admin.delete_project` nor the grace period purge removes a project's search index, on any engine, so `search.query` on a recreated project id returns the old project's file content, and a deleted project's search data is never removed. After this story one removal function serves both paths and wipes the index. Security: internal finding (CWE-226).

## Requirements covered
- SPEC-0024/BR-001: WHEN a project is recreated THE Storage context SHALL return `{"entries": [], "total": 0}` from `fs.trash_list` until a file of the new project is deleted.
- SPEC-0024/BR-002: WHEN a project is recreated THE Search context SHALL return no result from `search.query` for content that only the deleted project held, and `search.status` SHALL report 0 indexed documents and 0 vector chunks until the new project indexes content.
- SPEC-0024/BR-003: WHEN `admin.delete_project` or the grace period purge removes a project THE Search context SHALL remove every indexed document and chunk of that project, whether or not the id is reused.
- SPEC-0024/BR-007: The Storage and Search contexts SHALL NOT remove or change any row, index entry, file or blob of a project other than the one deleted.
- SPEC-0024/BR-008: WHILE a project is soft deleted and inside its grace period THE Storage and Search contexts SHALL keep its trash, artifacts and search index, so that `admin.undelete_project` restores them unchanged.

## Independent Test
With `docker compose -f docker-compose.test.yml up -d` and `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`: `cargo test -p mcp-fs-core --all-features --lib -- project_removal_e2e teardown_e2e conformance` green, and `make check` green (BT-002 runs there on SQLite). BT-001 reads results back through `search.query` and `fs.trash_list`; BT-006 lists the Tantivy directory.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0024): search removal uses the existing `SearchBackend::delete_all(volume_id)` (`crates/core/src/search/mod.rs:84`), which already drops the Tantivy directory and its cache entry (`search/bm25_sqlite.rs:202-224`), the sqlite-vec rows (`search/vector_sqlite.rs:214-226`) and the PostgreSQL rows (`search/bm25_pg.rs:111-119`, `search/vector_pg.rs:155`).
- DEC-003 (SPEC-0024): one `pub(crate) async fn remove_project(project_id: &str, stores: &StoreManager, admin: &dyn AdminBackend, config: &Arc<ServerConfig>, search: Option<&Arc<dyn SearchBackend>>) -> Result<()>` in `crates/core/src/purge.rs`, steps in order: `stores.teardown_volume`, `search.delete_all` when a backend is configured, `GitRepoStore::purge_repo` when git is enabled, `admin.delete_project`. Both `admin_delete_project` (`crates/core/src/mcp/server.rs:2446-2467`, today duplicating the steps at `:2456-2464`) and the grace period purge (`purge.rs:343-362`, today `:357-359`) call it. `sweep_grace_period` / `sweep_grace_period_at` gain `search: Option<&Arc<dyn SearchBackend>>`; `crates/core/src/app.rs:224` passes `state.search.as_ref()`; `crates/core/src/cli.rs:217` builds it with `search::build_backend` (`search/mod.rs:120`). Every step is idempotent, so a failed deletion is retried by calling again.
- DEC-005 (SPEC-0024): tests in a new `#[cfg(test)]` module `crates/core/src/tools/project_removal_e2e.rs`, harnesses from `crates/core/src/search/e2e.rs` (`make_sqlite_bm25_harness`, `make_sqlite_rag_harness`, `make_pg_bm25_harness`, stub embeddings) and `tools/pg_null_e2e.rs`. Fixtures follow `crates/core/src/tools/pg_null_e2e.rs` and `crates/core/src/tools/mssql_trash_e2e.rs` (fresh schema or rows per test, cleanup, `eprintln!` skip when `MCPFS_TEST_PG_DSN` / `MCPFS_TEST_MSSQL_DSN` is unset or the feature is off).
### Security
Occurrences fixed here: search rows and files (`mcp/server.rs:2456-2464`, `purge.rs:357-359`), the Tantivy cache (`search/bm25_sqlite.rs:40`), the sqlite-vec connection cache (`search/vector_sqlite.rs:36-37`). Absence of effect read through `search.status`, a directory listing of `search.tantivy_dir`, PostgreSQL row counts.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/purge.rs` | `remove_project`; grace purge takes the search backend |
| `crates/core/src/mcp/server.rs` | `admin_delete_project` calls `remove_project` |
| `crates/core/src/app.rs` | pass `state.search.as_ref()` |
| `crates/core/src/cli.rs` | build the backend for the `purge` verb |
| `crates/core/src/tools/project_removal_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod project_removal_e2e;` |
| `.agent_docs/architecture.md` | project removal steps, timestamped 2026-10-13 |

## Acceptance tests
> 100% must pass, through `make check` plus the opt in suites above.

### SPEC-0024/BT-001: A recreated project does not see the old project's trash or search content
- **Reproduces:** BUG-001
- **Driver:** MCP tool calls `fs.write_bytes`, `fs.delete`, `admin.delete_project`,
  `admin.create_project`, `fs.trash_list`, `search.query` on a server with PostgreSQL metadata and BM25 search.
- **Fails today because:** `storage/mod.rs:511` leaves `trash_entries`, and no deletion path removes the search index.
- Given `atlas` (owner `olivia@example.com`, index mode `bm25`) holding `/plan.txt` = `merger with Borealis`, and `/memo.txt` deleted by Olivia
- When the admin deletes `atlas`, creates `atlas` for `otto@example.com`, and Otto calls `fs.trash_list {mount_id: "atlas"}` and `search.query {mount_id: "atlas", query: "Borealis", mode: "bm25"}`
- Then `fs.trash_list` returns `{"entries": [], "total": 0}`
- And `search.query` returns `"results": []`

### SPEC-0024/BT-002: Same reproduction on SQLite metadata
- **Reproduces:** BUG-001 (search part, in `make check`)
- Given the BT-001 data on the default SQLite configuration with BM25 / When the same calls / Then `"results": []` and `total: 0`.

### SPEC-0024/BT-006: Search content of a deleted project
- **Covers:** `server.rs:2456-2464`, `purge.rs:357-359` (search index), every backend
- **Driver:** tool calls; one run per backend (`bm25` on Tantivy, `bm25` on PostgreSQL, `rag` on sqlite-vec with the stub embedding server of `search/e2e.rs`)
- **Red before the fix because:** `search.query` returns the old `chunk`.
- Given `atlas` indexed with `/plan.txt` = `merger with Borealis`
- When the project is deleted and recreated, and Otto calls `search.query {mount_id: "atlas", query: "Borealis"}`
- Then `"results": []`
- And `search.status {mount_id: "atlas"}` reports `vector_chunks: 0` and 0 indexed documents, and on disk no Tantivy directory of the old index remains (read by listing the configured `search.tantivy_dir`)

### SPEC-0024/BT-007: Grace period purge path
- **Covers:** `purge.rs:345-362`
- Given `atlas` soft deleted and its grace period elapsed / When the purge sweep runs, and `atlas` is recreated / Then BT-001's `fs.trash_list` and `search.query` assertions hold.

### SPEC-0024/BT-009: In memory vector index of a deleted project
- **Covers:** `search/vector_sqlite.rs:36-37` (per volume connection cache, `rag` mode on sqlite-vec)
- **Driver:** tool calls in one server process, the stub embedding server of `search/e2e.rs`
- **Red before the fix because:** the cached connection still holds the old chunks.
- Given `atlas` in index mode `rag` with `/plan.txt` = `merger with Borealis` indexed
- When `atlas` is deleted and recreated in the same process, and Otto calls `search.query {mount_id: "atlas", query: "Borealis", mode: "rag"}`
- Then `"results": []`
- And `search.status {mount_id: "atlas"}` reports `vector_chunks: 0`

### SPEC-0024/BT-011: The search index is removed even without reuse
- **Validates:** BR-003
- Given `atlas` indexed / When it is deleted / Then the PostgreSQL `search_fts` and `search_chunks` hold no `atlas` row, and the Tantivy directory of `atlas` is absent.

### SPEC-0024/BT-013: Deletion is retryable
- **Validates:** BR-003, BR-004
- Given a deletion whose project record removal failed after the volume teardown / When `admin.delete_project` is called again / Then it succeeds and BT-010 and BT-011 hold.

### SPEC-0024/BT-020: Other projects untouched
- **Protects:** BR-007
- Given `atlas` and `atlas2`, both with a file, a trash entry, an indexed document and a converted document
- When `atlas` is deleted / Then `atlas2`'s `fs.trash_list`, `search.query`, `fs.list_images` and file reads return exactly what they returned before.

### SPEC-0024/BT-021: Undelete inside the grace period restores everything
- **Protects:** BR-008, SPEC-0014
- Given `atlas` with a trash entry and an indexed document / When it is soft deleted then undeleted / Then `fs.trash_list` and `search.query` return what they returned before.

### SPEC-0024/BT-022: Existing suites unchanged
- **Protects:** git purge (`git/db.rs:581-585`), project deletion tests, conformance suites
- `make check` green; the three conformance suites green with their DSNs.

## Constraints
- Files not to touch: `crates/core/src/search/*.rs` backends (reuse `delete_all`), `crates/core/src/storage/*` (US-0025), `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Patterns to avoid: wiping in two places; a new trait method.
- Soft deletion must not wipe anything (BR-008); `tools/admin.rs` `#[cfg(test)] delete_project` may route to `remove_project`.

## Definition of done
- [ ] RED for BT-001, BT-002, BT-006, BT-007, BT-009, BT-011 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: git purge tests (`git/db.rs:581-585`), grace purge tests (`purge.rs:1109`), admin deletion tests, the three conformance suites.
- [ ] Docs updated: `.agent_docs/architecture.md`.
- [ ] Self review: full, plus the security rules.
- [ ] Closes spec: every BT- of SPEC-0024 green; BR-001..BR-008 cited by US-0025 or US-0026.
- [ ] One commit `US-0026: Project removal wipes the search index`, body `Spec: SPEC-0024`.
