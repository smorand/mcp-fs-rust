# US-0029: PostgreSQL store opens without locking a current schema

> Spec: SPEC-0026 (sdlc/specs/SPEC-0026-postgres-open-blocks-and-deadlocks/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 5
> Closes spec: yes

## Objective
On PostgreSQL every store open reruns `CREATE INDEX IF NOT EXISTS` and `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, which take table locks even when the object exists. The first request to a project then waits behind every write in flight and two opens can deadlock (`postgres: deadlock detected [40P01]`). After this story an open against a current schema takes no table lock, missing objects are added under a schema wide advisory lock, and transient failures are retried.

## Requirements covered
- SPEC-0026/BR-001: WHEN a store opens against a schema that already has every table, index and column it declares THE Storage context SHALL complete the open without waiting for any transaction of another connection.
- SPEC-0026/BR-002: WHEN several stores open against the same schema at the same time THE Storage context SHALL complete every open without error.
- SPEC-0026/BR-003: IF applying the schema fails with a deadlock or serialization failure THEN THE Storage context SHALL retry the whole application, up to 3 attempts, before reporting the error.
- SPEC-0026/BR-004: WHEN a store opens against a schema lacking a declared table, index or column THE Storage context SHALL create it, and a second open SHALL change nothing.
- SPEC-0026/BR-005: The Storage context SHALL NOT change the schema it produces, or the result of opening a store, on SQLite and SQL Server.
(All WHILE the store is PostgreSQL, unless stated.)

## Independent Test
With `docker compose -f docker-compose.test.yml up -d` and `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs`:
`cargo test -p mcp-fs-core --all-features --lib -- pg_open_e2e storage::rel` green, BT-021's 5 runs green, and `make check` green. BT-001 reads every file back with `fs.read_bytes`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0026): `PostgresRelationalDb::migrate` (`crates/core/src/storage/rel/postgres.rs:319-337`) first probes the catalog of its configured schema for every table, index (plain and typed) and migrated column the `SchemaSet` declares (`information_schema.tables`, `pg_indexes`, `information_schema.columns`, filtered on the schema), and returns before any DDL when all exist.
- DEC-002 (SPEC-0026): when something is missing, the transaction takes `pg_advisory_xact_lock` on a key derived from the schema name (for example `hashtext('mcpfs_migrate:' || <schema>)`), probes again, then runs today's DDL statements unchanged (`schema.render`, `render_column_migrations`).
- DEC-003 (SPEC-0026): `async fn with_schema_retry` reruns the whole application while the `ToolError` is `retryable` (`postgres.rs:150-153` marks 40001 and 40P01), up to `MAX_TX_ATTEMPTS` (`crates/core/src/storage/rel/mod.rs:340`), after `apply_table_recreations` as today; unit tested with a closure.
- DEC-004 (SPEC-0026): tool level tests in a new `#[cfg(test)]` module `crates/core/src/tools/pg_open_e2e.rs`, fixture after `crates/core/src/tools/pg_null_e2e.rs` (own schema per test, cleanup, skip without the DSN or the `postgres` feature).
### Contracts
`impl SchemaSet { pub fn declared_objects(&self) -> DeclaredObjects }` in `crates/core/src/storage/rel/schema.rs` (tables, `indexes` and `typed_indexes` names, `column_migrations` pairs; `virtual_tables` are SQLite only and ignored). Errors unchanged: `ERR_INTERNAL_ERROR`, `postgres: deadlock detected [40P01]`, `retryable: true`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/rel/postgres.rs` | probe, advisory lock, retry, unit and live tests |
| `crates/core/src/storage/rel/schema.rs` | `declared_objects` + test |
| `crates/core/src/tools/pg_open_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod pg_open_e2e;` |
| `.agent_docs/backends.md` | PostgreSQL migrate rule, timestamped 2026-10-14 |

## Acceptance tests
> 100% must pass, through `make check` plus the opt in suites above.

### SPEC-0026/BT-001: Concurrent first requests to new projects all succeed
- **Reproduces:** BUG-001
- **Driver:** `fs.write_bytes` tool calls on a server state with the metadata store on PostgreSQL.
- **Fails today because:** each first request applies the schema with lock taking statements (`schema.rs:279`, `dialect.rs:443-445`), and two of them deadlock.
- Given projects `p1` to `p8` created and never opened, and project `p0` writing `/loop.txt` continuously
- When `fs.write_bytes {mount_id: "pN", path: "/a.txt", base64: "YQ=="}` is called for the 8 projects at once, repeated over 5 fresh rounds (new project ids each round)
- Then all 40 calls return success, and `fs.read_bytes` of each `/a.txt` returns `a`

### SPEC-0026/BT-002: A new project opens while another project's write is uncommitted
- **Reproduces:** BUG-001 (stall)
- **Fails today because:** the open's `ALTER TABLE nodes` waits for the uncommitted write.
- Given a second connection holding an open transaction that inserted a row into `nodes` for project `p0`
- When `fs.write_bytes` is called on never opened project `p9`
- Then it returns success within 2 seconds, before the other transaction ends

### SPEC-0026/BT-010: Retry on a transient failure
- **Validates:** BR-003
- Given a schema application whose first attempt fails with SQLSTATE `40P01` (injected through the test seam the relational tests already use) / When a store opens / Then the open succeeds and the attempt count is 2.

### SPEC-0026/BT-011: Three failures give up
- **Validates:** BR-003
- Given every attempt fails with `40P01` / When a store opens / Then it fails with `ERR_INTERNAL_ERROR` containing `deadlock detected [40P01]`, `retryable: true`, after exactly 3 attempts.

### SPEC-0026/BT-012: Fresh schema and missing column
- **Validates:** BR-004
- Given an empty schema / When the metadata store opens twice / Then every declared table, index and column exists and the second open changes nothing; Given a `nodes` table without `rev` / When it opens / Then `rev` exists.

### SPEC-0026/BT-020: SQLite and SQL Server unchanged
- **Protects:** BR-005
- `make check` green; `sqlserver_passes_the_conformance_suite` green with `MCPFS_TEST_MSSQL_DSN`; the rendered SQLite and SQL Server DDL of every production schema is unchanged.

### SPEC-0026/BT-021: The shared schema suites are stable
- **Protects:** BL-0047 symptom
- With `MCPFS_TEST_PG_DSN` set, `cargo test -p mcp-fs-core --all-features --lib -- storage conformance` passes 5 consecutive runs, with the default test thread count.

BT-021 command: `for i in 1 2 3 4 5; do MCPFS_TEST_PG_DSN=... cargo test -q -p mcp-fs-core --all-features --lib -- storage conformance 2>&1 | rg -a "test result"; done`, every line `ok`.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/sqlite.rs`, `rel/sqlserver.rs`, `rel/dialect.rs`, the stores calling `migrate`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- The rendered DDL of every engine stays byte identical (BT-020).
- Prove BT-001 and BT-002 RED against the compose PostgreSQL before the fix; a test green before the fix is broken.

## Definition of done
- [ ] RED for BT-001, BT-002, BT-010 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: `live_migrate_is_idempotent_and_round_trips_every_value_type`, `serialization_and_deadlock_are_retryable`, the three conformance suites.
- [ ] Docs updated: `.agent_docs/backends.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0026 green; BR-001..BR-005 cited.
- [ ] One commit `US-0029: PostgreSQL store opens without locking a current schema`, body `Spec: SPEC-0026`.
