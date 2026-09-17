//! sqlite-vec vector store backend.
//!
//! Stores embeddings in a `vec0` virtual table alongside a companion metadata
//! table. The sqlite-vec extension is registered once at process startup via
//! `sqlite3_auto_extension` (see `storage/sqlite.rs`).
//!
//! Each `index_path` call embeds every chunk via the configured embedding
//! endpoint before storing the resulting float vector. `query_vector` likewise
//! embeds the query string before running the KNN `MATCH` scan, so callers
//! never need to produce their own vectors.

#![cfg(feature = "rag")]

use crate::config::EmbeddingConfig;
use crate::errors::{Result, ToolError};
use crate::search::{IndexStats, SearchBackend, SearchResult, embedding};
use async_trait::async_trait;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One open connection per volume, created on first use.
struct VolumeConn {
    conn: Connection,
}

/// Vector search backend using sqlite-vec virtual tables.
pub struct SqliteVecBackend {
    dimensions: u32,
    tantivy_dir: PathBuf,
    /// Shared HTTP client for embedding calls.
    client: Arc<reqwest::Client>,
    /// Embedding model config (endpoint, model name, API key env).
    embedding_config: EmbeddingConfig,
    /// Per-volume in-memory SQLite connections for the vector index.
    volumes: Arc<Mutex<HashMap<String, Arc<Mutex<VolumeConn>>>>>,
}

impl SqliteVecBackend {
    pub fn new(
        dimensions: u32,
        tantivy_dir: &str,
        client: Arc<reqwest::Client>,
        embedding_config: EmbeddingConfig,
    ) -> Self {
        Self {
            dimensions,
            tantivy_dir: PathBuf::from(tantivy_dir),
            client,
            embedding_config,
            volumes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_or_open(&self, volume_id: &str) -> Result<Arc<Mutex<VolumeConn>>> {
        let mut guard = self
            .volumes
            .lock()
            .map_err(|_| ToolError::internal("sqlite-vec volumes mutex poisoned"))?;
        if let Some(vc) = guard.get(volume_id) {
            return Ok(vc.clone());
        }
        let conn = Connection::open_in_memory()
            .map_err(|e| ToolError::internal(format!("sqlite-vec: open: {e}")))?;

        // Ensure the sqlite-vec extension symbol is resolved at compile time.
        #[cfg(feature = "rag")]
        {
            let _ = sqlite_vec::sqlite3_vec_init as *const ();
        }

        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS search_vec_meta (
               rowid     INTEGER PRIMARY KEY AUTOINCREMENT,
               volume_id TEXT NOT NULL,
               path      TEXT NOT NULL,
               chunk_idx INTEGER NOT NULL,
               chunk_text TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS search_vec
               USING vec0(embedding float[{dims}]);",
            dims = self.dimensions
        ))
        .map_err(|e| ToolError::internal(format!("sqlite-vec: create tables: {e}")))?;

        let vc = Arc::new(Mutex::new(VolumeConn { conn }));
        guard.insert(volume_id.to_string(), vc.clone());
        Ok(vc)
    }
}

#[async_trait]
impl SearchBackend for SqliteVecBackend {
    async fn index_path(
        &self,
        volume_id: &str,
        path: &str,
        text: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) -> Result<usize> {
        let vc = self.get_or_open(volume_id)?;
        let chunks = crate::search::chunker::chunk(text, chunk_size, chunk_overlap);
        let n = chunks.len();
        let vol = volume_id.to_string();
        let path_owned = path.to_string();

        // Compute embeddings before entering the blocking closure: embedding
        // calls are async and must not be made from within spawn_blocking.
        let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());
        for chunk_text in &chunks {
            let emb = embedding::embed(&self.client, &self.embedding_config, chunk_text).await?;
            embeddings.push(emb);
        }

