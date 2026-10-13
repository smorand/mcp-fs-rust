//! SPEC-0022 acceptance tests: a null written to a nullable numeric column is
//! accepted on PostgreSQL, driven through the real `McpServer` tool methods.
//!
//! Each test runs on its own PostgreSQL schema, holding both the admin registry
//! and the volume metadata, so concurrent tests never share a row. They skip,
//! never fail, when `MCPFS_TEST_PG_DSN` is unset or the `postgres` feature is off.

use crate::config::{Dsn, ServerConfig, backend};
use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
use crate::identity::IdentityResolver;
use crate::mcp::server::{
    CreateProjectArgs, DocumentizeArgs, GetProjectQuotaArgs, ListImagesArgs, McpServer,
    SetProjectQuotaArgs, SetPurgeConfigArgs,
};
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::StoreManager;
use crate::storage::rel::Query;
use crate::storage::traits::PurgeConfig;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;

const ADA: &str = "ada@example.com";
const OLIVIA: &str = "olivia@example.com";
const ATLAS: &str = "atlas";

/// One PostgreSQL schema plus a server acting as the platform admin `ADA`, who
/// also owns `ATLAS` so the `fs.*` tools are authorized.
struct PgFixture {
    _dir: tempfile::TempDir,
    state: Arc<AppState>,
    dsn: String,
    schema: String,
}

impl PgFixture {
    /// `None` when the suite must skip, after saying why.
    async fn new(test: &str, doc: Option<Conversion>) -> Option<Self> {
        let Some(dsn) = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
        else {
            eprintln!("skipping {test}: MCPFS_TEST_PG_DSN is unset");
            return None;
        };
        if !cfg!(feature = "postgres") {
            eprintln!("skipping {test}: built without --features postgres");
            return None;
        }
        let schema = format!("mcpfs_pgnull_{test}_{}", std::process::id());
        // A run that panicked leaves its schema behind; start from nothing.
        drop_schema(&dsn, &schema).await;

        let dir = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.infra.meta.backend = backend::POSTGRES.into();
        config.infra.meta.dsn = Dsn::new(dsn.clone());
        config.infra.meta.schema = schema.clone();
        config.infra.admin.backend = backend::POSTGRES.into();
        config.infra.admin.dsn = Dsn::new(dsn.clone());
        config.infra.admin.schema = schema.clone();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.admins = vec![ADA.to_string()];
        config.auth.jwt.public_key_path = String::new();
        let config = Arc::new(config);

        let registry = crate::storage::test_registry();
        let admin = crate::storage::build_admin_store(&config, &registry).await.unwrap();
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
            stores: Arc::new(StoreManager::new(config.clone(), registry)),
            safety,
            identity: Arc::new(IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service,
            search: None,
        });
        state.admin.create_project(ATLAS, ADA).await.unwrap();
        state.stores.provision_volume(ATLAS).await.unwrap();
        Some(Self { _dir: dir, state, dsn, schema })
    }

    fn server(&self) -> McpServer {
        McpServer::new(self.state.clone(), ADA.to_string())
    }

    async fn cleanup(self) {
        drop_schema(&self.dsn, &self.schema).await;
    }
}

#[cfg(feature = "postgres")]
async fn drop_schema(dsn: &str, schema: &str) {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb, RelationalDb as _};
    let db = PostgresRelationalDb::connect(dsn, "public", PoolSettings::default()).await.unwrap();
    db.execute(&Query::new(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))).await.unwrap();
}

