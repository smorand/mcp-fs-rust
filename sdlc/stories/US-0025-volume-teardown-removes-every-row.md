# US-0025: Volume teardown removes every metadata row

> Spec: SPEC-0024 (sdlc/specs/SPEC-0024-deleted-project-data-survives/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
On PostgreSQL and SQL Server, deleting a project leaves 6 of its 9 metadata tables behind, so a project recreated with the same id lists the old trash (file names, deleter emails), and the purge cycle's artifact sweep can delete the new project's bytes. After this story the teardown removes every metadata row of the volume. Security: internal finding (CWE-226).

## Requirements covered
- SPEC-0024/BR-001: WHEN a project is recreated THE Storage context SHALL return `{"entries": [], "total": 0}` from `fs.trash_list` until a file of the new project is deleted.
- SPEC-0024/BR-004: WHEN `admin.delete_project` or the grace period purge removes a project on a shared database THE Storage context SHALL leave no metadata row of that volume in any of its tables.
- SPEC-0024/BR-005: The Storage context SHALL NOT decrement the reference count of, or delete, any blob of a recreated project because of an artifact set of the deleted project.
- SPEC-0024/BR-006: IF a download link minted by `fs.export_zip` in a deleted project is requested after the project is recreated THEN THE Storage context SHALL answer `GET /exports/{token}` with 404.
- SPEC-0024/BR-007: The Storage and Search contexts SHALL NOT remove or change any row, index entry, file or blob of a project other than the one deleted.
(BR-001 and BR-007 partial: the metadata part; search is US-0026.)

## Independent Test
With `docker compose -f docker-compose.test.yml up -d` and `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`: `cargo test -p mcp-fs-core --all-features --lib -- teardown_e2e storage::meta` green, and `make check` green. BT-005 and BT-010 read a direct `COUNT(*)` on the database, BT-008 reads the file bytes back.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0024): `storage::meta::volume_tables()` returns every `schema()` table with a `volume_id` column (`crates/core/src/storage/meta.rs:166-318`, 9 today: `nodes`, `blob_refs`, `trash_entries`, `volume_usage`, `export_links`, `doc_artifact_current`, `doc_artifact_sets`, `doc_images`, `doc_tables`); the shared database teardown (`crates/core/src/storage/mod.rs:511`, today `for table in ["nodes", "blob_refs", "volume_usage"]`) deletes from each in one transaction. A unit test asserts the 9 names. Pattern: `git/db.rs:143`, `:581-585`.
- DEC-004 (SPEC-0024): no read time filtering by `created_at`; the rows are removed.
- DEC-005 (SPEC-0024): tests in a new `#[cfg(test)]` module `crates/core/src/tools/teardown_e2e.rs`. Fixtures follow `crates/core/src/tools/pg_null_e2e.rs` and `crates/core/src/tools/mssql_trash_e2e.rs` (fresh schema or rows per test, cleanup, `eprintln!` skip when `MCPFS_TEST_PG_DSN` / `MCPFS_TEST_MSSQL_DSN` is unset or the feature is off).
### Security
Occurrences fixed here, all at `crates/core/src/storage/mod.rs:511`: `trash_entries` (read by `fs.trash_list`, `tools/trash.rs:96`); `doc_*` (swept by `storage/meta.rs:818-826`, decref `:1199-1222`, GC `purge.rs:242-246`); `export_links` (`GET /exports/{token}`, `exports.rs`). Absence of effect is read on a second channel: direct counts, file bytes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | add `pub(crate) fn volume_tables() -> Vec<&'static str>` and its unit test |
| `crates/core/src/storage/mod.rs` | teardown iterates `volume_tables()` |
| `crates/core/src/tools/teardown_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod teardown_e2e;` |

## Acceptance tests
> 100% must pass, through `make check` plus the opt in suites above.

### SPEC-0024/BT-005: Trash rows of a deleted project
- **Covers:** `storage/mod.rs:511` (`trash_entries`)
- **Driver:** tool calls, PostgreSQL and SQL Server metadata
- **Red before the fix because:** the row survives and is listed.
- Given the BT-001 state before deletion, with the trash row `original_path: "/memo.txt"`, `deleted_by: "olivia@example.com"`
- When Otto calls `fs.trash_list {mount_id: "atlas"}` on the recreated project
- Then `total: 0`
- And a direct count `SELECT COUNT(*) FROM trash_entries WHERE volume_id='atlas'` read on the database is `0`

### SPEC-0024/BT-008: The artifact sweep spares the new project's blobs
- **Covers:** `storage/meta.rs:818-826`, `:1199-1222`, `purge.rs:242-246`
- **Driver:** tool calls plus the purge cycle, PostgreSQL metadata
- **Red before the fix because:** the old set is swept and decrements the new refcount.
- Given `atlas` converted `/memo.docx` producing image bytes `IMG-A`, then `atlas` deleted and recreated
- When Otto writes `/logo.png` with the bytes `IMG-A` and the purge cycle runs past the artifact grace period
- Then `fs.read_bytes {mount_id: "atlas", path: "/logo.png"}` returns `IMG-A`

### SPEC-0024/BT-010: No metadata row of a deleted volume remains
- **Validates:** BR-004
- Given `atlas` with a file, a trash entry, an export link and a converted document, on PostgreSQL and on SQL Server
- When the project is deleted / Then for each of the 9 metadata tables, `SELECT COUNT(*) ... WHERE volume_id='atlas'` is 0.

### SPEC-0024/BT-012: An old export link answers 404
- **Validates:** BR-006
- Given `atlas` minted an export link with `fs.export_zip` / When `atlas` is deleted and recreated and the link is requested / Then `GET /exports/{token}` answers 404.

## Constraints
- Files not to touch: `crates/core/src/search/*`, `crates/core/src/mcp/server.rs`, `crates/core/src/purge.rs` (US-0026), `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Patterns to avoid: a hand written table list; filtering reads.
- Every DELETE carries `volume_id` (AGENTS.md); deleting `atlas` must not touch `atlas2`.

## Definition of done
- [ ] RED for BT-005, BT-008, BT-010 (and BT-012 if the surviving row answers 500) against the compose databases before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: `teardown_on_a_shared_database_drops_the_usage_row` (`storage/mod.rs:600-630`), SQLite teardown tests, the three conformance suites.
- [ ] Self review: full, plus the security rules (literal state, rejection, absence of effect on a second channel).
- [ ] One commit `US-0025: Volume teardown removes every metadata row`, body `Spec: SPEC-0024`.
