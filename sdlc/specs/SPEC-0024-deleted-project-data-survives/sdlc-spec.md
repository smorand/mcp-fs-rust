# A deleted project's data survives and reaches the next project with the same id: Bug Specification

> Generated on: 2026-10-13
> Id: SPEC-0024
> Nature: BUG
> Depth: M
> Depth evidence: 2 modules (volume teardown in the storage layer, search indexes), 3 deletion entry points (`admin.delete_project`, grace period purge, the purge cycle that sweeps artifacts), 8 requirements, no public contract change, no data model change
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0048
> Security: internal finding
> CVSS: not scored
> Affected: every deployment; search on every engine since search shipped (SPEC-0008); trash and artifact rows on PostgreSQL and SQL Server since SPEC-0013 and SPEC-0019
> Fixed in: unreleased

`--security` applied without the flag: the defect discloses one tenant's file names, member emails and
file content to whoever owns a later project with the same id (CWE-226).

## 1. Defect

### BUG-001: deleting a project leaves its trash, artifact and search data behind
- **Observed behavior:**
  - `admin.delete_project` (`crates/core/src/mcp/server.rs:2446-2467`) and the grace period purge
    (`crates/core/src/purge.rs:345-362`) both call `StoreManager::teardown_volume`
    (`crates/core/src/storage/mod.rs:468-520`), then the git purge, then delete the project record.
  - On SQLite metadata, teardown deletes the volume file (`storage/mod.rs:479-490`), so every
    metadata row goes with it.
  - On PostgreSQL and SQL Server metadata (one database for every volume), teardown deletes rows of
    3 tables only: `for table in ["nodes", "blob_refs", "volume_usage"]` (`storage/mod.rs:511`). The
    metadata schema declares 9 tables keyed by `volume_id` (`crates/core/src/storage/meta.rs:166-318`:
    `nodes`, `blob_refs`, `trash_entries`, `volume_usage`, `export_links`, `doc_artifact_current`,
    `doc_artifact_sets`, `doc_images`, `doc_tables`). 6 survive.
  - Neither deletion path touches the search indexes, on any engine: no call into `crates/core/src/search/`
    from `server.rs:2446-2467` or `purge.rs:345-362`. The PostgreSQL tables `search_fts` and
    `search_chunks` (`search/bm25_pg.rs`, `search/vector_pg.rs`) keep the rows, the Tantivy directory per
    volume (`search/bm25_sqlite.rs:17-27`, `:52-94`) stays on disk, the open Tantivy index cache keyed
    by volume id (`search/bm25_sqlite.rs:40`) stays in memory, and so does the sqlite-vec index, an in
    memory connection per volume id (`search/vector_sqlite.rs:36-37`, opened at `:64`) that lives
    until the process restarts. The only whole volume wipe, `delete_all`, is called
    on an index mode change only (`search/indexer.rs:68`).
- **Expected per authority:**
  - SPEC-0002/FR-027: "WHEN project deletion succeeds THE server SHALL have torn down the volume, purged
    the project's source-control indexing state when that capability is enabled, and deleted the
    access-control record and every membership" (`sdlc/specs/SPEC-0002-platform-foundation/spec.md:296`).
  - The project's own rule for shared databases: `volume_id` "belongs in EVERY WHERE clause: omitting it
    leaks or corrupts another volume's rows" (`AGENTS.md`, Conventions).
  - The stated intent at the git purge: "a project recreated under the same id inherited stale refs.
    Purge it." (`crates/core/src/tools/admin.rs:380-381`); the same is done for git rows through
    `git::db::TABLES` (`crates/core/src/git/db.rs:143`).
- **Reproduction:** a PostgreSQL metadata store with BM25 search enabled. Project `atlas` (owner Olivia,
  index mode `bm25`) holds `/plan.txt` containing `merger with Borealis`; Olivia deletes `/memo.txt`.
  The admin deletes `atlas`, then creates `atlas` again for owner Otto. Otto calls
  `search.query {mount_id: "atlas", query: "Borealis"}` and `fs.trash_list {mount_id: "atlas"}`.
  - Expected: no results, `total: 0`.
  - Actual: a result with `path: "/plan.txt"` and `chunk` containing `merger with Borealis`, and a
    trash entry with `original_path: "/memo.txt"` and `deleted_by: "olivia@example.com"`.
  - With SQLite metadata the trash is empty, but the search result is still returned.
