> Id: SPEC-0005
> Nature: FEAT
> Status: as-built
> Area: multi-backend-storage
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Multi-Backend Storage and Migration — Design

Source: specs/SPEC-0005_2026-09-18_18-55-00-multi-backend-storage/spec.md (pre-move)

## 1. Components

| Component | File | Role |
|---|---|---|
| Seam traits | `crates/core/src/storage/rel/mod.rs` | `RelationalDb`, `RelationalTx`, `Query`/`SqlValue`/`RowValues`, `run_retrying`, `RelationalRegistry` |
| Dialect | `crates/core/src/storage/rel/dialect.rs` | `Dialect` enum, `ColumnType`, placeholder/type/upsert/quoting/escaping rendering |
| Schema | `crates/core/src/storage/rel/schema.rs` | Declarative DDL, `SchemaSet`, table recreation guards |
| SQLite engine | `crates/core/src/storage/rel/sqlite.rs`, `crates/core/src/storage/sqlite.rs` | Serialized connection, WAL, blocking-pool offload |
| PostgreSQL engine | `crates/core/src/storage/rel/postgres.rs` | sqlx pool, SQLSTATE-based retryable classification |
| SQL Server engine | `crates/core/src/storage/rel/sqlserver.rs` | tiberius-ng + bb8 pool |
| Conformance | `crates/core/src/storage/conformance.rs` | One test suite, run against every configured engine |
| Admin/ACL store | `crates/core/src/storage/admin.rs` | `project`/`project_member` tables, `PROJECT_ID_LEN = 64` |
| Local blob | `crates/core/src/storage/blob/local.rs` | On-disk layout `{dir}/{bucket}/{sha[..2]}/{sha}` |
| S3 blob | `crates/core/src/storage/blob/s3.rs` | Bucket-per-volume, path-style addressing, bucket lifecycle |
| Migration | `crates/core/src/migrate.rs` | Offline row-for-row copy between two deployments |
| Feature gating | `crates/mcp-fs/Cargo.toml` | `postgres`, `sqlserver`, `rag`, `all-backends` Cargo features |

## 2. Flows

**Boot (any engine):** config loads → each `infra.*` block is validated (backend known, driver compiled in, DSN present if required) → store opens its `RelationalDb` → `migrate(&schema)` applies DDL idempotently → ready.

**Request path:** tool/API handler → `core::fs_ops` → store call through `RelationalDb`/`RelationalTx` → canonical SQL + `Dialect` rendering → driver → `RowValues`/`Result`.

**Retry path:** `run_retrying(db, work)` → `begin()` → `work(tx)` → `commit()`; on a retryable error, drop the transaction, `begin()` again, re-run `work` from scratch; up to `MAX_TX_ATTEMPTS = 3` (`storage/rel/mod.rs:340,380`).

**Migration path:** `mcp-fs migrate --from a.yaml --to b.yaml` → open both deployments' stores (schema applied on each open, which also runs the legacy-key rebuild guard before `oauth_tokens` is read) → copy `project` then `project_member` → copy `oauth_tokens` (global, ciphertext, unscoped by project) → per-project copy of `nodes`/`blob_refs`/git tables in batches of 100 rows → per-table row-count verification → scan `nodes.sha256` against the destination blob store, collecting any missing → return `MigrationReport`.

**S3 bucket lifecycle:** project create → `ensure_bucket` (tolerates already-exists / racing create) → writes key by sha256 → project delete → empty bucket → delete bucket.

## 3. Interfaces

- `RelationalDb`: `dialect()`, `execute(&Query) -> u64`, `query(&Query) -> Vec<RowValues>`, `query_opt`, `begin() -> Box<dyn RelationalTx>`, `migrate(&SchemaSet)`.
- `RelationalTx`: `dialect()`, `execute`, `query`, `query_opt`, `commit(self: Box<Self>)`.
- `Dialect`: `quote_ident`, `column_type(ColumnType)`, `table_columns_query()`, placeholder rendering, upsert rendering, `LIKE` escaping.
- `BlobBackend` trait (`storage/traits.rs`): implemented by `local.rs` and `s3.rs`.
- CLI: `mcp-fs migrate --from <yaml> --to <yaml>`.

