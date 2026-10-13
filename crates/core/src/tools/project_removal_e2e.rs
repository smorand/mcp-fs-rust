//! SPEC-0024 acceptance tests (US-0026): removing a project, through
//! `admin.delete_project` or the grace period purge, wipes its search index, so
//! a project recreated under the same id finds none of the old content
//! (CWE-226).
//!
//! Every outcome is read twice: through the tools a user calls (`search.query`,
//! `fs.trash_list`), and on a second channel (`search.status` counts, a listing
//! of the configured Tantivy directory, PostgreSQL row counts). The SQLite
//! backends run in a plain `make check`; the PostgreSQL ones skip, never fail,
//! when `MCPFS_TEST_PG_DSN` is unset or the feature is off, and each owns a
//! fresh schema so the literal `atlas` id never collides with another suite.

use crate::config::{Dsn, ServerConfig, backend};
use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
use crate::errors::{Result, ToolError};
use crate::identity::IdentityResolver;
use crate::mcp::server::McpServer;
use crate::safety::SafetyManager;
use crate::search::SearchBackend;
use crate::state::AppState;
use crate::storage::StoreManager;
use crate::storage::rel::{Query, RelationalDb};
use crate::storage::traits::{AdminBackend, IndexMode, Member, Project, PurgeConfig};
use async_trait::async_trait;
use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const ADA: &str = "ada@example.com";
const OLIVIA: &str = "olivia@example.com";
const OTTO: &str = "otto@example.com";
const ATLAS: &str = "atlas";
const ATLAS2: &str = "atlas2";
/// The confidential content the deleted project held, and the word searched.
const LEAK: &[u8] = b"merger with Borealis";
const NEEDLE: &str = "Borealis";
/// Vector width of the stub embedding server.
const DIMS: u32 = 3;
/// How long an index may take to settle before the test calls it broken.
const INDEX_TIMEOUT_SECS: u64 = 15;
const DAY: f64 = 86_400.0;

/// Which search engine the server runs, and on which metadata store.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// SQLite metadata, BM25 on Tantivy.
    Tantivy,
    /// SQLite metadata, `rag` on the in memory sqlite-vec index.
    SqliteVec,
    /// PostgreSQL metadata, BM25 on `search_fts`.
    PgBm25,
    /// PostgreSQL metadata, `rag` on `search_chunks`.
    PgRag,
}

impl Kind {
    fn mode(self) -> &'static str {
        match self {
            Kind::Tantivy | Kind::PgBm25 => "bm25",
            Kind::SqliteVec | Kind::PgRag => "rag",
        }
    }

    fn is_pg(self) -> bool {
        matches!(self, Kind::PgBm25 | Kind::PgRag)
    }
}

/// Delegates every [`AdminBackend`] method to `inner`, except that the first
/// `delete_project` fails once `armed` is set: the project record removal
/// failing after the volume teardown and the index wipe already ran (BT-013).
struct FailOnceAdmin {
    inner: crate::storage::admin::RelationalAdminStore,
    armed: AtomicBool,
}

/// A server with `ADA` as platform admin, `atlas` owned by Olivia, and a real
/// search backend of the given kind.
struct Fixture {
    kind: Kind,
    _dir: tempfile::TempDir,
    tantivy_dir: tempfile::TempDir,
    state: Arc<AppState>,
    admin: Arc<FailOnceAdmin>,
    search: Arc<dyn SearchBackend>,
    /// A handle on the PostgreSQL scope, the second channel the counts read.
    db: Option<Arc<dyn RelationalDb>>,
    pg: Option<(String, String)>,
}

