//! SQL Server behind [`RelationalDb`], on a `bb8` pool over `tiberius-ng`.
//!
//! # Why a second driver instead of sqlx
//!
//! sqlx removed its MSSQL driver in 0.7 and the rewrite has never shipped, so the
//! single toolkit approach that covers SQLite and PostgreSQL cannot reach SQL
//! Server at all. `tiberius-ng` is the maintained fork of `tiberius`, which had no
//! release since 2024. The dependency is pinned and gated behind a non default
//! `sqlserver` feature, so a default build carries neither it nor its TLS stack.
//!
//! # Transactions without a transaction handle
//!
//! `tiberius` exposes no transaction object: `BEGIN`, `COMMIT` and `ROLLBACK` are
//! ordinary statements. So [`MssqlTx`] owns a pooled connection for the lifetime
//! of the transaction, which is also what keeps every statement of that
//! transaction on the same session.
//!
//! Dropping the handle without a commit must roll back, but `Drop` cannot await a
//! `ROLLBACK`. The connection is therefore poisoned so the pool discards it rather
//! than handing a session with an open transaction to the next caller, and closing
//! a TDS connection aborts its transaction server side. That costs one connection
//! on an error path, which is rare and strictly better than leaking a write.

use super::dialect::Dialect;
use super::schema::SchemaSet;
use super::{PoolSettings, Query, RelationalDb, RelationalTx, RowValues, SqlValue};
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use std::sync::Arc;
use tiberius::{ColumnData, ColumnType, Row, ToSql};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

/// The tiberius client over a tokio socket, plus the poison flag the pool reads.
pub struct MssqlConnection {
    client: tiberius::Client<Compat<TcpStream>>,
    /// Set when a transaction was abandoned without a commit, so [`TiberiusManager`]
    /// drops the session instead of returning it with work still open.
    poisoned: bool,
}

/// Opens tiberius connections for `bb8`.
pub struct TiberiusManager {
    config: tiberius::Config,
}

impl bb8::ManageConnection for TiberiusManager {
    type Connection = MssqlConnection;
    type Error = tiberius::error::Error;

    async fn connect(&self) -> std::result::Result<Self::Connection, Self::Error> {
        let tcp = TcpStream::connect(self.config.get_addr())
            .await
            .map_err(|e| tiberius::error::Error::Io { kind: e.kind(), message: e.to_string() })?;
        // TDS is request/response, so Nagle would add a round trip of latency to
        // every statement for no benefit.
        tcp.set_nodelay(true)
            .map_err(|e| tiberius::error::Error::Io { kind: e.kind(), message: e.to_string() })?;
        let client = tiberius::Client::connect(self.config.clone(), tcp.compat_write()).await?;
        Ok(MssqlConnection { client, poisoned: false })
    }

    async fn is_valid(&self, conn: &mut Self::Connection) -> std::result::Result<(), Self::Error> {
        conn.client.simple_query("SELECT 1").await?.into_results().await?;
        Ok(())
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.poisoned
    }
}

/// SQL Server as a relational backend.
pub struct SqlServerRelationalDb {
    pool: bb8::Pool<TiberiusManager>,
}

impl SqlServerRelationalDb {
    /// Connect and verify the pool, so a wrong DSN fails here instead of surfacing
    /// as a failed request much later.
    ///
    /// The DSN is an ADO.NET connection string, for example
    /// `Server=tcp:host,1433;Database=mcpfs;User Id=sa;Password=...;TrustServerCertificate=true`.
    pub async fn connect(dsn: &str, pool: PoolSettings) -> Result<Self> {
        // The DSN carries a password, so only the failure kind is ever reported.
        let config = tiberius::Config::from_ado_string(dsn).map_err(|_| {
            ToolError::invalid_argument(
                "the SQL Server dsn is not a valid ADO.NET connection string",
            )
        })?;

        let built = bb8::Pool::builder()
            .max_size(pool.max_connections)
            .connection_timeout(pool.acquire_timeout)
            .build(TiberiusManager { config })
            .await
            .map_err(map_error)?;

        // Prove the credentials now: `build` only tries one connection and a bad
        // password would otherwise surface on the first request.
        let mut probe = built.get().await.map_err(map_pool_error)?;
        probe
            .client
            .simple_query("SELECT 1")
            .await
            .map_err(map_error)?
            .into_results()
            .await
            .map_err(map_error)?;
        drop(probe);

        Ok(Self { pool: built })
    }

