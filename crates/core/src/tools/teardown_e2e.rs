//! SPEC-0024 acceptance tests (US-0025): deleting a project on a shared
//! database removes every metadata row of its volume, so a project recreated
//! under the same id inherits nothing (CWE-226).
//!
//! Each test owns its database scope: a fresh PostgreSQL schema, or a fresh SQL
//! Server database created beside `master`, so the literal `atlas` id of the
//! spec never collides with another suite running in parallel. The ACL registry
//! stays in memory. Every outcome is read twice: through the tools a user
//! calls, and on a second channel (a direct `COUNT(*)`, or the bytes read
//! back). Tests skip, never fail, when the DSN is unset or the feature is off.

use crate::config::{Dsn, ServerConfig, backend};
use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
use crate::errors::Result;
use crate::identity::IdentityResolver;
use crate::mcp::server::{
    CreateProjectArgs, DeleteArgs, DocumentizeArgs, ExportZipArgs, McpServer, ReadBytesArgs,
    WriteBytesArgs,
};
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::StoreManager;
use crate::storage::rel::{Query, RelationalDb};
use crate::storage::traits::AdminBackend;
use crate::tools::registry_support::{Args, ToolCtx, ToolRegistry};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt as _;

const ADA: &str = "ada@example.com";
const OLIVIA: &str = "olivia@example.com";
const OTTO: &str = "otto@example.com";
const ATLAS: &str = "atlas";
const ATLAS2: &str = "atlas2";
/// The image bytes the spec names `IMG-A`.
const IMG_A: &[u8] = b"IMG-A";

#[derive(Clone, Copy)]
enum Engine {
    Postgres,
    SqlServer,
}

/// A server whose metadata lives in a private scope of a shared database, with
/// `ADA` as platform admin and `atlas` owned by Olivia.
struct Fixture {
    _dir: tempfile::TempDir,
    state: Arc<AppState>,
    registry: ToolRegistry,
    /// A direct handle on the same scope, the second channel the counts read.
    db: Arc<dyn RelationalDb>,
    engine: Engine,
    dsn: String,
    scope: String,
}