        tokio::task::spawn_blocking(move || {
            let guard =
                vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;

            // Idempotent: remove any existing rows for this path before re-inserting.
            guard
                .conn
                .execute(
                    "DELETE FROM search_vec WHERE rowid IN \
                     (SELECT rowid FROM search_vec_meta WHERE volume_id=?1 AND path=?2)",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete vec: {e}")))?;
            guard
                .conn
                .execute(
                    "DELETE FROM search_vec_meta WHERE volume_id=?1 AND path=?2",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete meta: {e}")))?;

            for (idx, (chunk_text, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
                // Insert metadata row first; its auto-increment rowid is used
                // to link the vec0 table row.
                guard
                    .conn
                    .execute(
                        "INSERT INTO search_vec_meta (volume_id, path, chunk_idx, chunk_text) \
                         VALUES (?1, ?2, ?3, ?4)",
                        params![vol, path_owned, idx as i64, chunk_text],
                    )
                    .map_err(|e| ToolError::internal(format!("sqlite-vec: insert meta: {e}")))?;

                let meta_rowid = guard.conn.last_insert_rowid();

                // Convert f32 slice to raw bytes for the vec0 MATCH interface.
                //
                // SAFETY: f32 is a plain-data type with no padding; reinterpreting
                // its memory as bytes is always valid.
                let bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        embedding.as_ptr().cast::<u8>(),
                        embedding.len() * std::mem::size_of::<f32>(),
                    )
                };

                guard
                    .conn
                    .execute(
                        "INSERT INTO search_vec (rowid, embedding) VALUES (?1, ?2)",
                        params![meta_rowid, bytes],
                    )
                    .map_err(|e| ToolError::internal(format!("sqlite-vec: insert vec: {e}")))?;
            }
            Ok(n)
        })
        .await
        .map_err(|e| ToolError::internal(format!("sqlite-vec task join: {e}")))?
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let vc = self.get_or_open(volume_id)?;
        let vol = volume_id.to_string();
        let path_owned = path.to_string();

        tokio::task::spawn_blocking(move || {
            let guard =
                vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;
            let count: usize = guard
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM search_vec_meta WHERE volume_id=?1 AND path=?2",
                    params![vol, path_owned],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            guard
                .conn
                .execute(
                    "DELETE FROM search_vec WHERE rowid IN \
                     (SELECT rowid FROM search_vec_meta WHERE volume_id=?1 AND path=?2)",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete vec: {e}")))?;
            guard
                .conn
                .execute(
                    "DELETE FROM search_vec_meta WHERE volume_id=?1 AND path=?2",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete meta: {e}")))?;
            Ok(count)
        })
        .await
        .map_err(|e| ToolError::internal(format!("sqlite-vec task join: {e}")))?
    }

    async fn delete_all(&self, volume_id: &str) -> Result<usize> {
        let vc = self.get_or_open(volume_id)?;
        let vol = volume_id.to_string();

        tokio::task::spawn_blocking(move || {
            let guard =
                vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;
            let count: usize = guard
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM search_vec_meta WHERE volume_id=?1",
                    params![vol],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            // The vec0 rows are keyed by the meta rowid, so they go first: once
            // the meta rows are gone the subquery can no longer find them.
            guard
                .conn
                .execute(
                    "DELETE FROM search_vec WHERE rowid IN \
                     (SELECT rowid FROM search_vec_meta WHERE volume_id=?1)",
                    params![vol],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete all vec: {e}")))?;
            guard
                .conn
                .execute("DELETE FROM search_vec_meta WHERE volume_id=?1", params![vol])
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete all meta: {e}")))?;
            Ok(count)
        })
        .await
        .map_err(|e| ToolError::internal(format!("sqlite-vec task join: {e}")))?
    }

    async fn query_bm25(
        &self,
        _volume_id: &str,
        _query: &str,
        _top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        Err(ToolError::not_supported(
            "BM25 search is not available in rag mode; use mode=bm25 or mode=both",
        ))
    }

    async fn query_vector(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        // Embed the query text before entering the blocking section.
        let embedding = embedding::embed(&self.client, &self.embedding_config, query).await?;

        let vc = self.get_or_open(volume_id)?;
        let vol = volume_id.to_string();

        tokio::task::spawn_blocking(move || {
            let guard =
                vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;

            // Convert embedding to raw bytes for the vec0 MATCH operator.
            //
            // SAFETY: f32 is a plain-data type with no padding.
            let bytes: Vec<u8> = unsafe {
                std::slice::from_raw_parts(
                    embedding.as_ptr().cast::<u8>(),
                    embedding.len() * std::mem::size_of::<f32>(),
                )
                .to_vec()
            };

            let mut stmt = guard
                .conn
                .prepare(
                    "SELECT m.path, m.chunk_text, v.distance \
                     FROM search_vec v \
                     JOIN search_vec_meta m ON m.rowid = v.rowid \
                     WHERE m.volume_id = ?1 \
                       AND v.embedding MATCH ?2 \
                       AND k = ?3 \
                     ORDER BY v.distance",
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: prepare: {e}")))?;

            let rows: Vec<SearchResult> = stmt
                .query_map(params![vol, rusqlite::types::Value::Blob(bytes), top_k as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?))
                })
                .map_err(|e| ToolError::internal(format!("sqlite-vec: query: {e}")))?
                .enumerate()
                .filter_map(|(i, r)| {
                    r.ok().map(|(path, chunk, distance)| SearchResult {
                        path,
                        // Convert cosine distance to a similarity score in [0,1].
                        score: 1.0 - distance as f32,
                        chunk,
                        rank: i + 1,
                    })
                })
                .collect();

            Ok(rows)
        })
        .await
        .map_err(|e| ToolError::internal(format!("sqlite-vec task join: {e}")))?
    }

    async fn stats(&self, volume_id: &str) -> Result<IndexStats> {
        let vc = self.get_or_open(volume_id)?;
        let vol = volume_id.to_string();
        let tantivy_dir = self.tantivy_dir.join(volume_id);

        let count = tokio::task::spawn_blocking(move || {
            let guard =
                vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;
            let c: i64 = guard
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM search_vec_meta WHERE volume_id=?1",
                    params![vol],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            Ok::<i64, ToolError>(c)
        })
        .await
        .map_err(|e| ToolError::internal(format!("sqlite-vec task join: {e}")))??;

        let bm25_warm = tantivy_dir.exists()
            && tantivy_dir.read_dir().ok().map(|mut d| d.next().is_some()).unwrap_or(false);

        Ok(IndexStats {
            bm25_docs: 0,
            vector_chunks: count as usize,
            bm25_warm,
            mode: "rag".into(),
        })
    }

    fn supported_modes(&self) -> Vec<&'static str> {
        vec!["rag"]
    }
}

#[cfg(test)]
mod tests {
    // These tests use the real sqlite-vec vec0 MATCH operator. They need the
    // extension registered, which happens via the OnceLock in storage/sqlite.rs.
    // Since tests run in the same process and OnceLock fires once, we call the
    // registration function directly here.

