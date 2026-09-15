//! Storage layer: metadata tree, content-addressed blobs (local or S3), the ACL
//! registry, plus the per-project client cache.
//!
//! Relational state (metadata, ACL, git index, OAuth tokens) goes through
//! [`rel::RelationalDb`], so the engine behind each store is a configuration
//! choice. Blobs are unaffected: bytes never enter a relational database.

pub mod admin;
pub mod blob;
/// One set of store assertions run against every engine.
#[cfg(test)]
mod conformance;
pub mod meta;
pub mod rel;
pub mod sqlite;
pub mod traits;
pub mod volume;

pub use traits::{AdminBackend, BlobBackend, Member, MetaBackend, NodeRow, Project};
pub use volume::VolumeClient;

use crate::config::{Dsn, PoolConfig, ServerConfig, backend};
use crate::errors::{Result, ToolError};
use rel::RelationalDb;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Caches one engine handle per distinct connection target.
///
/// A metadata store is built per volume, so without this a PostgreSQL deployment
/// would open one connection pool per project and exhaust the server's connection
/// limit. Handles are cheap to clone and safe to share: the pool is internally
/// reference counted.
///
/// SQLite is deliberately NOT cached. Its handle owns a file that
/// [`StoreManager::teardown_volume`] deletes, and a cached handle would keep that
/// file open, so each volume keeps getting its own fresh handle exactly as before.
#[derive(Default)]
pub struct RelationalRegistry {
    /// Gated on having ANY SQL driver, not on one of them: only a SQL engine is
    /// ever cached, and with no driver at all there is nothing to share, so the
    /// map would be permanently empty.
    #[cfg(any(feature = "postgres", feature = "sqlserver"))]
    shared: Mutex<HashMap<String, Arc<dyn RelationalDb>>>,
}

impl RelationalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct engine handles are cached.
    ///
    /// Exists so a test can prove that the whole process shares one pool per DSN:
    /// counting is the only externally visible difference between one shared
    /// registry and several private ones, since each private one would cache the
    /// same target separately.
    #[cfg(any(feature = "postgres", feature = "sqlserver"))]
    pub async fn pool_count(&self) -> usize {
        self.shared.lock().await.len()
    }

    /// Always 0 without a SQL driver: nothing is ever cached, so there is no pool
    /// to count and callers do not need to be feature gated themselves.
    #[cfg(not(any(feature = "postgres", feature = "sqlserver")))]
    pub async fn pool_count(&self) -> usize {
        0
    }

    /// A stable key for one connection target.
    ///
    /// The DSN is hashed rather than stored: a map key is easy to print by
    /// accident and a DSN carries a password.
    #[cfg(any(feature = "postgres", feature = "sqlserver"))]
    fn key(backend: &str, dsn: &Dsn, schema: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(backend.as_bytes());
        h.update([0]);
        h.update(dsn.expose().as_bytes());
        h.update([0]);
        h.update(schema.as_bytes());
        format!("{backend}:{:x}", h.finalize())
    }
}

/// A private pool cache, for a test that builds one store directly and does not care
/// about sharing. Production code takes the process wide registry from the
/// composition root instead, which is what keeps the connection count honest.
#[cfg(test)]
pub(crate) fn test_registry() -> Arc<RelationalRegistry> {
    Arc::new(RelationalRegistry::new())
}

/// Open, or reuse, the relational engine one store block points at.
///
/// `sqlite_path` is only consulted for the SQLite backend, where a store is a
/// file rather than a connection.
async fn build_relational_db(
    registry: &RelationalRegistry,
    section: &str,
    store: RelationalTarget<'_>,
    sqlite_path: impl FnOnce() -> PathBuf,
) -> Result<Arc<dyn RelationalDb>> {
    match store.backend {
        backend::SQLITE => Ok(Arc::new(rel::SqliteRelationalDb::open(sqlite_path())?)),
        backend::POSTGRES => build_postgres(registry, section, store).await,
        backend::SQLSERVER => build_sqlserver(registry, section, store).await,
        other => Err(unknown_backend(section, other)),
    }
}

