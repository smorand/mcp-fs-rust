//! Per project git index (objects, refs, remotes) on any [`RelationalDb`].
//!
//! Rows carry a `volume_id` so one PostgreSQL or SQL Server database can hold the
//! index of every project. Under SQLite there is still one file per project, at
//! `state/git/{project_id}.db` (see `ServerConfig::git_db_path`), and the column
//! is constant within it.
//!
//! Object bytes are NOT here: they live in the blob store under `git:{sha}`. This
//! table is an index over them, which is what makes a short sha lookup cheap.

use crate::errors::Result;
use crate::storage::rel::dialect::{ColumnType, Upsert};
use crate::storage::rel::schema::{Column, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb};
use std::sync::Arc;

/// A project id is at most 32 characters.
const VOLUME_ID_LEN: u32 = 64;
/// A hex sha256 is 64 characters; doubled so a longer digest still fits a key.
const HASH_LEN: u32 = 128;
/// Ref names are short in practice, but a tag can nest deeply.
const REF_NAME_LEN: u32 = 400;

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

/// The tables this store owns, named once so a purge cannot miss one.
pub const TABLES: [&str; 3] = ["git_objects", "git_refs", "git_remotes"];

/// The tables this store owns.
pub fn schema() -> SchemaSet {
    let volume = || Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN));
    SchemaSet::new(
        vec![
            Table::new(
                "git_objects",
                vec![
                    volume(),
                    Column::required("hash", ColumnType::TextKey(HASH_LEN)),
                    Column::required("type", ColumnType::Text),
                    Column::required("size", ColumnType::BigInt),
                ],
                vec!["volume_id", "hash"],
            ),
            Table::new(
                "git_refs",
                vec![
                    volume(),
                    Column::required("name", ColumnType::TextKey(REF_NAME_LEN)),
                    Column::required("target", ColumnType::Text),
                    Column::required("symbolic", ColumnType::BigInt).default("0"),
                ],
                vec!["volume_id", "name"],
            ),
            Table::new(
                "git_remotes",
                vec![
                    volume(),
                    Column::required("name", ColumnType::TextKey(REF_NAME_LEN)),
                    Column::required("url", ColumnType::Text),
                ],
                vec!["volume_id", "name"],
            ),
        ],
        Vec::new(),
    )
}

pub struct RelationalGitDb {
    db: Arc<dyn RelationalDb>,
    volume_id: String,
}

impl RelationalGitDb {
    /// Apply the schema and return the index for one project.
    pub async fn open(db: Arc<dyn RelationalDb>, volume_id: impl Into<String>) -> Result<Self> {
        let me = Self { db, volume_id: volume_id.into() };
        me.db.migrate(&schema()).await?;
        Ok(me)
    }

