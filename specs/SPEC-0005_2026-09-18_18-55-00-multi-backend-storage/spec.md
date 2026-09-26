# mcp-fs Multi-Backend Storage and Migration — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** S4 of eight. S4 owns the `4xx` block: `SC-4xx`, `FR-4xx`, `E2E-4xx`, `DEC-4xx`, `EXC-4xx`.

## 1. Executive Summary

This document specifies the **relational seam** that lets one codebase run on SQLite, PostgreSQL or SQL Server, the **S3 blob backend** that complements the local one, and the **`migrate` CLI verb** that moves a deployment between backends without abandoning its data.

The central design rule: a store never speaks a driver. It speaks `RelationalDb` and `RelationalTx`, writes SQL once in a canonical form with `?N` placeholders, and lets `Dialect` render it per engine. Everything that varies between engines, placeholder syntax, column types, upsert form, identifier quoting, `LIKE` escaping, is confined to one file. Adding an engine means implementing two traits and a dialect branch, not editing every store.

Two consequences shape the whole layer. `volume_id` scopes every row, because one PostgreSQL or SQL Server database holds every volume. And `TextKey(n)` exists because SQL Server cannot index `NVARCHAR(MAX)`, which puts a real length ceiling on keyed text and is why S1 FR-033 exists at all.

Retro-specification of shipped behaviour; every claim carries a `file:LINE` citation.

## 2. Current State Analysis

### 2.1 Project Overview

`storage/rel/` is 3553 lines across six files: `mod.rs` (the traits, value model and retry helper), `dialect.rs` (every engine difference), `schema.rs` (declarative DDL), and one file per engine. `migrate.rs` is the offline copy. The default build carries **neither** optional driver: `postgres` and `sqlserver` are opt-in Cargo features (`crates/mcp-fs/Cargo.toml:31-44`).

### 2.2 Existing Specifications

- **S1**: the storage seam as an abstraction boundary (FR-043), `volume_id` scoping (FR-041), `LIKE` escaping (FR-042), content addressing (FR-038..FR-040), the path-length ceiling (FR-033), boot validation of store blocks (FR-003), and the error vocabulary including the `retryable` flag (FR-020). S4 specifies the engines underneath all of these.
- **S2**, **S3**: consumers of the seam; neither is affected by which engine is configured, which is the property S4 must guarantee.

### 2.3 Relevant Architecture

- **Traits**: `RelationalDb` (`storage/rel/mod.rs:253-273`), `RelationalTx` (`:277-287`).
- **Value model**: `SqlValue` (`:70`), `Query` with `bind` (`:136-160`), `RowValues` with typed accessors (`:163-250`).
- **Retry**: `run_retrying` and `MAX_TX_ATTEMPTS` (`:290-313`).
- **Dialect**: `Dialect` enum (`storage/rel/dialect.rs:13`), `ColumnType` including `TextKey(u32)` (`:24-28`), placeholder rendering (`:144-215`), column types per engine (`:216-246`).
- **Engines**: `sqlite.rs`, `postgres.rs`, `sqlserver.rs`.
- **Conformance**: `storage/conformance.rs`, one suite run against every engine.
- **Migration**: `migrate.rs`, with `BATCH_ROWS` 100 (`:44`).
- **S3 blob**: `storage/blob/s3.rs`.

## 3. Scope

### 3.1 In Scope

- The `RelationalDb` and `RelationalTx` contracts and their ownership model.
- The `SqlValue` / `Query` / `RowValues` value model and its typed accessors.
- `Dialect`: placeholder rendering, column type mapping, upsert forms, identifier quoting, `LIKE` escaping.
- `ColumnType`, including `TextKey(n)` and the SQL Server indexing constraint that forces it.
- The three engines: SQLite (default, serialized, WAL, offloaded), PostgreSQL (sqlx, schema-scoped), SQL Server (tiberius-ng + bb8).
- Transient failure classification and `run_retrying`, including the idempotence claim calling it makes.
- Connection pool settings.
- `volume_id` multi-tenancy as an engine-level concern.
- The S3 / MinIO blob backend: bucket naming, key layout, path-style addressing, bucket lifecycle.
- The `migrate` CLI verb: what it copies, what it deliberately does not, its verification and its idempotence.
- The conformance suite as the mechanism guaranteeing engine equivalence.
- The Cargo feature gating.

### 3.2 Out of Scope (Non-Goals)

- What the stores store. `nodes`, `blob_refs`, `project`, `project_member` and their semantics are S1's.
- Filesystem and REST behaviour (S2, S3): the guarantee here is that neither changes with the engine.
- The git tables' contents (S6). `migrate` copies them, and that copy is in scope; their meaning is not.
- Search index tables and vector columns (S7). `ColumnType::Tsvector` and `ColumnType::Vector(n)` appear in the dialect mapping and are specified here **only** as type mappings; their use is S7's.
- The local blob backend, specified in S1 (FR-038, on-disk layout).
- Blob byte migration between blob backends: `migrate` does not do it, and no other component does.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **Operator** | Chooses the engine per store in YAML, supplies DSNs through the environment, and runs `migrate` when changing engines. |
| **Backend author** | A developer adding a fourth engine. Bound by the dialect checklist and the conformance suite. |
| **Store author** | A developer writing a store. Writes canonical SQL once and never names a driver type. |
| **Deployment** | The running server, whose behaviour must not depend on which engine is configured. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Seam** | The engine-independent surface stores speak. | RelationalDb, RelationalTx, Query, RowValues, SqlValue |
| **Dialect** | Everything that differs between engines. | Dialect, ColumnType, placeholder, upsert, quoting |
| **Engine** | One concrete driver binding and its failure taxonomy. | pool, connection, SQLSTATE, retryable |
| **Blob** | Byte storage, local or S3. | bucket, object key, sha256 |
| **Migration** | The offline row copy between two deployments. | TableReport, MigrationReport, batch |

A "migration" in the Migration context is the CLI verb copying rows between deployments; `RelationalDb::migrate` in the Seam context is idempotent DDL application. The two are different operations sharing a word, which is why this table names both.

## 5. Usage Scenarios

### SC-401: Operator runs the default SQLite deployment

**Actor:** Operator
**Preconditions:** a default build, carrying neither optional driver.
**Flow:**
1. Operator leaves `infra.*.backend` at its default, `sqlite` (`config.rs:196-198`).
2. Each store opens its own SQLite database: one file per volume under `infra.meta.dir`, one admin database at `infra.admin.path`.
3. The schema is applied on every open, idempotently (`storage/rel/mod.rs:271`).
4. Access is serialized behind a mutex and run on the blocking thread pool so request handlers never block, with a WAL journal and a busy timeout; each unit of work runs inside a transaction that commits on success and rolls back on failure (`storage/sqlite.rs:1-6`).

**Postconditions:** the server runs with zero external services; every `fs.*` and `admin.*` behaviour is exactly as specified in S1 and S2.
**Exceptions:**
- EXC-401a: a DSN configured on a SQLite store → boot fails, because the DSN would never be read (S1 FR-003, `config.rs:874-915`)
- EXC-401b: the metadata directory not writable → the store fails to open
- EXC-401c: nothing is retryable on SQLite, so `run_retrying` makes exactly one attempt (`storage/rel/mod.rs:311-312`)

