//! ACL registry: projects and their members, on any [`RelationalDb`].
//!
//! Unlike the metadata tree there is no `volume_id` here: the registry is global
//! to the deployment, one row per project and one per membership.
//!
//! All identity comparisons are caseless (`normalize_identity`), so a person
//! added as `Bob@Example.com` is the same member as `bob@example.com`.

use crate::errors::{Result, ToolError};
use crate::storage::rel::dialect::{Assign, ColumnType, Upsert};
use crate::storage::rel::schema::{Column, ForeignKey, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb, RowValues};
use crate::storage::traits::{AdminBackend, Member, Project};
use crate::util::{normalize_identity, now_iso};
use async_trait::async_trait;
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
        ],
        Vec::new(),
    )
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
        Ok(Project { id: r.text(0)?, owner: r.text(1)?, created_at: r.text(2)? })
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

    async fn project_exists(&self, id: &str) -> Result<bool> {
        let row = self
            .db
            .query_opt(&Query::new("SELECT 1 FROM project WHERE id=?1").bind(id))
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
        let exists = tx
            .query_opt(&Query::new("SELECT 1 FROM project WHERE id=?1").bind(&id))
            .await?;
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
        Ok(Project { id, owner, created_at: now })
    }

    async fn delete_project(&self, project_id: &str) -> Result<()> {
        // Memberships cascade via the foreign key.
        self.db
            .execute(&Query::new("DELETE FROM project WHERE id=?1").bind(project_id))
            .await?;
        Ok(())
    }

    async fn add_member(&self, project_id: &str, person: &str, added_by: &str) -> Result<Member> {
        let id = project_id.to_string();
        let person = normalize_identity(person);
        let added_by = normalize_identity(added_by);

        // Same reasoning as create_project: idempotent, but a cold path, so the
        // owned handle is preferred over the retry helper.
        let mut tx = self.db.begin().await?;
        let exists = tx
            .query_opt(&Query::new("SELECT 1 FROM project WHERE id=?1").bind(&id))
            .await?;
        if exists.is_none() {
            return Err(ToolError::project_not_found(&id));
        }
        // Keep the role a member already has, so re-adding an owner never demotes
        // them to a plain member.
        let role = tx
            .query_opt(
                &Query::new(
                    "SELECT role FROM project_member WHERE project_id=?1 AND person=?2",
                )
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
        tx.execute(
            &Query::new(sql)
                .bind(&id)
                .bind(&person)
                .bind(&role)
                .bind(&added_by)
                .bind(&now),
        )
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
                &Query::new("SELECT id, owner, created_at FROM project WHERE id=?1")
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
                    "SELECT p.id, p.owner, p.created_at FROM project p \
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
            .query(&Query::new("SELECT id, owner, created_at FROM project ORDER BY id"))
            .await?;
        rows.iter().map(Self::read_project).collect()
    }

    async fn list_all_persons(&self) -> Result<Vec<String>> {
        let rows = self
            .db
            .query(&Query::new(
                "SELECT DISTINCT person FROM project_member ORDER BY person",
            ))
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
                &Query::new(
                    "SELECT 1 FROM project_member WHERE project_id=?1 AND person=?2",
                )
                .bind(project_id)
                .bind(normalize_identity(person)),
            )
            .await?;
        Ok(row.is_some())
    }

    async fn require_member(&self, project_id: &str, person: &str) -> Result<()> {
        if !self.project_exists(project_id).await? {
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
}

/// Project id rule: 3 to 32 chars, lowercase letters/digits/hyphens, alphanumeric
/// bounds. Matches the regex `^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$`.
pub fn validate_project_id(id: &str) -> Result<()> {
    let ok = id.len() >= 3
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
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

    /// Boundary table from the spec: 3 ok, 32 ok, 33 rejected, bad bounds rejected.
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