    use super::*;
    use crate::config::EmbeddingConfig;
    use crate::errors::code;
    use crate::search::SearchBackend;
    use axum::routing::post;
    use axum::{Json, Router};

    fn register_extension() {
        crate::storage::sqlite::register_sqlite_vec();
    }

    /// Spawn a minimal axum server returning a fixed N-dim embedding.
    async fn spawn_fake_embedding(dims: usize) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = Router::new().route(
            "/v1/embeddings",
            post(move || async move {
                let vec: Vec<f32> = (0..dims).map(|i| (i + 1) as f32 / dims as f32).collect();
                Json(serde_json::json!({
                    "data": [{"embedding": vec, "index": 0}],
                    "model": "fake",
                    "usage": {"prompt_tokens": 1, "total_tokens": 1}
                }))
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://127.0.0.1:{port}")
    }

    fn backend(endpoint: &str, dims: u32) -> (SqliteVecBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let client = Arc::new(reqwest::Client::new());
        let cfg = EmbeddingConfig {
            endpoint: endpoint.to_string(),
            model: "fake".to_string(),
            api_key_env: String::new(),
            dimensions: dims,
        };
        let b = SqliteVecBackend::new(dims, dir.path().to_str().unwrap(), client, cfg);
        (b, dir)
    }

    #[tokio::test]
    async fn stats_zero_on_empty_volume() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        let s = b.stats("vol1").await.unwrap();
        assert_eq!(s.vector_chunks, 0);
    }

    #[tokio::test]
    async fn index_increments_chunk_count() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol1", "/a.md", "hello world test", 200, 0).await.unwrap();
        let s = b.stats("vol1").await.unwrap();
        assert_eq!(s.vector_chunks, 1);
    }

