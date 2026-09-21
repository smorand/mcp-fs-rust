//! The relational seam: one async surface over SQLite, PostgreSQL and SQL Server.
//!
//! Every store speaks [`Query`] plus [`RowValues`] instead of a driver's own row
//! type, so a statement is written once and the engine is a configuration choice.
//! SQL is authored in a canonical form with `?N` placeholders and rendered per
//! engine by [`Dialect`], which is also where upserts, column types and `LIKE`
//! escaping diverge.
//!
//! # Transactions
//!
//! Multi statement work goes through [`RelationalDb::begin`], which returns an
//! owned [`RelationalTx`] handle. The plan sketched a closure taking form instead
//! (`transaction(TxFn)`), but that shape cannot carry a value out (its result was
//! `Result<()>`) and a higher ranked closure returning a boxed future over a trait
//! object is close to unusable at 50 call sites. An owned handle keeps the trait
//! object safe while letting a store write ordinary sequential async code, which
//! is what the ported stores need: read a row, branch in Rust, then write.
//!
//! Dropping a handle without [`RelationalTx::commit`] rolls the transaction back.
//! That is deliberate: an aborted store operation returns early with `?` and its
//! partial writes must vanish, exactly as the previous SQLite only code did.

pub mod dialect;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod schema;
pub mod sqlite;
#[cfg(feature = "sqlserver")]
pub mod sqlserver;

pub use dialect::{Assign, AssignValue, ColumnType, Dialect, Upsert, UpsertAction};
pub use schema::{Column, ColumnMigration, ForeignKey, Index, SchemaSet, Table};
pub use sqlite::SqliteRelationalDb;

#[cfg(feature = "postgres")]
pub use postgres::PostgresRelationalDb;

#[cfg(feature = "sqlserver")]
pub use sqlserver::SqlServerRelationalDb;

use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Pool sizing, mirrored from `infra.<store>.pool` in the config.
///
/// Engine neutral on purpose: it lives here rather than beside one driver so a
/// build with only the `sqlserver` feature still has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSettings {
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

impl Default for PoolSettings {
    fn default() -> Self {
        Self { max_connections: 10, acquire_timeout: Duration::from_secs(30) }
    }
}

/// A value crossing the driver boundary, in either direction.
///
/// Booleans are absent on purpose: the only boolean we store (`git_refs.symbolic`)
/// is an integer column on every engine, so the mapping stays in the store and the
/// value model has one representation per column type.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqlValue {
    /// The type name, for error messages only.
    fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Int(_) => "integer",
            Self::Real(_) => "real",
            Self::Text(_) => "text",
            Self::Blob(_) => "blob",
        }
    }
}

impl From<i64> for SqlValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<f64> for SqlValue {
    fn from(v: f64) -> Self {
        Self::Real(v)
    }
}
impl From<String> for SqlValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
impl From<&str> for SqlValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_string())
    }
}
/// Borrowed, so a store binds an owned field without cloning it at every call.
impl From<&String> for SqlValue {
    fn from(v: &String) -> Self {
        Self::Text(v.clone())
    }
}
impl From<Vec<u8>> for SqlValue {
    fn from(v: Vec<u8>) -> Self {
        Self::Blob(v)
    }
}
/// `None` binds SQL NULL, so a nullable column needs no special case.
impl<T: Into<SqlValue>> From<Option<T>> for SqlValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => Self::Null,
        }
    }
}

/// One statement plus its bound parameters.
///
/// `sql` uses canonical `?N` placeholders, one based, matching `params` order.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

impl Query {
    pub fn new(sql: impl Into<String>) -> Self {
        Self { sql: sql.into(), params: Vec::new() }
    }

    /// Append one parameter, in placeholder order.
    #[must_use]
    pub fn bind(mut self, value: impl Into<SqlValue>) -> Self {
        self.params.push(value.into());
        self
    }

    /// Append every parameter of an iterator, in order.
    #[must_use]
    pub fn bind_all<T: Into<SqlValue>>(mut self, values: impl IntoIterator<Item = T>) -> Self {
        self.params.extend(values.into_iter().map(Into::into));
        self
    }
}

/// One result row, addressable by column index or by column name.
#[derive(Debug, Clone, PartialEq)]
pub struct RowValues {
    /// Shared across every row of one result set.
    columns: Arc<[String]>,
    values: Vec<SqlValue>,
}