impl Fixture {
    /// `None` when the test must skip, after saying why.
    async fn new(engine: Engine, test: &str, doc: Option<Conversion>) -> Option<Self> {
        let (var, feature, enabled) = match engine {
            Engine::Postgres => ("MCPFS_TEST_PG_DSN", "postgres", cfg!(feature = "postgres")),
            Engine::SqlServer => ("MCPFS_TEST_MSSQL_DSN", "sqlserver", cfg!(feature = "sqlserver")),
        };
        let Some(dsn) = std::env::var(var).ok().filter(|v| !v.trim().is_empty()) else {
            eprintln!("skipping {test}: {var} is unset");
            return None;
        };
        if !enabled {
            eprintln!("skipping {test}: built without --features {feature}");
            return None;
        }
        let scope = format!("mcpfs_teardown_{test}_{}", std::process::id());
        // A run that panicked leaves its scope behind; start from nothing.
        drop_scope(engine, &dsn, &scope).await;
        let (db, scoped_dsn) = open_scope(engine, &dsn, &scope).await;

        let dir = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.infra.meta.backend = match engine {
            Engine::Postgres => backend::POSTGRES,
            Engine::SqlServer => backend::SQLSERVER,
        }
        .into();
        config.infra.meta.dsn = Dsn::new(scoped_dsn);
        config.infra.meta.schema = match engine {
            Engine::Postgres => scope.clone(),
            Engine::SqlServer => String::new(),
        };
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.admins = vec![ADA.to_string()];
        config.auth.jwt.public_key_path = String::new();
        let config = Arc::new(config);

        let admin =
            Arc::new(crate::storage::admin::RelationalAdminStore::in_memory().await.unwrap());
        admin.connect().await.unwrap();
        let safety = Arc::new(SafetyManager::new(
            config.safety.clone(),
            crate::storage::meta::max_path_len(&config.infra.meta.backend),
        ));
        safety.attach_limits(admin.clone());
        let doc_service = doc.map(|c| Arc::new(StubDocService::bundle(c)) as _);
        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(StoreManager::new(config.clone(), crate::storage::test_registry())),
            safety,
            identity: Arc::new(IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service,
            search: None,
        });
        let mut registry = ToolRegistry::new();
        super::register_fs(&mut registry);
        super::admin::register(&mut registry);
        let f = Self { _dir: dir, state, registry, db, engine, dsn, scope };
        f.create(ATLAS, OLIVIA).await;
        Some(f)
    }

    fn server(&self, person: &str) -> McpServer {
        McpServer::new(self.state.clone(), person.to_string())
    }

    /// Dispatch a tool as `person`, like `tools/call` does.
    async fn call(&self, person: &str, name: &str, args: Value) -> Result<Value> {
        let ctx = ToolCtx { person: person.to_string(), state: self.state.clone() };
        self.registry
            .call(name, ctx, Args::new(args))
            .await
            .unwrap_or_else(|| panic!("tool '{name}' is not registered"))
    }

    /// `admin.create_project` as the platform admin.
    async fn create(&self, project: &str, owner: &str) {
        let args = CreateProjectArgs {
            project_id: project.into(),
            owner: owner.into(),
            autopurge_enabled: false,
            use_internal_purge: false,
            file_retention_days: None,
            project_retention_days: None,
        };
        ok_json(self.server(ADA).admin_create_project(Parameters(args)).await);
    }

    /// `admin.delete_project` as the platform admin: the real teardown path.
    async fn delete(&self, project: &str) {
        let out = self.call(ADA, "admin.delete_project", json!({"project_id": project})).await;
        assert_eq!(out.unwrap(), json!({"project_id": project, "deleted": true}));
    }

    async fn write(&self, person: &str, project: &str, path: &str, bytes: &[u8]) {
        let args = WriteBytesArgs {
            mount_id: project.into(),
            path: path.into(),
            base64: b64(bytes),
            overwrite: true,
            create_parents: true,
            trigger_documentation_service: false,
        };
        ok_json(self.server(person).fs_write_bytes(Parameters(args)).await);
    }

    async fn remove(&self, person: &str, project: &str, path: &str) {
        let args = DeleteArgs {
            mount_id: project.into(),
            path: path.into(),
            recursive: false,
            trash: true,
        };
        ok_json(self.server(person).fs_delete(Parameters(args)).await);
    }

    async fn convert(&self, person: &str, project: &str, path: &str) {
        let args = DocumentizeArgs { mount_id: project.into(), path: path.into(), overwrite: true };
        ok_json(self.server(person).fs_documentize(Parameters(args)).await);
    }

    async fn export(&self, person: &str, project: &str, paths: &[&str]) -> String {
        let args = ExportZipArgs {
            mount_id: project.into(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        };
        let out = ok_json(self.server(person).fs_export_zip(Parameters(args)).await);
        out["url"].as_str().expect("export url").to_string()
    }

    async fn trash_list(&self, person: &str, project: &str) -> Value {
        self.call(person, "fs.trash_list", json!({"mount_id": project})).await.unwrap()
    }

    /// `SELECT COUNT(*) FROM {table} WHERE volume_id=...`, on the database itself.
    async fn count(&self, table: &str, volume: &str) -> i64 {
        let sql = format!("SELECT CAST(COUNT(*) AS BIGINT) FROM {table} WHERE volume_id=?1");
        let row = self.db.query_opt(&Query::new(sql).bind(volume)).await.unwrap();
        row.expect("COUNT(*) yields one row").i64(0).unwrap()
    }

    async fn cleanup(self) {
        let Self { state, db, engine, dsn, scope, .. } = self;
        // SQL Server refuses to drop a database a pooled connection still holds.
        drop(state);
        drop(db);
        drop_scope(engine, &dsn, &scope).await;
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn text_of(r: &CallToolResult) -> String {
    r.content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text content block")
}

fn ok_json(r: std::result::Result<CallToolResult, rmcp::ErrorData>) -> Value {
    let r = r.expect("a tool result, not a protocol error");
    assert_ne!(r.is_error, Some(true), "expected success, got: {}", text_of(&r));
    serde_json::from_str(&text_of(&r)).expect("the tool result is JSON")
}

/// The SPEC-0019 stub conversion: one picture whose bytes are `IMG-A`, and
/// one table so `doc_tables` holds a row too.
fn memo_conversion() -> Conversion {
    let target = "media/org.png";
    let meta = PictureMeta { caption: "Org chart".into(), caption_failed: false, page: Some(1) };
    Conversion {
        markdown: format!("# Memo\n\n![chart]({target})\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n"),
        files: [(target.to_string(), IMG_A.to_vec())].into_iter().collect(),
        manifest: Some(Manifest { pictures: [(target.to_string(), meta)].into_iter().collect() }),
    }
}

/// The BT-001 state before deletion: `/plan.txt` stored, `/memo.txt` deleted by
/// Olivia, so `trash_entries` holds `original_path: "/memo.txt"`,
/// `deleted_by: "olivia@example.com"`.
async fn seed_trash(f: &Fixture, project: &str) {
    f.write(OLIVIA, project, "/plan.txt", b"merger with Borealis").await;
    f.write(OLIVIA, project, "/memo.txt", b"confidential").await;
    f.remove(OLIVIA, project, "/memo.txt").await;
    let listed = f.trash_list(OLIVIA, project).await;
    assert_eq!(listed["total"], json!(1), "the leaked state must exist first: {listed}");
    assert_eq!(listed["entries"][0]["original_path"], json!("/memo.txt"), "got: {listed}");
    assert_eq!(listed["entries"][0]["deleted_by"], json!(OLIVIA), "got: {listed}");
}

/// Every metadata table with a `volume_id` column, read from the schema rather
/// than from the function under test, so a table the teardown forgets fails
/// here and a table added later is covered without editing this test.
async fn counts(f: &Fixture, volume: &str) -> Vec<(&'static str, i64)> {
    let mut out = Vec::new();
    for table in crate::storage::meta::schema().tables {
        if table.columns.iter().any(|c| c.name == "volume_id") {
            out.push((table.name, f.count(table.name, volume).await));
        }
    }
    out
}

#[cfg(feature = "postgres")]
async fn drop_scope(engine: Engine, dsn: &str, scope: &str) {
    if let Engine::Postgres = engine {
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        let db =
            PostgresRelationalDb::connect(dsn, "public", PoolSettings::default()).await.unwrap();
        let sql = format!("DROP SCHEMA IF EXISTS \"{scope}\" CASCADE");
        db.execute(&Query::new(sql)).await.unwrap();
        return;
    }
    drop_database(dsn, scope).await;
}

#[cfg(not(feature = "postgres"))]
async fn drop_scope(_engine: Engine, dsn: &str, scope: &str) {
    drop_database(dsn, scope).await;
}

#[cfg(feature = "sqlserver")]
async fn drop_database(dsn: &str, name: &str) {
    use crate::storage::rel::{PoolSettings, SqlServerRelationalDb};
    let db = SqlServerRelationalDb::connect(dsn, PoolSettings::default()).await.unwrap();
    // SINGLE_USER rolls back and closes any connection a dropped pool left open.
    let sql = format!(
        "IF DB_ID(N'{name}') IS NOT NULL BEGIN \
         ALTER DATABASE [{name}] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; \
         DROP DATABASE [{name}]; END"
    );
    db.execute(&Query::new(sql)).await.unwrap();
}

#[cfg(not(feature = "sqlserver"))]
async fn drop_database(_dsn: &str, _name: &str) {
    // Unreachable: the fixture skips before any database exists without the feature.
}

/// Creates the scope and returns a handle on it plus the DSN the server uses.
async fn open_scope(engine: Engine, dsn: &str, scope: &str) -> (Arc<dyn RelationalDb>, String) {
    match engine {
        Engine::Postgres => open_pg(dsn, scope).await,
        Engine::SqlServer => open_mssql(dsn, scope).await,
    }
}

#[cfg(feature = "postgres")]
async fn open_pg(dsn: &str, schema: &str) -> (Arc<dyn RelationalDb>, String) {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    let db = PostgresRelationalDb::connect(dsn, schema, PoolSettings::default()).await.unwrap();
    // Create the tables now, so a count before the first volume open reads 0.
    db.migrate(&crate::storage::meta::schema()).await.unwrap();
    (Arc::new(db), dsn.to_string())
}

#[cfg(not(feature = "postgres"))]
async fn open_pg(_dsn: &str, _schema: &str) -> (Arc<dyn RelationalDb>, String) {
    unreachable!("the fixture skips without the postgres feature")
}

#[cfg(feature = "sqlserver")]
async fn open_mssql(dsn: &str, name: &str) -> (Arc<dyn RelationalDb>, String) {
    use crate::storage::rel::{PoolSettings, SqlServerRelationalDb};
    let master = SqlServerRelationalDb::connect(dsn, PoolSettings::default()).await.unwrap();
    master.execute(&Query::new(format!("CREATE DATABASE [{name}]"))).await.unwrap();
    drop(master);
    let scoped = with_database(dsn, name);
    let db = SqlServerRelationalDb::connect(&scoped, PoolSettings::default()).await.unwrap();
    db.migrate(&crate::storage::meta::schema()).await.unwrap();
    (Arc::new(db), scoped)
}

#[cfg(not(feature = "sqlserver"))]
async fn open_mssql(_dsn: &str, _name: &str) -> (Arc<dyn RelationalDb>, String) {
    unreachable!("the fixture skips without the sqlserver feature")
}

/// The ADO.NET `dsn` pointed at database `name` instead of the one it names.
fn with_database(dsn: &str, name: &str) -> String {
    let mut parts: Vec<String> = dsn
        .split(';')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or_default().trim().to_ascii_lowercase();
            !p.trim().is_empty() && key != "database" && key != "initial catalog"
        })
        .map(str::to_string)
        .collect();
    parts.push(format!("Database={name}"));
    parts.join(";")
}

