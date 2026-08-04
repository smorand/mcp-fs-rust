//! Per project git repository store.
//!
//! Port of the C# `Git/GitRepoStore.cs`. One entry per project, created lazily and
//! cached in process:
//!
//! ```text
//! state/git/{project_id}.db       SqliteGitDb (refs, objects index, remotes)
//! state/git-repos/{project_id}/   bare libgit2 directory (HEAD, config, hooks)
//! ```
//!
//! The authoritative object store is the volume's blob backend (see
//! [`crate::git::odb`]); the bare directory only holds libgit2 bookkeeping plus a
//! rebuildable on disk object cache.
//!
//! Two behaviours are deliberately stricter than the C#:
//!
//! * `get_db` / `write_lock` open the project on demand instead of throwing
//!   "not initialized" when the in process map is cold. The C# threw after a
//!   restart even though the state on disk was intact.
//! * [`Self::purge_repo`] can delete the on disk state. The C# teardown only
//!   dropped the in memory entry, so `state/git/{p}.db` survived a project
//!   deletion and a project recreated under the same id inherited stale refs.
//!   [`Self::teardown_repo`] keeps the old behaviour for callers that want it.

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use crate::git::db::SqliteGitDb;
use crate::git::odb::BlobObjectDb;
use crate::storage::BlobBackend;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

/// Everything needed to serve one project's git traffic.
pub struct GitRepoEntry {
    pub project_id: String,
    /// libgit2 handle. `git2::Repository` is `Send` but not `Sync`, hence the mutex.
    pub repo: Mutex<git2::Repository>,
    pub db: Arc<SqliteGitDb>,
    pub objects: Arc<BlobObjectDb>,
    pub blobs: Arc<dyn BlobBackend>,
    /// Held for the whole of a push, so two concurrent `git-receive-pack` calls
    /// cannot interleave ref updates. Port of the C# `SemaphoreSlim(1, 1)`.
    pub write_lock: Mutex<()>,
}

pub struct GitRepoStore {
    config: Arc<ServerConfig>,
    entries: Mutex<HashMap<String, Arc<GitRepoEntry>>>,
}

static SHARED: OnceLock<Arc<GitRepoStore>> = OnceLock::new();

