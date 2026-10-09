> Id: SPEC-0005
> Nature: FEAT
> Status: as-built
> Area: multi-backend-storage
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Multi-Backend Storage and Migration

## 1. Summary

The relational seam lets one codebase run on SQLite, PostgreSQL or SQL Server behind a `RelationalDb`/`RelationalTx` trait pair, with an S3/MinIO blob backend complementing the local one, and an offline `migrate` CLI verb that moves a deployment's relational rows between backends. A store never speaks a driver: it writes SQL once in canonical `?N`-placeholder form and lets `Dialect` render it per engine. `volume_id` scopes every row because one PostgreSQL or SQL Server database holds every volume; `TextKey(n)` exists because SQL Server cannot index `NVARCHAR(MAX)`.

## 2. Current State

### 2.1 Prior State
None — no earlier spec covers this layer; it is specified here for the first time, retroactively, against already-shipped behaviour.

### 2.2 Gaps / Issues Found
None blocking. One functional requirement in the original draft (FR-424, no OAuth token copy) has since been superseded by later code; see §10.

### 2.3 Current Behaviour
`storage/rel/` holds the seam (`mod.rs`), the per-engine renderer (`dialect.rs`), declarative DDL (`schema.rs`), and one file per engine (`sqlite.rs`, `postgres.rs`, `sqlserver.rs`). `migrate.rs` is the offline row-copy. The default build carries neither optional driver: `postgres` and `sqlserver` are opt-in Cargo features (`crates/mcp-fs/Cargo.toml:16-19`).

## 3. Scope

### 3.1 In Scope
- The `RelationalDb`/`RelationalTx` contracts and their ownership model.
- The `SqlValue`/`Query`/`RowValues` value model and its typed accessors.
- `Dialect`: placeholder rendering, column type mapping, upsert forms, identifier quoting, `LIKE` escaping.
- `ColumnType`, including `TextKey(n)` and the SQL Server indexing constraint that forces it.
- The three engines: SQLite (default, serialized, WAL, offloaded), PostgreSQL (sqlx), SQL Server (tiberius-ng + bb8).
- Transient failure classification and `run_retrying`, including its idempotence claim.
- Connection pool settings.
- `volume_id` multi-tenancy as an engine-level concern.
- The S3/MinIO blob backend: bucket naming, key layout, path-style addressing, bucket lifecycle.
- The `migrate` CLI verb: what it copies (now including `oauth_tokens`, see §10), what it deliberately does not, its verification and its idempotence.
- The conformance suite as the mechanism guaranteeing engine equivalence.
- The Cargo feature gating.

### 3.2 Out of Scope
- What the stores store (`nodes`, `blob_refs`, `project`, `project_member` semantics).
- Filesystem and REST behaviour: the guarantee here is that neither changes with the engine.
- The git tables' contents (`migrate` copies them; their meaning is not specified here).
- Search index tables and vector columns: `ColumnType::Tsvector` and `ColumnType::Vector(n)` are specified here only as type mappings.
- The local blob backend's on-disk layout.
- Blob byte migration between blob backends: `migrate` does not do it.

## 4. Actors

| Actor | Description |
|---|---|
| **Operator** | Chooses the engine per store in YAML, supplies DSNs through the environment, and runs `migrate` when changing engines. |
| **Backend author** | A developer adding a fourth engine. Bound by the dialect checklist and the conformance suite. |
| **Store author** | A developer writing a store. Writes canonical SQL once and never names a driver type. |
| **Deployment** | The running server, whose behaviour must not depend on which engine is configured. |

## 5. Usage Scenarios

### SC-401: Operator runs the default SQLite deployment
1. Operator leaves `infra.*.backend` at its default, `sqlite`.
2. Each store opens its own SQLite database: one file per volume under `infra.meta.dir`, one admin database at `infra.admin.path`.
3. The schema is applied on every open, idempotently.
4. Access is serialized behind a mutex and run on the blocking thread pool so request handlers never block, with a WAL journal and a busy timeout; each unit of work runs inside a transaction that commits on success and rolls back on failure.

