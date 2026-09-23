//! Backend parametric conformance suite: one set of assertions, every engine.
//!
//! The per store test modules cover their own logic against SQLite. This suite
//! exists for a different question: does a store behave the SAME on another
//! engine? So every case here takes an [`Engine`] and is run once per backend.
//!
//! SQLite always runs. PostgreSQL runs only when `MCPFS_TEST_PG_DSN` is set and
//! SQL Server only when `MCPFS_TEST_MSSQL_DSN` is set, so `cargo test` stays green
//! with no database and no Docker:
//!
//! ```text
//! docker compose -f docker-compose.test.yml up -d
//! MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs \
//! MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' \
//!   cargo test --all-features conformance
//! ```
//!
//! Isolation differs per engine and that is the point. SQLite hands out a private
//! in memory database per call, while the server engines share one instance, so
//! every case derives unique ids from a per run tag. A case that passes on all
//! three has been proven not to depend on having the database to itself.

use crate::errors::Result;
use crate::git::db::RelationalGitDb;
use crate::git::oauth::cipher;
use crate::git::oauth::persistence::RelationalOAuthPersistence;
use crate::git::oauth::store::OAuthSession;
use crate::storage::admin::{ROLE_OWNER, RelationalAdminStore};
use crate::storage::meta::RelationalMetaStore;
use crate::storage::rel::{RelationalDb, SqliteRelationalDb};
use crate::storage::traits::{AdminBackend, IndexMode, MODE_FILE, MetaBackend};
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// Which engine a case runs against.
enum Engine {
    Sqlite,
    Postgres(String),
    SqlServer(String),
}

impl Engine {
    fn name(&self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres(_) => "postgres",
            Self::SqlServer(_) => "sqlserver",
        }
    }

    /// A connection to run one case against.
    ///
    /// SQLite gets a fresh private database, which is the strongest isolation
    /// available. PostgreSQL returns a pool onto the shared server, so cases must
    /// isolate themselves by id rather than by database.
    async fn db(&self) -> Result<Arc<dyn RelationalDb>> {
        match self {
            Self::Sqlite => Ok(Arc::new(SqliteRelationalDb::open_in_memory()?)),
            #[cfg(feature = "postgres")]
            Self::Postgres(dsn) => {
                let db = crate::storage::rel::PostgresRelationalDb::connect(
                    dsn,
                    "public",
                    crate::storage::rel::PoolSettings {
                        max_connections: 4,
                        acquire_timeout: std::time::Duration::from_secs(10),
                    },
                )
                .await?;
                Ok(Arc::new(db))
            }
            #[cfg(not(feature = "postgres"))]
            // The length, never the value: a DSN carries a password.
            Self::Postgres(dsn) => Err(crate::errors::ToolError::not_supported(format!(
                "the postgres feature is not compiled in, so the {} character dsn is unusable",
                dsn.len()
            ))),
            #[cfg(feature = "sqlserver")]
            Self::SqlServer(dsn) => {
                let db = crate::storage::rel::SqlServerRelationalDb::connect(
                    dsn,
                    crate::storage::rel::PoolSettings {
                        max_connections: 4,
                        acquire_timeout: std::time::Duration::from_secs(10),
                    },
                )
                .await?;
                Ok(Arc::new(db))
            }
            #[cfg(not(feature = "sqlserver"))]
            Self::SqlServer(dsn) => Err(crate::errors::ToolError::not_supported(format!(
                "the sqlserver feature is not compiled in, so the {} character dsn is unusable",
                dsn.len()
            ))),
        }
    }
}

/// Unique per run, so a shared PostgreSQL server does not see ids left behind by
/// an earlier run.
fn run_tag() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

fn session(token: &str) -> OAuthSession {
    OAuthSession {
        provider: "github".into(),
        access_token: token.into(),
        scopes: vec!["repo".into()],
        expires_at: Some(
            DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z").unwrap().with_timezone(&Utc),
        ),
        instance_url: None,
    }
}

