//! SPEC-0025 acceptance tests (US-0027): an index walk skips the top level
//! trash folder, so a deleted file does not come back in `search.query` or
//! inflate `search.status` counts, while an explicit non recursive index of a
//! trash path still works and a nested folder that merely shares the trash
//! folder's name at depth is still indexed (BR-001, BR-002, BR-003, BR-006
//! partial).
//!
//! Driven through the real `McpServer` tool methods, the same path a client
//! calls, with a real `TantivyBm25Backend` on a temp dir: BM25 on Tantivy runs
//! in a plain `make check`, no external database required. A PostgreSQL BM25
//! variant of BT-001 skips, never fails, when `MCPFS_TEST_PG_DSN` is unset.

use crate::config::{Dsn, ServerConfig, backend};
use crate::mcp::server::McpServer;
use crate::safety::SafetyManager;
use crate::search::SearchBackend;
use crate::state::AppState;
use crate::storage::traits::AdminBackend;
use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;

const ADA: &str = "ada@example.com";
const ATLAS: &str = "atlas";
/// How long an auto index hook or a backfill may take before the test calls it broken.
const INDEX_TIMEOUT_SECS: u64 = 15;

/// A server with `ADA` owning `atlas`, and a real BM25 backend wired.
struct Fixture {
    _dir: tempfile::TempDir,
    _tantivy_dir: tempfile::TempDir,
    state: Arc<AppState>,
}

impl Fixture {
    /// `trash_dir`: `None` keeps the default `.mcp_trash`; `Some` overrides it (BT-015).
    async fn new(trash_dir: Option<&str>) -> Self {
        use crate::search::bm25_sqlite::TantivyBm25Backend;

        let dir = tempfile::tempdir().unwrap();
        let tantivy_dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TantivyBm25Backend::new(
            tantivy_dir.path().to_str().expect("temp dir has valid utf-8 path"),
        )) as Arc<dyn SearchBackend>;

        let mut config = ServerConfig::default();
        config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.jwt.public_key_path = String::new();
        config.search.enabled = true;
        config.search.mode = "bm25".into();
        config.search.tantivy_dir = tantivy_dir.path().display().to_string();
        if let Some(td) = trash_dir {
            config.safety.trash_dir = td.into();
        }
        let config = Arc::new(config);