impl RowValues {
    pub fn new(columns: Arc<[String]>, values: Vec<SqlValue>) -> Self {
        Self { columns, values }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The raw value at `index`.
    pub fn value(&self, index: usize) -> Result<&SqlValue> {
        self.values.get(index).ok_or_else(|| {
            ToolError::internal(format!(
                "column index {index} out of range, row has {} columns",
                self.values.len()
            ))
        })
    }

    /// The raw value of the column called `name`.
    pub fn value_named(&self, name: &str) -> Result<&SqlValue> {
        let index = self
            .columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| ToolError::internal(format!("no column named '{name}'")))?;
        self.value(index)
    }

    fn mismatch<T>(&self, index: usize, want: &str) -> Result<T> {
        let got = self.value(index).map(SqlValue::type_name).unwrap_or("missing");
        Err(ToolError::internal(format!("column {index} is {got}, expected {want}")))
    }

    pub fn i64(&self, index: usize) -> Result<i64> {
        match self.value(index)? {
            SqlValue::Int(v) => Ok(*v),
            _ => self.mismatch(index, "integer"),
        }
    }

    /// Reads an integer or a real, because SQLite stores a whole float as an
    /// integer and would otherwise fail a mtime read after a round trip.
    pub fn f64(&self, index: usize) -> Result<f64> {
        match self.value(index)? {
            SqlValue::Real(v) => Ok(*v),
            SqlValue::Int(v) => Ok(*v as f64),
            _ => self.mismatch(index, "real"),
        }
    }

    pub fn text(&self, index: usize) -> Result<String> {
        match self.value(index)? {
            SqlValue::Text(v) => Ok(v.clone()),
            _ => self.mismatch(index, "text"),
        }
    }

    pub fn opt_text(&self, index: usize) -> Result<Option<String>> {
        match self.value(index)? {
            SqlValue::Text(v) => Ok(Some(v.clone())),
            SqlValue::Null => Ok(None),
            _ => self.mismatch(index, "text or null"),
        }
    }

    pub fn blob(&self, index: usize) -> Result<Vec<u8>> {
        match self.value(index)? {
            SqlValue::Blob(v) => Ok(v.clone()),
            _ => self.mismatch(index, "blob"),
        }
    }
}

/// A relational engine.
#[async_trait]
pub trait RelationalDb: Send + Sync {
    /// Which engine this is, so a store can render dialect specific fragments.
    fn dialect(&self) -> Dialect;

    /// Run one statement, returning the number of affected rows.
    async fn execute(&self, query: &Query) -> Result<u64>;

    /// Run one query, returning every row.
    async fn query(&self, query: &Query) -> Result<Vec<RowValues>>;

    /// Run one query, returning the first row if there is one.
    async fn query_opt(&self, query: &Query) -> Result<Option<RowValues>> {
        Ok(self.query(query).await?.into_iter().next())
    }

    /// Open a transaction. Dropping the handle without a commit rolls back.
    async fn begin(&self) -> Result<Box<dyn RelationalTx>>;

    /// Apply a schema. Idempotent, so a store calls it on every open.
    async fn migrate(&self, schema: &SchemaSet) -> Result<()>;
}

/// An open transaction. Every statement runs inside it until [`Self::commit`].
#[async_trait]
pub trait RelationalTx: Send {
    async fn execute(&mut self, query: &Query) -> Result<u64>;
    async fn query(&mut self, query: &Query) -> Result<Vec<RowValues>>;

    async fn query_opt(&mut self, query: &Query) -> Result<Option<RowValues>> {
        Ok(self.query(query).await?.into_iter().next())
    }

    /// Commit. Consumes the handle so a committed transaction cannot be reused.
    async fn commit(self: Box<Self>) -> Result<()>;
}

/// True when `table` exists but does not yet carry `column`: the guard behind
/// [`apply_table_recreations`]. `Ok(false)` when the table does not exist at
/// all, so a fresh install is never mistaken for a legacy deployment.
async fn table_lacks_column(db: &dyn RelationalDb, table: &str, column: &str) -> Result<bool> {
    let rows = db.query(&Query::new(db.dialect().table_columns_query()).bind(table)).await?;
    if rows.is_empty() {
        return Ok(false);
    }
    let has = rows.iter().any(|r| r.text(0).map(|c| c == column).unwrap_or(false));
    Ok(!has)
}

/// Drop every table declared in [`SchemaSet::table_recreations`] whose live
/// shape lacks its guard column, so the following `CREATE TABLE IF NOT EXISTS`
/// rebuilds it on the new shape (DRIFT-005). Called by every engine's
/// `migrate` before it renders the rest of the schema.
///
/// Never fires on a fresh install (no table yet) and never again once the
/// table already carries the guard column, so a restart never destroys rows a
/// previous boot already rebuilt.
pub(crate) async fn apply_table_recreations(
    db: &dyn RelationalDb,
    schema: &SchemaSet,
) -> Result<()> {
    for r in &schema.table_recreations {
        if table_lacks_column(db, r.table, r.guard_column).await? {
            db.execute(&Query::new(format!("DROP TABLE {}", db.dialect().quote_ident(r.table))))
                .await?;
        }
    }
    Ok(())
}

