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
use crate::storage::rel::schema::{Column, ColumnMigration, Index, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb, RelationalTx, RowValues, run_retrying};
use crate::storage::traits::{
    ExportLinkRow, MODE_DIR, MetaBackend, NodeRow, PutFileResult, TrashEntry, TrashEntryRow,
};
use crate::util::{PosixPath, now_unix};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One "path and everything below it" selection, rendered case exact (SPEC-0050).
///
/// `LIKE` alone is not enough: SQLite folds ASCII letter case in `LIKE`, so
/// `/video/%` also selects `/Video/x`. The `LIKE` stays to narrow the rows (and
/// its wildcard escaping is the one that keeps `a_b` from matching `axb`); the
/// exact substring equality decides, on every engine (DEC-003, DEC-004).
#[derive(Clone)]
struct SubtreeMatch {
    like: String,
    prefix: String,
    prefix_len: usize,
}

impl SubtreeMatch {
    fn new(dialect: Dialect, path: &str) -> Self {
        let trimmed = path.trim_end_matches('/');
        let prefix = format!("{trimmed}/");
        let like = format!("{}/%", dialect.escape_like_literal(trimmed));
        Self { like, prefix_len: dialect.char_len(&prefix), prefix }
    }

    /// `(col LIKE ?L ESCAPE '\' AND substr(col, 1, n) = ?P)`; the length is an
    /// integer literal, never user text.
    fn clause(&self, dialect: Dialect, col: &str, like_ph: usize, prefix_ph: usize) -> String {
        let head = format!("{}({col}, 1, {})", dialect.substr_fn(), self.prefix_len);
        let exact = dialect.exact_equals(&head, &format!("?{prefix_ph}"));
        format!("({col} LIKE ?{like_ph} ESCAPE '\\' AND {exact})")
    }
}

/// One file of the disk usage breakdown (SPEC-0047). `original_path` is the
/// path a trashed file had before its delete, `None` for an active file (and
/// for a trashed one whose trash row is missing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileUsageRow {
    pub path: String,
    pub size: i64,
    pub in_trash: bool,
    pub original_path: Option<String>,
}

/// One direct child of a folder in the folder breakdown (SPEC-0049). A folder
/// carries the total size and file count of everything below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderItemRow {
    pub path: String,
    pub is_folder: bool,
    pub size: i64,
    pub file_count: i64,
}

/// A folder's own totals and its direct items, largest first, cut to the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderItems {
    pub total_size: i64,
    pub total_files: i64,
    /// Direct items before the cap, so a cut list can be told from a complete one.
    pub item_total: i64,
    pub items: Vec<FolderItemRow>,
}

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
/// A UUIDv4 string is exactly 36 characters (SPEC-0012 US-0001, FR-NEW-015).
const EXPORT_TOKEN_LEN: u32 = 36;

/// A node revision and an artifact set id are UUIDv4 strings (SPEC-0019 DEC-003).
const UUID_LEN: u32 = 36;
/// `exact` or `approximate` (SPEC-0019 FR-NEW-002).
const QUALITY_LEN: u32 = 16;
/// An RFC 3339 timestamp as `util::now_iso` writes it is 32 characters. Doubled,
/// so another offset or precision stays storable (SPEC-0023 DEC-001).
const TIMESTAMP_LEN: u32 = 64;
/// Refusal when the document changed between reading it and committing its
/// artifacts (SPEC-0019 FR-NEW-035).
const CHANGED_DURING_CONVERSION: &str = "Document changed during conversion";

const SELECT_COLS: &str = "path, parent, name, kind, size, mode, mtime, ctime, atime, sha256";

/// Every column of `nodes`, in the order the upserts bind them. `rev` is last:
/// every insert, path change and byte write binds a fresh one (DEC-003).
const NODE_COLS: [&str; 12] = [
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
    "rev",
];

/// Every column of `doc_tables`, in the order the insert binds them.
const DOC_TABLE_COLS: [&str; 10] = [
    "volume_id",
    "set_id",
    "seq",
    "caption",
    "row_count",
    "col_count",
    "quality",
    "cells",
    "line_start",
    "line_count",
];

/// Every column of `doc_images`, in the order the insert binds them.
const DOC_IMAGE_COLS: [&str; 11] = [
    "volume_id",
    "set_id",
    "seq",
    "caption",
    "page",
    "format",
    "sha256",
    "size",
    "line_start",
    "line_count",
    "caption_line",
];

/// A fresh node revision: a node insert, path change or byte write calls this.
fn new_rev() -> String {
    uuid::Uuid::new_v4().to_string()
}

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

/// Every table of [`schema`] holding rows of one volume, in schema order.
///
/// Derived rather than listed by hand: a shared database teardown that names
/// its tables forgot six of them, so a recreated project inherited the old
/// trash, export links and artifact sets (SPEC-0024 DEC-001). A table added to
/// the schema with a `volume_id` column is torn down without another edit.
pub(crate) fn volume_tables() -> Vec<&'static str> {
    schema()
        .tables
        .into_iter()
        .filter(|t| t.columns.iter().any(|c| c.name == "volume_id"))
        .map(|t| t.name)
        .collect()
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
            Table::new(
                "trash_entries",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    // Indexed as part of the primary key, so it is keyed rather
                    // than unbounded text, same budget as `nodes.path`.
                    Column::required("trash_path", ColumnType::TextKey(PATH_LEN)),
                    Column::required("original_path", ColumnType::Text),
                    Column::required("size", ColumnType::BigInt).default("0"),
                    Column::required("kind", ColumnType::Text),
                    // Indexed by `idx_trash_entries_deleted_at`, so it is keyed:
                    // SQL Server refuses an unbounded column in an index (1919).
                    // SQLite and PostgreSQL still render TEXT (SPEC-0023 DEC-001).
                    Column::required("deleted_at", ColumnType::TextKey(TIMESTAMP_LEN)),
                    Column::new("deleted_by", ColumnType::Text),
                ],
                vec!["volume_id", "trash_path"],
            ),
            // SPEC-0020 DEC-002: a project's usage, kept by the node
            // transactions so a read is O(1). Derived data: a missing row is
            // rebuilt from `nodes`, so dropping the table loses nothing.
            Table::new(
                "volume_usage",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("used_bytes", ColumnType::BigInt).default("0"),
                ],
                vec!["volume_id"],
            ),
            Table::new(
                "export_links",
                vec![
                    Column::required("token", ColumnType::TextKey(EXPORT_TOKEN_LEN)),
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("created_at", ColumnType::Text),
                    Column::required("expires_at", ColumnType::Text),
                ],
                vec!["token"],
            ),
            // SPEC-0019 data model: the pointer a reader follows to the set
            // built from the node revision it names.
            Table::new(
                "doc_artifact_current",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("path", ColumnType::TextKey(PATH_LEN)),
                    Column::required("set_id", ColumnType::TextKey(UUID_LEN)),
                    Column::required("node_rev", ColumnType::TextKey(UUID_LEN)),
                    Column::required("updated_at", ColumnType::BigInt),
                ],
                vec!["volume_id", "path"],
            ),
            Table::new(
                "doc_artifact_sets",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("set_id", ColumnType::TextKey(UUID_LEN)),
                    Column::required("path", ColumnType::TextKey(PATH_LEN)),
                    Column::required("node_rev", ColumnType::TextKey(UUID_LEN)),
                    Column::new("text_sha", ColumnType::TextKey(SHA_LEN)),
                    Column::required("text_len", ColumnType::BigInt).default("0"),
                    Column::required("report", ColumnType::Text),
                    Column::new("superseded_at", ColumnType::BigInt),
                    Column::required("created_at", ColumnType::BigInt),
                ],
                vec!["volume_id", "set_id"],
            ),
            Table::new(
                "doc_images",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("set_id", ColumnType::TextKey(UUID_LEN)),
                    Column::required("seq", ColumnType::BigInt),
                    Column::required("caption", ColumnType::Text),
                    Column::new("page", ColumnType::BigInt),
                    Column::required("format", ColumnType::TextKey(8)),
                    Column::required("sha256", ColumnType::TextKey(SHA_LEN)),
                    Column::required("size", ColumnType::BigInt),
                    Column::required("line_start", ColumnType::BigInt),
                    Column::required("line_count", ColumnType::BigInt),
                    Column::new("caption_line", ColumnType::BigInt),
                ],
                vec!["volume_id", "set_id", "seq"],
            ),
            // `row_count`/`col_count` rather than the design's `rows`/`cols`:
            // `ROWS` is reserved in SQL Server's `OFFSET ... ROWS` grammar.
            Table::new(
                "doc_tables",
                vec![
                    Column::required("volume_id", ColumnType::TextKey(VOLUME_ID_LEN)),
                    Column::required("set_id", ColumnType::TextKey(UUID_LEN)),
                    Column::required("seq", ColumnType::BigInt),
                    Column::required("caption", ColumnType::Text),
                    Column::required("row_count", ColumnType::BigInt),
                    Column::required("col_count", ColumnType::BigInt),
                    Column::required("quality", ColumnType::TextKey(QUALITY_LEN)),
                    Column::required("cells", ColumnType::Text),
                    Column::required("line_start", ColumnType::BigInt),
                    Column::required("line_count", ColumnType::BigInt),
                ],
                vec!["volume_id", "set_id", "seq"],
            ),
        ],
        vec![
            Index {
                name: "idx_nodes_parent",
                table: "nodes",
                // Volume first: every lookup filters on it before the parent.
                columns: vec!["volume_id", "parent"],
            },
            Index {
                name: "idx_trash_entries_deleted_at",
                table: "trash_entries",
                // Supports `fs.trash_list`'s ordering (US-0004), scoped by volume.
                columns: vec!["volume_id", "deleted_at"],
            },
            Index {
                name: "idx_doc_artifact_sets_path",
                table: "doc_artifact_sets",
                columns: vec!["volume_id", "path"],
            },
        ],
    )
    // SPEC-0019 DEC-003: the validity token of a node. Existing rows keep NULL
    // until a conversion stamps them.
    .column_migration(ColumnMigration {
        table: "nodes",
        column: "rev",
        ty: ColumnType::TextKey(UUID_LEN),
        not_null: false,
        default: "NULL",
    })
}

