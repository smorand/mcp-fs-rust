//! Search backend trait and shared types.
//!
//! The `SearchBackend` trait is implemented by:
//! - `bm25_sqlite` (Tantivy, always compiled, runs when `search.mode` is "bm25" or "both")
//! - `bm25_pg` (PostgreSQL tsvector, `#[cfg(feature = "postgres")]`)
//! - `vector_pg` (pgvector, `#[cfg(feature = "rag")]`)
//! - `vector_sqlite` (sqlite-vec, `#[cfg(feature = "rag")]`)
//!
//! The factory `build_backend` in this module picks the right combination from
//! the server config and the `infra.meta.backend` setting.

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use std::sync::Arc;

#[cfg(feature = "postgres")]
pub mod bm25_pg;
pub mod bm25_sqlite;
pub mod chunker;
#[cfg(test)]
pub mod e2e;
#[cfg(feature = "rag")]
pub mod embedding;
pub mod fusion;
pub mod indexer;
#[cfg(feature = "rag")]
pub mod rerank;
#[cfg(feature = "rag")]
pub mod vector_pg;
#[cfg(feature = "rag")]
pub mod vector_sqlite;

/// One ranked result chunk returned by a search query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub path: String,
    pub score: f32,
    pub chunk: String,
    pub rank: usize,
}

/// Per-volume index statistics, as reported by `search.status`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStats {
    /// Number of indexed document chunks in the BM25 index.
    pub bm25_docs: usize,
    /// Number of stored vector chunks (0 when RAG is not enabled).
    pub vector_chunks: usize,
    /// True when the Tantivy index directory exists and is non-empty.
    pub bm25_warm: bool,
    /// The active mode ("bm25", "rag", or "both").
    pub mode: String,
}