## 4. Data and State

| Concept | SQLite | PostgreSQL | SQL Server |
|---|---|---|---|
| `Text` | `TEXT` | `TEXT` | `NVARCHAR(MAX)` |
| `TextKey(n)` | `TEXT` | `TEXT` | `NVARCHAR(n)` |
| `BigInt` | `INTEGER` | `BIGINT` | `BIGINT` |
| `Double` | `REAL` | `DOUBLE PRECISION` | `FLOAT` |
| `Blob` | `BLOB` | `BYTEA` | `VARBINARY(MAX)` |
| `Tsvector` | `TEXT` | `TSVECTOR` | `NVARCHAR(MAX)` |
| `Vector(n)` | `TEXT` | `VECTOR(n)` | `NVARCHAR(MAX)` |

(`storage/rel/dialect.rs:218-242`, verified against current code: SQL Server degrades `Tsvector`/`Vector(n)` to `NVARCHAR(MAX)` rather than leaving them unsupported.)

Storage layout: one SQLite database per volume; one PostgreSQL or SQL Server database holding every volume, distinguished by `volume_id`. Local blob layout: `{dir}/{bucket}/{sha[..2]}/{sha}`.

`oauth_tokens` is a global table, not scoped by `volume_id` or project; the migration copies it once, unconditionally (`migrate.rs:218-236`).

## 5. Configuration

- `infra.meta.backend` / `infra.admin.backend`: `sqlite` (default) | `postgres` | `sqlserver`.
- `infra.*.dsn`: `${VAR}`-expanded, redacted in `Debug`/`Display`; required for pooled backends, rejected on SQLite.
- `infra.*.schema`: PostgreSQL schema name.
- `infra.*.pool`: `PoolSettings` (min/max connections, timeouts), consumed by `RelationalRegistry`, cached per DSN.
- `infra.blob`: backend (`local` | `s3`), `bucket_prefix`, `endpoint`, `region`, `access_key`, `secret_key` for S3.
- Cargo features: `default = []`; `postgres = ["mcp-fs-core/postgres"]`; `sqlserver = ["mcp-fs-core/sqlserver"]`; `rag` (implies `postgres`); `all-backends` (`crates/mcp-fs/Cargo.toml:16-19`).
- `MCPFS_TOKEN_KEY`: encrypts/decrypts `oauth_tokens`; not touched by migration.

## 6. Observability

Database work is not traced beyond error reporting: no query log, no slow-query threshold, no span per statement. A DSN is never logged (redacted `Debug`/`Display` on `Dsn`). Migration prints a `MigrationReport` (tables, rows, projects, missing blob content) but is not instrumented with spans.

## 7. Decisions

- **DEC-001:** `Tsvector` and `Vector(n)` column types are specified here as type mappings only; their use is out of scope (owned by the search spec). **Rationale:** they live in `dialect.rs`, so a dialect spec omitting them would be incomplete, but their semantics belong elsewhere. **Code evidence:** `storage/rel/dialect.rs:218-242`.
- **DEC-002:** The retry helper's idempotence requirement is specified as an unwanted-behaviour requirement (FR-013) rather than a comment-only note. **Rationale:** misusing it silently double-charges quotas or duplicates messages, which no test catches unless the rule is stated and checked. **Code evidence:** `storage/rel/mod.rs:296-340`.
- **DEC-003 (supersedes the original DEC-403):** The migration's OAuth-token behaviour changed from "not copied" to "copied as ciphertext." **Rationale:** a later feature (FR-NEW-012, archived SPEC-0010 GitHub Enterprise/token-store spec) required the migration to carry tokens under the then-new per-host key, and the simplest correct mechanism was the same row-for-row copy already used for every other table, deferring decrypt/re-encrypt entirely to runtime. **Code evidence:** `crates/core/src/migrate.rs:17-30,218-236`.
- **DEC-004:** The offline requirement is stated as a requirement on the operator (FR-025) even though the tool cannot enforce it. **Rationale:** an unenforceable rule that is written is auditable; an unwritten one is folklore. **Code evidence:** `crates/core/src/migrate.rs:29-32` (module doc).
- **DEC-005:** The conformance suite is specified as a requirement (FR-027) rather than treated as test infrastructure. **Rationale:** FR-009's claim that behaviour does not vary by engine is only true because one suite runs everywhere. **Code evidence:** `storage/conformance.rs`.