// ── metadata cases ──────────────────────────────────────────────────────────────

async fn meta_tree_basics(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let m = RelationalMetaStore::open(engine.db().await?, format!("{tag}-basics")).await?;

    let root = m.get("/").await?.expect("root node exists");
    assert_eq!(root.kind, "dir", "{who}: root is a directory");
    assert_eq!(root.parent, None, "{who}: root has no parent");

    m.put_file("/a/b/c.txt", Some("sha-c"), 3, MODE_FILE).await?;
    assert!(m.get("/a").await?.expect("parent created").is_dir(), "{who}: parents are created");
    let f = m.get("/a/b/c.txt").await?.expect("file present");
    assert_eq!(f.size, 3, "{who}: size round trips");
    assert_eq!(f.sha256.as_deref(), Some("sha-c"), "{who}: sha round trips");
    assert_eq!(f.parent.as_deref(), Some("/a/b"), "{who}: parent is derived");

    // mtime is a float column, the one type SQLite stores loosely.
    assert!(f.mtime > 0.0, "{who}: mtime is a real number");

    for p in ["/z.txt", "/x.txt", "/y.txt"] {
        m.put_file(p, Some(p), 1, MODE_FILE).await?;
    }
    let names: Vec<String> = m.list_children("/").await?.into_iter().map(|n| n.name).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "{who}: children come back sorted by name");
    Ok(())
}

async fn meta_refcount_and_gc(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let m = RelationalMetaStore::open(engine.db().await?, format!("{tag}-refs")).await?;

    m.put_file("/a.txt", Some("dup"), 4, MODE_FILE).await?;
    m.put_file("/b.txt", Some("dup"), 4, MODE_FILE).await?;
    assert_eq!(m.delete_file("/a.txt").await?, None, "{who}: refcount 2 keeps the blob");
    assert_eq!(
        m.delete_file("/b.txt").await?.as_deref(),
        Some("dup"),
        "{who}: refcount 0 releases the blob"
    );

    // An overwrite releases the replaced blob and preserves ctime.
    m.put_file("/c.txt", Some("old"), 1, MODE_FILE).await?;
    let ctime = m.get("/c.txt").await?.expect("present").ctime;
    assert_eq!(
        m.put_file("/c.txt", Some("new"), 1, MODE_FILE).await?.as_deref(),
        Some("old"),
        "{who}: the replaced blob is released"
    );
    assert_eq!(
        m.get("/c.txt").await?.expect("present").ctime,
        ctime,
        "{who}: ctime survives an overwrite"
    );

    // An empty file stores no blob at all.
    m.put_file("/e.txt", None, 0, MODE_FILE).await?;
    assert_eq!(m.delete_file("/e.txt").await?, None, "{who}: no blob, nothing to release");
    Ok(())
}

/// The bug this port fixed: an unescaped `LIKE` prefix matched sibling rows, so a
/// subtree read, delete or rename reached paths it did not own.
async fn meta_like_escaping(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let m = RelationalMetaStore::open(engine.db().await?, format!("{tag}-like")).await?;

    m.put_file("/a_b/in.txt", Some("in1"), 1, MODE_FILE).await?;
    m.put_file("/axb/out.txt", Some("out1"), 1, MODE_FILE).await?;
    m.put_file("/50%/in.txt", Some("in2"), 1, MODE_FILE).await?;
    m.put_file("/50off/out.txt", Some("out2"), 1, MODE_FILE).await?;

    let under: Vec<String> = m.subtree("/a_b").await?.into_iter().map(|n| n.path).collect();
    assert!(under.contains(&"/a_b/in.txt".to_string()), "{who}: own child is listed");
    assert!(
        !under.contains(&"/axb/out.txt".to_string()),
        "{who}: '_' must match itself, not any character: {under:?}"
    );

    let gc = m.remove_subtree("/50%").await?;
    assert_eq!(gc, vec!["in2"], "{who}: only the owned blob is released");
    assert!(
        m.get("/50off/out.txt").await?.is_some(),
        "{who}: '%' must match itself, not any prefix"
    );

    m.rename("/a_b", "/moved").await?;
    assert!(m.get("/moved/in.txt").await?.is_some(), "{who}: the subtree moved");
    assert!(
        m.get("/axb/out.txt").await?.is_some(),
        "{who}: a sibling must not be dragged along by a rename"
    );
    Ok(())
}