- **Root cause:**
  - `crates/core/src/storage/mod.rs:511` names 3 of the 9 volume tables instead of all of them.
  - `crates/core/src/mcp/server.rs:2456-2464` and `crates/core/src/purge.rs:357-359` do not remove the
    volume's search index.
- **Contributing factors:**
  - On SQLite the metadata gap is masked, because the file is deleted. Every default test runs on
    SQLite, and the shared database suites are opt in.
  - The table list is hand written, while the git side derives its purge from `TABLES` and tests the
    count (`git/db.rs:581-585`).
- **Impact:**
  - **Disclosure:** a later owner of the same project id reads the previous project's file content
    (search chunks), file names, sizes and the emails of members who deleted files (trash rows).
  - **Integrity:** on a recreated volume, the purge cycle's artifact sweep (`purge.rs:242-246`) selects
    the old project's artifact sets as stale. Their nodes are gone, so the left join matches nothing
    (`storage/meta.rs:818-826`). It then decrements the new project's `blob_refs` for any blob with the
    same content (`storage/meta.rs:1199-1222`), and deletes those blobs from the new bucket when the
    count reaches 0 (`purge.rs:244-246`). A new file whose content equals an old artifact (an image, the
    Markdown text) can lose its bytes.
  - **Retention:** a deleted project's search data is never removed, even when no project reuses the id.
  - **Workaround:** none for users; an operator can delete the rows and directories by hand.
- **Rollback posture:** the fix only removes data of deleted projects. A regression must not touch
  live projects; reverting restores today's behavior. Soft deleted projects inside their grace period
  must stay restorable with `admin.undelete_project` (teardown happens only at hard deletion).

## 1bis. Class Sweep

**Class:** data stored under a reusable tenant identifier that is not removed when the tenant is
deleted, so the next tenant created with that identifier is served it (CWE-226, CWE-459).

**Searches run:**
1. Relational tables keyed by the tenant id, against the teardown deletes:
   `rg -n 'Table::new\(' -A1 crates/core/src/storage/meta.rs` (9 tables), then
   `rg -n 'for table in' crates/core/src/storage/mod.rs` (3 names). The same check was run for
   `storage/admin.rs`, `git/db.rs` and `git/oauth/persistence.rs` schemas.
2. Indexes and files derived from the tenant id outside the relational store:
   `rg -n 'delete_all\(|remove_dir_all|tantivy_dir' crates/core/src` against the deletion paths in
   `mcp/server.rs` and `purge.rs`.
3. In memory caches keyed by the tenant id: `StoreManager` clients (`storage/mod.rs:446-472`),
   `GitRepoStore` entries (`git/repo.rs:104-144`), the Tantivy index cache (`search/bm25_sqlite.rs:40-52`),
   the sqlite-vec connection cache (`search/vector_sqlite.rs:36-37`); confirmed by
   `rg -n 'HashMap<String,' crates/core/src/storage crates/core/src/search crates/core/src/git`.
4. Blob keys outside the file tree: `export:{token}`, `git:{sha}`, removed with the bucket (`storage/mod.rs:476`).

Not searched: session write budgets keyed by person and project. They live in memory only, and expire
with the session.

| Occurrence | Verdict | Disposition |
|---|---|---|
| `trash_entries` rows, shared database (`storage/mod.rs:511`; read by `fs.trash_list`, `tools/trash.rs:96`) | vulnerable: old `original_path`, `size`, `deleted_by` listed | BR-001 |
| search index rows and files, every engine (`server.rs:2456-2464`, `purge.rs:357-359`; read by `search.query`, `server.rs:2923-2934`) | vulnerable: old `chunk` text returned | BR-002 |
| `doc_artifact_sets`, `doc_images`, `doc_tables`, `doc_artifact_current` rows, shared database | safe from reading: the current pointer joins on the node revision, a random UUID (`storage/meta.rs:121-122`, `:917-928`); vulnerable to the sweep decrementing new refcounts (`storage/meta.rs:818-826`, `:1199-1222`) | BR-004, BR-005 |
| `export_links` rows, shared database | the old token is unusable today, because the blob went with the bucket (`storage/mod.rs:476`); the row still lingers | BR-004, BR-006 |
| git index rows, bare repository, git blobs | safe: purged (`storage/mod.rs:353-369`, `git/repo.rs:143-164`) | none |
| project, members, purge config | safe: deleted with the project record (`storage/admin.rs`, cascade checked by its tests) | none |
| volume client cache, git entry cache | safe: removed first (`storage/mod.rs:468-471`, `git/repo.rs:137-144`) | none |
| Tantivy index cache in memory (`search/bm25_sqlite.rs:40`) | vulnerable: the same process reattaches the old index | BR-002 |
| sqlite-vec connection cache in memory (`search/vector_sqlite.rs:36-37`) | vulnerable: the same process serves the old chunks | BR-002, BT-009 |

