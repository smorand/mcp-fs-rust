//! Metadata backend: the directory tree plus blob reference counts.
//!
//! Storage is any [`RelationalDb`], so the engine is a configuration choice. Rows
//! carry a `volume_id` discriminator: under SQLite there is still one database
//! file per volume and the column is constant, while a single PostgreSQL or SQL
//! Server database can hold every volume without a database per project.
//!
//! This store never touches bytes. It hands back the sha of a blob whose refcount
//! reached zero and the caller garbage collects it.

use crate::errors::{Result, ToolError};
use crate::storage::rel::dialect::{Assign, ColumnType, Dialect, Upsert};
use crate::storage::rel::schema::{Column, Index, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb, RelationalTx, RowValues, run_retrying};
use crate::storage::traits::{MODE_DIR, MetaBackend, NodeRow};
use crate::util::{PosixPath, now_unix};
use async_trait::async_trait;
use std::sync::Arc;

/// A project id is at most 32 characters (see `admin::validate_project_id`), so
/// this leaves room to spare while staying inside a SQL Server index key.
const VOLUME_ID_LEN: u32 = 64;
/// Keyed because the primary key and the parent index both use a path. SQL Server
/// cannot index unbounded text, so a length is required rather than optional.
const PATH_LEN: u32 = 450;

/// SQL Server caps a CLUSTERED index key at 900 bytes, and `NVARCHAR` costs 2 bytes
/// per character, so the primary key `(volume_id, path)` has 450 characters to share.
///
/// The limit bites on the actual row, not on the declared widths: a 449 character
/// path under a 9 character volume id built a 916 byte key and the server refused it
/// with error 1946. So the column may be wider than any path this store will accept.
const SQLSERVER_KEY_CHARS: usize = 900 / 2;

/// `admin::validate_project_id` caps a project id at 32 characters, and the volume id
/// IS the project id, so this is the worst case share of the key budget. Budgeting the
/// worst case keeps the ceiling identical in every project: a path that fits in one
/// volume must not be refused in another just because its name is longer.
const MAX_VOLUME_ID_CHARS: usize = 32;

/// The longest path that fits the primary key whatever the project id is called.
///
/// Deliberately below [`PATH_LEN`]: the column can hold more characters than a key
/// entry can carry, and it is the key that fails. Public because
/// [`max_path_len`] turns it into the caller facing limit, and the guard that rejects
/// an over long path MUST derive from the same arithmetic as the schema.
pub const MAX_PATH_CHARS: usize = SQLSERVER_KEY_CHARS - MAX_VOLUME_ID_CHARS;

// The character budget must really fit 900 bytes. Checked at compile time because the
// tempting mistake is to raise the budget as though a character cost one byte, and
// that failure would otherwise appear only against a real SQL Server.
const _: () = assert!(SQLSERVER_KEY_CHARS * 2 <= 900);
/// A hex sha256 is 64 characters. Doubled, so a longer digest stays storable.
const SHA_LEN: u32 = 128;

const SELECT_COLS: &str = "path, parent, name, kind, size, mode, mtime, ctime, atime, sha256";

/// Every column of `nodes`, in the order the upserts bind them.
const NODE_COLS: [&str; 11] = [
    "volume_id",
    "path",
    "parent",
    "name",
    "kind",
    "size",
    "mode",
    "mtime",
    "ctime",
    "atime",
    "sha256",
];

/// The longest path the metadata backend can store, when it has a limit at all.
///
/// Only SQL Server does: `path` is part of the primary key and of the `parent`
/// index, and SQL Server cannot index unbounded text, so the column is
/// `NVARCHAR(PATH_LEN)` there while SQLite and PostgreSQL take unbounded `TEXT`.
/// Returning `None` for those two is deliberate: a PostgreSQL deployment must not
/// inherit a restriction that only exists because of another engine.
pub fn max_path_len(backend: &str) -> Option<usize> {
    match backend {
        crate::config::backend::SQLSERVER => Some(MAX_PATH_CHARS),
        _ => None,
    }
}

/// [`max_path_len`] for a store that already holds a dialect rather than the
/// configured backend name. Same constant, so the two cannot disagree.
fn max_path_len_for(dialect: Dialect) -> Option<usize> {
    match dialect {
        Dialect::SqlServer => Some(MAX_PATH_CHARS),
        Dialect::Sqlite | Dialect::Postgres => None,
    }
}

/// Refuse a path the `nodes` columns cannot hold.
///
/// `SafetyManager::ensure_path_fits` already rejects an over long path the caller
/// supplied, but a move or a copy re-roots each descendant under a new prefix, so a
/// destination that fits can still produce children that do not. This is the
/// backstop on the computed form, at the one place every write funnels through.
fn ensure_storable_path(dialect: Dialect, path: &str) -> Result<()> {
    let Some(limit) = max_path_len_for(dialect) else {
        return Ok(());
    };
    let len = path.chars().count();
    if len <= limit {
        return Ok(());
    }
    Err(ToolError::invalid_argument(format!(
        "path is {len} characters but this backend allows at most {limit}: '{path}' would be \
         created by re-rooting under a longer prefix, so choose a shorter destination"
    )))
}