// ── BT-005: trash rows of a deleted project ─────────────────────────────────

async fn bt_005(engine: Engine, test: &str) {
    let Some(f) = Fixture::new(engine, test, None).await else { return };
    seed_trash(&f, ATLAS).await;

    f.delete(ATLAS).await;
    f.create(ATLAS, OTTO).await;

    let listed = f.trash_list(OTTO, ATLAS).await;
    assert_eq!(listed["total"], json!(0), "Otto must not see Olivia's trash: {listed}");
    assert_eq!(listed, json!({"entries": [], "total": 0}));
    assert_eq!(f.count("trash_entries", ATLAS).await, 0, "the row must be gone from the database");
    f.cleanup().await;
}

/// SPEC-0024/BT-005 on PostgreSQL (BR-001, trash part).
#[tokio::test]
async fn spec_0024_bt_005_trash_rows_of_a_deleted_project_pg() {
    bt_005(Engine::Postgres, "bt005pg").await;
}

/// SPEC-0024/BT-005 on SQL Server (BR-001, trash part).
#[tokio::test]
async fn spec_0024_bt_005_trash_rows_of_a_deleted_project_mssql() {
    bt_005(Engine::SqlServer, "bt005ms").await;
}

// ── BT-008: the artifact sweep spares the new project's blobs ───────────────