async fn meta_rename_deep_tree(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let m = RelationalMetaStore::open(engine.db().await?, format!("{tag}-rename")).await?;
    m.put_file("/a/b/c/d/deep.txt", Some("deep"), 1, MODE_FILE).await?;

    // Ordering matters: a child renamed before its parent would be orphaned. The
    // ORDER BY uses the dialect's length function, which is what this proves.
    m.rename("/a", "/z").await?;
    assert!(m.get("/z/b/c/d/deep.txt").await?.is_some(), "{who}: the whole tree moved");
    assert!(m.get("/a").await?.is_none(), "{who}: nothing is left behind");
    assert_eq!(
        m.get("/z/b/c/d").await?.expect("present").parent.as_deref(),
        Some("/z/b/c"),
        "{who}: parents are rewritten"
    );
    Ok(())
}

/// Two volumes in ONE database must not see each other. On SQLite they are
/// separate files, so this only really bites on a shared engine.
/// 25 writers race on one volume, all referencing one blob.
///
/// The refcount is a read modify write, so a lost update shows up here as a count
/// below 25 and nowhere else. On PostgreSQL and SQL Server these writers can
/// genuinely deadlock or lose a serialization race, which is what exercises
/// [`crate::storage::rel::run_retrying`]; on SQLite the single writer lock
/// serializes them instead. Either way the observable outcome must be identical,
/// which is the whole point of running this per engine.
async fn meta_concurrent_writes(engine: &Engine, tag: &str) -> Result<()> {
    const WRITERS: i64 = 25;
    let who = engine.name();
    let store =
        Arc::new(RelationalMetaStore::open(engine.db().await?, format!("{tag}-race")).await?);
    let sha = format!("{tag}-shared-sha");

    let mut tasks = Vec::with_capacity(WRITERS as usize);
    for i in 0..WRITERS {
        let store = store.clone();
        let sha = sha.clone();
        tasks.push(tokio::spawn(async move {
            store.put_file(&format!("/race/{i}.txt"), Some(&sha), 1, MODE_FILE).await
        }));
    }
    for t in tasks {
        t.await
            .map_err(|e| crate::errors::ToolError::internal(format!("{who}: task join: {e}")))??;
    }

    // Every writer landed: no attempt was silently dropped.
    let children = store.list_children("/race").await?;
    assert_eq!(children.len(), WRITERS as usize, "{who}: every concurrent write must be visible");

    // This delete sequence IS the refcount assertion, and it needs no accessor on
    // the store. A lost update would leave the count below 25, so one of the first
    // 24 deletes would release the blob early and trip the `None` check. A double
    // increment would leave it above 25, so the final delete would not release it.
    // Only an exact count of 25 satisfies both halves.
    for i in 0..WRITERS - 1 {
        let gc = store.delete_file(&format!("/race/{i}.txt")).await?;
        assert_eq!(
            gc, None,
            "{who}: the blob is still referenced, so a concurrent increment was lost"
        );
    }
    let gc = store.delete_file(&format!("/race/{}.txt", WRITERS - 1)).await?;
    assert_eq!(
        gc,
        Some(sha),
        "{who}: the last reference must release the blob, so no increment was double counted"
    );
    Ok(())
}