## 8. Requirement to Code Map

| FR | Code |
|---|---|
| FR-001..FR-004 | `storage/rel/mod.rs:253-330` (traits, value model) |
| FR-005..FR-008 | `storage/rel/dialect.rs:1-246` |
| FR-009..FR-011 | `storage/rel/{sqlite,postgres,sqlserver}.rs`; `postgres.rs:109-148` (SQLSTATE classification) |
| FR-010 | `crates/mcp-fs/Cargo.toml:16-19` (feature gating) |
| FR-012, FR-013 | `storage/rel/mod.rs:335-380` (`MAX_TX_ATTEMPTS`, `run_retrying`) |
| FR-014 | `storage/sqlite.rs:1-10` (serialized, blocking-pool offload) |
| FR-015 | `storage/rel/mod.rs:53` and `RelationalRegistry` (pool cache per DSN) |
| FR-016 | `storage/meta.rs` (volume_id on `nodes`/`blob_refs`/`git_*`); `storage/admin.rs:20` (`PROJECT_ID_LEN = 64`, a `TextKey` consequence) |
| FR-017..FR-019 | `storage/blob/s3.rs:3-5,17,34,111-140` |
| FR-020..FR-026 | `migrate.rs:1-50,183-283` (copy order, batching, verification, oauth_tokens, offline note) |
| FR-027 | `storage/conformance.rs` |

## 9. Legacy Mapping

Source: specs/SPEC-0005_2026-09-18_18-55-00-multi-backend-storage/spec.md (pre-move)

| Legacy ID | New ID | Note |
|---|---|---|
| FR-401 | FR-001 | unchanged |
| FR-402 | FR-002 | unchanged |
| FR-403 | FR-003 | unchanged |
| FR-404 | FR-004 | unchanged |
| FR-405 | FR-005 | unchanged |
| FR-406 | FR-006 | unchanged |
| FR-407 | FR-007 | unchanged |
| FR-408 | FR-008 | unchanged |
| FR-409 | FR-009 | unchanged |
| FR-410 | FR-010 | unchanged |
| FR-411 | FR-011 | unchanged |
| FR-412 | FR-012 | unchanged |
| FR-413 | FR-013 | unchanged |
| FR-414 | FR-014 | unchanged |
| FR-415 | FR-015 | unchanged |
| FR-416 | FR-016 | unchanged |
| FR-417 | FR-017 | unchanged |
| FR-418 | FR-018 | unchanged |
| FR-419 | FR-019 | unchanged |
| FR-420 | FR-020 | unchanged |
| FR-421 | FR-021 | unchanged |
| FR-422 | FR-022 | unchanged |
| FR-423 | FR-023 | unchanged |
| **FR-424** | **FR-024** | **Resolved drift.** Original text: "migration SHALL NOT copy blob bytes, and SHALL NOT copy OAuth tokens." Current behaviour: blob bytes are still never copied, but `oauth_tokens` IS copied row-for-row as ciphertext (FR-NEW-012, introduced by the later archived SPEC-0010 token-store spec). The as-built FR-024 states the current behaviour. |
| FR-425 | FR-025 | unchanged |
| FR-426 | FR-026 | unchanged |
| FR-427 | FR-027 | unchanged |
