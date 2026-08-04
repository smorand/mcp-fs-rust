//! SQLite ACL registry: projects and their members. 1:1 port of the C#
//! `Storage/SqliteAdminStore.cs`, same schema so the db is interchangeable.
//!
//! All identity comparisons are caseless (`normalize_identity`).

use crate::errors::{Result, ToolError};
use crate::storage::sqlite::SqliteDb;
use crate::storage::traits::{AdminBackend, Member, Project};
use crate::util::{normalize_identity, now_iso};
use async_trait::async_trait;
use rusqlite::Row;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS project (
        id         TEXT PRIMARY KEY,
        owner      TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS project_member (
        project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
        person     TEXT NOT NULL,
        role       TEXT NOT NULL,
        added_by   TEXT NOT NULL,
        added_at   TEXT NOT NULL,
        PRIMARY KEY (project_id, person)
    );
";

pub const ROLE_OWNER: &str = "owner";
pub const ROLE_MEMBER: &str = "member";

pub struct SqliteAdminStore {
    path: PathBuf,
    db: OnceLock<SqliteDb>,
    in_memory: bool,
}

impl SqliteAdminStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf(), db: OnceLock::new(), in_memory: false }
    }

    /// Connected in-memory store, for tests.
    pub fn in_memory() -> Result<Self> {
        let s = Self { path: PathBuf::new(), db: OnceLock::new(), in_memory: true };
        let db = SqliteDb::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        let _ = s.db.set(db);
        Ok(s)
    }

    fn store(&self) -> Result<&SqliteDb> {
        self.db.get().ok_or_else(|| ToolError::internal("admin store is not connected"))
    }

    fn read_project(r: &Row<'_>) -> rusqlite::Result<Project> {
        Ok(Project { id: r.get(0)?, owner: r.get(1)?, created_at: r.get(2)? })
    }

    fn read_member(r: &Row<'_>) -> rusqlite::Result<Member> {
        Ok(Member {
            project_id: r.get(0)?,
            person: r.get(1)?,
            role: r.get(2)?,
            added_by: r.get(3)?,
            added_at: r.get(4)?,
        })
    }
}

#[async_trait]
impl AdminBackend for SqliteAdminStore {
    async fn connect(&self) -> Result<()> {
        if self.db.get().is_some() {
            return Ok(());
        }
        if self.in_memory {
            return Ok(());
        }
        let db = SqliteDb::open(&self.path)?;
        db.execute_batch(SCHEMA)?;
        let _ = self.db.set(db);
        Ok(())
    }

    async fn create_project(&self, project_id: &str, owner: &str) -> Result<Project> {
        let id = project_id.to_string();
        let owner = normalize_identity(owner);
        self.store()?
            .run(move |tx| {
                let exists: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM project WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if exists > 0 {
                    return Err(ToolError::project_exists(&id));
                }
                let now = now_iso();
                tx.execute(
                    "INSERT INTO project(id, owner, created_at) VALUES(?1,?2,?3)",
                    (&id, &owner, &now),
                )?;
                tx.execute(
                    "INSERT INTO project_member(project_id, person, role, added_by, added_at)
                     VALUES(?1,?2,?3,?4,?5)",
                    (&id, &owner, ROLE_OWNER, &owner, &now),
                )?;
                Ok(Project { id, owner, created_at: now })
            })
            .await
    }

    async fn delete_project(&self, project_id: &str) -> Result<()> {
        let id = project_id.to_string();
        self.store()?
            .run(move |tx| {
                // members cascade via the foreign key
                tx.execute("DELETE FROM project WHERE id=?1", [&id])?;
                Ok(())
            })
            .await
    }

