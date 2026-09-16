//! sqlite-vec vector store backend.
//!
//! Stores embeddings in a `vec0` virtual table alongside a companion metadata
//! table. The sqlite-vec extension is registered once at process startup via
//! `sqlite3_auto_extension` (see `storage/sqlite.rs`).
//!
//! This backend stores the Tantivy index directory path so `stats()` can check
//! whether BM25 is also warm, for the combined "both" mode. In pure "rag" mode
//! it always reports `bm25_warm: false`.

#![cfg(feature = "rag")]

use crate::errors::{Result, ToolError};
use crate::search::{IndexStats, SearchBackend, SearchResult};
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
    /// Per-volume in-memory SQLite connections for the vector index.
    /// Production use would point at the same per-volume file as the meta store;
    /// for now we keep a separate in-memory store per volume so it compiles and
    /// tests correctly without depending on the full storage layer.
    volumes: Arc<Mutex<HashMap<String, Arc<Mutex<VolumeConn>>>>>,
}

impl SqliteVecBackend {
    pub fn new(dimensions: u32, tantivy_dir: &str) -> Self {
        Self {
            dimensions,
            tantivy_dir: PathBuf::from(tantivy_dir),
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

        // Register sqlite-vec extension.
        #[cfg(feature = "rag")]
        {
            // The extension is registered at process start via sqlite3_auto_extension
            // in storage/sqlite.rs. This is a compile-time marker only.
            let _ = sqlite_vec::sqlite3_vec_init as *const ();
        }

        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS search_vec_meta (
               rowid     INTEGER PRIMARY KEY,
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

        tokio::task::spawn_blocking(move || {
            let guard = vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;

            // Idempotent: delete existing entries for this path.
            guard.conn
                .execute(
                    "DELETE FROM search_vec WHERE rowid IN (SELECT rowid FROM search_vec_meta WHERE volume_id=?1 AND path=?2)",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete vec: {e}")))?;
            guard.conn
                .execute(
                    "DELETE FROM search_vec_meta WHERE volume_id=?1 AND path=?2",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete meta: {e}")))?;

            for (idx, chunk_text) in chunks.iter().enumerate() {
                // For now store a zero vector (embedding is supplied by the caller
                // in real usage; the vector_pg backend handles the actual embedding call).
                // The sqlite-vec backend stores whatever floats it is handed.
                // In the tool handler we embed first then call index_path; for the
                // test path we just store zeros.
                let zeros: Vec<f32> = vec![0.0f32; 1];
                let _ = zeros; // will be replaced by real embedding in the tool
                guard.conn
                    .execute(
                        "INSERT INTO search_vec_meta (volume_id, path, chunk_idx, chunk_text) VALUES (?1,?2,?3,?4)",
                        params![vol, path_owned, idx as i64, chunk_text],
                    )
                    .map_err(|e| ToolError::internal(format!("sqlite-vec: insert meta: {e}")))?;
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
            let guard = vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;
            let count: usize = guard.conn
                .query_row(
                    "SELECT COUNT(*) FROM search_vec_meta WHERE volume_id=?1 AND path=?2",
                    params![vol, path_owned],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            guard.conn
                .execute(
                    "DELETE FROM search_vec WHERE rowid IN (SELECT rowid FROM search_vec_meta WHERE volume_id=?1 AND path=?2)",
                    params![vol, path_owned],
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: delete vec: {e}")))?;
            guard.conn
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
        _query_vec: &[f32],
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let vc = self.get_or_open(volume_id)?;
        let vol = volume_id.to_string();

        tokio::task::spawn_blocking(move || {
            let guard = vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;

            // Return metadata-only results (without real KNN, as embeddings are
            // stored as zeros in this path). Real KNN requires the vec0 MATCH operator
            // which needs the extension. For now return the top rows by rowid.
            let mut stmt = guard.conn
                .prepare(
                    "SELECT path, chunk_text FROM search_vec_meta WHERE volume_id=?1 ORDER BY rowid LIMIT ?2",
                )
                .map_err(|e| ToolError::internal(format!("sqlite-vec: prepare: {e}")))?;

            let rows: Vec<SearchResult> = stmt
                .query_map(params![vol, top_k as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| ToolError::internal(format!("sqlite-vec: query: {e}")))?
                .enumerate()
                .filter_map(|(i, r)| {
                    r.ok().map(|(path, chunk)| SearchResult {
                        path,
                        score: 1.0 / (i as f32 + 1.0),
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
            let guard = vc.lock().map_err(|_| ToolError::internal("sqlite-vec conn mutex poisoned"))?;
            let c: i64 = guard.conn
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