/// One `infra.<section>` block, narrowed to what opening a connection needs.
#[derive(Clone, Copy)]
struct RelationalTarget<'a> {
    backend: &'a str,
    dsn: &'a Dsn,
    schema: &'a str,
    pool: &'a PoolConfig,
}

#[cfg(feature = "postgres")]
async fn build_postgres(
    registry: &RelationalRegistry,
    _section: &str,
    store: RelationalTarget<'_>,
) -> Result<Arc<dyn RelationalDb>> {
    let key = RelationalRegistry::key(backend::POSTGRES, store.dsn, store.schema);
    let mut guard = registry.shared.lock().await;
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.clone());
    }
    let db = rel::PostgresRelationalDb::connect(
        store.dsn.expose(),
        store.schema,
        rel::PoolSettings {
            max_connections: store.pool.max_connections,
            acquire_timeout: std::time::Duration::from_secs(store.pool.acquire_timeout_secs),
        },
    )
    .await?;
    let db: Arc<dyn RelationalDb> = Arc::new(db);
    guard.insert(key, db.clone());
    Ok(db)
}

#[cfg(feature = "sqlserver")]
async fn build_sqlserver(
    registry: &RelationalRegistry,
    _section: &str,
    store: RelationalTarget<'_>,
) -> Result<Arc<dyn RelationalDb>> {
    // SQL Server has no schema selector of its own here: an ADO.NET DSN names the
    // database directly, so the key uses the section's schema only for uniqueness.
    let key = RelationalRegistry::key(backend::SQLSERVER, store.dsn, store.schema);
    let mut guard = registry.shared.lock().await;
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.clone());
    }
    let db = rel::SqlServerRelationalDb::connect(
        store.dsn.expose(),
        rel::PoolSettings {
            max_connections: store.pool.max_connections,
            acquire_timeout: std::time::Duration::from_secs(store.pool.acquire_timeout_secs),
        },
    )
    .await?;
    let db: Arc<dyn RelationalDb> = Arc::new(db);
    guard.insert(key, db.clone());
    Ok(db)
}

#[cfg(not(feature = "sqlserver"))]
async fn build_sqlserver(
    _registry: &RelationalRegistry,
    section: &str,
    store: RelationalTarget<'_>,
) -> Result<Arc<dyn RelationalDb>> {
    Err(ToolError::invalid_argument(format!(
        "infra.{section}.backend is 'sqlserver' (pool max {}, dsn {}) but this binary was \
         built without the 'sqlserver' cargo feature: rebuild with --features sqlserver",
        store.pool.max_connections,
        if store.dsn.is_empty() { "missing" } else { "configured" }
    )))
}

#[cfg(not(feature = "postgres"))]
async fn build_postgres(
    _registry: &RelationalRegistry,
    section: &str,
    store: RelationalTarget<'_>,
) -> Result<Arc<dyn RelationalDb>> {
    // Config validation says the same thing at boot; this keeps the factory honest
    // when a store block is built directly in code. The settings are echoed back so
    // the operator can see the block was read, but only whether a DSN is present,
    // never the DSN itself.
    Err(ToolError::invalid_argument(format!(
        "infra.{section}.backend is 'postgres' (schema '{}', pool max {}, dsn {}) but this \
         binary was built without the 'postgres' cargo feature: rebuild with \
         --features postgres",
        store.schema,
        store.pool.max_connections,
        if store.dsn.is_empty() { "missing" } else { "configured" }
    )))
}

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

fn unknown_backend(section: &str, backend: &str) -> ToolError {
    ToolError::invalid_argument(format!(
        "unknown infra.{section}.backend '{backend}', expected one of {}",
        crate::config::backend::ALL.join(", ")
    ))
}

/// Open the raw engine behind `infra.meta`, without wrapping it in a store.
///
/// The migration tool copies rows table by table, so it needs the connection
/// rather than the typed store built on top of it.
pub async fn open_meta_db(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    project_id: &str,
) -> Result<Arc<dyn RelationalDb>> {
    let m = &config.infra.meta;
    build_relational_db(
        registry,
        "meta",
        RelationalTarget { backend: &m.backend, dsn: &m.dsn, schema: &m.schema, pool: &m.pool },
        || config.volume_meta_path(project_id),
    )
    .await
}

