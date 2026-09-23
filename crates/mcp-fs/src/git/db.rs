//! Per project git index (objects, refs, remotes) on any [`RelationalDb`].
//!
//! Rows carry a `volume_id` so one PostgreSQL or SQL Server database can hold the
//! index of every project. Under SQLite there is still one file per project, at
//! `state/git/{project_id}.db` (see `ServerConfig::git_db_path`), and the column
//! is constant within it.
//!
//! Object bytes are NOT here: they live in the blob store under `git:{sha}`. This
//! table is an index over them, which is what makes a short sha lookup cheap.

use crate::errors::{Result, ToolError};
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
/// An operation type or state is a short keyword from a closed set.
const OP_ENUM_LEN: u32 = 32;

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

/// One in-progress operation, paused because it needs a human decision.
///
/// The record is relational rather than in-memory so a paused rebase survives a
/// server restart and is visible to every member of the project (DEC-901).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOperationRow {
    pub op_type: GitOpType,
    /// Where the operation stands: exactly `conflicted` or `running`
    /// (FR-NEW-188).
    pub state: String,
    /// The ref the operation is combining in, when it has one; reported as the
    /// conflict response's `source_ref` and in `git.status` (FR-NEW-286).
    pub source_ref: Option<String>,
    /// The commit the operation replays onto, when it has one.
    pub onto_sha: Option<String>,
    /// The tip before the operation started, so an abort can restore it.
    pub original_tip_sha: Option<String>,
    /// The remaining plan, serialized by the caller (JSON today).
    pub todo: Option<String>,
    pub current_step: i64,
    pub total_steps: i64,
    /// Conflicting paths, serialized by the caller.
    pub conflicts: Option<String>,
    /// Resolutions recorded so far, serialized by the caller.
    pub resolutions: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The closed set of operations that can pause mid-flight (FR-NEW-285).
///
/// `stash_apply` and `stash_pop` are distinct because completion differs: a
/// resumed pop drops the stash entry, a resumed apply keeps it, so the row has
/// to remember which tool was called. A pull conflict is recorded as `merge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOpType {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    StashApply,
    StashPop,
}

impl GitOpType {
    /// Every value, in the order the specification lists them.
    pub const ALL: [GitOpType; 6] = [
        GitOpType::Merge,
        GitOpType::Rebase,
        GitOpType::CherryPick,
        GitOpType::Revert,
        GitOpType::StashApply,
        GitOpType::StashPop,
    ];

    /// The stored and reported spelling, shared by the conflict response and by
    /// the `operation` object of `git.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            GitOpType::Merge => "merge",
            GitOpType::Rebase => "rebase",
            GitOpType::CherryPick => "cherry_pick",
            GitOpType::Revert => "revert",
            GitOpType::StashApply => "stash_apply",
            GitOpType::StashPop => "stash_pop",
        }
    }

    /// The tool that completes this operation (FR-NEW-241). Carried by the
    /// type so the conflict response never guesses the pair per call site: a
    /// pull and a stash apply are completed by the merge tools, because both
    /// conflicts ARE merge conflicts.
    pub fn continue_with(self) -> &'static str {
        match self {
            GitOpType::Merge | GitOpType::StashApply | GitOpType::StashPop => "git.merge_resolve",
            GitOpType::Rebase => "git.rebase_continue",
            GitOpType::CherryPick => "git.cherry_pick_continue",
            GitOpType::Revert => "git.revert_continue",
        }
    }

    /// The tool that abandons this operation (FR-NEW-241).
    pub fn abort_with(self) -> &'static str {
        match self {
            GitOpType::Merge | GitOpType::StashApply | GitOpType::StashPop => "git.merge_abort",
            GitOpType::Rebase => "git.rebase_abort",
            GitOpType::CherryPick => "git.cherry_pick_abort",
            GitOpType::Revert => "git.revert_abort",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| ToolError::internal(format!("unknown git operation type: {s}")))
    }
}

/// The tables this store owns, named once so a purge cannot miss one.
pub const TABLES: [&str; 4] = ["git_objects", "git_operations", "git_refs", "git_remotes"];