    pub fn pool(&self) -> &bb8::Pool<TiberiusManager> {
        &self.pool
    }

    /// Run a statement batch with no parameters, draining its result sets.
    ///
    /// DDL and transaction control go through here: our SQL Server DDL is a
    /// conditional batch (`IF OBJECT_ID(..) IS NULL CREATE TABLE ..`), which the
    /// parameterized RPC path cannot carry.
    async fn batch(conn: &mut MssqlConnection, sql: &str) -> Result<()> {
        conn.client
            .simple_query(sql.to_string())
            .await
            .map_err(map_error)?
            .into_results()
            .await
            .map_err(map_error)?;
        Ok(())
    }
}

// ── error mapping ───────────────────────────────────────────────────────────────

/// SQL Server error numbers worth another attempt.
///
/// Everything here means "the server refused this attempt for a reason that may
/// not repeat", never "the statement is wrong".
fn is_retryable_error_number(number: u32) -> bool {
    matches!(
        number,
        // The engine picked this session as the victim of a deadlock.
        1205
            // Lock request timed out waiting for another transaction.
            | 1222
            // In memory OLTP write conflicts and dependency failures, which are
            // the MSSQL equivalent of a serialization failure.
            | 41301 | 41302 | 41305 | 41325 | 41839
            // Connection dropped or reset under us.
            | 233 | 10053 | 10054 | 10060 | 64
            // Azure SQL transient faults: throttling, failover, not yet ready.
            | 4060 | 40197 | 40501 | 40613 | 49918 | 49919 | 49920
    )
}

/// Map a driver error, keeping the engine name honest and flagging transience.
///
/// The message never carries the DSN: tiberius does not put it in an error, and we
/// do not add it, because it holds a password.
fn map_error(e: tiberius::error::Error) -> ToolError {
    match &e {
        // The socket died. At a statement the work never landed; at a COMMIT it is
        // in doubt, which is why `run_retrying` documents the window.
        tiberius::error::Error::Io { kind, message } => {
            ToolError::internal(format!("sqlserver: connection failed: {kind:?}: {message}"))
                .mark_retryable()
        }
        tiberius::error::Error::Server(token) => {
            let number = token.code();
            let mapped = ToolError::internal(format!("sqlserver: {} [{number}]", token.message()));
            if is_retryable_error_number(number) { mapped.mark_retryable() } else { mapped }
        }
        // A TLS handshake failure is a configuration problem, not a transient one.
        tiberius::error::Error::Tls(message) => {
            ToolError::internal(format!("sqlserver: tls handshake failed: {message}"))
        }
        other => ToolError::internal(format!("sqlserver: {other}")),
    }
}

/// Map a pool error. A checkout timeout is the pool's own state, not a database
/// complaint, and must never read as an internal SQLite failure.
fn map_pool_error(e: bb8::RunError<tiberius::error::Error>) -> ToolError {
    match e {
        bb8::RunError::TimedOut => {
            ToolError::internal("sqlserver: timed out waiting for a pooled connection")
                .mark_retryable()
        }
        bb8::RunError::User(inner) => map_error(inner),
    }
}

// ── value mapping ───────────────────────────────────────────────────────────────

/// Binding goes through the portable value model, so a store never names a driver
/// type. A NULL binds as a NULL string: every nullable column in our schemas is
/// textual, and SQL Server accepts a NULL of any declared type in any column.
impl ToSql for SqlValue {
    fn to_sql(&self) -> ColumnData<'_> {
        match self {
            Self::Null => ColumnData::String(None),
            Self::Int(v) => ColumnData::I64(Some(*v)),
            Self::Real(v) => ColumnData::F64(Some(*v)),
            Self::Text(v) => ColumnData::String(Some(std::borrow::Cow::Borrowed(v))),
            Self::Blob(v) => ColumnData::Binary(Some(std::borrow::Cow::Borrowed(v))),
        }
    }
}

