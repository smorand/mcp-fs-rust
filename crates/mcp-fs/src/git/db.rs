//! Per project SQLite index for git metadata (objects, refs, remotes).
//!
//! Port of the C# `Git/SqliteGitDb.cs`. Same three tables, same column names and
//! types, same statement semantics (`INSERT OR REPLACE` upserts), so a database
//! written by either implementation is readable by the other.
//!
//! Physical path: `state/git/{project_id}.db` (see `ServerConfig::git_db_path`).

use crate::errors::Result;
use crate::storage::sqlite::SqliteDb;
use std::path::Path;

/// One row of the `git_objects` index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitObjectRow {
    pub hash: String,
    /// "blob" | "tree" | "commit" | "tag"
    pub kind: String,
    pub size: i64,
}

/// One row of the `git_refs` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRefRow {
    pub name: String,
    pub target: String,
    pub symbolic: bool,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS git_objects (
    hash    TEXT PRIMARY KEY NOT NULL,
    type    TEXT NOT NULL,
    size    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS git_refs (
    name     TEXT PRIMARY KEY NOT NULL,
    target   TEXT NOT NULL,
    symbolic INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS git_remotes (
    name TEXT PRIMARY KEY NOT NULL,
    url  TEXT NOT NULL
);
";

#[derive(Clone)]
pub struct SqliteGitDb {
    db: SqliteDb,
}

impl SqliteGitDb {
    /// Open (creating the file and its parent directories) and apply the schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = SqliteDb::open(path)?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    /// In memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let db = SqliteDb::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    // ── objects ─────────────────────────────────────────────────────────────

    /// Index one object. `size` is the payload length, header excluded.
    pub async fn record_object(&self, hash: &str, kind: &str, size: i64) -> Result<()> {
        let (hash, kind) = (hash.to_string(), kind.to_string());
        self.db
            .run(move |tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO git_objects(hash, type, size) VALUES(?1, ?2, ?3)",
                    (&hash, &kind, size),
                )?;
                Ok(())
            })
            .await
    }

    pub async fn object_exists(&self, hash: &str) -> Result<bool> {
        let hash = hash.to_string();
        self.db
            .run(move |tx| {
                let n: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM git_objects WHERE hash=?1",
                    [&hash],
                    |r| r.get(0),
                )?;
                Ok(n > 0)
            })
            .await
    }

    pub async fn get_object(&self, hash: &str) -> Result<Option<GitObjectRow>> {
        let hash = hash.to_string();
        self.db
            .run(move |tx| {
                let mut st = tx.prepare(
                    "SELECT hash, type, size FROM git_objects WHERE hash=?1",
                )?;
                let mut rows = st.query([&hash])?;
                match rows.next()? {
                    Some(r) => Ok(Some(GitObjectRow {
                        hash: r.get(0)?,
                        kind: r.get(1)?,
                        size: r.get(2)?,
                    })),
                    None => Ok(None),
                }
            })
            .await
    }

    /// Every indexed hash starting with `prefix`. Used for short sha resolution.
    pub async fn find_objects_by_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let like = format!("{prefix}%");
        self.db
            .run(move |tx| {
                let mut st = tx.prepare(
                    "SELECT hash FROM git_objects WHERE hash LIKE ?1 ORDER BY hash",
                )?;
                let rows = st.query_map([&like], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    /// Full object index. Replaces the C# `ForEachObjectAsync` callback: returning
    /// the rows keeps the SQLite lock short and lets the caller await freely.
    pub async fn list_objects(&self) -> Result<Vec<GitObjectRow>> {
        self.db
            .run(|tx| {
                let mut st = tx.prepare("SELECT hash, type, size FROM git_objects")?;
                let rows = st.query_map([], |r| {
                    Ok(GitObjectRow { hash: r.get(0)?, kind: r.get(1)?, size: r.get(2)? })
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn count_objects(&self) -> Result<i64> {
        self.db
            .run(|tx| Ok(tx.query_row("SELECT COUNT(*) FROM git_objects", [], |r| r.get(0))?))
            .await
    }

    // ── refs ────────────────────────────────────────────────────────────────

    pub async fn get_ref(&self, name: &str) -> Result<Option<GitRefRow>> {
        let name = name.to_string();
        self.db
            .run(move |tx| {
                let mut st =
                    tx.prepare("SELECT target, symbolic FROM git_refs WHERE name=?1")?;
                let mut rows = st.query([&name])?;
                match rows.next()? {
                    Some(r) => Ok(Some(GitRefRow {
                        name: name.clone(),
                        target: r.get(0)?,
                        symbolic: r.get::<_, i64>(1)? != 0,
                    })),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn set_ref(&self, name: &str, target: &str, symbolic: bool) -> Result<()> {
        let (name, target) = (name.to_string(), target.to_string());
        self.db
            .run(move |tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO git_refs(name, target, symbolic) VALUES(?1, ?2, ?3)",
                    (&name, &target, i64::from(symbolic)),
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delete_ref(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.db
            .run(move |tx| {
                tx.execute("DELETE FROM git_refs WHERE name=?1", [&name])?;
                Ok(())
            })
            .await
    }

    /// All refs ordered by name, exactly like the C# `ListRefsAsync`.
    pub async fn list_refs(&self) -> Result<Vec<GitRefRow>> {
        self.db
            .run(|tx| {
                let mut st =
                    tx.prepare("SELECT name, target, symbolic FROM git_refs ORDER BY name")?;
                let rows = st.query_map([], |r| {
                    Ok(GitRefRow {
                        name: r.get(0)?,
                        target: r.get(1)?,
                        symbolic: r.get::<_, i64>(2)? != 0,
                    })
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    // ── remotes ─────────────────────────────────────────────────────────────

    pub async fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        let (name, url) = (name.to_string(), url.to_string());
        self.db
            .run(move |tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO git_remotes(name, url) VALUES(?1, ?2)",
                    (&name, &url),
                )?;
                Ok(())
            })
            .await
    }

    pub async fn remove_remote(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.db
            .run(move |tx| {
                tx.execute("DELETE FROM git_remotes WHERE name=?1", [&name])?;
                Ok(())
            })
            .await
    }

    pub async fn list_remotes(&self) -> Result<Vec<(String, String)>> {
        self.db
            .run(|tx| {
                let mut st = tx.prepare("SELECT name, url FROM git_remotes ORDER BY name")?;
                let rows = st.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> SqliteGitDb {
        SqliteGitDb::open_in_memory().unwrap()
    }

    #[tokio::test]
    async fn schema_has_the_three_csharp_tables() {
        let d = db().await;
        let names: Vec<String> = d
            .db
            .run(|tx| {
                let mut st = tx.prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                )?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?;
                let mut v = Vec::new();
                for r in rows {
                    v.push(r?);
                }
                Ok(v)
            })
            .await
            .unwrap();
        assert_eq!(names, vec!["git_objects", "git_refs", "git_remotes"]);
    }

    #[tokio::test]
    async fn record_object_then_read_back() {
        let d = db().await;
        d.record_object("aa11", "blob", 7).await.unwrap();
        assert!(d.object_exists("aa11").await.unwrap());
        assert!(!d.object_exists("bb22").await.unwrap());
        assert_eq!(
            d.get_object("aa11").await.unwrap().unwrap(),
            GitObjectRow { hash: "aa11".into(), kind: "blob".into(), size: 7 }
        );
        assert_eq!(d.get_object("nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn record_object_is_an_upsert() {
        let d = db().await;
        d.record_object("dup", "blob", 1).await.unwrap();
        d.record_object("dup", "commit", 42).await.unwrap();
        assert_eq!(d.count_objects().await.unwrap(), 1);
        let row = d.get_object("dup").await.unwrap().unwrap();
        assert_eq!(row.kind, "commit");
        assert_eq!(row.size, 42);
    }

    #[tokio::test]
    async fn find_objects_by_prefix() {
        let d = db().await;
        for h in ["abcd01", "abcd02", "abce03", "ffff04"] {
            d.record_object(h, "blob", 1).await.unwrap();
        }
        assert_eq!(
            d.find_objects_by_prefix("abcd").await.unwrap(),
            vec!["abcd01", "abcd02"]
        );
        assert_eq!(d.find_objects_by_prefix("abc").await.unwrap().len(), 3);
        assert_eq!(d.find_objects_by_prefix("ffff04").await.unwrap(), vec!["ffff04"]);
        assert!(d.find_objects_by_prefix("zz").await.unwrap().is_empty());
        // an empty prefix matches everything, like the C# LIKE '%'
        assert_eq!(d.find_objects_by_prefix("").await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn list_objects_returns_every_row() {
        let d = db().await;
        d.record_object("h1", "tree", 3).await.unwrap();
        d.record_object("h2", "tag", 9).await.unwrap();
        let mut got = d.list_objects().await.unwrap();
        got.sort_by(|a, b| a.hash.cmp(&b.hash));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, "tree");
        assert_eq!(got[1].size, 9);
    }

    #[tokio::test]
    async fn set_and_get_direct_ref() {
        let d = db().await;
        let sha = "1111111111111111111111111111111111111111";
        d.set_ref("refs/heads/main", sha, false).await.unwrap();
        let r = d.get_ref("refs/heads/main").await.unwrap().unwrap();
        assert_eq!(r.name, "refs/heads/main");
        assert_eq!(r.target, sha);
        assert!(!r.symbolic);
        assert_eq!(d.get_ref("refs/heads/absent").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_and_get_symbolic_ref() {
        let d = db().await;
        d.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
        let r = d.get_ref("HEAD").await.unwrap().unwrap();
        assert_eq!(r.target, "refs/heads/main");
        assert!(r.symbolic, "HEAD must round trip as symbolic");
    }

    #[tokio::test]
    async fn set_ref_overwrites_and_can_flip_symbolic() {
        let d = db().await;
        d.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
        d.set_ref("HEAD", "abc", false).await.unwrap();
        let r = d.get_ref("HEAD").await.unwrap().unwrap();
        assert_eq!(r.target, "abc");
        assert!(!r.symbolic);
        assert_eq!(d.list_refs().await.unwrap().len(), 1, "upsert, not insert");
    }

    #[tokio::test]
    async fn list_refs_is_ordered_by_name() {
        let d = db().await;
        d.set_ref("refs/tags/v1", "t1", false).await.unwrap();
        d.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
        d.set_ref("refs/heads/main", "m1", false).await.unwrap();
        let names: Vec<String> =
            d.list_refs().await.unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["HEAD", "refs/heads/main", "refs/tags/v1"]);
    }

    #[tokio::test]
    async fn delete_ref_is_idempotent() {
        let d = db().await;
        d.set_ref("refs/heads/x", "s", false).await.unwrap();
        d.delete_ref("refs/heads/x").await.unwrap();
        assert_eq!(d.get_ref("refs/heads/x").await.unwrap(), None);
        // deleting again must not error
        d.delete_ref("refs/heads/x").await.unwrap();
    }

    #[tokio::test]
    async fn remotes_upsert_list_and_remove() {
        let d = db().await;
        d.add_remote("origin", "https://example.test/a.git").await.unwrap();
        d.add_remote("upstream", "https://example.test/b.git").await.unwrap();
        d.add_remote("origin", "https://example.test/c.git").await.unwrap();
        assert_eq!(
            d.list_remotes().await.unwrap(),
            vec![
                ("origin".to_string(), "https://example.test/c.git".to_string()),
                ("upstream".to_string(), "https://example.test/b.git".to_string()),
            ]
        );
        d.remove_remote("origin").await.unwrap();
        assert_eq!(d.list_remotes().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn state_survives_reopen_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git").join("proj.db");
        {
            let d = SqliteGitDb::open(&path).unwrap();
            d.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
            d.record_object("deadbeef", "commit", 120).await.unwrap();
        }
        assert!(path.exists(), "open must create the db file and its parents");

        let d2 = SqliteGitDb::open(&path).unwrap();
        assert!(d2.get_ref("HEAD").await.unwrap().unwrap().symbolic);
        assert_eq!(d2.get_object("deadbeef").await.unwrap().unwrap().size, 120);
    }
}
