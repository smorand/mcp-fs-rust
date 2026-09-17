//! SQLite behind [`RelationalDb`], wrapping the existing [`SqliteDb`].
//!
//! The connection stays single and serialized, so this adapter changes no
//! behaviour: it only changes the shape callers speak. Statements still run on the
//! blocking pool, never on a request thread.
//!
//! A transaction is driven by a short lived actor. `rusqlite::Transaction` borrows
//! its `Connection` and a `MutexGuard` is not `Send`, so the transaction cannot be
//! held across an `.await`. Instead one blocking task takes the connection lock,
//! opens the transaction and serves statements over a channel until it is told to
//! commit or the handle is dropped. Holding the lock for the transaction's whole
//! life is exactly the serialization this store has always had.

use super::dialect::Dialect;
use super::schema::SchemaSet;
use super::{Query, RelationalDb, RelationalTx, RowValues, SqlValue};
use crate::errors::{Result, ToolError};
use crate::storage::sqlite::SqliteDb;
use async_trait::async_trait;
use rusqlite::Connection;
use rusqlite::types::Value as RawValue;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// SQLite as a relational backend.
pub struct SqliteRelationalDb {
    db: SqliteDb,
}

impl SqliteRelationalDb {
    /// Open a database file, creating it and its parents when absent.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self { db: SqliteDb::open(path)? })
    }

    /// In memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self { db: SqliteDb::open_in_memory()? })
    }

    /// Wrap an already open handle.
    pub fn from_db(db: SqliteDb) -> Self {
        Self { db }
    }
}

// ── value mapping ───────────────────────────────────────────────────────────────

fn to_raw(value: &SqlValue) -> RawValue {
    match value {
        SqlValue::Null => RawValue::Null,
        SqlValue::Int(v) => RawValue::Integer(*v),
        SqlValue::Real(v) => RawValue::Real(*v),
        SqlValue::Text(v) => RawValue::Text(v.clone()),
        SqlValue::Blob(v) => RawValue::Blob(v.clone()),
    }
}

fn from_raw(value: RawValue) -> SqlValue {
    match value {
        RawValue::Null => SqlValue::Null,
        RawValue::Integer(v) => SqlValue::Int(v),
        RawValue::Real(v) => SqlValue::Real(v),
        RawValue::Text(v) => SqlValue::Text(v),
        RawValue::Blob(v) => SqlValue::Blob(v),
    }
}

fn bound(query: &Query) -> Vec<RawValue> {
    query.params.iter().map(to_raw).collect()
}

/// Run a statement. Works on a connection or a transaction, which derefs to one.
fn exec_on(conn: &Connection, query: &Query) -> Result<u64> {
    let sql = Dialect::Sqlite.render_placeholders(&query.sql)?;
    let affected = conn.execute(&sql, rusqlite::params_from_iter(bound(query)))?;
    Ok(affected as u64)
}

fn query_on(conn: &Connection, query: &Query) -> Result<Vec<RowValues>> {
    let sql = Dialect::Sqlite.render_placeholders(&query.sql)?;
    let mut stmt = conn.prepare(&sql)?;
    let columns: Arc<[String]> =
        stmt.column_names().iter().map(|c| (*c).to_string()).collect::<Vec<_>>().into();
    let width = columns.len();
    let mut rows = stmt.query(rusqlite::params_from_iter(bound(query)))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(width);
        for i in 0..width {
            values.push(from_raw(row.get::<_, RawValue>(i)?));
        }
        out.push(RowValues::new(columns.clone(), values));
    }
    Ok(out)
}

// ── transaction actor ───────────────────────────────────────────────────────────

enum TxCommand {
    Execute { query: Query, reply: oneshot::Sender<Result<u64>> },
    Query { query: Query, reply: oneshot::Sender<Result<Vec<RowValues>>> },
    Commit { reply: oneshot::Sender<Result<()>> },
}

/// Handle to a transaction served by a blocking task.
struct SqliteTx {
    commands: mpsc::Sender<TxCommand>,
}

