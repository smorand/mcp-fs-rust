# SPEC-0022: PostgreSQL refuses absent values in numeric columns, design

> Spec: sdlc/specs/SPEC-0022-postgres-null-values-refused/sdlc-spec.md
> Nature: BUG
> Depth: S
> Security: n/a
> Designed on: 2026-10-12
> Target tier: 2 (default); US-0023 raised to 1, see section 11

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| PostgreSQL driver | sqlx, existing (`postgres` feature) | sqlx-postgres 0.9.0 (`Cargo.lock:5453-5454`) | DEC-001 |
| Tests | `cargo test`, opt in PostgreSQL via `MCPFS_TEST_PG_DSN` | existing | DEC-003 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: bind an untyped null at the root cause site
`crates/core/src/storage/rel/postgres.rs:160` binds `SqlValue::Null` as a parameter whose declared
type is unspecified (OID 0), so PostgreSQL infers it from the column or expression, as it does for a
literal `NULL`. One site fixes all 7 call sites and any future one (BR-006).
### Rejected: cast at each call site
`CAST(?n AS BIGINT)` in the 5 statements. Rejected (DEC-001): 7 edits, and the next nullable numeric
column reintroduces the bug.
### Rejected: typed null from the caller
A `SqlValue::NullOf(type)` variant. Rejected (DEC-001): every caller of `.bind(Option<_>)` would have to
change, across three engines.

## 3. Architecture
Only the PostgreSQL value mapping changes (`rel/postgres.rs:156-166`). SQLite (`rel/sqlite.rs:52`) and
SQL Server (`rel/sqlserver.rs:204`) are untouched. Stores call the layer unchanged.

## 4. Data model
No change.

## 5. Contracts and interfaces
No tool contract change. Internal: a private type in `rel/postgres.rs` implementing
`sqlx::Type<Postgres>` (type info `PgTypeInfo::with_oid(Oid(0))`, `sqlx-postgres-0.9.0/src/type_info.rs:336`)
and `sqlx::Encode<Postgres>` returning `IsNull::Yes`, used only for `SqlValue::Null`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/rel/postgres.rs` | modify null binding, add live test | BR-006, BT-020 | US-0023 |
| `crates/core/src/storage/conformance.rs` | add null quota and null retention cases (all engines) | BR-001, BR-002, BR-003, BT-002 | US-0023 |
| `crates/core/src/tools/pg_null_e2e.rs` | new, tool level PostgreSQL tests | BT-001, BT-010 to BT-013 | US-0023 |
| `crates/core/src/tools/mod.rs` | register the test module | DEC-003 | US-0023 |
| `.agent_docs/backends.md` | dialect checklist line on null binding | BR-006 | US-0023 |
Affected existing tests: all kept (spec 4.4).

## 7. Compatibility and risks
Risk: a null in an expression PostgreSQL cannot type (for example `SELECT ?1` alone) is now inferred as
text by the server, the same result as today. Statements that compare a null parameter with `=` were
already wrong on every engine; none exist (sweep, spec section 1). Rollback: revert the commit.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | `admin.set_project_quota`, `admin.get_project_quota` tool calls, PostgreSQL admin store | `crates/core/src/tools/pg_null_e2e.rs` | US-0023 |
| BT-002 | integration | `postgres_passes_the_conformance_suite` | `crates/core/src/storage/conformance.rs` | US-0023 |
| BT-010 | functional e2e | `admin.set_purge_config` | `pg_null_e2e.rs` | US-0023 |
| BT-011 | functional e2e | `admin.create_project` | `pg_null_e2e.rs` | US-0023 |
| BT-012 | functional e2e | `fs.documentize`, `fs.list_images`, stub conversion | `pg_null_e2e.rs` | US-0023 |
| BT-013 | functional e2e | same, reconversion | `pg_null_e2e.rs` | US-0023 |
| BT-020 | integration | live PostgreSQL round trip | `rel/postgres.rs` tests | US-0023 |
| BT-021 | suite | `make check` (SQLite) | existing | US-0023 |
| BT-022 | integration | SQL Server conformance, `MCPFS_TEST_MSSQL_DSN` | `conformance.rs` | US-0023 |
BT-001, BT-002, BT-010 to BT-013 are RED today with `ERR_INTERNAL_ERROR` `... is of type bigint but
expression is of type text [42804]`. Infra: `docker compose -f docker-compose.test.yml up -d`.

## 10. Decisions log
- **DEC-001:** bind `SqlValue::Null` on PostgreSQL as an untyped parameter (OID 0) through a private
  `sqlx::Type`/`Encode` impl, at `rel/postgres.rs:160`. **Obliged by:** SPEC-0022/BR-001 to BR-006.
  **Rationale:** fixes the root cause once, for every current and future nullable non text column.
  **Alternatives considered:** per site casts; a typed null variant. **Source:** design.
  **Evidence:** `sqlx-postgres-0.9.0/src/type_info.rs:336` (`with_oid`); psql probe in the spec.
- **DEC-002:** the conformance suite gains cases clearing a quota and saving a purge config and a
  project with null retention axes, so the three engines are held to one result. **Obliged by:**
  SPEC-0022/BR-006, BT-002, BT-022. **Rationale:** the suite is the parity mechanism
  (`conformance.rs:1045`). **Alternatives considered:** PostgreSQL only tests. **Source:** existing
  code `conformance.rs`. **Evidence:** spec section 1, 0 hits for `set_quota(`/`set_purge_config(`.
- **DEC-003:** tool level PostgreSQL tests live in a new `#[cfg(test)]` module `tools/pg_null_e2e.rs`,
  building the tools harness (`tools/mod.rs:288`) with `infra.admin` and `infra.meta` set to the
  `postgres` backend from `MCPFS_TEST_PG_DSN`, one fresh schema per test, skipped with a message when
  the variable is unset or the `postgres` feature is off (the `conformance.rs:1035-1042` pattern).
  BT-012/013 reuse the stub conversion of `spec_0019_e2e_115` (`tools/artifacts.rs:1770`).
  **Obliged by:** BT-001, BT-010 to BT-013 (driver: tool calls). **Rationale:** reproduces the user
  symptom, not a store call. **Alternatives considered:** store level only. **Source:** design.
  **Evidence:** `crates/core/src/config.rs:15`, `:351-354`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0023 | Absent values accepted on PostgreSQL | P1 | BR-001..BR-006 | main | 1 | yes |
min_tier 1: one root cause fix cannot be split from its tests, and the tool harness wiring to a
PostgreSQL store plus the sqlx untyped parameter need repository reading a tier 2 brief does not carry.
