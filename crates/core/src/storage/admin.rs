//! ACL registry: projects and their members, on any [`RelationalDb`].
//!
//! Unlike the metadata tree there is no `volume_id` here: the registry is global
//! to the deployment, one row per project and one per membership.
//!
//! All identity comparisons are caseless (`normalize_identity`), so a person
//! added as `Bob@Example.com` is the same member as `bob@example.com`.

use crate::errors::{Result, ToolError};
use crate::storage::rel::dialect::{Assign, ColumnType, Upsert};
use crate::storage::rel::schema::{Column, ColumnMigration, ForeignKey, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb, RowValues};
use crate::storage::traits::{AdminBackend, IndexMode, Member, Project, PurgeConfig};
use crate::util::{normalize_identity, now_iso};
use async_trait::async_trait;
use std::str::FromStr;
use std::sync::Arc;

/// A project id is at most 32 characters, an email comfortably under 320.
const PROJECT_ID_LEN: u32 = 64;
const PERSON_LEN: u32 = 320;

pub const ROLE_OWNER: &str = "owner";
pub const ROLE_MEMBER: &str = "member";

/// The tables this store owns.
pub fn schema() -> SchemaSet {
    SchemaSet::new(
        vec![
            Table::new(
                "project",
                vec![
                    Column::required("id", ColumnType::TextKey(PROJECT_ID_LEN)),
                    Column::required("owner", ColumnType::Text),
                    Column::required("created_at", ColumnType::Text),
                ],
                vec!["id"],
            ),
            Table::new(
                "project_member",
                vec![
                    Column::required("project_id", ColumnType::TextKey(PROJECT_ID_LEN)),
                    Column::required("person", ColumnType::TextKey(PERSON_LEN)),
                    Column::required("role", ColumnType::Text),
                    Column::required("added_by", ColumnType::Text),
                    Column::required("added_at", ColumnType::Text),
                ],
                vec!["project_id", "person"],
            )
            // Deleting a project takes its memberships with it, so the registry
            // cannot keep a membership pointing at a project that is gone.
            .foreign_key(ForeignKey {
                columns: vec!["project_id"],
                references_table: "project",
                references_columns: vec!["id"],
                on_delete_cascade: true,
            }),
            // One row per project that has ever had its auto purge behavior
            // configured. No `volume_id`: like `project` itself, this table is
            // global to the deployment, not scoped per volume (SPEC-0014 DEC-008
            // and the parent spec's Section 8 correction).
            Table::new(
                "project_purge_config",
                vec![
                    Column::required("project_id", ColumnType::TextKey(PROJECT_ID_LEN)),
                    Column::required("autopurge_enabled", ColumnType::BigInt).default("0"),
                    Column::required("use_internal_purge", ColumnType::BigInt).default("0"),
                    Column::new("file_retention_days", ColumnType::BigInt),
                    Column::new("project_retention_days", ColumnType::BigInt),
                ],
                vec!["project_id"],
            )
            .foreign_key(ForeignKey {
                columns: vec!["project_id"],
                references_table: "project",
                references_columns: vec!["id"],
                on_delete_cascade: true,
            }),
        ],
        Vec::new(),
    )
    // `project` shipped without this column, so a deployed database gains it here
    // rather than through the CREATE TABLE, which only runs on an empty database.
    .column_migration(ColumnMigration {
        table: "project",
        column: "index_mode",
        ty: ColumnType::Text,
        not_null: true,
        default: "'none'",
    })
    // Soft-delete marker for SPEC-0014: null for every existing and newly
    // created project until a later story starts writing it.
    .column_migration(ColumnMigration {
        table: "project",
        column: "deleted_at",
        ty: ColumnType::Text,
        not_null: false,
        default: "NULL",
    })
    // Storage quota in bytes for SPEC-0010: null (unlimited) for every existing
    // and newly created project until an admin sets one.
    .column_migration(ColumnMigration {
        table: "project",
        column: "quota_bytes",
        ty: ColumnType::BigInt,
        not_null: false,
        default: "NULL",
    })
}

pub struct RelationalAdminStore {
    db: Arc<dyn RelationalDb>,
}

impl RelationalAdminStore {
    /// The schema is applied by [`AdminBackend::connect`], which the composition
    /// root calls at startup, so constructing a store performs no IO.
    pub fn new(db: Arc<dyn RelationalDb>) -> Self {
        Self { db }
    }