async fn meta_volume_isolation(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let a = RelationalMetaStore::open(db.clone(), format!("{tag}-iso-a")).await?;
    let b = RelationalMetaStore::open(db, format!("{tag}-iso-b")).await?;

    a.put_file("/same.txt", Some("shared-blob"), 1, MODE_FILE).await?;
    b.put_file("/same.txt", Some("shared-blob"), 1, MODE_FILE).await?;

    assert_eq!(a.list_children("/").await?.len(), 1, "{who}: volume a sees one child");
    assert_eq!(b.list_children("/").await?.len(), 1, "{who}: volume b sees one child");

    // Refcounts are per volume, so each volume releases its own blob.
    assert_eq!(
        a.delete_file("/same.txt").await?.as_deref(),
        Some("shared-blob"),
        "{who}: volume a releases its own reference"
    );
    assert!(b.get("/same.txt").await?.is_some(), "{who}: volume b is untouched");

    a.put_file("/d/x.txt", Some("ax"), 1, MODE_FILE).await?;
    b.put_file("/d/x.txt", Some("bx"), 1, MODE_FILE).await?;
    a.remove_subtree("/d").await?;
    assert!(b.get("/d/x.txt").await?.is_some(), "{who}: a subtree removal must not cross volumes");
    Ok(())
}

// ── ACL cases ───────────────────────────────────────────────────────────────────

async fn admin_projects_and_members(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let store = RelationalAdminStore::new(engine.db().await?);
    store.connect().await?;
    let proj = format!("{tag}-acl");

    let p = store.create_project(&proj, "Alice@Test.COM").await?;
    assert_eq!(p.owner, "alice@test.com", "{who}: the owner is normalized");
    let members = store.list_members(&proj).await?;
    assert_eq!(members.len(), 1, "{who}: creating a project adds its owner");
    assert_eq!(members[0].role, ROLE_OWNER, "{who}: that member is the owner");

    assert_eq!(
        store.create_project(&proj, "bob@test.com").await.unwrap_err().code,
        crate::errors::code::PROJECT_EXISTS,
        "{who}: a duplicate project is refused"
    );

    store.add_member(&proj, "Bob@Test.COM", "alice@test.com").await?;
    assert!(store.is_member(&proj, "bob@test.com").await?, "{who}: membership is caseless");
    assert!(store.is_member(&proj, "BOB@TEST.COM").await?, "{who}: membership is caseless");
    assert!(!store.is_member(&proj, "carol@test.com").await?, "{who}: a stranger is not a member");

    store.require_member(&proj, "bob@test.com").await?;
    assert_eq!(
        store.require_member(&proj, "carol@test.com").await.unwrap_err().code,
        crate::errors::code::FORBIDDEN,
        "{who}: a non member is forbidden"
    );
    store.require_owner(&proj, "alice@test.com").await?;
    assert_eq!(
        store.require_owner(&proj, "bob@test.com").await.unwrap_err().code,
        crate::errors::code::FORBIDDEN,
        "{who}: a member is not the owner"
    );

    // Re-adding the owner must not demote them.
    store.add_member(&proj, "alice@test.com", "alice@test.com").await?;
    store.require_owner(&proj, "alice@test.com").await?;

    // The owner is not removable, a plain member is.
    store.remove_member(&proj, "alice@test.com").await?;
    assert!(store.is_member(&proj, "alice@test.com").await?, "{who}: the owner stays");
    store.remove_member(&proj, "bob@test.com").await?;
    assert!(!store.is_member(&proj, "bob@test.com").await?, "{who}: a member is removable");

    // Deleting the project cascades its memberships.
    store.delete_project(&proj).await?;
    assert!(store.get_project(&proj).await?.is_none(), "{who}: the project is gone");
    assert!(
        store.list_members(&proj).await?.is_empty(),
        "{who}: memberships cascade with the project"
    );
    Ok(())
}

