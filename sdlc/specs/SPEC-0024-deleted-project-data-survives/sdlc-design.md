# SPEC-0024: A deleted project's data survives and reaches the next project with the same id, design

> Spec: sdlc/specs/SPEC-0024-deleted-project-data-survives/sdlc-spec.md
> Nature: BUG
> Depth: M
> Security: internal finding
> Designed on: 2026-10-13
> Target tier: 2 (default); both stories raised to 1, see section 11

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024, existing workspace | `Cargo.toml` | n/a |
| Search backends | existing `SearchBackend` trait and its 5 implementations | `crates/core/src/search/mod.rs:84` | DEC-002 |
| Tests | `make check`; shared database suites behind `MCPFS_TEST_PG_DSN`, `MCPFS_TEST_MSSQL_DSN` | existing | DEC-005 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: complete the teardown at both root causes, through one removal function
1. Metadata: the shared database teardown deletes from every metadata table that carries `volume_id`,
   and the list comes from the schema itself (DEC-001), so a table added later is covered.
2. Search: one project removal function, called by `admin.delete_project` and by the grace period
   purge, calls the existing `SearchBackend::delete_all(volume_id)` (DEC-002, DEC-003). It already
   drops the Tantivy directory and its cache entry (`search/bm25_sqlite.rs:202-224`), the sqlite-vec
   rows (`search/vector_sqlite.rs:214-226`) and the PostgreSQL rows (`search/bm25_pg.rs:111-119`,
   `search/vector_pg.rs:155`).
- Pros: fixes at the root cause sites; reuses tested wipe code; the two deletion paths cannot drift again.
- Cons and risks: the grace period purge and the CLI `purge` verb need the search backend passed in.
- Affected scope: `storage/mod.rs`, `storage/meta.rs`, `purge.rs`, `mcp/server.rs`, `cli.rs`, `app.rs`.

### Rejected: filter reads by creation time
Make `fs.trash_list` and `search.query` ignore anything older than the project's `created_at`.
Rejected (DEC-004): the data is still retained (BR-003, BR-004), every reader must remember the filter,
and the artifact sweep corruption (BR-005) stays.

### Rejected: forbid reusing a project id
Breaks `admin.create_project` after a deletion, which the spec keeps (BR-001 assumes recreation).

## 3. Architecture
```
admin.delete_project (mcp/server.rs:2446)      grace purge (purge.rs:343, app.rs:224, cli.rs:217)
                 \                                  /
                  purge::remove_project(project_id, stores, admin, config, search)   (DEC-003)
                     1. stores.teardown_volume       all volume tables (DEC-001), bucket, SQLite file
                     2. search.delete_all            when a backend is configured (DEC-002)
                     3. GitRepoStore::purge_repo     when git is enabled (unchanged)
                     4. admin.delete_project         record and members (unchanged)
```
Every step is idempotent, so a failed deletion is retried by calling it again (BR-003, BR-004, BT-013).

## 4. Data model
No schema change. New derived list: the metadata tables with a `volume_id` column, computed from
`storage::meta::schema()` (`storage/meta.rs:166-318`), 9 today.