    async fn add_member(&self, project_id: &str, person: &str, added_by: &str) -> Result<Member> {
        let id = project_id.to_string();
        let person = normalize_identity(person);
        let added_by = normalize_identity(added_by);
        self.store()?
            .run(move |tx| {
                let exists: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM project WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if exists == 0 {
                    return Err(ToolError::project_not_found(&id));
                }
                let now = now_iso();
                // Never demote the owner to member.
                let role: String = tx
                    .query_row(
                        "SELECT role FROM project_member WHERE project_id=?1 AND person=?2",
                        (&id, &person),
                        |r| r.get(0),
                    )
                    .unwrap_or_else(|_| ROLE_MEMBER.to_string());
                tx.execute(
                    "INSERT INTO project_member(project_id, person, role, added_by, added_at)
                     VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(project_id, person) DO UPDATE SET added_by=excluded.added_by",
                    (&id, &person, &role, &added_by, &now),
                )?;
                Ok(Member {
                    project_id: id,
                    person,
                    role,
                    added_by,
                    added_at: now,
                })
            })
            .await
    }

    async fn remove_member(&self, project_id: &str, person: &str) -> Result<()> {
        let id = project_id.to_string();
        let person = normalize_identity(person);
        self.store()?
            .run(move |tx| {
                tx.execute(
                    "DELETE FROM project_member WHERE project_id=?1 AND person=?2 AND role<>'owner'",
                    (&id, &person),
                )?;
                Ok(())
            })
            .await
    }

    async fn get_project(&self, project_id: &str) -> Result<Option<Project>> {
        let id = project_id.to_string();
        self.store()?
            .run(move |tx| {
                let mut st =
                    tx.prepare("SELECT id, owner, created_at FROM project WHERE id=?1")?;
                let mut rows = st.query([&id])?;
                match rows.next()? {
                    Some(r) => Ok(Some(SqliteAdminStore::read_project(r)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn list_projects_for(&self, person: &str) -> Result<Vec<Project>> {
        let person = normalize_identity(person);
        self.store()?
            .run(move |tx| {
                let mut st = tx.prepare(
                    "SELECT p.id, p.owner, p.created_at FROM project p
                     JOIN project_member m ON m.project_id = p.id
                     WHERE m.person = ?1 ORDER BY p.id",
                )?;
                let out = st
                    .query_map([&person], SqliteAdminStore::read_project)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn list_all_projects(&self) -> Result<Vec<Project>> {
        self.store()?
            .run(|tx| {
                let mut st =
                    tx.prepare("SELECT id, owner, created_at FROM project ORDER BY id")?;
                let out = st
                    .query_map([], SqliteAdminStore::read_project)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn list_all_persons(&self) -> Result<Vec<String>> {
        self.store()?
            .run(|tx| {
                let mut st =
                    tx.prepare("SELECT DISTINCT person FROM project_member ORDER BY person")?;
                let out = st
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn list_members(&self, project_id: &str) -> Result<Vec<Member>> {
        let id = project_id.to_string();
        self.store()?
            .run(move |tx| {
                let mut st = tx.prepare(
                    "SELECT project_id, person, role, added_by, added_at
                     FROM project_member WHERE project_id=?1 ORDER BY person",
                )?;
                let out = st
                    .query_map([&id], SqliteAdminStore::read_member)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }

    async fn is_member(&self, project_id: &str, person: &str) -> Result<bool> {
        let id = project_id.to_string();
        let person = normalize_identity(person);
        self.store()?
            .run(move |tx| {
                let n: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM project_member WHERE project_id=?1 AND person=?2",
                    (&id, &person),
                    |r| r.get(0),
                )?;
                Ok(n > 0)
            })
            .await
    }

    async fn require_member(&self, project_id: &str, person: &str) -> Result<()> {
        if self.get_project(project_id).await?.is_none() {
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

/// Project id rule, identical to the C# regex `^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$`:
/// 3 to 32 chars, lowercase letters/digits/hyphens, alphanumeric bounds.
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

    async fn store() -> SqliteAdminStore {
        let s = SqliteAdminStore::in_memory().unwrap();
        s.connect().await.unwrap();
        s
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
    fn project_id_error_message_matches_csharp() {
        let e = validate_project_id("ab").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert_eq!(
            e.message,
            "project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds"
        );
    }
}