Exceptions: a DSN configured on a SQLite store fails boot; a non-writable metadata directory fails to open; nothing is retryable on SQLite, so `run_retrying` makes exactly one attempt.

### SC-402: Operator switches the deployment to PostgreSQL
1. Operator sets `infra.meta.backend`/`infra.admin.backend` to `postgres`, with `dsn: ${DATABASE_URL}` and a `schema`.
2. Boot validates each store block.
3. Stores open against the pooled connection and apply their schema.
4. Every volume's rows now live in one database, distinguished by `volume_id`.

Exceptions: the `postgres` feature not compiled in fails boot naming the missing driver; a missing DSN fails boot; a serialization failure or deadlock at commit is retried up to 3 attempts; a permanent SQL error surfaces immediately.

### SC-403: Operator migrates an existing deployment to another engine
1. Operator runs `mcp-fs migrate --from a.yaml --to b.yaml` with the server stopped.
2. The ACL registry is copied first, `project` before `project_member`, because member rows carry a foreign key.
3. The project list drives a per-volume copy of `nodes` and `blob_refs`, then the git tables, then the global `oauth_tokens` table.
4. Each table's row count is compared between source and destination; a mismatch aborts.
5. Every `sha256` a node refers to is checked against the destination blob store, and missing bytes are reported.
6. A report of tables, rows and projects is returned.

Exceptions: a row-count mismatch returns `ERR_INTERNAL_ERROR` naming the table and both counts; missing blob bytes are reported, not silently tolerated; a concurrent write during the copy is missed because the copy takes no locks; running it twice is a no-op rather than a duplication.

### SC-404: Operator stores blobs in S3 or MinIO
1. Operator sets the blob backend to S3 with a `bucket_prefix`.
2. Each project's volume maps to the bucket `{bucket_prefix}{project_id}`.
3. Objects are keyed by their sha256, matching the content addressing used by the local backend.
4. Path-style addressing is forced, which is what MinIO needs.
5. Provisioning a project creates its bucket; tearing it down empties and removes it.

Exceptions: an already-existing bucket, or two racing creators, are both tolerated; deleting a non-empty bucket empties it first, because S3 refuses otherwise; an unreachable endpoint fails with an internal error naming the bucket.

### SC-405: Backend author adds a fourth engine
1. Author adds a `Dialect` variant and fills every match arm: placeholders, column types, upserts, quoting, `LIKE` escaping.
2. Author implements `RelationalDb` and `RelationalTx`, including the failure classification that marks transient errors retryable.
3. Author adds the factory branch and the feature gate.
4. Author runs the conformance suite against the new engine.

Exceptions: a missing dialect arm is a compile error because the match is exhaustive; a transient failure not classified as retryable makes `run_retrying` give up on a condition a retry would resolve; an engine unable to index long text is bounded via `TextKey(n)`, at the cost of a path-length ceiling.

## 6. Functional Requirements

### The seam

#### FR-001 [EARS-U]: Stores speak the seam, never a driver
> Every store SHALL access its data exclusively through `RelationalDb` and `RelationalTx`, and SHALL NOT reference any driver type.

- **Business Rules:** SQL is authored once in canonical form with `?N` placeholders and rendered per engine by `Dialect`.
- **Priority:** Must-have

#### FR-002 [EARS-U]: A transaction is an owned handle
> `RelationalDb::begin` SHALL return an owned transaction handle, `RelationalTx::commit` SHALL consume it, and dropping an uncommitted handle SHALL roll back.

- **Business Rules:** commit consumes the handle so a committed transaction cannot be reused; this is a compile-time guarantee, not a runtime check. The handle is not a closure, so a caller can interleave other work.
- **Priority:** Must-have