impl Fixture {
    /// `None` when the test must skip, after saying why.
    async fn new(kind: Kind, test: &str) -> Option<Self> {
        if matches!(kind, Kind::SqliteVec | Kind::PgRag) && !cfg!(feature = "rag") {
            eprintln!("skipping {test}: built without --features rag");
            return None;
        }
        let pg = if kind.is_pg() {
            let Some(dsn) =
                std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
            else {
                eprintln!("skipping {test}: MCPFS_TEST_PG_DSN is unset");
                return None;
            };
            if !cfg!(feature = "postgres") {
                eprintln!("skipping {test}: built without --features postgres");
                return None;
            }
            Some((dsn, format!("mcpfs_removal_{test}_{}", std::process::id())))
        } else {
            None
        };

        let dir = tempfile::tempdir().unwrap();
        let tantivy_dir = tempfile::tempdir().unwrap();
        let embedding =
            if kind.mode() == "rag" { Some(spawn_fake_embedding().await) } else { None };

        let mut config = ServerConfig::default();
        config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.admins = vec![ADA.to_string()];
        config.auth.jwt.public_key_path = String::new();
        config.search.enabled = true;
        config.search.mode = kind.mode().into();
        config.search.tantivy_dir = tantivy_dir.path().display().to_string();
        if let Some(url) = &embedding {
            config.search.embedding.endpoint = format!("{url}/v1/embeddings");
            config.search.embedding.dimensions = DIMS;
        }
        let mut db = None;
        if let Some((dsn, schema)) = &pg {
            drop_schema(dsn, schema).await;
            db = Some(open_schema(dsn, schema).await);
            config.infra.meta.backend = backend::POSTGRES.into();
            config.infra.meta.dsn = Dsn::new(dsn.clone());
            config.infra.meta.schema = schema.clone();
        }
        let config = Arc::new(config);
        let search = build_search(kind, &config, pg.as_ref()).await;

        let inner = crate::storage::admin::RelationalAdminStore::in_memory().await.unwrap();
        inner.connect().await.unwrap();
        let admin = Arc::new(FailOnceAdmin { inner, armed: AtomicBool::new(false) });
        let safety = Arc::new(SafetyManager::new(
            config.safety.clone(),
            crate::storage::meta::max_path_len(&config.infra.meta.backend),
        ));
        safety.attach_limits(admin.clone());
        let state = Arc::new(AppState {
            config: config.clone(),
            admin: admin.clone(),
            stores: Arc::new(StoreManager::new(config.clone(), crate::storage::test_registry())),
            safety,
            identity: Arc::new(IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service: Some(Arc::new(StubDocService::bundle(memo_conversion()))),
            search: Some(search.clone()),
        });
        let f = Self { kind, _dir: dir, tantivy_dir, state, admin, search, db, pg };
        f.create(ATLAS, OLIVIA).await;
        Some(f)
    }

    fn server(&self, person: &str) -> McpServer {
        McpServer::new(self.state.clone(), person.to_string())
    }

    async fn create(&self, project: &str, owner: &str) {
        ok_json(
            self.server(ADA)
                .admin_create_project(args(json!({"project_id": project, "owner": owner})))
                .await,
        );
    }

    /// `admin.delete_project` as the platform admin, the real tool.
    async fn delete(&self, project: &str) -> std::result::Result<Value, String> {
        tool_json(self.server(ADA).admin_delete_project(args(json!({"project_id": project}))).await)
    }

    async fn write(&self, person: &str, project: &str, path: &str, bytes: &[u8]) {
        let a = args(json!({
            "mount_id": project, "path": path, "base64": b64(bytes),
            "overwrite": true, "create_parents": true,
        }));
        ok_json(self.server(person).fs_write_bytes(a).await);
    }

    async fn remove(&self, person: &str, project: &str, path: &str) {
        let a = args(json!({"mount_id": project, "path": path, "recursive": false, "trash": true}));
        ok_json(self.server(person).fs_delete(a).await);
    }

    async fn trash_list(&self, person: &str, project: &str) -> Value {
        ok_json(self.server(person).fs_trash_list(args(json!({"mount_id": project}))).await)
    }

    async fn query(&self, person: &str, project: &str) -> Value {
        let a = args(json!({"mount_id": project, "query": NEEDLE, "mode": self.kind.mode()}));
        ok_json(self.server(person).search_query(a).await)
    }

    async fn status(&self, person: &str, project: &str) -> Value {
        ok_json(self.server(person).search_status(args(json!({"mount_id": project}))).await)
    }

    async fn set_mode(&self, person: &str, project: &str) {
        let a = args(json!({"project_id": project, "mode": self.kind.mode()}));
        ok_json(self.server(person).admin_set_index_mode(a).await);
    }

    async fn list_images(&self, person: &str, project: &str) -> Value {
        let a = args(json!({"mount_id": project, "path": "/memo.docx"}));
        ok_json(self.server(person).fs_list_images(a).await)
    }

    async fn read(&self, person: &str, project: &str, path: &str) -> Value {
        let a = args(json!({"mount_id": project, "path": path}));
        ok_json(self.server(person).fs_read_bytes(a).await)
    }

    /// Indexed documents plus vector chunks, as `search.status` reports them.
    async fn indexed(&self, person: &str, project: &str) -> u64 {
        let s = self.status(person, project).await;
        s["bm25_docs"].as_u64().unwrap_or(0) + s["vector_chunks"].as_u64().unwrap_or(0)
    }

    /// Wait until the index of `project` holds exactly `expected` entries.
    /// Indexing is fire and forget; a timeout FAILS the test.
    async fn await_indexed(&self, person: &str, project: &str, expected: u64) {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(INDEX_TIMEOUT_SECS);
        let mut last = u64::MAX;
        while std::time::Instant::now() < deadline {
            last = self.indexed(person, project).await;
            if last == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("{project}: index never reached {expected} entries, last saw {last}");
    }

    /// The configured `search.tantivy_dir`, listed: one directory per volume.
    fn tantivy_listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.tantivy_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// PostgreSQL rows of `volume` in `table` (`None` off PostgreSQL).
    async fn pg_rows(&self, table: &str, volume: &str) -> Option<i64> {
        let db = self.db.as_ref()?;
        let sql = format!("SELECT CAST(COUNT(*) AS BIGINT) FROM {table} WHERE volume_id=?1");
        let row = db.query_opt(&Query::new(sql).bind(volume)).await.unwrap();
        Some(row.expect("COUNT(*) yields one row").i64(0).unwrap())
    }

    /// The search table this kind stores its index in on PostgreSQL.
    fn pg_table(&self) -> &'static str {
        if self.kind == Kind::PgRag { "search_chunks" } else { "search_fts" }
    }

    /// The BT-001 state: `/plan.txt` = `merger with Borealis` indexed, and
    /// `/memo.txt` deleted by Olivia into the trash.
    async fn seed(&self, project: &str) {
        // Both writes are indexed before the delete fires: the write and delete
        // hooks are detached tasks, and a write hook landing after the delete
        // hook would leave /memo.txt indexed and the counts below misread.
        self.set_mode(OLIVIA, project).await;
        self.write(OLIVIA, project, "/plan.txt", LEAK).await;
        self.write(OLIVIA, project, "/memo.txt", b"confidential").await;
        self.await_indexed(OLIVIA, project, 2).await;
        self.remove(OLIVIA, project, "/memo.txt").await;
        self.await_indexed(OLIVIA, project, 1).await;
        let found = self.query(OLIVIA, project).await;
        let chunk = found["results"][0]["chunk"].as_str().unwrap_or_default();
        assert!(chunk.contains("merger with Borealis"), "the leaked state must exist: {found}");
        assert_eq!(self.trash_list(OLIVIA, project).await["total"], json!(1));
        if let Some(rows) = self.pg_rows(self.pg_table(), project).await {
            assert!(rows > 0, "the index must hold rows of {project} first");
        }
    }

    async fn cleanup(self) {
        let Self { state, search, db, pg, .. } = self;
        drop(state);
        drop(search);
        drop(db);
        if let Some((dsn, schema)) = pg {
            drop_schema(&dsn, &schema).await;
        }
    }
}

#[async_trait]
impl AdminBackend for FailOnceAdmin {
    async fn connect(&self) -> Result<()> {
        self.inner.connect().await
    }
    async fn create_project(&self, project_id: &str, owner: &str) -> Result<Project> {
        self.inner.create_project(project_id, owner).await
    }
    async fn delete_project(&self, project_id: &str) -> Result<()> {
        if self.armed.swap(false, Ordering::SeqCst) {
            return Err(ToolError::internal(format!("simulated failure removing '{project_id}'")));
        }
        self.inner.delete_project(project_id).await
    }
    async fn add_member(&self, project_id: &str, person: &str, added_by: &str) -> Result<Member> {
        self.inner.add_member(project_id, person, added_by).await
    }
    async fn remove_member(&self, project_id: &str, person: &str) -> Result<()> {
        self.inner.remove_member(project_id, person).await
    }
    async fn get_project(&self, project_id: &str) -> Result<Option<Project>> {
        self.inner.get_project(project_id).await
    }
    async fn list_projects_for(&self, person: &str) -> Result<Vec<Project>> {
        self.inner.list_projects_for(person).await
    }
    async fn list_all_projects(&self) -> Result<Vec<Project>> {
        self.inner.list_all_projects().await
    }
    async fn list_all_persons(&self) -> Result<Vec<String>> {
        self.inner.list_all_persons().await
    }
    async fn list_members(&self, project_id: &str) -> Result<Vec<Member>> {
        self.inner.list_members(project_id).await
    }
    async fn is_member(&self, project_id: &str, person: &str) -> Result<bool> {
        self.inner.is_member(project_id, person).await
    }
    async fn require_member(&self, project_id: &str, person: &str) -> Result<()> {
        self.inner.require_member(project_id, person).await
    }
    async fn require_owner(&self, project_id: &str, person: &str) -> Result<Project> {
        self.inner.require_owner(project_id, person).await
    }
    async fn set_index_mode(&self, project_id: &str, mode: IndexMode) -> Result<()> {
        self.inner.set_index_mode(project_id, mode).await
    }
    async fn get_index_mode(&self, project_id: &str) -> Result<IndexMode> {
        self.inner.get_index_mode(project_id).await
    }
    async fn get_purge_config(&self, project_id: &str) -> Result<PurgeConfig> {
        self.inner.get_purge_config(project_id).await
    }
    async fn set_purge_config(&self, project_id: &str, config: PurgeConfig) -> Result<()> {
        self.inner.set_purge_config(project_id, config).await
    }
    async fn soft_delete_project(&self, project_id: &str) -> Result<bool> {
        self.inner.soft_delete_project(project_id).await
    }
    async fn undelete_project(&self, project_id: &str) -> Result<()> {
        self.inner.undelete_project(project_id).await
    }
    async fn list_soft_deleted_projects(&self) -> Result<Vec<(String, String)>> {
        self.inner.list_soft_deleted_projects().await
    }
    async fn set_quota(&self, project_id: &str, quota_bytes: Option<i64>) -> Result<()> {
        self.inner.set_quota(project_id, quota_bytes).await
    }
    async fn get_quota(&self, project_id: &str) -> Result<Option<i64>> {
        self.inner.get_quota(project_id).await
    }
}

fn args<T: DeserializeOwned>(v: Value) -> Parameters<T> {
    Parameters(serde_json::from_value(v).expect("the arguments must deserialize"))
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

/// `Ok(json)` on success, `Err(text)` on a tool error.
fn tool_json(
    r: std::result::Result<CallToolResult, rmcp::ErrorData>,
) -> std::result::Result<Value, String> {
    let r = r.expect("a tool result, not a protocol error");
    if r.is_error == Some(true) {
        return Err(text_of(&r));
    }
    Ok(serde_json::from_str(&text_of(&r)).expect("the tool result is JSON"))
}

fn ok_json(r: std::result::Result<CallToolResult, rmcp::ErrorData>) -> Value {
    tool_json(r).unwrap_or_else(|e| panic!("expected success, got: {e}"))
}

/// One picture and one table, so `fs.list_images` has something to return.
fn memo_conversion() -> Conversion {
    let target = "media/org.png";
    let meta = PictureMeta { caption: "Org chart".into(), caption_failed: false, page: Some(1) };
    Conversion {
        markdown: format!("# Memo\n\n![chart]({target})\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n"),
        files: [(target.to_string(), b"IMG-A".to_vec())].into_iter().collect(),
        manifest: Some(Manifest { pictures: [(target.to_string(), meta)].into_iter().collect() }),
    }
}

/// An embedding server on loopback answering a fixed vector to any request.
async fn spawn_fake_embedding() -> String {
    use axum::routing::post;
    use axum::{Json, Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            let vec: Vec<f32> = (0..DIMS).map(|i| (i + 1) as f32 / DIMS as f32).collect();
            Json(json!({"data": [{"embedding": vec, "index": 0}], "model": "fake"}))
        }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

async fn build_search(
    kind: Kind,
    config: &ServerConfig,
    pg: Option<&(String, String)>,
) -> Arc<dyn SearchBackend> {
    match kind {
        Kind::Tantivy => Arc::new(crate::search::bm25_sqlite::TantivyBm25Backend::new(
            &config.search.tantivy_dir,
        )),
        Kind::SqliteVec => sqlite_vec(config),
        Kind::PgBm25 | Kind::PgRag => {
            let (dsn, schema) = pg.expect("a PostgreSQL kind has a scope");
            pg_search(kind, config, dsn, schema).await
        }
    }
}

#[cfg(feature = "rag")]
fn sqlite_vec(config: &ServerConfig) -> Arc<dyn SearchBackend> {
    crate::storage::sqlite::register_sqlite_vec();
    Arc::new(crate::search::vector_sqlite::SqliteVecBackend::new(
        DIMS,
        &config.search.tantivy_dir,
        Arc::new(reqwest::Client::new()),
        config.search.embedding.clone(),
    ))
}

#[cfg(not(feature = "rag"))]
fn sqlite_vec(_config: &ServerConfig) -> Arc<dyn SearchBackend> {
    unreachable!("the fixture skips without the rag feature")
}

#[cfg(feature = "postgres")]
async fn pg_search(
    kind: Kind,
    config: &ServerConfig,
    dsn: &str,
    schema: &str,
) -> Arc<dyn SearchBackend> {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    let db = PostgresRelationalDb::connect(dsn, schema, PoolSettings::default()).await.unwrap();
    let pool = db.pool().clone();
    if kind == Kind::PgRag {
        return pg_vector(pool, config).await;
    }
    Arc::new(crate::search::bm25_pg::PostgresBm25Backend::new(pool).await.unwrap())
}

#[cfg(not(feature = "postgres"))]
async fn pg_search(_: Kind, _: &ServerConfig, _: &str, _: &str) -> Arc<dyn SearchBackend> {
    unreachable!("the fixture skips without the postgres feature")
}

#[cfg(feature = "rag")]
async fn pg_vector(pool: sqlx::PgPool, config: &ServerConfig) -> Arc<dyn SearchBackend> {
    let client = Arc::new(reqwest::Client::new());
    let embedding = config.search.embedding.clone();
    Arc::new(
        crate::search::vector_pg::PostgresVectorBackend::new(pool, DIMS, client, embedding)
            .await
            .unwrap(),
    )
}

#[cfg(all(feature = "postgres", not(feature = "rag")))]
async fn pg_vector(_pool: sqlx::PgPool, _config: &ServerConfig) -> Arc<dyn SearchBackend> {
    unreachable!("the fixture skips without the rag feature")
}

#[cfg(feature = "postgres")]
async fn open_schema(dsn: &str, schema: &str) -> Arc<dyn RelationalDb> {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    let db = PostgresRelationalDb::connect(dsn, schema, PoolSettings::default()).await.unwrap();
    db.migrate(&crate::storage::meta::schema()).await.unwrap();
    Arc::new(db)
}

#[cfg(not(feature = "postgres"))]
async fn open_schema(_dsn: &str, _schema: &str) -> Arc<dyn RelationalDb> {
    unreachable!("the fixture skips without the postgres feature")
}

#[cfg(feature = "postgres")]
async fn drop_schema(dsn: &str, schema: &str) {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    let db = PostgresRelationalDb::connect(dsn, "public", PoolSettings::default()).await.unwrap();
    db.execute(&Query::new(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))).await.unwrap();
}

#[cfg(not(feature = "postgres"))]
async fn drop_schema(_dsn: &str, _schema: &str) {}

/// After removal and recreation for Otto: nothing of the old project shows,
/// through the tools and on the second channel.
async fn assert_recreated_project_is_empty(f: &Fixture) {
    let found = f.query(OTTO, ATLAS).await;
    assert_eq!(found["results"], json!([]), "the old content leaked through search.query");
    let status = f.status(OTTO, ATLAS).await;
    assert_eq!(status["bm25_docs"].as_u64().unwrap_or(0), 0, "status: {status}");
    assert_eq!(status["vector_chunks"].as_u64().unwrap_or(0), 0, "status: {status}");
    let trash = f.trash_list(OTTO, ATLAS).await;
    assert_eq!(trash["entries"], json!([]), "got: {trash}");
    assert_eq!(trash["total"], json!(0), "got: {trash}");
}

/// What remains of the old index right after the removal, read before
/// anything reopens it: whether the configured Tantivy directory still lists
/// `atlas`, and the PostgreSQL rows of `atlas` in the search table.
struct Remains {
    tantivy: Vec<String>,
    pg_rows: Option<i64>,
}

async fn remains(f: &Fixture) -> Remains {
    Remains { tantivy: f.tantivy_listing(), pg_rows: f.pg_rows(f.pg_table(), ATLAS).await }
}

/// No row and no Tantivy directory of the old project remained (BR-003).
fn assert_index_was_gone(f: &Fixture, r: &Remains) {
    assert!(
        !r.tantivy.iter().any(|n| n == ATLAS),
        "the Tantivy directory survives: {:?}",
        r.tantivy
    );
    if let Some(rows) = r.pg_rows {
        assert_eq!(rows, 0, "{} still holds rows of atlas", f.pg_table());
    }
}

// ── BT-001, BT-002, BT-006, BT-011: deletion then recreation ────────────────

async fn delete_then_recreate(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.seed(ATLAS).await;

    assert_eq!(f.delete(ATLAS).await, Ok(json!({"project_id": ATLAS, "deleted": true})));
    let left = remains(&f).await;
    f.create(ATLAS, OTTO).await;
    assert_recreated_project_is_empty(&f).await;
    assert_index_was_gone(&f, &left);
    f.cleanup().await;
}

/// SPEC-0024/BT-001 (BR-001, BR-002): PostgreSQL metadata, BM25 search.
#[tokio::test]
async fn spec_0024_bt_001_recreated_project_sees_no_old_trash_or_search_pg() {
    delete_then_recreate(Kind::PgBm25, "bt001").await;
}

/// SPEC-0024/BT-002 (BR-002): the same on the default SQLite configuration,
/// BM25 on Tantivy; runs in a plain `make check`.
#[tokio::test]
async fn spec_0024_bt_002_recreated_project_sees_no_old_search_sqlite() {
    delete_then_recreate(Kind::Tantivy, "bt002").await;
}

/// SPEC-0024/BT-006 (BR-002, BR-003) on the in memory sqlite-vec index.
#[tokio::test]
async fn spec_0024_bt_006_search_content_of_a_deleted_project_sqlite_vec() {
    delete_then_recreate(Kind::SqliteVec, "bt006vec").await;
}

/// SPEC-0024/BT-006 on PostgreSQL vectors (`search_chunks`).
#[tokio::test]
async fn spec_0024_bt_006_search_content_of_a_deleted_project_pg_rag() {
    delete_then_recreate(Kind::PgRag, "bt006pgrag").await;
}

/// SPEC-0024/BT-011 (BR-003): deleted and NOT recreated, the index is gone.
async fn removed_without_reuse(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.seed(ATLAS).await;
    f.delete(ATLAS).await.unwrap();
    let left = remains(&f).await;
    // The backend itself, the only reader once the project is gone.
    let found = match kind.mode() {
        "bm25" => f.search.query_bm25(ATLAS, NEEDLE, 10).await.unwrap(),
        _ => f.search.query_vector(ATLAS, NEEDLE, 10).await.unwrap(),
    };
    assert!(found.is_empty(), "the deleted project is still searchable: {found:?}");
    assert_index_was_gone(&f, &left);
    f.cleanup().await;
}

#[tokio::test]
async fn spec_0024_bt_011_index_removed_without_reuse_tantivy() {
    removed_without_reuse(Kind::Tantivy, "bt011tan").await;
}

#[tokio::test]
async fn spec_0024_bt_011_index_removed_without_reuse_pg_bm25() {
    removed_without_reuse(Kind::PgBm25, "bt011pg").await;
}

#[tokio::test]
async fn spec_0024_bt_011_index_removed_without_reuse_pg_rag() {
    removed_without_reuse(Kind::PgRag, "bt011pgrag").await;
}

// ── BT-007: grace period purge path ─────────────────────────────────────────

async fn grace_purge(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.seed(ATLAS).await;
    assert!(f.state.admin.soft_delete_project(ATLAS).await.unwrap());

    let grace = f.state.config.safety.project_purge_grace_days as f64 * DAY;
    let later = crate::util::now_unix() + grace + DAY;
    let removed = crate::purge::sweep_grace_period_at(
        later,
        &f.state.stores,
        f.state.admin.as_ref(),
        &f.state.config,
        f.state.search.as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(removed, 1, "the grace sweep must remove atlas");
    let left = remains(&f).await;
    f.create(ATLAS, OTTO).await;
    assert_recreated_project_is_empty(&f).await;
    assert_index_was_gone(&f, &left);
    f.cleanup().await;
}

/// SPEC-0024/BT-007 (BR-003) on Tantivy, in `make check`.
#[tokio::test]
async fn spec_0024_bt_007_grace_purge_wipes_the_index_tantivy() {
    grace_purge(Kind::Tantivy, "bt007tan").await;
}

/// SPEC-0024/BT-007 on PostgreSQL.
#[tokio::test]
async fn spec_0024_bt_007_grace_purge_wipes_the_index_pg() {
    grace_purge(Kind::PgBm25, "bt007pg").await;
}

// ── BT-009: in memory vector index, one process ─────────────────────────────

/// SPEC-0024/BT-009: the cached sqlite-vec connection must not keep the old
/// chunks across a delete and recreate in the same process.
#[tokio::test]
async fn spec_0024_bt_009_in_memory_vector_index_of_a_deleted_project() {
    let Some(f) = Fixture::new(Kind::SqliteVec, "bt009").await else { return };
    f.seed(ATLAS).await;
    f.delete(ATLAS).await.unwrap();
    f.create(ATLAS, OTTO).await;
    let found = f.query(OTTO, ATLAS).await;
    assert_eq!(found["results"], json!([]), "the cached connection leaked: {found}");
    assert_eq!(f.status(OTTO, ATLAS).await["vector_chunks"], json!(0));
    f.cleanup().await;
}

// ── BT-013: a failed deletion is retried ────────────────────────────────────

async fn retry_after_failure(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.seed(ATLAS).await;

    f.admin.armed.store(true, Ordering::SeqCst);
    let first = f.delete(ATLAS).await;
    assert!(first.unwrap_err().contains("simulated failure"), "the failure must surface");
    assert!(f.state.admin.get_project(ATLAS).await.unwrap().is_some(), "the record survives");

    assert_eq!(f.delete(ATLAS).await, Ok(json!({"project_id": ATLAS, "deleted": true})));
    assert!(f.state.admin.get_project(ATLAS).await.unwrap().is_none());
    let left = remains(&f).await;
    // BT-010 on PostgreSQL: no metadata row of the volume survives the retry.
    if let Some(db) = &f.db {
        for table in crate::storage::meta::schema().tables {
            if table.columns.iter().any(|c| c.name == "volume_id") {
                let sql = format!(
                    "SELECT CAST(COUNT(*) AS BIGINT) FROM {} WHERE volume_id=?1",
                    table.name
                );
                let row = db.query_opt(&Query::new(sql).bind(ATLAS)).await.unwrap().unwrap();
                assert_eq!(row.i64(0).unwrap(), 0, "{} keeps rows of atlas", table.name);
            }
        }
    }
    f.create(ATLAS, OTTO).await;
    assert_recreated_project_is_empty(&f).await;
    assert_index_was_gone(&f, &left);
    f.cleanup().await;
}

/// SPEC-0024/BT-013 (BR-003, BR-004) on Tantivy, in `make check`.
#[tokio::test]
async fn spec_0024_bt_013_deletion_is_retryable_tantivy() {
    retry_after_failure(Kind::Tantivy, "bt013tan").await;
}

/// SPEC-0024/BT-013 on PostgreSQL, with the BT-010 row counts.
#[tokio::test]
async fn spec_0024_bt_013_deletion_is_retryable_pg() {
    retry_after_failure(Kind::PgBm25, "bt013pg").await;
}

// ── BT-020: other projects untouched ────────────────────────────────────────

/// What a user of `atlas2` observes, through the tools.
async fn snapshot(f: &Fixture, project: &str) -> Vec<Value> {
    vec![
        f.trash_list(OLIVIA, project).await,
        f.query(OLIVIA, project).await,
        f.status(OLIVIA, project).await,
        f.list_images(OLIVIA, project).await,
        f.read(OLIVIA, project, "/plan.txt").await,
    ]
}

async fn settled_snapshot(f: &Fixture, project: &str) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(INDEX_TIMEOUT_SECS);
    let mut last = snapshot(f, project).await;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let next = snapshot(f, project).await;
        if next == last {
            return next;
        }
        last = next;
    }
    panic!("{project} never settled: {last:?}");
}

async fn others_untouched(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.create(ATLAS2, OLIVIA).await;
    for project in [ATLAS, ATLAS2] {
        f.seed(project).await;
        f.write(OLIVIA, project, "/memo.docx", b"PK docx").await;
        let a = args(json!({"mount_id": project, "path": "/memo.docx", "overwrite": true}));
        ok_json(f.server(OLIVIA).fs_documentize(a).await);
    }
    // The documentize hooks index and unindex in the background, so the
    // baseline is taken once two reads a beat apart agree.
    let before = settled_snapshot(&f, ATLAS2).await;
    let pg_before = f.pg_rows(f.pg_table(), ATLAS2).await;

    f.delete(ATLAS).await.unwrap();

    assert_eq!(snapshot(&f, ATLAS2).await, before, "atlas2 changed");
    assert_eq!(f.pg_rows(f.pg_table(), ATLAS2).await, pg_before, "atlas2 rows changed");
    assert!(f.tantivy_listing().iter().any(|n| n == ATLAS2) || kind != Kind::Tantivy);
    f.cleanup().await;
}

/// SPEC-0024/BT-020 (BR-007) on Tantivy, in `make check`.
#[tokio::test]
async fn spec_0024_bt_020_other_projects_untouched_tantivy() {
    others_untouched(Kind::Tantivy, "bt020tan").await;
}

/// SPEC-0024/BT-020 on PostgreSQL.
#[tokio::test]
async fn spec_0024_bt_020_other_projects_untouched_pg() {
    others_untouched(Kind::PgBm25, "bt020pg").await;
}

// ── BT-021: undelete inside the grace period restores everything ────────────

async fn undelete_restores(kind: Kind, test: &str) {
    let Some(f) = Fixture::new(kind, test).await else { return };
    f.seed(ATLAS).await;
    let trash = f.trash_list(OLIVIA, ATLAS).await;
    let found = f.query(OLIVIA, ATLAS).await;

    assert!(f.state.admin.soft_delete_project(ATLAS).await.unwrap());
    // A grace sweep inside the grace period removes nothing (BR-008).
    let removed = crate::purge::sweep_grace_period_at(
        crate::util::now_unix(),
        &f.state.stores,
        f.state.admin.as_ref(),
        &f.state.config,
        f.state.search.as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(removed, 0);
    ok_json(f.server(ADA).admin_undelete_project(args(json!({"project_id": ATLAS}))).await);

    assert_eq!(f.trash_list(OLIVIA, ATLAS).await, trash);
    assert_eq!(f.query(OLIVIA, ATLAS).await, found);
    assert_eq!(f.indexed(OLIVIA, ATLAS).await, 1);
    f.cleanup().await;
}

/// SPEC-0024/BT-021 (BR-008) on Tantivy, in `make check`.
#[tokio::test]
async fn spec_0024_bt_021_undelete_restores_everything_tantivy() {
    undelete_restores(Kind::Tantivy, "bt021tan").await;
}

/// SPEC-0024/BT-021 on PostgreSQL.
#[tokio::test]
async fn spec_0024_bt_021_undelete_restores_everything_pg() {
    undelete_restores(Kind::PgBm25, "bt021pg").await;
}
