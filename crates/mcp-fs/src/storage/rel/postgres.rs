//! PostgreSQL behind [`RelationalDb`], on a real `sqlx` connection pool.
//!
//! Unlike the SQLite adapter, which serializes everything behind one connection,
//! this one hands out a pooled connection per statement and per transaction. That
//! is the throughput reason for supporting Postgres at all, and it is also why
//! transient failures appear here that SQLite could not produce: two concurrent
//! writers can genuinely deadlock or lose a serialization race. Those are mapped
//! to a retryable [`ToolError`] so [`super::run_retrying`] can run the work again.
//!
//! # Why `AssertSqlSafe`
//!
//! sqlx 0.9 requires SQL text to be `'static` or explicitly asserted safe. Our
//! statements are built from static templates plus [`Dialect::render_placeholders`],
//! and every caller supplied value travels as a bound parameter, never as text.
//! So the assertion holds by construction: no user input reaches the SQL string.

use super::dialect::Dialect;
use super::schema::SchemaSet;
use super::{PoolSettings, Query, RelationalDb, RelationalTx, RowValues, SqlValue};
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use sqlx::postgres::{PgArguments, PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::{
    Arguments, AssertSqlSafe, Column, Executor, Postgres, Row, Transaction, ValueRef,
};
use std::sync::Arc;

/// PostgreSQL as a relational backend.
pub struct PostgresRelationalDb {
    pool: sqlx::PgPool,
}

impl PostgresRelationalDb {
    /// Connect and verify the connection, so a wrong DSN fails here instead of
    /// surfacing as a failed request much later.
    ///
    /// `schema` is created when missing and put at the front of `search_path`, so
    /// several deployments can share one database without colliding.
    pub async fn connect(dsn: &str, schema: &str, pool: PoolSettings) -> Result<Self> {
        let options: PgConnectOptions = dsn
            .parse()
            // The DSN carries a password, so only the failure kind is reported.
            .map_err(|_| {
                ToolError::invalid_argument(
                    "the PostgreSQL dsn is not a valid connection string",
                )
            })?;

        let schema = schema.trim();
        if !is_safe_identifier(schema) {
            return Err(ToolError::invalid_argument(format!(
                "'{schema}' is not a usable schema name, expected letters, digits and underscore"
            )));
        }
        let schema = schema.to_string();

        let pool = PgPoolOptions::new()
            .max_connections(pool.max_connections)
            .acquire_timeout(pool.acquire_timeout)
            // Each pooled connection pins the schema, so no statement has to
            // qualify a table name and the SQL stays engine neutral.
            .after_connect(move |conn, _meta| {
                let schema = schema.clone();
                Box::pin(async move {
                    // pgvector must be enabled before the search_path is set;
                    // otherwise CREATE EXTENSION applies to the wrong schema.
                    #[cfg(feature = "rag")]
                    conn.execute(AssertSqlSafe("CREATE EXTENSION IF NOT EXISTS vector")).await?;
                    let ddl = format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"");
                    conn.execute(AssertSqlSafe(ddl)).await?;
                    // Include public so extension types (e.g. VECTOR from pgvector)
                    // installed in public remain resolvable from any app schema.
                    let path = format!("SET search_path TO \"{schema}\", public");
                    conn.execute(AssertSqlSafe(path)).await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .map_err(map_error)?;

        Ok(Self { pool })
    }

    /// Wrap an already built pool, for tests.
    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

/// True for a bare identifier we are willing to interpolate into DDL.
///
/// `search_path` and `CREATE SCHEMA` cannot take a bound parameter, so the schema
/// name is the one value that must be inlined. Restricting the shape is what makes
/// that safe.
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

// ── error mapping ───────────────────────────────────────────────────────────────

/// SQLSTATE classes worth another attempt.
///
/// Everything here means "the database refused this attempt for a reason that may
/// not repeat", never "the statement is wrong".
fn is_retryable_sqlstate(code: &str) -> bool {
    matches!(
        code,
        // 40001 serialization failure, 40P01 deadlock detected. Both are the
        // engine picking a victim between concurrent writers.
        "40001" | "40P01"
            // Class 08: the connection dropped under us.
            | "08000" | "08003" | "08006"
            // The server is shutting down or not yet accepting connections.
            | "57P01" | "57P02" | "57P03"
            // Momentarily out of connection slots.
            | "53300"
    )
}

/// Map a driver error, keeping the engine name honest and flagging transience.
///
/// The message never carries the DSN: sqlx does not put it in an error, and we do
/// not add it, because it holds a password.
fn map_error(e: sqlx::Error) -> ToolError {
    match &e {
        // A pool timeout is the pool's own state, not a database complaint. It
        // must not read as an internal SQLite failure, which is what the previous
        // blanket mapping would have produced.
        sqlx::Error::PoolTimedOut => ToolError::internal(
            "postgres: timed out waiting for a pooled connection",
        )
        .mark_retryable(),
        sqlx::Error::PoolClosed => {
            ToolError::internal("postgres: the connection pool is closed")
        }
        // The socket died mid statement, so the work never landed.
        sqlx::Error::Io(io) => {
            ToolError::internal(format!("postgres: connection failed: {io}")).mark_retryable()
        }
        sqlx::Error::WorkerCrashed => {
            ToolError::internal("postgres: the connection worker stopped").mark_retryable()
        }
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default().to_string();
            let mapped =
                ToolError::internal(format!("postgres: {} [{code}]", db.message()));
            if is_retryable_sqlstate(&code) { mapped.mark_retryable() } else { mapped }
        }
        other => ToolError::internal(format!("postgres: {other}")),
    }
}

// ── value mapping ───────────────────────────────────────────────────────────────

fn bind_all(query: &Query) -> Result<PgArguments> {
    let mut args = PgArguments::default();
    for value in &query.params {
        let added = match value {
            SqlValue::Null => args.add(None::<String>),
            SqlValue::Int(v) => args.add(*v),
            SqlValue::Real(v) => args.add(*v),
            SqlValue::Text(v) => args.add(v.clone()),
            SqlValue::Blob(v) => args.add(v.clone()),
        };
        added.map_err(|e| ToolError::internal(format!("postgres: cannot bind value: {e}")))?;
    }
    Ok(args)
}

/// Decode one column into the portable value model.
///
/// Dispatch is on the column's own type, so a value is never guessed. The integer
/// widths and `NUMERIC` are accepted because a schema created by an earlier
/// version, or by hand, may not use exactly the type we would pick today.
fn decode(row: &PgRow, index: usize) -> Result<SqlValue> {
    let raw = row.try_get_raw(index).map_err(map_error)?;
    if raw.is_null() {
        return Ok(SqlValue::Null);
    }
    let type_name = row.column(index).type_info().to_string();
    // Each integer and float width is a DISTINCT type to the driver, so it has to
    // be decoded at its own width and widened here. Asking for an i64 on an INT4 is
    // a decode error rather than a cast, and INT4 turns up as soon as a statement
    // selects a literal: `SELECT 1` for an existence check is an INT4.
    let value = match type_name.as_str() {
        "INT2" => SqlValue::Int(i64::from(row.try_get::<i16, _>(index).map_err(map_error)?)),
        "INT4" => SqlValue::Int(i64::from(row.try_get::<i32, _>(index).map_err(map_error)?)),
        "INT8" => SqlValue::Int(row.try_get::<i64, _>(index).map_err(map_error)?),
        // `SELECT EXISTS(...)` and any boolean expression land here.
        "BOOL" => SqlValue::Int(i64::from(row.try_get::<bool, _>(index).map_err(map_error)?)),
        "FLOAT4" => SqlValue::Real(f64::from(row.try_get::<f32, _>(index).map_err(map_error)?)),
        "FLOAT8" => SqlValue::Real(row.try_get::<f64, _>(index).map_err(map_error)?),
        "BYTEA" => SqlValue::Blob(row.try_get::<Vec<u8>, _>(index).map_err(map_error)?),
        // TEXT, VARCHAR, CHAR, NAME and anything else textual.
        _ => SqlValue::Text(row.try_get::<String, _>(index).map_err(map_error)?),
    };
    Ok(value)
}

fn to_rows(rows: Vec<PgRow>) -> Result<Vec<RowValues>> {
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let columns: Arc<[String]> =
        first.columns().iter().map(|c| c.name().to_string()).collect::<Vec<_>>().into();
    let width = columns.len();

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut values = Vec::with_capacity(width);
        for i in 0..width {
            values.push(decode(row, i)?);
        }
        out.push(RowValues::new(columns.clone(), values));
    }
    Ok(out)
}

fn rendered(query: &Query) -> Result<String> {
    Dialect::Postgres.render_placeholders(&query.sql)
}

// ── transaction ─────────────────────────────────────────────────────────────────

/// An open `sqlx` transaction. Dropping it without a commit rolls back, which is
/// sqlx's own behaviour and matches the SQLite adapter.
struct PgTx {
    tx: Transaction<'static, Postgres>,
}

#[async_trait]
impl RelationalTx for PgTx {
    async fn execute(&mut self, query: &Query) -> Result<u64> {
        let sql = rendered(query)?;
        let done = sqlx::query_with(AssertSqlSafe(sql), bind_all(query)?)
            .execute(&mut *self.tx)
            .await
            .map_err(map_error)?;
        Ok(done.rows_affected())
    }

    async fn query(&mut self, query: &Query) -> Result<Vec<RowValues>> {
        let sql = rendered(query)?;
        let rows = sqlx::query_with(AssertSqlSafe(sql), bind_all(query)?)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(map_error)?;
        to_rows(rows)
    }

    async fn commit(self: Box<Self>) -> Result<()> {
        self.tx.commit().await.map_err(map_error)
    }
}

#[async_trait]
impl RelationalDb for PostgresRelationalDb {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn execute(&self, query: &Query) -> Result<u64> {
        let sql = rendered(query)?;
        let done = sqlx::query_with(AssertSqlSafe(sql), bind_all(query)?)
            .execute(&self.pool)
            .await
            .map_err(map_error)?;
        Ok(done.rows_affected())
    }

    async fn query(&self, query: &Query) -> Result<Vec<RowValues>> {
        let sql = rendered(query)?;
        let rows = sqlx::query_with(AssertSqlSafe(sql), bind_all(query)?)
            .fetch_all(&self.pool)
            .await
            .map_err(map_error)?;
        to_rows(rows)
    }

    async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
        let tx = self.pool.begin().await.map_err(map_error)?;
        Ok(Box::new(PgTx { tx }))
    }

    async fn migrate(&self, schema: &SchemaSet) -> Result<()> {
        // One transaction for the whole schema: Postgres has transactional DDL, so
        // a half applied schema cannot be left behind.
        let mut tx = self.pool.begin().await.map_err(map_error)?;
        for statement in schema.render(Dialect::Postgres) {
            sqlx::query(AssertSqlSafe(statement))
                .execute(&mut *tx)
                .await
                .map_err(map_error)?;
        }
        // Rendered with their own `IF NOT EXISTS` guard, so a second apply is a no op.
        for statement in schema.render_column_migrations(Dialect::Postgres) {
            sqlx::query(AssertSqlSafe(statement))
                .execute(&mut *tx)
                .await
                .map_err(map_error)?;
        }
        tx.commit().await.map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pool_defaults_match_the_config_defaults() {
        let p = PoolSettings::default();
        assert_eq!(p.max_connections, 10);
        assert_eq!(p.acquire_timeout, Duration::from_secs(30));
    }

    #[test]
    fn serialization_and_deadlock_are_retryable() {
        assert!(is_retryable_sqlstate("40001"), "serialization failure");
        assert!(is_retryable_sqlstate("40P01"), "deadlock victim");
    }

    #[test]
    fn a_dropped_connection_is_retryable() {
        for code in ["08000", "08003", "08006", "57P01", "57P02", "57P03", "53300"] {
            assert!(is_retryable_sqlstate(code), "{code} should be retryable");
        }
    }

    /// A constraint violation or a syntax error repeats forever, so retrying it
    /// would only multiply the work before failing anyway.
    #[test]
    fn a_permanent_failure_is_not_retryable() {
        for code in ["23505", "42601", "42P01", "22001", ""] {
            assert!(!is_retryable_sqlstate(code), "{code} must not be retryable");
        }
    }

    /// The pool's own timeout must not be reported as a SQLite internal error,
    /// which is what a blanket mapping produced before.
    #[test]
    fn a_pool_timeout_is_retryable_and_names_postgres() {
        let e = map_error(sqlx::Error::PoolTimedOut);
        assert!(e.retryable, "a pool timeout can succeed on a second attempt");
        assert!(e.message.contains("postgres"), "{}", e.message);
        assert!(!e.message.contains("sqlite"), "{}", e.message);
    }

    #[test]
    fn a_closed_pool_is_not_retryable() {
        let e = map_error(sqlx::Error::PoolClosed);
        assert!(!e.retryable, "the pool does not reopen on its own");
        assert!(e.message.contains("postgres"), "{}", e.message);
    }

    #[test]
    fn a_dropped_socket_is_retryable() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
        let e = map_error(sqlx::Error::Io(io));
        assert!(e.retryable);
        assert!(e.message.contains("postgres: connection failed"), "{}", e.message);
    }

    #[test]
    fn schema_names_are_restricted_to_bare_identifiers() {
        for good in ["public", "mcpfs", "mcp_fs_2", "A_b9"] {
            assert!(is_safe_identifier(good), "{good} should be accepted");
        }
        // A quote, a semicolon or a dot would let a schema name carry SQL, and it
        // is the one value that cannot travel as a bound parameter.
        for bad in ["", "public; DROP TABLE nodes", "a\"b", "a.b", "9lives", "a-b", "a b"] {
            assert!(!is_safe_identifier(bad), "{bad:?} must be rejected");
        }
    }

    #[tokio::test]
    async fn a_bad_dsn_is_an_invalid_argument_and_never_echoes_the_password() {
        let e = PostgresRelationalDb::connect("not a dsn at all", "public", PoolSettings::default())
            .await
            .err()
            .expect("a malformed dsn cannot connect");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(!e.message.contains("not a dsn"), "the dsn must not be echoed: {}", e.message);
    }

    // ── live server, opt in ────────────────────────────────────────────────
    //
    // Set MCPFS_TEST_PG_DSN to run these against `docker-compose.test.yml`. They
    // skip themselves when it is unset, so the default gate needs no Docker.

    /// A schema per test run, so concurrent tests cannot see each other's tables.
    async fn live_db(schema: &str) -> Option<PostgresRelationalDb> {
        let dsn = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())?;
        let db = PostgresRelationalDb::connect(&dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but unusable");
        Some(db)
    }

    fn probe_schema() -> SchemaSet {
        // Shadows `sqlx::Column`, which this module imports for row decoding.
        use super::super::dialect::ColumnType;
        use super::super::schema::{Column, Index, Table};
        SchemaSet::new(
            vec![Table::new(
                "probe",
                vec![
                    Column::required("k", ColumnType::TextKey(64)),
                    Column::required("n", ColumnType::BigInt),
                    Column::new("r", ColumnType::Double),
                    Column::new("b", ColumnType::Blob),
                    Column::new("t", ColumnType::Text),
                ],
                vec!["k"],
            )],
            vec![Index { name: "idx_probe_n", table: "probe", columns: vec!["n"] }],
        )
    }

    #[tokio::test]
    async fn live_migrate_is_idempotent_and_round_trips_every_value_type() {
        let Some(db) = live_db("mcpfs_probe_roundtrip").await else {
            return;
        };
        let schema = probe_schema();
        db.migrate(&schema).await.unwrap();
        // Applying twice must be a no op, because every store migrates on open.
        db.migrate(&schema).await.unwrap();

        db.execute(&Query::new("DELETE FROM probe")).await.unwrap();
        let affected = db
            .execute(
                &Query::new("INSERT INTO probe (k, n, r, b, t) VALUES (?1, ?2, ?3, ?4, ?5)")
                    .bind("a")
                    .bind(42i64)
                    .bind(1.5f64)
                    .bind(vec![0u8, 1, 255])
                    .bind(None::<String>),
            )
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let rows = db
            .query(&Query::new("SELECT k, n, r, b, t FROM probe WHERE k=?1").bind("a"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.text(0).unwrap(), "a");
        assert_eq!(r.i64(1).unwrap(), 42);
        assert!((r.f64(2).unwrap() - 1.5).abs() < f64::EPSILON);
        assert_eq!(r.blob(3).unwrap(), vec![0u8, 1, 255]);
        assert_eq!(r.opt_text(4).unwrap(), None);
        // Names come back from the server, which is what a store maps on.
        assert_eq!(r.columns(), ["k", "n", "r", "b", "t"]);
    }

    #[tokio::test]
    async fn live_a_dropped_transaction_rolls_back() {
        let Some(db) = live_db("mcpfs_probe_rollback").await else {
            return;
        };
        db.migrate(&probe_schema()).await.unwrap();
        db.execute(&Query::new("DELETE FROM probe")).await.unwrap();

        {
            let mut tx = db.begin().await.unwrap();
            tx.execute(&Query::new("INSERT INTO probe (k, n) VALUES (?1, ?2)").bind("gone").bind(1i64))
                .await
                .unwrap();
            // Dropped without commit: an aborted operation must leave nothing.
        }

        let rows = db.query(&Query::new("SELECT k FROM probe")).await.unwrap();
        assert!(rows.is_empty(), "an uncommitted insert must not persist");
    }

    #[tokio::test]
    async fn live_a_committed_transaction_persists() {
        let Some(db) = live_db("mcpfs_probe_commit").await else {
            return;
        };
        db.migrate(&probe_schema()).await.unwrap();
        db.execute(&Query::new("DELETE FROM probe")).await.unwrap();

        let mut tx = db.begin().await.unwrap();
        tx.execute(&Query::new("INSERT INTO probe (k, n) VALUES (?1, ?2)").bind("kept").bind(2i64))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows = db.query(&Query::new("SELECT k, n FROM probe")).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text(0).unwrap(), "kept");
    }

    /// The upsert the dialect renders has to be accepted by the real server, which
    /// a unit test on the SQL string cannot prove.
    #[tokio::test]
    async fn live_a_rendered_upsert_updates_on_conflict() {
        let Some(db) = live_db("mcpfs_probe_upsert").await else {
            return;
        };
        db.migrate(&probe_schema()).await.unwrap();
        db.execute(&Query::new("DELETE FROM probe")).await.unwrap();

        let upsert = super::super::dialect::Upsert::update(
            "probe",
            vec!["k", "n"],
            vec!["k"],
            vec![super::super::dialect::Assign::expr("n", "{target}.n + 1")],
        );
        let sql = Dialect::Postgres.render_upsert(&upsert);
        for _ in 0..3 {
            db.execute(&Query::new(sql.clone()).bind("same").bind(10i64)).await.unwrap();
        }

        let rows = db.query(&Query::new("SELECT n FROM probe WHERE k=?1").bind("same")).await.unwrap();
        assert_eq!(rows.len(), 1, "the upsert must not insert a duplicate");
        assert_eq!(rows[0].i64(0).unwrap(), 12, "inserted at 10, then incremented twice");
    }

    #[tokio::test]
    async fn live_an_unknown_table_is_a_permanent_failure() {
        let Some(db) = live_db("mcpfs_probe_errors").await else {
            return;
        };
        let e = db
            .query(&Query::new("SELECT * FROM table_that_does_not_exist"))
            .await
            .expect_err("an unknown table cannot be queried");
        assert!(!e.retryable, "a missing table will not appear on a retry");
        assert!(e.message.contains("postgres:"), "{}", e.message);
    }

    #[tokio::test]
    async fn a_rejected_schema_name_fails_before_any_connection() {
        let e = PostgresRelationalDb::connect(
            "postgres://user:hunter2@127.0.0.1:1/db",
            "evil; DROP TABLE nodes",
            PoolSettings::default(),
        )
        .await
        .err()
        .expect("an unusable schema name is refused");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(
            !e.message.contains("hunter2"),
            "a password must never reach an error message: {}",
            e.message
        );
    }
}