## 2. Expected Behavior

All requirements hold on SQLite, PostgreSQL and SQL Server metadata, and with every search mode
(`bm25`, `rag`, `both`) and backend. "Recreated" means `admin.create_project` called with the id of a
project removed by `admin.delete_project` or by the grace period purge.

#### BR-001 [EARS-E]: A recreated project's trash is empty
> WHEN a project is recreated THE Storage context SHALL return `{"entries": [], "total": 0}` from
> `fs.trash_list` until a file of the new project is deleted.
- **Source:** restored from SPEC-0002/FR-027 ("torn down the volume") and `AGENTS.md` volume isolation.

#### BR-002 [EARS-E]: A recreated project's search returns nothing of the old project
> WHEN a project is recreated THE Search context SHALL return no result from `search.query` for content
> that only the deleted project held, and `search.status` SHALL report 0 indexed documents and 0 vector
> chunks until the new project indexes content.
- **Source:** restored from SPEC-0002/FR-027; the search index is volume state.
- **Exact names:** `search.query` response `{"results": [...], "mode_used": ...}`, result fields
  `path`, `score`, `chunk`, `rank`; `search.status` fields include `vector_chunks`, `mode`.

#### BR-003 [EARS-E]: Deletion removes the search index
> WHEN `admin.delete_project` or the grace period purge removes a project THE Search context SHALL
> remove every indexed document and chunk of that project, whether or not the id is reused.
- **Source:** restored from SPEC-0002/FR-027.

#### BR-004 [EARS-E]: Deletion removes every metadata row of the volume
> WHEN `admin.delete_project` or the grace period purge removes a project on a shared database THE
> Storage context SHALL leave no metadata row of that volume in any of its tables.
- **Source:** restored from SPEC-0002/FR-027 and `AGENTS.md` volume isolation.

#### BR-005 [EARS-UB]: A recreated project never loses content to the old project's artifacts
> The Storage context SHALL NOT decrement the reference count of, or delete, any blob of a recreated
> project because of an artifact set of the deleted project.
- **Source:** restored from SPEC-0019 artifact lifecycle (sets are swept only for their own volume's
  files) and `AGENTS.md` ("omitting it leaks or corrupts another volume's rows").

#### BR-006 [EARS-O]: An old export link stays dead
> IF a download link minted by `fs.export_zip` in a deleted project is requested after the project is
> recreated THEN THE Storage context SHALL answer `GET /exports/{token}` with 404.
- **Source:** restored from SPEC-0012 export links ("the token is the credential", single use, tied to
  its project); 404 is the existing answer for an unknown token (`crates/core/src/exports.rs:39`).

#### BR-007 [EARS-UB]: Other projects untouched
> The Storage and Search contexts SHALL NOT remove or change any row, index entry, file or blob of a
> project other than the one deleted.
- **Source:** restored from `AGENTS.md` volume isolation.

#### BR-008 [EARS-S]: Soft deletion keeps everything
> WHILE a project is soft deleted and inside its grace period THE Storage and Search contexts SHALL keep
> its trash, artifacts and search index, so that `admin.undelete_project` restores them unchanged.
- **Source:** restored from SPEC-0014 (soft delete and undelete, `/app/deleted-projects`).

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| SQLite metadata | trash already empty today; search must still be empty | BR-001, BR-002 |
| search disabled (`search.enabled: false`) | deletion succeeds; nothing to remove | BR-003 |
| index mode `none` at deletion, an index left from an earlier mode | that index is removed too | BR-003 |
| project id never recreated | the search index and rows are removed anyway | BR-003, BR-004 |
| new file with the same bytes as an old artifact image | still readable after a purge cycle | BR-005 |
| two projects `atlas` and `atlas2` (prefix of another id) | deleting `atlas` leaves `atlas2` intact | BR-007 |
| deletion interrupted mid way | rerunning `admin.delete_project` (or the next purge) completes it | BR-003, BR-004 |

