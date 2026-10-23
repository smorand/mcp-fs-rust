# Relational backends

Server state lives in a relational database: the per volume metadata tree, the ACL
registry, the git index and the OAuth token store. Two engines are supported,
selected per store from the YAML config: **SQLite** (default, always compiled) and
**PostgreSQL** (behind the `postgres` cargo feature).

Blob bytes are NOT here. They stay in `infra.blob` (local filesystem or S3) and are
unaffected by a relational backend change.

## The layer

`storage/rel/` is the whole abstraction. Everything above it is engine agnostic.

| File | Holds |
|---|---|
| `mod.rs` | `RelationalDb` and `RelationalTx` traits, `Query`, `SqlValue`, `RowValues`, `run_retrying`, `MAX_TX_ATTEMPTS` |
| `dialect.rs` | `Dialect` enum plus every place the two engines differ |
| `schema.rs` | `SchemaSet`, `Table`, `Column`, `ColumnType`: each schema declared once, rendered per engine |
| `sqlite.rs` | `SqliteRelationalDb`, wrapping the existing serialized `SqliteDb` |
| `postgres.rs` | `PgRelationalDb` on an `sqlx::PgPool`, `#[cfg(feature = "postgres")]` |

### The trait is an owned handle, not a closure

```rust
async fn begin(&self) -> Result<Box<dyn RelationalTx>>;
```

A transaction is an owned object. Statements run on it, then `commit()` consumes it;
dropping without a commit rolls back. This is deliberate and differs from the original
plan sketch, which passed a closure. A closure returning a boxed future over a trait
object is unusable across the roughly 60 call sites here, and it cannot carry a value
out of the transaction, which most of these stores need.

### Retry is opt in, and calling it is a claim

`run_retrying(db, work)` runs `work` in a transaction and retries the whole closure
while the failure is `retryable` (`MAX_TX_ATTEMPTS` is 3). Because a retry re runs
`work` from the start against a NEW transaction, **calling `run_retrying` asserts the
work is idempotent**.

Safe: read a value, compute, write it back. The first attempt rolled back, the second
re reads.

Not safe: anything touching state outside the transaction, such as incrementing an in
memory counter, charging a quota, appending to a log or sending a message. Those would
happen twice. Use `begin()` directly and let the error surface.

SQLite never produces a retryable error (one serialized connection cannot deadlock with
itself), so retry only ever engages on a server engine.

## Adding a backend

1. **Implement `RelationalDb` and `RelationalTx`** in a new `storage/rel/<engine>.rs`,
   gated behind a cargo feature that is NOT in `default`.
2. **Add a `Dialect` variant** and work through the checklist below. The compiler finds
   every site for you: every `match` on `Dialect` is exhaustive on purpose, so a new
   variant does not compile until each difference is answered.
3. **Map errors**, including which ones are transient. Call `mark_retryable()` on a
   serialization conflict, a deadlock victim, a pool timeout and a dropped connection.
   Never let a pool timeout surface as a generic internal error.
4. **Register it** in `config::backend`, in `validate_store` (dsn required, feature
   compiled) and in the `build_*` factories in `storage/mod.rs`.
5. **Run the conformance suite** against it. That is the real acceptance test: the same
   assertions already pass on the other engines.

### Dialect checklist

Every item is a real difference between the two engines, each with tests in
`dialect.rs`.

