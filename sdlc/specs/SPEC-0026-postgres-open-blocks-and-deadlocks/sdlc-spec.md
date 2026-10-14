# Opening a store on a shared PostgreSQL schema blocks and deadlocks: Bug Specification

> Generated on: 2026-10-14
> Id: SPEC-0026
> Nature: BUG
> Depth: S
> Depth evidence: 1 module (relational layer, PostgreSQL schema application), 1 defective behavior (store open), 5 requirements, no public contract change, no data model change
> Status: specified
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0049, BL-0047
> Security: n/a
> CVSS: n/a
> Affected: every deployment with a PostgreSQL store
> Fixed in: unreleased

BL-0047 (a flaky shared schema test) has the same root cause and is consumed here.

## 1. Defect

### BUG-001: opening a store re-takes table locks even when the schema is current
- **Observed behavior:**
  - **When migrate runs.** Every store applies its schema when it opens:
    - the metadata store once per volume, on the first access in a process
      (`crates/core/src/storage/meta.rs:474-476`);
    - the admin store (`crates/core/src/storage/admin.rs:211`);
    - the git index (`crates/core/src/git/db.rs:237`);
    - the token store (`crates/core/src/git/oauth/store.rs:784`).
  - **What it runs on PostgreSQL** (`crates/core/src/storage/rel/postgres.rs:319-337`), in one
    transaction:
    - every `CREATE TABLE IF NOT EXISTS`;
    - every `CREATE INDEX IF NOT EXISTS ...` (`crates/core/src/storage/rel/schema.rs:278-280`);
    - every column migration `ALTER TABLE ... ADD COLUMN IF NOT EXISTS ...`
      (`crates/core/src/storage/rel/dialect.rs:443-445`). On the metadata schema that includes
      `nodes.rev` (`storage/meta.rs:340-346`).
  - **What PostgreSQL does with them.** It takes the table lock before it checks that the index or
    column already exists. Verified on the compose PostgreSQL on 2026-10-14: with a transaction
    holding an uncommitted `INSERT` on a table, both
    `ALTER TABLE ... ADD COLUMN IF NOT EXISTS rev TEXT DEFAULT NULL` and
    `CREATE INDEX IF NOT EXISTS ...` on an existing index stopped with
    `canceling statement due to lock timeout` (`lock_timeout` 2s). `CREATE TABLE IF NOT EXISTS`
    returned at once.
  - **The consequence.** On a shared database, every volume's metadata lives in the same `nodes`
    table (`.agent_docs/backends.md`, SPEC-0005 SC-402 step 4).
    - Opening one volume waits for every write in flight on any volume. While it waits and holds its
      lock, it blocks every later read and write on `nodes`.
    - Two opens running at once can deadlock: `postgres: deadlock detected [40P01]`. Observed in
      BL-0049 on `main` at `8fa5a9b`, `one_dsn_yields_one_pool_across_every_consumer`, 2 runs of 4.
- **Expected per authority:**
  - SPEC-0005/FR-003: "`RelationalDb::migrate` SHALL apply a `SchemaSet` idempotently, so a store can
    call it on every open" (`sdlc/specs/SPEC-0005-multi-backend-storage/spec.md:119-120`).
  - SPEC-0005/SC-402, PostgreSQL deployment: "a serialization failure or deadlock at commit is
    retried up to 3 attempts" (`spec.md:68-75`).
  - The same schema application on SQL Server checks the catalog first and takes no lock when the
    object exists (`schema.rs:281-286`, `dialect.rs:446-450`).
- **Reproduction:** shared PostgreSQL metadata store, projects `p1` to `p8` never opened in this process, and project `p0` writing in a loop.
  - When the 8 projects each receive their first `fs.write_bytes` at the same time:
    - Expected: 8 successes.
    - Actual: some calls fail with `ERR_INTERNAL_ERROR`, `postgres: deadlock detected [40P01]`. While
      `p0` holds a write transaction, the first call to a new project stalls until that transaction ends.
- **Root cause:**
  - **The lock:** `crates/core/src/storage/rel/schema.rs:279` and `crates/core/src/storage/rel/dialect.rs:443-445`
    emit statements that take table locks even when the object exists. `crates/core/src/storage/rel/postgres.rs:329-335`
    runs them on every open.
  - **No retry:** `postgres.rs:319-337` runs the schema transaction once. Its errors are marked
    retryable (`postgres.rs:150-153`), but nothing retries them.
- **Contributing factors:** the PostgreSQL suites are opt in and not part of `make check`; most of
  them use the same `public` schema at once (`crates/core/src/storage/mod.rs:604`, `:636`).
- **Impact:**
  - Any PostgreSQL deployment can see random `ERR_INTERNAL_ERROR` on the first request to a project
    after a start or restart.
  - Under write load, opening a project stalls every project.
  - Two server processes starting at once can fail to start.
  - Workaround: retry the call.
- **Rollback posture:** schema contents are unchanged by the fix; reverting restores today's behavior.

## 2. Expected Behavior

All requirements hold WHILE the store is PostgreSQL, unless stated.

#### BR-001 [EARS-E]: Opening against a current schema does not wait on writers
> WHEN a store opens against a schema that already has every table, index and column it declares THE
> Storage context SHALL complete the open without waiting for any transaction of another connection.
- **Source:** restored from SPEC-0005/FR-003 ("so a store can call it on every open"); SQL Server parity (`schema.rs:281-286`).

#### BR-002 [EARS-E]: Concurrent opens succeed
> WHEN several stores open against the same schema at the same time THE Storage context SHALL complete every open without error.
- **Source:** restored from SPEC-0005/FR-003 and SC-402.