### SC-402: Operator switches the deployment to PostgreSQL

**Actor:** Operator
**Preconditions:** a build with the `postgres` feature; a reachable PostgreSQL; the DSN in the environment.
**Flow:**
1. Operator sets `infra.meta.backend` and `infra.admin.backend` to `postgres`, with `dsn: ${DATABASE_URL}` and a `schema`.
2. Boot validates each store block (S1 FR-003).
3. Stores open against the pooled connection and apply their schema.
4. Every volume's rows now live in one database, distinguished by `volume_id`.

**Postconditions:** identical observable behaviour to SC-401, verified by the conformance suite; one database holds every volume.
**Exceptions:**
- EXC-402a: the `postgres` feature not compiled in → boot fails naming the missing driver (S1 FR-003)
- EXC-402b: a missing DSN on a backend that needs one → boot fails
- EXC-402c: a serialization failure or deadlock at commit → retried whole, up to 3 attempts
- EXC-402d: a permanent SQL error → surfaced immediately, not retried

### SC-403: Operator migrates an existing deployment to another engine

**Actor:** Operator
**Preconditions:** **the server is stopped**; two config files naming the source and destination backends.
**Flow:**
1. Operator runs `mcp-fs migrate --from a.yaml --to b.yaml`.
2. The ACL registry is copied first, `project` before `project_member`, because member rows carry a foreign key (`migrate.rs:203-205`).
3. The project list drives a per-volume copy of `nodes` and `blob_refs`, then the git tables.
4. Each table's row count is compared between source and destination, and a mismatch aborts (`migrate.rs:207-213`).
5. Every `sha256` a node refers to is checked against the destination blob store, and missing bytes are reported.
6. A report of tables, rows and projects is returned.

**Postconditions:** the destination holds every row the source held, with `mtime` and `ctime` preserved exactly; the operator can point the server at the new config.
**Exceptions:**
- EXC-403a: a row-count mismatch → `ERR_INTERNAL_ERROR` naming the table and both counts (`migrate.rs:209-212`)
- EXC-403b: blob bytes absent at the destination → reported, not silently tolerated
- EXC-403c: a concurrent write during the copy → missed, because the copy takes no locks
- EXC-403d: running it twice → the second run is a no-op rather than a duplication

### SC-404: Operator stores blobs in S3 or MinIO

**Actor:** Operator
**Preconditions:** `infra.blob` configured for S3 with an endpoint and credentials.
**Flow:**
1. Operator sets the blob backend to S3 with a `bucket_prefix`.
2. Each project's volume maps to the bucket `{bucket_prefix}{project_id}`.
3. Objects are keyed by their sha256, matching the content addressing of S1 FR-038.
4. Path-style addressing is forced, which is what MinIO needs (`storage/blob/s3.rs:34`).
5. Provisioning a project creates its bucket; tearing it down empties and removes it.

**Postconditions:** blob bytes live in object storage; the metadata tree is unaffected and can sit on any relational engine.
**Exceptions:**
- EXC-404a: a bucket that already exists, or two racing creators → both tolerated (`storage/blob/s3.rs:119-123`)
- EXC-404b: deleting a non-empty bucket → the backend empties it first, because S3 refuses otherwise (`storage/blob/s3.rs:130-131`)
- EXC-404c: an unreachable endpoint → the operation fails with an internal error naming the bucket

### SC-405: Backend author adds a fourth engine

**Actor:** Backend author
**Preconditions:** a new driver crate behind a new Cargo feature.
**Flow:**
1. Author adds a `Dialect` variant and fills every match arm: placeholders, column types, upserts, quoting, `LIKE` escaping.
2. Author implements `RelationalDb` and `RelationalTx`, including the failure classification that marks transient errors retryable.
3. Author adds the factory branch and the feature gate.
4. Author runs the conformance suite against the new engine.

**Postconditions:** every store works on the new engine with no store-level change; the conformance suite passes identically.
**Exceptions:**
- EXC-405a: a missing dialect arm → a compile error, because the match is exhaustive
- EXC-405b: a transient failure not classified as retryable → `run_retrying` gives up on a condition that a retry resolves
- EXC-405c: an engine unable to index long text → `TextKey(n)` bounds it, at the cost of a path-length ceiling (S1 FR-033)

## 6. Functional Requirements

### The seam

#### FR-401 [EARS-U]: Stores speak the seam, never a driver
> Every store SHALL access its data exclusively through `RelationalDb` and `RelationalTx`, and SHALL NOT reference any driver type.

- **Inputs:** any store operation.
- **Outputs:** engine-independent SQL and rows (`storage/rel/mod.rs:1-7`).
- **Business Rules:** SQL is authored once in canonical form with `?N` placeholders and rendered per engine by `Dialect`. A store that names a driver type needs editing for every new engine, which is exactly what this seam exists to prevent.
- **Priority:** Must-have

#### FR-402 [EARS-U]: A transaction is an owned handle
> `RelationalDb::begin` SHALL return an owned transaction handle, `RelationalTx::commit` SHALL consume it, and dropping an uncommitted handle SHALL roll back.

- **Inputs:** a `begin` call.
- **Outputs:** `Box<dyn RelationalTx>` (`storage/rel/mod.rs:268,286`).
- **Business Rules:** commit consumes the handle so a committed transaction cannot be reused, which makes the misuse a compile error rather than a runtime surprise. The handle is not a closure, so a caller can interleave other work.
- **Priority:** Must-have

#### FR-403 [EARS-U]: Schema application is idempotent
> `RelationalDb::migrate` SHALL apply a `SchemaSet` idempotently, so a store can call it on every open.

- **Inputs:** the declared `SchemaSet`.
- **Outputs:** the tables and indexes, created if absent (`storage/rel/mod.rs:271`).
- **Business Rules:** calling it on every open is what lets a store work against a fresh database and an existing one with the same code path, and is why `migrate` can call it on both sides of a copy (`migrate.rs:193-194`).
- **Priority:** Must-have

#### FR-404 [EARS-U]: The typed value model
> Rows SHALL be exchanged as `RowValues` with typed accessors, and parameters as `SqlValue` bound positionally.

- **Inputs:** a `Query` with bound values.
- **Outputs:** `RowValues` with `i64`, `f64`, `text`, `opt_text` and `blob` accessors, by index or by column name (`storage/rel/mod.rs:163-250`).
- **Business Rules:** an accessor whose column holds an incompatible type returns an error rather than a silent default. Column names are available, so a store can read by name where positional reading is fragile.
- **Priority:** Must-have

### Dialect

#### FR-405 [EARS-E]: Placeholder rendering
> WHEN canonical SQL is executed THE dialect SHALL render its `?N` placeholders into the engine's own form.

- **Inputs:** canonical SQL with `?1`, `?2`, and so on.
- **Outputs:** engine SQL (`storage/rel/dialect.rs:144-215`).
- **Business Rules:** the canonical form is numbered rather than positional, so a value bound once can be referenced twice without being bound twice.
- **Priority:** Must-have

#### FR-406 [EARS-E]: Column type mapping
> WHEN a schema is applied THE dialect SHALL map each `ColumnType` to the engine's type.

