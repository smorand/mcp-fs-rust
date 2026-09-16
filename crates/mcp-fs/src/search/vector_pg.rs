//! pgvector vector-search backend.
//!
//! Stores chunk embeddings in a `VECTOR(N)` column backed by the `vector`
//! extension. The extension is enabled per connection in
//! `storage/rel/postgres.rs` (`CREATE EXTENSION IF NOT EXISTS vector`), so it
//! is already present whenever this backend connects to the same server.
//!
//! Both `index_path` and `query_vector` call the configured embedding endpoint
//! to produce float vectors; callers never need to pre-compute embeddings.
//!
//! Raw `sqlx` is used because the `VECTOR` type and its operators (`<=>`) are
//! not representable through `SchemaSet` / `RelationalDb`.

#![cfg(feature = "rag")]

use crate::config::EmbeddingConfig;
use crate::errors::{Result, ToolError};
use crate::search::{IndexStats, SearchBackend, SearchResult, embedding};
use async_trait::async_trait;
use sqlx::AssertSqlSafe;
use sqlx::Row;
use std::sync::Arc;

/// pgvector-backed vector search backend.
pub struct PostgresVectorBackend {
    pool: sqlx::PgPool,
    /// Stored for documentation; used only at construction time to set up the
    /// VECTOR(N) column with the correct dimension.
    #[allow(dead_code)]
    dimensions: u32,
    client: Arc<reqwest::Client>,
    embedding_config: EmbeddingConfig,
}

impl PostgresVectorBackend {
    /// Create the backend and ensure the schema exists.
    ///
    /// The `CREATE EXTENSION IF NOT EXISTS vector` statement is executed by the
    /// pool's `after_connect` hook in `storage/rel/postgres.rs`, so it is
    /// already present when we arrive here.
    pub async fn new(
        pool: sqlx::PgPool,
        dimensions: u32,
        client: Arc<reqwest::Client>,
        embedding_config: EmbeddingConfig,
    ) -> Result<Self> {
        // The VECTOR(N) column type requires the dimension to be part of the
        // DDL string; it cannot be passed as a bind parameter.
        let create_table = format!(
            "CREATE TABLE IF NOT EXISTS search_chunks (\
                volume_id  TEXT     NOT NULL,\
                path       TEXT     NOT NULL,\
                chunk_idx  INTEGER  NOT NULL,\
                chunk_text TEXT     NOT NULL,\
                embedding  VECTOR({dimensions}) NOT NULL,\
                PRIMARY KEY (volume_id, path, chunk_idx)\
            )"
        );

        sqlx::query(AssertSqlSafe(create_table))
            .execute(&pool)
            .await
            .map_err(|e| ToolError::internal(format!("vector_pg: create table: {e}")))?;

        sqlx::query(AssertSqlSafe(
            "CREATE INDEX IF NOT EXISTS idx_search_chunks_emb \
             ON search_chunks USING ivfflat (embedding vector_cosine_ops)",
        ))
        .execute(&pool)
        .await
        .map_err(|e| ToolError::internal(format!("vector_pg: create index: {e}")))?;

        Ok(Self { pool, dimensions, client, embedding_config })
    }
}

/// Format a `Vec<f32>` as a pgvector literal string, e.g. `"[0.1,0.2,0.3]"`.
fn vec_to_pg(v: &[f32]) -> String {
    let inner = v.iter().map(f32::to_string).collect::<Vec<_>>().join(",");
    format!("[{inner}]")
}

#[async_trait]
impl SearchBackend for PostgresVectorBackend {
    async fn index_path(
        &self,
        volume_id: &str,
        path: &str,
        text: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) -> Result<usize> {
        let chunks = crate::search::chunker::chunk(text, chunk_size, chunk_overlap);
        let n = chunks.len();

        // Compute all embeddings before touching the database. Embedding calls
        // are async and must not run inside a database transaction.
        let mut embeddings: Vec<String> = Vec::with_capacity(chunks.len());
        for chunk_text in &chunks {
            let emb = embedding::embed(&self.client, &self.embedding_config, chunk_text).await?;
            embeddings.push(vec_to_pg(&emb));
        }

        // Idempotent: remove existing rows for this (volume, path) pair.
        sqlx::query(AssertSqlSafe(
            "DELETE FROM search_chunks WHERE volume_id = $1 AND path = $2",
        ))
        .bind(volume_id)
        .bind(path)
        .execute(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("vector_pg: delete: {e}")))?;

