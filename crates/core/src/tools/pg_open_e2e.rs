//! SPEC-0026 acceptance tests: opening a project's metadata store on a shared
//! PostgreSQL schema takes no table lock once the schema is current, so first
//! requests neither deadlock nor wait behind another project's writes. Driven
//! through the real `McpServer` tool methods.
//!
//! Each test runs on its own PostgreSQL schema, holding both the admin registry
//! and the volume metadata. They skip, never fail, when `MCPFS_TEST_PG_DSN` is
//! unset or the `postgres` feature is off.

use crate::config::{Dsn, ServerConfig, backend};
use crate::identity::IdentityResolver;
use crate::mcp::server::{McpServer, ReadBytesArgs, WriteBytesArgs};
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::StoreManager;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

const ADA: &str = "ada@example.com";
/// The project writing continuously while the others open.
const BUSY: &str = "p0";
/// Projects opened at once per round, and rounds (BT-001).
const FANOUT: usize = 8;
const ROUNDS: usize = 5;
/// Ceiling on one first request: today's stall would otherwise hang the suite.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// BT-002's bound on an open while another transaction holds `nodes`.
const OPEN_BOUND: Duration = Duration::from_secs(2);

struct PgFixture {
    _dir: tempfile::TempDir,
    state: Arc<AppState>,
    dsn: String,
    schema: String,
}

impl PgFixture {
    /// `None` when the suite must skip, after saying why.
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
        let schema = format!("mcpfs_pgopen_{test}_{}", std::process::id());
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
        let safety = Arc::new(SafetyManager::new(config.safety.clone()));
        safety.attach_limits(admin.clone());
        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(StoreManager::new(config.clone(), registry)),
            safety,
            identity: Arc::new(IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service: None,
            search: None,
        });
        // p0 is opened here, which applies the schema once: every later open
        // finds it current.
        state.admin.create_project(BUSY, ADA).await.unwrap();
        state.stores.provision_volume(BUSY).await.unwrap();
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
async fn pool(dsn: &str, schema: &str) -> crate::storage::rel::PostgresRelationalDb {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    PostgresRelationalDb::connect(dsn, schema, PoolSettings::default()).await.unwrap()
}

#[cfg(feature = "postgres")]
async fn drop_schema(dsn: &str, schema: &str) {
    use crate::storage::rel::{Query, RelationalDb as _};
    let db = pool(dsn, "public").await;
    db.execute(&Query::new(format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))).await.unwrap();
}

#[cfg(not(feature = "postgres"))]
async fn drop_schema(_dsn: &str, _schema: &str) {
    // Unreachable: the fixture skips before any schema exists without the feature.
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

/// The tool's JSON on success, its error text otherwise.
fn outcome(r: Result<CallToolResult, rmcp::ErrorData>) -> Result<Value, String> {
    let r = r.map_err(|e| format!("protocol error: {e:?}"))?;
    if r.is_error == Some(true) {
        return Err(text_of(&r));
    }
    serde_json::from_str(&text_of(&r)).map_err(|e| format!("not JSON: {e}"))
}

fn write_args(mount_id: &str, path: &str, overwrite: bool) -> Parameters<WriteBytesArgs> {
    let v = json!({"mount_id": mount_id, "path": path, "base64": "YQ==", "overwrite": overwrite});
    Parameters(serde_json::from_value(v).unwrap())
}

fn read_args(mount_id: &str, path: &str) -> Parameters<ReadBytesArgs> {
    Parameters(serde_json::from_value(json!({"mount_id": mount_id, "path": path})).unwrap())
}

/// SPEC-0026/BT-001: eight first requests at once, five rounds, while another
/// project writes continuously, all succeed (BR-001, BR-002).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn spec_0026_bt_001_concurrent_first_requests_all_succeed() {
    let Some(f) = PgFixture::new("bt001").await else { return };
    let stop = Arc::new(AtomicBool::new(false));
    let loops = Arc::new(AtomicU64::new(0));
    let busy = {
        let (server, stop, loops) = (f.server(), stop.clone(), loops.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                outcome(server.fs_write_bytes(write_args(BUSY, "/loop.txt", true)).await)
                    .expect("the busy project's write");
                loops.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        let ids: Vec<String> = (1..=FANOUT).map(|n| format!("r{round}p{n}")).collect();
        for id in &ids {
            f.state.admin.create_project(id, ADA).await.unwrap();
        }
        let calls = ids.iter().map(|id| {
            let (server, id) = (f.server(), id.clone());
            tokio::spawn(async move {
                let call = server.fs_write_bytes(write_args(&id, "/a.txt", false));
                match tokio::time::timeout(CALL_TIMEOUT, call).await {
                    Ok(r) => outcome(r).map(|_| id.clone()).map_err(|e| format!("{id}: {e}")),
                    Err(_) => Err(format!("{id}: stalled beyond {CALL_TIMEOUT:?}")),
                }
            })
        });
        for done in futures::future::join_all(calls).await {
            match done.unwrap() {
                Ok(id) => {
                    let read = outcome(f.server().fs_read_bytes(read_args(&id, "/a.txt")).await)
                        .unwrap_or_else(|e| panic!("{id}: read back failed: {e}"));
                    assert_eq!(read["base64"], json!("YQ=="), "{id}: got {read}");
                }
                Err(e) => failures.push(format!("round {round}: {e}")),
            }
        }
    }

    stop.store(true, Ordering::SeqCst);
    busy.await.unwrap();
    assert!(loops.load(Ordering::SeqCst) > 0, "the busy project never wrote");
    f.cleanup().await;
    assert!(
        failures.is_empty(),
        "{} of {} calls failed: {failures:#?}",
        failures.len(),
        FANOUT * ROUNDS
    );
}

/// SPEC-0026/BT-002: a never opened project's first write succeeds within two
/// seconds while another connection holds an uncommitted insert into `nodes`
/// (BR-001).
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0026_bt_002_open_while_another_write_is_uncommitted() {
    use crate::storage::rel::{Query, RelationalDb as _};
    let Some(f) = PgFixture::new("bt002").await else { return };
    f.state.admin.create_project("p9", ADA).await.unwrap();

    // The held transaction copies p0's root row under a new path, so it needs
    // no knowledge of the `nodes` column list.
    let other = pool(&f.dsn, &f.schema).await;
    let mut held = other.begin().await.unwrap();
    held.execute(
        &Query::new(
            "INSERT INTO nodes SELECT (jsonb_populate_record(NULL::nodes, \
         to_jsonb(n) || '{\"path\": \"/held.txt\"}'::jsonb)).* \
         FROM nodes n WHERE volume_id = ?1 AND path = '/'",
        )
        .bind(BUSY),
    )
    .await
    .unwrap();

    let started = std::time::Instant::now();
    let server = f.server();
    let first =
        tokio::time::timeout(OPEN_BOUND, server.fs_write_bytes(write_args("p9", "/a.txt", false)))
            .await;
    let elapsed = started.elapsed();
    // Released on every path, before any assertion can unwind past it.
    drop(held);

    let r = match first {
        Ok(r) => outcome(r),
        Err(_) => Err(format!("still waiting after {elapsed:?}, bound {OPEN_BOUND:?}")),
    };
    f.cleanup().await;
    let r = r.expect("p9's first write while p0's insert is uncommitted");
    assert!(r.is_object(), "got {r}");
}