    #[tokio::test]
    async fn delete_removes_chunks() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol1", "/a.md", "some text to delete", 200, 0).await.unwrap();
        let deleted = b.delete_path("vol1", "/a.md").await.unwrap();
        assert!(deleted > 0, "delete must report at least one chunk removed");
        let s = b.stats("vol1").await.unwrap();
        assert_eq!(s.vector_chunks, 0);
    }

    #[tokio::test]
    async fn idempotent_reindex() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol1", "/a.md", "first content", 200, 0).await.unwrap();
        b.index_path("vol1", "/a.md", "second content", 200, 0).await.unwrap();
        let s = b.stats("vol1").await.unwrap();
        assert_eq!(s.vector_chunks, 1, "re-index must replace, not append");
    }

    #[tokio::test]
    async fn query_vector_returns_results() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol1", "/doc.md", "rust systems programming", 200, 0).await.unwrap();
        // The fake server returns the same vector for every input, so cosine
        // distance between query and stored embedding is 0 (identical).
        let r = b.query_vector("vol1", "systems programming", 10).await.unwrap();
        assert!(!r.is_empty(), "expected at least one result");
        assert_eq!(r[0].path, "/doc.md");
        assert_eq!(r[0].rank, 1);
    }

    #[tokio::test]
    async fn volume_isolation() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol_a", "/x.md", "content a", 200, 0).await.unwrap();
        b.index_path("vol_b", "/x.md", "content b", 200, 0).await.unwrap();
        assert_eq!(b.stats("vol_a").await.unwrap().vector_chunks, 1);
        assert_eq!(b.stats("vol_b").await.unwrap().vector_chunks, 1);
    }

    #[tokio::test]
    async fn delete_all_removes_every_chunk_of_the_volume() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol1", "/a.md", "alpha", 200, 0).await.unwrap();
        b.index_path("vol1", "/b.md", "beta", 200, 0).await.unwrap();
        assert_eq!(b.stats("vol1").await.unwrap().vector_chunks, 2);

        assert_eq!(b.delete_all("vol1").await.unwrap(), 2);
        assert_eq!(b.stats("vol1").await.unwrap().vector_chunks, 0);
        assert!(b.query_vector("vol1", "alpha", 10).await.unwrap().is_empty());

        // Usable again straight after the wipe.
        b.index_path("vol1", "/c.md", "gamma", 200, 0).await.unwrap();
        assert_eq!(b.stats("vol1").await.unwrap().vector_chunks, 1);
    }

    #[tokio::test]
    async fn delete_all_is_scoped_to_one_volume() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        b.index_path("vol_a", "/x.md", "content a", 200, 0).await.unwrap();
        b.index_path("vol_b", "/x.md", "content b", 200, 0).await.unwrap();
        b.delete_all("vol_a").await.unwrap();
        assert_eq!(b.stats("vol_a").await.unwrap().vector_chunks, 0);
        assert_eq!(b.stats("vol_b").await.unwrap().vector_chunks, 1);
    }

    #[tokio::test]
    async fn delete_all_on_an_empty_volume_is_zero() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        assert_eq!(b.delete_all("never-seen").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn query_bm25_returns_not_supported() {
        register_extension();
        let base = spawn_fake_embedding(3).await;
        let (b, _d) = backend(&format!("{base}/v1/embeddings"), 3);
        let err = b.query_bm25("vol1", "anything", 10).await.unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
    }
}