/// Every column of `git_operations`, in placeholder order. Named once so the
/// upsert and the select cannot drift apart.
const OPERATION_COLUMNS: [&str; 13] = [
    "volume_id",
    "op_type",
    "state",
    "source_ref",
    "onto_sha",
    "original_tip_sha",
    "todo",
    "current_step",
    "total_steps",
    "conflicts",
    "resolutions",
    "created_at",
    "updated_at",
];

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
            // Keyed on `volume_id` alone: a volume holds at most one in-progress
            // operation, so the primary key is what enforces FR-NEW-283.
            Table::new(
                "git_operations",
                vec![
                    volume(),
                    Column::required("op_type", ColumnType::TextKey(OP_ENUM_LEN)),
                    Column::required("state", ColumnType::TextKey(OP_ENUM_LEN)),
                    // Declared in the table rather than as a column migration:
                    // `git_operations` is introduced on this same unreleased
                    // branch, so no deployed database carries the narrow shape.
                    Column::new("source_ref", ColumnType::TextKey(REF_NAME_LEN)),
                    Column::new("onto_sha", ColumnType::TextKey(HASH_LEN)),
                    Column::new("original_tip_sha", ColumnType::TextKey(HASH_LEN)),
                    // Payloads are unbounded: a todo list or a conflict set has no
                    // useful length ceiling and neither is ever indexed.
                    Column::new("todo", ColumnType::Text),
                    Column::required("current_step", ColumnType::BigInt).default("0"),
                    Column::required("total_steps", ColumnType::BigInt).default("0"),
                    Column::new("conflicts", ColumnType::Text),
                    Column::new("resolutions", ColumnType::Text),
                    Column::required("created_at", ColumnType::Text),
                    Column::required("updated_at", ColumnType::Text),
                ],
                vec!["volume_id"],
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

    // ── operations ──────────────────────────────────────────────────────────

    /// Write the in-progress operation of this volume, replacing any previous
    /// one: a volume holds at most one (FR-NEW-283).
    pub async fn set_operation(&self, op: &GitOperationRow) -> Result<()> {
        let sql = self.db.dialect().render_upsert(&Upsert::replace(
            "git_operations",
            OPERATION_COLUMNS.to_vec(),
            vec!["volume_id"],
        ));
        self.db
            .execute(
                &Query::new(sql)
                    .bind(&self.volume_id)
                    .bind(op.op_type.as_str())
                    .bind(&op.state)
                    .bind(op.source_ref.as_ref())
                    .bind(op.onto_sha.as_ref())
                    .bind(op.original_tip_sha.as_ref())
                    .bind(op.todo.as_ref())
                    .bind(op.current_step)
                    .bind(op.total_steps)
                    .bind(op.conflicts.as_ref())
                    .bind(op.resolutions.as_ref())
                    .bind(&op.created_at)
                    .bind(&op.updated_at),
            )
            .await?;
        Ok(())
    }

    /// The in-progress operation of this volume, if any.
    pub async fn get_operation(&self) -> Result<Option<GitOperationRow>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT op_type, state, source_ref, onto_sha, original_tip_sha, todo, \
                     current_step, total_steps, conflicts, resolutions, created_at, updated_at \
                     FROM git_operations WHERE volume_id=?1",
                )
                .bind(&self.volume_id),
            )
            .await?;
        match row {
            Some(r) => Ok(Some(GitOperationRow {
                op_type: GitOpType::parse(&r.text(0)?)?,
                state: r.text(1)?,
                source_ref: r.opt_text(2)?,
                onto_sha: r.opt_text(3)?,
                original_tip_sha: r.opt_text(4)?,
                todo: r.opt_text(5)?,
                current_step: r.i64(6)?,
                total_steps: r.i64(7)?,
                conflicts: r.opt_text(8)?,
                resolutions: r.opt_text(9)?,
                created_at: r.text(10)?,
                updated_at: r.text(11)?,
            })),
            None => Ok(None),
        }
    }

    /// Drop the operation row. Returns whether there was one, which is how a
    /// caller tells "aborted" from "nothing in progress" (FR-NEW-284).
    pub async fn clear_operation(&self) -> Result<bool> {
        let affected = self
            .db
            .execute(
                &Query::new("DELETE FROM git_operations WHERE volume_id=?1").bind(&self.volume_id),
            )
            .await?;
        Ok(affected > 0)
    }

    /// Operation rows for this volume. Zero or one by construction.
    pub async fn count_operations(&self) -> Result<i64> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT COUNT(*) FROM git_operations WHERE volume_id=?1")
                    .bind(&self.volume_id),
            )
            .await?;
        match row {
            Some(r) => r.i64(0),
            None => Ok(0),
        }
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
    async fn schema_has_the_four_tables() {
        let d = db().await;
        assert_eq!(
            d.table_names().await.unwrap(),
            vec!["git_objects", "git_operations", "git_refs", "git_remotes"]
        );
    }

    fn paused_rebase() -> GitOperationRow {
        GitOperationRow {
            op_type: GitOpType::Rebase,
            state: "conflicted".into(),
            source_ref: Some("feature".into()),
            onto_sha: Some("onto1".into()),
            original_tip_sha: Some("tip1".into()),
            todo: Some(r#"[{"sha":"F1"},{"sha":"F2"}]"#.into()),
            current_step: 1,
            total_steps: 2,
            conflicts: Some(r#"["/a.txt"]"#.into()),
            resolutions: None,
            created_at: "2026-09-22T10:00:00Z".into(),
            updated_at: "2026-09-22T10:00:00Z".into(),
        }
    }

    /// FR-NEW-276: a purge iterates `TABLES`, so the new table must be named there
    /// or a deleted project leaves its operation rows behind.
    #[test]
    fn git_operations_is_registered_for_purge() {
        assert_eq!(TABLES.len(), 4, "the purge list must carry every owned table");
        assert!(TABLES.contains(&"git_operations"), "{TABLES:?}");
    }

    /// FR-NEW-285: the six values round trip through their stored spelling.
    #[test]
    fn op_type_is_a_six_value_closed_set() {
        let spelled: Vec<&str> = GitOpType::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            spelled,
            vec!["merge", "rebase", "cherry_pick", "revert", "stash_apply", "stash_pop"]
        );
        for t in GitOpType::ALL {
            assert_eq!(GitOpType::parse(t.as_str()).unwrap(), t);
        }
        assert!(GitOpType::parse("pull").is_err(), "the set is closed");
    }

    /// FR-NEW-275: every declared field survives the round trip.
    #[tokio::test]
    async fn operation_row_round_trips() {
        let d = db().await;
        assert_eq!(d.get_operation().await.unwrap(), None, "nothing in progress at rest");
        let op = paused_rebase();
        d.set_operation(&op).await.unwrap();
        assert_eq!(d.get_operation().await.unwrap().unwrap(), op);
        assert_eq!(d.count_operations().await.unwrap(), 1);
    }

    /// FR-NEW-285: each of the six types round trips through the column.
    #[tokio::test]
    async fn every_op_type_round_trips_through_the_column() {
        let d = db().await;
        for t in GitOpType::ALL {
            let op = GitOperationRow { op_type: t, ..paused_rebase() };
            d.set_operation(&op).await.unwrap();
            assert_eq!(d.get_operation().await.unwrap().unwrap().op_type, t);
        }
    }

    /// FR-NEW-283: one row per volume, and every query scoped by `volume_id`.
    #[tokio::test]
    async fn one_operation_per_volume() {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open_in_memory().unwrap());
        let a = RelationalGitDb::open(db.clone(), "proj1").await.unwrap();
        let b = RelationalGitDb::open(db, "proj2").await.unwrap();

        a.set_operation(&paused_rebase()).await.unwrap();
        a.set_operation(&GitOperationRow {
            op_type: GitOpType::Merge,
            current_step: 2,
            ..paused_rebase()
        })
        .await
        .unwrap();
        assert_eq!(a.count_operations().await.unwrap(), 1, "the second write replaces the first");
        let got = a.get_operation().await.unwrap().unwrap();
        assert_eq!(got.op_type, GitOpType::Merge);
        assert_eq!(got.current_step, 2);

        assert_eq!(b.get_operation().await.unwrap(), None, "another volume is unaffected");
        b.set_operation(&paused_rebase()).await.unwrap();
        assert_eq!(a.get_operation().await.unwrap().unwrap().op_type, GitOpType::Merge);
    }

    /// E2E-NEW-620 at the store level (FR-NEW-284): abort clears the row, a second
    /// abort has nothing to clear, and a fresh operation pauses identically.
    #[tokio::test]
    async fn e2e_new_620_abort_clears_the_row() {
        let d = db().await;
        d.set_operation(&paused_rebase()).await.unwrap();
        assert_eq!(d.count_operations().await.unwrap(), 1);

        assert!(d.clear_operation().await.unwrap(), "the abort found a row");
        assert_eq!(d.count_operations().await.unwrap(), 0, "no row survives the abort");
        assert_eq!(d.get_operation().await.unwrap(), None);

        assert!(!d.clear_operation().await.unwrap(), "nothing left to abort");

        // The cleared row must not poison the next attempt.
        d.set_operation(&paused_rebase()).await.unwrap();
        let again = d.get_operation().await.unwrap().unwrap();
        assert_eq!(again, paused_rebase(), "the same todo pauses identically");
        assert_eq!(again.current_step, 1, "current_step still identifies F1");
    }

    /// FR-NEW-278: a paused operation is resumable after a restart.
    #[tokio::test]
    async fn paused_operation_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git").join("restart.db");
        let open = |p: &std::path::Path| {
            let db = crate::storage::rel::SqliteRelationalDb::open(p).unwrap();
            RelationalGitDb::open(Arc::new(db), "proj")
        };
        {
            let d = open(&path).await.unwrap();
            d.set_operation(&paused_rebase()).await.unwrap();
        }
        let d2 = open(&path).await.unwrap();
        let got = d2.get_operation().await.unwrap().unwrap();
        assert_eq!(got.current_step, 1, "current_step unchanged");
        assert_eq!(got.todo, paused_rebase().todo, "todo unchanged");
        assert_eq!(got.conflicts, paused_rebase().conflicts, "conflicts unchanged");
        assert_eq!(got, paused_rebase());
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