/// SPEC-0024/BT-008 (BR-005): the deleted project's artifact set must not
/// decrement, then GC, a blob the recreated project owns.
#[tokio::test]
async fn spec_0024_bt_008_the_artifact_sweep_spares_the_new_projects_blobs() {
    let Some(f) = Fixture::new(Engine::Postgres, "bt008", Some(memo_conversion())).await else {
        return;
    };
    f.write(OLIVIA, ATLAS, "/memo.docx", b"PK docx").await;
    f.convert(OLIVIA, ATLAS, "/memo.docx").await;
    assert_eq!(f.count("doc_images", ATLAS).await, 1, "the conversion must keep IMG-A");

    f.delete(ATLAS).await;
    f.create(ATLAS, OTTO).await;
    f.write(OTTO, ATLAS, "/logo.png", IMG_A).await;

    // Far past the artifact grace period: `run_cycle` sweeps with the wall clock,
    // and an orphan set (its node is gone) is stale whatever its age.
    crate::purge::run_cycle(
        &f.state.stores,
        f.state.admin.as_ref(),
        &f.state.safety,
        f.state.search.as_ref(),
    )
    .await
    .unwrap();

    let args = ReadBytesArgs {
        mount_id: ATLAS.into(),
        path: "/logo.png".into(),
        offset_bytes: 0,
        length_bytes: 65536,
    };
    let read = f.server(OTTO).fs_read_bytes(Parameters(args)).await.unwrap();
    assert_ne!(read.is_error, Some(true), "Otto's /logo.png must survive: {}", text_of(&read));
    let read: Value = serde_json::from_str(&text_of(&read)).unwrap();
    assert_eq!(read["base64"], json!(b64(IMG_A)), "got: {read}");
    // Second channel: the bytes in the blob store itself, by content address.
    let client = f.state.stores.client(ATLAS).await.unwrap();
    let sha = crate::storage::VolumeClient::sha256_hex(IMG_A);
    assert_eq!(client.blob.get(&sha, 0, None).await.unwrap(), IMG_A, "IMG-A must stay stored");
    drop(client);
    f.cleanup().await;
}