#### BR-003 [EARS-O]: A transient failure during schema application is retried
> IF applying the schema fails with a deadlock or serialization failure THEN THE Storage context SHALL
> retry the whole application, up to 3 attempts, before reporting the error.
- **Source:** restored from SPEC-0005/SC-402 ("retried up to 3 attempts"; `MAX_TX_ATTEMPTS`, `crates/core/src/storage/rel/mod.rs:340`).

#### BR-004 [EARS-E]: A missing object is still created
> WHEN a store opens against a schema lacking a declared table, index or column THE Storage context
> SHALL create it, and a second open SHALL change nothing.
- **Source:** restored from SPEC-0005/FR-003.

#### BR-005 [EARS-UB]: Other engines unchanged
> The Storage context SHALL NOT change the schema it produces, or the result of opening a store, on SQLite and SQL Server.
- **Source:** restored from SPEC-0005 ("one codebase run on SQLite, PostgreSQL or SQL Server", `spec.md:12`).

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| fresh, empty schema | every object created on the first open | BR-004 |
| schema created before `nodes.rev` existed | the column is added on the next open | BR-004 |
| a writer holds an uncommitted write on `nodes` | a new volume opens within 2 seconds | BR-001 |
| 3 consecutive deadlocks on one open | the open fails with the 40P01 error, `retryable: true` | BR-003 |

## 4. Tests

PostgreSQL tests run when `MCPFS_TEST_PG_DSN` is set, each in its own schema, and skip otherwise.

### 4.1 End to end reproduction (mandatory)
#### BT-001: Concurrent first requests to new projects all succeed
- **Reproduces:** BUG-001
- **Driver:** `fs.write_bytes` tool calls on a server state with the metadata store on PostgreSQL.
- **Fails today because:** each first request applies the schema with lock taking statements (`schema.rs:279`, `dialect.rs:443-445`), and two of them deadlock.
- Given projects `p1` to `p8` created and never opened, and project `p0` writing `/loop.txt` continuously
- When `fs.write_bytes {mount_id: "pN", path: "/a.txt", base64: "YQ=="}` is called for the 8 projects at once, repeated over 5 fresh rounds (new project ids each round)
- Then all 40 calls return success, and `fs.read_bytes` of each `/a.txt` returns `a`

#### BT-002: A new project opens while another project's write is uncommitted
- **Reproduces:** BUG-001 (stall)
- **Fails today because:** the open's `ALTER TABLE nodes` waits for the uncommitted write.
- Given a second connection holding an open transaction that inserted a row into `nodes` for project `p0`
- When `fs.write_bytes` is called on never opened project `p9`
- Then it returns success within 2 seconds, before the other transaction ends

### 4.2 New behavior
#### BT-010: Retry on a transient failure
- **Validates:** BR-003
- Given a schema application whose first attempt fails with SQLSTATE `40P01` (injected through the test seam the relational tests already use) / When a store opens / Then the open succeeds and the attempt count is 2.

#### BT-011: Three failures give up
- **Validates:** BR-003
- Given every attempt fails with `40P01` / When a store opens / Then it fails with `ERR_INTERNAL_ERROR` containing `deadlock detected [40P01]`, `retryable: true`, after exactly 3 attempts.

#### BT-012: Fresh schema and missing column
- **Validates:** BR-004
- Given an empty schema / When the metadata store opens twice / Then every declared table, index and column exists and the second open changes nothing; Given a `nodes` table without `rev` / When it opens / Then `rev` exists.

### 4.3 Non regression
#### BT-020: SQLite and SQL Server unchanged
- **Protects:** BR-005
- `make check` green; `sqlserver_passes_the_conformance_suite` green with `MCPFS_TEST_MSSQL_DSN`; the rendered SQLite and SQL Server DDL of every production schema is unchanged.

#### BT-021: The shared schema suites are stable
- **Protects:** BL-0047 symptom
- With `MCPFS_TEST_PG_DSN` set, `cargo test -p mcp-fs-core --all-features --lib -- storage conformance` passes 5 consecutive runs, with the default test thread count.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/rel/postgres.rs:428` | `live_migrate_is_idempotent_and_round_trips_every_value_type` | Kept | idempotence unchanged |
| `crates/core/src/storage/rel/postgres.rs:353` | `serialization_and_deadlock_are_retryable` | Kept | classification unchanged |
| `crates/core/src/storage/mod.rs:604` | `teardown_on_a_shared_database_drops_the_usage_row` | Kept | flaky today, covered by BT-021 |
| `crates/core/src/storage/mod.rs:636` | `one_dsn_yields_one_pool_across_every_consumer` | Kept | flaky today, covered by BT-021 |

## 5. Existing Specs Affected
None deviated. Restores SPEC-0005/FR-003 and SC-402 on PostgreSQL.

## 6. Assumptions
- **Assumption:** 2 seconds (BT-002) is the bound for "does not wait"; today the open waits for the whole write.
- **Assumption:** BT-001's 8 by 5 rounds reproduces reliably on the compose PostgreSQL; BL-0049 saw 2 of 4 runs fail with fewer concurrent opens.
- **Backlog entries created by this run:** none. Consumed: BL-0049, BL-0047.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS (root cause only; the fix is for design) |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS (tool names, error code and message, SQLSTATE) |
| 4 | No forced choice | PASS (bounds stated) |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001, BT-002 tool calls) |
| 7ante | Security misclassification | PASS (availability, no exposure) |
| 8 | No hidden prerequisite | PASS |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (5 requirements at S) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS (lock behavior verified by a live probe) |
| 13 | Writer and reader read | PASS (both engines' renderers read, `schema.rs:278-286`) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