fn params_of(query: &Query) -> Vec<&dyn ToSql> {
    query.params.iter().map(|v| v as &dyn ToSql).collect()
}

/// Read an integer column whose width the server chose.
///
/// A nullable integer arrives as `Intn`, a variable length type, so the width is
/// only known from the payload. Asking for the wrong width is a decode error
/// rather than a cast, exactly as on PostgreSQL.
fn decode_variable_int(row: &Row, index: usize) -> Result<SqlValue> {
    if let Ok(Some(v)) = row.try_get::<i64, _>(index) {
        return Ok(SqlValue::Int(v));
    }
    if let Ok(Some(v)) = row.try_get::<i32, _>(index) {
        return Ok(SqlValue::Int(i64::from(v)));
    }
    if let Ok(Some(v)) = row.try_get::<i16, _>(index) {
        return Ok(SqlValue::Int(i64::from(v)));
    }
    if let Ok(Some(v)) = row.try_get::<u8, _>(index) {
        return Ok(SqlValue::Int(i64::from(v)));
    }
    Err(ToolError::internal(format!("sqlserver: column {index} is not a readable integer")))
}

fn decode_variable_float(row: &Row, index: usize) -> Result<SqlValue> {
    if let Ok(Some(v)) = row.try_get::<f64, _>(index) {
        return Ok(SqlValue::Real(v));
    }
    if let Ok(Some(v)) = row.try_get::<f32, _>(index) {
        return Ok(SqlValue::Real(f64::from(v)));
    }
    Err(ToolError::internal(format!("sqlserver: column {index} is not a readable float")))
}