#### FR-003 [EARS-U]: Schema application is idempotent
> `RelationalDb::migrate` SHALL apply a `SchemaSet` idempotently, so a store can call it on every open.

- **Business Rules:** calling it on every open lets a store work against a fresh database and an existing one with the same code path, and is why `mcp-fs migrate` can call it on both sides of a copy.
- **Priority:** Must-have

#### FR-004 [EARS-U]: The typed value model
> Rows SHALL be exchanged as `RowValues` with typed accessors, and parameters as `SqlValue` bound positionally.

- **Business Rules:** `RowValues` exposes `i64`, `f64`, `text`, `opt_text` and `blob` accessors, by index or column name. An accessor whose column holds an incompatible type returns an error rather than a silent default.
- **Priority:** Must-have

### Dialect

#### FR-005 [EARS-E]: Placeholder rendering
> WHEN canonical SQL is executed THE dialect SHALL render its `?N` placeholders into the engine's own form.

- **Business Rules:** the canonical form is numbered rather than positional, so a value bound once can be referenced twice without being bound twice.
- **Priority:** Must-have

#### FR-006 [EARS-E]: Column type mapping
> WHEN a schema is applied THE dialect SHALL map each `ColumnType` to the engine's type.

- **Business Rules:** the mapping is exhaustive per engine. `Text` is `TEXT` on SQLite and PostgreSQL and `NVARCHAR(MAX)` on SQL Server; `Blob` is `BLOB`, `BYTEA` and `VARBINARY(MAX)`; `Double` is `REAL`, `DOUBLE PRECISION` and `FLOAT`. `Tsvector` and `Vector(n)` map to real types only on PostgreSQL and degrade to `TEXT`/`NVARCHAR(MAX)` elsewhere.
- **Priority:** Must-have

#### FR-007 [EARS-O]: Bounded keyed text
> IF a text column is used as a key or is indexed THEN it SHALL be declared `TextKey(n)` rather than `Text`.

- **Business Rules:** SQL Server cannot index `NVARCHAR(MAX)`, so keyed text carries a length ceiling there and only there. This is the cause behind the path-length ceiling and of `PROJECT_ID_LEN` being 64. The ceiling is enforced before any database call so the caller gets a deterministic error.
- **Priority:** Must-have

#### FR-008 [EARS-U]: Upsert, quoting and LIKE escaping are dialect concerns
> The dialect SHALL provide the engine's upsert form, identifier quoting and `LIKE` literal escaping, and no store SHALL construct any of them by hand.

- **Business Rules:** upserts are expressed as `ignore`, `replace` or `update` with `Assign::inserted` or `Assign::expr`. An unescaped `LIKE` prefix once made `a_b` match `axb` and a subtree delete removed unrelated rows.
- **Priority:** Must-have

### Engines and failure

#### FR-009 [EARS-U]: Three engines, one behaviour
> The server SHALL support SQLite, PostgreSQL and SQL Server as relational backends, and observable behaviour SHALL NOT differ between them.

- **Business Rules:** SQLite is the default and needs no external service. Each backend is selected per store, so metadata and the ACL registry can sit on different engines.
- **Priority:** Must-have

#### FR-010 [EARS-U]: Optional drivers are opt-in at compile time
> A default build SHALL carry neither the PostgreSQL nor the SQL Server driver.

- **Business Rules:** `default` is empty; `postgres` pulls `sqlx`; `sqlserver` pulls `tiberius-ng`, `bb8` and `tokio-util`; `rag` implies `postgres`; `all-backends` enables all three. Configuring a backend whose driver was not compiled in fails at boot naming it, rather than at first use.
- **Priority:** Must-have

#### FR-011 [EARS-E]: Transient failures are marked retryable
> WHEN a driver reports a transient failure THE engine binding SHALL mark the resulting `ToolError` retryable without introducing a new error code.