- **Inputs:** a `ColumnType`.
- **Outputs:** the engine type string (`storage/rel/dialect.rs:216-246`).
- **Business Rules:** the mapping is exhaustive per engine. `Text` is `TEXT` on SQLite and PostgreSQL and `NVARCHAR(MAX)` on SQL Server; `Blob` is `BLOB`, `BYTEA` and `VARBINARY(MAX)`; `Double` is `REAL`, `DOUBLE PRECISION` and `FLOAT`. `Tsvector` and `Vector(n)` map to real types only on PostgreSQL and degrade to `TEXT` on SQLite.
- **Priority:** Must-have

#### FR-407 [EARS-O]: Bounded keyed text
> IF a text column is used as a key or is indexed THEN it SHALL be declared `TextKey(n)` rather than `Text`.

- **Inputs:** the column declaration.
- **Outputs:** `NVARCHAR(n)` on SQL Server, `TEXT` elsewhere (`storage/rel/dialect.rs:21-28,218,235`).
- **Business Rules:** SQL Server cannot index `NVARCHAR(MAX)`, so keyed text carries a length ceiling there and only there. This is the root cause of the path-length ceiling in S1 FR-033, and of `PROJECT_ID_LEN` being 64 (`storage/admin.rs:20`). The ceiling is enforced before any database call so the caller gets a deterministic error.
- **Priority:** Must-have

#### FR-408 [EARS-U]: Upsert, quoting and LIKE escaping are dialect concerns
> The dialect SHALL provide the engine's upsert form, identifier quoting and `LIKE` literal escaping, and no store SHALL construct any of them by hand.

- **Inputs:** an upsert specification, an identifier, a `LIKE` literal.
- **Outputs:** engine-correct SQL (`storage/rel/dialect.rs:51-143,247`).
- **Business Rules:** upserts are expressed as `ignore`, `replace` or `update` with `Assign::inserted` or `Assign::expr`. `LIKE` escaping is the mechanism behind S1 FR-042; an unescaped prefix once made `a_b` match `axb` and a subtree delete removed unrelated rows.
- **Priority:** Must-have

### Engines and failure

#### FR-409 [EARS-U]: Three engines, one behaviour
> The server SHALL support SQLite, PostgreSQL and SQL Server as relational backends, and observable behaviour SHALL NOT differ between them.

- **Inputs:** the `infra.*.backend` setting per store.
- **Outputs:** identical results, asserted by the conformance suite (`storage/conformance.rs`).
- **Business Rules:** SQLite is the default and needs no external service. Each backend is selected per store, so metadata and the ACL registry can sit on different engines.
- **Priority:** Must-have

#### FR-410 [EARS-U]: Optional drivers are opt-in at compile time
> A default build SHALL carry neither the PostgreSQL nor the SQL Server driver.

- **Inputs:** the Cargo feature set.
- **Outputs:** the compiled binary (`crates/mcp-fs/Cargo.toml:31-44`).
- **Business Rules:** `default` is empty; `postgres` pulls `sqlx`; `sqlserver` pulls `tiberius-ng`, `bb8` and `tokio-util`; `rag` implies `postgres`; `all-backends` enables all three. Configuring a backend whose driver was not compiled in fails at boot with a message naming it, rather than at first use.
- **Priority:** Must-have

#### FR-411 [EARS-E]: Transient failures are marked retryable
> WHEN a driver reports a transient failure THE engine binding SHALL mark the resulting `ToolError` retryable without introducing a new error code.

- **Inputs:** a driver error.
- **Outputs:** a `ToolError` carrying the `retryable` flag (`storage/rel/postgres.rs:109-149`).
- **Business Rules:** on PostgreSQL the retryable SQLSTATEs include `40001` (serialization failure) and `40P01` (deadlock victim), plus connection-loss classes; a permanent error is not retryable (`storage/rel/postgres.rs:314-331`). A pool timeout and a dropped connection are retryable. Nothing is retryable on SQLite.
- **Priority:** Must-have

#### FR-412 [EARS-E]: Whole-transaction retry
> WHEN work inside `run_retrying` fails with a retryable error THE helper SHALL open a new transaction and run the work again from the start, up to 3 attempts.

- **Inputs:** the work closure.
- **Outputs:** the result, or the last error (`storage/rel/mod.rs:290-330`).
- **Business Rules:** `MAX_TX_ATTEMPTS` is 3, chosen to cover a lost race against one or two competing writers; a higher bound turns a genuine repeatable conflict into a long stall while still failing. The commit is part of the attempt, because PostgreSQL reports a serialization failure at `COMMIT` rather than at the conflicting write.
- **Priority:** Must-have

#### FR-413 [EARS-UB]: Retry-unsafe work must not use the retry helper
> Work whose effects reach outside the transaction SHALL NOT be run through `run_retrying`.

- **Inputs:** any candidate closure.
- **Outputs:** correct accounting.
- **Business Rules:** calling `run_retrying` **is** an idempotence claim: on a retry the closure runs again against a new transaction, so incrementing an in-memory counter, appending to a log, charging a quota or sending a message happens twice (`storage/rel/mod.rs:296-309`). Such work uses `begin()` directly and lets the error surface. `put_file` documents why it qualifies: every value it writes is derived from rows read in the same transaction (`storage/meta.rs:480-484`).
- **Priority:** Must-have

#### FR-414 [EARS-UB]: No request thread blocks on the database
> The server SHALL NOT perform blocking database work on a request thread.

- **Inputs:** any store call.
- **Outputs:** an async result.
- **Business Rules:** the seam is async throughout; the SQLite binding holds one connection per database, serializes access behind a mutex and runs it on the blocking thread pool, so a request handler never blocks (`storage/sqlite.rs:1-6`).
- **Priority:** Must-have

#### FR-415 [EARS-U]: Connection pooling is configurable
> Each pooled backend SHALL take its pool bounds from `PoolSettings`.

- **Inputs:** the `infra.*.pool` config block (`config.rs:116`).
- **Outputs:** the constructed pool (`storage/rel/mod.rs:53`).
- **Business Rules:** pools are cached per DSN by `RelationalRegistry`, so two stores naming the same database share one pool rather than each opening its own.
- **Priority:** Must-have

### Multi-tenancy

#### FR-416 [EARS-U]: Every volume-scoped row carries volume_id
> `volume_id` SHALL be part of the key of every `nodes`, `blob_refs` and `git_*` row, and SHALL appear in every predicate touching them.

- **Inputs:** any volume-scoped query.
- **Outputs:** rows belonging to exactly one volume (`storage/meta.rs:862-863`).
- **Business Rules:** one PostgreSQL or SQL Server database holds every volume, so omitting `volume_id` leaks or corrupts another volume's rows. Under SQLite each volume is its own file and the column is redundant, but it is still written, so the same SQL works on every engine. Refcounts do not leak across volumes (`storage/meta.rs:1194`).
- **Priority:** Must-have

### Blob storage

#### FR-417 [EARS-U]: S3 bucket and key layout
> The S3 backend SHALL use one bucket per volume named `{bucket_prefix}{project_id}` and SHALL key each object by its sha256.