## 5. Contracts and interfaces
No tool or route change. Internal:
- `pub(crate) fn volume_tables() -> Vec<&'static str>` in `storage/meta.rs` (DEC-001).
- `pub(crate) async fn remove_project(project_id: &str, stores: &StoreManager, admin: &dyn AdminBackend, config: &Arc<ServerConfig>, search: Option<&Arc<dyn SearchBackend>>) -> Result<()>` in `purge.rs` (DEC-003).
- `sweep_grace_period` and `sweep_grace_period_at` gain the `search: Option<&Arc<dyn SearchBackend>>` parameter; `app.rs:224` passes `state.search.as_ref()`; `cli.rs:217` builds it with `search::build_backend` (`search/mod.rs:120`).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | add `volume_tables()` | BR-004 | US-0025 |
| `crates/core/src/storage/mod.rs` | teardown iterates `volume_tables()` (`:511`) | BR-001, BR-004, BR-005, BR-006 | US-0025 |
| `crates/core/src/tools/teardown_e2e.rs` | new tests, shared database and SQLite | BT-005, 008, 010, 012 | US-0025 |
| `crates/core/src/tools/mod.rs` | register test modules | DEC-005 | US-0025, US-0026 |
| `crates/core/src/purge.rs` | `remove_project`, grace purge takes the search backend | BR-002, BR-003, BR-008 | US-0026 |
| `crates/core/src/mcp/server.rs` | `admin_delete_project` calls `remove_project` | BR-002, BR-003 | US-0026 |
| `crates/core/src/app.rs`, `crates/core/src/cli.rs` | pass or build the search backend | BR-003 | US-0026 |
| `crates/core/src/tools/project_removal_e2e.rs` | new tests | BT-001, 002, 006, 007, 009, 011, 013, 020, 021, 022 | US-0026 |
| `.agent_docs/architecture.md` | project removal steps, 2026-10-13 | BR-003, BR-004 | US-0026 |
Affected existing tests: all kept (spec 4.4); call sites of `sweep_grace_period` in tests gain `None`.

## 7. Compatibility and risks
- Removes only data of deleted projects. A soft deleted project is never torn down before its grace
  period ends (`purge.rs:343-362`), so undelete is untouched (BR-008).
- Ordering: US-0025 alone fixes trash, artifacts and export rows; search stays exposed until US-0026.
- With `search.enabled: false` at deletion time, an index written while it was enabled is not removed.
  Accepted, as the spec's edge case states: nothing reachable serves it while search is off, and the
  next mode change wipes it (`search/indexer.rs:68`).
- Rollback: revert the commits; no stored format changes.

## 8. Security design
- Threat: a new owner of a reused project id reads the previous tenant's data. Trust boundary: tenant
  identity is the project id.
- Occurrences and fixes:
  | Occurrence | Fix site | Mechanism | DEC |
  |---|---|---|---|
  | `trash_entries` survive (`storage/mod.rs:511`) | `storage/mod.rs:511` | delete every volume table from the schema derived list | DEC-001 |
  | `doc_*` survive, sweep corrupts refcounts (`storage/meta.rs:818-826`, `:1199-1222`) | `storage/mod.rs:511` | same; no stale set survives to be swept | DEC-001 |
  | `export_links` survive | `storage/mod.rs:511` | same | DEC-001 |
  | search rows, Tantivy directory and cache, sqlite-vec cache | `mcp/server.rs:2456`, `purge.rs:357` | `remove_project` calls `delete_all` | DEC-002, DEC-003 |
- Exploit tests read the absence of effect on a second channel: a direct `COUNT(*)` on the shared
  database (BT-005, BT-010), the Tantivy directory listing (BT-006, BT-011), the bytes of a file read
  back (BT-008), `search.status` counts (BT-006, BT-009).
- Nothing secret is logged; the removal logs the project id and the counts returned by `delete_all`.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-005 | exploit | tool calls + direct count, PG and SQL Server | `tools/teardown_e2e.rs` | US-0025 |
| BT-008 | exploit | tool calls + purge cycle + `fs.read_bytes`, PG | `tools/teardown_e2e.rs` | US-0025 |
| BT-010 | integration | delete + direct counts, PG and SQL Server | `tools/teardown_e2e.rs` | US-0025 |
| BT-012 | functional e2e | `GET /exports/{token}` | `tools/teardown_e2e.rs` | US-0025 |
| BT-001 | functional e2e | tool calls, PG metadata, BM25 | `tools/project_removal_e2e.rs` | US-0026 |
| BT-002 | functional e2e | tool calls, SQLite, BM25 (in `make check`) | `tools/project_removal_e2e.rs` | US-0026 |
| BT-006 | exploit | tool calls per backend + directory listing | `tools/project_removal_e2e.rs` | US-0026 |
| BT-007 | exploit | grace purge path | `tools/project_removal_e2e.rs` | US-0026 |
| BT-009 | exploit | rag on sqlite-vec, stub embeddings (`search/e2e.rs`) | `tools/project_removal_e2e.rs` | US-0026 |
| BT-011 | integration | delete + PG rows + directory | `tools/project_removal_e2e.rs` | US-0026 |
| BT-013 | functional e2e | failing record removal then retry | `tools/project_removal_e2e.rs` | US-0026 |
| BT-020 | functional e2e | two projects `atlas`, `atlas2` | `tools/project_removal_e2e.rs` | US-0026 |
| BT-021 | functional e2e | soft delete then undelete | `tools/project_removal_e2e.rs` | US-0026 |
| BT-022 | suite | `make check` + three conformance suites | existing | US-0026 |
Red before the fix: BT-005, BT-008, BT-010 on the surviving rows; BT-012 if the surviving row answers
500; BT-001, BT-002, BT-006, BT-007, BT-009, BT-011 on the surviving search data. Fixtures follow
`tools/pg_null_e2e.rs` and `tools/mssql_trash_e2e.rs` (fresh schema or rows, cleanup, skip without DSN)
and `search/e2e.rs` harnesses.