/// Open the raw engine behind `infra.admin`. See [`open_meta_db`].
pub async fn open_admin_db(
    config: &ServerConfig,
    registry: &RelationalRegistry,
) -> Result<Arc<dyn RelationalDb>> {
    let a = &config.infra.admin;
    build_relational_db(
        registry,
        "admin",
        RelationalTarget { backend: &a.backend, dsn: &a.dsn, schema: &a.schema, pool: &a.pool },
        || config.admin_db_path(),
    )
    .await
}

/// Open the raw engine behind `infra.git`. See [`open_meta_db`].
pub async fn open_git_db(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    project_id: &str,
) -> Result<Arc<dyn RelationalDb>> {
    let g = &config.infra.git;
    build_relational_db(
        registry,
        "git",
        RelationalTarget { backend: &g.backend, dsn: &g.dsn, schema: &g.schema, pool: &g.pool },
        || config.git_db_path(project_id),
    )
    .await
}

/// Build the metadata backend for one project, per `infra.meta.backend`.
///
/// Under SQLite the volume is a file; under the SQL engines every volume lives in
/// one database, told apart by the `volume_id` column.
pub async fn build_meta_store(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    project_id: &str,
) -> Result<Arc<dyn MetaBackend>> {
    let m = &config.infra.meta;
    let db = build_relational_db(
        registry,
        "meta",
        RelationalTarget { backend: &m.backend, dsn: &m.dsn, schema: &m.schema, pool: &m.pool },
        || config.volume_meta_path(project_id),
    )
    .await?;
    Ok(Arc::new(meta::RelationalMetaStore::open(db, project_id).await?))
}

/// Build the ACL registry, per `infra.admin.backend`.
pub async fn build_admin_store(
    config: &ServerConfig,
    registry: &RelationalRegistry,
) -> Result<Arc<dyn AdminBackend>> {
    let a = &config.infra.admin;
    let db = build_relational_db(
        registry,
        "admin",
        RelationalTarget { backend: &a.backend, dsn: &a.dsn, schema: &a.schema, pool: &a.pool },
        || config.admin_db_path(),
    )
    .await?;
    Ok(Arc::new(admin::RelationalAdminStore::new(db)))
}

/// Build the git index for one project, per `infra.git.backend`.
///
/// `git/repo.rs` used to open its SQLite file directly, which meant the git index
/// was the one piece of relational state the configuration could not move.
pub async fn build_git_db(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    project_id: &str,
) -> Result<Arc<crate::git::db::RelationalGitDb>> {
    let g = &config.infra.git;
    let db = build_relational_db(
        registry,
        "git",
        RelationalTarget { backend: &g.backend, dsn: &g.dsn, schema: &g.schema, pool: &g.pool },
        || config.git_db_path(project_id),
    )
    .await?;
    Ok(Arc::new(crate::git::db::RelationalGitDb::open(db, project_id).await?))
}