- **Inputs:** the blob config and the project id.
- **Outputs:** the bucket and object names (`storage/blob/s3.rs:3-4,17`).
- **Business Rules:** the naming is a stable contract: an existing bucket must stay readable after an upgrade. The key equals the content address, so the S3 and local backends agree on identity.
- **Priority:** Must-have

#### FR-418 [EARS-U]: Path-style addressing
> The S3 backend SHALL force path-style addressing.

- **Inputs:** the endpoint configuration.
- **Outputs:** the client configuration (`storage/blob/s3.rs:34`).
- **Business Rules:** MinIO requires it; forcing it unconditionally keeps one code path for both MinIO and S3.
- **Priority:** Must-have

#### FR-419 [EARS-E]: Bucket lifecycle
> WHEN a volume is provisioned THE backend SHALL create its bucket if absent, and WHEN a volume is torn down THE backend SHALL empty the bucket before removing it.

- **Inputs:** provisioning and teardown calls.
- **Outputs:** the bucket state (`storage/blob/s3.rs:111-140`).
- **Business Rules:** an already-existing bucket and two racing creators are both tolerated rather than treated as errors (`storage/blob/s3.rs:119-123`). Emptying before deletion is required because S3 refuses to delete a non-empty bucket.
- **Priority:** Must-have

### Migration

#### FR-420 [EARS-E]: Row-level copy between deployments
> WHEN `mcp-fs migrate --from a.yaml --to b.yaml` runs THE server SHALL copy every relational row it owns from the source backends to the destination backends, preserving every column value exactly.

- **Inputs:** two config files.
- **Outputs:** a `MigrationReport` of tables, rows and projects (`migrate.rs:183-283`).
- **Business Rules:** the copy is a row-for-row transfer at the `RelationalDb` level rather than a replay through the store APIs, because replaying `put_file` stamps fresh timestamps and silently loses every `mtime` and `ctime`, and has to reconstruct directory order (`migrate.rs:8-15`). Rows travel in batches of 100, chosen so the widest table's ten bound columns stay clear of SQL Server's 2100-parameter statement ceiling (`migrate.rs:39-44`).
- **Priority:** Must-have

#### FR-421 [EARS-U]: Copy order respects foreign keys
> The migration SHALL copy `project` before `project_member`.

- **Inputs:** the ACL tables.
- **Outputs:** the copied rows (`migrate.rs:203-205`).
- **Business Rules:** member rows carry a foreign key onto the project, so the other order fails on an engine that enforces it. `nodes` and `blob_refs` are independent and their order is presentational only.
- **Priority:** Must-have

#### FR-422 [EARS-E]: Row counts are verified per table
> WHEN a table has been copied THE migration SHALL compare the source and destination row counts and SHALL abort on a mismatch.

- **Inputs:** the copied table.
- **Outputs:** `ERR_INTERNAL_ERROR` naming the table and both counts (`migrate.rs:207-213`).
- **Business Rules:** the check runs per table rather than once at the end, so the failure names the table that lost rows.
- **Priority:** Must-have

#### FR-423 [EARS-E]: Missing blob bytes are reported
> WHEN the migration finds a node referencing a sha256 absent from the destination blob store THE migration SHALL report it.

- **Inputs:** the node rows and the destination blob store.
- **Outputs:** the report's missing-content list (`migrate.rs:19-24,166-181`).
- **Business Rules:** this turns a silent dangling reference into a visible one. It is a report, not a failure, because the blob store is configured separately and is legitimately moved by other means.
- **Priority:** Must-have

#### FR-424 [EARS-UB]: The migration does not move blob bytes or OAuth tokens
> The migration SHALL NOT copy blob bytes, and SHALL NOT copy OAuth tokens.

- **Inputs:** the source deployment.
- **Outputs:** a destination holding relational rows only (`migrate.rs:17-27`).
- **Business Rules:** blob bytes are configured separately through `infra.blob` and are unaffected by a relational backend change, so copying them is a different operation. OAuth tokens are session state encrypted with `MCPFS_TOKEN_KEY`; a device flow re-establishes them, and copying ciphertext to a deployment whose key differs produces rows that never decrypt.
- **Priority:** Must-have

#### FR-425 [EARS-U]: The migration is offline
> The migration SHALL be run with the server stopped.

- **Inputs:** the operator's procedure.
- **Outputs:** a complete copy (`migrate.rs:29-32`).
- **Business Rules:** it takes no locks against a live writer, so a concurrent write during the copy is missed. This is stated as a requirement on the operator because the tool cannot enforce it.
- **Priority:** Must-have

#### FR-426 [EARS-O]: Re-running the migration is a no-op
> IF the migration is run twice against the same pair of deployments THEN the second run SHALL leave the destination unchanged.

- **Inputs:** a second run.
- **Outputs:** the same row counts (`migrate.rs:413`).
- **Business Rules:** idempotence comes from the copy's upsert form, so an interrupted migration is resumed by running it again rather than by cleaning up first.
- **Priority:** Must-have

### Equivalence

#### FR-427 [EARS-U]: One conformance suite, every engine
> The storage layer SHALL be verified by a single conformance suite executed against every supported engine.

- **Inputs:** each configured engine.
- **Outputs:** identical assertions passing (`storage/conformance.rs`).
- **Business Rules:** this is the mechanism that makes FR-409 true rather than aspirational. A behaviour that holds on one engine and not another breaks in production and in no test unless one suite covers both. PostgreSQL and SQL Server suites are opt-in and need the containers from `docker-compose.test.yml`.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Pools are cached per DSN, so two stores on one database share connections (FR-415).
- Migration batches 100 rows per statement (FR-420).
- Retries are bounded at 3 attempts, so a genuine conflict fails fast rather than stalling (FR-412).
- SQLite runs in WAL mode with serialized access and offloaded blocking work (SC-401).

### 7.2 Security
- DSNs are secrets: supplied through the environment, expanded into the YAML at load, and redacted in `Debug` and `Display` (S1 FR-002).
- The migration refuses to move OAuth ciphertext across deployments (FR-424), which prevents a token surviving into an environment whose key differs.
- `volume_id` scoping is a tenancy boundary, not an optimization (FR-416).

### 7.3 Usability
- A misconfigured store fails at boot naming the section, not at first use (S1 FR-003).
- The migration report names tables, row counts and projects, and lists missing content addresses (FR-423).
- A driver that was not compiled in produces a message saying so (FR-410).

### 7.4 Reliability
- Transient failures are retried automatically where it is safe, and the helper's contract makes the unsafe case explicit (FR-412, FR-413).
- Per-table row-count verification catches a partial copy (FR-422).
- Re-running a migration is safe (FR-426).

### 7.5 Observability
Unchanged from S1 §7.5. Database work is not traced beyond error reporting; there is no query log, no slow-query threshold and no span per statement. A DSN is never logged.

### 7.6 Deployment
- **SQLite**: no external service; state under `state/`.
- **PostgreSQL**: a reachable server, a DSN, and a schema; build with `--features postgres`.
- **SQL Server**: a reachable server and a DSN; build with `--features sqlserver`.
- **S3/MinIO**: an endpoint, credentials and a bucket prefix.
- `docker-compose.test.yml` provides PostgreSQL and SQL Server for the opt-in suites.

