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
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::types::Oid;
use sqlx::postgres::{
    PgArgumentBuffer, PgArguments, PgConnectOptions, PgPoolOptions, PgRow, PgTypeInfo,
};
use sqlx::{Arguments, AssertSqlSafe, Column, Executor, Postgres, Row, Transaction, ValueRef};
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
                ToolError::invalid_argument("the PostgreSQL dsn is not a valid connection string")
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
        sqlx::Error::PoolTimedOut => {
            ToolError::internal("postgres: timed out waiting for a pooled connection")
                .mark_retryable()
        }
        sqlx::Error::PoolClosed => ToolError::internal("postgres: the connection pool is closed"),
        // The socket died mid statement, so the work never landed.
        sqlx::Error::Io(io) => {
            ToolError::internal(format!("postgres: connection failed: {io}")).mark_retryable()
        }
        sqlx::Error::WorkerCrashed => {
            ToolError::internal("postgres: the connection worker stopped").mark_retryable()
        }
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default().to_string();
            let mapped = ToolError::internal(format!("postgres: {} [{code}]", db.message()));
            if is_retryable_sqlstate(&code) { mapped.mark_retryable() } else { mapped }
        }
        other => ToolError::internal(format!("postgres: {other}")),
    }
}

// ── value mapping ───────────────────────────────────────────────────────────────

/// A null whose type the server infers from where it lands.
///
/// `SqlValue::Null` carries no type, so binding it as any concrete one is a
/// guess: a `TEXT` null was refused by every `BIGINT` column (42804), which broke
/// clearing a quota and every nullable number. OID 0 is the protocol's
/// "unspecified" parameter type, so the server types it from context, exactly
/// as SQLite accepts a null anywhere (SPEC-0022 DEC-001).
struct UntypedNull;

impl sqlx::Type<Postgres> for UntypedNull {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_oid(Oid(0))
    }
}

impl sqlx::Encode<'_, Postgres> for UntypedNull {
    fn encode_by_ref(
        &self,
        _buf: &mut PgArgumentBuffer,
    ) -> std::result::Result<IsNull, BoxDynError> {
        Ok(IsNull::Yes)
    }
}