    /// Connected in memory store, for tests.
    pub async fn in_memory() -> Result<Self> {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open_in_memory()?);
        let s = Self::new(db);
        s.connect().await?;
        Ok(s)
    }

    fn read_project(r: &RowValues) -> Result<Project> {
        Ok(Project {
            id: r.text(0)?,
            owner: r.text(1)?,
            created_at: r.text(2)?,
            index_mode: IndexMode::from_str(&r.text(3)?)?,
            quota_bytes: r.opt_i64(4)?,
        })
    }

    fn read_member(r: &RowValues) -> Result<Member> {
        Ok(Member {
            project_id: r.text(0)?,
            person: r.text(1)?,
            role: r.text(2)?,
            added_by: r.text(3)?,
            added_at: r.text(4)?,
        })
    }

    /// Excludes a soft-deleted row (`deleted_at` non-null). Used only by
    /// `require_member`, the single enforcement point for SPEC-0014's access
    /// gate; every other existence check in this store must keep seeing
    /// soft-deleted projects unchanged.
    async fn live_project_exists(&self, id: &str) -> Result<bool> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT 1 FROM project WHERE id=?1 AND deleted_at IS NULL").bind(id),
            )
            .await?;
        Ok(row.is_some())
    }
}

#[async_trait]
impl AdminBackend for RelationalAdminStore {
    async fn connect(&self) -> Result<()> {
        self.db.migrate(&schema()).await
    }