    /// In memory SQLite index, for tests.
    pub async fn open_in_memory() -> Result<Self> {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open_in_memory()?);
        Self::open(db, "test").await
    }

    pub fn volume_id(&self) -> &str {
        &self.volume_id
    }

    /// Table names visible to this connection, for diagnostics and tests.
    pub async fn table_names(&self) -> Result<Vec<String>> {
        let rows = self.db.query(&Query::new(self.db.dialect().table_names_query())).await?;
        rows.iter().map(|r| r.text(0)).collect()
    }

    fn upsert_sql(&self, table: &'static str, columns: Vec<&'static str>) -> String {
        self.db.dialect().render_upsert(&Upsert::replace(table, columns, vec!["volume_id", "name"]))
    }

    // ── objects ─────────────────────────────────────────────────────────────

    /// Index one object. `size` is the payload length, header excluded.
    pub async fn record_object(&self, hash: &str, kind: &str, size: i64) -> Result<()> {
        let sql = self.db.dialect().render_upsert(&Upsert::replace(
            "git_objects",
            vec!["volume_id", "hash", "type", "size"],
            vec!["volume_id", "hash"],
        ));
        self.db
            .execute(&Query::new(sql).bind(&self.volume_id).bind(hash).bind(kind).bind(size))
            .await?;
        Ok(())
    }

    pub async fn object_exists(&self, hash: &str) -> Result<bool> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT 1 FROM git_objects WHERE volume_id=?1 AND hash=?2")
                    .bind(&self.volume_id)
                    .bind(hash),
            )
            .await?;
        Ok(row.is_some())
    }

    pub async fn get_object(&self, hash: &str) -> Result<Option<GitObjectRow>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT hash, type, size FROM git_objects \
                     WHERE volume_id=?1 AND hash=?2",
                )
                .bind(&self.volume_id)
                .bind(hash),
            )
            .await?;
        match row {
            Some(r) => {
                Ok(Some(GitObjectRow { hash: r.text(0)?, kind: r.text(1)?, size: r.i64(2)? }))
            }
            None => Ok(None),
        }
    }

    /// Every indexed hash starting with `prefix`. Used for short sha resolution.
    ///
    /// The prefix is escaped: a sha is hex so it cannot hold a wildcard today, but
    /// an unescaped `LIKE` argument is the bug this store just fixed elsewhere.
    pub async fn find_objects_by_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let pattern = format!("{}%", self.db.dialect().escape_like_literal(prefix));
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT hash FROM git_objects \
                     WHERE volume_id=?1 AND hash LIKE ?2 ESCAPE '\\' ORDER BY hash",
                )
                .bind(&self.volume_id)
                .bind(pattern),
            )
            .await?;
        rows.iter().map(|r| r.text(0)).collect()
    }

    /// Full object index. Returning rows keeps the database lock short and lets
    /// the caller await freely.
    pub async fn list_objects(&self) -> Result<Vec<GitObjectRow>> {
        let rows = self
            .db
            .query(
                &Query::new("SELECT hash, type, size FROM git_objects WHERE volume_id=?1")
                    .bind(&self.volume_id),
            )
            .await?;
        rows.iter()
            .map(|r| Ok(GitObjectRow { hash: r.text(0)?, kind: r.text(1)?, size: r.i64(2)? }))
            .collect()
    }

    pub async fn count_objects(&self) -> Result<i64> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT COUNT(*) FROM git_objects WHERE volume_id=?1")
                    .bind(&self.volume_id),
            )
            .await?;
        match row {
            Some(r) => r.i64(0),
            None => Ok(0),
        }
    }

    // ── refs ────────────────────────────────────────────────────────────────

    pub async fn get_ref(&self, name: &str) -> Result<Option<GitRefRow>> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT target, symbolic FROM git_refs WHERE volume_id=?1 AND name=?2")
                    .bind(&self.volume_id)
                    .bind(name),
            )
            .await?;
        match row {
            Some(r) => Ok(Some(GitRefRow {
                name: name.to_string(),
                target: r.text(0)?,
                // Stored as an integer on every engine, so the boolean mapping
                // stays here rather than in the value model.
                symbolic: r.i64(1)? != 0,
            })),
            None => Ok(None),
        }
    }

    pub async fn set_ref(&self, name: &str, target: &str, symbolic: bool) -> Result<()> {
        let sql = self.upsert_sql("git_refs", vec!["volume_id", "name", "target", "symbolic"]);
        self.db
            .execute(
                &Query::new(sql)
                    .bind(&self.volume_id)
                    .bind(name)
                    .bind(target)
                    .bind(i64::from(symbolic)),
            )
            .await?;
        Ok(())
    }

    pub async fn delete_ref(&self, name: &str) -> Result<()> {
        self.db
            .execute(
                &Query::new("DELETE FROM git_refs WHERE volume_id=?1 AND name=?2")
                    .bind(&self.volume_id)
                    .bind(name),
            )
            .await?;
        Ok(())
    }

    /// All refs ordered by name.
    pub async fn list_refs(&self) -> Result<Vec<GitRefRow>> {
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT name, target, symbolic FROM git_refs \
                     WHERE volume_id=?1 ORDER BY name",
                )
                .bind(&self.volume_id),
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok(GitRefRow { name: r.text(0)?, target: r.text(1)?, symbolic: r.i64(2)? != 0 })
            })
            .collect()
    }

    // ── remotes ─────────────────────────────────────────────────────────────

    pub async fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        let sql = self.upsert_sql("git_remotes", vec!["volume_id", "name", "url"]);
        self.db.execute(&Query::new(sql).bind(&self.volume_id).bind(name).bind(url)).await?;
        Ok(())
    }

    pub async fn remove_remote(&self, name: &str) -> Result<()> {
        self.db
            .execute(
                &Query::new("DELETE FROM git_remotes WHERE volume_id=?1 AND name=?2")
                    .bind(&self.volume_id)
                    .bind(name),
            )
            .await?;
        Ok(())
    }

    pub async fn list_remotes(&self) -> Result<Vec<(String, String)>> {
        let rows = self
            .db
            .query(
                &Query::new("SELECT name, url FROM git_remotes WHERE volume_id=?1 ORDER BY name")
                    .bind(&self.volume_id),
            )
            .await?;
        rows.iter().map(|r| Ok((r.text(0)?, r.text(1)?))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> RelationalGitDb {
        RelationalGitDb::open_in_memory().await.unwrap()
    }

    /// Introspection goes through the dialect instead of `sqlite_master`, which
    /// only exists on one of the three engines.
    #[tokio::test]
    async fn schema_has_the_three_tables() {
        let d = db().await;
        assert_eq!(d.table_names().await.unwrap(), vec!["git_objects", "git_refs", "git_remotes"]);
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
        assert_eq!(d.find_objects_by_prefix("abcd").await.unwrap(), vec!["abcd01", "abcd02"]);
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
        let names: Vec<String> = d.list_refs().await.unwrap().into_iter().map(|r| r.name).collect();
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
        let open = |p: &std::path::Path| {
            let db = crate::storage::rel::SqliteRelationalDb::open(p).unwrap();
            RelationalGitDb::open(Arc::new(db), "proj")
        };
        {
            let d = open(&path).await.unwrap();
            d.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
            d.record_object("deadbeef", "commit", 120).await.unwrap();
        }
        assert!(path.exists(), "open must create the db file and its parents");

        let d2 = open(&path).await.unwrap();
        assert!(d2.get_ref("HEAD").await.unwrap().unwrap().symbolic);
        assert_eq!(d2.get_object("deadbeef").await.unwrap().unwrap().size, 120);
    }
}