#[cfg(not(feature = "postgres"))]
async fn drop_schema(_dsn: &str, _schema: &str) {
    // Unreachable: the fixture skips before any schema exists without the feature.
    let _ = Query::new("");
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

fn ok_json(r: Result<CallToolResult, rmcp::ErrorData>) -> Value {
    let r = r.expect("a tool result, not a protocol error");
    assert_ne!(r.is_error, Some(true), "expected success, got: {}", text_of(&r));
    serde_json::from_str(&text_of(&r)).expect("the tool result is JSON")
}

fn quota_args(max_mb: Option<u32>) -> SetProjectQuotaArgs {
    SetProjectQuotaArgs { project_id: ATLAS.into(), max_mb }
}

fn purge_args(file: Option<u32>, project: Option<u32>) -> SetPurgeConfigArgs {
    SetPurgeConfigArgs {
        project_id: ATLAS.into(),
        autopurge_enabled: true,
        use_internal_purge: true,
        file_retention_days: file,
        project_retention_days: project,
    }
}

/// The SPEC-0019 E2E-115 stub: a Word picture whose converter claims a page.
fn memo_conversion(target: &str, caption: &str) -> Conversion {
    let meta = PictureMeta { caption: caption.into(), caption_failed: false, page: Some(1) };
    Conversion {
        markdown: format!("# Memo\n\n![chart]({target})\n"),
        files: [(target.to_string(), b"org".to_vec())].into_iter().collect(),
        manifest: Some(Manifest { pictures: [(target.to_string(), meta)].into_iter().collect() }),
    }
}

async fn put(f: &PgFixture, path: &str, bytes: &[u8]) {
    f.state.stores.client(ATLAS).await.unwrap().write_bytes_atomic(path, bytes).await.unwrap();
}

async fn convert(server: &McpServer, path: &str) -> Value {
    let args = DocumentizeArgs { mount_id: ATLAS.into(), path: path.into(), overwrite: true };
    ok_json(server.fs_documentize(Parameters(args)).await)
}

async fn images(server: &McpServer, path: &str) -> Value {
    let args = ListImagesArgs { mount_id: ATLAS.into(), path: path.into(), marker: None };
    ok_json(server.fs_list_images(Parameters(args)).await)
}

/// SPEC-0022/BT-001: clearing a quota on PostgreSQL (BR-001).
#[tokio::test]
async fn spec_0022_bt_001_clearing_a_quota() {
    let Some(f) = PgFixture::new("bt001", None).await else { return };
    let server = f.server();
    ok_json(server.admin_set_project_quota(Parameters(quota_args(Some(5)))).await);
    let cleared = ok_json(server.admin_set_project_quota(Parameters(quota_args(None))).await);
    assert_eq!(cleared, json!({"project_id": ATLAS, "max_mb": null}));
    let read = ok_json(
        server
            .admin_get_project_quota(Parameters(GetProjectQuotaArgs { project_id: ATLAS.into() }))
            .await,
    );
    assert_eq!(read["max_mb"], Value::Null, "got: {read}");
    f.cleanup().await;
}

/// SPEC-0022/BT-010: a purge config with one, then both, retention axes null (BR-002).
#[tokio::test]
async fn spec_0022_bt_010_purge_config_with_null_axes() {
    let Some(f) = PgFixture::new("bt010", None).await else { return };
    let server = f.server();
    ok_json(server.admin_set_purge_config(Parameters(purge_args(Some(30), None))).await);
    let one = f.state.admin.get_purge_config(ATLAS).await.unwrap();
    assert_eq!(one.file_retention_days, Some(30));
    assert_eq!(one.project_retention_days, None);

    ok_json(server.admin_set_purge_config(Parameters(purge_args(None, None))).await);
    let both = f.state.admin.get_purge_config(ATLAS).await.unwrap();
    assert_eq!(both.file_retention_days, None);
    assert_eq!(both.project_retention_days, None);
    f.cleanup().await;
}

/// SPEC-0022/BT-011: a project created with purge on and one axis null (BR-003).
#[tokio::test]
async fn spec_0022_bt_011_project_creation_with_a_null_axis() {
    let Some(f) = PgFixture::new("bt011", None).await else { return };
    let args = CreateProjectArgs {
        project_id: "borea".into(),
        owner: OLIVIA.into(),
        autopurge_enabled: true,
        use_internal_purge: false,
        file_retention_days: Some(7),
        project_retention_days: None,
    };
    ok_json(f.server().admin_create_project(Parameters(args)).await);
    assert!(f.state.admin.get_project("borea").await.unwrap().is_some(), "borea exists");
    assert_eq!(
        f.state.admin.get_purge_config("borea").await.unwrap(),
        PurgeConfig {
            autopurge_enabled: true,
            use_internal_purge: false,
            file_retention_days: Some(7),
            project_retention_days: None,
        }
    );
    f.cleanup().await;
}

/// SPEC-0022/BT-012: a conversion on PostgreSQL keeps its artifacts, the Word
/// picture reporting a null page (BR-004, BR-005).
#[tokio::test]
async fn spec_0022_bt_012_a_conversion_keeps_its_artifacts() {
    let conversion = memo_conversion("media/org.png", "Org chart");
    let Some(f) = PgFixture::new("bt012", Some(conversion)).await else { return };
    let server = f.server();
    put(&f, "/memo.docx", b"PK docx").await;
    convert(&server, "/memo.docx").await;
    assert_eq!(
        images(&server, "/memo.docx").await,
        json!({"images": [{"id": "image-1", "caption": "Org chart", "page": null}], "marker": null})
    );
    f.cleanup().await;
}

/// SPEC-0022/BT-013: reconverting supersedes the previous set, which writes a
/// null `superseded_at` for the new set and stamps the old one (BR-004).
#[tokio::test]
async fn spec_0022_bt_013_reconverting_supersedes_the_previous_set() {
    let Some(f) =
        PgFixture::new("bt013", Some(memo_conversion("media/org.png", "Org chart"))).await
    else {
        return;
    };
    put(&f, "/memo.docx", b"PK docx").await;
    convert(&f.server(), "/memo.docx").await;

    let s = &f.state;
    let next = AppState {
        config: s.config.clone(),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: s.identity.clone(),
        editors: s.editors.clone(),
        doc_service: Some(Arc::new(StubDocService::bundle(memo_conversion(
            "media/team.png",
            "Team photo",
        )))),
        search: None,
    };
    let server = McpServer::new(Arc::new(next), ADA.to_string());
    put(&f, "/memo.docx", b"PK docx, second draft").await;
    convert(&server, "/memo.docx").await;
    assert_eq!(
        images(&server, "/memo.docx").await,
        json!({"images": [{"id": "image-1", "caption": "Team photo", "page": null}], "marker": null})
    );
    f.cleanup().await;
}