### 7.7 Scalability
- On PostgreSQL and SQL Server, one database holds every volume, so project count costs rows rather than databases. On SQLite each volume is a file, which bounds practical project count by filesystem behaviour rather than by the engine.
- The S3 backend puts one bucket per project, so project count is bounded by the provider's bucket limit. That limit is real on AWS and is not documented anywhere in the codebase; recorded as TBD-401.

## 8. Data Model

No new entity. S4 specifies how S1's entities are rendered per engine.

| Concept | SQLite | PostgreSQL | SQL Server |
|---|---|---|---|
| `Text` | `TEXT` | `TEXT` | `NVARCHAR(MAX)` |
| `TextKey(n)` | `TEXT` | `TEXT` | `NVARCHAR(n)` |
| `BigInt` | `INTEGER` | `BIGINT` | `BIGINT` |
| `Double` | `REAL` | `DOUBLE PRECISION` | `FLOAT` |
| `Blob` | `BLOB` | `BYTEA` | `VARBINARY(MAX)` |
| `Tsvector` | `TEXT` | `TSVECTOR` | unsupported |
| `Vector(n)` | `TEXT` | `VECTOR(n)` | unsupported |

Per `storage/rel/dialect.rs:216-246`. Storage layout: one database per volume on SQLite; one database holding every volume, distinguished by `volume_id`, on the other two.

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/storage/rel/mod.rs` | Specified, unchanged | FR-401..FR-404, FR-412..FR-415 |
| `crates/mcp-fs/src/storage/rel/dialect.rs` | Specified, unchanged | FR-405..FR-408 |
| `crates/mcp-fs/src/storage/rel/{sqlite,postgres,sqlserver}.rs` | Specified, unchanged | FR-409..FR-411 |
| `crates/mcp-fs/src/storage/sqlite.rs` | Specified, unchanged | FR-414, the serialized SQLite layer |
| `crates/mcp-fs/src/storage/blob/local.rs` | Specified, unchanged | the local counterpart of FR-417, layout `{dir}/{bucket}/{sha[..2]}/{sha}` |
| `crates/mcp-fs/src/storage/blob/s3.rs` | Specified, unchanged | FR-417..FR-419 |
| `crates/mcp-fs/src/migrate.rs` | Specified, unchanged | FR-420..FR-426 |
| `crates/mcp-fs/src/storage/conformance.rs` | Specified, extended | FR-427 |
| `crates/mcp-fs/Cargo.toml` | Referenced | FR-410 |

### 9.2 Affected Requirements

S1 FR-041 (`volume_id` scoping), FR-042 (`LIKE` escaping), FR-043 (the seam) and FR-033 (the path ceiling) are restated here at engine level as FR-416, FR-408, FR-401 and FR-407. No S1 requirement changes; S4 supplies the reason each exists.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/storage/conformance.rs` | one suite per engine | Keep; annotate; extend per §12 |
| `crates/mcp-fs/src/storage/rel/dialect.rs` `#[cfg(test)]` | rendering, types, escaping | Keep; annotate |
| `crates/mcp-fs/src/storage/rel/postgres.rs` `#[cfg(test)]:314-331` | SQLSTATE classification | Keep; annotate |
| `crates/mcp-fs/src/migrate.rs` `#[cfg(test)]:310-470` | tree/ACL copy, idempotence, missing blobs, writability | Keep; annotate |
| `docker-compose.test.yml` | PostgreSQL and SQL Server containers | Required for the opt-in suites |

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | Documentation index | Reference this spec |
| `.agent_docs/backends.md` | dialect checklist, SQL Server decision record | Cross-reference §6; keep the decision record |
| `.agent_docs/config.md` | backends and dsn | Cross-reference FR-410 |

### 9.5 Dependencies & Risks

- **`tiberius-ng` is a young fork.** `.agent_docs/backends.md` holds the decision record; it is the reason SQL Server is off by default. Risk accepted and documented, not resolved here.
- **The opt-in suites need containers.** A contributor without Docker runs the SQLite suite only, so an engine-specific regression can reach `main` if CI does not run the full matrix. Recorded as TBD-402.
- **Bucket-per-project does not scale past the provider's bucket limit** (TBD-401).

## 10. Documentation Requirements

### 10.1 README.md
State that the default build is SQLite-only and that PostgreSQL and SQL Server need a feature flag.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add this spec to the index.
- `.agent_docs/backends.md`: point the dialect checklist at FR-405..FR-408; keep the SQL Server driver decision record, which this spec references rather than duplicates.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-401 | FR-403, FR-409, FR-414, FR-415 | E2E-401, E2E-402 | E2E-403, E2E-404, E2E-405 | E2E-406, E2E-407 |
| SC-402 | FR-410, FR-411, FR-412, FR-413, FR-416 | E2E-408, E2E-409 | E2E-410, E2E-411, E2E-412, E2E-413 | E2E-414, E2E-415, E2E-416 |
| SC-403 | FR-420, FR-421, FR-422, FR-423, FR-424, FR-425, FR-426 | E2E-417, E2E-418 | E2E-419, E2E-420, E2E-421, E2E-422 | E2E-423, E2E-424, E2E-425, E2E-426 |
| SC-404 | FR-417, FR-418, FR-419 | E2E-427, E2E-428 | E2E-429, E2E-430 | E2E-431, E2E-432 |
| SC-405 | FR-401, FR-402, FR-404, FR-405, FR-406, FR-407, FR-408, FR-427 | E2E-433, E2E-434 | E2E-435, E2E-436, E2E-437 | E2E-438, E2E-439, E2E-440, E2E-441, E2E-442 |

Per-FR coverage:

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-401 | E2E-433, E2E-435, E2E-438 | FR-415 | E2E-402, E2E-405, E2E-407 |
| FR-402 | E2E-434, E2E-436, E2E-439 | FR-416 | E2E-409, E2E-413, E2E-416 |
| FR-403 | E2E-401, E2E-403, E2E-406 | FR-417 | E2E-427, E2E-429, E2E-431 |
| FR-404 | E2E-433, E2E-437, E2E-440 | FR-418 | E2E-428, E2E-430, E2E-432 |
| FR-405 | E2E-434, E2E-437, E2E-441 | FR-419 | E2E-427, E2E-429, E2E-431 |
| FR-406 | E2E-433, E2E-436, E2E-441 | FR-420 | E2E-417, E2E-419, E2E-423 |
| FR-407 | E2E-435, E2E-438, E2E-442 | FR-421 | E2E-418, E2E-420, E2E-424 |
| FR-408 | E2E-434, E2E-435, E2E-442 | FR-422 | E2E-419, E2E-421, E2E-425 |
| FR-409 | E2E-401, E2E-404, E2E-407 | FR-423 | E2E-420, E2E-422, E2E-426 |
| FR-410 | E2E-408, E2E-410, E2E-414 | FR-424 | E2E-421, E2E-422, E2E-426 |
| FR-411 | E2E-409, E2E-411, E2E-415 | FR-425 | E2E-423, E2E-425, E2E-417 |
| FR-412 | E2E-408, E2E-412, E2E-415 | FR-426 | E2E-418, E2E-424, E2E-426 |
| FR-413 | E2E-411, E2E-413, E2E-414 | FR-427 | E2E-433, E2E-439, E2E-440 |
| FR-414 | E2E-402, E2E-404, E2E-406 | | |