- **Business Rules:** on PostgreSQL the retryable SQLSTATEs include `40001` (serialization failure) and `40P01` (deadlock victim), plus connection-loss classes; a permanent error is not retryable. A pool timeout and a dropped connection are retryable. Nothing is retryable on SQLite.
- **Priority:** Must-have

#### FR-012 [EARS-E]: Whole-transaction retry
> WHEN work inside `run_retrying` fails with a retryable error THE helper SHALL open a new transaction and run the work again from the start, up to 3 attempts.

- **Business Rules:** `MAX_TX_ATTEMPTS` is 3, chosen to cover a lost race against one or two competing writers. The commit is part of the attempt, because PostgreSQL reports a serialization failure at `COMMIT` rather than at the conflicting write.
- **Priority:** Must-have

#### FR-013 [EARS-UB]: Retry-unsafe work must not use the retry helper
> Work whose effects reach outside the transaction SHALL NOT be run through `run_retrying`.

- **Business Rules:** calling `run_retrying` is an idempotence claim: on a retry the closure runs again against a new transaction, so incrementing an in-memory counter, appending to a log, charging a quota or sending a message happens twice. Such work uses `begin()` directly and lets the error surface.
- **Priority:** Must-have

#### FR-014 [EARS-UB]: No request thread blocks on the database
> The server SHALL NOT perform blocking database work on a request thread.

- **Business Rules:** the seam is async throughout; the SQLite binding holds one connection per database, serializes access behind a mutex and runs it on the blocking thread pool, so a request handler never blocks.
- **Priority:** Must-have

#### FR-015 [EARS-U]: Connection pooling is configurable
> Each pooled backend SHALL take its pool bounds from `PoolSettings`.

- **Business Rules:** pools are cached per DSN by `RelationalRegistry`, so two stores naming the same database share one pool rather than each opening its own.
- **Priority:** Must-have

### Multi-tenancy

#### FR-016 [EARS-U]: Every volume-scoped row carries volume_id
> `volume_id` SHALL be part of the key of every `nodes`, `blob_refs` and `git_*` row, and SHALL appear in every predicate touching them.

- **Business Rules:** one PostgreSQL or SQL Server database holds every volume, so omitting `volume_id` leaks or corrupts another volume's rows. Under SQLite each volume is its own file and the column is redundant, but it is still written, so the same SQL works on every engine.
- **Priority:** Must-have

### Blob storage

#### FR-017 [EARS-U]: S3 bucket and key layout
> The S3 backend SHALL use one bucket per volume named `{bucket_prefix}{project_id}` and SHALL key each object by its sha256.

- **Business Rules:** the naming is a stable contract: an existing bucket must stay readable after an upgrade. The key equals the content address, so the S3 and local backends agree on identity.
- **Priority:** Must-have

#### FR-018 [EARS-U]: Path-style addressing
> The S3 backend SHALL force path-style addressing.

- **Business Rules:** MinIO requires it; forcing it unconditionally keeps one code path for both MinIO and S3.
- **Priority:** Must-have

#### FR-019 [EARS-E]: Bucket lifecycle
> WHEN a volume is provisioned THE backend SHALL create its bucket if absent, and WHEN a volume is torn down THE backend SHALL empty the bucket before removing it.

- **Business Rules:** an already-existing bucket and two racing creators are both tolerated rather than treated as errors. Emptying before deletion is required because S3 refuses to delete a non-empty bucket.
- **Priority:** Must-have

### Migration

#### FR-020 [EARS-E]: Row-level copy between deployments
> WHEN `mcp-fs migrate --from a.yaml --to b.yaml` runs THE server SHALL copy every relational row it owns from the source backends to the destination backends, preserving every column value exactly.

- **Business Rules:** the copy is a row-for-row transfer at the `RelationalDb` level rather than a replay through the store APIs, because replaying `put_file` stamps fresh timestamps and silently loses every `mtime` and `ctime`. Rows travel in batches of 100, chosen so the widest table's ten bound columns stay clear of SQL Server's 2100-parameter statement ceiling.
- **Priority:** Must-have