impl SqliteTx {
    /// The actor is gone only if it panicked or already committed, both of which
    /// mean this handle is unusable.
    fn closed() -> ToolError {
        ToolError::internal("sqlite transaction is no longer open")
    }

    async fn send(&self, command: TxCommand) -> Result<()> {
        self.commands.send(command).await.map_err(|_| Self::closed())
    }
}

#[async_trait]
impl RelationalTx for SqliteTx {
    async fn execute(&mut self, query: &Query) -> Result<u64> {
        let (reply, answer) = oneshot::channel();
        self.send(TxCommand::Execute { query: query.clone(), reply }).await?;
        answer.await.map_err(|_| Self::closed())?
    }

    async fn query(&mut self, query: &Query) -> Result<Vec<RowValues>> {
        let (reply, answer) = oneshot::channel();
        self.send(TxCommand::Query { query: query.clone(), reply }).await?;
        answer.await.map_err(|_| Self::closed())?
    }

    async fn commit(self: Box<Self>) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.send(TxCommand::Commit { reply }).await?;
        answer.await.map_err(|_| Self::closed())?
    }
}

#[async_trait]
impl RelationalDb for SqliteRelationalDb {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn execute(&self, query: &Query) -> Result<u64> {
        let query = query.clone();
        self.db.run(move |tx| exec_on(tx, &query)).await
    }

    async fn query(&self, query: &Query) -> Result<Vec<RowValues>> {
        let query = query.clone();
        self.db.run(move |tx| query_on(tx, &query)).await
    }

    async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
        let (commands, mut inbox) = mpsc::channel::<TxCommand>(1);
        let (ready, opened) = oneshot::channel::<Result<()>>();
        let db = self.db.clone();

        // Detached on purpose: the task owns the connection lock until it commits
        // or rolls back, so the next begin simply waits on that lock.
        tokio::task::spawn_blocking(move || {
            let _ = db.with_connection_blocking(|conn| {
                let tx = match conn.transaction() {
                    Ok(tx) => tx,
                    Err(e) => {
                        let _ = ready.send(Err(ToolError::from(e)));
                        return Ok(());
                    }
                };
                if ready.send(Ok(())).is_err() {
                    // Caller vanished before the transaction opened.
                    return Ok(());
                }
                while let Some(command) = inbox.blocking_recv() {
                    match command {
                        TxCommand::Execute { query, reply } => {
                            let _ = reply.send(exec_on(&tx, &query));
                        }
                        TxCommand::Query { query, reply } => {
                            let _ = reply.send(query_on(&tx, &query));
                        }
                        TxCommand::Commit { reply } => {
                            let _ = reply.send(tx.commit().map_err(ToolError::from));
                            return Ok(());
                        }
                    }
                }
                // The handle was dropped without a commit, so dropping `tx` here
                // rolls back: an aborted operation must leave no partial write.
                Ok(())
            });
        });