/// The `index_mode` column arrives through a `ColumnMigration`, which every
/// engine guards differently. Applying the schema twice must stay a no op, and
/// the column must be readable and writable afterwards on all of them.
async fn admin_column_migration_is_idempotent(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let store = RelationalAdminStore::new(db.clone());
    store.connect().await?;
    // A store calls connect on every open, so the second apply is the real case.
    store.connect().await?;

    let proj = format!("{tag}-mode");
    let created = store.create_project(&proj, "owner@test.com").await?;
    assert_eq!(created.index_mode, IndexMode::None, "{who}: a new project starts at none");
    assert_eq!(
        store.get_index_mode(&proj).await?,
        IndexMode::None,
        "{who}: the column default is none"
    );

    for mode in [IndexMode::Bm25, IndexMode::Rag, IndexMode::Both, IndexMode::None] {
        store.set_index_mode(&proj, mode).await?;
        assert_eq!(store.get_index_mode(&proj).await?, mode, "{who}: {mode} round trips");
        let p = store.get_project(&proj).await?.expect("the project exists");
        assert_eq!(p.index_mode, mode, "{who}: get_project agrees with get_index_mode");
    }

    // A third apply, now that the column holds data, must not disturb it.
    store.set_index_mode(&proj, IndexMode::Both).await?;
    store.connect().await?;
    assert_eq!(
        store.get_index_mode(&proj).await?,
        IndexMode::Both,
        "{who}: re-migrating must not reset the stored mode"
    );

    store.delete_project(&proj).await?;
    Ok(())
}

// ── git index cases ─────────────────────────────────────────────────────────────

/// Purging one project's git index must not touch another's.
///
/// Under SQLite the index is a file and deleting it is enough, but on a shared
/// database it is a set of rows: without a `volume_id` filter a purge would wipe
/// every project, and without a purge at all a project recreated under the same id
/// would inherit the old refs. Both failures are silent, so they are pinned here.
async fn git_purge_is_scoped_to_one_volume(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let doomed = format!("{tag}-git-doomed");
    let keeper = format!("{tag}-git-keeper");

    let a = RelationalGitDb::open(db.clone(), &doomed).await?;
    let b = RelationalGitDb::open(db.clone(), &keeper).await?;

    for g in [&a, &b] {
        g.record_object("cafe01", "blob", 3).await?;
        g.set_ref("refs/heads/main", "cafe01", false).await?;
        g.add_remote("origin", "https://example.invalid/r.git").await?;
        g.set_operation(&paused_operation()).await?;
    }

    // What `purge_git_rows` runs, against the engine under test.
    let mut tx = db.begin().await?;
    for table in crate::git::db::TABLES {
        tx.execute(
            &crate::storage::rel::Query::new(format!("DELETE FROM {table} WHERE volume_id=?1"))
                .bind(&doomed),
        )
        .await?;
    }
    tx.commit().await?;

    assert_eq!(a.count_objects().await?, 0, "{who}: the purged index is empty");
    assert!(a.list_refs().await?.is_empty(), "{who}: the purged refs are gone");
    assert!(a.list_remotes().await?.is_empty(), "{who}: the purged remotes are gone");
    assert_eq!(a.count_operations().await?, 0, "{who}: the purged operation row is gone");

    assert_eq!(b.count_objects().await?, 1, "{who}: another project keeps its objects");
    assert_eq!(b.list_refs().await?.len(), 1, "{who}: another project keeps its refs");
    assert_eq!(b.list_remotes().await?.len(), 1, "{who}: another project keeps its remotes");
    assert_eq!(
        b.count_operations().await?,
        1,
        "{who}: another project keeps its in-progress operation"
    );

    // A project recreated under the purged id starts empty, which is the defect
    // the purge exists to prevent.
    let reborn = RelationalGitDb::open(db, &doomed).await?;
    assert_eq!(
        reborn.count_objects().await?,
        0,
        "{who}: a recreated project must not inherit the old object index"
    );
    assert!(
        reborn.list_refs().await?.is_empty(),
        "{who}: a recreated project must not inherit the old refs"
    );
    Ok(())
}