#### FR-021 [EARS-U]: Copy order respects foreign keys
> The migration SHALL copy `project` before `project_member`.

- **Business Rules:** member rows carry a foreign key onto the project, so the other order fails on an engine that enforces it. `nodes` and `blob_refs` are independent and their order is presentational only.
- **Priority:** Must-have

#### FR-022 [EARS-E]: Row counts are verified per table
> WHEN a table has been copied THE migration SHALL compare the source and destination row counts and SHALL abort on a mismatch.

- **Business Rules:** the check runs per table rather than once at the end, so the failure names the table that lost rows.
- **Priority:** Must-have

#### FR-023 [EARS-E]: Missing blob bytes are reported
> WHEN the migration finds a node referencing a sha256 absent from the destination blob store THE migration SHALL report it.

- **Business Rules:** this turns a silent dangling reference into a visible one. It is a report, not a failure, because the blob store is configured separately and is legitimately moved by other means.
- **Priority:** Must-have

#### FR-024 [EARS-UB]: The migration does not move blob bytes; OAuth tokens travel as ciphertext
> The migration SHALL NOT copy blob bytes. It SHALL copy the `oauth_tokens` table row-for-row, as ciphertext, using the same copy mechanism as every other table.

- **Business Rules:** blob bytes are configured separately through `infra.blob` and are unaffected by a relational backend change, so copying them is a different operation. OAuth tokens are session state encrypted with `MCPFS_TOKEN_KEY`; the migration does not decrypt or re-encrypt them, so a destination running under a different key never decrypts the copied rows. Opening the source store applies its schema first, which rebuilds a source still on the legacy `(person, provider)` key as empty before anything is read from it. See §10 for why this requirement changed from the original draft.
- **Priority:** Must-have

#### FR-025 [EARS-U]: The migration is offline
> The migration SHALL be run with the server stopped.

- **Business Rules:** it takes no locks against a live writer, so a concurrent write during the copy is missed. This is stated as a requirement on the operator because the tool cannot enforce it.
- **Priority:** Must-have

#### FR-026 [EARS-O]: Re-running the migration is a no-op
> IF the migration is run twice against the same pair of deployments THEN the second run SHALL leave the destination unchanged.

- **Business Rules:** idempotence comes from the copy's upsert form, so an interrupted migration is resumed by running it again rather than by cleaning up first.
- **Priority:** Must-have

### Equivalence

#### FR-027 [EARS-U]: One conformance suite, every engine
> The storage layer SHALL be verified by a single conformance suite executed against every supported engine.

- **Business Rules:** this is the mechanism that makes FR-009 true rather than aspirational. PostgreSQL and SQL Server suites are opt-in and need the containers from `docker-compose.test.yml`.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Pools are cached per DSN, so two stores on one database share connections (FR-015).
- Migration batches 100 rows per statement (FR-020).
- Retries are bounded at 3 attempts, so a genuine conflict fails fast rather than stalling (FR-012).
- SQLite runs in WAL mode with serialized access and offloaded blocking work (SC-401).

### 7.2 Security
- DSNs are secrets: supplied through the environment, expanded into the YAML at load, and redacted in `Debug` and `Display`.
- OAuth ciphertext moves between deployments as opaque bytes; a destination under a different `MCPFS_TOKEN_KEY` holds rows it cannot decrypt rather than rows that leak (FR-024).
- `volume_id` scoping is a tenancy boundary, not an optimization (FR-016).

### 7.3 Usability
- A misconfigured store fails at boot naming the section, not at first use.
- The migration report names tables, row counts and projects, and lists missing content addresses (FR-023).
- A driver that was not compiled in produces a message saying so (FR-010).

### 7.4 Reliability
- Transient failures are retried automatically where it is safe, and the helper's contract makes the unsafe case explicit (FR-012, FR-013).
- Per-table row-count verification catches a partial copy (FR-022).
- Re-running a migration is safe (FR-026).

