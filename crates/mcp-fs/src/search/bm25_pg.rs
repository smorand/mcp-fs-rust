//! PostgreSQL full-text search backend using `tsvector` / `to_tsquery`.
//!
//! Each indexed chunk is stored with a `tsvector` column populated by
//! `to_tsvector('english', chunk_text)`. Queries use `plainto_tsquery` so raw
//! user input is safe without any sanitisation on our side. Ranking uses
//! `ts_rank_cd` (cover density), which weights phrase proximity.
//!
//! Tables are created idempotently on construction, so no separate migration
//! step is needed. Raw `sqlx` is used instead of the `RelationalDb` abstraction
//! because `tsvector` DDL cannot be expressed through `SchemaSet`.

#![cfg(feature = "postgres")]

use crate::errors::{Result, ToolError};
use crate::search::{IndexStats, SearchBackend, SearchResult};
use async_trait::async_trait;
use sqlx::AssertSqlSafe;
use sqlx::Row;

/// PostgreSQL BM25 (tsvector) search backend.
pub struct PostgresBm25Backend {
    pool: sqlx::PgPool,
}

impl PostgresBm25Backend {
    /// Connect and create the search tables if they do not already exist.
    pub async fn new(pool: sqlx::PgPool) -> Result<Self> {
        sqlx::query(AssertSqlSafe(
            "CREATE TABLE IF NOT EXISTS search_fts (
                volume_id  TEXT    NOT NULL,
                path       TEXT    NOT NULL,
                chunk_idx  INTEGER NOT NULL,
                chunk_text TEXT    NOT NULL,
                tsv        TSVECTOR NOT NULL,
                PRIMARY KEY (volume_id, path, chunk_idx)
            )",
        ))
        .execute(&pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: create table: {e}")))?;

        sqlx::query(AssertSqlSafe(
            "CREATE INDEX IF NOT EXISTS idx_search_fts_tsv \
             ON search_fts USING GIN (tsv)",
        ))
        .execute(&pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: create index: {e}")))?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl SearchBackend for PostgresBm25Backend {
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

        // Idempotent: remove any existing rows for this (volume, path) pair.
        sqlx::query(AssertSqlSafe(
            "DELETE FROM search_fts WHERE volume_id = $1 AND path = $2",
        ))
        .bind(volume_id)
        .bind(path)
        .execute(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: delete: {e}")))?;

        for (idx, chunk_text) in chunks.iter().enumerate() {
            sqlx::query(AssertSqlSafe(
                "INSERT INTO search_fts (volume_id, path, chunk_idx, chunk_text, tsv) \
                 VALUES ($1, $2, $3, $4, to_tsvector('english', $4))",
            ))
            .bind(volume_id)
            .bind(path)
            .bind(idx as i32)
            .bind(chunk_text.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| ToolError::internal(format!("bm25_pg: insert chunk {idx}: {e}")))?;
        }

        Ok(n)
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let row = sqlx::query(AssertSqlSafe(
            "WITH deleted AS (
                DELETE FROM search_fts WHERE volume_id = $1 AND path = $2
                RETURNING 1
             ) SELECT COUNT(*) AS cnt FROM deleted",
        ))
        .bind(volume_id)
        .bind(path)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: delete: {e}")))?;

        let cnt: i64 = row.try_get("cnt")
            .map_err(|e| ToolError::internal(format!("bm25_pg: read count: {e}")))?;
        Ok(cnt as usize)
    }

    async fn query_bm25(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let rows = sqlx::query(AssertSqlSafe(
            "SELECT path, chunk_text, \
                    ts_rank_cd(tsv, plainto_tsquery('english', $2)) AS score \
             FROM search_fts \
             WHERE volume_id = $1 \
               AND tsv @@ plainto_tsquery('english', $2) \
             ORDER BY score DESC \
             LIMIT $3",
        ))
        .bind(volume_id)
        .bind(query)
        .bind(top_k as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: query: {e}")))?;

        let mut results = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let path: String = row
                .try_get("path")
                .map_err(|e| ToolError::internal(format!("bm25_pg: read path: {e}")))?;
            let chunk: String = row
                .try_get("chunk_text")
                .map_err(|e| ToolError::internal(format!("bm25_pg: read chunk_text: {e}")))?;
            let score: f32 = row
                .try_get("score")
                .map_err(|e| ToolError::internal(format!("bm25_pg: read score: {e}")))?;
            results.push(SearchResult { path, score, chunk, rank: i + 1 });
        }
        Ok(results)
    }

    async fn query_vector(
        &self,
        _volume_id: &str,
        _query: &str,
        _top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        Err(ToolError::not_supported(
            "vector search is not available in bm25 mode; use mode=rag or mode=both",
        ))
    }

    async fn stats(&self, volume_id: &str) -> Result<IndexStats> {
        let row = sqlx::query(AssertSqlSafe(
            "SELECT COUNT(*) AS cnt FROM search_fts WHERE volume_id = $1",
        ))
        .bind(volume_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: stats: {e}")))?;

        let cnt: i64 = row
            .try_get("cnt")
            .map_err(|e| ToolError::internal(format!("bm25_pg: read stats count: {e}")))?;

        Ok(IndexStats {
            bm25_docs: cnt as usize,
            vector_chunks: 0,
            bm25_warm: cnt > 0,
            mode: "bm25".into(),
        })
    }

    fn supported_modes(&self) -> Vec<&'static str> {
        vec!["bm25"]
    }
}

#[cfg(test)]
mod tests {
    use crate::search::SearchBackend;

    // Live tests against a real Postgres require MCPFS_TEST_PG_DSN to be set.
    // They are skipped automatically in the default gate so no Docker is needed.

    async fn live_pool(schema: &str) -> Option<sqlx::PgPool> {
        let dsn = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())?;
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        let db = PostgresRelationalDb::connect(&dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but connection failed");
        Some(db.pool().clone())
    }

    #[tokio::test]
    async fn live_index_and_query() {
        let Some(pool) = live_pool("mcpfs_bm25pg_test").await else {
            return;
        };
        let b = super::PostgresBm25Backend::new(pool).await.unwrap();
        b.index_path("v1", "/a.md", "the quick brown fox", 200, 0).await.unwrap();
        let r = b.query_bm25("v1", "fox", 10).await.unwrap();
        assert!(!r.is_empty());
        assert_eq!(r[0].path, "/a.md");
    }

    #[tokio::test]
    async fn live_delete_removes_doc() {
        let Some(pool) = live_pool("mcpfs_bm25pg_delete").await else {
            return;
        };
        let b = super::PostgresBm25Backend::new(pool).await.unwrap();
        b.index_path("v1", "/b.md", "hello world", 200, 0).await.unwrap();
        b.delete_path("v1", "/b.md").await.unwrap();
        let r = b.query_bm25("v1", "hello", 10).await.unwrap();
        assert!(r.is_empty());
    }
}