## 4. Tests

Shared database tests run when `MCPFS_TEST_PG_DSN` and `MCPFS_TEST_MSSQL_DSN` are set, and skip
otherwise. SQLite tests run in `make check`.

### 4.1 End to end reproduction (mandatory)
#### BT-001: A recreated project does not see the old project's trash or search content
- **Reproduces:** BUG-001
- **Driver:** MCP tool calls `fs.write_bytes`, `fs.delete`, `admin.delete_project`,
  `admin.create_project`, `fs.trash_list`, `search.query` on a server with PostgreSQL metadata and BM25 search.
- **Fails today because:** `storage/mod.rs:511` leaves `trash_entries`, and no deletion path removes the search index.
- Given `atlas` (owner `olivia@example.com`, index mode `bm25`) holding `/plan.txt` = `merger with Borealis`, and `/memo.txt` deleted by Olivia
- When the admin deletes `atlas`, creates `atlas` for `otto@example.com`, and Otto calls `fs.trash_list {mount_id: "atlas"}` and `search.query {mount_id: "atlas", query: "Borealis", mode: "bm25"}`
- Then `fs.trash_list` returns `{"entries": [], "total": 0}`
- And `search.query` returns `"results": []`

#### BT-002: Same reproduction on SQLite metadata
- **Reproduces:** BUG-001 (search part, in `make check`)
- Given the BT-001 data on the default SQLite configuration with BM25 / When the same calls / Then `"results": []` and `total: 0`.

### 4.1bis Exploit tests
#### BT-005: Trash rows of a deleted project
- **Covers:** `storage/mod.rs:511` (`trash_entries`)
- **Driver:** tool calls, PostgreSQL and SQL Server metadata
- **Red before the fix because:** the row survives and is listed.
- Given the BT-001 state before deletion, with the trash row `original_path: "/memo.txt"`, `deleted_by: "olivia@example.com"`
- When Otto calls `fs.trash_list {mount_id: "atlas"}` on the recreated project
- Then `total: 0`
- And a direct count `SELECT COUNT(*) FROM trash_entries WHERE volume_id='atlas'` read on the database is `0`

#### BT-006: Search content of a deleted project
- **Covers:** `server.rs:2456-2464`, `purge.rs:357-359` (search index), every backend
- **Driver:** tool calls; one run per backend (`bm25` on Tantivy, `bm25` on PostgreSQL, `rag` on sqlite-vec with the stub embedding server of `search/e2e.rs`)
- **Red before the fix because:** `search.query` returns the old `chunk`.
- Given `atlas` indexed with `/plan.txt` = `merger with Borealis`
- When the project is deleted and recreated, and Otto calls `search.query {mount_id: "atlas", query: "Borealis"}`
- Then `"results": []`
- And `search.status {mount_id: "atlas"}` reports `vector_chunks: 0` and 0 indexed documents, and on disk no Tantivy directory of the old index remains (read by listing the configured `search.tantivy_dir`)

#### BT-007: Grace period purge path
- **Covers:** `purge.rs:345-362`
- Given `atlas` soft deleted and its grace period elapsed / When the purge sweep runs, and `atlas` is recreated / Then BT-001's `fs.trash_list` and `search.query` assertions hold.

#### BT-008: The artifact sweep spares the new project's blobs
- **Covers:** `storage/meta.rs:818-826`, `:1199-1222`, `purge.rs:242-246`
- **Driver:** tool calls plus the purge cycle, PostgreSQL metadata
- **Red before the fix because:** the old set is swept and decrements the new refcount.
- Given `atlas` converted `/memo.docx` producing image bytes `IMG-A`, then `atlas` deleted and recreated
- When Otto writes `/logo.png` with the bytes `IMG-A` and the purge cycle runs past the artifact grace period
- Then `fs.read_bytes {mount_id: "atlas", path: "/logo.png"}` returns `IMG-A`