/// One table of an artifact set about to be committed (SPEC-0019 data model).
pub(crate) struct NewDocTable {
    /// The n of `table-n`.
    pub seq: i64,
    pub caption: String,
    pub rows: i64,
    pub cols: i64,
    pub quality: String,
    /// JSON array of arrays of strings.
    pub cells: String,
    /// 0 based first line of the table in the conversion text.
    pub line_start: i64,
    pub line_count: i64,
}

/// One image of an artifact set about to be committed; its picture blob is
/// already stored under `sha256` (SPEC-0019 DEC-004).
pub(crate) struct NewDocImage {
    /// The n of `image-n`.
    pub seq: i64,
    pub caption: String,
    /// `None` when the document has no pages (FR-NEW-001).
    pub page: Option<i64>,
    pub format: String,
    pub sha256: String,
    pub size: i64,
    /// 0 based line of the picture reference in the conversion text.
    pub line_start: i64,
    pub line_count: i64,
}

/// A listed image: what `fs.list_images` reports.
pub(crate) struct DocImageRow {
    pub seq: i64,
    pub caption: String,
    pub page: Option<i64>,
}

/// A stored image: its listed fields plus where its picture lives.
pub(crate) struct DocImage {
    pub caption: String,
    pub page: Option<i64>,
    pub format: String,
    pub sha256: String,
}

/// An image as a full document view places it.
pub(crate) struct ViewImageRow {
    pub seq: i64,
    pub caption: String,
    /// 0 based line of the picture reference in the conversion text.
    pub line: i64,
}

/// A table as a full document view renders it.
pub(crate) struct ViewTableRow {
    pub seq: i64,
    pub caption: String,
    /// JSON array of arrays of strings.
    pub cells: String,
    pub line_start: i64,
    pub line_count: i64,
}

/// A conversion's whole artifact set, committed at once.
pub(crate) struct NewArtifactSet {
    pub set_id: String,
    pub path: String,
    pub node_rev: String,
    pub text_sha: Option<String>,
    pub text_len: i64,
    pub report: String,
    pub images: Vec<NewDocImage>,
    pub tables: Vec<NewDocTable>,
}

/// A listed table: what `fs.list_tables` reports.
pub(crate) struct DocTableRow {
    pub seq: i64,
    pub caption: String,
    pub rows: i64,
    pub cols: i64,
    pub quality: String,
}

/// The metadata tree of one volume.
pub struct RelationalMetaStore {
    db: Arc<dyn RelationalDb>,
    volume_id: String,
    /// Serializes each document's sibling write and set commit (DEC-008).
    artifact_locks: PathLocks,
}

/// One async lock per path, so the conversion that commits last also wrote
/// the last sibling (SPEC-0019 DEC-008, FR-NEW-010). The store lives in the
/// cached volume client, so every request of the process shares these locks.
#[derive(Default)]
struct PathLocks {
    /// The std mutex only guards the map and is never held across an await;
    /// the per path tokio mutex is what a conversion waits on.
    map: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl PathLocks {
    /// Wait for `path`'s lock. Entries nobody holds or awaits are dropped on
    /// the way, so the map only holds paths being converted right now.
    async fn lock(&self, path: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            map.retain(|_, l| Arc::strong_count(l) > 1);
            map.entry(path.to_string()).or_default().clone()
        };
        lock.lock_owned().await
    }

    /// How many conversions hold or await `path`'s lock.
    #[cfg(test)]
    fn contenders(&self, path: &str) -> usize {
        let map = self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(path).map_or(0, |l| Arc::strong_count(l) - 1)
    }
}