    async fn create_project(&self, project_id: &str, owner: &str) -> Result<Project> {
        let id = project_id.to_string();
        let owner = normalize_identity(owner);
        let mut tx = self.db.begin().await?;
        // Safe to retry in principle: a failed attempt rolls back, so the duplicate
        // check would re-read the original state. Left on an owned handle because
        // creating a project is a cold path where a serialization conflict is
        // vanishingly unlikely, and the sequential form reads better than a closure.
        let exists =
            tx.query_opt(&Query::new("SELECT 1 FROM project WHERE id=?1").bind(&id)).await?;
        if exists.is_some() {
            return Err(ToolError::project_exists(&id));
        }
        let now = now_iso();
        tx.execute(
            &Query::new("INSERT INTO project (id, owner, created_at) VALUES (?1, ?2, ?3)")
                .bind(&id)
                .bind(&owner)
                .bind(&now),
        )
        .await?;
        tx.execute(
            &Query::new(
                "INSERT INTO project_member (project_id, person, role, added_by, added_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&id)
            .bind(&owner)
            .bind(ROLE_OWNER)
            .bind(&owner)
            .bind(&now),
        )
        .await?;
        tx.commit().await?;
        Ok(Project { id, owner, created_at: now, index_mode: IndexMode::None, quota_bytes: None })
    }

    async fn delete_project(&self, project_id: &str) -> Result<()> {
        // Memberships cascade via the foreign key.
        self.db.execute(&Query::new("DELETE FROM project WHERE id=?1").bind(project_id)).await?;
        Ok(())
    }

    async fn add_member(&self, project_id: &str, person: &str, added_by: &str) -> Result<Member> {
        let id = project_id.to_string();
        let person = normalize_identity(person);
        let added_by = normalize_identity(added_by);

        // Same reasoning as create_project: idempotent, but a cold path, so the
        // owned handle is preferred over the retry helper.
        let mut tx = self.db.begin().await?;
        let exists =
            tx.query_opt(&Query::new("SELECT 1 FROM project WHERE id=?1").bind(&id)).await?;
        if exists.is_none() {
            return Err(ToolError::project_not_found(&id));
        }
        // Keep the role a member already has, so re-adding an owner never demotes
        // them to a plain member.
        let role = tx
            .query_opt(
                &Query::new("SELECT role FROM project_member WHERE project_id=?1 AND person=?2")
                    .bind(&id)
                    .bind(&person),
            )
            .await?
            .map(|r| r.text(0))
            .transpose()?
            .unwrap_or_else(|| ROLE_MEMBER.to_string());

        let now = now_iso();
        // Re-adding an existing member refreshes who added them, nothing else.
        let sql = self.db.dialect().render_upsert(&Upsert::update(
            "project_member",
            vec!["project_id", "person", "role", "added_by", "added_at"],
            vec!["project_id", "person"],
            vec![Assign::inserted("added_by")],
        ));
        tx.execute(&Query::new(sql).bind(&id).bind(&person).bind(&role).bind(&added_by).bind(&now))
            .await?;
        tx.commit().await?;
        Ok(Member { project_id: id, person, role, added_by, added_at: now })
    }

    async fn remove_member(&self, project_id: &str, person: &str) -> Result<()> {
        // The owner is never removable, which is why the role is part of the
        // predicate rather than checked separately.
        self.db
            .execute(
                &Query::new(
                    "DELETE FROM project_member \
                     WHERE project_id=?1 AND person=?2 AND role<>'owner'",
                )
                .bind(project_id)
                .bind(normalize_identity(person)),
            )
            .await?;
        Ok(())
    }

    async fn get_project(&self, project_id: &str) -> Result<Option<Project>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT id, owner, created_at, index_mode, quota_bytes FROM project WHERE id=?1",
                )
                .bind(project_id),
            )
            .await?;
        match row {
            Some(r) => Ok(Some(Self::read_project(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_projects_for(&self, person: &str) -> Result<Vec<Project>> {
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT p.id, p.owner, p.created_at, p.index_mode, p.quota_bytes FROM project p \
                     JOIN project_member m ON m.project_id = p.id \
                     WHERE m.person = ?1 ORDER BY p.id",
                )
                .bind(normalize_identity(person)),
            )
            .await?;
        rows.iter().map(Self::read_project).collect()
    }

    async fn list_all_projects(&self) -> Result<Vec<Project>> {
        let rows = self
            .db
            .query(&Query::new(
                "SELECT id, owner, created_at, index_mode, quota_bytes FROM project ORDER BY id",
            ))
            .await?;
        rows.iter().map(Self::read_project).collect()
    }

    async fn list_all_persons(&self) -> Result<Vec<String>> {
        let rows = self
            .db
            .query(&Query::new("SELECT DISTINCT person FROM project_member ORDER BY person"))
            .await?;
        rows.iter().map(|r| r.text(0)).collect()
    }

    async fn list_members(&self, project_id: &str) -> Result<Vec<Member>> {
        let rows = self
            .db
            .query(
                &Query::new(
                    "SELECT project_id, person, role, added_by, added_at \
                     FROM project_member WHERE project_id=?1 ORDER BY person",
                )
                .bind(project_id),
            )
            .await?;
        rows.iter().map(Self::read_member).collect()
    }

    async fn is_member(&self, project_id: &str, person: &str) -> Result<bool> {
        let row = self
            .db
            .query_opt(
                &Query::new("SELECT 1 FROM project_member WHERE project_id=?1 AND person=?2")
                    .bind(project_id)
                    .bind(normalize_identity(person)),
            )
            .await?;
        Ok(row.is_some())
    }

    async fn require_member(&self, project_id: &str, person: &str) -> Result<()> {
        // Soft-deleted is indistinguishable from absent here (SPEC-0014 DEC-009):
        // reuse ERR_PROJECT_NOT_FOUND rather than minting a new code. This is the
        // single enforcement point for every fs.*/git.* call, MCP and REST alike,
        // since both surfaces route through `authorize()` -> `require_member`.
        // `get_project`/`require_owner`/`require_owner_or_admin` must keep seeing
        // soft-deleted rows unchanged, so the filter lives only in this query.
        if !self.live_project_exists(project_id).await? {
            return Err(ToolError::project_not_found(project_id));
        }
        if !self.is_member(project_id, person).await? {
            return Err(ToolError::forbidden(format!(
                "'{person}' is not a member of '{project_id}'"
            )));
        }
        Ok(())
    }

    async fn require_owner(&self, project_id: &str, person: &str) -> Result<Project> {
        let p = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| ToolError::project_not_found(project_id))?;
        if p.owner != normalize_identity(person) {
            return Err(ToolError::forbidden(format!(
                "'{person}' is not the owner of '{project_id}'"
            )));
        }
        Ok(p)
    }

    async fn set_index_mode(&self, project_id: &str, mode: IndexMode) -> Result<()> {
        let affected = self
            .db
            .execute(
                &Query::new("UPDATE project SET index_mode=?1 WHERE id=?2")
                    .bind(mode.as_str())
                    .bind(project_id),
            )
            .await?;
        if affected == 0 {
            return Err(ToolError::project_not_found(project_id));
        }
        Ok(())
    }

    async fn get_index_mode(&self, project_id: &str) -> Result<IndexMode> {
        let row = self
            .db
            .query_opt(&Query::new("SELECT index_mode FROM project WHERE id=?1").bind(project_id))
            .await?
            .ok_or_else(|| ToolError::project_not_found(project_id))?;
        IndexMode::from_str(&row.text(0)?)
    }

    async fn get_purge_config(&self, project_id: &str) -> Result<PurgeConfig> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT autopurge_enabled, use_internal_purge, file_retention_days, \
                     project_retention_days FROM project_purge_config WHERE project_id=?1",
                )
                .bind(project_id),
            )
            .await?;
        match row {
            Some(r) => Ok(PurgeConfig {
                autopurge_enabled: r.i64(0)? != 0,
                use_internal_purge: r.i64(1)? != 0,
                file_retention_days: r.opt_i64(2)?,
                project_retention_days: r.opt_i64(3)?,
            }),
            None => Ok(PurgeConfig::default()),
        }
    }

    async fn set_purge_config(&self, project_id: &str, config: PurgeConfig) -> Result<()> {
        if self.get_project(project_id).await?.is_none() {
            return Err(ToolError::project_not_found(project_id));
        }
        let sql = self.db.dialect().render_upsert(&Upsert::replace(
            "project_purge_config",
            vec![
                "project_id",
                "autopurge_enabled",
                "use_internal_purge",
                "file_retention_days",
                "project_retention_days",
            ],
            vec!["project_id"],
        ));
        self.db
            .execute(
                &Query::new(sql)
                    .bind(project_id)
                    .bind(i64::from(config.autopurge_enabled))
                    .bind(i64::from(config.use_internal_purge))
                    .bind(config.file_retention_days)
                    .bind(config.project_retention_days),
            )
            .await?;
        Ok(())
    }

    async fn set_quota(&self, project_id: &str, quota_bytes: Option<i64>) -> Result<()> {
        let affected = self
            .db
            .execute(
                &Query::new("UPDATE project SET quota_bytes=?1 WHERE id=?2")
                    .bind(quota_bytes)
                    .bind(project_id),
            )
            .await?;
        if affected == 0 {
            return Err(ToolError::project_not_found(project_id));
        }
        Ok(())
    }

    async fn get_quota(&self, project_id: &str) -> Result<Option<i64>> {
        let row = self
            .db
            .query_opt(&Query::new("SELECT quota_bytes FROM project WHERE id=?1").bind(project_id))
            .await?
            .ok_or_else(|| ToolError::project_not_found(project_id))?;
        row.opt_i64(0)
    }

    async fn soft_delete_project(&self, project_id: &str) -> Result<bool> {
        let now = now_iso();
        let affected = self
            .db
            .execute(
                &Query::new(
                    "UPDATE project SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                )
                .bind(&now)
                .bind(project_id),
            )
            .await?;
        Ok(affected > 0)
    }

    async fn list_soft_deleted_projects(&self) -> Result<Vec<(String, String)>> {
        let rows = self
            .db
            .query(&Query::new(
                "SELECT id, deleted_at FROM project WHERE deleted_at IS NOT NULL ORDER BY id",
            ))
            .await?;
        rows.iter().map(|r| Ok((r.text(0)?, r.text(1)?))).collect()
    }

    async fn undelete_project(&self, project_id: &str) -> Result<()> {
        let deleted_at = self
            .db
            .query_opt(&Query::new("SELECT deleted_at FROM project WHERE id = ?1").bind(project_id))
            .await?
            .ok_or_else(|| ToolError::project_not_found(project_id))?
            .opt_text(0)?;
        if deleted_at.is_none() {
            return Err(ToolError::invalid_argument(format!(
                "project '{project_id}' is not soft-deleted"
            )));
        }
        self.db
            .execute(
                &Query::new("UPDATE project SET deleted_at = NULL WHERE id = ?1").bind(project_id),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
impl RelationalAdminStore {
    /// Direct seed of `project_purge_config`, bypassing every validation and
    /// authorization rule `admin.set_purge_config` will enforce (SPEC-0014
    /// US-0009, not yet implemented). Test-only: this story only needs a
    /// config row in place to sweep against.
    pub(crate) async fn seed_purge_config_for_test(
        &self,
        project_id: &str,
        autopurge_enabled: bool,
        use_internal_purge: bool,
        file_retention_days: Option<i64>,
        project_retention_days: Option<i64>,
    ) -> Result<()> {
        let sql = self.db.dialect().render_upsert(&Upsert::replace(
            "project_purge_config",
            vec![
                "project_id",
                "autopurge_enabled",
                "use_internal_purge",
                "file_retention_days",
                "project_retention_days",
            ],
            vec!["project_id"],
        ));
        self.db
            .execute(
                &Query::new(sql)
                    .bind(project_id)
                    .bind(i64::from(autopurge_enabled))
                    .bind(i64::from(use_internal_purge))
                    .bind(file_retention_days)
                    .bind(project_retention_days),
            )
            .await?;
        Ok(())
    }

    /// Direct overwrite of `project.created_at`, bypassing `create_project`'s
    /// `now_iso()` stamp. Test-only: lets a test pin a project's age without
    /// racing the wall clock (SPEC-0014 US-0006).
    pub(crate) async fn seed_created_at_for_test(
        &self,
        project_id: &str,
        created_at: &str,
    ) -> Result<()> {
        self.db
            .execute(
                &Query::new("UPDATE project SET created_at = ?1 WHERE id = ?2")
                    .bind(created_at)
                    .bind(project_id),
            )
            .await?;
        Ok(())
    }

    /// Direct overwrite of `project.deleted_at`, bypassing `soft_delete_project`'s
    /// `now_iso()` stamp. Test-only: lets a grace-sweep test pin a project's age
    /// since soft-delete without racing the wall clock (SPEC-0014 US-0008).
    pub(crate) async fn seed_deleted_at_for_test(
        &self,
        project_id: &str,
        deleted_at: &str,
    ) -> Result<()> {
        self.db
            .execute(
                &Query::new("UPDATE project SET deleted_at = ?1 WHERE id = ?2")
                    .bind(deleted_at)
                    .bind(project_id),
            )
            .await?;
        Ok(())
    }

    /// Reads `project.deleted_at` directly, bypassing `require_member`'s gate.
    /// Test-only: lets a test assert on the raw value, including equality across
    /// two sweeps (SPEC-0014 US-0006 idempotency).
    pub(crate) async fn deleted_at_for_test(&self, project_id: &str) -> Result<Option<String>> {
        let row = self
            .db
            .query_opt(&Query::new("SELECT deleted_at FROM project WHERE id = ?1").bind(project_id))
            .await?
            .ok_or_else(|| ToolError::project_not_found(project_id))?;
        row.opt_text(0)
    }
}

/// Project id rule: 3 to 32 chars, lowercase letters/digits/hyphens, alphanumeric
/// bounds. Matches the regex `^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$`.
pub fn validate_project_id(id: &str) -> Result<()> {
    let ok = id.len() >= 3
        && id.len() <= 32
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().last().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(ToolError::invalid_argument(
            "project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> RelationalAdminStore {
        RelationalAdminStore::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_project_adds_owner_membership() {
        let s = store().await;
        let p = s.create_project("proj", "Alice@Test.COM").await.unwrap();
        assert_eq!(p.id, "proj");
        assert_eq!(p.owner, "alice@test.com", "owner is normalized");
        assert!(!p.created_at.is_empty());

        let members = s.list_members("proj").await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].person, "alice@test.com");
        assert_eq!(members[0].role, ROLE_OWNER);
    }

    #[tokio::test]
    async fn duplicate_project_is_rejected() {
        let s = store().await;
        s.create_project("proj", "a@t.c").await.unwrap();
        let e = s.create_project("proj", "b@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_EXISTS);
    }

    #[tokio::test]
    async fn membership_is_caseless() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();
        s.add_member("proj", "Bob@Test.COM", "owner@t.c").await.unwrap();
        assert!(s.is_member("proj", "bob@test.com").await.unwrap());
        assert!(s.is_member("proj", "BOB@TEST.COM").await.unwrap());
        assert!(!s.is_member("proj", "carol@t.c").await.unwrap());
    }

    #[tokio::test]
    async fn require_member_distinguishes_missing_from_forbidden() {
        let s = store().await;
        let e = s.require_member("nope", "a@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);

        s.create_project("proj", "owner@t.c").await.unwrap();
        let e = s.require_member("proj", "stranger@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::FORBIDDEN);
        assert!(e.message.contains("is not a member of 'proj'"));

        s.require_member("proj", "owner@t.c").await.unwrap();
    }

    /// SPEC-0014 US-0004 FR-NEW-013: a soft-deleted project (`deleted_at` set)
    /// is indistinguishable from an absent one to `require_member`, including
    /// for its owner — no implicit access for anyone, platform admin included
    /// (enforced one layer up in `state.rs`, this just makes the row invisible).
    #[tokio::test]
    async fn require_member_rejects_soft_deleted_project_as_not_found() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();
        s.add_member("proj", "member@t.c", "owner@t.c").await.unwrap();

        s.db.execute(
            &Query::new("UPDATE project SET deleted_at = ?1 WHERE id = ?2")
                .bind("2026-10-04T00:00:00Z")
                .bind("proj"),
        )
        .await
        .unwrap();

        let e = s.require_member("proj", "member@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);

        // The owner gets no bypass either.
        let e = s.require_member("proj", "owner@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);

        // get_project/require_owner must keep seeing the row unchanged.
        assert!(s.get_project("proj").await.unwrap().is_some());
        s.require_owner("proj", "owner@t.c").await.unwrap();
    }

    /// Closes the loop: the gate reads live `deleted_at` on every call, never a
    /// cached value, so clearing it (simulating undelete) immediately restores
    /// access without reconnecting or recreating the store.
    #[tokio::test]
    async fn require_member_re_reads_live_state_after_undelete() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();

        s.db.execute(
            &Query::new("UPDATE project SET deleted_at = ?1 WHERE id = ?2")
                .bind("2026-10-04T00:00:00Z")
                .bind("proj"),
        )
        .await
        .unwrap();
        let e = s.require_member("proj", "owner@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);

        s.db.execute(
            &Query::new("UPDATE project SET deleted_at = NULL WHERE id = ?1").bind("proj"),
        )
        .await
        .unwrap();
        s.require_member("proj", "owner@t.c").await.unwrap();
    }

    #[tokio::test]
    async fn require_owner_checks_ownership() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();
        s.add_member("proj", "member@t.c", "owner@t.c").await.unwrap();

        s.require_owner("proj", "owner@t.c").await.unwrap();
        let e = s.require_owner("proj", "member@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::FORBIDDEN);
        assert!(e.message.contains("is not the owner"));
    }

    #[tokio::test]
    async fn list_projects_for_only_returns_own_projects() {
        let s = store().await;
        s.create_project("mine", "me@t.c").await.unwrap();
        s.create_project("theirs", "them@t.c").await.unwrap();

        let mine: Vec<String> =
            s.list_projects_for("me@t.c").await.unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(mine, vec!["mine"]);

        let all: Vec<String> =
            s.list_all_projects().await.unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(all, vec!["mine", "theirs"]);
    }

    #[tokio::test]
    async fn delete_project_cascades_members() {
        let s = store().await;
        s.create_project("proj", "o@t.c").await.unwrap();
        s.add_member("proj", "m@t.c", "o@t.c").await.unwrap();
        s.delete_project("proj").await.unwrap();
        assert!(s.get_project("proj").await.unwrap().is_none());
        assert!(s.list_members("proj").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_member_cannot_remove_owner() {
        let s = store().await;
        s.create_project("proj", "o@t.c").await.unwrap();
        s.remove_member("proj", "o@t.c").await.unwrap();
        assert!(s.is_member("proj", "o@t.c").await.unwrap(), "owner stays a member");
    }

    #[tokio::test]
    async fn add_member_to_missing_project_errors() {
        let s = store().await;
        let e = s.add_member("nope", "a@t.c", "b@t.c").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);
    }

    #[tokio::test]
    async fn list_all_persons_is_distinct_and_sorted() {
        let s = store().await;
        s.create_project("p1", "a@t.c").await.unwrap();
        s.create_project("p2", "a@t.c").await.unwrap();
        s.add_member("p1", "b@t.c", "a@t.c").await.unwrap();
        assert_eq!(s.list_all_persons().await.unwrap(), vec!["a@t.c", "b@t.c"]);
    }

    #[tokio::test]
    async fn a_new_project_defaults_to_index_mode_none() {
        let s = store().await;
        let p = s.create_project("proj", "o@t.c").await.unwrap();
        assert_eq!(p.index_mode, IndexMode::None);
        assert_eq!(s.get_index_mode("proj").await.unwrap(), IndexMode::None);
        assert_eq!(s.get_project("proj").await.unwrap().unwrap().index_mode, IndexMode::None);
    }

    #[tokio::test]
    async fn index_mode_round_trips_through_the_store() {
        let s = store().await;
        s.create_project("proj", "o@t.c").await.unwrap();
        for mode in [IndexMode::Bm25, IndexMode::Rag, IndexMode::Both, IndexMode::None] {
            s.set_index_mode("proj", mode).await.unwrap();
            assert_eq!(s.get_index_mode("proj").await.unwrap(), mode);
            // Every read path must agree, not just the dedicated getter.
            assert_eq!(s.get_project("proj").await.unwrap().unwrap().index_mode, mode);
            assert_eq!(s.list_all_projects().await.unwrap()[0].index_mode, mode);
            assert_eq!(s.list_projects_for("o@t.c").await.unwrap()[0].index_mode, mode);
        }
    }

    #[tokio::test]
    async fn index_mode_on_a_missing_project_is_not_found() {
        let s = store().await;
        assert_eq!(
            s.get_index_mode("ghost").await.unwrap_err().code,
            crate::errors::code::PROJECT_NOT_FOUND
        );
        assert_eq!(
            s.set_index_mode("ghost", IndexMode::Bm25).await.unwrap_err().code,
            crate::errors::code::PROJECT_NOT_FOUND
        );
    }

    /// The real case the `ColumnMigration` exists for: a database created before
    /// the column shipped. A fresh `connect()` would prove nothing, because
    /// `CREATE TABLE` already carries every column, so the old table is built by
    /// hand here.
    #[tokio::test]
    async fn column_migration_adds_index_mode_to_an_old_database() {
        use crate::storage::rel::SqliteRelationalDb;

        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        // The `project` table exactly as it shipped, without `index_mode`.
        db.execute(&Query::new(
            "CREATE TABLE \"project\" (\"id\" TEXT NOT NULL, \"owner\" TEXT NOT NULL, \
             \"created_at\" TEXT NOT NULL, PRIMARY KEY (\"id\"))",
        ))
        .await
        .unwrap();
        db.execute(
            &Query::new("INSERT INTO project (id, owner, created_at) VALUES (?1, ?2, ?3)")
                .bind("legacy")
                .bind("o@t.c")
                .bind("2020-01-01T00:00:00Z"),
        )
        .await
        .unwrap();

        let s = RelationalAdminStore::new(db);
        s.connect().await.unwrap();

        let p = s.get_project("legacy").await.unwrap().expect("the legacy row survives");
        assert_eq!(p.index_mode, IndexMode::None, "an existing row defaults to none");

        // Idempotent: connect runs on every open, so a second one must not fail.
        s.connect().await.unwrap();
        assert_eq!(s.get_index_mode("legacy").await.unwrap(), IndexMode::None);

        // And the column is really usable afterwards.
        s.set_index_mode("legacy", IndexMode::Both).await.unwrap();
        assert_eq!(s.get_index_mode("legacy").await.unwrap(), IndexMode::Both);
    }

    /// E2E-coverage for SPEC-0014 US-0002: a fresh database gets the new
    /// `project_purge_config` table and `project.deleted_at` column, both
    /// unused by any production code yet, so this exercises the schema layer
    /// directly with raw SQL rather than through a store method.
    #[tokio::test]
    async fn fresh_schema_includes_project_purge_config_and_deleted_at() {
        let s = store().await;
        s.create_project("proj", "o@t.c").await.unwrap();

        // `deleted_at` exists, is nullable, and defaults to NULL.
        let row = s
            .db
            .query_opt(&Query::new("SELECT deleted_at FROM project WHERE id = ?1").bind("proj"))
            .await
            .unwrap()
            .expect("the project row exists");
        assert_eq!(row.opt_text(0).unwrap(), None, "deleted_at starts out null");

        // `project_purge_config` exists, is insertable, and FK-cascades with the project.
        s.db.execute(
            &Query::new(
                "INSERT INTO project_purge_config \
                     (project_id, autopurge_enabled, use_internal_purge, \
                      file_retention_days, project_retention_days) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind("proj")
            .bind(1_i64)
            .bind(0_i64)
            .bind(14_i64)
            .bind(Option::<i64>::None),
        )
        .await
        .unwrap();

        let row =
            s.db.query_opt(
                &Query::new(
                    "SELECT autopurge_enabled, use_internal_purge, file_retention_days, \
                     project_retention_days FROM project_purge_config WHERE project_id = ?1",
                )
                .bind("proj"),
            )
            .await
            .unwrap()
            .expect("the purge config row exists");
        assert_eq!(row.i64(0).unwrap(), 1);
        assert_eq!(row.i64(1).unwrap(), 0);
        assert_eq!(row.i64(2).unwrap(), 14);
        assert_eq!(*row.value(3).unwrap(), crate::storage::rel::SqlValue::Null);

        s.delete_project("proj").await.unwrap();
        let remaining =
            s.db.query(&Query::new("SELECT project_id FROM project_purge_config")).await.unwrap();
        assert!(remaining.is_empty(), "deleting the project cascades its purge config");
    }

    /// Same precedent as `column_migration_adds_index_mode_to_an_old_database`:
    /// a database created before `deleted_at` shipped must gain it via
    /// `ALTER TABLE`, not just on a fresh `CREATE TABLE`.
    #[tokio::test]
    async fn column_migration_adds_deleted_at_to_an_old_database() {
        use crate::storage::rel::SqliteRelationalDb;

        let db = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        db.execute(&Query::new(
            "CREATE TABLE \"project\" (\"id\" TEXT NOT NULL, \"owner\" TEXT NOT NULL, \
             \"created_at\" TEXT NOT NULL, PRIMARY KEY (\"id\"))",
        ))
        .await
        .unwrap();
        db.execute(
            &Query::new("INSERT INTO project (id, owner, created_at) VALUES (?1, ?2, ?3)")
                .bind("legacy")
                .bind("o@t.c")
                .bind("2020-01-01T00:00:00Z"),
        )
        .await
        .unwrap();

        let s = RelationalAdminStore::new(db);
        s.connect().await.unwrap();

        let row = s
            .db
            .query_opt(&Query::new("SELECT deleted_at FROM project WHERE id = ?1").bind("legacy"))
            .await
            .unwrap()
            .expect("the legacy row survives");
        assert_eq!(row.opt_text(0).unwrap(), None, "a migrated row starts out not deleted");

        // Idempotent: connect runs on every open, so a second one must not fail.
        s.connect().await.unwrap();
    }

    #[tokio::test]
    async fn undelete_project_clears_deleted_at() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();
        assert!(s.soft_delete_project("proj").await.unwrap());

        s.undelete_project("proj").await.unwrap();

        let row = s
            .db
            .query_opt(&Query::new("SELECT deleted_at FROM project WHERE id = ?1").bind("proj"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.opt_text(0).unwrap(), None);
    }

    #[tokio::test]
    async fn undelete_project_rejects_already_live_project() {
        let s = store().await;
        s.create_project("proj", "owner@t.c").await.unwrap();

        let e = s.undelete_project("proj").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn undelete_project_rejects_unknown_project() {
        let s = store().await;

        let e = s.undelete_project("ghost").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_NOT_FOUND);
    }

    #[test]
    fn project_id_validation_boundaries() {
        assert!(validate_project_id("abc").is_ok());
        assert!(validate_project_id(&"a".repeat(32)).is_ok());
        assert!(validate_project_id("a-b").is_ok());
        assert!(validate_project_id("a1-2b").is_ok());

        assert!(validate_project_id("ab").is_err(), "2 chars too short");
        assert!(validate_project_id(&"a".repeat(33)).is_err(), "33 chars too long");
        assert!(validate_project_id("-abc").is_err(), "leading hyphen");
        assert!(validate_project_id("abc-").is_err(), "trailing hyphen");
        assert!(validate_project_id("Abc").is_err(), "uppercase");
        assert!(validate_project_id("a_c").is_err(), "underscore");
        assert!(validate_project_id("a c").is_err(), "space");
    }

    #[test]
    fn project_id_error_message_is_stable() {
        let e = validate_project_id("ab").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert_eq!(
            e.message,
            "project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds"
        );
    }
}