### 7.5 Observability
Database work is not traced beyond error reporting; there is no query log, no slow-query threshold and no span per statement. A DSN is never logged.

### 7.6 Deployment
- **SQLite**: no external service; state under `state/`.
- **PostgreSQL**: a reachable server, a DSN, and a schema; build with `--features postgres`.
- **SQL Server**: a reachable server and a DSN; build with `--features sqlserver`.
- **S3/MinIO**: an endpoint, credentials and a bucket prefix.
- `docker-compose.test.yml` provides PostgreSQL and SQL Server for the opt-in suites.

### 7.7 Scalability
- On PostgreSQL and SQL Server, one database holds every volume, so project count costs rows rather than databases. On SQLite each volume is a file, which bounds practical project count by filesystem behaviour rather than by the engine.
- The S3 backend puts one bucket per project, so project count is bounded by the provider's bucket limit (not documented in the codebase, open question).

## 8. E2E Tests

| Test ID | Scenario | Requirements | Category |
|---|---|---|---|
| E2E-001 | SC-401 | FR-003, FR-009 | Core journey |
| E2E-002 | SC-401 | FR-014, FR-015 | Feature |
| E2E-003 | SC-401 | FR-003 | Error (apply schema twice, no change) |
| E2E-004 | SC-401 | FR-009, FR-014 | Error (no thread blocks the runtime) |
| E2E-005 | SC-401 | FR-015 | Edge (pool bounds honoured) |
| E2E-006 | SC-401 | FR-003, FR-014 | Edge (registry caches one pool per DSN) |
| E2E-007 | SC-401 | FR-009, FR-015 | Edge (suite passes on every configured engine) |
| E2E-008 | SC-402 | FR-010, FR-012 | Core journey |
| E2E-009 | SC-402 | FR-011, FR-016 | Feature |
| E2E-010 | SC-402 | FR-010 | Error (driver not compiled in) |
| E2E-011 | SC-402 | FR-011, FR-013 | Error |
| E2E-012 | SC-402 | FR-012 | Error (serialization failure retried then succeeds) |
| E2E-013 | SC-402 | FR-013, FR-016 | Data integrity (retried transaction does not double-count) |
| E2E-014 | SC-402 | FR-010, FR-013 | Edge (boot failure, not first use) |
| E2E-015 | SC-402 | FR-011, FR-012 | Edge (retries bounded at 3) |
| E2E-016 | SC-402 | FR-016 | Edge |
| E2E-017 | SC-403 | FR-020, FR-025 | Core journey |
| E2E-018 | SC-403 | FR-021, FR-026 | Feature |
| E2E-019 | SC-403 | FR-020, FR-022 | Error (truncated copy detected by row count) |
| E2E-020 | SC-403 | FR-021, FR-023 | Error |
| E2E-021 | SC-403 | FR-022, FR-024 | Error (oauth_tokens row count verified like any other) |
| E2E-022 | SC-403 | FR-023, FR-024 | Error |
| E2E-023 | SC-403 | FR-020, FR-025 | Edge (timestamps survive the copy exactly) |
| E2E-024 | SC-403 | FR-021, FR-026 | Edge |
| E2E-025 | SC-403 | FR-022, FR-025 | Edge (empty deployment migrates cleanly) |
| E2E-026 | SC-403 | FR-023, FR-024, FR-026 | Edge |
| E2E-027 | SC-404 | FR-017, FR-019 | Feature (provisioning creates the bucket, teardown removes it) |
| E2E-028 | SC-404 | FR-018 | Feature (path-style addressing used) |
| E2E-029 | SC-404 | FR-017, FR-019 | Error (racing bucket creation tolerated) |
| E2E-030 | SC-404 | FR-018 | Error (unreachable endpoint names the bucket) |
| E2E-031 | SC-404 | FR-017, FR-019 | Edge (S3 and local backends agree on identity) |
| E2E-032 | SC-404 | FR-018 | Edge (same content twice stores one object) |
| E2E-033 | SC-405 | FR-001, FR-004, FR-006, FR-027 | Core journey |
| E2E-034 | SC-405 | FR-002, FR-005, FR-008 | Feature |
| E2E-035 | SC-405 | FR-001, FR-007, FR-008 | Error |
| E2E-036 | SC-405 | FR-002, FR-006 | Error (dropped transaction handle rolls back) |
| E2E-037 | SC-405 | FR-004, FR-005 | Error (typed accessor rejects incompatible column) |
| E2E-038 | SC-405 | FR-001, FR-007 | Edge |
| E2E-039 | SC-405 | FR-002, FR-027 | Edge (conformance covers the transaction contract) |
| E2E-040 | SC-405 | FR-004, FR-027 | Edge (conformance covers the value model) |
| E2E-041 | SC-405 | FR-005, FR-006 | Edge |
| E2E-042 | SC-405 | FR-007, FR-008 | Edge (SQL Server key ceiling accepted / refused) |

