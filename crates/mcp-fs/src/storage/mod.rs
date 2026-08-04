//! Storage layer: metadata tree (SQLite), content-addressed blobs (local or S3),
//! ACL registry (SQLite), plus the per-project client cache.

pub mod admin;
pub mod blob;
pub mod meta;
pub mod sqlite;
pub mod traits;
pub mod volume;

pub use traits::{AdminBackend, BlobBackend, Member, MetaBackend, NodeRow, Project};
pub use volume::VolumeClient;

use crate::config::ServerConfig;
use crate::errors::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Build the blob backend for one project, per `infra.blob.backend`.
pub fn build_blob_store(config: &ServerConfig, project_id: &str) -> Result<Arc<dyn BlobBackend>> {
    let bucket = config.volume_bucket(project_id);
    match config.infra.blob.backend.as_str() {
        "local" => Ok(Arc::new(blob::local::LocalBlobStore::new(
            &config.infra.blob.dir,
            &bucket,
        ))),
        "minio" | "s3" => Ok(Arc::new(blob::s3::S3BlobStore::new(
            &config.infra.blob,
            bucket,
        )?)),
        other => Err(crate::errors::ToolError::invalid_argument(format!(
            "unknown blob backend '{other}' (expected local, minio or s3)"
        ))),
    }
}

/// Build the metadata backend for one project, per `infra.meta.backend`.
pub fn build_meta_store(config: &ServerConfig, project_id: &str) -> Result<Arc<dyn MetaBackend>> {
    match config.infra.meta.backend.as_str() {
        "sqlite" => Ok(Arc::new(meta::SqliteMetaStore::open(
            config.volume_meta_path(project_id),
        )?)),
        other => Err(crate::errors::ToolError::invalid_argument(format!(
            "unknown metadata backend '{other}' (expected sqlite)"
        ))),
    }
}

/// Build the ACL registry, per `infra.admin.backend`.
pub fn build_admin_store(config: &ServerConfig) -> Result<Arc<dyn AdminBackend>> {
    match config.infra.admin.backend.as_str() {
        "sqlite" => Ok(Arc::new(admin::SqliteAdminStore::new(config.admin_db_path()))),
        other => Err(crate::errors::ToolError::invalid_argument(format!(
            "unknown admin backend '{other}' (expected sqlite)"
        ))),
    }
}

/// Caches one `VolumeClient` per project and provisions / tears down volumes.
/// Port of the C# `Storage/StoreManager.cs`.
pub struct StoreManager {
    config: Arc<ServerConfig>,
    clients: Mutex<HashMap<String, Arc<VolumeClient>>>,
}

impl StoreManager {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config, clients: Mutex::new(HashMap::new()) }
    }

    /// Get (or open) the client for a project.
    pub async fn client(&self, project_id: &str) -> Result<Arc<VolumeClient>> {
        let mut guard = self.clients.lock().await;
        if let Some(c) = guard.get(project_id) {
            return Ok(c.clone());
        }
        let meta = build_meta_store(&self.config, project_id)?;
        let blob = build_blob_store(&self.config, project_id)?;
        blob.ensure_bucket().await?;
        let client = Arc::new(VolumeClient::new(project_id, meta, blob));
        guard.insert(project_id.to_string(), client.clone());
        Ok(client)
    }

    /// Create the volume for a new project (metadata db + blob bucket).
    pub async fn provision_volume(&self, project_id: &str) -> Result<()> {
        let _ = self.client(project_id).await?;
        Ok(())
    }

    /// Tear a volume down: drop the cached client, remove the bucket and the db file.
    pub async fn teardown_volume(&self, project_id: &str) -> Result<()> {
        let client = {
            let mut guard = self.clients.lock().await;
            guard.remove(project_id)
        };
        let blob = match client {
            Some(c) => c.blob.clone(),
            None => build_blob_store(&self.config, project_id)?,
        };
        blob.remove_bucket().await?;
        let db = self.config.volume_meta_path(project_id);
        for suffix in ["", "-wal", "-shm"] {
            let p = if suffix.is_empty() {
                db.clone()
            } else {
                std::path::PathBuf::from(format!("{}{}", db.display(), suffix))
            };
            let _ = tokio::fs::remove_file(&p).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(root: &std::path::Path) -> Arc<ServerConfig> {
        let mut c = ServerConfig::default();
        c.infra.meta.dir = root.join("volumes").display().to_string();
        c.infra.blob.dir = root.join("blobs").display().to_string();
        c.infra.admin.path = root.join("admin.db").display().to_string();
        Arc::new(c)
    }

    #[tokio::test]
    async fn client_is_cached_per_project() {
        let d = tempfile::tempdir().unwrap();
        let m = StoreManager::new(cfg(d.path()));
        let a = m.client("proj").await.unwrap();
        let b = m.client("proj").await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same client instance is reused");
    }

    #[tokio::test]
    async fn provision_then_teardown_removes_state() {
        let d = tempfile::tempdir().unwrap();
        let config = cfg(d.path());
        let m = StoreManager::new(config.clone());

        m.provision_volume("proj").await.unwrap();
        let c = m.client("proj").await.unwrap();
        c.write_text_atomic("/a.txt", "x").await.unwrap();
        assert!(config.volume_meta_path("proj").exists());

        m.teardown_volume("proj").await.unwrap();
        assert!(!config.volume_meta_path("proj").exists(), "metadata db removed");
        assert!(
            !d.path().join("blobs").join("mcpfs-proj").exists(),
            "blob bucket removed"
        );
    }

    #[tokio::test]
    async fn unknown_backends_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let mut c = (*cfg(d.path())).clone();
        c.infra.blob.backend = "carrier-pigeon".into();
        let e = build_blob_store(&c, "p").err().expect("unknown backend must fail");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);

        let mut c2 = (*cfg(d.path())).clone();
        c2.infra.meta.backend = "punchcards".into();
        assert!(build_meta_store(&c2, "p").is_err());
    }

    #[tokio::test]
    async fn volumes_are_isolated_from_each_other() {
        let d = tempfile::tempdir().unwrap();
        let m = StoreManager::new(cfg(d.path()));
        let a = m.client("proj-a").await.unwrap();
        let b = m.client("proj-b").await.unwrap();
        a.write_text_atomic("/only-in-a.txt", "x").await.unwrap();
        assert!(a.exists("/only-in-a.txt").await.unwrap());
        assert!(!b.exists("/only-in-a.txt").await.unwrap());
    }
}