## 12. End-to-End Test Suite

**Placement.** Engine-level tests are Rust tests in `storage/conformance.rs`, run against every configured engine, plus the per-file `#[cfg(test)]` modules already present. Migration tests live in `migrate.rs`. PostgreSQL, SQL Server and MinIO tests are **opt-in**, gated on the containers from `docker-compose.test.yml`, and skip with a message when the service is absent.

**Fixtures:** a source deployment with two projects `proj-a` and `proj-b`, `proj-a` holding `/a.txt` (11 bytes) and `/dir/b.txt`, one shared blob referenced twice, and one ACL member beyond each owner.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-401 | Existing | Core Journey | SC-401 | FR-403, FR-409 | Critical |
| E2E-402 | Existing | Feature | SC-401 | FR-414, FR-415 | Critical |
| E2E-403 | New | Error | SC-401 | FR-403 | High |
| E2E-404 | New | Error | SC-401 | FR-409, FR-414 | High |
| E2E-405 | New | Error | SC-401 | FR-415 | Medium |
| E2E-406 | New | Edge | SC-401 | FR-403, FR-414 | High |
| E2E-407 | New | Edge | SC-401 | FR-409, FR-415 | Medium |
| E2E-408 | Existing | Core Journey | SC-402 | FR-410, FR-412 | Critical |
| E2E-409 | Existing | Feature | SC-402 | FR-411, FR-416 | Critical |
| E2E-410 | Existing | Error | SC-402 | FR-410 | Critical |
| E2E-411 | Existing | Error | SC-402 | FR-411, FR-413 | Critical |
| E2E-412 | New | Error | SC-402 | FR-412 | High |
| E2E-413 | New | Data Integrity | SC-402 | FR-413, FR-416 | Critical |
| E2E-414 | New | Edge | SC-402 | FR-410, FR-413 | High |
| E2E-415 | New | Edge | SC-402 | FR-411, FR-412 | High |
| E2E-416 | Existing | Edge | SC-402 | FR-416 | Critical |
| E2E-417 | Existing | Core Journey | SC-403 | FR-420, FR-425 | Critical |
| E2E-418 | Existing | Feature | SC-403 | FR-421, FR-426 | Critical |
| E2E-419 | New | Error | SC-403 | FR-420, FR-422 | Critical |
| E2E-420 | Existing | Error | SC-403 | FR-421, FR-423 | High |
| E2E-421 | New | Error | SC-403 | FR-422, FR-424 | High |
| E2E-422 | Existing | Error | SC-403 | FR-423, FR-424 | High |
| E2E-423 | New | Edge | SC-403 | FR-420, FR-425 | High |
| E2E-424 | Existing | Edge | SC-403 | FR-421, FR-426 | High |
| E2E-425 | New | Edge | SC-403 | FR-422, FR-425 | Medium |
| E2E-426 | Existing | Edge | SC-403 | FR-423, FR-424, FR-426 | High |
| E2E-427 | New | Feature | SC-404 | FR-417, FR-419 | Critical |
| E2E-428 | New | Feature | SC-404 | FR-418 | High |
| E2E-429 | New | Error | SC-404 | FR-417, FR-419 | High |
| E2E-430 | New | Error | SC-404 | FR-418 | Medium |
| E2E-431 | New | Edge | SC-404 | FR-417, FR-419 | High |
| E2E-432 | New | Edge | SC-404 | FR-418 | Medium |
| E2E-433 | Existing | Core Journey | SC-405 | FR-401, FR-404, FR-406, FR-427 | Critical |
| E2E-434 | Existing | Feature | SC-405 | FR-402, FR-405, FR-408 | Critical |
| E2E-435 | Existing | Error | SC-405 | FR-401, FR-407, FR-408 | Critical |
| E2E-436 | New | Error | SC-405 | FR-402, FR-406 | High |
| E2E-437 | New | Error | SC-405 | FR-404, FR-405 | High |
| E2E-438 | Existing | Edge | SC-405 | FR-401, FR-407 | Critical |
| E2E-439 | New | Edge | SC-405 | FR-402, FR-427 | High |
| E2E-440 | New | Edge | SC-405 | FR-404, FR-427 | High |
| E2E-441 | Existing | Edge | SC-405 | FR-405, FR-406 | High |
| E2E-442 | New | Edge | SC-405 | FR-407, FR-408 | Critical |

**Coverage Statistics** (42 tests):
- Happy path (Core Journey + Feature): 12
- Failure/error (Error + Security): 17
- Data integrity: 1
- Edge cases: 12
- Happy:Failure ratio: 1:1.42

### 12.2 New Test Specifications

#### E2E-403: Applying a schema twice changes nothing
- **Category:** Error | **Scenario:** SC-401 | **Requirements:** FR-403
- **Steps:**
  - Given a fresh SQLite database with the metadata schema applied and one node row inserted at `/a.txt`
  - When `migrate` is called a second time with the same `SchemaSet`
  - Then the call succeeds
  - And the node row at `/a.txt` is still present and unchanged
  - And no duplicate table or index exists
- **Priority:** High

#### E2E-404: A store call never blocks the async runtime
- **Category:** Error | **Scenario:** SC-401 | **Requirements:** FR-409, FR-414
- **Steps:**
  - Given a current-thread Tokio runtime with one worker
  - When 20 concurrent `put_file` calls are issued against the same SQLite volume
  - Then all 20 complete
  - And an unrelated `tokio::time::sleep` of 10 ms scheduled alongside them completes rather than starving
- **Priority:** High

#### E2E-405: Pool bounds are honoured
- **Category:** Error | **Scenario:** SC-401 | **Requirements:** FR-415
- **Steps:**
  - Given a store configured with a pool maximum of 2
  - When 10 concurrent queries are issued
  - Then all 10 complete successfully
  - And the number of connections opened never exceeds 2
- **Priority:** Medium

#### E2E-406: The registry caches one pool per DSN
- **Category:** Edge | **Scenario:** SC-401 | **Requirements:** FR-403, FR-414
- **Steps:**
  - Given a `RelationalRegistry` and two stores configured against the same DSN
  - When both are opened
  - Then `pool_count` reports exactly 1
  - And opening a third store against a different DSN raises it to 2
- **Priority:** High

#### E2E-407: The same suite passes on every configured engine
- **Category:** Edge | **Scenario:** SC-401 | **Requirements:** FR-409, FR-415
- **Steps:**
  - Given the conformance suite
  - When it is run against SQLite
  - Then every assertion passes
  - And when PostgreSQL is available and the `postgres` feature is compiled in, the identical suite passes against it
  - And when the service is absent the suite skips with a message rather than failing
- **Priority:** Medium

#### E2E-412: A serialization failure is retried and then succeeds
- **Category:** Error | **Scenario:** SC-402 | **Requirements:** FR-412
- **Preconditions:** PostgreSQL available; the `postgres` feature compiled in.
- **Steps:**
  - Given two transactions contending on the same `blob_refs` row
  - When one is aborted by the engine with SQLSTATE `40001` at commit
  - Then `run_retrying` opens a new transaction and runs the work again
  - And the operation ultimately succeeds
  - And the final refcount equals the value expected from both writers, not from one