/// The interface every search backend implements.
///
/// Each implementation handles one engine (Tantivy BM25 on SQLite, pgvector on
/// PostgreSQL, sqlite-vec on SQLite). A combined "both" backend is assembled by
/// the factory function, not by a wrapper type, so the trait stays flat.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Index the given text under `(volume_id, path)`, splitting into chunks.
    /// Idempotent: existing chunks for the path are deleted before inserting.
    /// Returns the number of chunks inserted.
    async fn index_path(
        &self,
        volume_id: &str,
        path: &str,
        text: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) -> Result<usize>;

    /// Remove all indexed chunks for `path` under `volume_id`.
    /// Returns the number of chunks deleted.
    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize>;

    /// Remove every indexed chunk of `volume_id`, whatever the path.
    /// Returns the number of chunks deleted.
    ///
    /// This is the wipe half of a project index mode change, so it must be scoped
    /// to the one volume: another project's chunks live in the same tables.
    async fn delete_all(&self, volume_id: &str) -> Result<usize>;

    /// BM25 full-text query. Returns the top `top_k` results by relevance.
    async fn query_bm25(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Vector KNN query. Returns the top `top_k` results by cosine similarity.
    ///
    /// The `query` text is embedded internally by the backend. Backends that do
    /// not support vector search return `ERR_NOT_SUPPORTED`.
    async fn query_vector(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Return index statistics for the volume.
    async fn stats(&self, volume_id: &str) -> Result<IndexStats>;

    /// Which query modes this backend can serve ("bm25", "rag", or both).
    fn supported_modes(&self) -> Vec<&'static str>;
}

/// Build the search backend for the given config.
///
/// Returns `None` when `search.enabled` is false. The backend is shared across
/// all requests, so it is returned as an `Arc`.
///
/// `relational` is the process-wide pool cache. `http_client` is reused across
/// all embedding calls for the lifetime of the process.
#[allow(unused_variables)] // relational and http_client are only used under feature flags
pub async fn build_backend(
    config: &ServerConfig,
    relational: &crate::storage::RelationalRegistry,
    http_client: Arc<reqwest::Client>,
) -> Result<Option<Arc<dyn SearchBackend>>> {
    if !config.search.enabled {
        return Ok(None);
    }

    let mode = config.search.mode.as_str();
    let meta_backend = config.infra.meta.backend.as_str();

    // RAG and both modes require an embedding endpoint. Config validation
    // already checks this, but guard again here so the error message is clear
    // when called from tests that skip boot validation.
    if matches!(mode, "rag" | "both") && config.search.embedding.endpoint.is_empty() {
        return Err(ToolError::invalid_argument(
            "search.mode is 'rag' or 'both' but search.embedding.endpoint is empty",
        ));
    }

    let backend: Arc<dyn SearchBackend> = match (mode, meta_backend) {
        ("bm25", "postgres") => {
            #[cfg(feature = "postgres")]
            {
                let pool = get_pg_pool(config, relational).await?;
                Arc::new(bm25_pg::PostgresBm25Backend::new(pool).await?)
            }
            #[cfg(not(feature = "postgres"))]
            {
                return Err(ToolError::not_supported(
                    "search.mode=bm25 with backend=postgres requires the postgres feature",
                ));
            }
        }
        ("bm25", _) => Arc::new(bm25_sqlite::TantivyBm25Backend::new(&config.search.tantivy_dir)),
        #[cfg(feature = "rag")]
        ("rag", "postgres") => {
            let pool = get_pg_pool(config, relational).await?;
            Arc::new(
                vector_pg::PostgresVectorBackend::new(
                    pool,
                    config.search.embedding.dimensions,
                    http_client,
                    config.search.embedding.clone(),
                )
                .await?,
            )
        }
        #[cfg(feature = "rag")]
        ("rag", _) => Arc::new(vector_sqlite::SqliteVecBackend::new(
            config.search.embedding.dimensions,
            &config.search.tantivy_dir,
            http_client,
            config.search.embedding.clone(),
        )),
        #[cfg(feature = "rag")]
        ("both", "postgres") => {
            let pool = get_pg_pool(config, relational).await?;
            let bm25: Arc<dyn SearchBackend> =
                Arc::new(bm25_pg::PostgresBm25Backend::new(pool.clone()).await?);
            let vector: Arc<dyn SearchBackend> = Arc::new(
                vector_pg::PostgresVectorBackend::new(
                    pool,
                    config.search.embedding.dimensions,
                    http_client,
                    config.search.embedding.clone(),
                )
                .await?,
            );
            Arc::new(CombinedBackend { bm25, vector, mode: "both".into() })
        }
        #[cfg(feature = "rag")]
        ("both", _) => {
            let bm25: Arc<dyn SearchBackend> =
                Arc::new(bm25_sqlite::TantivyBm25Backend::new(&config.search.tantivy_dir));
            let vector: Arc<dyn SearchBackend> = Arc::new(vector_sqlite::SqliteVecBackend::new(
                config.search.embedding.dimensions,
                &config.search.tantivy_dir,
                http_client,
                config.search.embedding.clone(),
            ));
            Arc::new(CombinedBackend { bm25, vector, mode: "both".into() })
        }
        (other, _) => {
            return Err(ToolError::invalid_argument(format!(
                "unknown search.mode '{other}', expected bm25, rag, or both"
            )));
        }
    };

    Ok(Some(backend))
}

/// Open (or reuse) a `PgPool` rooted at the same server as `infra.meta`.
///
/// We cannot downcast `Arc<dyn RelationalDb>` to `PostgresRelationalDb` to
/// extract the pool, so a second pool object is created against the same DSN.
/// PostgreSQL handles multiple pool objects sharing one server without issue;
/// the pool is bounded by `pool.max_connections` so the connection count stays
/// within the operator's configured limit.
#[cfg(feature = "postgres")]
async fn get_pg_pool(
    config: &ServerConfig,
    _relational: &crate::storage::RelationalRegistry,
) -> Result<sqlx::PgPool> {
    use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
    let m = &config.infra.meta;
    let pg = PostgresRelationalDb::connect(
        m.dsn.expose(),
        &m.schema,
        PoolSettings {
            max_connections: m.pool.max_connections,
            acquire_timeout: std::time::Duration::from_secs(m.pool.acquire_timeout_secs),
        },
    )
    .await?;
    Ok(pg.pool().clone())
}

/// A backend that runs both BM25 and vector search in parallel, merging with RRF.
/// Only compiled when the `rag` feature is on.
#[cfg(feature = "rag")]
struct CombinedBackend {
    bm25: Arc<dyn SearchBackend>,
    vector: Arc<dyn SearchBackend>,
    mode: String,
}

#[cfg(feature = "rag")]
#[async_trait]
impl SearchBackend for CombinedBackend {
    async fn index_path(
        &self,
        volume_id: &str,
        path: &str,
        text: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) -> Result<usize> {
        let n1 = self.bm25.index_path(volume_id, path, text, chunk_size, chunk_overlap).await?;
        // Index into vector store too; ignore the count (bm25 chunk count is authoritative).
        let _ = self.vector.index_path(volume_id, path, text, chunk_size, chunk_overlap).await?;
        Ok(n1)
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let n1 = self.bm25.delete_path(volume_id, path).await?;
        let _ = self.vector.delete_path(volume_id, path).await?;
        Ok(n1)
    }

    async fn delete_all(&self, volume_id: &str) -> Result<usize> {
        // Both halves are wiped even if the first reports nothing, otherwise a
        // mode change would leave the vector store holding stale chunks.
        let n1 = self.bm25.delete_all(volume_id).await?;
        let n2 = self.vector.delete_all(volume_id).await?;
        Ok(n1 + n2)
    }

    async fn query_bm25(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.bm25.query_bm25(volume_id, query, top_k).await
    }

    async fn query_vector(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.vector.query_vector(volume_id, query, top_k).await
    }

    async fn stats(&self, volume_id: &str) -> Result<IndexStats> {
        let s1 = self.bm25.stats(volume_id).await?;
        let s2 = self.vector.stats(volume_id).await?;
        Ok(IndexStats {
            bm25_docs: s1.bm25_docs,
            vector_chunks: s2.vector_chunks,
            bm25_warm: s1.bm25_warm,
            mode: self.mode.clone(),
        })
    }

    fn supported_modes(&self) -> Vec<&'static str> {
        vec!["bm25", "rag", "both"]
    }
}