        let admin =
            Arc::new(crate::storage::admin::RelationalAdminStore::in_memory().await.unwrap());
        admin.connect().await.unwrap();
        admin.create_project(ATLAS, ADA).await.unwrap();
        let safety = Arc::new(SafetyManager::new(
            config.safety.clone(),
            crate::storage::meta::max_path_len(&config.infra.meta.backend),
        ));
        safety.attach_limits(admin.clone());
        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(
                config.clone(),
                crate::storage::test_registry(),
            )),
            safety,
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service: None,
            search: Some(backend),
        });
        state.stores.provision_volume(ATLAS).await.unwrap();
        Self { _dir: dir, _tantivy_dir: tantivy_dir, state }
    }

    fn server(&self) -> McpServer {
        McpServer::new(self.state.clone(), ADA.to_string())
    }

    async fn write(&self, path: &str, content: &str) {
        let a = args(json!({
            "mount_id": ATLAS, "path": path, "base64": b64(content.as_bytes()),
            "overwrite": true, "create_parents": true,
        }));
        ok_json(self.server().fs_write_bytes(a).await);
    }

    async fn delete(&self, path: &str) {
        let a = args(json!({"mount_id": ATLAS, "path": path}));
        ok_json(self.server().fs_delete(a).await);
    }

    async fn mv(&self, source: &str, destination: &str) {
        let a = args(json!({"mount_id": ATLAS, "source": source, "destination": destination}));
        ok_json(self.server().fs_move(a).await);
    }

    async fn set_mode(&self, mode: &str) {
        let a = args(json!({"project_id": ATLAS, "mode": mode}));
        ok_json(self.server().admin_set_index_mode(a).await);
    }

    async fn index(&self, path: &str, recursive: bool) -> Value {
        let a = args(json!({"mount_id": ATLAS, "path": path, "recursive": recursive}));
        ok_json(self.server().search_index(a).await)
    }

    async fn query(&self, query: &str) -> Value {
        let a = args(json!({"mount_id": ATLAS, "query": query, "mode": "bm25"}));
        ok_json(self.server().search_query(a).await)
    }

    async fn status(&self) -> Value {
        ok_json(self.server().search_status(args(json!({"mount_id": ATLAS}))).await)
    }

    async fn trash_list(&self) -> Value {
        ok_json(self.server().fs_trash_list(args(json!({"mount_id": ATLAS}))).await)
    }

    async fn trash_restore(&self, trash_path: &str) -> Value {
        let a = args(json!({"mount_id": ATLAS, "trash_path": trash_path}));
        ok_json(self.server().fs_trash_restore(a).await)
    }

    /// Ages `path`'s `atime` by `age_seconds` so the auto purge sweep sees it
    /// as stale (SPEC-0014's staleness threshold is `now - atime`).
    async fn age_atime(&self, path: &str, age_seconds: f64) {
        let client = self.state.stores.client(ATLAS).await.unwrap();
        client
            .trash
            .as_ref()
            .expect("the test harness attaches a trash store")
            .set_atime_for_test(path, crate::util::now_unix() - age_seconds)
            .await
            .unwrap();
    }

    async fn set_purge_config(&self, file_retention_days: i64) {
        self.state
            .admin
            .set_purge_config(
                ATLAS,
                crate::storage::traits::PurgeConfig {
                    autopurge_enabled: true,
                    use_internal_purge: true,
                    file_retention_days: Some(file_retention_days),
                    project_retention_days: None,
                },
            )
            .await
            .unwrap();
    }

    /// Runs the real purge cycle (DEC-004): the same `run_cycle` the background
    /// loop and the CLI `purge` verb call.
    async fn run_purge_cycle(&self) {
        crate::purge::run_cycle(
            &self.state.stores,
            self.state.admin.as_ref(),
            &self.state.safety,
            self.state.search.as_ref(),
        )
        .await
        .unwrap();
    }

    /// The current `trash_path` of the entry whose `original_path` is `original`.
    async fn trash_path_of(&self, original: &str) -> String {
        let listed = self.trash_list().await;
        listed["entries"]
            .as_array()
            .expect("entries is an array")
            .iter()
            .find(|e| e["original_path"].as_str() == Some(original))
            .unwrap_or_else(|| panic!("no trash entry for {original}: {listed}"))["trash_path"]
            .as_str()
            .expect("trash_path is a string")
            .to_string()
    }

    async fn bm25_docs(&self) -> u64 {
        self.status().await["bm25_docs"].as_u64().unwrap_or(0)
    }

    /// Wait until `search.status` reports exactly `expected` bm25 docs.
    ///
    /// A mode change backfill and an auto index hook are both fire and forget,
    /// so the index lags a write or a backfill start by one pass. A timeout
    /// FAILS the test rather than passing on a stale count.
    async fn await_bm25_docs(&self, expected: u64) {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(INDEX_TIMEOUT_SECS);
        let mut last = u64::MAX;
        while std::time::Instant::now() < deadline {
            last = self.bm25_docs().await;
            if last == expected {
                // Settled: give a straggler task a beat to add a doc we would
                // then see as a wrong count rather than as a pass.
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                let again = self.bm25_docs().await;
                assert_eq!(
                    again, expected,
                    "atlas: bm25_docs moved to {again} after settling at {expected}"
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("atlas: bm25_docs never reached {expected}, last saw {last}");
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

fn results_of(found: &Value) -> Vec<String> {
    found["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results is an array: {found}"))
        .iter()
        .map(|r| r["path"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// SPEC-0025/BT-001: a deleted file does not come back after a mode change.
#[tokio::test]
async fn spec_0025_bt_001_a_deleted_file_does_not_come_back_after_a_mode_change() {
    let f = Fixture::new(None).await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.write("/plan.txt", "merger with Borealis").await;
    f.delete("/memo.txt").await;

    f.set_mode("bm25").await;
    f.await_bm25_docs(1).await;

    let found = f.query("forecast").await;
    assert_eq!(results_of(&found), Vec::<String>::new(), "got: {found}");
    assert_eq!(f.status().await["bm25_docs"], json!(1));
}

/// SPEC-0025/BT-010: a recursive index of the root skips the trash.
#[tokio::test]
async fn spec_0025_bt_010_recursive_index_of_the_root_skips_the_trash() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.write("/plan.txt", "merger with Borealis").await;
    f.await_bm25_docs(2).await;
    f.delete("/memo.txt").await;
    f.await_bm25_docs(1).await;

    let report = f.index("/", true).await;
    assert_eq!(report["skipped"], json!(0), "got: {report}");

    let forecast = f.query("forecast").await;
    assert_eq!(results_of(&forecast), Vec::<String>::new(), "got: {forecast}");
    let borealis = f.query("Borealis").await;
    assert_eq!(results_of(&borealis), vec!["/plan.txt".to_string()], "got: {borealis}");
}

/// SPEC-0025/BT-011: an explicit, non recursive index of one trash file still works.
#[tokio::test]
async fn spec_0025_bt_011_explicit_index_of_one_trash_file_still_works() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.await_bm25_docs(1).await;
    f.delete("/memo.txt").await;
    f.await_bm25_docs(0).await;

    let trash_path = f.trash_path_of("/memo.txt").await;
    f.index(&trash_path, false).await;
    f.await_bm25_docs(1).await;

    let found = f.query("forecast").await;
    assert_eq!(results_of(&found), vec![trash_path.clone()], "got: {found}");
}

/// SPEC-0025/BT-012: a move into the trash folder removes the source's chunks
/// and does not index the destination.
#[tokio::test]
async fn spec_0025_bt_012_move_into_the_trash_folder() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;
    f.write("/plan.txt", "merger with Borealis").await;
    f.await_bm25_docs(1).await;

    f.mv("/plan.txt", "/.mcp_trash/manual.txt").await;
    f.await_bm25_docs(0).await;

    let found = f.query("Borealis").await;
    assert_eq!(results_of(&found), Vec::<String>::new(), "got: {found}");
}

/// SPEC-0025/BT-015: a custom trash folder is skipped the same way.
#[tokio::test]
async fn spec_0025_bt_015_custom_trash_folder() {
    let f = Fixture::new(Some(".bin")).await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    // A live file, so the wait below means "the backfill finished": waiting
    // for 0 documents would pass before the backfill even started.
    f.write("/plan.txt", "merger with Borealis").await;
    f.delete("/memo.txt").await;

    f.set_mode("bm25").await;
    f.await_bm25_docs(1).await;

    let found = f.query("forecast").await;
    assert_eq!(results_of(&found), Vec::<String>::new(), "got: {found}");
}

/// SPEC-0025/BT-016: a nested folder that merely shares the trash folder's
/// name at depth is a normal, live folder and is indexed.
#[tokio::test]
async fn spec_0025_bt_016_nested_folder_named_like_the_trash_is_indexed() {
    let f = Fixture::new(None).await;
    f.write("/docs/.mcp_trash/a.txt", "nested keep").await;

    f.set_mode("bm25").await;
    f.await_bm25_docs(1).await;

    let found = f.query("nested").await;
    assert_eq!(results_of(&found), vec!["/docs/.mcp_trash/a.txt".to_string()], "got: {found}");
}

/// SPEC-0025/BT-020: non regression, `fs.delete` still clears and `fs.write_bytes`
/// still indexes.
#[tokio::test]
async fn spec_0025_bt_020_delete_still_clears_write_still_indexes() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;

    f.write("/a.txt", "alpha content").await;
    f.await_bm25_docs(1).await;
    let found = f.query("alpha").await;
    assert_eq!(results_of(&found), vec!["/a.txt".to_string()], "got: {found}");

    f.delete("/a.txt").await;
    f.await_bm25_docs(0).await;
    let empty = f.query("alpha").await;
    assert_eq!(results_of(&empty), Vec::<String>::new(), "got: {empty}");

    f.write("/b.txt", "beta content").await;
    f.await_bm25_docs(1).await;
    let found = f.query("beta").await;
    assert_eq!(results_of(&found), vec!["/b.txt".to_string()], "got: {found}");
}

/// SPEC-0025/BT-002: a restored file is found again.
#[tokio::test]
async fn spec_0025_bt_002_a_restored_file_is_found_again() {
    let f = Fixture::new(None).await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.write("/plan.txt", "merger with Borealis").await;
    f.set_mode("bm25").await;
    f.await_bm25_docs(2).await;
    f.delete("/memo.txt").await;
    f.await_bm25_docs(1).await;

    let trash_path = f.trash_path_of("/memo.txt").await;
    f.trash_restore(&trash_path).await;
    f.await_bm25_docs(2).await;

    let found = f.query("forecast").await;
    assert_eq!(results_of(&found), vec!["/memo.txt".to_string()], "got: {found}");
}

/// SPEC-0025/BT-013: the auto purge removes the stale file from search.
#[tokio::test]
async fn spec_0025_bt_013_auto_purge_removes_the_stale_file_from_search() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;
    f.write("/old.txt", "obsolete roadmap").await;
    f.await_bm25_docs(1).await;
    f.age_atime("/old.txt", 2.0 * 86_400.0).await;
    f.set_purge_config(1).await;

    f.run_purge_cycle().await;
    f.await_bm25_docs(0).await;

    let found = f.query("roadmap").await;
    assert_eq!(results_of(&found), Vec::<String>::new(), "got: {found}");
    let listed = f.trash_list().await;
    let originals: Vec<&str> = listed["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .map(|e| e["original_path"].as_str().unwrap_or_default())
        .collect();
    assert!(originals.contains(&"/old.txt"), "got: {listed}");
}

/// SPEC-0025/BT-014: restoring onto a taken path keeps both files searchable.
#[tokio::test]
async fn spec_0025_bt_014_restore_onto_a_taken_path() {
    let f = Fixture::new(None).await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.set_mode("bm25").await;
    f.await_bm25_docs(1).await;
    f.delete("/memo.txt").await;
    f.await_bm25_docs(0).await;

    let trash_path = f.trash_path_of("/memo.txt").await;
    f.write("/memo.txt", "fresh notes").await;
    f.await_bm25_docs(1).await;

    f.trash_restore(&trash_path).await;
    f.await_bm25_docs(2).await;

    let forecast = f.query("forecast").await;
    assert_eq!(results_of(&forecast), vec!["/memo.txt_restored".to_string()], "got: {forecast}");
    let fresh = f.query("fresh").await;
    assert_eq!(results_of(&fresh), vec!["/memo.txt".to_string()], "got: {fresh}");
}

// ── PostgreSQL BM25 variant (optional) ──────────────────────────────────────

/// A PostgreSQL twin of the [`Fixture`], the metadata store on a fresh schema
/// and the BM25 index on `search_fts`. `None` when `MCPFS_TEST_PG_DSN` is
/// unset or the `postgres` feature is off: this variant skips, never fails.
struct PgFixture {
    inner: Fixture,
    dsn: String,
    schema: String,
}

impl PgFixture {
    async fn new(test: &str) -> Option<Self> {
        let Some(dsn) = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
        else {
            eprintln!("skipping {test}: MCPFS_TEST_PG_DSN is unset");
            return None;
        };
        if !cfg!(feature = "postgres") {
            eprintln!("skipping {test}: built without --features postgres");
            return None;
        }
        let schema = format!("mcpfs_trash_search_{test}_{}", std::process::id());
        drop_schema(&dsn, &schema).await;

        let dir = tempfile::tempdir().unwrap();
        let tantivy_dir = tempfile::tempdir().unwrap();

        let mut config = ServerConfig::default();
        config.infra.meta.backend = backend::POSTGRES.into();
        config.infra.meta.dsn = Dsn::new(dsn.clone());
        config.infra.meta.schema = schema.clone();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.jwt.public_key_path = String::new();
        config.search.enabled = true;
        config.search.mode = "bm25".into();
        config.search.tantivy_dir = tantivy_dir.path().display().to_string();
        let config = Arc::new(config);

        let pool = open_schema(&dsn, &schema).await;
        let backend = pg_bm25_backend(pool).await;

        let admin =
            Arc::new(crate::storage::admin::RelationalAdminStore::in_memory().await.unwrap());
        admin.connect().await.unwrap();
        admin.create_project(ATLAS, ADA).await.unwrap();
        let safety = Arc::new(SafetyManager::new(
            config.safety.clone(),
            crate::storage::meta::max_path_len(&config.infra.meta.backend),
        ));
        safety.attach_limits(admin.clone());
        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(
                config.clone(),
                crate::storage::test_registry(),
            )),
            safety,
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service: None,
            search: Some(backend),
        });
        state.stores.provision_volume(ATLAS).await.unwrap();

        Some(Self { inner: Fixture { _dir: dir, _tantivy_dir: tantivy_dir, state }, dsn, schema })
    }

    async fn cleanup(self) {
        drop_schema(&self.dsn, &self.schema).await;
    }
}