- **Priority:** High

#### E2E-413: A retried transaction does not double-count external state
- **Category:** Data Integrity | **Scenario:** SC-402 | **Requirements:** FR-413, FR-416
- **Steps:**
  - Given a closure passed to `run_retrying` that writes only values derived from rows it read in the same transaction
  - When the first attempt fails with a retryable error and the second succeeds
  - Then the resulting row values equal those of a single successful attempt
  - And the same closure run against a second volume leaves the first volume's rows untouched
- **Priority:** Critical

#### E2E-414: A backend whose driver is absent fails at boot, not at first use
- **Category:** Edge | **Scenario:** SC-402 | **Requirements:** FR-410, FR-413
- **Steps:**
  - Given a default build with neither optional feature
  - When the server is started with `infra.meta.backend: postgres`
  - Then startup fails
  - And the message names the backend and states the driver was not compiled in
  - And no socket is listening
- **Priority:** High

#### E2E-415: Retries are bounded at three attempts
- **Category:** Edge | **Scenario:** SC-402 | **Requirements:** FR-411, FR-412
- **Steps:**
  - Given a work closure that always fails with a retryable error
  - When it is run through `run_retrying`
  - Then the closure is invoked exactly 3 times
  - And the final error is returned to the caller
  - And a closure failing with a non-retryable error is invoked exactly once
- **Priority:** High

#### E2E-419: A truncated copy is detected by the row count check
- **Category:** Error | **Scenario:** SC-403 | **Requirements:** FR-420, FR-422
- **Steps:**
  - Given a source deployment holding 250 node rows for `proj-a`
  - When the migration is run and the destination is made to lose rows partway
  - Then the migration returns `ERR_INTERNAL_ERROR`
  - And the message names the table `nodes` and both the source and destination counts
- **Priority:** Critical

#### E2E-421: OAuth tokens are absent from the destination
- **Category:** Error | **Scenario:** SC-403 | **Requirements:** FR-422, FR-424
- **Steps:**
  - Given a source deployment holding at least one stored OAuth token
  - When the migration completes successfully
  - Then the destination's OAuth store holds zero rows
  - And the migration report does not list an OAuth table
- **Priority:** High

#### E2E-423: Timestamps survive the copy exactly
- **Category:** Edge | **Scenario:** SC-403 | **Requirements:** FR-420, FR-425
- **Steps:**
  - Given a source node at `/a.txt` whose `mtime` and `ctime` are recorded before the copy
  - When the migration completes
  - Then the destination node's `mtime` and `ctime` equal the source values bit for bit
  - And its `size`, `mode` and `sha256` are likewise identical
- **Priority:** High

#### E2E-425: A migration of an empty deployment succeeds and reports nothing
- **Category:** Edge | **Scenario:** SC-403 | **Requirements:** FR-422, FR-425
- **Steps:**
  - Given a source deployment with no projects
  - When the migration runs
  - Then it succeeds
  - And the report's total row count is 0 and its project list is empty
  - And the destination is writable afterwards: creating a project there succeeds
- **Priority:** Medium

#### E2E-427: Provisioning creates the bucket, teardown removes it
- **Category:** Feature | **Scenario:** SC-404 | **Requirements:** FR-417, FR-419
- **Preconditions:** MinIO available; blob backend set to S3 with `bucket_prefix` `mcpfs-`.
- **Steps:**
  - Given no bucket named `mcpfs-proj-a`
  - When the project `proj-a` is created
  - Then the bucket `mcpfs-proj-a` exists
  - And after writing one file, the bucket holds exactly one object whose key equals the file's sha256
  - And after `admin.delete_project`, the bucket no longer exists even though it was not empty
- **Priority:** Critical

#### E2E-428: Path-style addressing is used
- **Category:** Feature | **Scenario:** SC-404 | **Requirements:** FR-418
- **Preconditions:** MinIO available.
- **Steps:**
  - Given a configured S3 backend pointed at the MinIO endpoint
  - When a blob is written and read back
  - Then the bytes round-trip identically
  - And the operation succeeds against MinIO, which rejects virtual-host-style addressing
- **Priority:** High

#### E2E-429: A racing bucket creation is tolerated
- **Category:** Error | **Scenario:** SC-404 | **Requirements:** FR-417, FR-419
- **Preconditions:** MinIO available.
- **Steps:**
  - Given two concurrent provisioning calls for the same new project
  - When both run
  - Then neither fails
  - And exactly one bucket exists afterwards
  - And provisioning a project whose bucket already exists also succeeds
- **Priority:** High

#### E2E-430: An unreachable endpoint fails with a message naming the bucket
- **Category:** Error | **Scenario:** SC-404 | **Requirements:** FR-418
- **Steps:**
  - Given an S3 backend pointed at a closed port
  - When a blob write is attempted
  - Then the call fails with `ERR_INTERNAL_ERROR`
  - And the message contains the bucket name
- **Priority:** Medium

#### E2E-431: The S3 and local backends agree on identity
- **Category:** Edge | **Scenario:** SC-404 | **Requirements:** FR-417, FR-419
- **Preconditions:** MinIO available.
- **Steps:**
  - Given the same 11-byte content written through the local backend and through the S3 backend
  - When both keys are compared
  - Then the S3 object key equals the sha256 the local backend used as its filename
  - And the local layout places it at `{dir}/{bucket}/{sha[..2]}/{sha}`, the two-character shard directory being part of the on-disk contract
- **Priority:** High

#### E2E-432: Writing the same content twice stores one object
- **Category:** Edge | **Scenario:** SC-404 | **Requirements:** FR-418
- **Preconditions:** MinIO available.
- **Steps:**
  - Given identical content written at two paths in one volume
  - When the bucket is listed
  - Then it holds exactly one object
  - And deleting one of the two paths leaves the object present
  - And deleting the second removes it
- **Priority:** Medium

#### E2E-436: A dropped transaction handle rolls back
- **Category:** Error | **Scenario:** SC-405 | **Requirements:** FR-402, FR-406
- **Steps:**
  - Given a transaction opened with `begin` in which one node row is inserted
  - When the handle is dropped without calling `commit`
  - Then the row is absent from a subsequent query
  - And the same sequence ending in `commit` leaves the row present
- **Priority:** High

#### E2E-437: A typed accessor rejects an incompatible column
- **Category:** Error | **Scenario:** SC-405 | **Requirements:** FR-404, FR-405
- **Steps:**
  - Given a row whose first column holds text
  - When `i64(0)` is called on it
  - Then the call returns an error rather than a default value
  - And `text(0)` on the same column succeeds
  - And `value_named` on an absent column name returns an error
- **Priority:** High

#### E2E-439: Conformance covers the transaction contract on every engine
- **Category:** Edge | **Scenario:** SC-405 | **Requirements:** FR-402, FR-427
- **Steps:**
  - Given the conformance suite
  - When it runs against each configured engine
  - Then it asserts on every engine that an uncommitted transaction rolls back
  - And that a committed one persists
  - And that a second commit on the same handle is impossible, which the type system enforces