/// How many times [`run_retrying`] runs the work before giving up.
///
/// Three attempts covers a lost race against one or two competing writers, which
/// is the realistic case. More would turn a genuine, repeatable conflict into a
/// long stall while still failing.
pub const MAX_TX_ATTEMPTS: u32 = 3;

/// Run `work` inside a transaction, retrying it whole while the failure is
/// transient (a serialization conflict, a deadlock victim, a pool timeout or a
/// dropped connection: see [`ToolError::retryable`]).
///
/// # Calling this is an idempotence claim
///
/// On a retry `work` runs again from the start against a NEW transaction, so the
/// caller asserts that running it twice has the same effect as running it once.
/// Anything that reads a value, computes from it and writes the result back is
/// fine, because the first attempt's writes were rolled back and the second
/// attempt re-reads. Anything that depends on state OUTSIDE the transaction is
/// not: incrementing an in memory counter, appending to a log, charging a quota,
/// or sending a message would happen twice. Use [`RelationalDb::begin`] directly
/// for those and let the error surface.
///
/// A retry is not free of visible effect on SQLite, where nothing is retryable,
/// so this helper is a no op there beyond the single attempt.
pub async fn run_retrying<T, F>(db: &dyn RelationalDb, work: F) -> Result<T>
where
    F: for<'t> Fn(
        &'t mut (dyn RelationalTx + 't),
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 't>>,
{
    let mut attempt = 1;
    loop {
        // A fresh transaction per attempt: the failed one is already unusable.
        let outcome = match db.begin().await {
            Ok(mut tx) => match work(&mut *tx).await {
                // The commit is part of the attempt: Postgres reports a
                // serialization failure at COMMIT, not at the conflicting write.
                Ok(value) => tx.commit().await.map(|()| value),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };

        match outcome {
            Ok(value) => return Ok(value),
            Err(e) if e.retryable && attempt < MAX_TX_ATTEMPTS => {
                // Linear backoff. The contended resource is a row lock held by a
                // transaction that is about to finish, so a short pause is enough
                // and an exponential curve would only add latency.
                tokio::time::sleep(Duration::from_millis(10 * u64::from(attempt))).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A database that does nothing but count transactions, so the retry policy
    /// can be tested on its own. The engines cannot be used here: SQLite never
    /// produces a retryable error and Postgres would need a live server.
    struct CountingDb {
        begins: AtomicU32,
        /// Number of commits that fail with a retryable error before one succeeds.
        failing_commits: AtomicU32,
    }

    impl CountingDb {
        fn new(failing_commits: u32) -> Self {
            Self { begins: AtomicU32::new(0), failing_commits: AtomicU32::new(failing_commits) }
        }
    }

    struct CountingTx {
        fail_commit: bool,
    }

    #[async_trait]
    impl RelationalTx for CountingTx {
        async fn execute(&mut self, _query: &Query) -> Result<u64> {
            Ok(0)
        }
        async fn query(&mut self, _query: &Query) -> Result<Vec<RowValues>> {
            Ok(Vec::new())
        }
        async fn commit(self: Box<Self>) -> Result<()> {
            if self.fail_commit {
                // Postgres reports a serialization failure at COMMIT, not at the
                // conflicting statement, so this path has to be retryable too.
                return Err(ToolError::internal("commit conflict").mark_retryable());
            }
            Ok(())
        }
    }

    #[async_trait]
    impl RelationalDb for CountingDb {
        fn dialect(&self) -> Dialect {
            Dialect::Postgres
        }
        async fn execute(&self, _query: &Query) -> Result<u64> {
            Ok(0)
        }
        async fn query(&self, _query: &Query) -> Result<Vec<RowValues>> {
            Ok(Vec::new())
        }
        async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
            self.begins.fetch_add(1, Ordering::SeqCst);
            let fail_commit = self
                .failing_commits
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)))
                .is_ok_and(|remaining| remaining > 0);
            Ok(Box::new(CountingTx { fail_commit }))
        }
        async fn migrate(&self, _schema: &SchemaSet) -> Result<()> {
            Ok(())
        }
    }

    /// The work runs again from scratch, and the value of the winning attempt is
    /// the one returned.
    #[tokio::test]
    async fn run_retrying_retries_a_transient_failure_then_succeeds() {
        let db = CountingDb::new(0);
        let attempts = AtomicU32::new(0);

        let value = run_retrying(&db, |_tx| {
            let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                if n < 3 {
                    return Err(ToolError::internal("deadlock victim").mark_retryable());
                }
                Ok(n)
            })
        })
        .await
        .expect("the third attempt succeeds");

        assert_eq!(value, 3, "the successful attempt's value is returned");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(db.begins.load(Ordering::SeqCst), 3, "each attempt gets a fresh transaction");
    }

    /// A constraint violation will fail identically forever, so retrying it only
    /// delays the error.
    #[tokio::test]
    async fn run_retrying_gives_up_at_once_on_a_permanent_failure() {
        let db = CountingDb::new(0);
        let attempts = AtomicU32::new(0);

        let e = run_retrying::<(), _>(&db, |_tx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(ToolError::no_clobber("already exists")) })
        })
        .await
        .expect_err("a permanent failure is returned as is");

        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "no retry without the retryable flag");
    }

    #[tokio::test]
    async fn run_retrying_stops_after_the_attempt_ceiling() {
        let db = CountingDb::new(0);
        let attempts = AtomicU32::new(0);

        let e = run_retrying::<(), _>(&db, |_tx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(ToolError::internal("still conflicting").mark_retryable()) })
        })
        .await
        .expect_err("a repeatable conflict eventually surfaces");

        assert!(e.retryable, "the last error keeps its classification");
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_TX_ATTEMPTS);
    }

    /// The commit is part of the attempt: Postgres raises the serialization error
    /// there, so a commit failure must retry the whole body too.
    #[tokio::test]
    async fn run_retrying_also_retries_a_failing_commit() {
        let db = CountingDb::new(1);
        let attempts = AtomicU32::new(0);

        let value = run_retrying(&db, |_tx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(7) })
        })
        .await
        .expect("the second attempt commits");

        assert_eq!(value, 7);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "the body ran again after the failed commit"
        );
    }

    /// A `RelationalDb` double with no real storage: `query` answers the
    /// `table_columns_query` probe from a seeded column list (or reports the
    /// table absent), `execute` just records what ran, or fails, so the
    /// DRIFT-005 guard (`apply_table_recreations`) can be tested without a live
    /// database.
    struct SpyDb {
        table_exists: bool,
        existing_columns: Vec<&'static str>,
        executed: std::sync::Mutex<Vec<String>>,
        fail_on_drop: bool,
    }

    impl SpyDb {
        fn new(table_exists: bool, existing_columns: Vec<&'static str>) -> Self {
            Self {
                table_exists,
                existing_columns,
                executed: std::sync::Mutex::new(Vec::new()),
                fail_on_drop: false,
            }
        }
    }

    #[async_trait]
    impl RelationalDb for SpyDb {
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }

        async fn execute(&self, query: &Query) -> Result<u64> {
            self.executed.lock().unwrap().push(query.sql.clone());
            if self.fail_on_drop && query.sql.starts_with("DROP TABLE") {
                // Named after the (fake) engine, as a real driver's map_error does.
                return Err(ToolError::internal("spydb: rejected the schema change"));
            }
            Ok(0)
        }

        async fn query(&self, _query: &Query) -> Result<Vec<RowValues>> {
            if !self.table_exists {
                return Ok(Vec::new());
            }
            let cols: Arc<[String]> = vec!["name".to_string()].into();
            Ok(self
                .existing_columns
                .iter()
                .map(|c| RowValues::new(cols.clone(), vec![SqlValue::Text((*c).to_string())]))
                .collect())
        }

        async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
            Err(ToolError::internal("spydb: begin not supported"))
        }

        async fn migrate(&self, _schema: &SchemaSet) -> Result<()> {
            Ok(())
        }
    }

    fn oauth_like_schema() -> SchemaSet {
        SchemaSet::default().recreate_if_missing_column("oauth_tokens", "host")
    }

    /// E2E-NEW-040: a fresh install (no table at all) performs no drop.
    #[tokio::test]
    async fn e2e_new_040_a_fresh_install_performs_no_drop() {
        let db = SpyDb::new(false, vec![]);
        apply_table_recreations(&db, &oauth_like_schema()).await.unwrap();
        assert!(db.executed.lock().unwrap().is_empty(), "a fresh install must not run a DROP");
    }

    #[tokio::test]
    async fn table_recreation_drops_a_legacy_shaped_table() {
        let db = SpyDb::new(true, vec!["person", "provider"]); // no host column
        apply_table_recreations(&db, &oauth_like_schema()).await.unwrap();
        let ran = db.executed.lock().unwrap();
        assert_eq!(ran.len(), 1);
        assert!(ran[0].starts_with("DROP TABLE"), "{}", ran[0]);
    }

    /// This is the guard `survives_reopen_on_disk`
    /// (`crates/mcp-fs/src/git/oauth/persistence.rs`) pins end to end: once the
    /// table already carries the guard column, a re-migrate must not drop it.
    #[tokio::test]
    async fn table_recreation_is_a_no_op_once_the_column_exists() {
        let db = SpyDb::new(true, vec!["person", "host", "provider"]);
        apply_table_recreations(&db, &oauth_like_schema()).await.unwrap();
        assert!(db.executed.lock().unwrap().is_empty(), "already on the new shape: no drop");
    }

    /// E2E-NEW-039: a backend that rejects the schema change fails boot, and the
    /// error names the backend (every real driver's `map_error` already prefixes
    /// its engine name; this proves the guard propagates that failure rather
    /// than swallowing it).
    #[tokio::test]
    async fn e2e_new_039_a_failing_schema_change_fails_boot_naming_the_backend() {
        let mut db = SpyDb::new(true, vec!["person", "provider"]);
        db.fail_on_drop = true;
        let err = apply_table_recreations(&db, &oauth_like_schema()).await.unwrap_err();
        assert!(
            err.message.contains("spydb"),
            "the message must name the backend: {}",
            err.message
        );
    }

    fn row() -> RowValues {
        let cols: Arc<[String]> =
            vec!["path".to_string(), "size".to_string(), "mtime".to_string(), "sha".to_string()]
                .into();
        RowValues::new(
            cols,
            vec![
                SqlValue::Text("/a.txt".into()),
                SqlValue::Int(3),
                SqlValue::Real(1.5),
                SqlValue::Null,
            ],
        )
    }

    #[test]
    fn typed_accessors_read_by_index() {
        let r = row();
        assert_eq!(r.text(0).unwrap(), "/a.txt");
        assert_eq!(r.i64(1).unwrap(), 3);
        assert!((r.f64(2).unwrap() - 1.5).abs() < f64::EPSILON);
        assert_eq!(r.opt_text(3).unwrap(), None);
        assert_eq!(r.len(), 4);
        assert!(!r.is_empty());
    }

    #[test]
    fn values_are_addressable_by_name() {
        let r = row();
        assert_eq!(r.value_named("path").unwrap(), &SqlValue::Text("/a.txt".into()));
        assert_eq!(r.value_named("size").unwrap(), &SqlValue::Int(3));
        assert_eq!(r.columns().len(), 4);
    }

    #[test]
    fn a_wrong_type_is_an_internal_error_not_a_panic() {
        let r = row();
        let e = r.i64(0).expect_err("text is not an integer");
        assert_eq!(e.code, crate::errors::code::INTERNAL_ERROR);
        assert!(e.message.contains("expected integer"), "{}", e.message);
    }

    #[test]
    fn out_of_range_and_unknown_columns_are_reported() {
        let r = row();
        assert!(r.value(9).is_err());
        let e = r.value_named("nope").expect_err("unknown column");
        assert!(e.message.contains("no column named 'nope'"), "{}", e.message);
    }

    /// SQLite hands back a whole float as an integer, so a mtime of exactly 1.0
    /// must still read as a real instead of failing the round trip.
    #[test]
    fn f64_accepts_an_integer_encoded_float() {
        let cols: Arc<[String]> = vec!["mtime".to_string()].into();
        let r = RowValues::new(cols, vec![SqlValue::Int(1)]);
        assert!((r.f64(0).unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn query_binds_in_placeholder_order() {
        let q = Query::new("SELECT a FROM t WHERE p=?1 AND s=?2 AND n=?3")
            .bind("/a.txt")
            .bind(7i64)
            .bind(None::<String>);
        assert_eq!(
            q.params,
            vec![SqlValue::Text("/a.txt".into()), SqlValue::Int(7), SqlValue::Null]
        );
    }

    #[test]
    fn bind_all_appends_every_value() {
        let q = Query::new("x").bind(1i64).bind_all(vec!["a", "b"]);
        assert_eq!(
            q.params,
            vec![SqlValue::Int(1), SqlValue::Text("a".into()), SqlValue::Text("b".into())]
        );
    }

    #[test]
    fn option_binds_null_or_the_inner_value() {
        assert_eq!(SqlValue::from(None::<&str>), SqlValue::Null);
        assert_eq!(SqlValue::from(Some("x")), SqlValue::Text("x".into()));
        assert_eq!(SqlValue::from(Some(4i64)), SqlValue::Int(4));
    }
}