        match opened.await {
            Ok(Ok(())) => Ok(Box::new(SqliteTx { commands })),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ToolError::internal("sqlite transaction never opened")),
        }
    }

    async fn migrate(&self, schema: &SchemaSet) -> Result<()> {
        let statements = schema.render(Dialect::Sqlite);
        if !statements.is_empty() {
            let sql = format!("{};", statements.join(";\n"));
            let db = self.db.clone();
            tokio::task::spawn_blocking(move || db.execute_batch(&sql))
                .await
                .map_err(|e| ToolError::internal(format!("sqlite migrate task join: {e}")))??;
        }

        // SQLite has no `ADD COLUMN IF NOT EXISTS`, so the guard is a probe: the
        // alternative, swallowing a "duplicate column name" error, would also
        // swallow a genuinely broken migration.
        for (m, statement) in
            schema.column_migrations.iter().zip(schema.render_column_migrations(Dialect::Sqlite))
        {
            let present = self
                .query_opt(
                    &Query::new("SELECT 1 FROM pragma_table_info(?1) WHERE name=?2")
                        .bind(m.table)
                        .bind(m.column),
                )
                .await?
                .is_some();
            if !present {
                self.execute(&Query::new(statement)).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::rel::dialect::ColumnType;
    use crate::storage::rel::schema::{Column, Index, Table};

    fn schema() -> SchemaSet {
        SchemaSet::new(
            vec![Table::new(
                "t",
                vec![
                    Column::required("k", ColumnType::TextKey(200)),
                    Column::required("n", ColumnType::BigInt).default("0"),
                    Column::new("r", ColumnType::Double),
                    Column::new("b", ColumnType::Blob),
                ],
                vec!["k"],
            )],
            vec![Index { name: "idx_t_n", table: "t", columns: vec!["n"] }],
        )
    }

    async fn db() -> SqliteRelationalDb {
        let db = SqliteRelationalDb::open_in_memory().unwrap();
        db.migrate(&schema()).await.unwrap();
        db
    }

    #[tokio::test]
    async fn dialect_is_sqlite() {
        assert_eq!(db().await.dialect(), Dialect::Sqlite);
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let db = db().await;
        // Every store calls migrate on open, so a second apply must be a no op.
        db.migrate(&schema()).await.unwrap();
        let names = db.query(&Query::new(Dialect::Sqlite.table_names_query())).await.unwrap();
        let names: Vec<String> = names.iter().map(|r| r.text(0).unwrap()).collect();
        assert_eq!(names, vec!["t"]);
    }

    #[tokio::test]
    async fn every_value_type_round_trips() {
        let db = db().await;
        let affected = db
            .execute(
                &Query::new("INSERT INTO t (k, n, r, b) VALUES (?1, ?2, ?3, ?4)")
                    .bind("a")
                    .bind(7i64)
                    .bind(1.5f64)
                    .bind(vec![0u8, 159, 255]),
            )
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let row = db
            .query_opt(&Query::new("SELECT k, n, r, b FROM t WHERE k=?1").bind("a"))
            .await
            .unwrap()
            .expect("row present");
        assert_eq!(row.text(0).unwrap(), "a");
        assert_eq!(row.i64(1).unwrap(), 7);
        assert!((row.f64(2).unwrap() - 1.5).abs() < f64::EPSILON);
        assert_eq!(row.blob(3).unwrap(), vec![0u8, 159, 255]);
    }

    #[tokio::test]
    async fn null_round_trips_as_none() {
        let db = db().await;
        db.execute(
            &Query::new("INSERT INTO t (k, n, r) VALUES (?1, ?2, ?3)")
                .bind("a")
                .bind(1i64)
                .bind(None::<f64>),
        )
        .await
        .unwrap();
        let row = db
            .query_opt(&Query::new("SELECT b FROM t WHERE k=?1").bind("a"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.value(0).unwrap(), &SqlValue::Null);
    }

    #[tokio::test]
    async fn query_opt_is_none_on_no_rows() {
        let db = db().await;
        let row =
            db.query_opt(&Query::new("SELECT k FROM t WHERE k=?1").bind("missing")).await.unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn rows_carry_their_column_names() {
        let db = db().await;
        db.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(1i64))
            .await
            .unwrap();
        let row = db.query_opt(&Query::new("SELECT k, n FROM t")).await.unwrap().unwrap();
        assert_eq!(row.columns(), ["k".to_string(), "n".to_string()]);
        assert_eq!(row.value_named("n").unwrap(), &SqlValue::Int(1));
    }

    #[tokio::test]
    async fn committed_transaction_persists_every_statement() {
        let db = db().await;
        let mut tx = db.begin().await.unwrap();
        tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(1i64))
            .await
            .unwrap();
        tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("b").bind(2i64))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let n = db.query(&Query::new("SELECT k FROM t")).await.unwrap().len();
        assert_eq!(n, 2);
    }

    /// A store aborts by returning early with `?`, which drops the handle. The
    /// partial writes must vanish, matching the previous rollback on error.
    #[tokio::test]
    async fn dropping_the_handle_rolls_back() {
        let db = db().await;
        {
            let mut tx = db.begin().await.unwrap();
            tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(1i64))
                .await
                .unwrap();
        }
        let rows = db.query(&Query::new("SELECT k FROM t")).await.unwrap();
        assert!(rows.is_empty(), "an uncommitted write must not persist");
    }

    #[tokio::test]
    async fn a_transaction_reads_its_own_uncommitted_writes() {
        let db = db().await;
        let mut tx = db.begin().await.unwrap();
        tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(5i64))
            .await
            .unwrap();
        let row = tx
            .query_opt(&Query::new("SELECT n FROM t WHERE k=?1").bind("a"))
            .await
            .unwrap()
            .expect("visible inside the transaction");
        assert_eq!(row.i64(0).unwrap(), 5);
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn a_failed_statement_leaves_the_transaction_usable() {
        let db = db().await;
        let mut tx = db.begin().await.unwrap();
        tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(1i64))
            .await
            .unwrap();
        // Duplicate primary key: the statement fails, the transaction does not.
        let e = tx
            .execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(2i64))
            .await
            .expect_err("duplicate key must fail");
        assert_eq!(e.code, crate::errors::code::INTERNAL_ERROR);
        tx.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("c").bind(3i64))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows = db.query(&Query::new("SELECT k FROM t ORDER BY k")).await.unwrap();
        let keys: Vec<String> = rows.iter().map(|r| r.text(0).unwrap()).collect();
        assert_eq!(keys, vec!["a", "c"]);
    }

    /// Transactions must not interleave: the connection lock is held for the whole
    /// transaction, so concurrent writers queue instead of corrupting each other.
    #[tokio::test]
    async fn concurrent_transactions_are_serialized() {
        let db = Arc::new(db().await);
        let mut handles = Vec::new();
        for i in 0..12i64 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                let mut tx = db.begin().await.unwrap();
                tx.execute(
                    &Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)")
                        .bind(format!("k{i}"))
                        .bind(i),
                )
                .await
                .unwrap();
                tx.commit().await.unwrap();
            }));
        }
        for h in handles {
            h.await.expect("no writer should fail or deadlock");
        }
        let n = db.query(&Query::new("SELECT k FROM t")).await.unwrap().len();
        assert_eq!(n, 12);
    }

    #[tokio::test]
    async fn a_committed_handle_cannot_be_reused() {
        let db = db().await;
        let tx = db.begin().await.unwrap();
        tx.commit().await.unwrap();
        // The handle was consumed by commit, so reuse is a compile error. What is
        // observable here is that the next transaction still opens cleanly.
        let mut next = db.begin().await.unwrap();
        next.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("z").bind(1i64))
            .await
            .unwrap();
        next.commit().await.unwrap();
    }

    #[tokio::test]
    async fn placeholders_beyond_nine_bind_correctly() {
        let db = SqliteRelationalDb::open_in_memory().unwrap();
        let cols: Vec<String> = (0..11).map(|i| format!("c{i}")).collect();
        let ddl = format!(
            "CREATE TABLE wide ({});",
            cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", ")
        );
        db.db.execute_batch(&ddl).unwrap();

        let placeholders: Vec<String> = (1..=11).map(|i| format!("?{i}")).collect();
        let q = Query::new(format!(
            "INSERT INTO wide ({}) VALUES ({})",
            cols.join(", "),
            placeholders.join(", ")
        ))
        .bind_all((0..11i64).collect::<Vec<_>>());
        db.execute(&q).await.unwrap();

        let row = db.query_opt(&Query::new("SELECT c10 FROM wide")).await.unwrap().unwrap();
        assert_eq!(row.i64(0).unwrap(), 10);
    }

    #[tokio::test]
    async fn on_disk_state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/x.db");
        {
            let db = SqliteRelationalDb::open(&path).unwrap();
            db.migrate(&schema()).await.unwrap();
            db.execute(&Query::new("INSERT INTO t (k, n) VALUES (?1, ?2)").bind("a").bind(1i64))
                .await
                .unwrap();
        }
        assert!(path.exists(), "open creates parent directories");

        let reopened = SqliteRelationalDb::open(&path).unwrap();
        reopened.migrate(&schema()).await.unwrap();
        let row = reopened
            .query_opt(&Query::new("SELECT n FROM t WHERE k=?1").bind("a"))
            .await
            .unwrap()
            .expect("row survived");
        assert_eq!(row.i64(0).unwrap(), 1);
    }
}