#[cfg(feature = "postgres")]
async fn pg_bm25_backend(pool: sqlx::PgPool) -> Arc<dyn SearchBackend> {
    Arc::new(
        crate::search::bm25_pg::PostgresBm25Backend::new(pool)
            .await
            .expect("bm25 backend must build"),
    )
}

#[cfg(not(feature = "postgres"))]
async fn pg_bm25_backend(_pool: ()) -> Arc<dyn SearchBackend> {
    unreachable!("PgFixture::new skips before touching a database without the feature")
}

#[cfg(feature = "postgres")]
async fn open_schema(dsn: &str, schema: &str) -> sqlx::PgPool {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    PostgresRelationalDb::connect(dsn, schema, PoolSettings::default())
        .await
        .expect("MCPFS_TEST_PG_DSN is set but connection failed")
        .pool()
        .clone()
}

#[cfg(not(feature = "postgres"))]
async fn open_schema(_dsn: &str, _schema: &str) {
    // Unreachable: PgFixture::new skips before touching a database without the feature.
}

#[cfg(feature = "postgres")]
async fn drop_schema(dsn: &str, schema: &str) {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb, Query, RelationalDb as _};
    let Ok(db) = PostgresRelationalDb::connect(dsn, "public", PoolSettings::default()).await else {
        return;
    };
    let _ = db.execute(&Query::new(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))).await;
}

