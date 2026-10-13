# SPEC-0023: SQL Server metadata store cannot be created, design

> Spec: sdlc/specs/SPEC-0023-sqlserver-trash-index-refused/sdlc-spec.md
> Nature: BUG
> Depth: S
> Security: n/a
> Designed on: 2026-10-12
> Target tier: 2 (default); US-0024 raised to 1, see section 11

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| SQL Server driver | tiberius-ng + bb8, existing (`sqlserver` feature) | `Cargo.lock` | n/a |
| Tests | `cargo test`, opt in SQL Server via `MCPFS_TEST_MSSQL_DSN` | existing | DEC-003 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: bound the column at the root cause site
`crates/core/src/storage/meta.rs:207` declares `deleted_at` as `ColumnType::TextKey(TIMESTAMP_LEN)`.
`TextKey` renders `NVARCHAR(n)` on SQL Server and `TEXT` on SQLite and PostgreSQL
(`crates/core/src/storage/rel/dialect.rs:218`, `:226`, `:235`), so only SQL Server DDL changes.
### Rejected: drop or change the index
Removing `deleted_at` from `idx_trash_entries_deleted_at` breaks SPEC-0013's stated index (`spec.md:111`).
### Rejected: store an integer timestamp
Changes the stored value on SQLite and PostgreSQL (violates BR-004) and needs a data migration.

## 3. Architecture
Metadata schema only (`meta.rs:166-318`). No store, tool or dialect code changes. A schema level guard
(DEC-002) checks every production schema (`storage/meta.rs:166`, `storage/admin.rs:27`,
`git/db.rs:164`, `git/oauth/persistence.rs:37`).

## 4. Data model
`trash_entries.deleted_at`: before `ColumnType::Text` (`NVARCHAR(MAX)` on SQL Server), after
`ColumnType::TextKey(64)` (`NVARCHAR(64)` on SQL Server, `TEXT` unchanged elsewhere). Longest value
written today: 32 characters (`util/mod.rs:23-25`). Migration: none. SQLite and PostgreSQL render the
same DDL, so existing tables already match; no SQL Server database holds the table
(`rel/sqlserver.rs:455-467` rolls back). Rollback: revert the commit.

## 5. Contracts and interfaces
No tool contract change. Internal: `const TIMESTAMP_LEN: u32 = 64;` in `meta.rs`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | `deleted_at` to `TextKey(TIMESTAMP_LEN)` | BR-001 | US-0024 |
| `crates/core/src/storage/rel/schema.rs` | guard test over every production schema | BR-003, DEC-002 | US-0024 |
| `crates/core/src/storage/conformance.rs` | apply twice case, verbatim timestamp ordering case (all engines) | BR-001, BR-002, BT-010, BT-011 | US-0024 |
| `crates/core/src/tools/mssql_trash_e2e.rs` | new, tool level SQL Server test | BT-001 | US-0024 |
| `crates/core/src/tools/mod.rs` | register the test module | DEC-003 | US-0024 |
| `.agent_docs/backends.md` | dialect checklist note, timestamped | BR-003 | US-0024 |
Affected existing tests: all kept (spec 4.4).

## 7. Compatibility and risks
SQLite and PostgreSQL DDL byte identical before and after (BT-021 asserts it). A value longer than 64
characters would be refused on SQL Server only; none is written (`util/mod.rs:23-25`,
`tools/trash.rs:155-167`).

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | `fs.write_bytes`, `fs.delete`, `fs.trash_list` on a SQL Server meta store | `crates/core/src/tools/mssql_trash_e2e.rs` | US-0024 |
| BT-002 | integration | `sqlserver_passes_the_conformance_suite` | `storage/conformance.rs` | US-0024 |
| BT-010 | integration | conformance case, apply twice | `storage/conformance.rs` | US-0024 |
| BT-011 | integration | conformance case, verbatim `deleted_at`, order | `storage/conformance.rs` | US-0024 |
| BT-020 | suite | `make check` and `postgres_passes_the_conformance_suite` | existing | US-0024 |
| BT-021 | unit | rendered SQLite and PostgreSQL DDL for `trash_entries` unchanged | `storage/rel/schema.rs` tests | US-0024 |
BT-001, BT-002 RED today with error 1919. Infra: `docker compose -f docker-compose.test.yml up -d`.

## 10. Decisions log
- **DEC-001:** `trash_entries.deleted_at` becomes `ColumnType::TextKey(TIMESTAMP_LEN)` with
  `TIMESTAMP_LEN = 64`. **Obliged by:** SPEC-0023/BR-001, BR-002, BR-004. **Rationale:** the existing
  bounded key mechanism (`dialect.rs:21-22`); twice the longest value written. **Alternatives
  considered:** drop the index column; integer timestamp. **Source:** existing code `dialect.rs:21-28`.
  **Evidence:** `util/mod.rs:23-25`.
- **DEC-002:** a unit test in `rel/schema.rs` walks the four production schemas and fails when any
  primary key or index column is `ColumnType::Text`, naming table and column. **Obliged by:**
  SPEC-0023/BR-003 (the class behind the bug), the spec's assumption in section 6. **Rationale:** runs
  in `make check`, unlike the opt in SQL Server suite. **Alternatives considered:** rely on the SQL
  Server suite only. **Source:** design. **Evidence:** spec contributing factors.
- **DEC-003:** BT-001 lives in a new `#[cfg(test)]` module `tools/mssql_trash_e2e.rs` that builds
  application state with the meta store on SQL Server, following `tools/pg_null_e2e.rs` (fresh database
  objects per test, cleanup, skip with `eprintln!` when `MCPFS_TEST_MSSQL_DSN` is unset or the
  `sqlserver` feature is off). **Obliged by:** BT-001 (driver: tool calls). **Rationale:** reproduces the
  user symptom. **Alternatives considered:** conformance only. **Source:** existing code `tools/pg_null_e2e.rs`.
  **Evidence:** n/a.
- **DEC-004:** BT-021 asserts the SQLite and PostgreSQL rendering of `trash_entries` is unchanged by
  comparing it with a literal of today's DDL. **Obliged by:** SPEC-0023/BR-004. **Rationale:** proves
  no migration is needed for existing stores. **Alternatives considered:** a two version upgrade test.
  **Source:** design. **Evidence:** `dialect.rs:218`, `:226`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0024 | SQL Server metadata store opens and lists the trash | P1 | BR-001..BR-004 | main | 1 | yes |
min_tier 1: 6 files and a SQL Server state harness the repository does not yet have.