| Concern | SQLite | PostgreSQL |
|---|---|---|
| Placeholder | `?1` | `$1` |
| Identifier quoting | `"x"` | `"x"` |
| Text | `TEXT` | `TEXT` |
| 64 bit integer | `INTEGER` | `BIGINT` |
| Float | `REAL` | `DOUBLE PRECISION` |
| Bytes | `BLOB` | `BYTEA` |
| Upsert | `ON CONFLICT ... DO UPDATE / DO NOTHING` | same |
| `LIKE` escaping | `\`, `%`, `_` | same |
| Null parameter | untyped | untyped (OID 0, `UntypedNull`) |

Two of those deserve the reason, because getting them wrong is silent:

* **Placeholder rendering skips quoted literals.** A `?` inside a string literal is not a
  parameter. The renderer tracks single quoted literals, doubled quote escapes included.
* **A null parameter must be untyped (2026-10-12, SPEC-0022).** `SqlValue::Null` carries no
  type. PostgreSQL once bound it as a `TEXT` null, which every `BIGINT` column refused with
  42804 (clearing a quota, null retention days, `superseded_at`). `rel/postgres.rs` now binds
  it with OID 0 so the server infers the type from context; never bind a null as a concrete
  type, and never cast per call site. The conformance case `admin_null_values_round_trip`
  pins it on every engine.

## PostgreSQL migrate: probe first, lock only when something is missing (2026-10-14, SPEC-0026)

Every store calls `migrate` on open, and on PostgreSQL `CREATE INDEX IF NOT EXISTS` and
`ADD COLUMN IF NOT EXISTS` take their table lock even when the object exists. A blind
replay made each project's first request queue behind every write in flight on `nodes`
and, with several first requests at once, end in `deadlock detected [40P01]`.
`PostgresRelationalDb::migrate` now:

1. runs `apply_table_recreations` as before;
2. probes the catalog of `current_schema()` (one query each on `information_schema.tables`,
   `pg_indexes`, `information_schema.columns`) against `SchemaSet::declared_objects`, and
   returns with no DDL when everything exists;
3. otherwise takes `pg_advisory_xact_lock(hashtext('mcpfs_migrate:' || current_schema()))`
   in its transaction, probes again, and runs the unchanged rendered DDL;
4. reruns step 2 and 3 through `with_schema_retry` while the error is retryable, up to
   `MAX_TX_ATTEMPTS`.

Rules: a new declared object kind must be added to both `declared_objects` and the probe,
or a missing object is never created. The rendered DDL of every engine is pinned by
`production_ddl_is_unchanged_on_every_engine` in `rel/schema.rs`. Live tests:
`tools/pg_open_e2e.rs` (concurrent first requests, open beside an uncommitted write) and
`spec_0026_bt_012_fresh_schema_and_missing_column` in `rel/postgres.rs`.

## Multi tenancy: `volume_id`

SQLite gets one database file per volume, so a tenant column is unnecessary there. A
PostgreSQL deployment cannot create a database per project on the fly, so
one database holds every volume and the tenant is a column.

`nodes`, `blob_refs` and the three `git_*` tables carry `volume_id` as the first element
of the primary key (`PRIMARY KEY (volume_id, path)`). Under SQLite the column is still
written, constant within each file, so the layout stays one file per volume and behaves
as it always did.

**Consequence to respect:** `volume_id` belongs in the `WHERE` clause of every statement
touching those tables. Omitting it in a read leaks another volume's rows; omitting it in
a delete or a refcount update corrupts another volume. The conformance suite runs the
server engine against one shared instance precisely so a missing predicate shows up.

## Gotchas from the original port, 2026-09-15

Recorded because each one cost real time and none is guessable from the code alone.

* **The unescaped `LIKE` bug was real and pre existing.** Subtree read, subtree delete
  and rename built `path LIKE '{prefix}/%'` with a raw prefix, so a file named `a_b`
  pulled in its sibling `axb`: a subtree delete could remove unrelated rows. All three
  sites now route through one `descendant_pattern` helper that escapes. When you add a
  `LIKE`, use that helper.
* **A `rusqlite` transaction cannot cross an `.await`.** `Transaction` borrows its
  `Connection` and a `MutexGuard` is not `Send`. So `sqlite.rs` drives each transaction
  from a short lived actor: one blocking task takes the lock, opens the transaction and
  serves statements over a channel until commit or drop. Holding the lock for the whole
  transaction is exactly the serialization this store always had, so behaviour is
  unchanged.
* **Retry made non idempotent work newly dangerous.** SQLite's single mutex made a
  deadlock impossible, so no closure ever ran twice. On a server engine it can. Read the
  idempotence contract above before wrapping anything in `run_retrying`.

## See also

* [`config.md`](config.md) for the YAML keys, the DSN, and validation at boot.
* [`architecture.md`](architecture.md) for where the layer sits in a request.
* [`testing.md`](testing.md) for running the suite against PostgreSQL.