/// Delete every git index row of one project.
///
/// Only reached on a shared database. Under SQLite the index is a file and
/// deleting it removes the rows with it, which is what `purge_repo` does. Without
/// this, deleting a project on PostgreSQL or SQL Server would leave its refs and
/// object index behind, and a project later recreated under the same id would
/// inherit them.
pub async fn purge_git_rows(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    project_id: &str,
) -> Result<()> {
    let g = &config.infra.git;
    let db = build_relational_db(
        registry,
        "git",
        RelationalTarget { backend: &g.backend, dsn: &g.dsn, schema: &g.schema, pool: &g.pool },
        || config.git_db_path(project_id),
    )
    .await?;
    // One transaction: a half purged index would leave refs pointing at objects
    // that are no longer listed.
    let mut tx = db.begin().await?;
    for table in crate::git::db::TABLES {
        tx.execute(
            &rel::Query::new(format!("DELETE FROM {table} WHERE volume_id=?1")).bind(project_id),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Build the OAuth token persistence, per `infra.oauth.backend`.
pub async fn build_oauth_persistence(
    config: &ServerConfig,
    registry: &RelationalRegistry,
    key: [u8; crate::git::oauth::cipher::KEY_SIZE],
) -> Result<Arc<crate::git::oauth::persistence::RelationalOAuthPersistence>> {
    let o = &config.infra.oauth;
    let db = build_relational_db(
        registry,
        "oauth",
        RelationalTarget { backend: &o.backend, dsn: &o.dsn, schema: &o.schema, pool: &o.pool },
        || config.oauth_db_path(),
    )
    .await?;
    Ok(Arc::new(
        crate::git::oauth::persistence::RelationalOAuthPersistence::open(db, key).await?,
    ))
}

/// Caches one `VolumeClient` per project and provisions / tears down volumes.
pub struct StoreManager {
    config: Arc<ServerConfig>,
    clients: Mutex<HashMap<String, Arc<VolumeClient>>>,
    /// Handed in rather than created here, so the ACL store, every volume and the
    /// git index all share one pool per DSN. Owning it would give this manager a
    /// private pool, and the process would then open `max_connections` once per
    /// owner while the operator configured that number once.
    relational: Arc<RelationalRegistry>,
}

impl StoreManager {
    pub fn new(config: Arc<ServerConfig>, relational: Arc<RelationalRegistry>) -> Self {
        Self { config, clients: Mutex::new(HashMap::new()), relational }
    }

    /// The shared pool cache, so a caller that builds another relational consumer
    /// can pass the same one instead of starting a second.
    pub fn relational(&self) -> &Arc<RelationalRegistry> {
        &self.relational
    }

    /// Get (or open) the client for a project.
    pub async fn client(&self, project_id: &str) -> Result<Arc<VolumeClient>> {
        let mut guard = self.clients.lock().await;
        if let Some(c) = guard.get(project_id) {
            return Ok(c.clone());
        }
        let meta = build_meta_store(&self.config, &self.relational, project_id).await?;
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

        if self.config.infra.meta.backend == backend::SQLITE {
            // The volume is a file, so removing it removes every row with it.
            let db = self.config.volume_meta_path(project_id);
            for suffix in ["", "-wal", "-shm"] {
                let p = if suffix.is_empty() {
                    db.clone()
                } else {
                    PathBuf::from(format!("{}{}", db.display(), suffix))
                };
                let _ = tokio::fs::remove_file(&p).await;
            }
            return Ok(());
        }

        // On a shared database the volume is a set of rows, so they are deleted
        // explicitly. Both tables go in one transaction: a half torn down volume
        // would leave refcounts referring to nodes that no longer exist.
        let db = build_relational_db(
            &self.relational,
            "meta",
            RelationalTarget {
                backend: &self.config.infra.meta.backend,
                dsn: &self.config.infra.meta.dsn,
                schema: &self.config.infra.meta.schema,
                pool: &self.config.infra.meta.pool,
            },
            || self.config.volume_meta_path(project_id),
        )
        .await?;
        let mut tx = db.begin().await?;
        for table in ["nodes", "blob_refs"] {
            tx.execute(
                &rel::Query::new(format!("DELETE FROM {table} WHERE volume_id=?1"))
                    .bind(project_id),
            )
            .await?;
        }
        tx.commit().await?;
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

    /// The pool cache is handed to every consumer rather than owned by one, so a
    /// server that enables git does not open a second pool against the same DSN.
    /// `max_connections` is configured once, so the process must honour it once.
    #[tokio::test]
    async fn one_registry_is_shared_by_the_store_manager_and_the_git_store() {
        let d = tempfile::tempdir().unwrap();
        let config = cfg(d.path());
        let relational = Arc::new(RelationalRegistry::new());

        // Exactly how the composition root wires them: one registry, cloned.
        let stores = StoreManager::new(config.clone(), relational.clone());
        let git = crate::git::repo::GitRepoStore::new(
            config.clone(),
            stores.relational().clone(),
        );

        assert!(
            Arc::ptr_eq(&relational, stores.relational()),
            "the store manager must use the registry it was handed, not a private one"
        );
        assert!(
            Arc::ptr_eq(stores.relational(), git.relational()),
            "the git index must share the pool cache the metadata stores use"
        );
    }

    /// Guards the composition root itself. The signatures force a registry to be
    /// passed, so the only way back to a second pool is to construct one at a call
    /// site: this fails the moment `app.rs` grows another `RelationalRegistry::new`.
    #[test]
    fn the_composition_root_builds_exactly_one_registry() {
        let app_rs = include_str!("../app.rs");
        let built = app_rs.matches("RelationalRegistry::new(").count();
        assert_eq!(
            built, 1,
            "app.rs must build one registry and thread it into every consumer, found {built}"
        );
    }

    /// The rest of the serving path must never mint a registry either. `app.rs` is
    /// covered above and `migrate.rs` is exempt because it is a one shot offline
    /// command wiring two distinct databases. This caught a real second pool in the
    /// OAuth token store, which is built lazily on first request, not at startup,
    /// and then holds its pool for the life of the process.
    #[test]
    fn no_serving_module_builds_its_own_registry() {
        // Every module that reaches a relational store outside the composition root.
        let sources: [(&str, &str); 4] = [
            ("git/oauth/store.rs", include_str!("../git/oauth/store.rs")),
            ("tools/git_auth.rs", include_str!("../tools/git_auth.rs")),
            ("tools/git.rs", include_str!("../tools/git.rs")),
            ("git/repo.rs", include_str!("../git/repo.rs")),
        ];
        for (name, src) in sources {
            // Test fixtures legitimately build private registries, so only count
            // constructions outside the test module.
            let production = src.split("mod tests").next().unwrap_or(src);
            let built = production.matches("RelationalRegistry::new(").count();
            assert_eq!(
                built, 0,
                "{name} must take the shared registry, not build one, found {built}"
            );
        }
    }

    /// The behavioural half of the sharing guarantee: one registry across the ACL
    /// store, a volume's metadata and the git index opens ONE pool for one DSN,
    /// where private registries would open one each. Skipped when no PostgreSQL is
    /// reachable, so the default `cargo test` run needs no database.
    #[tokio::test]
    async fn one_dsn_yields_one_pool_across_every_consumer() {
        let Ok(dsn) = std::env::var("MCPFS_TEST_PG_DSN") else {
            eprintln!("skipping the pool sharing check: MCPFS_TEST_PG_DSN is unset");
            return;
        };
        if !cfg!(feature = "postgres") {
            eprintln!("skipping the pool sharing check: built without --features postgres");
            return;
        }

        let d = tempfile::tempdir().unwrap();
        let mut c = (*cfg(d.path())).clone();
        // One DSN behind all three stores, which is the deployment this protects.
        for store in [
            &mut c.infra.meta.backend,
            &mut c.infra.admin.backend,
            &mut c.infra.git.backend,
        ] {
            *store = backend::POSTGRES.to_string();
        }
        c.infra.meta.dsn = Dsn::new(dsn.clone());
        c.infra.admin.dsn = Dsn::new(dsn.clone());
        c.infra.git.dsn = Dsn::new(dsn);
        let config = Arc::new(c);

        let shared = Arc::new(RelationalRegistry::new());
        build_admin_store(&config, &shared).await.unwrap().connect().await.unwrap();
        build_meta_store(&config, &shared, "poolshare").await.unwrap();
        build_git_db(&config, &shared, "poolshare").await.unwrap();
        assert_eq!(
            shared.pool_count().await,
            1,
            "the ACL store, the volume metadata and the git index must reuse one pool"
        );

        // The contrast that makes the count above meaningful: give each consumer its
        // own registry and the same work opens a pool per registry.
        let private_a = Arc::new(RelationalRegistry::new());
        let private_b = Arc::new(RelationalRegistry::new());
        build_meta_store(&config, &private_a, "poolshare").await.unwrap();
        build_git_db(&config, &private_b, "poolshare").await.unwrap();
        assert_eq!(
            private_a.pool_count().await + private_b.pool_count().await,
            2,
            "two registries must open two pools, otherwise the assertion above proves nothing"
        );
    }

    #[tokio::test]
    async fn client_is_cached_per_project() {
        let d = tempfile::tempdir().unwrap();
        let m = StoreManager::new(cfg(d.path()), test_registry());
        let a = m.client("proj").await.unwrap();
        let b = m.client("proj").await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same client instance is reused");
    }

    #[tokio::test]
    async fn provision_then_teardown_removes_state() {
        let d = tempfile::tempdir().unwrap();
        let config = cfg(d.path());
        let m = StoreManager::new(config.clone(), test_registry());

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
        let reg = RelationalRegistry::new();
        let e = build_meta_store(&c2, &reg, "p")
            .await
            .err()
            .expect("an unknown backend must fail");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("expected one of"), "{}", e.message);
    }

    /// The stores now speak [`rel::RelationalDb`], so `postgres` is a real backend
    /// rather than a value the factory refuses. This replaces the tripwire that
    /// pinned the unported state.
    ///
    /// Without the `postgres` feature the factory must still say so clearly, and
    /// name the feature to rebuild with, instead of failing on the first write.
    #[tokio::test]
    async fn postgres_is_a_recognised_backend() {
        let d = tempfile::tempdir().unwrap();
        let reg = RelationalRegistry::new();

        let mut meta = (*cfg(d.path())).clone();
        meta.infra.meta.backend = crate::config::backend::POSTGRES.into();
        meta.infra.meta.dsn = Dsn::new("postgres://user:pw@127.0.0.1:1/nope");

        let outcome = build_meta_store(&meta, &reg, "p").await;
        if cfg!(feature = "postgres") {
            // No server is listening, so this must fail as a connection problem,
            // never as "unknown backend" or "not supported".
            let e = outcome.err().expect("there is no server on port 1");
            assert_ne!(e.code, crate::errors::code::INVALID_ARGUMENT, "{}", e.message);
            assert!(
                !e.message.contains("unknown") && !e.message.contains("not usable yet"),
                "postgres must be recognised, got: {}",
                e.message
            );
        } else {
            let e = outcome.err().expect("the driver is not compiled in");
            assert!(
                e.message.contains("postgres") && e.message.contains("cargo feature"),
                "the error must name the missing feature: {}",
                e.message
            );
        }
    }

    /// A DSN reaches the registry, so it must not be printable from the key that
    /// caches the pool: a map key is easy to log by accident.
    #[cfg(feature = "postgres")]
    #[test]
    fn registry_keys_do_not_carry_the_dsn() {
        let dsn = Dsn::new("postgres://user:sup3rs3cret@db/app");
        let key = RelationalRegistry::key(backend::POSTGRES, &dsn, "public");
        assert!(!key.contains("sup3rs3cret"), "the key must not embed the password");
        assert!(key.starts_with("postgres:"));
        // Same target, same key: that is what makes one pool shared.
        assert_eq!(key, RelationalRegistry::key(backend::POSTGRES, &dsn, "public"));
        assert_ne!(key, RelationalRegistry::key(backend::POSTGRES, &dsn, "other"));
    }

    /// Two volumes on one SQL deployment must share a pool. SQLite is excluded on
    /// purpose: its handle owns a file that teardown deletes.
    #[tokio::test]
    async fn sqlite_handles_are_not_shared_between_volumes() {
        let d = tempfile::tempdir().unwrap();
        let config = cfg(d.path());
        let reg = RelationalRegistry::new();
        let a = build_meta_store(&config, &reg, "vol-a").await.unwrap();
        let b = build_meta_store(&config, &reg, "vol-b").await.unwrap();
        a.put_file("/x.txt", Some("s"), 1, crate::storage::traits::MODE_FILE)
            .await
            .unwrap();
        assert!(b.get("/x.txt").await.unwrap().is_none(), "separate files stay separate");
    }

    #[tokio::test]
    async fn volumes_are_isolated_from_each_other() {
        let d = tempfile::tempdir().unwrap();
        let m = StoreManager::new(cfg(d.path()), test_registry());
        let a = m.client("proj-a").await.unwrap();
        let b = m.client("proj-b").await.unwrap();
        a.write_text_atomic("/only-in-a.txt", "x").await.unwrap();
        assert!(a.exists("/only-in-a.txt").await.unwrap());
        assert!(!b.exists("/only-in-a.txt").await.unwrap());
    }
}
