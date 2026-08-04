//! Serialized SQLite access.
//!
//! Mirrors the C# `Storage/SqliteDb.cs` contract: one connection per database,
//! every access serialized behind a mutex and run on the blocking thread pool so
//! request handlers never block, WAL journal, busy timeout. Each `run` executes
//! inside a transaction (commit on Ok, rollback on Err).

use crate::errors::{Result, ToolError};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SqliteDb {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteDb {
    /// Open (creating parent dirs and the file if needed) and apply the pragmas.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// In-memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Run a closure inside a transaction, synchronously. Prefer [`Self::run`].
    pub fn run_sync<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| ToolError::internal("sqlite mutex poisoned"))?;
        let tx = guard.transaction()?;
        match f(&tx) {
            Ok(v) => {
                tx.commit()?;
                Ok(v)
            }
            Err(e) => {
                // rollback is implicit on drop, but be explicit about intent
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    /// Run a closure inside a transaction on the blocking pool.
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T> + Send + 'static,
    {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.run_sync(f))
            .await
            .map_err(|e| ToolError::internal(format!("sqlite task join: {e}")))?
    }

    /// Execute DDL / statements without a transaction wrapper (schema setup).
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| ToolError::internal("sqlite mutex poisoned"))?;
        guard.execute_batch(sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> SqliteDb {
        let db = SqliteDb::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (k TEXT PRIMARY KEY, v INTEGER NOT NULL);")
            .unwrap();
        db
    }

    #[tokio::test]
    async fn commits_on_ok() {
        let db = db();
        db.run(|tx| {
            tx.execute("INSERT INTO t (k, v) VALUES (?1, ?2)", ("a", 1))?;
            Ok(())
        })
        .await
        .unwrap();

        let n: i64 = db
            .run(|tx| Ok(tx.query_row("SELECT v FROM t WHERE k='a'", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn rolls_back_on_err() {
        let db = db();
        let res: Result<()> = db
            .run(|tx| {
                tx.execute("INSERT INTO t (k, v) VALUES (?1, ?2)", ("b", 2))?;
                Err(ToolError::internal("boom"))
            })
            .await;
        assert!(res.is_err());

        let count: i64 = db
            .run(|tx| Ok(tx.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(count, 0, "failed transaction must not persist rows");
    }

    #[tokio::test]
    async fn concurrent_writes_are_serialized() {
        let db = db();
        let mut handles = Vec::new();
        for i in 0..25 {
            let d = db.clone();
            handles.push(tokio::spawn(async move {
                d.run(move |tx| {
                    tx.execute("INSERT INTO t (k, v) VALUES (?1, ?2)", (format!("k{i}"), i))?;
                    Ok(())
                })
                .await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("no write should fail or report 'database is locked'");
        }
        let count: i64 = db
            .run(|tx| Ok(tx.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(count, 25);
    }

    #[test]
    fn open_creates_parent_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nested/deeper/x.db");
        let db = SqliteDb::open(&p).unwrap();
        db.execute_batch("CREATE TABLE z (a INTEGER);").unwrap();
        assert!(p.exists());
    }
}