## 10. Decisions log
- **DEC-001:** `storage::meta::volume_tables()` returns every `schema()` table with a `volume_id`
  column; the shared database teardown (`storage/mod.rs:511`) deletes from each in one transaction. A
  unit test asserts the 9 names, so a new table forces a conscious update. **Obliged by:** BR-001,
  BR-004, BR-005, BR-006, BR-007. **Rationale:** the hand written list is the root cause; derive it.
  **Alternatives considered:** extend the literal list; per table cleanup in each store. **Source:**
  existing pattern `git/db.rs:143`, `:581-585`. **Evidence:** `storage/meta.rs:166-318`.
- **DEC-002:** search removal uses the existing `SearchBackend::delete_all(volume_id)`. **Obliged by:**
  BR-002, BR-003. **Rationale:** it already removes rows, the Tantivy directory and the cache entry.
  **Alternatives considered:** a new `drop_volume` trait method. **Source:** existing code
  `search/mod.rs:84`, `search/bm25_sqlite.rs:202-224`. **Evidence:** `search/indexer.rs:68`.
- **DEC-003:** one `purge::remove_project` performs teardown, search wipe, git purge, record deletion,
  in that order, for both `admin.delete_project` and the grace period purge; `sweep_grace_period`
  takes the search backend, `app.rs` passes `state.search`, `cli.rs` builds it with
  `search::build_backend`. **Obliged by:** BR-003, BR-008, BT-007, BT-013. **Rationale:** today the two
  paths duplicate the steps (`server.rs:2456-2464`, `purge.rs:357-359`) and both missed search.
  **Alternatives considered:** add the call in both places. **Source:** design, AGENTS.md "one
  implementation". **Evidence:** cited lines.
- **DEC-004:** no read time filtering by `created_at`. **Obliged by:** BR-003, BR-004, BR-005.
  **Rationale:** filtering leaves the data and the refcount corruption. **Alternatives considered:**
  as stated. **Source:** design. **Evidence:** n/a.
- **DEC-005:** new `#[cfg(test)]` modules `tools/teardown_e2e.rs` and `tools/project_removal_e2e.rs`,
  fixtures following `tools/pg_null_e2e.rs`, `tools/mssql_trash_e2e.rs`, `search/e2e.rs`; shared database
  cases skip without their DSN; BT-002 runs on SQLite in `make check`. **Obliged by:** spec section 4
  drivers. **Rationale:** tool level reproduction. **Alternatives considered:** store level only.
  **Source:** existing code. **Evidence:** cited files.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0025 | Volume teardown removes every metadata row | P1 | BR-001, BR-004, BR-005, BR-006, BR-007 | main | 1 | no |
| US-0026 | Project removal wipes the search index | P1 | BR-001, BR-002, BR-003, BR-007, BR-008 | US-0025 | 1 | yes |
min_tier 1 for both: shared database and search harnesses across three engines, and a cross path refactor in US-0026.