impl RelationalMetaStore {
    /// Apply the schema, ensure the root directory row, and return the store.
    pub async fn open(db: Arc<dyn RelationalDb>, volume_id: impl Into<String>) -> Result<Self> {
        let store = Self { db, volume_id: volume_id.into(), artifact_locks: PathLocks::default() };
        store.db.migrate(&schema()).await?;
        store.ensure_root().await?;
        // Backfill before this process writes anything, so no write of ours
        // can land between the SUM and the insert and be counted twice or lost.
        store.used_bytes().await?;
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
                    .bind(None::<String>)
                    .bind(new_rev()),
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

    /// Page through `trash_entries` for this volume, newest delete first,
    /// optionally restricted to `path_prefix` and its descendants, alongside the
    /// unpaged total count matching the same filter (SPEC-0011 US-0004,
    /// FR-NEW-007). Two queries in one transaction: `list_trash_entries` alone
    /// cannot report `total` when `limit` cuts the result short.
    pub(crate) async fn list_trash_entries_page(
        &self,
        path_prefix: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<TrashEntryRow>, i64)> {
        let mut tx = self.db.begin().await?;
        let rows =
            list_trash_entries(&mut *tx, &self.volume_id, path_prefix, limit, offset).await?;
        let total = count_trash_entries(&mut *tx, &self.volume_id, path_prefix).await?;
        tx.commit().await?;
        Ok((rows, total))
    }

    /// The largest files of the volume, largest first, ties by path in
    /// character code order, plus the count of every file (SPEC-0047
    /// FR-NEW-002, 004, 005).
    ///
    /// SQL finds the size of the `cap`th largest file, then every file at least
    /// that big is fetched and ordered here: the text order of `path` differs
    /// per engine (PostgreSQL follows the database collation, SQL Server
    /// compares UTF-16 units), but UTF-8 byte order is code point order on all
    /// of them (DEC-003). Time O(n log k) for n files and cap k, space O(k +
    /// ties at the cutoff). A file under `trash_dir` is `in_trash`; a soft
    /// delete keeps a trashed folder's files below one flat entry
    /// (`/<trash_dir>/<stamp>__<flat>`), so the original path is that entry's
    /// row plus the rest of the path (DEC-008), not a row of the file itself.
    pub(crate) async fn largest_files(
        &self,
        cap: usize,
        trash_dir: &str,
    ) -> Result<(Vec<FileUsageRow>, i64)> {
        let mut tx = self.db.begin().await?;
        let volume = &self.volume_id;
        let count = tx
            .query_opt(
                &Query::new("SELECT COUNT(*) FROM nodes WHERE volume_id=?1 AND kind='file'")
                    .bind(volume),
            )
            .await?
            .map_or(Ok(0), |r| r.i64(0))?;
        let cutoff = if count > cap as i64 {
            let page = tx.dialect().render_limit_offset(1, cap as i64 - 1);
            tx.query_opt(
                &Query::new(format!(
                    "SELECT size FROM nodes WHERE volume_id=?1 AND kind='file' \
                     ORDER BY size DESC {page}"
                ))
                .bind(volume),
            )
            .await?
            .map(|r| r.i64(0))
            .transpose()?
        } else {
            None
        };
        let rows = match cutoff {
            Some(size) => {
                tx.query(
                    &Query::new(
                        "SELECT path, size FROM nodes \
                         WHERE volume_id=?1 AND kind='file' AND size>=?2",
                    )
                    .bind(volume)
                    .bind(size),
                )
                .await?
            }
            None => {
                tx.query(
                    &Query::new("SELECT path, size FROM nodes WHERE volume_id=?1 AND kind='file'")
                        .bind(volume),
                )
                .await?
            }
        };
        let mut files = rows
            .iter()
            .map(|r| {
                Ok(FileUsageRow {
                    path: r.text(0)?,
                    size: r.i64(1)?,
                    in_trash: false,
                    original_path: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        files.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        files.truncate(cap);
        // One lookup per distinct trash entry, inside the same transaction.
        let prefix = format!("/{}/", trash_dir.trim_matches('/'));
        let mut originals: HashMap<String, Option<String>> = HashMap::new();
        for file in &mut files {
            let Some(rest) = file.path.strip_prefix(&prefix) else { continue };
            let name = rest.split('/').next().unwrap_or(rest);
            let suffix = rest[name.len()..].to_string();
            let top = format!("{prefix}{name}");
            if !originals.contains_key(&top) {
                let row = get_trash_entry(&mut *tx, volume, &top).await?;
                originals.insert(top.clone(), row.map(|r| r.original_path));
            }
            file.in_trash = true;
            file.original_path = originals[&top].as_ref().map(|o| format!("{o}{suffix}"));
        }
        tx.commit().await?;
        Ok((files, count))
    }

    /// The direct items of `folder` (`/` or an absolute path without a trailing
    /// slash), each folder rolled up to everything below it (SPEC-0049
    /// FR-NEW-002, 004, 007).
    ///
    /// One query reads every file row below the folder, a second the direct
    /// folder rows (so a folder holding no file is listed with 0 bytes). The
    /// files are bucketed by the first path component after the folder, which
    /// matches whole components: `/doc` never absorbs `/docs` (DEC-004). Ties
    /// are ordered by path as Rust strings, engine independent (DEC-005).
    /// Time O(n + k log k) for n files below and k direct items, space O(n) for
    /// the rows read plus O(k).
    pub(crate) async fn folder_items(&self, folder: &str, cap: usize) -> Result<FolderItems> {
        let mut tx = self.db.begin().await?;
        let volume = &self.volume_id;
        let files = tx
            .query(
                &Query::new(
                    "SELECT path, size FROM nodes \
                     WHERE volume_id=?1 AND kind='file' AND path LIKE ?2 ESCAPE '\\'",
                )
                .bind(volume)
                .bind(self.descendant_pattern(folder)),
            )
            .await?;
        let dirs = tx
            .query(
                &Query::new(
                    "SELECT path FROM nodes WHERE volume_id=?1 AND kind='dir' AND parent=?2",
                )
                .bind(volume)
                .bind(folder),
            )
            .await?;
        tx.commit().await?;

        let prefix = format!("{}/", folder.trim_end_matches('/'));
        let mut buckets: HashMap<String, FolderItemRow> = HashMap::new();
        let (mut total_size, mut total_files) = (0i64, 0i64);
        for row in &files {
            let (path, size) = (row.text(0)?, row.i64(1)?);
            // SQLite's LIKE ignores ASCII case, so `/Video/x` comes back for
            // `/video`; the exact prefix decides, before anything is counted.
            let Some(rest) = path.strip_prefix(&prefix) else { continue };
            total_size += size;
            total_files += 1;
            let (name, is_folder) = match rest.split_once('/') {
                Some((first, _)) => (first, true),
                None => (rest, false),
            };
            let item = buckets.entry(format!("{prefix}{name}")).or_insert_with_key(|key| {
                FolderItemRow { path: key.clone(), is_folder, size: 0, file_count: 0 }
            });
            item.size += size;
            item.file_count += 1;
        }
        for row in &dirs {
            let path = row.text(0)?;
            buckets.entry(path.clone()).or_insert(FolderItemRow {
                path,
                is_folder: true,
                size: 0,
                file_count: 0,
            });
        }
        let mut items: Vec<FolderItemRow> = buckets.into_values().collect();
        items.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        let item_total = items.len() as i64;
        items.truncate(cap);
        Ok(FolderItems { total_size, total_files, item_total, items })
    }

    /// Insert a `trash_entries` row for `trash_path` unless one already exists
    /// (SPEC-0011 US-0004, FR-NEW-011): the lazy backfill of a legacy trashed
    /// node that predates `trash_entries`. Idempotent by construction, the
    /// get-then-insert runs inside one transaction so two concurrent backfills
    /// of the same node cannot both insert.
    pub(crate) async fn backfill_trash_entry(
        &self,
        trash_path: &str,
        original_path: &str,
        size: i64,
        kind: &str,
        deleted_at: &str,
    ) -> Result<()> {
        let mut tx = self.db.begin().await?;
        if get_trash_entry(&mut *tx, &self.volume_id, trash_path).await?.is_none() {
            insert_trash_entry(
                &mut *tx,
                &self.volume_id,
                trash_path,
                original_path,
                size,
                kind,
                deleted_at,
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record a `trash_entries` row on its own, outside any rename. Fixture helper
    /// only (SPEC-0044 DR-004): production writes the row inside the rename's
    /// transaction through [`MetaBackend::rename_into_trash`], and keeping this
    /// compiled for tests alone stops the two step write coming back.
    #[cfg(test)]
    pub(crate) async fn record_trash_entry(
        &self,
        trash_path: &str,
        original_path: &str,
        size: i64,
        kind: &str,
        deleted_at: &str,
        deleted_by: Option<&str>,
    ) -> Result<()> {
        let volume = self.volume_id.clone();
        let (trash_path, original_path, kind, deleted_at, deleted_by) = (
            trash_path.to_string(),
            original_path.to_string(),
            kind.to_string(),
            deleted_at.to_string(),
            deleted_by.map(str::to_string),
        );
        run_retrying(&*self.db, move |tx| {
            let (volume, trash_path, original_path, kind, deleted_at, deleted_by) = (
                volume.clone(),
                trash_path.clone(),
                original_path.clone(),
                kind.clone(),
                deleted_at.clone(),
                deleted_by.clone(),
            );
            Box::pin(async move {
                insert_trash_entry(
                    tx,
                    &volume,
                    &trash_path,
                    &original_path,
                    size,
                    &kind,
                    &deleted_at,
                    deleted_by.as_deref(),
                )
                .await
            })
        })
        .await
    }

    /// Look up a single `trash_entries` row for `fs.trash_restore` (SPEC-0011
    /// US-0005, FR-NEW-009): production counterpart to
    /// [`Self::get_trash_entry_for_test`].
    pub(crate) async fn get_trash_entry(&self, trash_path: &str) -> Result<Option<TrashEntryRow>> {
        let mut tx = self.db.begin().await?;
        let row = get_trash_entry(&mut *tx, &self.volume_id, trash_path).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// The revision of the node at `path`, stamping a fresh one first when the
    /// row predates the `rev` column (SPEC-0019 DEC-003). `None` when no node
    /// is there. Idempotent: the stamp only fills a NULL, so a retry or a
    /// concurrent caller reads back the one value that won.
    pub(crate) async fn ensure_node_rev(&self, path: &str) -> Result<Option<String>> {
        let volume = self.volume_id.clone();
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path) = (volume.clone(), path.clone());
            Box::pin(async move {
                tx.execute(
                    &Query::new(
                        "UPDATE nodes SET rev=?1 WHERE volume_id=?2 AND path=?3 AND rev IS NULL",
                    )
                    .bind(new_rev())
                    .bind(&volume)
                    .bind(&path),
                )
                .await?;
                tx_node_rev(tx, &volume, &path).await
            })
        })
        .await
    }

    /// Whether anything in this volume, a file node or an artifact set,
    /// holds a reference to `sha`. `tx_decref` drops the row at zero, so the
    /// row's presence is the answer (SPEC-0033 DEC-002).
    /// Every hash this volume holds a reference to, in one query, so the
    /// orphan sweep diffs a whole listing in memory instead of querying once
    /// per listed object (SPEC-0034 DEC-001).
    pub(crate) async fn referenced_hashes(&self) -> Result<HashSet<String>> {
        let rows = self
            .db
            .query(
                &Query::new("SELECT sha256 FROM blob_refs WHERE volume_id=?1")
                    .bind(&self.volume_id),
            )
            .await?;
        rows.iter().map(|r| r.text(0)).collect()
    }

    pub(crate) async fn blob_is_referenced(&self, sha: &str) -> Result<bool> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT refcount FROM blob_refs WHERE volume_id=?1 AND sha256=?2")
                    .bind(&self.volume_id)
                    .bind(sha),
            )
            .await?;
        Ok(row.is_some())
    }

    /// Commit a conversion's artifact set and point `path` at it, in one
    /// transaction that first checks the node still carries `set.node_rev`
    /// (SPEC-0019 DEC-008). The set it replaces is marked superseded, never
    /// deleted here, so a reader that already resolved it keeps reading whole
    /// rows. The conversion text blob and every picture blob gain their
    /// reference here, one per image row (DEC-004), so a set deletion drops
    /// exactly what this commit added.
    pub(crate) async fn commit_artifact_set(&self, set: &NewArtifactSet) -> Result<()> {
        let dialect = self.db.dialect();
        let volume = self.volume_id.as_str();
        let now = now_unix() as i64;
        let mut tx = self.db.begin().await?;
        if tx_node_rev(&mut *tx, volume, &set.path).await?.as_deref() != Some(set.node_rev.as_str())
        {
            return Err(changed_during_conversion(&set.path));
        }
        tx.execute(
            &Query::new(
                "UPDATE doc_artifact_sets SET superseded_at=?1 \
                 WHERE volume_id=?2 AND path=?3 AND superseded_at IS NULL",
            )
            .bind(now)
            .bind(volume)
            .bind(&set.path),
        )
        .await?;
        tx.execute(
            &Query::new(
                "INSERT INTO doc_artifact_sets (volume_id, set_id, path, node_rev, text_sha, \
                 text_len, report, superseded_at, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(volume)
            .bind(&set.set_id)
            .bind(&set.path)
            .bind(&set.node_rev)
            .bind(set.text_sha.clone())
            .bind(set.text_len)
            .bind(&set.report)
            .bind(None::<i64>)
            .bind(now),
        )
        .await?;
        let insert_table = format!(
            "INSERT INTO doc_tables ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            DOC_TABLE_COLS.join(", ")
        );
        for t in &set.tables {
            tx.execute(
                &Query::new(insert_table.clone())
                    .bind(volume)
                    .bind(&set.set_id)
                    .bind(t.seq)
                    .bind(&t.caption)
                    .bind(t.rows)
                    .bind(t.cols)
                    .bind(&t.quality)
                    .bind(&t.cells)
                    .bind(t.line_start)
                    .bind(t.line_count),
            )
            .await?;
        }
        let insert_image = format!(
            "INSERT INTO doc_images ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            DOC_IMAGE_COLS.join(", ")
        );
        for i in &set.images {
            tx.execute(
                &Query::new(insert_image.clone())
                    .bind(volume)
                    .bind(&set.set_id)
                    .bind(i.seq)
                    .bind(&i.caption)
                    .bind(i.page)
                    .bind(&i.format)
                    .bind(&i.sha256)
                    .bind(i.size)
                    .bind(i.line_start)
                    .bind(i.line_count)
                    .bind(None::<i64>),
            )
            .await?;
            tx_incref(&mut *tx, dialect, volume, Some(&i.sha256), i.size).await?;
        }
        tx_incref(&mut *tx, dialect, volume, set.text_sha.as_deref(), set.text_len).await?;
        let pointer = dialect.render_upsert(&Upsert::replace(
            "doc_artifact_current",
            vec!["volume_id", "path", "set_id", "node_rev", "updated_at"],
            vec!["volume_id", "path"],
        ));
        tx.execute(
            &Query::new(pointer)
                .bind(volume)
                .bind(&set.path)
                .bind(&set.set_id)
                .bind(&set.node_rev)
                .bind(now),
        )
        .await?;
        tx.commit().await
    }

    /// Refuse when `path` no longer carries `rev`, a missing node included
    /// (FR-NEW-035). Run under the artifact lock before a conversion charges
    /// or writes anything, so an overtaken conversion leaves nothing behind.
    pub(crate) async fn ensure_rev_unchanged(&self, path: &str, rev: &str) -> Result<()> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT rev FROM nodes WHERE volume_id=?1 AND path=?2")
                    .bind(&self.volume_id)
                    .bind(path),
            )
            .await?;
        let current = row.map(|r| r.opt_text(0)).transpose()?.flatten();
        if current.as_deref() == Some(rev) { Ok(()) } else { Err(changed_during_conversion(path)) }
    }

    /// Deletes every stale artifact set in this volume (SPEC-0019 DEC-011): one
    /// superseded for more than `grace_seconds` relative to `now`, or one whose
    /// `node_rev` no longer matches the node at its path (moved, overwritten
    /// outside a conversion, or the node is gone). The current set of an
    /// unchanged document never matches either arm, so it is never touched.
    ///
    /// Deletes the set's `doc_images`/`doc_tables` rows and, when the stale set
    /// is still the current pointer, the `doc_artifact_current` row too (a
    /// no-op `DELETE` otherwise, since a superseded set is never the pointer).
    /// Decrefs every image blob and the text blob, returning the shas that
    /// reached zero so the caller GCs them, the same `PutFileResult.gc` style
    /// every other mutation here already uses. One transaction, so a crash
    /// mid-sweep leaves either the old state or the fully swept state, never a
    /// half deleted set; `run_retrying` re-selects the stale rows afresh on
    /// retry, so re-running this is idempotent.
    pub(crate) async fn sweep_stale_artifact_sets(
        &self,
        now: i64,
        grace_seconds: i64,
    ) -> Result<(usize, Vec<String>)> {
        let volume = self.volume_id.clone();
        run_retrying(&*self.db, move |tx| {
            let volume = volume.clone();
            Box::pin(async move {
                let stale = tx
                    .query(
                        &Query::new(
                            "SELECT s.set_id, s.path, s.text_sha FROM doc_artifact_sets s \
                             LEFT JOIN nodes n ON n.volume_id = s.volume_id \
                             AND n.path = s.path AND n.rev = s.node_rev \
                             WHERE s.volume_id = ?1 AND ( \
                             (s.superseded_at IS NOT NULL AND ?2 - s.superseded_at > ?3) \
                             OR (s.superseded_at IS NULL AND n.path IS NULL))",
                        )
                        .bind(&volume)
                        .bind(now)
                        .bind(grace_seconds),
                    )
                    .await?;
                let mut gc = Vec::new();
                let mut sets_swept = 0usize;
                for row in stale {
                    let set_id = row.text(0)?;
                    let path = row.text(1)?;
                    let text_sha = row.opt_text(2)?;
                    sets_swept += 1;
                    let images = tx
                        .query(
                            &Query::new(
                                "SELECT sha256 FROM doc_images WHERE volume_id=?1 AND set_id=?2",
                            )
                            .bind(&volume)
                            .bind(&set_id),
                        )
                        .await?;
                    for img in images {
                        let sha = img.text(0)?;
                        if tx_decref(tx, &volume, Some(&sha)).await? {
                            gc.push(sha);
                        }
                    }
                    if let Some(sha) = text_sha
                        && tx_decref(tx, &volume, Some(&sha)).await?
                    {
                        gc.push(sha);
                    }
                    tx.execute(
                        &Query::new("DELETE FROM doc_images WHERE volume_id=?1 AND set_id=?2")
                            .bind(&volume)
                            .bind(&set_id),
                    )
                    .await?;
                    tx.execute(
                        &Query::new("DELETE FROM doc_tables WHERE volume_id=?1 AND set_id=?2")
                            .bind(&volume)
                            .bind(&set_id),
                    )
                    .await?;
                    tx.execute(
                        &Query::new(
                            "DELETE FROM doc_artifact_sets WHERE volume_id=?1 AND set_id=?2",
                        )
                        .bind(&volume)
                        .bind(&set_id),
                    )
                    .await?;
                    tx.execute(
                        &Query::new(
                            "DELETE FROM doc_artifact_current WHERE volume_id=?1 \
                             AND path=?2 AND set_id=?3",
                        )
                        .bind(&volume)
                        .bind(&path)
                        .bind(&set_id),
                    )
                    .await?;
                }
                Ok((sets_swept, gc))
            })
        })
        .await
    }

    /// Hold `path`'s artifact lock across a sibling write and the set commit
    /// that goes with it, so overlapping conversions of one document commit in
    /// the order they wrote their siblings (DEC-008).
    pub(crate) async fn lock_artifacts(&self, path: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.artifact_locks.lock(path).await
    }

    /// How many conversions hold or await `path`'s artifact lock.
    #[cfg(test)]
    pub(crate) fn artifact_lock_contenders(&self, path: &str) -> usize {
        self.artifact_locks.contenders(path)
    }

    /// The id of the set `path` currently points at, only while the node still
    /// carries the revision that set was built from (FR-NEW-023): one query, so
    /// the rev check and the pointer read cannot disagree.
    pub(crate) async fn current_artifact_set(&self, path: &str) -> Result<Option<String>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT c.set_id FROM doc_artifact_current c \
                     JOIN nodes n ON n.volume_id=c.volume_id AND n.path=c.path \
                     WHERE c.volume_id=?1 AND c.path=?2 AND n.rev=c.node_rev",
                )
                .bind(&self.volume_id)
                .bind(path),
            )
            .await?;
        row.map(|r| r.text(0)).transpose()
    }