Fixtures: a source deployment with two projects `proj-a` and `proj-b`, `proj-a` holding `/a.txt` (11 bytes) and `/dir/b.txt`, one shared blob referenced twice, and one ACL member beyond each owner. PostgreSQL, SQL Server and MinIO tests are opt-in, gated on the containers from `docker-compose.test.yml`, and skip with a message when the service is absent.

## 9. Glossary

| Term | Definition |
|---|---|
| **Seam** | The engine-independent surface, `RelationalDb` plus `RelationalTx`, that every store speaks. |
| **Canonical SQL** | SQL authored once with `?N` placeholders, rendered per engine. |
| **Dialect** | The per-engine renderer holding every difference: placeholders, types, upserts, quoting, escaping. |
| **TextKey** | A bounded text column type, needed because SQL Server cannot index unbounded text. |
| **Upsert** | An insert-or-update expressed as `ignore`, `replace` or `update` rather than as engine SQL. |
| **Retryable** | A flag marking a transient driver failure, carried without a distinct error code. |
| **Whole-transaction retry** | Re-running a closure from the start against a new transaction, bounded at three attempts. |
| **Idempotence claim** | What a caller asserts by using the retry helper: running the work twice equals running it once. |
| **Pool** | The cached set of connections per DSN, bounded by `PoolSettings`. |
| **volume_id** | The column scoping every node, blob reference and git row to one volume. |
| **Bucket prefix** | The configured string prepended to a project id to name its S3 bucket. |
| **Path-style addressing** | The S3 URL form MinIO requires, forced unconditionally. |
| **Migration** | The offline, row-level copy of relational state between two deployments. |
| **Conformance suite** | The single test suite run against every engine, which is what makes engine equivalence real. |

## 10. Confidence Notes

- Confidence: **high**. Every FR was spot-checked against current source (`storage/rel/mod.rs`, `dialect.rs`, `migrate.rs`, `storage/blob/s3.rs`, `crates/mcp-fs/Cargo.toml`, `storage/admin.rs`, `storage/rel/postgres.rs`); all citations in the original draft resolved to the described code.
- **One requirement changed since the original draft.** The source draft's FR-424 stated the migration does not copy OAuth tokens. A later change (tracked under FR-NEW-012 in the archived SPEC-0010 GitHub Enterprise/token-store spec) made the migration copy `oauth_tokens` row-for-row as ciphertext instead. This as-built spec states the current behaviour as FR-024; the original non-copy claim is obsolete, not current. See design.md §9 for the mapping.
- All other content (dialect mapping table, retry bound of 3, `TextKey`/`PROJECT_ID_LEN` = 64, SQLSTATE classification, S3 bucket/path-style behaviour) matches code exactly as described in the source draft.