        for (idx, (chunk_text, vec_str)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            // The embedding is passed as a TEXT parameter and cast to VECTOR with
            // the `::vector` suffix, which is the recommended approach for dynamic
            // vector values in sqlx where no native pgvector type binding exists.
            sqlx::query(AssertSqlSafe(
                "INSERT INTO search_chunks (volume_id, path, chunk_idx, chunk_text, embedding) \
                 VALUES ($1, $2, $3, $4, $5::vector)",
            ))
            .bind(volume_id)
            .bind(path)
            .bind(idx as i32)
            .bind(chunk_text.as_str())
            .bind(vec_str.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| ToolError::internal(format!("vector_pg: insert chunk {idx}: {e}")))?;
        }

        Ok(n)
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let row = sqlx::query(AssertSqlSafe(
            "WITH deleted AS (
                DELETE FROM search_chunks WHERE volume_id = $1 AND path = $2
                RETURNING 1
             ) SELECT COUNT(*) AS cnt FROM deleted",
        ))
        .bind(volume_id)
        .bind(path)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("vector_pg: delete: {e}")))?;

        let cnt: i64 = row
            .try_get("cnt")
            .map_err(|e| ToolError::internal(format!("vector_pg: read count: {e}")))?;
        Ok(cnt as usize)
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
        let emb = embedding::embed(&self.client, &self.embedding_config, query).await?;
        let vec_str = vec_to_pg(&emb);

        // `<=>` is the cosine distance operator provided by pgvector.
        let rows = sqlx::query(AssertSqlSafe(
            "SELECT path, chunk_text, \
                    (embedding <=> $1::vector) AS distance \
             FROM search_chunks \
             WHERE volume_id = $2 \
             ORDER BY distance ASC \
             LIMIT $3",
        ))
        .bind(vec_str.as_str())
        .bind(volume_id)
        .bind(top_k as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("vector_pg: query: {e}")))?;

        let mut results = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let path: String = row
                .try_get("path")
                .map_err(|e| ToolError::internal(format!("vector_pg: read path: {e}")))?;
            let chunk: String = row
                .try_get("chunk_text")
                .map_err(|e| ToolError::internal(format!("vector_pg: read chunk_text: {e}")))?;
            let distance: f64 = row
                .try_get("distance")
                .map_err(|e| ToolError::internal(format!("vector_pg: read distance: {e}")))?;
            // Convert cosine distance [0,2] to a similarity score in [0,1].
            let score = 1.0 - (distance as f32 / 2.0);
            results.push(SearchResult { path, score, chunk, rank: i + 1 });
        }
        Ok(results)
    }

    async fn stats(&self, volume_id: &str) -> Result<IndexStats> {
        let row = sqlx::query(AssertSqlSafe(
            "SELECT COUNT(*) AS cnt FROM search_chunks WHERE volume_id = $1",
        ))
        .bind(volume_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("vector_pg: stats: {e}")))?;

        let cnt: i64 = row
            .try_get("cnt")
            .map_err(|e| ToolError::internal(format!("vector_pg: read stats count: {e}")))?;

        Ok(IndexStats {
            bm25_docs: 0,
            vector_chunks: cnt as usize,
            bm25_warm: false,
            mode: "rag".into(),
        })
    }

    fn supported_modes(&self) -> Vec<&'static str> {
        vec!["rag"]
    }
}

#[cfg(test)]
mod tests {
    // Live tests against a real Postgres + pgvector require MCPFS_TEST_PG_DSN.
    // They are skipped automatically when the variable is unset.

    async fn live_pool(schema: &str) -> Option<sqlx::PgPool> {
        let dsn = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())?;
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        let db = PostgresRelationalDb::connect(&dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but connection failed");
        Some(db.pool().clone())
    }

    #[tokio::test]
    async fn live_stats_returns_zero_on_empty_volume() {
        use super::*;
        let Some(pool) = live_pool("mcpfs_vecpg_test").await else {
            return;
        };
        let client = Arc::new(reqwest::Client::new());
        let cfg = EmbeddingConfig::default();
        let b = PostgresVectorBackend::new(pool, 3, client, cfg).await.unwrap();
        let s = b.stats("empty_vol").await.unwrap();
        assert_eq!(s.vector_chunks, 0);
    }
}