/// The tables this store owns.
pub fn schema() -> SchemaSet {
    SchemaSet::new(
        vec![
            Table::new(
                "nodes",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("path", ColumnType::TextKey(PATH_LEN)),
                    // Indexed, so it is keyed rather than unbounded text.
                    Column::new("parent", ColumnType::TextKey(PATH_LEN)),
                    Column::required("name", ColumnType::Text),
                    Column::required("kind", ColumnType::Text),
                    Column::required("size", ColumnType::BigInt).default("0"),
                    Column::required("mode", ColumnType::BigInt),
                    Column::required("mtime", ColumnType::Double),
                    Column::required("ctime", ColumnType::Double),
                    Column::required("atime", ColumnType::Double),
                    Column::new("sha256", ColumnType::Text),
                ],
                vec!["volume_id", "path"],
            ),
            Table::new(
                "blob_refs",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("sha256", ColumnType::TextKey(SHA_LEN)),
                    Column::required("refcount", ColumnType::BigInt),
                    Column::required("size", ColumnType::BigInt),
                ],
                vec!["volume_id", "sha256"],
            ),
        ],
        vec![Index {
            name: "idx_nodes_parent",
            table: "nodes",
            // Volume first: every lookup filters on it before the parent.
            columns: vec!["volume_id", "parent"],
        }],
    )
}

/// The metadata tree of one volume.
pub struct RelationalMetaStore {
    db: Arc<dyn RelationalDb>,
    volume_id: String,
}

impl RelationalMetaStore {
    /// Apply the schema, ensure the root directory row, and return the store.
    pub async fn open(db: Arc<dyn RelationalDb>, volume_id: impl Into<String>) -> Result<Self> {
        let store = Self { db, volume_id: volume_id.into() };
        store.db.migrate(&schema()).await?;
        store.ensure_root().await?;
        Ok(store)
    }

    /// In memory SQLite store, for tests.
    pub async fn in_memory(volume_id: &str) -> Result<Self> {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open_in_memory()?);
        Self::open(db, volume_id).await
    }

    pub fn volume_id(&self) -> &str {
        &self.volume_id
    }

    /// Insert the root node unless it is already there. Idempotent, so reopening
    /// an existing volume leaves its root untouched.
    async fn ensure_root(&self) -> Result<()> {
        let now = now_unix();
        let sql = self.db.dialect().render_upsert(&Upsert::ignore(
            "nodes",
            NODE_COLS.to_vec(),
            vec!["volume_id", "path"],
        ));
        self.db
            .execute(
                &Query::new(sql)
                    .bind(&self.volume_id)
                    .bind("/")
                    .bind(None::<String>)
                    .bind("")
                    .bind("dir")
                    .bind(0i64)
                    .bind(MODE_DIR)
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .bind(None::<String>),
            )
            .await?;
        Ok(())
    }

    fn read_row(r: &RowValues) -> Result<NodeRow> {
        Ok(NodeRow {
            path: r.text(0)?,
            parent: r.opt_text(1)?,
            name: r.text(2)?,
            kind: r.text(3)?,
            size: r.i64(4)?,
            mode: r.i64(5)?,
            mtime: r.f64(6)?,
            ctime: r.f64(7)?,
            atime: r.f64(8)?,
            sha256: r.opt_text(9)?,
        })
    }

    /// A `LIKE` pattern matching every strict descendant of `path`.
    ///
    /// The prefix is escaped, so a path holding `%` or `_` matches only itself.
    /// Without this a file called `a_b` pulls in its sibling `axb`, which turned a
    /// subtree delete into a delete of unrelated rows.
    fn descendant_pattern(&self, path: &str) -> String {
        let base = self.db.dialect().escape_like_literal(path.trim_end_matches('/'));
        format!("{base}/%")
    }

    /// The upsert that writes a file row, rendered for this engine.
    fn put_node_sql(&self) -> String {
        self.db.dialect().render_upsert(&Upsert::replace(
            "nodes",
            NODE_COLS.to_vec(),
            vec!["volume_id", "path"],
        ))
    }
}

// ── transaction helpers ─────────────────────────────────────────────────────────
//
// Free functions rather than methods: they take the transaction, so they compose
// inside a `run_retrying` body without borrowing the store mutably.

async fn tx_exists(tx: &mut dyn RelationalTx, volume: &str, path: &str) -> Result<bool> {
    let row = tx
        .query_opt(
            &Query::new("SELECT 1 FROM nodes WHERE volume_id=?1 AND path=?2")
                .bind(volume)
                .bind(path),
        )
        .await?;
    Ok(row.is_some())
}

async fn tx_kind(tx: &mut dyn RelationalTx, volume: &str, path: &str) -> Result<Option<String>> {
    let row = tx
        .query_opt(
            &Query::new("SELECT kind FROM nodes WHERE volume_id=?1 AND path=?2")
                .bind(volume)
                .bind(path),
        )
        .await?;
    match row {
        Some(r) => Ok(Some(r.text(0)?)),
        None => Ok(None),
    }
}

/// Add one reference to a blob, creating the row on first use.
async fn tx_incref(
    tx: &mut dyn RelationalTx,
    dialect: crate::storage::rel::Dialect,
    volume: &str,
    sha: Option<&str>,
    size: i64,
) -> Result<()> {
    let Some(sha) = sha else { return Ok(()) };
    let sql = dialect.render_upsert(&Upsert::update(
        "blob_refs",
        vec!["volume_id", "sha256", "refcount", "size"],
        vec!["volume_id", "sha256"],
        vec![Assign::expr("refcount", "{target}.refcount + 1")],
    ));
    tx.execute(&Query::new(sql).bind(volume).bind(sha).bind(1i64).bind(size)).await?;
    Ok(())
}

