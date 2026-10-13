# US-0024: SQL Server metadata store opens and lists the trash

> Spec: SPEC-0023 (sdlc/specs/SPEC-0023-sqlserver-trash-index-refused/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 6
> Closes spec: yes

## Objective
A metadata store on SQL Server cannot be created: schema setup fails with error 1919 because
`trash_entries.deleted_at` is unbounded text inside an index. After this story the store opens, the
trash lists on SQL Server, and a guard in `make check` stops any unbounded text key from coming back.

## Requirements covered
- SPEC-0023/BR-001: WHEN a metadata store is opened on a SQL Server database THE Storage context SHALL apply its schema without error, and applying it again SHALL change nothing.
- SPEC-0023/BR-002: WHEN `fs.trash_list` is called on a SQL Server metadata store THE Storage context SHALL return the entries ordered by `deleted_at` descending, each `deleted_at` the RFC 3339 value recorded at deletion.
- SPEC-0023/BR-003: The Storage context SHALL pass, on SQL Server, every case of the conformance suite that it passes on SQLite.
- SPEC-0023/BR-004: The Storage context SHALL NOT change the value or the listing order of any `deleted_at` already stored by a SQLite or PostgreSQL metadata store.

## Independent Test
With `docker compose -f docker-compose.test.yml up -d`:
`MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' cargo test -p mcp-fs-core --all-features --lib -- conformance mssql_trash_e2e storage::rel::schema`
all green, and `make check` green. BT-001 reads the result back through `fs.trash_list`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0023): `trash_entries.deleted_at` becomes `ColumnType::TextKey(TIMESTAMP_LEN)` with `const TIMESTAMP_LEN: u32 = 64;` in `crates/core/src/storage/meta.rs` (today `meta.rs:207` `Column::required("deleted_at", ColumnType::Text)`). `TextKey` renders `NVARCHAR(n)` on SQL Server and `TEXT` elsewhere (`rel/dialect.rs:218`, `:226`, `:235`).
- DEC-002 (SPEC-0023): a unit test in `crates/core/src/storage/rel/schema.rs` walks the four production schemas (`storage::meta::schema`, `storage::admin::schema`, `git::db::schema`, `git::oauth::persistence::schema`) and fails when any primary key or index column is `ColumnType::Text`, naming table and column.
- DEC-003 (SPEC-0023): BT-001 in a new `#[cfg(test)]` module `crates/core/src/tools/mssql_trash_e2e.rs`, state built with the meta store on SQL Server, following `crates/core/src/tools/pg_null_e2e.rs` (fixture, isolation, cleanup, skip with `eprintln!` when `MCPFS_TEST_MSSQL_DSN` is unset or feature `sqlserver` off).
- DEC-004 (SPEC-0023): BT-021 compares the SQLite and PostgreSQL rendering of `trash_entries` with a literal of today's DDL.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify `deleted_at` type, add the const |
| `crates/core/src/storage/rel/schema.rs` | DEC-002 guard, DEC-004 BT-021 |
| `crates/core/src/storage/conformance.rs` | BT-010, BT-011 cases |
| `crates/core/src/tools/mssql_trash_e2e.rs` | new, BT-001 |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod mssql_trash_e2e;` |
| `.agent_docs/backends.md` | dialect checklist note, 2026-10-12 |

## Acceptance tests
> 100% must pass, through `make check` plus the opt in suites above.

### SPEC-0023/BT-001: A file deleted on a SQL Server store is listed in the trash
- Red before the fix because: the store cannot open, `Column 'deleted_at' in table 'trash_entries' is of a type that is invalid for use as a key column in an index. [1919]`.
- Given project `atlas` with a fresh SQL Server metadata database
- When the owner writes `/a.txt` (3 bytes), then `/b.txt`, deletes `/a.txt`, then `/b.txt`
- Then each call succeeds; `fs.trash_list {mount_id: "atlas"}` returns `total: 2`, `original_path` `/b.txt` then `/a.txt`

### SPEC-0023/BT-002: The conformance suite passes on SQL Server
- Red before the fix because: panic at `conformance.rs:1110` with 1919.
- `sqlserver_passes_the_conformance_suite` passes, the SPEC-0022 null case included.

### SPEC-0023/BT-010: Schema applied twice on SQL Server
- Conformance case on every engine: migrate the metadata schema twice / Then no error, table and index sets unchanged.

### SPEC-0023/BT-011: Recorded timestamp returned verbatim
- Conformance case on every engine: record entries with `deleted_at` `2026-10-12T22:11:27.123456+00:00` and `2026-10-11T08:00:00.000000+00:00` / list / Then values byte for byte, the 2026-10-12 entry first.

### SPEC-0023/BT-020: SQLite and PostgreSQL trash listing unchanged
- `make check` and `postgres_passes_the_conformance_suite` green.

### SPEC-0023/BT-021: Existing SQLite and PostgreSQL stores need no migration
- The rendered `CREATE TABLE` for `trash_entries` on SQLite and on PostgreSQL equals the literal produced by today's code (capture it before the change).

## Constraints
- Files not to touch: `rel/dialect.rs`, `rel/sqlserver.rs`, `rel/postgres.rs`, `rel/sqlite.rs`, `tools/trash.rs`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Patterns to avoid: removing the index; changing the stored timestamp format.
- If the DEC-002 guard finds another unbounded key in another schema: fix it the same way only if it is in `storage/meta.rs`; otherwise write a backlog entry and mark that pair as a declared exception inside the guard.

## Definition of done
- [ ] RED for BT-001, BT-002 against the compose SQL Server, and for the DEC-002 guard, before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: trash tests in `tools/trash.rs`, `trash_screen.rs`, all three conformance suites.
- [ ] Docs updated: `.agent_docs/backends.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0023 green; BR-001..BR-004 cited.
- [ ] One commit `US-0024: SQL Server metadata store opens and lists the trash`, body `Spec: SPEC-0023`.