- **Priority:** High

#### E2E-440: Conformance covers the value model on every engine
- **Category:** Edge | **Scenario:** SC-405 | **Requirements:** FR-404, FR-427
- **Steps:**
  - Given a row containing a text, a big integer, a double, a nullable text and a blob
  - When it is written and read back on each configured engine
  - Then every value round-trips identically, including the null
  - And the blob's bytes are unchanged, including bytes that are not valid UTF-8
- **Priority:** High

#### E2E-442: A path at the SQL Server key ceiling is accepted and one past it is refused
- **Category:** Edge | **Scenario:** SC-405 | **Requirements:** FR-407, FR-408
- **Steps:**
  - Given a `SafetyManager` constructed with the SQL Server ceiling
  - When a path whose normalized length equals the ceiling is normalized
  - Then it is accepted unchanged
  - And a path one character longer is refused with `ERR_INVALID_ARGUMENT` naming both the length and the limit
  - And on SQLite and PostgreSQL the same long path is accepted, because neither imposes a ceiling
  - And a subtree pattern built for a directory named `a_b` does not match a sibling named `axb`
- **Priority:** Critical

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **Two meanings of "migrate".** `RelationalDb::migrate` applies DDL idempotently; `mcp-fs migrate` copies rows between deployments. Both names are in the code and neither is changed here; §4.5 and §16 name both so the audit does not read a contradiction.
2. **S1 FR-033 and S4 FR-407 describe the same ceiling from two directions.** S1 states the caller-facing rule, S4 states its cause. Neither supersedes the other.
3. **`Tsvector` and `Vector(n)` appear in the dialect mapping** (FR-406) although search is S7's subject. The type mapping is specified here because it lives in `dialect.rs`; their use is not.

## 14. Migration & Implementation Notes

No production code change. Test work, in order:

1. **Annotate existing tests** in `conformance.rs`, `dialect.rs`, `postgres.rs` and `migrate.rs` with their `E2E-4xx` ids.
2. **Add the SQLite-only new tests first** (E2E-403, E2E-406, E2E-415, E2E-436, E2E-437, E2E-442): they need no container and no feature flag.
3. **Add the migration tests** (E2E-419, E2E-421, E2E-423, E2E-425), which run entirely on SQLite-to-SQLite pairs and therefore need no external service either.
4. **Container-gated tests last**: E2E-412, E2E-413 and E2E-414 need PostgreSQL; E2E-427 through E2E-432 need MinIO. Each skips with a message when its service is absent, following the existing opt-in convention.
5. **E2E-419 needs a destination that loses rows partway.** Achieve it by deleting rows at the destination between the copy and the count rather than by adding a failure hook to production code.
6. **Order matters for E2E-414**: it asserts a boot failure, so it starts its own process rather than reusing a fixture server.

## 15. Open Questions & TBDs

- **TBD-401:** One bucket per project (FR-417) is bounded by the provider's bucket limit, which is real on AWS and documented nowhere in the codebase. A deployment with thousands of projects on real S3 would hit it. Whether to move to one bucket with a project prefix is a product decision.
- **TBD-402:** The PostgreSQL, SQL Server and MinIO suites are opt-in. If CI does not run the full matrix, an engine-specific regression can reach `main` while every local run stays green. Whether CI runs the matrix is not verifiable from the repository contents alone and needs confirmation.
- **TBD-403:** Database work is not traced (§7.5). On PostgreSQL under contention, a retry storm would be invisible except through latency. A counter or a span per retry is the obvious fix and is not implemented.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Seam** | The engine-independent surface, `RelationalDb` plus `RelationalTx`, that every store speaks. | Seam |
| **Canonical SQL** | SQL authored once with `?N` placeholders, rendered per engine. | Seam |
| **Dialect** | The per-engine renderer holding every difference: placeholders, types, upserts, quoting, escaping. | Dialect |
| **TextKey** | A bounded text column type, needed because SQL Server cannot index unbounded text. | Dialect |
| **Upsert** | An insert-or-update expressed as `ignore`, `replace` or `update` rather than as engine SQL. | Dialect |
| **Retryable** | A flag marking a transient driver failure, carried without a distinct error code. | Engine |
| **Whole-transaction retry** | Re-running a closure from the start against a new transaction, bounded at three attempts. | Engine |
| **Idempotence claim** | What a caller asserts by using the retry helper: running the work twice equals running it once. | Engine |
| **Pool** | The cached set of connections per DSN, bounded by `PoolSettings`. | Engine |
| **volume_id** | The column scoping every node, blob reference and git row to one volume. | Seam |
| **Bucket prefix** | The configured string prepended to a project id to name its S3 bucket. | Blob |
| **Path-style addressing** | The S3 URL form MinIO requires, forced unconditionally. | Blob |
| **Migration** | The offline, row-level copy of relational state between two deployments. | Migration |
| **Conformance suite** | The single test suite run against every engine, which is what makes engine equivalence real. | Seam |

## 17. Interview Decisions Log

Produced non-interactively from the code.

- **DEC-401:** Vector and tsvector column types are specified here as type mappings only, with their use deferred to S7. **Rationale:** they live in `dialect.rs`, so a spec of the dialect that omitted them would be incomplete, but their semantics belong to search. **Alternatives considered:** deferring the mapping entirely to S7. **Implemented by:** FR-406, §13 item 3. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/storage/rel/dialect.rs:230-242`.
- **DEC-402:** The retry helper's idempotence requirement is specified as an unwanted-behaviour requirement (FR-413) rather than as a note. **Rationale:** misusing it is silent and produces double-charged quotas or duplicated messages, which no test catches unless the rule is stated and checked. **Alternatives considered:** documenting it only in the helper's doc comment, which is where it lives today. **Implemented by:** FR-413, E2E-413. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/storage/rel/mod.rs:296-309`.
- **DEC-403:** What the migration deliberately does not copy is specified as a requirement (FR-424), not as a limitation. **Rationale:** both omissions are reasoned decisions with consequences an operator must plan for, and stating them as prose would let a future change quietly copy OAuth ciphertext across deployments. **Alternatives considered:** listing them under risks. **Implemented by:** FR-424, E2E-421. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/migrate.rs:17-27`.
- **DEC-404:** The offline requirement is stated as a requirement on the operator (FR-425) even though the tool cannot enforce it. **Rationale:** an unenforceable rule that is written is auditable; an unwritten one is folklore. **Alternatives considered:** adding a lock or a liveness probe to the migration, which is new code and outside a retro-spec. **Implemented by:** FR-425. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/migrate.rs:29-32`.
- **DEC-405:** The conformance suite is specified as a requirement (FR-427) rather than treated as test infrastructure. **Rationale:** FR-409's claim that behaviour does not vary by engine is only true because one suite runs everywhere; without that mechanism the claim is untestable. **Alternatives considered:** asserting engine equivalence in prose. **Implemented by:** FR-427, E2E-407, E2E-439, E2E-440. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/storage/conformance.rs`.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 2 (A-2 blob layout, A-4 storage/sqlite.rs citation) | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** layout corrected in E2E-431; citations added to SC-401, FR-414 and section 9.1
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