/// Drop one reference. Returns true when it reached zero, so the caller GCs.
async fn tx_decref(tx: &mut dyn RelationalTx, volume: &str, sha: Option<&str>) -> Result<bool> {
    let Some(sha) = sha else { return Ok(false) };
    let row = tx
        .query_opt(
            &Query::new("SELECT refcount FROM blob_refs WHERE volume_id=?1 AND sha256=?2")
                .bind(volume)
                .bind(sha),
        )
        .await?;
    let Some(row) = row else { return Ok(false) };
    let remaining = row.i64(0)? - 1;
    if remaining <= 0 {
        tx.execute(
            &Query::new("DELETE FROM blob_refs WHERE volume_id=?1 AND sha256=?2")
                .bind(volume)
                .bind(sha),
        )
        .await?;
        return Ok(true);
    }
    tx.execute(
        &Query::new("UPDATE blob_refs SET refcount=?1 WHERE volume_id=?2 AND sha256=?3")
            .bind(remaining)
            .bind(volume)
            .bind(sha),
    )
    .await?;
    Ok(false)
}

/// Insert a directory node, tolerating a row a concurrent writer created first.
///
/// Returns false when the row was already there. A plain INSERT here was a real
/// race: SQLite serializes every writer behind one mutex, so a check then insert
/// could not interleave, but PostgreSQL and SQL Server run writers at the same
/// time and two of them creating the same parent directory raised a duplicate key.
/// Both callers need the outcome, so the conflict is reported rather than hidden:
/// [`tx_mkdirs_chain`] revalidates that the winner stored a directory, and `mkdir`
/// turns it into the `no_clobber` its contract promises.
async fn tx_insert_dir(
    tx: &mut dyn RelationalTx,
    dialect: crate::storage::rel::Dialect,
    volume: &str,
    path: &str,
) -> Result<bool> {
    let now = now_unix();
    let sql = dialect.render_upsert(&Upsert::ignore(
        "nodes",
        NODE_COLS.to_vec(),
        vec!["volume_id", "path"],
    ));
    let affected = tx
        .execute(
            &Query::new(sql)
                .bind(volume)
                .bind(path)
                .bind(PosixPath::parent_of(path))
                .bind(PosixPath::name_of(path))
                .bind("dir")
                .bind(0i64)
                .bind(MODE_DIR)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(None::<String>),
        )
        .await?;
    Ok(affected > 0)
}

