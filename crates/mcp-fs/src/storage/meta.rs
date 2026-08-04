//! SQLite metadata backend: the directory tree plus blob reference counts.
//! One database file per volume. 1:1 port of the C# `Storage/SqliteMetaStore.cs`
//! including the exact schema, so a volume written by either implementation is
//! readable by the other. This store never touches bytes.

use crate::errors::{Result, ToolError};
use crate::storage::sqlite::SqliteDb;
use crate::storage::traits::{MODE_DIR, MetaBackend, NodeRow};
use crate::util::{PosixPath, now_unix};
use async_trait::async_trait;
use rusqlite::{Row, Transaction};
use std::path::Path;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS nodes (
        path   TEXT PRIMARY KEY,
        parent TEXT,
        name   TEXT NOT NULL,
        kind   TEXT NOT NULL,
        size   INTEGER NOT NULL DEFAULT 0,
        mode   INTEGER NOT NULL,
        mtime  REAL NOT NULL,
        ctime  REAL NOT NULL,
        atime  REAL NOT NULL,
        sha256 TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_nodes_parent ON nodes(parent);
    CREATE TABLE IF NOT EXISTS blob_refs (
        sha256   TEXT PRIMARY KEY,
        refcount INTEGER NOT NULL,
        size     INTEGER NOT NULL
    );
";

const SELECT_COLS: &str =
    "path, parent, name, kind, size, mode, mtime, ctime, atime, sha256";

pub struct SqliteMetaStore {
    db: SqliteDb,
}

impl SqliteMetaStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = SqliteDb::open(path)?;
        db.execute_batch(SCHEMA)?;
        // Ensure the root directory node exists.
        let now = now_unix();
        db.run_sync(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO nodes(path,parent,name,kind,size,mode,mtime,ctime,atime,sha256)
                 VALUES('/',NULL,'','dir',0,?1,?2,?3,?4,NULL)",
                (MODE_DIR, now, now, now),
            )?;
            Ok(())
        })?;
        Ok(Self { db })
    }

    pub fn in_memory() -> Result<Self> {
        let db = SqliteDb::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        let now = now_unix();
        db.run_sync(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO nodes(path,parent,name,kind,size,mode,mtime,ctime,atime,sha256)
                 VALUES('/',NULL,'','dir',0,?1,?2,?3,?4,NULL)",
                (MODE_DIR, now, now, now),
            )?;
            Ok(())
        })?;
        Ok(Self { db })
    }

    fn read_row(r: &Row<'_>) -> rusqlite::Result<NodeRow> {
        Ok(NodeRow {
            path: r.get(0)?,
            parent: r.get(1)?,
            name: r.get(2)?,
            kind: r.get(3)?,
            size: r.get(4)?,
            mode: r.get(5)?,
            mtime: r.get(6)?,
            ctime: r.get(7)?,
            atime: r.get(8)?,
            sha256: r.get(9)?,
        })
    }

    // ── sync helpers, all inside a transaction ───────────────────────────────

    fn exists(tx: &Transaction<'_>, path: &str) -> Result<bool> {
        let n: i64 = tx.query_row(
            "SELECT COUNT(*) FROM nodes WHERE path=?1",
            [path],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    fn incref(tx: &Transaction<'_>, sha: Option<&str>, size: i64) -> Result<()> {
        if let Some(sha) = sha {
            tx.execute(
                "INSERT INTO blob_refs(sha256,refcount,size) VALUES(?1,1,?2)
                 ON CONFLICT(sha256) DO UPDATE SET refcount=refcount+1",
                (sha, size),
            )?;
        }
        Ok(())
    }

    /// Decrement a blob refcount. Returns true when it hit 0 (caller GCs the blob).
    fn decref(tx: &Transaction<'_>, sha: Option<&str>) -> Result<bool> {
        let Some(sha) = sha else { return Ok(false) };
        let current: Option<i64> = tx
            .query_row("SELECT refcount FROM blob_refs WHERE sha256=?1", [sha], |r| r.get(0))
            .ok();
        let Some(refcount) = current else { return Ok(false) };
        let remaining = refcount - 1;
        if remaining <= 0 {
            tx.execute("DELETE FROM blob_refs WHERE sha256=?1", [sha])?;
            Ok(true)
        } else {
            tx.execute(
                "UPDATE blob_refs SET refcount=?1 WHERE sha256=?2",
                (remaining, sha),
            )?;
            Ok(false)
        }
    }

    fn insert_dir(tx: &Transaction<'_>, path: &str) -> Result<()> {
        let now = now_unix();
        tx.execute(
            "INSERT INTO nodes(path,parent,name,kind,size,mode,mtime,ctime,atime,sha256)
             VALUES(?1,?2,?3,'dir',0,?4,?5,?6,?7,NULL)",
            (
                path,
                PosixPath::parent_of(path),
                PosixPath::name_of(path),
                MODE_DIR,
                now,
                now,
                now,
            ),
        )?;
        Ok(())
    }

    fn mkdirs_chain(tx: &Transaction<'_>, path: &str) -> Result<()> {
        let mut current = String::new();
        for part in path.trim_matches('/').split('/').filter(|p| !p.is_empty()) {
            current = format!("{current}/{part}");
            let kind: Option<String> = tx
                .query_row("SELECT kind FROM nodes WHERE path=?1", [&current], |r| r.get(0))
                .ok();
            match kind.as_deref() {
                None => Self::insert_dir(tx, &current)?,
                Some("dir") => {}
                Some(_) => {
                    return Err(ToolError::no_clobber(format!(
                        "'{current}' already exists and is not a directory"
                    )));
                }
            }
        }
        Ok(())
    }

    fn ensure_parents(tx: &Transaction<'_>, path: &str) -> Result<()> {
        if let Some(parent) = PosixPath::parent_of(path)
            && parent != "/" {
                Self::mkdirs_chain(tx, &parent)?;
            }
        Ok(())
    }
}

#[async_trait]
impl MetaBackend for SqliteMetaStore {
    async fn get(&self, path: &str) -> Result<Option<NodeRow>> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                let mut st = tx.prepare(&format!(
                    "SELECT {SELECT_COLS} FROM nodes WHERE path=?1"
                ))?;
                let mut rows = st.query([&path])?;
                match rows.next()? {
                    Some(r) => Ok(Some(SqliteMetaStore::read_row(r)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn list_children(&self, parent: &str) -> Result<Vec<NodeRow>> {
        let parent = parent.to_string();
        self.db
            .run(move |tx| {
                let mut st = tx.prepare(&format!(
                    "SELECT {SELECT_COLS} FROM nodes WHERE parent=?1 ORDER BY name"
                ))?;
                let out = st
                    .query_map([&parent], SqliteMetaStore::read_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn subtree(&self, root: &str) -> Result<Vec<NodeRow>> {
        let root = root.to_string();
        self.db
            .run(move |tx| {
                let prefix = format!("{}/%", root.trim_end_matches('/'));
                let mut st = tx.prepare(&format!(
                    "SELECT {SELECT_COLS} FROM nodes WHERE path=?1 OR path LIKE ?2 ORDER BY path"
                ))?;
                let out = st
                    .query_map([&root, &prefix], SqliteMetaStore::read_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn put_file(
        &self,
        path: &str,
        sha256: Option<&str>,
        size: i64,
        mode: i64,
    ) -> Result<Option<String>> {
        let path = path.to_string();
        let sha = sha256.map(str::to_string);
        self.db
            .run(move |tx| {
                Self::ensure_parents(tx, &path)?;
                // Preserve the original ctime when overwriting.
                let existing: Option<(String, f64, Option<String>)> = tx
                    .query_row(
                        "SELECT kind, ctime, sha256 FROM nodes WHERE path=?1",
                        [&path],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .ok();
                let mut ctime = now_unix();
                let mut old_sha: Option<String> = None;
                if let Some((kind, c, s)) = existing {
                    if kind == "dir" {
                        return Err(ToolError::invalid_argument(format!(
                            "'{path}' is a directory"
                        )));
                    }
                    ctime = c;
                    old_sha = s;
                }
                let now = now_unix();
                let mut gc = None;
                if old_sha.as_deref() != sha.as_deref() {
                    Self::incref(tx, sha.as_deref(), size)?;
                    if Self::decref(tx, old_sha.as_deref())? {
                        gc = old_sha;
                    }
                }
                tx.execute(
                    "INSERT OR REPLACE INTO nodes(path,parent,name,kind,size,mode,mtime,ctime,atime,sha256)
                     VALUES(?1,?2,?3,'file',?4,?5,?6,?7,?8,?9)",
                    (
                        &path,
                        PosixPath::parent_of(&path),
                        PosixPath::name_of(&path),
                        size,
                        mode,
                        now,
                        ctime,
                        now,
                        &sha,
                    ),
                )?;
                Ok(gc)
            })
            .await
    }

    async fn delete_file(&self, path: &str) -> Result<Option<String>> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                let found: Option<(String, Option<String>)> = tx
                    .query_row(
                        "SELECT kind, sha256 FROM nodes WHERE path=?1",
                        [&path],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                let Some((kind, sha)) = found else {
                    return Err(ToolError::not_found(format!("'{path}' not found")));
                };
                if kind == "dir" {
                    return Err(ToolError::invalid_argument(format!("'{path}' is a directory")));
                }
                let gc = if Self::decref(tx, sha.as_deref())? { sha } else { None };
                tx.execute("DELETE FROM nodes WHERE path=?1", [&path])?;
                Ok(gc)
            })
            .await
    }

    async fn remove_subtree(&self, path: &str) -> Result<Vec<String>> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                let prefix = format!("{}/%", path.trim_end_matches('/'));
                let shas: Vec<Option<String>> = {
                    let mut st = tx.prepare(
                        "SELECT sha256 FROM nodes WHERE path=?1 OR path LIKE ?2",
                    )?;
                    st.query_map([&path, &prefix], |r| r.get(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut gc = Vec::new();
                for sha in shas.iter() {
                    if sha.is_some() && Self::decref(tx, sha.as_deref())? {
                        gc.push(sha.clone().unwrap());
                    }
                }
                tx.execute(
                    "DELETE FROM nodes WHERE path=?1 OR path LIKE ?2",
                    [&path, &prefix],
                )?;
                Ok(gc)
            })
            .await
    }

    async fn mkdirs(&self, path: &str, exist_ok: bool) -> Result<()> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                let kind: Option<String> = tx
                    .query_row("SELECT kind FROM nodes WHERE path=?1", [&path], |r| r.get(0))
                    .ok();
                if let Some(kind) = kind {
                    if kind != "dir" || !exist_ok {
                        return Err(ToolError::no_clobber(format!("'{path}' already exists")));
                    }
                    return Ok(());
                }
                Self::mkdirs_chain(tx, &path)
            })
            .await
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                if let Some(parent) = PosixPath::parent_of(&path)
                    && !Self::exists(tx, &parent)? {
                        return Err(ToolError::not_found(format!("'{parent}' not found")));
                    }
                if Self::exists(tx, &path)? {
                    return Err(ToolError::no_clobber(format!("'{path}' already exists")));
                }
                Self::insert_dir(tx, &path)
            })
            .await
    }

    async fn rmdir(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db
            .run(move |tx| {
                tx.execute("DELETE FROM nodes WHERE path=?1 AND kind='dir'", [&path])?;
                Ok(())
            })
            .await
    }

    async fn rename(&self, src: &str, dst: &str) -> Result<()> {
        let src = src.to_string();
        let dst = dst.to_string();
        self.db
            .run(move |tx| {
                if !Self::exists(tx, &src)? {
                    return Err(ToolError::not_found(format!("'{src}' not found")));
                }
                if Self::exists(tx, &dst)? {
                    return Err(ToolError::no_clobber(format!("'{dst}' already exists")));
                }
                Self::ensure_parents(tx, &dst)?;
                let prefix = format!("{}/%", src.trim_end_matches('/'));
                let paths: Vec<String> = {
                    let mut st = tx.prepare(
                        "SELECT path FROM nodes WHERE path=?1 OR path LIKE ?2 ORDER BY length(path)",
                    )?;
                    st.query_map([&src, &prefix], |r| r.get(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for old in paths {
                    let new = if old == src {
                        dst.clone()
                    } else {
                        format!("{}{}", dst, &old[src.len()..])
                    };
                    tx.execute(
                        "UPDATE nodes SET path=?1, parent=?2, name=?3 WHERE path=?4",
                        (
                            &new,
                            PosixPath::parent_of(&new),
                            PosixPath::name_of(&new),
                            &old,
                        ),
                    )?;
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::traits::MODE_FILE;

    fn store() -> SqliteMetaStore {
        SqliteMetaStore::in_memory().unwrap()
    }

    #[tokio::test]
    async fn root_exists_after_open() {
        let s = store();
        let root = s.get("/").await.unwrap().expect("root node");
        assert_eq!(root.kind, "dir");
        assert_eq!(root.name, "");
        assert_eq!(root.parent, None);
        assert_eq!(root.mode, MODE_DIR);
    }

    #[tokio::test]
    async fn put_file_creates_parents_and_increfs() {
        let s = store();
        let gc = s.put_file("/a/b/c.txt", Some("sha1"), 3, MODE_FILE).await.unwrap();
        assert_eq!(gc, None);
        assert!(s.get("/a").await.unwrap().unwrap().is_dir());
        assert!(s.get("/a/b").await.unwrap().unwrap().is_dir());
        let f = s.get("/a/b/c.txt").await.unwrap().unwrap();
        assert!(f.is_file());
        assert_eq!(f.size, 3);
        assert_eq!(f.sha256.as_deref(), Some("sha1"));
        assert_eq!(f.name, "c.txt");
        assert_eq!(f.parent.as_deref(), Some("/a/b"));
    }

    /// Two files with identical content share one blob (refcount 2); the blob is
    /// only GC'd on the second delete.
    #[tokio::test]
    async fn dedup_refcount_and_gc_at_zero() {
        let s = store();
        s.put_file("/a.txt", Some("same"), 4, MODE_FILE).await.unwrap();
        s.put_file("/b.txt", Some("same"), 4, MODE_FILE).await.unwrap();

        let gc1 = s.delete_file("/a.txt").await.unwrap();
        assert_eq!(gc1, None, "refcount was 2, nothing to GC yet");

        let gc2 = s.delete_file("/b.txt").await.unwrap();
        assert_eq!(gc2.as_deref(), Some("same"), "refcount hit 0, GC the blob");
    }

    #[tokio::test]
    async fn overwrite_gcs_old_blob_and_keeps_ctime() {
        let s = store();
        s.put_file("/a.txt", Some("old"), 3, MODE_FILE).await.unwrap();
        let ctime0 = s.get("/a.txt").await.unwrap().unwrap().ctime;

        let gc = s.put_file("/a.txt", Some("new"), 3, MODE_FILE).await.unwrap();
        assert_eq!(gc.as_deref(), Some("old"), "the replaced blob is GC'd");
        let f = s.get("/a.txt").await.unwrap().unwrap();
        assert_eq!(f.sha256.as_deref(), Some("new"));
        assert_eq!(f.ctime, ctime0, "ctime is preserved across overwrite");
    }

    #[tokio::test]
    async fn rewriting_same_sha_does_not_gc() {
        let s = store();
        s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        let gc = s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        assert_eq!(gc, None);
    }

    /// An empty file stores no blob: sha256 is NULL and nothing is refcounted.
    #[tokio::test]
    async fn empty_file_has_no_blob() {
        let s = store();
        s.put_file("/e.txt", None, 0, MODE_FILE).await.unwrap();
        let f = s.get("/e.txt").await.unwrap().unwrap();
        assert_eq!(f.sha256, None);
        assert_eq!(f.size, 0);
        let gc = s.delete_file("/e.txt").await.unwrap();
        assert_eq!(gc, None);
    }

    #[tokio::test]
    async fn list_children_is_sorted_by_name() {
        let s = store();
        for p in ["/c.txt", "/a.txt", "/b.txt"] {
            s.put_file(p, Some(p), 1, MODE_FILE).await.unwrap();
        }
        let names: Vec<String> =
            s.list_children("/").await.unwrap().into_iter().map(|n| n.name).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);
    }

    #[tokio::test]
    async fn subtree_includes_root_and_descendants() {
        let s = store();
        s.put_file("/d/x.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/d/sub/y.txt", Some("2"), 1, MODE_FILE).await.unwrap();
        s.put_file("/outside.txt", Some("3"), 1, MODE_FILE).await.unwrap();

        let paths: Vec<String> =
            s.subtree("/d").await.unwrap().into_iter().map(|n| n.path).collect();
        assert!(paths.contains(&"/d".to_string()));
        assert!(paths.contains(&"/d/x.txt".to_string()));
        assert!(paths.contains(&"/d/sub/y.txt".to_string()));
        assert!(!paths.contains(&"/outside.txt".to_string()));
    }

    #[tokio::test]
    async fn remove_subtree_gcs_all_orphaned_blobs() {
        let s = store();
        s.put_file("/d/a.txt", Some("s1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/d/b.txt", Some("s2"), 1, MODE_FILE).await.unwrap();
        let mut gc = s.remove_subtree("/d").await.unwrap();
        gc.sort();
        assert_eq!(gc, vec!["s1", "s2"]);
        assert!(s.get("/d").await.unwrap().is_none());
        assert!(s.get("/d/a.txt").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_subtree_keeps_blob_shared_outside() {
        let s = store();
        s.put_file("/d/a.txt", Some("shared"), 1, MODE_FILE).await.unwrap();
        s.put_file("/keep.txt", Some("shared"), 1, MODE_FILE).await.unwrap();
        let gc = s.remove_subtree("/d").await.unwrap();
        assert!(gc.is_empty(), "blob still referenced by /keep.txt");
    }

    #[tokio::test]
    async fn mkdir_requires_existing_parent_and_rejects_duplicates() {
        let s = store();
        let e = s.mkdir("/nope/child").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);

        s.mkdir("/x").await.unwrap();
        let e = s.mkdir("/x").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn mkdirs_is_idempotent_when_exist_ok() {
        let s = store();
        s.mkdirs("/a/b/c", true).await.unwrap();
        s.mkdirs("/a/b/c", true).await.unwrap();
        assert!(s.get("/a/b/c").await.unwrap().unwrap().is_dir());

        let e = s.mkdirs("/a/b/c", false).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn rename_moves_whole_subtree() {
        let s = store();
        s.put_file("/src/a.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/src/sub/b.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        s.rename("/src", "/dst").await.unwrap();
        assert!(s.get("/src").await.unwrap().is_none());
        assert!(s.get("/dst/a.txt").await.unwrap().is_some());
        let moved = s.get("/dst/sub/b.txt").await.unwrap().unwrap();
        assert_eq!(moved.parent.as_deref(), Some("/dst/sub"));
        assert_eq!(moved.name, "b.txt");
    }

    #[tokio::test]
    async fn rename_rejects_missing_source_and_existing_target() {
        let s = store();
        s.put_file("/a.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/b.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        assert_eq!(
            s.rename("/nope", "/x").await.unwrap_err().code,
            crate::errors::code::NOT_FOUND
        );
        assert_eq!(
            s.rename("/a.txt", "/b.txt").await.unwrap_err().code,
            crate::errors::code::NO_CLOBBER
        );
    }

    #[tokio::test]
    async fn delete_file_errors_are_typed() {
        let s = store();
        assert_eq!(
            s.delete_file("/nope").await.unwrap_err().code,
            crate::errors::code::NOT_FOUND
        );
        s.mkdir("/d").await.unwrap();
        assert_eq!(
            s.delete_file("/d").await.unwrap_err().code,
            crate::errors::code::INVALID_ARGUMENT
        );
    }

    #[tokio::test]
    async fn put_file_over_a_directory_is_rejected() {
        let s = store();
        s.mkdir("/d").await.unwrap();
        let e = s.put_file("/d", Some("x"), 1, MODE_FILE).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }
}
