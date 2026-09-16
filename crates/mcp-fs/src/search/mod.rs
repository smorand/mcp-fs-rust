//! Search backend trait and shared types.
//!
//! The `SearchBackend` trait is implemented by:
//! - `bm25_sqlite` (Tantivy, always compiled, runs when `search.mode` is "bm25" or "both")
//! - `vector_pg` (pgvector, `#[cfg(feature = "rag")]`)
//! - `vector_sqlite` (sqlite-vec, `#[cfg(feature = "rag")]`)
//!
//! The factory `build_backend` in this module picks the right combination from
//! the server config.

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use std::sync::Arc;

pub mod bm25_sqlite;
pub mod chunker;
pub mod fusion;
#[cfg(feature = "rag")]
pub mod embedding;
#[cfg(feature = "rag")]
pub mod rerank;
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

    /// BM25 full-text query. Returns the top `top_k` results by relevance.
    async fn query_bm25(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Vector KNN query. Returns the top `top_k` results by cosine similarity.
    async fn query_vector(
        &self,
        volume_id: &str,
        query_vec: &[f32],
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
pub async fn build_backend(
    config: &ServerConfig,
) -> Result<Option<Arc<dyn SearchBackend>>> {
    if !config.search.enabled {
        return Ok(None);
    }

    let mode = config.search.mode.as_str();

    // RAG and both modes require an embedding endpoint. Config validation
    // already checks this, but guard again here so the error message is clear
    // when called from tests that skip boot validation.
    if matches!(mode, "rag" | "both") && config.search.embedding.endpoint.is_empty() {
        return Err(ToolError::invalid_argument(
            "search.mode is 'rag' or 'both' but search.embedding.endpoint is empty",
        ));
    }

    let backend: Arc<dyn SearchBackend> = match mode {
        "bm25" => {
            let b = bm25_sqlite::TantivyBm25Backend::new(&config.search.tantivy_dir);
            Arc::new(b)
        }
        #[cfg(feature = "rag")]
        "rag" => {
            let b = vector_sqlite::SqliteVecBackend::new(
                config.search.embedding.dimensions,
                &config.search.tantivy_dir,
            );
            Arc::new(b)
        }
        #[cfg(feature = "rag")]
        "both" => {
            let combined = CombinedBackend {
                bm25: bm25_sqlite::TantivyBm25Backend::new(&config.search.tantivy_dir),
                vector: vector_sqlite::SqliteVecBackend::new(
                    config.search.embedding.dimensions,
                    &config.search.tantivy_dir,
                ),
                mode: "both".into(),
            };
            Arc::new(combined)
        }
        other => {
            return Err(ToolError::invalid_argument(format!(
                "unknown search.mode '{other}', expected bm25, rag, or both"
            )));
        }
    };

    Ok(Some(backend))
}

/// A backend that runs both BM25 and vector search, merging with RRF.
/// Only compiled when the `rag` feature is on.
#[cfg(feature = "rag")]
struct CombinedBackend {
    bm25: bm25_sqlite::TantivyBm25Backend,
    vector: vector_sqlite::SqliteVecBackend,
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
        let _ = self.vector.index_path(volume_id, path, text, chunk_size, chunk_overlap).await?;
        Ok(n1)
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let n1 = self.bm25.delete_path(volume_id, path).await?;
        let _ = self.vector.delete_path(volume_id, path).await?;
        Ok(n1)
    }

    async fn query_bm25(&self, volume_id: &str, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
        self.bm25.query_bm25(volume_id, query, top_k).await
    }

    async fn query_vector(&self, volume_id: &str, query_vec: &[f32], top_k: usize) -> Result<Vec<SearchResult>> {
        self.vector.query_vector(volume_id, query_vec, top_k).await
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