/// Create every missing directory along `path`, failing if a component is a file.
async fn tx_mkdirs_chain(
    tx: &mut dyn RelationalTx,
    dialect: crate::storage::rel::Dialect,
    volume: &str,
    path: &str,
) -> Result<()> {
    let mut current = String::new();
    for part in path.trim_matches('/').split('/').filter(|p| !p.is_empty()) {
        current = format!("{current}/{part}");
        match tx_kind(tx, volume, &current).await?.as_deref() {
            None => {
                // A concurrent writer may have created this component between the
                // read and the insert, so the insert tolerates the collision and
                // the kind is confirmed afterwards: losing the race to a file must
                // still refuse, otherwise a file would gain children.
                if !tx_insert_dir(tx, dialect, volume, &current).await?
                    && tx_kind(tx, volume, &current).await?.as_deref() != Some("dir")
                {
                    return Err(ToolError::no_clobber(format!(
                        "'{current}' already exists and is not a directory"
                    )));
                }
            }
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

async fn tx_ensure_parents(
    tx: &mut dyn RelationalTx,
    dialect: crate::storage::rel::Dialect,
    volume: &str,
    path: &str,
) -> Result<()> {
    if let Some(parent) = PosixPath::parent_of(path)
        && parent != "/"
    {
        tx_mkdirs_chain(tx, dialect, volume, &parent).await?;
    }
    Ok(())
}

#[async_trait]
impl MetaBackend for RelationalMetaStore {
    async fn get(&self, path: &str) -> Result<Option<NodeRow>> {
        let row = self
            .db
            .query_opt(
                &Query::new(format!(
                    "SELECT {SELECT_COLS} FROM nodes WHERE volume_id=?1 AND path=?2"
                ))
                .bind(&self.volume_id)
                .bind(path),
            )
            .await?;
        match row {
            Some(r) => Ok(Some(Self::read_row(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_children(&self, parent: &str) -> Result<Vec<NodeRow>> {
        let rows = self
            .db
            .query(
                &Query::new(format!(
                    "SELECT {SELECT_COLS} FROM nodes \
                     WHERE volume_id=?1 AND parent=?2 ORDER BY name"
                ))
                .bind(&self.volume_id)
                .bind(parent),
            )
            .await?;
        rows.iter().map(Self::read_row).collect()
    }

    async fn subtree(&self, root: &str) -> Result<Vec<NodeRow>> {
        let rows = self
            .db
            .query(
                &Query::new(format!(
                    "SELECT {SELECT_COLS} FROM nodes \
                     WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\') \
                     ORDER BY path"
                ))
                .bind(&self.volume_id)
                .bind(root)
                .bind(self.descendant_pattern(root)),
            )
            .await?;
        rows.iter().map(Self::read_row).collect()
    }

    async fn put_file(
        &self,
        path: &str,
        sha256: Option<&str>,
        size: i64,
        mode: i64,
    ) -> Result<Option<String>> {
        let dialect = self.db.dialect();
        ensure_storable_path(dialect, path)?;
        let put_sql = self.put_node_sql();
        let volume = self.volume_id.clone();
        let path = path.to_string();
        let sha = sha256.map(str::to_string);
        // Idempotent: every value written is derived from rows read in the same
        // transaction, so a retry recomputes from the rolled back original state.
        // The captures are owned and re-cloned per attempt, so no attempt can see
        // a value the previous one produced.
        run_retrying(&*self.db, move |tx| {
            let (volume, put_sql, path, sha) =
                (volume.clone(), put_sql.clone(), path.clone(), sha.clone());
            Box::pin(async move {
                tx_ensure_parents(tx, dialect, &volume, &path).await?;
                let existing = tx
                    .query_opt(
                        &Query::new(
                            "SELECT kind, ctime, sha256 FROM nodes \
                             WHERE volume_id=?1 AND path=?2",
                        )
                        .bind(&volume)
                        .bind(&path),
                    )
                    .await?;

                let mut ctime = now_unix();
                let mut old_sha: Option<String> = None;
                if let Some(row) = existing {
                    if row.text(0)? == "dir" {
                        return Err(ToolError::invalid_argument(format!(
                            "'{path}' is a directory"
                        )));
                    }
                    ctime = row.f64(1)?;
                    old_sha = row.opt_text(2)?;
                }

                let mut gc = None;
                if old_sha.as_deref() != sha.as_deref() {
                    tx_incref(tx, dialect, &volume, sha.as_deref(), size).await?;
                    if tx_decref(tx, &volume, old_sha.as_deref()).await? {
                        gc = old_sha;
                    }
                }

                let now = now_unix();
                tx.execute(
                    &Query::new(put_sql)
                        .bind(&volume)
                        .bind(&path)
                        .bind(PosixPath::parent_of(&path))
                        .bind(PosixPath::name_of(&path))
                        .bind("file")
                        .bind(size)
                        .bind(mode)
                        .bind(now)
                        .bind(ctime)
                        .bind(now)
                        .bind(sha.clone()),
                )
                .await?;
                Ok(gc)
            })
        })
        .await
    }

    async fn delete_file(&self, path: &str) -> Result<Option<String>> {
        let volume = self.volume_id.clone();
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path) = (volume.clone(), path.clone());
            Box::pin(async move {
                let found = tx
                    .query_opt(
                        &Query::new(
                            "SELECT kind, sha256 FROM nodes WHERE volume_id=?1 AND path=?2",
                        )
                        .bind(&volume)
                        .bind(&path),
                    )
                    .await?;
                let Some(row) = found else {
                    return Err(ToolError::not_found(format!("'{path}' not found")));
                };
                if row.text(0)? == "dir" {
                    return Err(ToolError::invalid_argument(format!("'{path}' is a directory")));
                }
                let sha = row.opt_text(1)?;
                let gc = if tx_decref(tx, &volume, sha.as_deref()).await? { sha } else { None };
                tx.execute(
                    &Query::new("DELETE FROM nodes WHERE volume_id=?1 AND path=?2")
                        .bind(&volume)
                        .bind(&path),
                )
                .await?;
                Ok(gc)
            })
        })
        .await
    }

    async fn remove_subtree(&self, path: &str) -> Result<Vec<String>> {
        let pattern = self.descendant_pattern(path);
        let volume = self.volume_id.clone();
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path, pattern) = (volume.clone(), path.clone(), pattern.clone());
            // The accumulator is built inside the body, so a retry starts from an
            // empty list instead of appending to the previous attempt's result.
            Box::pin(async move {
                let rows = tx
                    .query(
                        &Query::new(
                            "SELECT sha256 FROM nodes \
                             WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')",
                        )
                        .bind(&volume)
                        .bind(&path)
                        .bind(&pattern),
                    )
                    .await?;
                let shas: Vec<Option<String>> =
                    rows.iter().map(|r| r.opt_text(0)).collect::<Result<_>>()?;

                let mut gc = Vec::new();
                for sha in &shas {
                    if sha.is_some() && tx_decref(tx, &volume, sha.as_deref()).await? {
                        gc.extend(sha.clone());
                    }
                }
                tx.execute(
                    &Query::new(
                        "DELETE FROM nodes \
                         WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\')",
                    )
                    .bind(&volume)
                    .bind(&path)
                    .bind(&pattern),
                )
                .await?;
                Ok(gc)
            })
        })
        .await
    }

    async fn mkdirs(&self, path: &str, exist_ok: bool) -> Result<()> {
        let volume = self.volume_id.clone();
        let dialect = self.db.dialect();
        ensure_storable_path(dialect, path)?;
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path) = (volume.clone(), path.clone());
            Box::pin(async move {
                if let Some(kind) = tx_kind(tx, &volume, &path).await? {
                    if kind != "dir" || !exist_ok {
                        return Err(ToolError::no_clobber(format!("'{path}' already exists")));
                    }
                    return Ok(());
                }
                tx_mkdirs_chain(tx, dialect, &volume, &path).await
            })
        })
        .await
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        let volume = self.volume_id.clone();
        let dialect = self.db.dialect();
        ensure_storable_path(dialect, path)?;
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path) = (volume.clone(), path.clone());
            Box::pin(async move {
                if let Some(parent) = PosixPath::parent_of(&path)
                    && !tx_exists(tx, &volume, &parent).await?
                {
                    return Err(ToolError::not_found(format!("'{parent}' not found")));
                }
                if tx_exists(tx, &volume, &path).await? {
                    return Err(ToolError::no_clobber(format!("'{path}' already exists")));
                }
                // The insert reports a collision instead of raising a duplicate
                // key, so a writer that lost the race gets the documented error.
                if tx_insert_dir(tx, dialect, &volume, &path).await? {
                    Ok(())
                } else {
                    Err(ToolError::no_clobber(format!("'{path}' already exists")))
                }
            })
        })
        .await
    }

    async fn rmdir(&self, path: &str) -> Result<()> {
        self.db
            .execute(
                &Query::new("DELETE FROM nodes WHERE volume_id=?1 AND path=?2 AND kind='dir'")
                    .bind(&self.volume_id)
                    .bind(path),
            )
            .await?;
        Ok(())
    }

    async fn rename(&self, src: &str, dst: &str) -> Result<()> {
        let pattern = self.descendant_pattern(src);
        // Shortest path first, so a parent is renamed before its children and no
        // intermediate state collides with a row that has not moved yet.
        let order = self.db.dialect().length_fn();
        let dialect = self.db.dialect();
        let volume = self.volume_id.clone();
        let (src, dst) = (src.to_string(), dst.to_string());
        run_retrying(&*self.db, move |tx| {
            let (volume, src, dst, pattern) =
                (volume.clone(), src.clone(), dst.clone(), pattern.clone());
            Box::pin(async move {
                if !tx_exists(tx, &volume, &src).await? {
                    return Err(ToolError::not_found(format!("'{src}' not found")));
                }
                if tx_exists(tx, &volume, &dst).await? {
                    return Err(ToolError::no_clobber(format!("'{dst}' already exists")));
                }
                tx_ensure_parents(tx, dialect, &volume, &dst).await?;

                let rows = tx
                    .query(
                        &Query::new(format!(
                            "SELECT path FROM nodes \
                             WHERE volume_id=?1 AND (path=?2 OR path LIKE ?3 ESCAPE '\\') \
                             ORDER BY {order}(path)"
                        ))
                        .bind(&volume)
                        .bind(&src)
                        .bind(&pattern),
                    )
                    .await?;
                let paths: Vec<String> = rows.iter().map(|r| r.text(0)).collect::<Result<_>>()?;

                for old in paths {
                    let new = if old == src {
                        dst.clone()
                    } else {
                        format!("{}{}", dst, &old[src.len()..])
                    };
                    // A descendant grows by however much `dst` is longer than `src`, so
                    // it can overflow the column even though `dst` itself fitted. The
                    // whole rename is one transaction, so refusing here leaves the tree
                    // untouched rather than half moved.
                    ensure_storable_path(dialect, &new)?;
                    tx.execute(
                        &Query::new(
                            "UPDATE nodes SET path=?1, parent=?2, name=?3 \
                             WHERE volume_id=?4 AND path=?5",
                        )
                        .bind(&new)
                        .bind(PosixPath::parent_of(&new))
                        .bind(PosixPath::name_of(&new))
                        .bind(&volume)
                        .bind(&old),
                    )
                    .await?;
                }
                Ok(())
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::rel::{Dialect, SqliteRelationalDb};
    use crate::storage::traits::MODE_FILE;

    async fn store() -> RelationalMetaStore {
        RelationalMetaStore::in_memory("proj").await.unwrap()
    }

    /// Only SQL Server bounds a path, and both accessors must agree, since one
    /// serves the safety guard and the other the store itself.
    #[test]
    fn the_path_ceiling_is_sqlserver_only_and_agrees_across_accessors() {
        use crate::config::backend;
        assert_eq!(max_path_len(backend::SQLSERVER), Some(MAX_PATH_CHARS));
        // The ceiling must leave room for the longest project id inside the 900 byte
        // clustered key, which is what SQL Server actually enforces.
        assert_eq!(MAX_PATH_CHARS, 418);
        // The volume budget must match the real project id cap, since the volume id IS
        // the project id: if validation ever allowed a longer one, the key would grow.
        let longest_ok = "p".repeat(MAX_VOLUME_ID_CHARS);
        assert!(crate::storage::admin::validate_project_id(&longest_ok).is_ok());
        let one_over = "p".repeat(MAX_VOLUME_ID_CHARS + 1);
        assert!(
            crate::storage::admin::validate_project_id(&one_over).is_err(),
            "a project id longer than the budgeted worst case must be impossible"
        );
        assert_eq!(max_path_len(backend::SQLITE), None);
        assert_eq!(max_path_len(backend::POSTGRES), None);

        assert_eq!(max_path_len_for(Dialect::SqlServer), max_path_len(backend::SQLSERVER));
        assert_eq!(max_path_len_for(Dialect::Sqlite), max_path_len(backend::SQLITE));
        assert_eq!(max_path_len_for(Dialect::Postgres), max_path_len(backend::POSTGRES));
    }

    /// The backstop for a computed path. A move or a copy re-roots descendants, so a
    /// destination that fits can still produce children that do not.
    #[test]
    fn the_store_refuses_a_computed_path_past_the_column_width() {
        let limit = MAX_PATH_CHARS;
        let at_limit = format!("/{}", "a".repeat(limit - 1));
        let over = format!("/{}", "a".repeat(limit));

        assert!(ensure_storable_path(Dialect::SqlServer, &at_limit).is_ok());
        let e = ensure_storable_path(Dialect::SqlServer, &over).unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains(&format!("{}", limit + 1)));
        assert!(e.message.contains("re-rooting"), "explains the cause: {}", e.message);

        // An unbounded engine takes it, so the restriction stays where it belongs.
        assert!(ensure_storable_path(Dialect::Sqlite, &over).is_ok());
        assert!(ensure_storable_path(Dialect::Postgres, &over).is_ok());
    }

    /// A rename whose destination fits but whose descendants would not must fail as a
    /// whole: the transaction is one unit, so the tree stays where it was.
    #[tokio::test]
    async fn a_rename_that_would_overflow_a_descendant_is_refused_live() {
        let Ok(dsn) = std::env::var("MCPFS_TEST_MSSQL_DSN") else {
            eprintln!("skipping the live path ceiling check: MCPFS_TEST_MSSQL_DSN is unset");
            return;
        };
        if !cfg!(feature = "sqlserver") {
            eprintln!("skipping the live path ceiling check: built without --features sqlserver");
            return;
        }
        #[cfg(feature = "sqlserver")]
        {
            let db = crate::storage::rel::SqlServerRelationalDb::connect(
                &dsn,
                crate::storage::rel::PoolSettings {
                    max_connections: 2,
                    acquire_timeout: std::time::Duration::from_secs(10),
                },
            )
            .await
            .expect("connect to the test SQL Server");
            let db: Arc<dyn RelationalDb> = Arc::new(db);
            let s = RelationalMetaStore::open(db, &"p".repeat(MAX_VOLUME_ID_CHARS)).await.unwrap();
            s.remove_subtree("/src").await.ok();
            s.remove_subtree("/dst").await.ok();

            // A path at the ceiling must round trip under the LONGEST project id, which
            // is the case the arithmetic budgets for. This is what caught the original
            // 450 character ceiling: it built a 916 byte key and SQL Server refused it.
            let deep = format!("/{}", "a".repeat(MAX_PATH_CHARS - 1));
            assert_eq!(deep.chars().count(), MAX_PATH_CHARS);
            s.put_file(&deep, Some("shalimit"), 3, crate::storage::traits::MODE_FILE)
                .await
                .expect("a path at the ceiling must be storable");
            assert!(s.get(&deep).await.unwrap().is_some(), "and readable back");
            s.remove_subtree(&deep).await.unwrap();

            // Now a short tree, moved under a prefix long enough to overflow a child.
            let child = "/src/".to_string() + &"b".repeat(40);
            s.put_file(&child, Some("shamove"), 3, crate::storage::traits::MODE_FILE)
                .await
                .unwrap();
            let long_dst = format!("/{}", "d".repeat(MAX_PATH_CHARS - 20));
            assert!(long_dst.chars().count() <= MAX_PATH_CHARS, "the destination fits");

            let e = s.rename("/src", &long_dst).await.unwrap_err();
            assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
            assert!(
                s.get(&child).await.unwrap().is_some(),
                "the refused rename must leave the tree untouched"
            );
            s.remove_subtree("/src").await.unwrap();
        }
    }

    #[tokio::test]
    async fn schema_declares_both_tables_keyed_by_volume() {
        let s = schema();
        let nodes = s.tables.iter().find(|t| t.name == "nodes").expect("nodes table");
        assert_eq!(nodes.primary_key, vec!["volume_id", "path"]);
        let refs = s.tables.iter().find(|t| t.name == "blob_refs").expect("blob_refs table");
        assert_eq!(refs.primary_key, vec!["volume_id", "sha256"]);
        // The parent index must lead with the volume, every lookup filters on it.
        assert_eq!(s.indexes[0].columns, vec!["volume_id", "parent"]);
    }

    /// The escape clause and the escaped prefix have to travel together: a
    /// pattern without `ESCAPE` treats our backslash as a literal character.
    #[tokio::test]
    async fn descendant_pattern_escapes_wildcards() {
        let s = store().await;
        assert_eq!(s.descendant_pattern("/a/b"), "/a/b/%");
        assert_eq!(s.descendant_pattern("/50%"), "/50\\%/%");
        assert_eq!(s.descendant_pattern("/a_b"), "/a\\_b/%");
        assert_eq!(s.descendant_pattern("/d/"), "/d/%", "a trailing slash is dropped");
    }

    #[tokio::test]
    async fn root_exists_after_open() {
        let s = store().await;
        let root = s.get("/").await.unwrap().expect("root node");
        assert_eq!(root.kind, "dir");
        assert_eq!(root.name, "");
        assert_eq!(root.parent, None);
        assert_eq!(root.mode, MODE_DIR);
    }

    /// Reopening must not reset the tree, which is what makes `migrate` plus
    /// `ensure_root` safe to run on every open.
    #[tokio::test]
    async fn reopening_keeps_existing_rows() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let first = RelationalMetaStore::open(db.clone(), "proj").await.unwrap();
        first.put_file("/a.txt", Some("s"), 1, MODE_FILE).await.unwrap();

        let second = RelationalMetaStore::open(db, "proj").await.unwrap();
        assert!(second.get("/a.txt").await.unwrap().is_some(), "row survived reopen");
    }

    #[tokio::test]
    async fn put_file_creates_parents_and_increfs() {
        let s = store().await;
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

    #[tokio::test]
    async fn dedup_refcount_and_gc_at_zero() {
        let s = store().await;
        s.put_file("/a.txt", Some("same"), 4, MODE_FILE).await.unwrap();
        s.put_file("/b.txt", Some("same"), 4, MODE_FILE).await.unwrap();

        let gc1 = s.delete_file("/a.txt").await.unwrap();
        assert_eq!(gc1, None, "refcount was 2, nothing to GC yet");

        let gc2 = s.delete_file("/b.txt").await.unwrap();
        assert_eq!(gc2.as_deref(), Some("same"), "refcount hit 0, GC the blob");
    }

    #[tokio::test]
    async fn overwrite_gcs_old_blob_and_keeps_ctime() {
        let s = store().await;
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
        let s = store().await;
        s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        let gc = s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        assert_eq!(gc, None);
    }

    #[tokio::test]
    async fn empty_file_has_no_blob() {
        let s = store().await;
        s.put_file("/e.txt", None, 0, MODE_FILE).await.unwrap();
        let f = s.get("/e.txt").await.unwrap().unwrap();
        assert_eq!(f.sha256, None);
        assert_eq!(f.size, 0);
        let gc = s.delete_file("/e.txt").await.unwrap();
        assert_eq!(gc, None);
    }

    #[tokio::test]
    async fn list_children_is_sorted_by_name() {
        let s = store().await;
        for p in ["/c.txt", "/a.txt", "/b.txt"] {
            s.put_file(p, Some(p), 1, MODE_FILE).await.unwrap();
        }
        let names: Vec<String> =
            s.list_children("/").await.unwrap().into_iter().map(|n| n.name).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);
    }

    #[tokio::test]
    async fn subtree_includes_root_and_descendants() {
        let s = store().await;
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
        let s = store().await;
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
        let s = store().await;
        s.put_file("/d/a.txt", Some("shared"), 1, MODE_FILE).await.unwrap();
        s.put_file("/keep.txt", Some("shared"), 1, MODE_FILE).await.unwrap();
        let gc = s.remove_subtree("/d").await.unwrap();
        assert!(gc.is_empty(), "blob still referenced by /keep.txt");
    }

    #[tokio::test]
    async fn mkdir_requires_existing_parent_and_rejects_duplicates() {
        let s = store().await;
        let e = s.mkdir("/nope/child").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);

        s.mkdir("/x").await.unwrap();
        let e = s.mkdir("/x").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn mkdirs_is_idempotent_when_exist_ok() {
        let s = store().await;
        s.mkdirs("/a/b/c", true).await.unwrap();
        s.mkdirs("/a/b/c", true).await.unwrap();
        assert!(s.get("/a/b/c").await.unwrap().unwrap().is_dir());

        let e = s.mkdirs("/a/b/c", false).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn mkdirs_refuses_to_tunnel_through_a_file() {
        let s = store().await;
        s.put_file("/f", Some("x"), 1, MODE_FILE).await.unwrap();
        let e = s.mkdirs("/f/child", true).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn rename_moves_whole_subtree() {
        let s = store().await;
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
        let s = store().await;
        s.put_file("/a.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/b.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        assert_eq!(s.rename("/nope", "/x").await.unwrap_err().code, crate::errors::code::NOT_FOUND);
        assert_eq!(
            s.rename("/a.txt", "/b.txt").await.unwrap_err().code,
            crate::errors::code::NO_CLOBBER
        );
    }

    #[tokio::test]
    async fn delete_file_errors_are_typed() {
        let s = store().await;
        assert_eq!(s.delete_file("/nope").await.unwrap_err().code, crate::errors::code::NOT_FOUND);
        s.mkdir("/d").await.unwrap();
        assert_eq!(
            s.delete_file("/d").await.unwrap_err().code,
            crate::errors::code::INVALID_ARGUMENT
        );
    }

    #[tokio::test]
    async fn put_file_over_a_directory_is_rejected() {
        let s = store().await;
        s.mkdir("/d").await.unwrap();
        let e = s.put_file("/d", Some("x"), 1, MODE_FILE).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn blob_refcount_drops_to_zero_after_all_referencing_files_deleted() {
        let s = store().await;
        let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        s.put_file("/a.txt", Some(sha), 4, MODE_FILE).await.unwrap();
        s.put_file("/b.txt", Some(sha), 4, MODE_FILE).await.unwrap();

        let gc1 = s.delete_file("/a.txt").await.unwrap();
        assert_eq!(gc1, None, "refcount was 2; blob must survive first delete");

        let gc2 = s.delete_file("/b.txt").await.unwrap();
        assert_eq!(gc2.as_deref(), Some(sha), "refcount hit 0; blob must be returned for GC");
    }

    #[tokio::test]
    async fn rmdir_only_removes_a_directory() {
        let s = store().await;
        s.mkdir("/d").await.unwrap();
        s.put_file("/f.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        s.rmdir("/d").await.unwrap();
        assert!(s.get("/d").await.unwrap().is_none());
        s.rmdir("/f.txt").await.unwrap();
        assert!(s.get("/f.txt").await.unwrap().is_some(), "a file is not a directory");
    }

    // ── the LIKE escaping regression: a wildcard in a name must not over match ──

    /// `_` is a single character wildcard. Before the prefix was escaped, the
    /// subtree of `/a_b` also matched `/axb`, so an unrelated sibling was listed.
    #[tokio::test]
    async fn subtree_does_not_match_a_sibling_through_an_underscore() {
        let s = store().await;
        s.put_file("/a_b/inside.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/axb/outside.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        let paths: Vec<String> =
            s.subtree("/a_b").await.unwrap().into_iter().map(|n| n.path).collect();
        assert!(paths.contains(&"/a_b/inside.txt".to_string()));
        assert!(
            !paths.contains(&"/axb/outside.txt".to_string()),
            "an underscore must match itself, not any character: {paths:?}"
        );
    }

    /// `%` matches any run of characters, so an unescaped `/50%` prefix matched
    /// every path starting with `/50`. Deleting it took the siblings with it.
    #[tokio::test]
    async fn remove_subtree_does_not_delete_a_sibling_through_a_percent() {
        let s = store().await;
        s.put_file("/50%/inside.txt", Some("in"), 1, MODE_FILE).await.unwrap();
        s.put_file("/50off/outside.txt", Some("out"), 1, MODE_FILE).await.unwrap();

        let gc = s.remove_subtree("/50%").await.unwrap();
        assert_eq!(gc, vec!["in"], "only the blob inside the subtree is orphaned");
        assert!(s.get("/50%/inside.txt").await.unwrap().is_none(), "the subtree is gone");
        assert!(
            s.get("/50off/outside.txt").await.unwrap().is_some(),
            "a percent must match itself, not any prefix"
        );
    }

    /// The same over match in `rename` silently moved a sibling into the target.
    #[tokio::test]
    async fn rename_does_not_drag_a_sibling_along() {
        let s = store().await;
        s.put_file("/a_b/inside.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.put_file("/axb/outside.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        s.rename("/a_b", "/moved").await.unwrap();
        assert!(s.get("/moved/inside.txt").await.unwrap().is_some());
        assert!(
            s.get("/axb/outside.txt").await.unwrap().is_some(),
            "the sibling must stay where it was"
        );
        assert!(s.get("/moved/outside.txt").await.unwrap().is_none());
    }

    /// A backslash is the escape character itself, so a path containing one has
    /// to survive a round trip through the pattern.
    #[tokio::test]
    async fn a_backslash_in_a_name_is_escaped_not_dropped() {
        let s = store().await;
        s.put_file("/a\\b/inside.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        let paths: Vec<String> =
            s.subtree("/a\\b").await.unwrap().into_iter().map(|n| n.path).collect();
        assert!(paths.contains(&"/a\\b/inside.txt".to_string()), "{paths:?}");
    }

    // ── volume isolation, newly possible to get wrong ──────────────────────────

    /// Two volumes sharing one database must not see each other's rows. Without
    /// the volume_id filter every query would return both trees.
    #[tokio::test]
    async fn volumes_sharing_a_database_are_isolated() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalMetaStore::open(db.clone(), "vol-a").await.unwrap();
        let b = RelationalMetaStore::open(db, "vol-b").await.unwrap();

        a.put_file("/only-in-a.txt", Some("sa"), 1, MODE_FILE).await.unwrap();
        assert!(a.get("/only-in-a.txt").await.unwrap().is_some());
        assert!(b.get("/only-in-a.txt").await.unwrap().is_none());

        assert_eq!(b.list_children("/").await.unwrap().len(), 0);
        assert_eq!(a.list_children("/").await.unwrap().len(), 1);
    }

    /// Refcounts are per volume, so deleting a file in one volume must not GC a
    /// blob another volume still references, and must still GC its own.
    #[tokio::test]
    async fn refcounts_do_not_leak_across_volumes() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalMetaStore::open(db.clone(), "vol-a").await.unwrap();
        let b = RelationalMetaStore::open(db, "vol-b").await.unwrap();

        a.put_file("/f.txt", Some("shared"), 1, MODE_FILE).await.unwrap();
        b.put_file("/f.txt", Some("shared"), 1, MODE_FILE).await.unwrap();

        let gc = a.delete_file("/f.txt").await.unwrap();
        assert_eq!(
            gc.as_deref(),
            Some("shared"),
            "volume a held the only reference it knows about"
        );
        assert!(b.get("/f.txt").await.unwrap().is_some(), "volume b is untouched");
        let gc_b = b.delete_file("/f.txt").await.unwrap();
        assert_eq!(gc_b.as_deref(), Some("shared"), "volume b GCs its own reference");
    }

    /// A subtree removal in one volume must not touch an identically named path
    /// in another.
    #[tokio::test]
    async fn remove_subtree_is_scoped_to_its_volume() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalMetaStore::open(db.clone(), "vol-a").await.unwrap();
        let b = RelationalMetaStore::open(db, "vol-b").await.unwrap();

        a.put_file("/d/x.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        b.put_file("/d/x.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        a.remove_subtree("/d").await.unwrap();
        assert!(a.get("/d/x.txt").await.unwrap().is_none());
        assert!(b.get("/d/x.txt").await.unwrap().is_some(), "volume b keeps its tree");
    }

    #[tokio::test]
    async fn rename_is_scoped_to_its_volume() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalMetaStore::open(db.clone(), "vol-a").await.unwrap();
        let b = RelationalMetaStore::open(db, "vol-b").await.unwrap();

        a.put_file("/src/f.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        b.put_file("/src/f.txt", Some("2"), 1, MODE_FILE).await.unwrap();

        a.rename("/src", "/dst").await.unwrap();
        assert!(a.get("/dst/f.txt").await.unwrap().is_some());
        assert!(b.get("/src/f.txt").await.unwrap().is_some(), "volume b did not move");
        assert!(b.get("/dst/f.txt").await.unwrap().is_none());
    }

    /// `rename` orders by path length so a parent moves before its children. On
    /// SQL Server that function is spelled differently, so the store asks the
    /// dialect rather than hardcoding it.
    #[test]
    fn rename_orders_by_the_dialect_length_function() {
        assert_eq!(Dialect::Sqlite.length_fn(), "length");
        assert_eq!(Dialect::SqlServer.length_fn(), "LEN");
    }

    /// A deep tree exercises the ordering: every child must land under its moved
    /// parent instead of being orphaned.
    #[tokio::test]
    async fn rename_moves_a_deep_tree_in_parent_first_order() {
        let s = store().await;
        s.put_file("/a/b/c/d/deep.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        s.rename("/a", "/z").await.unwrap();
        assert!(s.get("/z/b/c/d/deep.txt").await.unwrap().is_some());
        assert!(s.get("/a").await.unwrap().is_none());
        let moved = s.get("/z/b/c/d").await.unwrap().unwrap();
        assert_eq!(moved.parent.as_deref(), Some("/z/b/c"));
    }
}
