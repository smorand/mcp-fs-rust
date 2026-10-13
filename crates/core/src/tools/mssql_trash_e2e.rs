//! SPEC-0023 acceptance test: a metadata store on SQL Server opens, and a file
//! deleted there is listed by `fs.trash_list`, driven through the tool dispatch.
//!
//! The metadata store is the shared SQL Server database named by
//! `MCPFS_TEST_MSSQL_DSN`; the ACL registry stays in memory. Rows of the test
//! volume are removed before and after, so a run that panicked leaves nothing a
//! later run can trip on. Skips, never fails, when the DSN is unset or the
//! `sqlserver` feature is off.

use crate::config::{Dsn, ServerConfig, backend};
use crate::errors::Result;
use crate::state::AppState;
use crate::storage::traits::AdminBackend;
use crate::tools::registry_support::{Args, ToolCtx, ToolRegistry};
use serde_json::{Value, json};
use std::sync::Arc;

const ADA: &str = "ada@example.com";
const ATLAS: &str = "atlas";

/// A server state whose metadata store is SQL Server, `ADA` owning `ATLAS`.
struct MssqlFixture {
    _dir: tempfile::TempDir,
    state: Arc<AppState>,
    registry: ToolRegistry,
    dsn: String,
}

impl MssqlFixture {
    /// `None` when the test must skip, after saying why.
    async fn new(test: &str) -> Option<Self> {
        let Some(dsn) = std::env::var("MCPFS_TEST_MSSQL_DSN").ok().filter(|v| !v.trim().is_empty())
        else {
            eprintln!("skipping {test}: MCPFS_TEST_MSSQL_DSN is unset");
            return None;
        };
        if !cfg!(feature = "sqlserver") {
            eprintln!("skipping {test}: built without --features sqlserver");
            return None;
        }
        purge_volume(&dsn, ATLAS).await;

        let dir = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.infra.meta.backend = backend::SQLSERVER.into();
        config.infra.meta.dsn = Dsn::new(dsn.clone());
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.jwt.public_key_path = String::new();
        let config = Arc::new(config);

        let admin =
            Arc::new(crate::storage::admin::RelationalAdminStore::in_memory().await.unwrap());
        admin.connect().await.unwrap();
        admin.create_project(ATLAS, ADA).await.unwrap();
        let safety = crate::safety::SafetyManager::new(
            config.safety.clone(),
            crate::storage::meta::max_path_len(&config.infra.meta.backend),
        );
        safety.attach_limits(admin.clone());
        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(
                config.clone(),
                crate::storage::test_registry(),
            )),
            safety: Arc::new(safety),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
            doc_service: None,
            search: None,
        });
        // Before the fix this is where the store failed to open, with 1919.
        state.stores.provision_volume(ATLAS).await.unwrap();

        let mut registry = ToolRegistry::new();
        super::register_fs(&mut registry);
        Some(Self { _dir: dir, state, registry, dsn })
    }

    /// Dispatch a tool as `ADA`, like `tools/call` does.
    async fn call(&self, name: &str, args: Value) -> Result<Value> {
        let ctx = ToolCtx { person: ADA.to_string(), state: self.state.clone() };
        self.registry
            .call(name, ctx, Args::new(args))
            .await
            .unwrap_or_else(|| panic!("tool '{name}' is not registered"))
    }

    async fn cleanup(self) {
        purge_volume(&self.dsn, ATLAS).await;
    }
}

/// Remove every metadata row of `volume`, table by table, in the shared database.
#[cfg(feature = "sqlserver")]
async fn purge_volume(dsn: &str, volume: &str) {
    use crate::storage::rel::{PoolSettings, Query, RelationalDb as _, SqlServerRelationalDb};
    let db = SqlServerRelationalDb::connect(dsn, PoolSettings::default()).await.unwrap();
    for table in crate::storage::meta::schema().tables {
        if !table.columns.iter().any(|c| c.name == "volume_id") {
            continue;
        }
        // A table absent before the first open (a fresh database) has nothing to purge.
        let sql = format!(
            "IF OBJECT_ID(N'{t}', N'U') IS NOT NULL DELETE FROM [{t}] WHERE volume_id=?1",
            t = table.name
        );
        db.execute(&Query::new(sql).bind(volume)).await.unwrap();
    }
}

#[cfg(not(feature = "sqlserver"))]
async fn purge_volume(_dsn: &str, _volume: &str) {
    // Unreachable: the fixture skips before touching a database without the feature.
}

/// SPEC-0023/BT-001: a file deleted on a SQL Server store is listed in the
/// trash, newest delete first (BR-001, BR-002).
#[tokio::test]
async fn spec_0023_bt_001_deleted_files_are_listed_in_the_trash() {
    let Some(f) = MssqlFixture::new("spec_0023_bt_001").await else { return };
    for path in ["/a.txt", "/b.txt"] {
        // "YWJj" is the 3 bytes "abc".
        f.call("fs.write_bytes", json!({"mount_id": ATLAS, "path": path, "base64": "YWJj"}))
            .await
            .unwrap_or_else(|e| panic!("write {path}: {e:?}"));
    }
    for path in ["/a.txt", "/b.txt"] {
        f.call("fs.delete", json!({"mount_id": ATLAS, "path": path}))
            .await
            .unwrap_or_else(|e| panic!("delete {path}: {e:?}"));
    }

    let listed = f.call("fs.trash_list", json!({"mount_id": ATLAS})).await.unwrap();
    assert_eq!(listed["total"], json!(2), "got: {listed}");
    let entries = listed["entries"].as_array().expect("entries is an array");
    let originals: Vec<&str> =
        entries.iter().map(|e| e["original_path"].as_str().unwrap_or_default()).collect();
    assert_eq!(originals, vec!["/b.txt", "/a.txt"], "got: {listed}");
    for e in entries {
        let at = e["deleted_at"].as_str().expect("deleted_at is text");
        assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok(), "RFC 3339 deleted_at: {at}");
        assert_eq!(e["size"], json!(3), "got: {e}");
    }
    f.cleanup().await;
}