/// A paused rebase, the shape every engine must store and return unchanged.
fn paused_operation() -> crate::git::db::GitOperationRow {
    crate::git::db::GitOperationRow {
        op_type: crate::git::db::GitOpType::Rebase,
        state: "conflicted".into(),
        source_ref: Some("feature".into()),
        onto_sha: Some("onto1".into()),
        original_tip_sha: Some("tip1".into()),
        todo: Some(r#"[{"sha":"F1"}]"#.into()),
        current_step: 1,
        total_steps: 2,
        conflicts: Some(r#"["/a.txt"]"#.into()),
        resolutions: None,
        created_at: "2026-09-22T10:00:00Z".into(),
        updated_at: "2026-09-22T10:00:01Z".into(),
    }
}

/// FR-NEW-277: the operation row is stored and read back identically on every
/// engine, including the unbounded payload columns and the nullable ones.
async fn git_operation_row_round_trips(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let g = RelationalGitDb::open(db, format!("{tag}-git-op")).await?;

    assert_eq!(g.get_operation().await?, None, "{who}: nothing in progress at rest");
    let op = paused_operation();
    g.set_operation(&op).await?;
    assert_eq!(g.get_operation().await?.as_ref(), Some(&op), "{who}: every field round trips");

    // One row per volume: the second write replaces, it does not insert.
    let moved = crate::git::db::GitOperationRow { current_step: 2, ..op };
    g.set_operation(&moved).await?;
    assert_eq!(g.count_operations().await?, 1, "{who}: at most one operation per volume");
    assert_eq!(g.get_operation().await?.unwrap().current_step, 2, "{who}: the last write wins");

    assert!(g.clear_operation().await?, "{who}: the abort found a row");
    assert_eq!(g.count_operations().await?, 0, "{who}: completion clears the row");
    assert!(!g.clear_operation().await?, "{who}: nothing left to clear");
    Ok(())
}

async fn git_objects_refs_remotes(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let g = RelationalGitDb::open(db.clone(), format!("{tag}-git")).await?;

    g.record_object("abcd01", "blob", 7).await?;
    assert!(g.object_exists("abcd01").await?, "{who}: the object is indexed");
    assert_eq!(
        g.get_object("abcd01").await?.expect("present").size,
        7,
        "{who}: the size round trips"
    );

    // record_object is an upsert, so re-indexing replaces rather than duplicates.
    g.record_object("abcd01", "commit", 42).await?;
    let row = g.get_object("abcd01").await?.expect("present");
    assert_eq!(row.kind, "commit", "{who}: an upsert overwrites the type");
    assert_eq!(row.size, 42, "{who}: an upsert overwrites the size");

    g.record_object("abcd02", "blob", 1).await?;
    g.record_object("ffff03", "blob", 1).await?;
    assert_eq!(
        g.find_objects_by_prefix("abcd").await?,
        vec!["abcd01", "abcd02"],
        "{who}: a short sha resolves by prefix"
    );

    g.set_ref("refs/heads/main", "1111", false).await?;
    g.set_ref("HEAD", "refs/heads/main", true).await?;
    let head = g.get_ref("HEAD").await?.expect("HEAD present");
    assert!(head.symbolic, "{who}: a symbolic ref round trips as symbolic");
    assert_eq!(head.target, "refs/heads/main", "{who}: the target round trips");

    // A ref can flip from symbolic to direct, and stays one row.
    g.set_ref("HEAD", "2222", false).await?;
    let head = g.get_ref("HEAD").await?.expect("HEAD present");
    assert!(!head.symbolic, "{who}: symbolic flips off");
    let names: Vec<String> = g.list_refs().await?.into_iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["HEAD", "refs/heads/main"], "{who}: refs are ordered by name");

    g.delete_ref("HEAD").await?;
    assert!(g.get_ref("HEAD").await?.is_none(), "{who}: a deleted ref is gone");
    g.delete_ref("HEAD").await?;

    g.add_remote("origin", "https://example.test/a.git").await?;
    g.add_remote("origin", "https://example.test/b.git").await?;
    let remotes = g.list_remotes().await?;
    assert_eq!(remotes.len(), 1, "{who}: adding a remote twice is an upsert");
    assert_eq!(remotes[0].1, "https://example.test/b.git", "{who}: the url is replaced");
    g.remove_remote("origin").await?;
    assert!(g.list_remotes().await?.is_empty(), "{who}: the remote is gone");

    // A second project in the same database must not see the first one's index.
    let other = RelationalGitDb::open(db, format!("{tag}-git-other")).await?;
    assert!(!other.object_exists("abcd01").await?, "{who}: the git index is scoped to its project");
    assert_eq!(other.count_objects().await?, 0, "{who}: a fresh project starts empty");
    assert_eq!(g.count_objects().await?, 3, "{who}: the original still has its objects");
    Ok(())
}

// ── OAuth persistence case ──────────────────────────────────────────────────────

async fn oauth_round_trip(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let key = [7u8; cipher::KEY_SIZE];
    let p = RelationalOAuthPersistence::open(engine.db().await?, key).await?;
    let person = format!("{tag}@test.com");
    let host = "github.com";

    p.upsert(&person, host, &session("gho_secret")).await?;
    let all = p.load_all().await?;
    let mine = all.iter().find(|(who_, _, _)| who_ == &person).expect("the row we just wrote");
    assert_eq!(mine.2.access_token, "gho_secret", "{who}: the token decrypts");
    assert_eq!(mine.2.scopes, vec!["repo"], "{who}: scopes round trip");
    assert_eq!(
        mine.2.expires_at.unwrap().to_rfc3339(),
        "2030-01-01T00:00:00+00:00",
        "{who}: the expiry round trips"
    );

    // The upsert replaces in place: one row per (person, host).
    p.upsert(&person, host, &session("gho_rotated")).await?;
    let all = p.load_all().await?;
    let count = all.iter().filter(|(w, _, _)| w == &person).count();
    assert_eq!(count, 1, "{who}: the primary key is (person, host)");
    let mine = all.iter().find(|(w, _, _)| w == &person).expect("present");
    assert_eq!(mine.2.access_token, "gho_rotated", "{who}: the token was replaced");

    p.delete(&person, host).await?;
    let all = p.load_all().await?;
    assert!(!all.iter().any(|(w, _, _)| w == &person), "{who}: the row is deleted");
    p.delete(&person, host).await?;
    Ok(())
}

/// DRIFT-005: a live deployment still on the legacy `(person, provider)` shape
/// is rebuilt on the new `(person, host)` key identically on every engine, not
/// just on SQLite. Starts by dropping whatever a previous case in this same run
/// left behind, so it is safe to re-run against a shared server (PostgreSQL,
/// SQL Server) rather than assuming a database it has to itself.
async fn oauth_legacy_table_is_rebuilt_identically(engine: &Engine, tag: &str) -> Result<()> {
    let who = engine.name();
    let db = engine.db().await?;
    let d = db.dialect();

    db.execute(&crate::storage::rel::Query::new(format!(
        "DROP TABLE IF EXISTS {}",
        d.quote_ident("oauth_tokens")
    )))
    .await?;

    // Recreate on the OLD shape, as a live pre-upgrade deployment would have it.
    let person_ty = d.column_type(crate::storage::rel::ColumnType::TextKey(320));
    let provider_ty = d.column_type(crate::storage::rel::ColumnType::TextKey(64));
    let text_ty = d.column_type(crate::storage::rel::ColumnType::Text);
    let blob_ty = d.column_type(crate::storage::rel::ColumnType::Blob);
    let ddl = format!(
        "CREATE TABLE {} ({} {person_ty} NOT NULL, {} {provider_ty} NOT NULL, \
         {} {blob_ty} NOT NULL, {} {text_ty} NOT NULL, {} {text_ty} NOT NULL, \
         {} {text_ty}, PRIMARY KEY ({}, {}))",
        d.quote_ident("oauth_tokens"),
        d.quote_ident("person"),
        d.quote_ident("provider"),
        d.quote_ident("token_enc"),
        d.quote_ident("scopes"),
        d.quote_ident("expires_at"),
        d.quote_ident("instance_url"),
        d.quote_ident("person"),
        d.quote_ident("provider"),
    );
    db.execute(&crate::storage::rel::Query::new(ddl)).await?;

    let person = format!("{tag}@test.com");
    db.execute(
        &crate::storage::rel::Query::new(
            "INSERT INTO oauth_tokens \
             (person, provider, token_enc, scopes, expires_at, instance_url) \
             VALUES (?1, 'github', ?2, 'repo', '2030-01-01T00:00:00Z', NULL)",
        )
        .bind(&person)
        .bind(vec![0u8; 10]),
    )
    .await?;

    let key = [9u8; cipher::KEY_SIZE];
    let p = RelationalOAuthPersistence::open(db, key).await?;
    assert_eq!(p.count().await?, 0, "{who}: the legacy row was dropped on rebuild");

    // the rebuilt table works normally afterward
    p.upsert(&person, "github.ibm.com", &session("rebuilt")).await?;
    let all = p.load_all().await?;
    assert_eq!(all.len(), 1, "{who}: the new shape accepts writes");
    assert_eq!(all[0].1, "github.ibm.com", "{who}: keyed by host now");
    Ok(())
}

// ── the suite ───────────────────────────────────────────────────────────────────

async fn run_suite(engine: &Engine) -> Result<()> {
    let tag = run_tag();
    meta_tree_basics(engine, &tag).await?;
    meta_refcount_and_gc(engine, &tag).await?;
    meta_like_escaping(engine, &tag).await?;
    meta_rename_deep_tree(engine, &tag).await?;
    meta_volume_isolation(engine, &tag).await?;
    meta_concurrent_writes(engine, &tag).await?;
    admin_projects_and_members(engine, &tag).await?;
    admin_column_migration_is_idempotent(engine, &tag).await?;
    git_objects_refs_remotes(engine, &tag).await?;
    git_operation_row_round_trips(engine, &tag).await?;
    git_purge_is_scoped_to_one_volume(engine, &tag).await?;
    oauth_round_trip(engine, &tag).await?;
    oauth_legacy_table_is_rebuilt_identically(engine, &tag).await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_passes_the_conformance_suite() {
    run_suite(&Engine::Sqlite).await.expect("sqlite must satisfy every case");
}

/// Skipped, not failed, when `MCPFS_TEST_PG_DSN` is unset: the default
/// `cargo test` run must not need a database.
#[tokio::test]
async fn postgres_passes_the_conformance_suite() {
    let Ok(dsn) = std::env::var("MCPFS_TEST_PG_DSN") else {
        eprintln!("skipping the postgres conformance suite: MCPFS_TEST_PG_DSN is unset");
        return;
    };
    if !cfg!(feature = "postgres") {
        eprintln!("skipping the postgres conformance suite: built without --features postgres");
        return;
    }
    run_suite(&Engine::Postgres(dsn))
        .await
        .expect("postgres must satisfy exactly the same cases as sqlite");
}

/// Skipped, not failed, when `MCPFS_TEST_MSSQL_DSN` is unset: the default
/// `cargo test` run must not need a database.
#[tokio::test]
async fn sqlserver_passes_the_conformance_suite() {
    let Ok(dsn) = std::env::var("MCPFS_TEST_MSSQL_DSN") else {
        eprintln!("skipping the sqlserver conformance suite: MCPFS_TEST_MSSQL_DSN is unset");
        return;
    };
    if !cfg!(feature = "sqlserver") {
        eprintln!("skipping the sqlserver conformance suite: built without --features sqlserver");
        return;
    }
    run_suite(&Engine::SqlServer(dsn))
        .await
        .expect("sqlserver must satisfy exactly the same cases as sqlite");
}