impl GitRepoStore {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config, entries: Mutex::new(HashMap::new()) }
    }

    /// The process wide instance, matching the C# DI singleton. The composition
    /// root and the `git.*` tools must share one store, otherwise each would keep
    /// its own repository handles and write locks. Tests build their own store
    /// with [`Self::new`] instead, since this one is fixed for the process.
    pub fn shared(config: Arc<ServerConfig>) -> Arc<Self> {
        SHARED.get_or_init(|| Arc::new(Self::new(config))).clone()
    }

    pub fn config(&self) -> &Arc<ServerConfig> {
        &self.config
    }

    /// Create the repo for a new project and point HEAD at `refs/heads/main`.
    /// Idempotent: an already open project is returned untouched.
    pub async fn init_repo(&self, project_id: &str) -> Result<Arc<GitRepoEntry>> {
        let mut guard = self.entries.lock().await;
        if let Some(e) = guard.get(project_id) {
            return Ok(e.clone());
        }
        let entry = self.create_entry(project_id, true).await?;
        guard.insert(project_id.to_string(), entry.clone());
        Ok(entry)
    }

    /// Get the cached entry, opening (and creating if absent) the on disk state.
    pub async fn get_or_open_repo(&self, project_id: &str) -> Result<Arc<GitRepoEntry>> {
        {
            let guard = self.entries.lock().await;
            if let Some(e) = guard.get(project_id) {
                return Ok(e.clone());
            }
        }
        let mut guard = self.entries.lock().await;
        // Re-check: another task may have opened it while we waited for the lock.
        if let Some(e) = guard.get(project_id) {
            return Ok(e.clone());
        }
        let entry = self.create_entry(project_id, false).await?;
        guard.insert(project_id.to_string(), entry.clone());
        Ok(entry)
    }

    /// True when the project has git state, whether or not it is open in process.
    /// File based so it survives a restart, unlike a pure in memory check.
    pub async fn is_initialized(&self, project_id: &str) -> bool {
        {
            let guard = self.entries.lock().await;
            if guard.contains_key(project_id) {
                return true;
            }
        }
        self.config.git_db_path(project_id).exists()
    }

    /// Drop the in process entry, leaving the on disk state alone (C# parity).
    pub async fn teardown_repo(&self, project_id: &str) -> Result<()> {
        let mut guard = self.entries.lock().await;
        guard.remove(project_id);
        Ok(())
    }

    /// Drop the entry and delete the on disk state (index db plus bare repo dir).
    /// Git objects live in the volume's blob bucket, removed by the volume teardown.
    pub async fn purge_repo(&self, project_id: &str) -> Result<()> {
        self.teardown_repo(project_id).await?;
        let db = self.config.git_db_path(project_id);
        for suffix in ["", "-wal", "-shm"] {
            let p = if suffix.is_empty() {
                db.clone()
            } else {
                std::path::PathBuf::from(format!("{}{}", db.display(), suffix))
            };
            let _ = tokio::fs::remove_file(&p).await;
        }
        let _ = tokio::fs::remove_dir_all(self.config.git_repo_dir(project_id)).await;
        Ok(())
    }

    /// The metadata index for a project, opening it if needed.
    pub async fn get_db(&self, project_id: &str) -> Result<Arc<SqliteGitDb>> {
        Ok(self.get_or_open_repo(project_id).await?.db.clone())
    }

    /// The object store for a project, opening it if needed.
    pub async fn get_objects(&self, project_id: &str) -> Result<Arc<BlobObjectDb>> {
        Ok(self.get_or_open_repo(project_id).await?.objects.clone())
    }

    /// How many projects are open in process. Test and diagnostics helper.
    pub async fn open_count(&self) -> usize {
        self.entries.lock().await.len()
    }

    // ── internals ───────────────────────────────────────────────────────────

    async fn create_entry(&self, project_id: &str, init: bool) -> Result<Arc<GitRepoEntry>> {
        let repo_dir = self.config.git_repo_dir(project_id);
        let db_path = self.config.git_db_path(project_id);

        tokio::fs::create_dir_all(&repo_dir).await?;

        // `object_format: sha256` is accepted in config but ignored: the bundled
        // libgit2 build only hashes sha1, exactly like LibGit2Sharp in the C#.
        let repo = if init || git2::Repository::open_bare(&repo_dir).is_err() {
            git2::Repository::init_bare(&repo_dir)
                .map_err(|e| ToolError::internal(format!("git init failed: {e}")))?
        } else {
            git2::Repository::open_bare(&repo_dir)
                .map_err(|e| ToolError::internal(format!("git open failed: {e}")))?
        };

        let db = Arc::new(SqliteGitDb::open(&db_path)?);
        let blobs = crate::storage::build_blob_store(&self.config, project_id)?;
        blobs.ensure_bucket().await?;
        let objects = Arc::new(BlobObjectDb::new(blobs.clone(), db.clone()));

        if init {
            db.set_ref("HEAD", "refs/heads/main", true).await?;
            // Keep the on disk HEAD in agreement with the symbolic ref we serve,
            // so tools using libgit2 directly land on the same default branch.
            let _ = repo.set_head("refs/heads/main");
        }

        Ok(Arc::new(GitRepoEntry {
            project_id: project_id.to_string(),
            repo: Mutex::new(repo),
            db,
            objects,
            blobs,
            write_lock: Mutex::new(()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &std::path::Path) -> Arc<ServerConfig> {
        let mut c = ServerConfig::default();
        c.infra.meta.dir = root.join("state/volumes").display().to_string();
        c.infra.blob.dir = root.join("state/blobs").display().to_string();
        c.infra.admin.path = root.join("state/admin.db").display().to_string();
        c.git.enabled = true;
        Arc::new(c)
    }

    #[tokio::test]
    async fn init_creates_the_expected_layout() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config(d.path());
        let store = GitRepoStore::new(cfg.clone());

        store.init_repo("proj").await.unwrap();

        assert!(cfg.git_db_path("proj").exists(), "index db created");
        assert!(cfg.git_repo_dir("proj").join("HEAD").exists(), "bare repo created");
        assert_eq!(
            cfg.git_db_path("proj"),
            d.path().join("state/git/proj.db"),
            "path layout must match the C#"
        );
        assert_eq!(cfg.git_repo_dir("proj"), d.path().join("state/git-repos/proj"));
    }

    #[tokio::test]
    async fn init_sets_head_symbolic_to_main() {
        let d = tempfile::tempdir().unwrap();
        let store = GitRepoStore::new(config(d.path()));
        let e = store.init_repo("proj").await.unwrap();
        let head = e.db.get_ref("HEAD").await.unwrap().unwrap();
        assert_eq!(head.target, "refs/heads/main");
        assert!(head.symbolic);
    }

    #[tokio::test]
    async fn entries_are_cached_per_project() {
        let d = tempfile::tempdir().unwrap();
        let store = GitRepoStore::new(config(d.path()));
        let a = store.get_or_open_repo("proj").await.unwrap();
        let b = store.get_or_open_repo("proj").await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "one entry per project");
        store.get_or_open_repo("other").await.unwrap();
        assert_eq!(store.open_count().await, 2);
    }

    #[tokio::test]
    async fn is_initialized_tracks_the_db_file() {
        let d = tempfile::tempdir().unwrap();
        let store = GitRepoStore::new(config(d.path()));
        assert!(!store.is_initialized("proj").await);
        store.init_repo("proj").await.unwrap();
        assert!(store.is_initialized("proj").await);
    }

    #[tokio::test]
    async fn reopens_after_in_memory_state_is_dropped() {
        // This is the C# bug that was fixed: a restart must not lose the repo.
        let d = tempfile::tempdir().unwrap();
        let cfg = config(d.path());
        let sha = {
            let store = GitRepoStore::new(cfg.clone());
            let e = store.init_repo("proj").await.unwrap();
            e.db.set_ref("refs/heads/main", "a".repeat(40).as_str(), false).await.unwrap();
            let sha = e.objects.write(git2::ObjectType::Blob, b"persisted").await.unwrap();
            store.teardown_repo("proj").await.unwrap();
            assert_eq!(store.open_count().await, 0);
            // still initialized after teardown, the files are there
            assert!(store.is_initialized("proj").await);
            sha
        };

        // brand new store, cold in memory map
        let store2 = GitRepoStore::new(cfg.clone());
        assert!(store2.is_initialized("proj").await);
        let db = store2.get_db("proj").await.unwrap();
        assert_eq!(
            db.get_ref("refs/heads/main").await.unwrap().unwrap().target,
            "a".repeat(40)
        );
        let objects = store2.get_objects("proj").await.unwrap();
        assert_eq!(objects.read(&sha).await.unwrap().1, b"persisted");
    }

    #[tokio::test]
    async fn purge_removes_on_disk_state() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config(d.path());
        let store = GitRepoStore::new(cfg.clone());
        store.init_repo("proj").await.unwrap();

        store.purge_repo("proj").await.unwrap();
        assert!(!cfg.git_db_path("proj").exists());
        assert!(!cfg.git_repo_dir("proj").exists());
        assert!(!store.is_initialized("proj").await);
        // idempotent
        store.purge_repo("proj").await.unwrap();
    }

    #[tokio::test]
    async fn write_lock_serializes_pushes() {
        let d = tempfile::tempdir().unwrap();
        let store = GitRepoStore::new(config(d.path()));
        let entry = store.init_repo("proj").await.unwrap();

        let guard = entry.write_lock.lock().await;
        let e2 = entry.clone();
        let racer = tokio::spawn(async move {
            let _g = e2.write_lock.lock().await;
            "second"
        });
        // give the racer a chance to run: it must still be blocked
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!racer.is_finished(), "a second push must wait for the first");
        drop(guard);
        assert_eq!(racer.await.unwrap(), "second");
    }

    #[tokio::test]
    async fn open_uses_a_separate_blob_bucket_per_project() {
        let d = tempfile::tempdir().unwrap();
        let store = GitRepoStore::new(config(d.path()));
        let a = store.init_repo("proj-a").await.unwrap();
        let b = store.init_repo("proj-b").await.unwrap();
        let sha = a.objects.write(git2::ObjectType::Blob, b"only in a").await.unwrap();
        assert!(a.objects.exists(&sha).await.unwrap());
        assert!(!b.objects.exists(&sha).await.unwrap(), "volumes stay isolated");
    }
}