    /// One page of a set's tables, in reading order.
    pub(crate) async fn list_artifact_tables(
        &self,
        set_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DocTableRow>> {
        let page = self.db.dialect().render_limit_offset(limit, offset);
        let rows = self
            .db
            .query(
                &Query::new(format!(
                    "SELECT seq, caption, row_count, col_count, quality FROM doc_tables \
                     WHERE volume_id=?1 AND set_id=?2 ORDER BY seq {page}"
                ))
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok(DocTableRow {
                    seq: r.i64(0)?,
                    caption: r.text(1)?,
                    rows: r.i64(2)?,
                    cols: r.i64(3)?,
                    quality: r.text(4)?,
                })
            })
            .collect()
    }

    /// One page of a set's images, in reading order.
    pub(crate) async fn list_artifact_images(
        &self,
        set_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DocImageRow>> {
        let page = self.db.dialect().render_limit_offset(limit, offset);
        let rows = self
            .db
            .query(
                &Query::new(format!(
                    "SELECT seq, caption, page FROM doc_images \
                     WHERE volume_id=?1 AND set_id=?2 ORDER BY seq {page}"
                ))
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        rows.iter()
            .map(|r| Ok(DocImageRow { seq: r.i64(0)?, caption: r.text(1)?, page: r.opt_i64(2)? }))
            .collect()
    }

    /// Image `seq` of a set, `None` when the set has no such image (a gap or
    /// past the end).
    pub(crate) async fn artifact_image(&self, set_id: &str, seq: i64) -> Result<Option<DocImage>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT caption, page, format, sha256 FROM doc_images \
                     WHERE volume_id=?1 AND set_id=?2 AND seq=?3",
                )
                .bind(&self.volume_id)
                .bind(set_id)
                .bind(seq),
            )
            .await?;
        row.map(|r| {
            Ok(DocImage {
                caption: r.text(0)?,
                page: r.opt_i64(1)?,
                format: r.text(2)?,
                sha256: r.text(3)?,
            })
        })
        .transpose()
    }

    /// The JSON cells of table `seq` in a set, `None` when the set has no such
    /// table (a gap or past the end).
    pub(crate) async fn artifact_table_cells(
        &self,
        set_id: &str,
        seq: i64,
    ) -> Result<Option<String>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT cells FROM doc_tables WHERE volume_id=?1 AND set_id=?2 AND seq=?3",
                )
                .bind(&self.volume_id)
                .bind(set_id)
                .bind(seq),
            )
            .await?;
        row.map(|r| r.text(0)).transpose()
    }

    /// The sha256 of a set's conversion text blob, `None` when the text was
    /// empty (no blob is stored for it).
    pub(crate) async fn artifact_text_sha(&self, set_id: &str) -> Result<Option<String>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT text_sha FROM doc_artifact_sets WHERE volume_id=?1 AND set_id=?2",
                )
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        Ok(row.map(|r| r.opt_text(0)).transpose()?.flatten())
    }

    /// Every image of a set with its position in the conversion text, in
    /// reading order: what a full document view places (FR-NEW-007).
    pub(crate) async fn artifact_view_images(&self, set_id: &str) -> Result<Vec<ViewImageRow>> {
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT seq, caption, line_start FROM doc_images \
                     WHERE volume_id=?1 AND set_id=?2 ORDER BY seq",
                )
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        rows.iter()
            .map(|r| Ok(ViewImageRow { seq: r.i64(0)?, caption: r.text(1)?, line: r.i64(2)? }))
            .collect()
    }

    /// Every table of a set with its cells and position in the conversion
    /// text, in reading order (FR-NEW-007).
    pub(crate) async fn artifact_view_tables(&self, set_id: &str) -> Result<Vec<ViewTableRow>> {
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT seq, caption, cells, line_start, line_count FROM doc_tables \
                     WHERE volume_id=?1 AND set_id=?2 ORDER BY seq",
                )
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok(ViewTableRow {
                    seq: r.i64(0)?,
                    caption: r.text(1)?,
                    cells: r.text(2)?,
                    line_start: r.i64(3)?,
                    line_count: r.i64(4)?,
                })
            })
            .collect()
    }

    /// Delete a single `trash_entries` row once `fs.trash_restore` has renamed
    /// the node back (SPEC-0011 US-0005, FR-NEW-009): production counterpart to
    /// [`Self::delete_trash_entry_for_test`].
    pub(crate) async fn delete_trash_entry(&self, trash_path: &str) -> Result<()> {
        let mut tx = self.db.begin().await?;
        delete_trash_entry(&mut *tx, &self.volume_id, trash_path).await?;
        tx.commit().await?;
        Ok(())
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

/// The FR-NEW-035 refusal, in the invalid argument category.
fn changed_during_conversion(path: &str) -> ToolError {
    ToolError::invalid_argument(format!("{CHANGED_DURING_CONVERSION}: {path}"))
}

/// The `rev` of a node, `None` when there is no node or it was never stamped.
async fn tx_node_rev(
    tx: &mut dyn RelationalTx,
    volume: &str,
    path: &str,
) -> Result<Option<String>> {
    let row = tx
        .query_opt(
            &Query::new("SELECT rev FROM nodes WHERE volume_id=?1 AND path=?2")
                .bind(volume)
                .bind(path),
        )
        .await?;
    match row {
        Some(r) => r.opt_text(0),
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

/// Move a volume's usage by `delta` bytes inside the node transaction that
/// caused it (SPEC-0020 DEC-002). A volume with no row yet is left alone: the
/// backfill reads `nodes`, which already holds this transaction's change.
async fn tx_add_usage(tx: &mut dyn RelationalTx, volume: &str, delta: i64) -> Result<()> {
    if delta == 0 {
        return Ok(());
    }
    tx.execute(
        &Query::new("UPDATE volume_usage SET used_bytes=used_bytes+?1 WHERE volume_id=?2")
            .bind(delta)
            .bind(volume),
    )
    .await?;
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
                .bind(None::<String>)
                .bind(new_rev()),
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

/// Every column of `trash_entries`, in the order the insert binds them.
const TRASH_COLS: [&str; 7] =
    ["volume_id", "trash_path", "original_path", "size", "kind", "deleted_at", "deleted_by"];

// The functions below are pure infrastructure (SPEC-0011 US-0001): no caller
// exists yet, US-0002/US-0003 are the first ones. `allow(dead_code)` is
// deliberate rather than a leftover, since the lint cannot see a future
// story's call site.
#[allow(dead_code)]
fn read_trash_row(r: &RowValues) -> Result<TrashEntryRow> {
    Ok(TrashEntryRow {
        volume_id: r.text(0)?,
        trash_path: r.text(1)?,
        original_path: r.text(2)?,
        size: r.i64(3)?,
        kind: r.text(4)?,
        deleted_at: r.text(5)?,
        deleted_by: r.opt_text(6)?,
    })
}

/// A `LIKE` pattern matching `prefix` itself and every strict descendant of it,
/// for [`list_trash_entries`]'s `path_prefix` filter. A free function rather
/// than a method on [`RelationalMetaStore`] (the trash CRUD functions take
/// only the caller's open transaction, not the store), but it goes through
/// the same [`Dialect::escape_like_literal`] as every other `LIKE` in this
/// codebase, never a hand built prefix.
#[allow(dead_code)]
fn trash_path_prefix_pattern(dialect: Dialect, prefix: &str) -> String {
    let base = dialect.escape_like_literal(prefix.trim_end_matches('/'));
    format!("{base}/%")
}

/// Move `src` (and its subtree) to `dst` inside the caller's transaction. Shared
/// by `rename` and `rename_into_trash` so the move logic exists once (SPEC-0044
/// DR-001). `sm` is the case exact selection of `src` and what lies below it (SPEC-0050).
async fn tx_rename(
    tx: &mut dyn RelationalTx,
    dialect: Dialect,
    volume: &str,
    src: &str,
    dst: &str,
    sm: &SubtreeMatch,
) -> Result<()> {
    // Shortest path first, so a parent is renamed before its children and no
    // intermediate state collides with a row that has not moved yet.
    let order = dialect.length_fn();
    if !tx_exists(tx, volume, src).await? {
        return Err(ToolError::not_found(format!("'{src}' not found")));
    }
    if tx_exists(tx, volume, dst).await? {
        return Err(ToolError::no_clobber(format!("'{dst}' already exists")));
    }
    tx_ensure_parents(tx, dialect, volume, dst).await?;

    let rows = tx
        .query(
            &Query::new(format!(
                "SELECT path FROM nodes WHERE volume_id=?1 AND (path=?2 OR {within}) \
                 ORDER BY {order}(path)",
                within = sm.clause(dialect, "path", 3, 4),
            ))
            .bind(volume)
            .bind(src)
            .bind(&sm.like)
            .bind(&sm.prefix),
        )
        .await?;
    let paths: Vec<String> = rows.iter().map(|r| r.text(0)).collect::<Result<_>>()?;

    for old in paths {
        let new =
            if old == src { dst.to_string() } else { format!("{}{}", dst, &old[src.len()..]) };
        // A descendant grows by however much `dst` is longer than `src`, so
        // it can overflow the column even though `dst` itself fitted. The
        // whole rename is one transaction, so refusing here leaves the tree
        // untouched rather than half moved.
        ensure_storable_path(dialect, &new)?;
        tx.execute(
            &Query::new(
                "UPDATE nodes SET path=?1, parent=?2, name=?3, rev=?4 \
                     WHERE volume_id=?5 AND path=?6",
            )
            .bind(&new)
            .bind(PosixPath::parent_of(&new))
            .bind(PosixPath::name_of(&new))
            .bind(new_rev())
            .bind(volume)
            .bind(&old),
        )
        .await?;
    }
    Ok(())
}

/// Insert or overwrite one soft-delete record (SPEC-0011 US-0001, DEC-001).
///
/// Takes the caller's open transaction instead of opening one, so US-0002's
/// delete can commit this row atomically with the `rename` that moved the file
/// into the trash (NFR 7.4): a failed rename never leaves an orphaned row, and
/// a failed insert never leaves an untracked rename, because both roll back
/// together.
// One argument per column plus the transaction: a struct wrapper would just
// rename the same seven fields the story's call site spells out directly.
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) async fn insert_trash_entry(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    trash_path: &str,
    original_path: &str,
    size: i64,
    kind: &str,
    deleted_at: &str,
    deleted_by: Option<&str>,
) -> Result<()> {
    tx.execute(
        &Query::new(format!(
            "INSERT INTO trash_entries ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            TRASH_COLS.join(", ")
        ))
        .bind(volume_id)
        .bind(trash_path)
        .bind(original_path)
        .bind(size)
        .bind(kind)
        .bind(deleted_at)
        .bind(deleted_by),
    )
    .await?;
    Ok(())
}

/// Remove exactly one row. A second call for the same key is a no-op: the
/// `DELETE` simply matches nothing, so a caller never has to check existence
/// first to make the delete idempotent.
#[allow(dead_code)]
pub(crate) async fn delete_trash_entry(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    trash_path: &str,
) -> Result<()> {
    tx.execute(
        &Query::new("DELETE FROM trash_entries WHERE volume_id=?1 AND trash_path=?2")
            .bind(volume_id)
            .bind(trash_path),
    )
    .await?;
    Ok(())
}

/// The row for `(volume_id, trash_path)`, or `None` when there is none.
#[allow(dead_code)]
pub(crate) async fn get_trash_entry(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    trash_path: &str,
) -> Result<Option<TrashEntryRow>> {
    let row = tx
        .query_opt(
            &Query::new(format!(
                "SELECT {} FROM trash_entries WHERE volume_id=?1 AND trash_path=?2",
                TRASH_COLS.join(", ")
            ))
            .bind(volume_id)
            .bind(trash_path),
        )
        .await?;
    row.as_ref().map(read_trash_row).transpose()
}

/// Count of `trash_entries` rows matching the same `path_prefix` filter as
/// [`list_trash_entries`], unpaged (US-0004's `total`).
async fn count_trash_entries(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    path_prefix: &str,
) -> Result<i64> {
    let dialect = tx.dialect();
    let row = if path_prefix.is_empty() {
        tx.query_opt(
            &Query::new("SELECT COUNT(*) FROM trash_entries WHERE volume_id=?1").bind(volume_id),
        )
        .await?
    } else {
        let pattern = trash_path_prefix_pattern(dialect, path_prefix);
        tx.query_opt(
            &Query::new(
                "SELECT COUNT(*) FROM trash_entries WHERE volume_id=?1 \
                 AND (original_path=?2 OR original_path LIKE ?3 ESCAPE '\\')",
            )
            .bind(volume_id)
            .bind(path_prefix)
            .bind(pattern),
        )
        .await?
    };
    match row {
        Some(r) => r.i64(0),
        None => Ok(0),
    }
}

/// Every row for `volume_id`, newest delete first, optionally restricted to
/// `path_prefix` and its descendants (US-0004's `fs.trash_list` filter; an
/// empty prefix matches every row). `limit`/`offset` page the result.
pub(crate) async fn list_trash_entries(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    path_prefix: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<TrashEntryRow>> {
    let dialect = tx.dialect();
    let page = dialect.render_limit_offset(limit, offset);
    let cols = TRASH_COLS.join(", ");
    let rows = if path_prefix.is_empty() {
        tx.query(
            &Query::new(format!(
                "SELECT {cols} FROM trash_entries WHERE volume_id=?1 \
                 ORDER BY deleted_at DESC {page}"
            ))
            .bind(volume_id),
        )
        .await?
    } else {
        let pattern = trash_path_prefix_pattern(dialect, path_prefix);
        tx.query(
            &Query::new(format!(
                "SELECT {cols} FROM trash_entries WHERE volume_id=?1 \
                 AND (original_path=?2 OR original_path LIKE ?3 ESCAPE '\\') \
                 ORDER BY deleted_at DESC {page}"
            ))
            .bind(volume_id)
            .bind(path_prefix)
            .bind(pattern),
        )
        .await?
    };
    rows.iter().map(read_trash_row).collect()
}

// ── export_links CRUD (SPEC-0012 US-0001, FR-NEW-015) ───────────────────────────
//
// Called by tools/export.rs (insert), exports.rs (atomic delete-and-fetch) and
// purge.rs (sweep of expired rows).

/// Every column of `export_links`, in the order the insert binds them.
const EXPORT_LINK_COLS: [&str; 4] = ["token", "volume_id", "created_at", "expires_at"];

fn read_export_link_row(r: &RowValues) -> Result<ExportLinkRow> {
    Ok(ExportLinkRow {
        token: r.text(0)?,
        volume_id: r.text(1)?,
        created_at: r.text(2)?,
        expires_at: r.text(3)?,
    })
}

/// Insert one pending export record (FR-NEW-007's write side).
pub(crate) async fn insert_export_link(
    tx: &mut dyn RelationalTx,
    token: &str,
    volume_id: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<()> {
    tx.execute(
        &Query::new(format!(
            "INSERT INTO export_links ({}) VALUES (?1, ?2, ?3, ?4)",
            EXPORT_LINK_COLS.join(", ")
        ))
        .bind(token)
        .bind(volume_id)
        .bind(created_at)
        .bind(expires_at),
    )
    .await?;
    Ok(())
}

/// Whether `volume_id` still holds an `export_links` row for `token`, expired
/// or not, so the orphan sweep never deletes a zip a row still points at
/// (SPEC-0034/DR-004, DR-005); expiry is the export sweep's business.
pub(crate) async fn export_link_exists(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    token: &str,
) -> Result<bool> {
    let row = tx
        .query_opt(
            &Query::new("SELECT token FROM export_links WHERE volume_id=?1 AND token=?2")
                .bind(volume_id)
                .bind(token),
        )
        .await?;
    Ok(row.is_some())
}

/// Atomically delete and report whether `token` was a live, unexpired row
/// (FR-NEW-009/FR-NEW-010/FR-NEW-011/FR-NEW-013's shared storage primitive).
///
/// A single `DELETE ... WHERE token=? AND expires_at>?` followed by its
/// affected-row count IS the whole mechanism: a replay of an already
/// consumed token, an unknown token, and an expired-but-still-present token
/// all return `false` with no row left behind, and two concurrent callers
/// racing on the same token can only ever have one of them see `true`.
pub(crate) async fn delete_export_link_if_live(
    tx: &mut dyn RelationalTx,
    token: &str,
    now: &str,
) -> Result<bool> {
    let affected = tx
        .execute(
            &Query::new("DELETE FROM export_links WHERE token=?1 AND expires_at>?2")
                .bind(token)
                .bind(now),
        )
        .await?;
    Ok(affected == 1)
}

/// Every row for `volume_id` whose `expires_at` has passed, for the sweep to
/// iterate and delete (FR-NEW-014, later story; DEC-013's one-timestamp
/// criterion).
pub(crate) async fn select_expired_export_links(
    tx: &mut dyn RelationalTx,
    volume_id: &str,
    now: &str,
) -> Result<Vec<ExportLinkRow>> {
    let cols = EXPORT_LINK_COLS.join(", ");
    let rows = tx
        .query(
            &Query::new(format!(
                "SELECT {cols} FROM export_links WHERE volume_id=?1 AND expires_at<=?2"
            ))
            .bind(volume_id)
            .bind(now),
        )
        .await?;
    rows.iter().map(read_export_link_row).collect()
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

    async fn used_bytes(&self) -> Result<i64> {
        let read = Query::new("SELECT used_bytes FROM volume_usage WHERE volume_id=?1")
            .bind(&self.volume_id);
        if let Some(row) = self.db.query_opt(&read).await? {
            return row.i64(0);
        }
        // DEC-002 backfill, once per volume: a volume written before the
        // counter existed. The sum and the insert share one transaction, and
        // the insert ignores a row a concurrent backfill already wrote.
        let insert = self.db.dialect().render_upsert(&Upsert::ignore(
            "volume_usage",
            vec!["volume_id", "used_bytes"],
            vec!["volume_id"],
        ));
        let volume = self.volume_id.clone();
        run_retrying(&*self.db, move |tx| {
            let (volume, insert) = (volume.clone(), insert.clone());
            Box::pin(async move {
                // CAST: PostgreSQL widens SUM(BIGINT) to NUMERIC.
                let total = tx
                    .query_opt(
                        &Query::new(
                            "SELECT CAST(COALESCE(SUM(size), 0) AS BIGINT) FROM nodes \
                             WHERE volume_id=?1 AND kind='file'",
                        )
                        .bind(&volume),
                    )
                    .await?
                    .map_or(Ok(0), |r| r.i64(0))?;
                tx.execute(&Query::new(insert).bind(&volume).bind(total)).await?;
                let row = tx
                    .query_opt(
                        &Query::new("SELECT used_bytes FROM volume_usage WHERE volume_id=?1")
                            .bind(&volume),
                    )
                    .await?;
                row.map_or(Ok(total), |r| r.i64(0))
            })
        })
        .await
    }

    async fn stale_files(&self, before: f64, exclude_root: &str) -> Result<Vec<NodeRow>> {
        let rows = self
            .db
            .query(
                &Query::new(format!(
                    "SELECT {SELECT_COLS} FROM nodes \
                     WHERE volume_id=?1 AND kind='file' AND atime<?2 \
                     AND path<>?3 AND path NOT LIKE ?4 ESCAPE '\\' \
                     ORDER BY path"
                ))
                .bind(&self.volume_id)
                .bind(before)
                .bind(exclude_root)
                .bind(self.descendant_pattern(exclude_root)),
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
    ) -> Result<PutFileResult> {
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
                            "SELECT kind, ctime, sha256, size FROM nodes \
                             WHERE volume_id=?1 AND path=?2",
                        )
                        .bind(&volume)
                        .bind(&path),
                    )
                    .await?;

                let mut ctime = now_unix();
                let mut old_sha: Option<String> = None;
                let mut old_size: i64 = 0;
                if let Some(row) = existing {
                    if row.text(0)? == "dir" {
                        return Err(ToolError::invalid_argument(format!(
                            "'{path}' is a directory"
                        )));
                    }
                    ctime = row.f64(1)?;
                    old_sha = row.opt_text(2)?;
                    old_size = row.i64(3)?;
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
                        .bind(sha.clone())
                        .bind(new_rev()),
                )
                .await?;
                // FR-NEW-002: the declared size per path, so an overwrite moves
                // usage by the difference and shared content counts once per path.
                tx_add_usage(tx, &volume, size - old_size).await?;
                Ok(PutFileResult { gc, old_size })
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
                            "SELECT kind, sha256, size FROM nodes WHERE volume_id=?1 AND path=?2",
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
                let size = row.i64(2)?;
                let gc = if tx_decref(tx, &volume, sha.as_deref()).await? { sha } else { None };
                tx.execute(
                    &Query::new("DELETE FROM nodes WHERE volume_id=?1 AND path=?2")
                        .bind(&volume)
                        .bind(&path),
                )
                .await?;
                tx_add_usage(tx, &volume, -size).await?;
                Ok(gc)
            })
        })
        .await
    }

    async fn remove_subtree(&self, path: &str) -> Result<Vec<String>> {
        let dialect = self.db.dialect();
        let sm = SubtreeMatch::new(dialect, path);
        let within = sm.clause(dialect, "path", 3, 4);
        let (select_sql, delete_sql) = (
            format!(
                "SELECT sha256, kind, size FROM nodes \
                 WHERE volume_id=?1 AND (path=?2 OR {within})"
            ),
            format!("DELETE FROM nodes WHERE volume_id=?1 AND (path=?2 OR {within})"),
        );
        let volume = self.volume_id.clone();
        let path = path.to_string();
        run_retrying(&*self.db, move |tx| {
            let (volume, path) = (volume.clone(), path.clone());
            let (select_sql, delete_sql) = (select_sql.clone(), delete_sql.clone());
            let (like, prefix) = (sm.like.clone(), sm.prefix.clone());
            // The accumulator is built inside the body, so a retry starts from an
            // empty list instead of appending to the previous attempt's result.
            Box::pin(async move {
                let rows = tx
                    .query(
                        &Query::new(select_sql).bind(&volume).bind(&path).bind(&like).bind(&prefix),
                    )
                    .await?;
                let shas: Vec<Option<String>> =
                    rows.iter().map(|r| r.opt_text(0)).collect::<Result<_>>()?;
                let mut removed = 0i64;
                for r in &rows {
                    if r.text(1)? == "file" {
                        removed += r.i64(2)?;
                    }
                }

                let mut gc = Vec::new();
                for sha in &shas {
                    if sha.is_some() && tx_decref(tx, &volume, sha.as_deref()).await? {
                        gc.extend(sha.clone());
                    }
                }
                tx.execute(
                    &Query::new(delete_sql).bind(&volume).bind(&path).bind(&like).bind(&prefix),
                )
                .await?;
                tx_add_usage(tx, &volume, -removed).await?;
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
        let dialect = self.db.dialect();
        let sm = SubtreeMatch::new(dialect, src);
        let volume = self.volume_id.clone();
        let (src, dst) = (src.to_string(), dst.to_string());
        run_retrying(&*self.db, move |tx| {
            let (volume, src, dst, sm) = (volume.clone(), src.clone(), dst.clone(), sm.clone());
            Box::pin(async move { tx_rename(tx, dialect, &volume, &src, &dst, &sm).await })
        })
        .await
    }

    async fn rename_into_trash(&self, src: &str, dst: &str, entry: &TrashEntry) -> Result<()> {
        let dialect = self.db.dialect();
        let sm = SubtreeMatch::new(dialect, src);
        let volume = self.volume_id.clone();
        let (src, dst, entry) = (src.to_string(), dst.to_string(), entry.clone());
        run_retrying(&*self.db, move |tx| {
            let (volume, src, dst, sm, entry) =
                (volume.clone(), src.clone(), dst.clone(), sm.clone(), entry.clone());
            Box::pin(async move {
                // One transaction: the row exists exactly when the move does.
                tx_rename(tx, dialect, &volume, &src, &dst, &sm).await?;
                insert_trash_entry(
                    tx,
                    &volume,
                    &dst,
                    &entry.original_path,
                    entry.size,
                    &entry.kind,
                    &entry.deleted_at,
                    entry.deleted_by.as_deref(),
                )
                .await
            })
        })
        .await
    }

    async fn touch_atime(&self, path: &str) -> Result<()> {
        self.db
            .execute(
                &Query::new("UPDATE nodes SET atime=?1 WHERE volume_id=?2 AND path=?3")
                    .bind(now_unix())
                    .bind(&self.volume_id)
                    .bind(path),
            )
            .await?;
        Ok(())
    }

    async fn touch_atime_mtime(&self, path: &str) -> Result<()> {
        let now = now_unix();
        self.db
            .execute(
                &Query::new("UPDATE nodes SET atime=?1, mtime=?2 WHERE volume_id=?3 AND path=?4")
                    .bind(now)
                    .bind(now)
                    .bind(&self.volume_id)
                    .bind(path),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
impl RelationalMetaStore {
    /// Direct write of an arbitrary `atime`, bypassing `touch_atime`'s
    /// always-now contract. Test-only: used to seed files whose `atime` is a
    /// specific number of days in the past (SPEC-0014 US-0005 boundary tests).
    pub(crate) async fn set_atime_for_test(&self, path: &str, atime: f64) -> Result<()> {
        self.db
            .execute(
                &Query::new("UPDATE nodes SET atime=?1 WHERE volume_id=?2 AND path=?3")
                    .bind(atime)
                    .bind(&self.volume_id)
                    .bind(path),
            )
            .await?;
        Ok(())
    }

    /// Read one `trash_entries` row directly, for a `fs_ops.rs` test asserting
    /// what SPEC-0011 US-0002's `delete_path` wrote.
    pub(crate) async fn get_trash_entry_for_test(
        &self,
        trash_path: &str,
    ) -> Result<Option<TrashEntryRow>> {
        let mut tx = self.db.begin().await?;
        let row = get_trash_entry(&mut *tx, &self.volume_id, trash_path).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Count every `trash_entries` row for this volume, for a `fs_ops.rs` test
    /// asserting how many rows a delete wrote (SPEC-0011 US-0002).
    pub(crate) async fn count_trash_entries_for_test(&self) -> Result<usize> {
        let mut tx = self.db.begin().await?;
        let rows = list_trash_entries(&mut *tx, &self.volume_id, "", i64::MAX, 0).await?;
        tx.commit().await?;
        Ok(rows.len())
    }

    /// Give each of `shas` one `blob_refs` row in this volume, in one
    /// transaction, so a SPEC-0034 sweep test can seed thousands of referenced
    /// blobs without writing a file node per blob.
    pub(crate) async fn add_blob_refs_for_test(&self, shas: &[String]) -> Result<()> {
        let dialect = self.db.dialect();
        let mut tx = self.db.begin().await?;
        for sha in shas {
            tx_incref(&mut *tx, dialect, &self.volume_id, Some(sha), 1).await?;
        }
        tx.commit().await
    }

    /// The `blob_refs` count of `sha` in this volume, `None` when it has no
    /// row, for the SPEC-0019 DEC-004 conformance case.
    pub(crate) async fn blob_refcount_for_test(&self, sha: &str) -> Result<Option<i64>> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT refcount FROM blob_refs WHERE volume_id=?1 AND sha256=?2")
                    .bind(&self.volume_id)
                    .bind(sha),
            )
            .await?;
        row.map(|r| r.i64(0)).transpose()
    }

    /// Count of `doc_artifact_sets` rows for `set_id` in this volume, for a
    /// `purge.rs` test asserting a swept set's row is really gone (US-0013).
    pub(crate) async fn count_artifact_set_rows_for_test(&self, set_id: &str) -> Result<i64> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT COUNT(*) FROM doc_artifact_sets WHERE volume_id=?1 AND set_id=?2",
                )
                .bind(&self.volume_id)
                .bind(set_id),
            )
            .await?;
        row.map_or(Ok(0), |r| r.i64(0))
    }

    /// Count of `doc_images` rows for `set_id` in this volume, for the same
    /// US-0013 test.
    pub(crate) async fn count_doc_image_rows_for_test(&self, set_id: &str) -> Result<i64> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT COUNT(*) FROM doc_images WHERE volume_id=?1 AND set_id=?2")
                    .bind(&self.volume_id)
                    .bind(set_id),
            )
            .await?;
        row.map_or(Ok(0), |r| r.i64(0))
    }

    /// Remove one `trash_entries` row directly, for a `fs_ops.rs` test
    /// simulating a closed soft-delete/restore cycle ahead of US-0005
    /// (SPEC-0011 US-0002, E2E-NEW-456).
    pub(crate) async fn delete_trash_entry_for_test(&self, trash_path: &str) -> Result<()> {
        let mut tx = self.db.begin().await?;
        delete_trash_entry(&mut *tx, &self.volume_id, trash_path).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Drop this volume's `volume_usage` row, the state of a volume written
    /// before SPEC-0020, so a test can prove the backfill rebuilds it.
    pub(crate) async fn clear_usage_row_for_test(&self) -> Result<()> {
        self.db
            .execute(
                &Query::new("DELETE FROM volume_usage WHERE volume_id=?1").bind(&self.volume_id),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::rel::{Dialect, SqliteRelationalDb};
    use crate::storage::traits::MODE_FILE;

    /// SPEC-0024 DEC-001: the shared database teardown deletes from exactly these
    /// tables, so a table added to `schema()` with a `volume_id` must show up here.
    #[test]
    fn volume_tables_lists_every_table_keyed_by_volume() {
        assert_eq!(
            volume_tables(),
            vec![
                "nodes",
                "blob_refs",
                "trash_entries",
                "volume_usage",
                "export_links",
                "doc_artifact_current",
                "doc_artifact_sets",
                "doc_images",
                "doc_tables",
            ]
        );
    }

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

    /// SPEC-0050 DEC-004..006: the clause keeps the escaped `LIKE` and adds the
    /// exact substring equality, rendered per engine.
    #[test]
    fn subtree_match_renders_a_case_exact_clause_per_dialect() {
        let sqlite = SubtreeMatch::new(Dialect::Sqlite, "/video");
        assert_eq!(sqlite.like, "/video/%");
        assert_eq!(sqlite.prefix, "/video/");
        assert_eq!(
            sqlite.clause(Dialect::Sqlite, "path", 3, 4),
            "(path LIKE ?3 ESCAPE '\\' AND substr(path, 1, 7) = ?4)"
        );
        let mssql = SubtreeMatch::new(Dialect::SqlServer, "/é📊/");
        assert_eq!(mssql.prefix, "/é📊/", "one trailing slash, not two");
        assert_eq!(
            mssql.clause(Dialect::SqlServer, "original_path", 2, 3),
            "(original_path LIKE ?2 ESCAPE '\\' AND SUBSTRING(original_path, 1, 5) = ?3 \
             COLLATE Latin1_General_100_BIN2)"
        );
        let root = SubtreeMatch::new(Dialect::Postgres, "/");
        assert_eq!((root.like.as_str(), root.prefix.as_str()), ("/%", "/"));
        let wild = SubtreeMatch::new(Dialect::Sqlite, "/a_b");
        assert_eq!(wild.like, "/a\\_b/%");
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
        let res = s.put_file("/a/b/c.txt", Some("sha1"), 3, MODE_FILE).await.unwrap();
        assert_eq!(res.gc, None);
        assert_eq!(res.old_size, 0, "no prior node at this path");
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

        let res = s.put_file("/a.txt", Some("new"), 3, MODE_FILE).await.unwrap();
        assert_eq!(res.gc.as_deref(), Some("old"), "the replaced blob is GC'd");
        let f = s.get("/a.txt").await.unwrap().unwrap();
        assert_eq!(f.sha256.as_deref(), Some("new"));
        assert_eq!(f.ctime, ctime0, "ctime is preserved across overwrite");
    }

    /// DRIFT-001: `put_file`'s existing in-transaction node lookup must expose
    /// the pre-existing size at `path`, so FR-NEW-009's quota check can read it
    /// without a second query.
    #[tokio::test]
    async fn put_file_reports_old_size_of_overwritten_path() {
        let s = store().await;
        s.put_file("/a.txt", Some("old"), 7, MODE_FILE).await.unwrap();

        let res = s.put_file("/a.txt", Some("new"), 3, MODE_FILE).await.unwrap();
        assert_eq!(res.old_size, 7, "old size read from the same transaction's lookup");

        let res_new = s.put_file("/new.txt", Some("n"), 5, MODE_FILE).await.unwrap();
        assert_eq!(res_new.old_size, 0, "no prior node means old_size is 0");
    }

    #[tokio::test]
    async fn rewriting_same_sha_does_not_gc() {
        let s = store().await;
        s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        let res = s.put_file("/a.txt", Some("x"), 1, MODE_FILE).await.unwrap();
        assert_eq!(res.gc, None);
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

    #[tokio::test]
    async fn touch_atime_bumps_only_atime() {
        let s = store().await;
        s.put_file("/a.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        let before = s.get("/a.txt").await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        s.touch_atime("/a.txt").await.unwrap();

        let after = s.get("/a.txt").await.unwrap().unwrap();
        assert!(after.atime > before.atime, "atime must advance");
        assert_eq!(after.mtime, before.mtime, "mtime must stay untouched");
    }

    #[tokio::test]
    async fn touch_atime_mtime_bumps_both() {
        let s = store().await;
        s.put_file("/a.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        let before = s.get("/a.txt").await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        s.touch_atime_mtime("/a.txt").await.unwrap();

        let after = s.get("/a.txt").await.unwrap().unwrap();
        assert!(after.atime > before.atime, "atime must advance");
        assert!(after.mtime > before.mtime, "mtime must advance");
    }

    #[tokio::test]
    async fn touch_atime_is_scoped_to_its_volume() {
        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalMetaStore::open(db.clone(), "vol-a").await.unwrap();
        let b = RelationalMetaStore::open(db, "vol-b").await.unwrap();
        a.put_file("/f.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        b.put_file("/f.txt", Some("1"), 1, MODE_FILE).await.unwrap();
        let b_before = b.get("/f.txt").await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        a.touch_atime_mtime("/f.txt").await.unwrap();

        let b_after = b.get("/f.txt").await.unwrap().unwrap();
        assert_eq!(b_after.atime, b_before.atime, "volume b must be untouched");
        assert_eq!(b_after.mtime, b_before.mtime, "volume b must be untouched");
    }

    // ── trash_entries CRUD (SPEC-0011 US-0001) ──────────────────────────────────

    /// This story's one acceptance test: every `trash_entries` trait method,
    /// called from inside a transaction the test holds open, exactly as
    /// US-0002/US-0003 will call them from inside their own `run_retrying` body.
    #[tokio::test]
    async fn trash_entries_round_trip_insert_get_delete_list_and_isolate_volumes() {
        let db: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.migrate(&schema()).await.unwrap();
        let mut tx = db.begin().await.unwrap();

        insert_trash_entry(
            &mut *tx,
            "v1",
            "/.mcp_trash/123__a.txt",
            "/a.txt",
            5,
            "file",
            "2026-01-01T00:00:00Z",
            Some("alice"),
        )
        .await
        .unwrap();
        let got = get_trash_entry(&mut *tx, "v1", "/.mcp_trash/123__a.txt")
            .await
            .unwrap()
            .expect("row was just inserted");
        assert_eq!(got.volume_id, "v1");
        assert_eq!(got.trash_path, "/.mcp_trash/123__a.txt");
        assert_eq!(got.original_path, "/a.txt");
        assert_eq!(got.size, 5);
        assert_eq!(got.kind, "file");
        assert_eq!(got.deleted_at, "2026-01-01T00:00:00Z");
        assert_eq!(got.deleted_by, Some("alice".to_string()));

        // A second row with `deleted_by: None` must round-trip a real SQL NULL.
        insert_trash_entry(
            &mut *tx,
            "v1",
            "/.mcp_trash/456__b.txt",
            "/b.txt",
            9,
            "file",
            "2026-01-02T00:00:00Z",
            None,
        )
        .await
        .unwrap();
        let raw = tx
            .query_opt(
                &Query::new(
                    "SELECT deleted_by FROM trash_entries WHERE volume_id=?1 AND trash_path=?2",
                )
                .bind("v1")
                .bind("/.mcp_trash/456__b.txt"),
            )
            .await
            .unwrap()
            .expect("row was just inserted");
        assert_eq!(
            raw.value(0).unwrap(),
            &crate::storage::rel::SqlValue::Null,
            "a real SQL NULL, not an empty string"
        );
        let got_none = get_trash_entry(&mut *tx, "v1", "/.mcp_trash/456__b.txt")
            .await
            .unwrap()
            .expect("row was just inserted");
        assert_eq!(got_none.deleted_by, None);

        // A third row, deleted before the second, so descending order is
        // actually exercised once the first row is gone.
        insert_trash_entry(
            &mut *tx,
            "v1",
            "/.mcp_trash/789__d.txt",
            "/d.txt",
            1,
            "dir",
            "2025-12-31T00:00:00Z",
            Some("bob"),
        )
        .await
        .unwrap();

        // Idempotent delete: removes exactly that row, a second call is a no-op.
        delete_trash_entry(&mut *tx, "v1", "/.mcp_trash/123__a.txt").await.unwrap();
        assert!(get_trash_entry(&mut *tx, "v1", "/.mcp_trash/123__a.txt").await.unwrap().is_none());
        delete_trash_entry(&mut *tx, "v1", "/.mcp_trash/123__a.txt")
            .await
            .expect("deleting an already absent key is a no-op, not an error");

        // A second volume's row must be invisible to a query scoped to "v1".
        insert_trash_entry(
            &mut *tx,
            "v2",
            "/.mcp_trash/999__c.txt",
            "/c.txt",
            1,
            "file",
            "2026-01-03T00:00:00Z",
            None,
        )
        .await
        .unwrap();

        let listed = list_trash_entries(&mut *tx, "v1", "", 10, 0).await.unwrap();
        let paths: Vec<&str> = listed.iter().map(|r| r.trash_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/.mcp_trash/456__b.txt", "/.mcp_trash/789__d.txt"],
            "newest delete first, and volume v2's row is excluded"
        );

        tx.commit().await.unwrap();
    }

    // ── export_links CRUD (SPEC-0012 US-0001, FR-NEW-015) ───────────────────────────

    /// E2E-NEW-040/E2E-NEW-050's storage-layer floor: insert one row, read it
    /// back directly, assert every column matches.
    #[tokio::test]
    async fn export_link_insert_round_trips_every_column() {
        let db: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.migrate(&schema()).await.unwrap();
        let mut tx = db.begin().await.unwrap();

        insert_export_link(
            &mut *tx,
            "11111111-1111-4111-8111-111111111111",
            "proj",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:05:00Z",
        )
        .await
        .unwrap();

        let rows =
            select_expired_export_links(&mut *tx, "proj", "2099-01-01T00:00:00Z").await.unwrap();
        assert_eq!(rows.len(), 1, "the row must exist and belong to this volume");
        let got = &rows[0];
        assert_eq!(got.token, "11111111-1111-4111-8111-111111111111");
        assert_eq!(got.volume_id, "proj");
        assert_eq!(got.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(got.expires_at, "2026-01-01T00:05:00Z");

        tx.commit().await.unwrap();
    }

    /// The atomic consume primitive: success once, "not found" on replay of
    /// the same token (FR-NEW-009/FR-NEW-010's storage-layer mechanism).
    #[tokio::test]
    async fn export_link_delete_if_live_succeeds_once_then_reports_not_found_on_replay() {
        let db: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.migrate(&schema()).await.unwrap();
        let mut tx = db.begin().await.unwrap();

        insert_export_link(
            &mut *tx,
            "22222222-2222-4222-8222-222222222222",
            "proj",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:05:00Z",
        )
        .await
        .unwrap();

        let consumed = delete_export_link_if_live(
            &mut *tx,
            "22222222-2222-4222-8222-222222222222",
            "2026-01-01T00:01:00Z",
        )
        .await
        .unwrap();
        assert!(consumed, "a live, unexpired token must be reported as consumed");

        let replay = delete_export_link_if_live(
            &mut *tx,
            "22222222-2222-4222-8222-222222222222",
            "2026-01-01T00:01:00Z",
        )
        .await
        .unwrap();
        assert!(!replay, "a second consume of the same token must report not found");

        let unknown =
            delete_export_link_if_live(&mut *tx, "not-a-real-token", "2026-01-01T00:01:00Z")
                .await
                .unwrap();
        assert!(!unknown, "an unknown token must report not found, not error");

        tx.commit().await.unwrap();
    }

    /// FR-NEW-011's storage-layer mechanism: a row whose `expires_at` is
    /// already in the past must report "not found" even though it still
    /// physically exists in the table.
    #[tokio::test]
    async fn export_link_delete_if_live_reports_not_found_once_expired() {
        let db: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.migrate(&schema()).await.unwrap();
        let mut tx = db.begin().await.unwrap();

        insert_export_link(
            &mut *tx,
            "33333333-3333-4333-8333-333333333333",
            "proj",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:05:00Z",
        )
        .await
        .unwrap();

        let expired = delete_export_link_if_live(
            &mut *tx,
            "33333333-3333-4333-8333-333333333333",
            "2026-01-01T00:10:00Z",
        )
        .await
        .unwrap();
        assert!(!expired, "a token whose expires_at has already passed must report not found");

        // The row is untouched by the failed delete: still selectable as expired.
        let still_there =
            select_expired_export_links(&mut *tx, "proj", "2026-01-01T00:10:00Z").await.unwrap();
        assert_eq!(still_there.len(), 1, "the row must still physically exist");

        tx.commit().await.unwrap();
    }

    /// The sweep accessor: only the row whose `expires_at` has passed comes
    /// back, the still-live row does not (FR-NEW-014's storage-layer floor).
    #[tokio::test]
    async fn select_expired_export_links_returns_only_the_expired_row() {
        let db: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.migrate(&schema()).await.unwrap();
        let mut tx = db.begin().await.unwrap();

        insert_export_link(
            &mut *tx,
            "44444444-4444-4444-8444-444444444444",
            "proj",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:05:00Z",
        )
        .await
        .unwrap();
        insert_export_link(
            &mut *tx,
            "55555555-5555-4555-8555-555555555555",
            "proj",
            "2026-01-01T01:00:00Z",
            "2026-01-01T01:05:00Z",
        )
        .await
        .unwrap();

        let expired =
            select_expired_export_links(&mut *tx, "proj", "2026-01-01T00:30:00Z").await.unwrap();
        let tokens: Vec<&str> = expired.iter().map(|r| r.token.as_str()).collect();
        assert_eq!(
            tokens,
            vec!["44444444-4444-4444-8444-444444444444"],
            "only the row whose expires_at has passed is returned"
        );

        tx.commit().await.unwrap();
    }
}