/// Decode one column into the portable value model, dispatching on the column's
/// own type so a value is never guessed.
fn decode(row: &Row, index: usize) -> Result<SqlValue> {
    let Some(column) = row.columns().get(index) else {
        return Err(ToolError::internal(format!("sqlserver: column index {index} out of range")));
    };

    // A NULL is indistinguishable from an absent value at the typed accessors, so
    // it is resolved per branch below by treating `Ok(None)` as NULL.
    let value = match column.column_type() {
        ColumnType::Bit | ColumnType::Bitn => match row.try_get::<bool, _>(index) {
            Ok(Some(v)) => SqlValue::Int(i64::from(v)),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Int1 => match row.try_get::<u8, _>(index) {
            Ok(Some(v)) => SqlValue::Int(i64::from(v)),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Int2 => match row.try_get::<i16, _>(index) {
            Ok(Some(v)) => SqlValue::Int(i64::from(v)),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Int4 => match row.try_get::<i32, _>(index) {
            Ok(Some(v)) => SqlValue::Int(i64::from(v)),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Int8 => match row.try_get::<i64, _>(index) {
            Ok(Some(v)) => SqlValue::Int(v),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        // Variable width: a nullable BIGINT is `Intn`, and `COUNT(*)` is `Int4`.
        ColumnType::Intn => {
            if row.try_get::<i64, _>(index).ok().flatten().is_none()
                && row.try_get::<i32, _>(index).ok().flatten().is_none()
                && row.try_get::<i16, _>(index).ok().flatten().is_none()
                && row.try_get::<u8, _>(index).ok().flatten().is_none()
            {
                SqlValue::Null
            } else {
                decode_variable_int(row, index)?
            }
        }
        ColumnType::Float4 => match row.try_get::<f32, _>(index) {
            Ok(Some(v)) => SqlValue::Real(f64::from(v)),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Float8 => match row.try_get::<f64, _>(index) {
            Ok(Some(v)) => SqlValue::Real(v),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        ColumnType::Floatn => {
            if row.try_get::<f64, _>(index).ok().flatten().is_none()
                && row.try_get::<f32, _>(index).ok().flatten().is_none()
            {
                SqlValue::Null
            } else {
                decode_variable_float(row, index)?
            }
        }
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => {
            match row.try_get::<&[u8], _>(index) {
                Ok(Some(v)) => SqlValue::Blob(v.to_vec()),
                Ok(None) => SqlValue::Null,
                Err(e) => return Err(map_error(e)),
            }
        }
        ColumnType::NVarchar
        | ColumnType::NChar
        | ColumnType::BigVarChar
        | ColumnType::BigChar
        | ColumnType::Text
        | ColumnType::NText => match row.try_get::<&str, _>(index) {
            Ok(Some(v)) => SqlValue::Text(v.to_string()),
            Ok(None) => SqlValue::Null,
            Err(e) => return Err(map_error(e)),
        },
        // Named rather than silently stringified: our schemas create none of these,
        // so reaching one means a hand altered column and the caller needs to know.
        other => {
            return Err(ToolError::internal(format!(
                "sqlserver: column {index} has unsupported type {other:?}"
            )));
        }
    };
    Ok(value)
}

fn to_rows(rows: Vec<Row>) -> Result<Vec<RowValues>> {
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
    Dialect::SqlServer.render_placeholders(&query.sql)
}

async fn execute_on(conn: &mut MssqlConnection, query: &Query) -> Result<u64> {
    let sql = rendered(query)?;
    let done = conn.client.execute(sql, &params_of(query)).await.map_err(map_error)?;
    // One statement, so the per statement counts collapse to their sum.
    Ok(done.rows_affected().iter().sum())
}

async fn query_on(conn: &mut MssqlConnection, query: &Query) -> Result<Vec<RowValues>> {
    let sql = rendered(query)?;
    let rows = conn
        .client
        .query(sql, &params_of(query))
        .await
        .map_err(map_error)?
        .into_first_result()
        .await
        .map_err(map_error)?;
    to_rows(rows)
}

// ── transaction ─────────────────────────────────────────────────────────────────

/// An open SQL Server transaction, owning its connection for the duration.
struct MssqlTx {
    conn: bb8::PooledConnection<'static, TiberiusManager>,
    committed: bool,
}

impl Drop for MssqlTx {
    fn drop(&mut self) {
        if !self.committed {
            // `Drop` cannot await a ROLLBACK, so the session is poisoned instead:
            // the pool discards it and closing a TDS connection aborts its
            // transaction server side. See the module docs.
            self.conn.poisoned = true;
        }
    }
}

#[async_trait]
impl RelationalTx for MssqlTx {
    async fn execute(&mut self, query: &Query) -> Result<u64> {
        execute_on(&mut self.conn, query).await
    }

    async fn query(&mut self, query: &Query) -> Result<Vec<RowValues>> {
        query_on(&mut self.conn, query).await
    }

    async fn commit(mut self: Box<Self>) -> Result<()> {
        SqlServerRelationalDb::batch(&mut self.conn, "COMMIT TRANSACTION").await?;
        // Only now is the session clean, so `Drop` must not poison it.
        self.committed = true;
        Ok(())
    }
}

#[async_trait]
impl RelationalDb for SqlServerRelationalDb {
    fn dialect(&self) -> Dialect {
        Dialect::SqlServer
    }

    async fn execute(&self, query: &Query) -> Result<u64> {
        let mut conn = self.pool.get().await.map_err(map_pool_error)?;
        execute_on(&mut conn, query).await
    }

    async fn query(&self, query: &Query) -> Result<Vec<RowValues>> {
        let mut conn = self.pool.get().await.map_err(map_pool_error)?;
        query_on(&mut conn, query).await
    }

    async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
        let mut conn = self.pool.get_owned().await.map_err(map_pool_error)?;
        SqlServerRelationalDb::batch(&mut conn, "BEGIN TRANSACTION").await?;
        Ok(Box::new(MssqlTx { conn, committed: false }))
    }

    async fn migrate(&self, schema: &SchemaSet) -> Result<()> {
        let mut conn = self.pool.get_owned().await.map_err(map_pool_error)?;
        // SQL Server has transactional DDL, so a half applied schema cannot be
        // left behind. The guard is a manual rollback because the statements run
        // through `batch` rather than through a transaction handle.
        SqlServerRelationalDb::batch(&mut conn, "BEGIN TRANSACTION").await?;
        let statements = schema
            .render(Dialect::SqlServer)
            .into_iter()
            // Rendered with their own sys.columns guard, so a second apply is a no op.
            .chain(schema.render_column_migrations(Dialect::SqlServer));
        for statement in statements {
            if let Err(e) = SqlServerRelationalDb::batch(&mut conn, &statement).await {
                let _ = SqlServerRelationalDb::batch(&mut conn, "ROLLBACK TRANSACTION").await;
                return Err(e);
            }
        }
        SqlServerRelationalDb::batch(&mut conn, "COMMIT TRANSACTION").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn deadlock_and_lock_timeout_are_retryable() {
        assert!(is_retryable_error_number(1205), "deadlock victim");
        assert!(is_retryable_error_number(1222), "lock request timeout");
    }

    #[test]
    fn in_memory_oltp_write_conflicts_are_retryable() {
        for n in [41301, 41302, 41305, 41325, 41839] {
            assert!(is_retryable_error_number(n), "{n} should be retryable");
        }
    }

    #[test]
    fn dropped_connections_and_azure_faults_are_retryable() {
        for n in [233, 10053, 10054, 10060, 64, 4060, 40197, 40501, 40613, 49918, 49919, 49920] {
            assert!(is_retryable_error_number(n), "{n} should be retryable");
        }
    }

    /// A constraint violation or a syntax error repeats forever, so retrying it
    /// would only multiply the work before failing anyway.
    #[test]
    fn permanent_failures_are_not_retryable() {
        // 2627 primary key, 2601 duplicate index, 547 foreign key, 8152 truncation,
        // 102 syntax, 208 invalid object name.
        for n in [2627, 2601, 547, 8152, 102, 208, 0] {
            assert!(!is_retryable_error_number(n), "{n} must not be retryable");
        }
    }

    /// The pool's own timeout must not be reported as a SQLite internal error,
    /// which is what a blanket mapping would produce.
    #[test]
    fn a_pool_timeout_is_retryable_and_names_sqlserver() {
        let e = map_pool_error(bb8::RunError::TimedOut);
        assert!(e.retryable, "a checkout timeout can succeed on a second attempt");
        assert!(e.message.contains("sqlserver"), "{}", e.message);
        assert!(!e.message.contains("sqlite"), "{}", e.message);
    }

    #[test]
    fn a_dropped_socket_is_retryable_and_names_sqlserver() {
        let e = map_error(tiberius::error::Error::Io {
            kind: std::io::ErrorKind::ConnectionReset,
            message: "reset by peer".into(),
        });
        assert!(e.retryable);
        assert!(e.message.contains("sqlserver: connection failed"), "{}", e.message);
        assert!(!e.message.contains("sqlite"), "{}", e.message);
    }

    /// A TLS problem is a misconfiguration: retrying it just fails three times.
    #[test]
    fn a_tls_failure_is_not_retryable() {
        let e = map_error(tiberius::error::Error::Tls("bad certificate".into()));
        assert!(!e.retryable);
        assert!(e.message.contains("tls handshake failed"), "{}", e.message);
    }

    #[tokio::test]
    async fn a_bad_dsn_is_an_invalid_argument_and_never_echoes_the_password() {
        let e = SqlServerRelationalDb::connect(
            "this is not a connection string at all",
            PoolSettings::default(),
        )
        .await
        .err()
        .expect("a malformed dsn cannot connect");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(
            !e.message.contains("not a connection string"),
            "the dsn must not be echoed: {}",
            e.message
        );
    }

    /// A DSN holds a password, so a connection failure must not quote it back.
    #[tokio::test]
    async fn a_password_never_reaches_an_error_message() {
        let e = SqlServerRelationalDb::connect(
            "Server=tcp:127.0.0.1,1;Database=d;User Id=sa;Password=hunter2;TrustServerCertificate=true",
            PoolSettings { max_connections: 1, acquire_timeout: Duration::from_millis(200) },
        )
        .await
        .err()
        .expect("nothing is listening on port 1");
        assert!(!e.message.contains("hunter2"), "password leaked: {}", e.message);
    }

    // ── live server, opt in ────────────────────────────────────────────────
    //
    // Set MCPFS_TEST_MSSQL_DSN to run these against `docker-compose.test.yml`.
    // They skip themselves when it is unset, so the default gate needs no Docker.

    async fn live_db() -> Option<SqlServerRelationalDb> {
        let dsn = std::env::var("MCPFS_TEST_MSSQL_DSN").ok().filter(|v| !v.trim().is_empty())?;
        Some(
            SqlServerRelationalDb::connect(&dsn, PoolSettings::default())
                .await
                .expect("MCPFS_TEST_MSSQL_DSN is set but unusable"),
        )
    }

    /// One probe table per test.
    ///
    /// These tests assert on whole table contents, so sharing one table made them
    /// fail whenever the runner scheduled two of them at once: a neighbour's
    /// committed row read exactly like a rollback that had not happened. A private
    /// table per test keeps every assertion global and exact while removing the
    /// coupling, which is why the assertions below still read the entire table.
    fn probe_schema(table: &'static str, index: &'static str) -> SchemaSet {
        use super::super::dialect::ColumnType as SchemaColumnType;
        use super::super::schema::{Column, Index, Table};
        SchemaSet::new(
            vec![Table::new(
                table,
                vec![
                    Column::required("k", SchemaColumnType::TextKey(64)),
                    Column::required("n", SchemaColumnType::BigInt),
                    Column::new("r", SchemaColumnType::Double),
                    Column::new("b", SchemaColumnType::Blob),
                    Column::new("t", SchemaColumnType::Text),
                ],
                vec!["k"],
            )],
            vec![Index { name: index, table, columns: vec!["n"] }],
        )
    }

    #[tokio::test]
    async fn live_migrate_is_idempotent_and_round_trips_every_value_type() {
        let Some(db) = live_db().await else {
            return;
        };
        const T: &str = "probe_mssql_values";
        let schema = probe_schema(T, "idx_probe_mssql_values_n");
        db.migrate(&schema).await.unwrap();
        // Applying twice must be a no op, because every store migrates on open.
        db.migrate(&schema).await.unwrap();

        db.execute(&Query::new(format!("DELETE FROM {T}"))).await.unwrap();
        let affected = db
            .execute(
                &Query::new(format!("INSERT INTO {T} (k, n, r, b, t) VALUES (?1, ?2, ?3, ?4, ?5)"))
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
            .query(&Query::new(format!("SELECT k, n, r, b, t FROM {T} WHERE k=?1")).bind("a"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.text(0).unwrap(), "a");
        assert_eq!(r.i64(1).unwrap(), 42);
        assert!((r.f64(2).unwrap() - 1.5).abs() < f64::EPSILON);
        assert_eq!(r.blob(3).unwrap(), vec![0u8, 1, 255]);
        assert_eq!(r.opt_text(4).unwrap(), None);
        assert_eq!(r.columns(), ["k", "n", "r", "b", "t"]);
    }

    #[tokio::test]
    async fn live_a_dropped_transaction_rolls_back() {
        let Some(db) = live_db().await else {
            return;
        };
        const T: &str = "probe_mssql_rollback";
        db.migrate(&probe_schema(T, "idx_probe_mssql_rollback_n")).await.unwrap();
        db.execute(&Query::new(format!("DELETE FROM {T}"))).await.unwrap();

        {
            let mut tx = db.begin().await.unwrap();
            tx.execute(
                &Query::new(format!("INSERT INTO {T} (k, n) VALUES (?1, ?2)"))
                    .bind("gone")
                    .bind(1i64),
            )
            .await
            .unwrap();
            // Dropped without commit: an aborted operation must leave nothing.
        }

        let rows = db.query(&Query::new(format!("SELECT k FROM {T}"))).await.unwrap();
        assert!(rows.is_empty(), "an uncommitted insert must not persist");
    }

    #[tokio::test]
    async fn live_a_committed_transaction_persists() {
        let Some(db) = live_db().await else {
            return;
        };
        const T: &str = "probe_mssql_commit";
        db.migrate(&probe_schema(T, "idx_probe_mssql_commit_n")).await.unwrap();
        db.execute(&Query::new(format!("DELETE FROM {T}"))).await.unwrap();

        let mut tx = db.begin().await.unwrap();
        tx.execute(
            &Query::new(format!("INSERT INTO {T} (k, n) VALUES (?1, ?2)")).bind("kept").bind(2i64),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let rows = db.query(&Query::new(format!("SELECT k, n FROM {T}"))).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text(0).unwrap(), "kept");
        assert_eq!(rows[0].i64(1).unwrap(), 2);
    }

    /// The MERGE the dialect renders has to be accepted by the real server. A unit
    /// test on the SQL string cannot prove that, and an aliased MERGE referring to
    /// its base table is exactly the shape SQL Server rejects.
    #[tokio::test]
    async fn live_a_rendered_upsert_updates_on_conflict() {
        let Some(db) = live_db().await else {
            return;
        };
        const T: &str = "probe_mssql_upsert";
        db.migrate(&probe_schema(T, "idx_probe_mssql_upsert_n")).await.unwrap();
        db.execute(&Query::new(format!("DELETE FROM {T}"))).await.unwrap();

        let upsert = super::super::dialect::Upsert::update(
            T,
            vec!["k", "n"],
            vec!["k"],
            vec![super::super::dialect::Assign::expr("n", "{target}.n + 1")],
        );
        let sql = Dialect::SqlServer.render_upsert(&upsert);
        for _ in 0..3 {
            db.execute(&Query::new(sql.clone()).bind("same").bind(10i64)).await.unwrap();
        }

        let rows = db
            .query(&Query::new(format!("SELECT n FROM {T} WHERE k=?1")).bind("same"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "the upsert must not insert a duplicate");
        assert_eq!(rows[0].i64(0).unwrap(), 12, "inserted at 10, then incremented twice");
    }

    #[tokio::test]
    async fn live_an_unknown_table_is_a_permanent_failure() {
        let Some(db) = live_db().await else {
            return;
        };
        let e = db
            .query(&Query::new("SELECT * FROM table_that_does_not_exist"))
            .await
            .expect_err("an unknown table cannot be queried");
        assert!(!e.retryable, "a missing table will not appear on a retry");
        assert!(e.message.contains("sqlserver:"), "{}", e.message);
    }

    /// A poisoned session must not come back from the pool with its transaction
    /// still open, which would make the next caller inherit uncommitted work.
    #[tokio::test]
    async fn live_a_reused_connection_after_an_abort_is_clean() {
        let Some(db) = live_db().await else {
            return;
        };
        const T: &str = "probe_mssql_abort";
        db.migrate(&probe_schema(T, "idx_probe_mssql_abort_n")).await.unwrap();
        db.execute(&Query::new(format!("DELETE FROM {T}"))).await.unwrap();

        for i in 0..4 {
            let mut tx = db.begin().await.unwrap();
            tx.execute(
                &Query::new(format!("INSERT INTO {T} (k, n) VALUES (?1, ?2)"))
                    .bind(format!("abandoned{i}"))
                    .bind(i),
            )
            .await
            .unwrap();
            drop(tx);
        }

        let rows = db.query(&Query::new(format!("SELECT k FROM {T}"))).await.unwrap();
        assert!(rows.is_empty(), "no abandoned transaction may have committed");

        // The pool still works after discarding those sessions.
        db.execute(
            &Query::new(format!("INSERT INTO {T} (k, n) VALUES (?1, ?2)")).bind("after").bind(1i64),
        )
        .await
        .unwrap();
        let rows = db.query(&Query::new(format!("SELECT k FROM {T}"))).await.unwrap();
        assert_eq!(rows.len(), 1);
    }
}
