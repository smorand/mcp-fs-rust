# Relational backends

Server state lives in a relational database: the per volume metadata tree, the ACL
registry, the git index and the OAuth token store. Three engines are supported,
selected per store from the YAML config: **SQLite** (default, always compiled),
**PostgreSQL** and **SQL Server** (each behind a cargo feature).

Blob bytes are NOT here. They stay in `infra.blob` (local filesystem or S3) and are
unaffected by a relational backend change.

## The layer

`storage/rel/` is the whole abstraction. Everything above it is engine agnostic.

| File | Holds |
|---|---|
| `mod.rs` | `RelationalDb` and `RelationalTx` traits, `Query`, `SqlValue`, `RowValues`, `run_retrying`, `MAX_TX_ATTEMPTS` |
| `dialect.rs` | `Dialect` enum plus every place the three engines differ |
| `schema.rs` | `SchemaSet`, `Table`, `Column`, `ColumnType`: each schema declared once, rendered per engine |
| `sqlite.rs` | `SqliteRelationalDb`, wrapping the existing serialized `SqliteDb` |
| `postgres.rs` | `PgRelationalDb` on an `sqlx::PgPool`, `#[cfg(feature = "postgres")]` |
| `sqlserver.rs` | `MssqlRelationalDb` on a `bb8` pool over `tiberius-ng`, `#[cfg(feature = "sqlserver")]` |

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

Every item is a real difference between the three engines, each with tests in
`dialect.rs`.

| Concern | SQLite | PostgreSQL | SQL Server |
|---|---|---|---|
| Placeholder | `?1` | `$1` | `@P1` |
| Identifier quoting | `"x"` | `"x"` | `[x]` |
| Text | `TEXT` | `TEXT` | `NVARCHAR(MAX)` |
| Indexed text (`TextKey(n)`) | `TEXT` | `TEXT` | `NVARCHAR(n)` |
| 64 bit integer | `INTEGER` | `BIGINT` | `BIGINT` |
| Float | `REAL` | `DOUBLE PRECISION` | `FLOAT` |
| Bytes | `BLOB` | `BYTEA` | `VARBINARY(MAX)` |
| Upsert | `ON CONFLICT ... DO UPDATE / DO NOTHING` | same | `MERGE ... WITH (HOLDLOCK)` |
| String length | `length()` | `length()` | `LEN()` |
| `LIKE` escaping | `\`, `%`, `_` | same | plus `[` |

Four of those deserve the reason, because getting them wrong is silent:

* **`TextKey` exists because SQL Server cannot index `NVARCHAR(MAX)`.** A column in a
  primary key or an index needs a bounded length there, while the same column is plain
  `TEXT` elsewhere. Use `ColumnType::TextKey(n)` for any keyed text.
* **`MERGE` needs `WITH (HOLDLOCK)`.** Without it, `MERGE` races a concurrent insert and
  raises a duplicate key error instead of updating.
* **`[` matters only on SQL Server**, where it opens a character class in `LIKE`.
  Escaping it on the other engines would be wrong, because there it is a literal.
* **Placeholder rendering skips quoted literals.** A `?` inside a string literal is not a
  parameter. The renderer tracks single quoted literals, doubled quote escapes included.

## Multi tenancy: `volume_id`

SQLite gets one database file per volume, so a tenant column is unnecessary there. A
PostgreSQL or SQL Server deployment cannot create a database per project on the fly, so
one database holds every volume and the tenant is a column.

`nodes`, `blob_refs` and the three `git_*` tables carry `volume_id` as the first element
of the primary key (`PRIMARY KEY (volume_id, path)`). Under SQLite the column is still
written, constant within each file, so the layout stays one file per volume and behaves
as it always did.

**Consequence to respect:** `volume_id` belongs in the `WHERE` clause of every statement
touching those tables. Omitting it in a read leaks another volume's rows; omitting it in
a delete or a refcount update corrupts another volume. The conformance suite runs the
server engines against one shared instance precisely so a missing predicate shows up.

## Gotchas from the original port, 2026-09-15

Recorded because each one cost real time and none is guessable from the code alone.

* **sqlx cannot reach SQL Server.** The MSSQL driver was removed in sqlx 0.7 and the
  rewrite has never shipped, so the appealing "one toolkit, three engines" design is not
  available. SQL Server needs a second driver, which is why `tiberius-ng` plus `bb8` sits
  beside `sqlx`.
* **`tiberius` itself is unmaintained.** No release since 2024 and it ships unpatched
  advisories. `tiberius-ng` is the maintained fork, pinned exactly (`=0.13.1`) because it
  is young and low traffic. If it goes stale, `odbc-api` is the fallback and only
  `sqlserver.rs` changes. This is the weakest dependency in the tree; keep it optional.
* **`tiberius` has no transaction object.** `BEGIN`, `COMMIT` and `ROLLBACK` are ordinary
  statements, so `MssqlTx` owns a pooled connection for the transaction's whole life to
  keep the statements on one session.
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
* [`testing.md`](testing.md) for running the suite against PostgreSQL and SQL Server.