// ── BT-010: no metadata row of a deleted volume remains ─────────────────────

async fn bt_010(engine: Engine, test: &str) {
    let Some(f) = Fixture::new(engine, test, Some(memo_conversion())).await else { return };
    f.create(ATLAS2, OLIVIA).await;
    for project in [ATLAS, ATLAS2] {
        seed_trash(&f, project).await;
        f.write(OLIVIA, project, "/memo.docx", b"PK docx").await;
        f.convert(OLIVIA, project, "/memo.docx").await;
        f.export(OLIVIA, project, &["/plan.txt"]).await;
    }
    let before = counts(&f, ATLAS).await;
    assert!(
        before.iter().all(|(_, n)| *n > 0),
        "every table must hold a row of atlas before the delete, or the test proves nothing: \
         {before:?}"
    );
    let neighbour = counts(&f, ATLAS2).await;

    f.delete(ATLAS).await;

    let after = counts(&f, ATLAS).await;
    let left: Vec<_> = after.iter().filter(|(_, n)| *n != 0).collect();
    assert!(left.is_empty(), "rows of the deleted volume survive: {left:?}");
    // BR-007: the neighbour whose id `atlas` prefixes keeps every row.
    assert_eq!(counts(&f, ATLAS2).await, neighbour, "atlas2 must be untouched");
    f.cleanup().await;
}

/// SPEC-0024/BT-010 on PostgreSQL (BR-004, BR-007).
#[tokio::test]
async fn spec_0024_bt_010_no_metadata_row_of_a_deleted_volume_remains_pg() {
    bt_010(Engine::Postgres, "bt010pg").await;
}

/// SPEC-0024/BT-010 on SQL Server (BR-004, BR-007).
#[tokio::test]
async fn spec_0024_bt_010_no_metadata_row_of_a_deleted_volume_remains_mssql() {
    bt_010(Engine::SqlServer, "bt010ms").await;
}

// ── BT-012: an old export link answers 404 ──────────────────────────────────

/// SPEC-0024/BT-012 (BR-006): a link minted before the delete is dead after
/// the recreation.
#[tokio::test]
async fn spec_0024_bt_012_an_old_export_link_answers_404() {
    let Some(f) = Fixture::new(Engine::Postgres, "bt012", None).await else { return };
    f.write(OLIVIA, ATLAS, "/plan.txt", b"merger with Borealis").await;
    let url = f.export(OLIVIA, ATLAS, &["/plan.txt"]).await;

    f.delete(ATLAS).await;
    f.create(ATLAS, OTTO).await;

    let app = crate::exports::router(f.state.clone());
    let res = app.oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap()).await;
    let res = res.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND, "the old archive leaked {} bytes", body.len());
    assert_eq!(f.count("export_links", ATLAS).await, 0, "no link row of the old project remains");
    f.cleanup().await;
}