#[cfg(not(feature = "postgres"))]
async fn drop_schema(_dsn: &str, _schema: &str) {}

/// SPEC-0025/BT-001, the PostgreSQL twin: the same scenario, on a real
/// `search_fts` table.
#[tokio::test]
async fn spec_0025_bt_001_pg_a_deleted_file_does_not_come_back_after_a_mode_change() {
    let Some(f) = PgFixture::new("spec_0025_bt_001_pg").await else { return };
    f.inner.write("/memo.txt", "quarterly forecast draft").await;
    f.inner.write("/plan.txt", "merger with Borealis").await;
    f.inner.delete("/memo.txt").await;

    f.inner.set_mode("bm25").await;
    f.inner.await_bm25_docs(1).await;

    let found = f.inner.query("forecast").await;
    assert_eq!(results_of(&found), Vec::<String>::new(), "got: {found}");
    assert_eq!(f.inner.status().await["bm25_docs"], json!(1));
    f.cleanup().await;
}

/// SPEC-0025/BT-021: non regression, the trash listing and the restore answer
/// exactly what they answered before the index started following the trash.
#[tokio::test]
async fn spec_0025_bt_021_trash_listing_and_restore_unchanged() {
    let f = Fixture::new(None).await;
    f.set_mode("bm25").await;
    f.write("/memo.txt", "quarterly forecast draft").await;
    f.await_bm25_docs(1).await;
    f.delete("/memo.txt").await;

    let listed = f.trash_list().await;
    assert_eq!(listed["total"], json!(1), "got: {listed}");
    let entry = &listed["entries"][0];
    assert_eq!(entry["original_path"], json!("/memo.txt"), "got: {listed}");
    assert_eq!(entry["size"], json!(24), "got: {listed}");
    assert_eq!(entry["kind"], json!("file"), "got: {listed}");
    let trash_path = entry["trash_path"].as_str().expect("trash_path is a string").to_string();
    assert!(trash_path.starts_with("/.mcp_trash/"), "got: {trash_path}");

    let restored = f.trash_restore(&trash_path).await;
    assert_eq!(restored, json!({"restored_path": "/memo.txt", "trash_path": trash_path}));
    assert_eq!(f.trash_list().await["total"], json!(0));
}