#### BT-009: In memory vector index of a deleted project
- **Covers:** `search/vector_sqlite.rs:36-37` (per volume connection cache, `rag` mode on sqlite-vec)
- **Driver:** tool calls in one server process, the stub embedding server of `search/e2e.rs`
- **Red before the fix because:** the cached connection still holds the old chunks.
- Given `atlas` in index mode `rag` with `/plan.txt` = `merger with Borealis` indexed
- When `atlas` is deleted and recreated in the same process, and Otto calls `search.query {mount_id: "atlas", query: "Borealis", mode: "rag"}`
- Then `"results": []`
- And `search.status {mount_id: "atlas"}` reports `vector_chunks: 0`

### 4.2 New behavior
#### BT-010: No metadata row of a deleted volume remains
- **Validates:** BR-004
- Given `atlas` with a file, a trash entry, an export link and a converted document, on PostgreSQL and on SQL Server
- When the project is deleted / Then for each of the 9 metadata tables, `SELECT COUNT(*) ... WHERE volume_id='atlas'` is 0.

#### BT-011: The search index is removed even without reuse
- **Validates:** BR-003
- Given `atlas` indexed / When it is deleted / Then the PostgreSQL `search_fts` and `search_chunks` hold no `atlas` row, and the Tantivy directory of `atlas` is absent.

#### BT-012: An old export link answers 404
- **Validates:** BR-006
- Given `atlas` minted an export link with `fs.export_zip` / When `atlas` is deleted and recreated and the link is requested / Then `GET /exports/{token}` answers 404.

#### BT-013: Deletion is retryable
- **Validates:** BR-003, BR-004
- Given a deletion whose project record removal failed after the volume teardown / When `admin.delete_project` is called again / Then it succeeds and BT-010 and BT-011 hold.

### 4.3 Non regression
#### BT-020: Other projects untouched
- **Protects:** BR-007
- Given `atlas` and `atlas2`, both with a file, a trash entry, an indexed document and a converted document
- When `atlas` is deleted / Then `atlas2`'s `fs.trash_list`, `search.query`, `fs.list_images` and file reads return exactly what they returned before.

#### BT-021: Undelete inside the grace period restores everything
- **Protects:** BR-008, SPEC-0014
- Given `atlas` with a trash entry and an indexed document / When it is soft deleted then undeleted / Then `fs.trash_list` and `search.query` return what they returned before.

#### BT-022: Existing suites unchanged
- **Protects:** git purge (`git/db.rs:581-585`), project deletion tests, conformance suites
- `make check` green; the three conformance suites green with their DSNs.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/mod.rs:600-630` | `teardown_on_a_shared_database_drops_the_usage_row` | Kept | still true; extended by BT-010 |
| `crates/core/src/storage/mod.rs:690-700` | teardown tests on SQLite | Kept | unaffected |
| `crates/core/src/git/db.rs:581-585` | the `TABLES` count test | Kept | git purge unchanged |
| `crates/core/src/purge.rs:1109` | grace period purge tests | Kept | extended by BT-007 |
| `crates/core/src/tools/admin.rs` (test module) | `delete_project` tests | Kept | unaffected |

No existing test asserts that data survives deletion.

## 5. Existing Specs Affected
None deviated. Restores SPEC-0002/FR-027 for the trash, artifact, export and search state added by
SPEC-0008, SPEC-0012, SPEC-0013 and SPEC-0019.

## 6. Assumptions
- **Assumption:** the 404 of BR-006 is already today's answer after recreation. The download path
  scans live projects (`exports.rs:100-125`), and a surviving row whose blob is gone may answer 500
  instead. ASSUMED, settled by BT-012.
- **Assumption:** in memory session write budgets need no reset. They are keyed by person and
  project, expire with the session, and grant nothing to a new owner.
- **Overshoot note:** none.
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS (root causes only) |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS (tools, fields, route, status) |
| 4 | No forced choice | PASS |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001 tool calls) |
| 7ante | Security misclassification | PASS (field set to `internal finding`) |
| 7bis | Class sweep real | PASS (class without location; 4 searches; every safe verdict cited; exploit test per vulnerable occurrence BT-005 to BT-009; sqlite-vec cache added after a fresh audit, second channel: direct count or disk listing or byte read) |
| 8 | No hidden prerequisite | PASS (DSNs, search mode in Given) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (8 requirements at M) |
| 11 | Length | PASS (M, compact) |
| 12 | Claims cited | PASS (export status labeled ASSUMED) |
| 13 | Writer and reader read | PASS (teardown writer `storage/mod.rs:511`; readers `tools/trash.rs:96`, `server.rs:2923-2934`, `meta.rs:917-928`) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