fn bind_all(query: &Query) -> Result<PgArguments> {
    let mut args = PgArguments::default();
    for value in &query.params {
        let added = match value {
            SqlValue::Null => args.add(UntypedNull),
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
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

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

// ── schema application ──────────────────────────────────────────────────────────

/// True when the connection's schema already holds every table, index and
/// migrated column `schema` declares.
///
/// Reads the catalog only, so it takes no lock a writer could hold up. One query
/// per object kind, compared here, keeps the cost flat in the schema's size.
async fn is_schema_current(conn: &mut sqlx::PgConnection, schema: &SchemaSet) -> Result<bool> {
    let declared = schema.declared_objects();
    let names = |rows: Vec<PgRow>| -> Result<std::collections::HashSet<String>> {
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(map_error)).collect()
    };

    let tables = names(
        sqlx::query(
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema()",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(map_error)?,
    )?;
    if !declared.tables.iter().all(|t| tables.contains(*t)) {
        return Ok(false);
    }

    let indexes = names(
        sqlx::query("SELECT indexname::text FROM pg_indexes WHERE schemaname = current_schema()")
            .fetch_all(&mut *conn)
            .await
            .map_err(map_error)?,
    )?;
    if !declared.indexes.iter().all(|i| indexes.contains(*i)) {
        return Ok(false);
    }

    if declared.columns.is_empty() {
        return Ok(true);
    }
    // A tab cannot appear in one of our identifiers, so it separates the pair.
    let columns = names(
        sqlx::query(
            "SELECT table_name::text || E'\\t' || column_name::text \
             FROM information_schema.columns WHERE table_schema = current_schema()",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(map_error)?,
    )?;
    Ok(declared.columns.iter().all(|(t, c)| columns.contains(&format!("{t}\t{c}"))))
}

/// Run one schema application again while it fails with a transient error, up
/// to [`super::MAX_TX_ATTEMPTS`] attempts in all.
///
/// Two opens racing on an old database can still pick a deadlock victim, and
/// the victim's application was rolled back whole, so running it again is safe.
async fn with_schema_retry<F, Fut>(mut attempt: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut n = 1;
    loop {
        match attempt().await {
            Err(e) if e.retryable && n < super::MAX_TX_ATTEMPTS => {
                // Same linear backoff as `run_retrying`: the winner is about to commit.
                tokio::time::sleep(std::time::Duration::from_millis(10 * u64::from(n))).await;
                n += 1;
            }
            outcome => return outcome,
        }
    }
}

impl PostgresRelationalDb {
    /// One attempt at bringing the schema up to `schema`.
    ///
    /// `CREATE INDEX IF NOT EXISTS` and `ADD COLUMN IF NOT EXISTS` take their
    /// table lock even when the object exists, so a blind replay on every open
    /// queued behind every write in flight and could deadlock against them. The
    /// catalog probe skips all DDL on a current schema; only a schema really
    /// missing something runs it, serialized per schema by an advisory lock.
    async fn apply_schema(&self, schema: &SchemaSet) -> Result<()> {
        {
            let mut conn = self.pool.acquire().await.map_err(map_error)?;
            if is_schema_current(&mut conn, schema).await? {
                return Ok(());
            }
        }

        // One transaction for the whole schema: Postgres has transactional DDL,
        // so a half applied schema cannot be left behind.
        let mut tx = self.pool.begin().await.map_err(map_error)?;
        // Released at commit or rollback. Keyed on the schema, so two deployments
        // in one database never wait on each other.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('mcpfs_migrate:' || current_schema()))")
            .execute(&mut *tx)
            .await
            .map_err(map_error)?;
        // Whoever held the lock before may have added everything already.
        if is_schema_current(&mut tx, schema).await? {
            return tx.commit().await.map_err(map_error);
        }
        for statement in schema.render(Dialect::Postgres) {
            sqlx::query(AssertSqlSafe(statement)).execute(&mut *tx).await.map_err(map_error)?;
        }
        // Rendered with their own `IF NOT EXISTS` guard, so a second apply is a no op.
        for statement in schema.render_column_migrations(Dialect::Postgres) {
            sqlx::query(AssertSqlSafe(statement)).execute(&mut *tx).await.map_err(map_error)?;
        }
        tx.commit().await.map_err(map_error)
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
        // DRIFT-005: drop any table whose live shape predates a key change,
        // before the CREATE TABLE below rebuilds it on the new shape. Runs
        // ahead of the transaction below: it is its own DDL step, exactly like
        // the SQLite pragma probe it mirrors.
        super::apply_table_recreations(self, schema).await?;
        with_schema_retry(|| self.apply_schema(schema)).await
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
        let e =
            PostgresRelationalDb::connect("not a dsn at all", "public", PoolSettings::default())
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
                    Column::required("k", ColumnType::Text),
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

        // SPEC-0022 BT-020: a null bound where the server expects a number. `n` is
        // NOT NULL, so the null reaches it through COALESCE, which needs the
        // parameter untyped; `r` takes a stored null directly.
        db.execute(
            &Query::new("INSERT INTO probe (k, n, r) VALUES (?1, COALESCE(?2, ?3), ?4)")
                .bind("nulls")
                .bind(None::<i64>)
                .bind(7i64)
                .bind(None::<f64>),
        )
        .await
        .unwrap();
        db.execute(
            &Query::new("UPDATE probe SET n = COALESCE(?1, n) WHERE k=?2")
                .bind(None::<i64>)
                .bind("nulls"),
        )
        .await
        .unwrap();
        let rows =
            db.query(&Query::new("SELECT n, r FROM probe WHERE k=?1").bind("nulls")).await.unwrap();
        assert_eq!(rows[0].i64(0).unwrap(), 7);
        assert_eq!(rows[0].value(1).unwrap(), &SqlValue::Null);
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
            tx.execute(
                &Query::new("INSERT INTO probe (k, n) VALUES (?1, ?2)").bind("gone").bind(1i64),
            )
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

        let rows =
            db.query(&Query::new("SELECT n FROM probe WHERE k=?1").bind("same")).await.unwrap();
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

    fn deadlock() -> ToolError {
        ToolError::internal("postgres: deadlock detected [40P01]").mark_retryable()
    }

    /// SPEC-0026/BT-010: a deadlock on the first schema application is retried,
    /// and the second attempt's success is the open's result (BR-003).
    #[tokio::test]
    async fn spec_0026_bt_010_a_transient_failure_is_retried() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let outcome = with_schema_retry(|| async {
            let n = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == 1 { Err(deadlock()) } else { Ok(()) }
        })
        .await;
        assert!(outcome.is_ok(), "the retry must succeed: {outcome:?}");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// SPEC-0026/BT-011: a deadlock on every attempt gives up after exactly
    /// `MAX_TX_ATTEMPTS`, with the driver's error unchanged (BR-003).
    #[tokio::test]
    async fn spec_0026_bt_011_three_failures_give_up() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let e = with_schema_retry(|| async {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(deadlock())
        })
        .await
        .expect_err("every attempt failed");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            super::super::MAX_TX_ATTEMPTS
        );
        assert_eq!(e.code, crate::errors::code::INTERNAL_ERROR);
        assert!(e.message.contains("deadlock detected [40P01]"), "{}", e.message);
        assert!(e.retryable);
    }

    /// A permanent failure is not retried: a wrong statement fails the same way
    /// every time.
    #[tokio::test]
    async fn a_permanent_schema_failure_is_not_retried() {
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let e = with_schema_retry(|| async {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(ToolError::internal("postgres: syntax error [42601]"))
        })
        .await
        .expect_err("a permanent failure surfaces");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!e.retryable);
    }

    /// Tables, indexes and columns of `schema`, read straight from the catalog
    /// independently of the code under test.
    async fn catalog_snapshot(db: &PostgresRelationalDb) -> Vec<String> {
        let q = "SELECT 't:' || table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema() \
                 UNION ALL SELECT 'i:' || indexname || ':' || indexdef FROM pg_indexes \
                 WHERE schemaname = current_schema() \
                 UNION ALL SELECT 'c:' || table_name || '.' || column_name || ':' || data_type \
                 FROM information_schema.columns WHERE table_schema = current_schema() \
                 ORDER BY 1";
        let rows = db.query(&Query::new(q)).await.unwrap();
        rows.iter().map(|r| r.text(0).unwrap()).collect()
    }

    async fn drop_live_schema(db: &PostgresRelationalDb, schema: &str) {
        db.execute(&Query::new(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE")))
            .await
            .unwrap();
    }

    /// SPEC-0026/BT-012: an empty schema gets every declared object, a second
    /// open changes nothing, and a `nodes` lacking `rev` gets it back (BR-004).
    #[tokio::test]
    async fn spec_0026_bt_012_fresh_schema_and_missing_column() {
        let schema_name = format!("mcpfs_pgopen_bt012_{}", std::process::id());
        let Some(db) = live_db(&schema_name).await else {
            eprintln!("skipping spec_0026_bt_012: MCPFS_TEST_PG_DSN is unset");
            return;
        };
        drop_live_schema(&db, &schema_name).await;
        // The pool pinned the schema at connect; recreate it after the drop. A new
        // pooled connection may already have recreated it, hence the guard.
        db.execute(&Query::new(format!("CREATE SCHEMA IF NOT EXISTS \"{schema_name}\"")))
            .await
            .unwrap();
        let set = crate::storage::meta::schema();

        db.migrate(&set).await.unwrap();
        let first = catalog_snapshot(&db).await;
        let declared = set.declared_objects();
        for t in &declared.tables {
            assert!(first.contains(&format!("t:{t}")), "table {t} missing: {first:?}");
        }
        for i in &declared.indexes {
            assert!(
                first.iter().any(|o| o.starts_with(&format!("i:{i}:"))),
                "index {i} missing: {first:?}"
            );
        }
        for (t, c) in &declared.columns {
            assert!(
                first.iter().any(|o| o.starts_with(&format!("c:{t}.{c}:"))),
                "column {t}.{c} missing: {first:?}"
            );
        }

        db.migrate(&set).await.unwrap();
        assert_eq!(catalog_snapshot(&db).await, first, "a second open must change nothing");

        db.execute(&Query::new("ALTER TABLE nodes DROP COLUMN rev")).await.unwrap();
        assert!(!catalog_snapshot(&db).await.iter().any(|o| o.starts_with("c:nodes.rev:")));
        db.migrate(&set).await.unwrap();
        assert_eq!(catalog_snapshot(&db).await, first, "rev must be added back");

        drop_live_schema(&db, &schema_name).await;
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
