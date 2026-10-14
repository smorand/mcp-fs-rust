# SPEC-0026: Opening a store on a shared PostgreSQL schema blocks and deadlocks, design

> Spec: sdlc/specs/SPEC-0026-postgres-open-blocks-and-deadlocks/sdlc-spec.md
> Nature: BUG
> Depth: S
> Security: n/a
> Designed on: 2026-10-14
> Target tier: 2 (default); US-0029 raised to 1, see section 11

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| PostgreSQL driver | sqlx, existing | sqlx-postgres 0.9.0 (`Cargo.lock`) | DEC-001 |
| Retry | existing `MAX_TX_ATTEMPTS` and `ToolError::retryable` | `crates/core/src/storage/rel/mod.rs:340` | DEC-003 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: probe the catalog, apply only what is missing, serialized and retried
`PostgresRelationalDb::migrate` (`crates/core/src/storage/rel/postgres.rs:319-337`) first reads the
catalog of its schema for every table, index (plain and typed) and column migration the `SchemaSet`
declares. When all exist it returns without running any DDL, so no table lock is taken (DEC-001).
Otherwise it takes `pg_advisory_xact_lock` on a key derived from the schema name inside its
transaction, probes again, and runs the DDL (DEC-002). The whole application is retried on a
retryable error up to `MAX_TX_ATTEMPTS` (DEC-003). This is the SQL Server approach
(`schema.rs:281-286`, `dialect.rs:446-450`), done once per open instead of per statement.
### Rejected: per statement catalog guards in the rendered DDL (`DO $$ ... IF NOT EXISTS ...`)
Mirrors SQL Server literally, but changes the rendered PostgreSQL DDL text that tests and
`mcp-fs migrate` share, and still sends one round trip per object. DEC-001.
### Rejected: advisory lock only
Serializes opens (no deadlock between them) but every open still waits behind writers on `nodes`
(BR-001, BT-002 fail). DEC-001.

## 3. Architecture
Only the PostgreSQL engine's `migrate` changes, plus a read helper on `SchemaSet` listing what it
declares. Stores call `migrate` unchanged (`storage/meta.rs:476`, `storage/admin.rs:211`,
`git/db.rs:237`, `git/oauth/store.rs:784`). SQLite and SQL Server engines are untouched.

## 4. Data model
No change. The produced schema is identical.

## 5. Contracts and interfaces
- `impl SchemaSet { pub fn declared_objects(&self) -> DeclaredObjects }` in `crates/core/src/storage/rel/schema.rs`:
  table names, index names (plain and typed), and `(table, column)` pairs of column migrations.
- `async fn with_schema_retry<F, Fut>(attempt: F) -> Result<()>` in `postgres.rs`: runs `attempt` up to
  `MAX_TX_ATTEMPTS` while the error is `retryable`, so the retry is unit tested with a closure (DEC-003).
- Errors unchanged: `ERR_INTERNAL_ERROR`, `postgres: deadlock detected [40P01]`, `retryable: true`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/rel/postgres.rs` | probe, advisory lock, retry; BT-010, BT-011, BT-012 tests | BR-001 to BR-004 | US-0029 |
| `crates/core/src/storage/rel/schema.rs` | `declared_objects` and its unit test | BR-001 | US-0029 |
| `crates/core/src/tools/pg_open_e2e.rs` | new, BT-001, BT-002 | BR-001, BR-002 | US-0029 |
| `crates/core/src/tools/mod.rs` | register the module | DEC-004 | US-0029 |
| `.agent_docs/backends.md` | PostgreSQL migrate rule, 2026-10-14 | BR-001 | US-0029 |
Affected existing tests: kept (spec 4.4).

## 7. Compatibility and risks
A partially migrated database (an older version) still gets its missing objects on the first open,
under the advisory lock. The probe reads `information_schema` / `pg_indexes` of the configured schema
only. Rollback: revert.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | concurrent `fs.write_bytes`, PostgreSQL meta store | `tools/pg_open_e2e.rs` | US-0029 |
| BT-002 | functional e2e | `fs.write_bytes` while a second sqlx connection holds an uncommitted insert on `nodes` | `tools/pg_open_e2e.rs` | US-0029 |
| BT-010, BT-011 | unit | `with_schema_retry` with a closure failing with a 40P01 `ToolError` | `rel/postgres.rs` tests | US-0029 |
| BT-012 | integration | live migrate on an empty schema, twice, then on `nodes` without `rev` | `rel/postgres.rs` tests | US-0029 |
| BT-020 | suite | `make check`, SQL Server conformance with DSN, rendered DDL unchanged | existing + `schema.rs` test | US-0029 |
| BT-021 | suite | PostgreSQL storage and conformance suites, 5 runs | command in the story | US-0029 |
PostgreSQL tests use their own schema per test (`tools/pg_null_e2e.rs` pattern) and skip without
`MCPFS_TEST_PG_DSN`. Red before: BT-001 (40P01 on some call), BT-002 (timeout beyond 2 s), BT-010
(first failure returned).

## 10. Decisions log
- **DEC-001:** migrate probes the catalog for every declared table, index and migrated column, and
  returns before any DDL when all exist. **Obliged by:** BR-001, BR-002. **Rationale:** the only way an
  open takes no table lock; one round trip set. **Alternatives considered:** per statement `DO`
  guards; advisory lock only. **Source:** design, SQL Server parity. **Evidence:** live probe in the
  spec; `schema.rs:278-286`.
- **DEC-002:** when something is missing, the transaction takes `pg_advisory_xact_lock(<key of the
  schema name>)`, probes again, then applies the DDL. **Obliged by:** BR-002, BR-004. **Rationale:**
  two first opens of an old database cannot interleave their DDL. **Source:** design.
- **DEC-003:** `with_schema_retry` reruns the whole application while the error is `retryable`, up to
  `MAX_TX_ATTEMPTS`, after `apply_table_recreations` as today. **Obliged by:** BR-003. **Rationale:**
  SPEC-0005/SC-402 retry rule; testable with a closure. **Source:** existing code
  `rel/mod.rs:340-360`, `postgres.rs:150-153`.
- **DEC-004:** tool level tests in a new `#[cfg(test)]` module `tools/pg_open_e2e.rs`, fixture after
  `tools/pg_null_e2e.rs`. **Obliged by:** BT-001, BT-002. **Source:** existing code.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0029 | PostgreSQL store opens without locking a current schema | P1 | BR-001..BR-005 | main | 1 | yes |
min_tier 1: concurrency tests against a live database and catalog introspection across typed indexes.
