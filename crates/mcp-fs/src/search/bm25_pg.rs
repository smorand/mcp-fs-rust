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
        sqlx::query(AssertSqlSafe("DELETE FROM search_fts WHERE volume_id = $1 AND path = $2"))
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

        let cnt: i64 = row
            .try_get("cnt")
            .map_err(|e| ToolError::internal(format!("bm25_pg: read count: {e}")))?;
        Ok(cnt as usize)
    }

    async fn delete_all(&self, volume_id: &str) -> Result<usize> {
        let row = sqlx::query(AssertSqlSafe(
            "WITH deleted AS (
                DELETE FROM search_fts WHERE volume_id = $1
                RETURNING 1
             ) SELECT COUNT(*) AS cnt FROM deleted",
        ))
        .bind(volume_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::internal(format!("bm25_pg: delete all: {e}")))?;

        let cnt: i64 = row
            .try_get("cnt")
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
    use crate::errors::code;
    use crate::search::SearchBackend;

    // Live tests require MCPFS_TEST_PG_DSN. They skip automatically when the
    // variable is unset, so `cargo test --workspace` stays green with no Docker.

    async fn live_backend(schema: &str) -> Option<super::PostgresBm25Backend> {
        let dsn = std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())?;
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        let db = PostgresRelationalDb::connect(&dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but connection failed");
        Some(super::PostgresBm25Backend::new(db.pool().clone()).await.unwrap())
    }

    #[tokio::test]
    async fn live_index_and_query() {
        let Some(b) = live_backend("mcpfs_bm25pg_idx").await else { return };
        b.index_path("v1", "/a.md", "the quick brown fox jumps", 200, 0).await.unwrap();
        let r = b.query_bm25("v1", "fox", 10).await.unwrap();
        assert!(!r.is_empty(), "expected at least one result");
        assert_eq!(r[0].path, "/a.md");
    }

    #[tokio::test]
    async fn live_ranks_start_at_one() {
        let Some(b) = live_backend("mcpfs_bm25pg_ranks").await else { return };
        b.index_path("v1", "/a.md", "quick fox", 200, 0).await.unwrap();
        b.index_path("v1", "/b.md", "quick fox brown", 200, 0).await.unwrap();
        let r = b.query_bm25("v1", "fox", 10).await.unwrap();
        assert_eq!(r[0].rank, 1, "first result must have rank 1");
        for (i, res) in r.iter().enumerate() {
            assert_eq!(res.rank, i + 1, "ranks must be consecutive");
        }
    }

    #[tokio::test]
    async fn live_delete_removes_doc() {
        let Some(b) = live_backend("mcpfs_bm25pg_del").await else { return };
        b.index_path("v1", "/b.md", "hello world test", 200, 0).await.unwrap();
        let deleted = b.delete_path("v1", "/b.md").await.unwrap();
        assert!(deleted > 0, "delete must report at least one chunk removed");
        let r = b.query_bm25("v1", "hello", 10).await.unwrap();
        assert!(r.is_empty(), "deleted doc must not appear in results");
    }

    #[tokio::test]
    async fn live_stats_zero_on_empty_volume() {
        let Some(b) = live_backend("mcpfs_bm25pg_stats").await else { return };
        let s = b.stats("empty_vol").await.unwrap();
        assert_eq!(s.bm25_docs, 0);
        assert!(!s.bm25_warm, "empty volume must not be warm");
    }

    #[tokio::test]
    async fn live_stats_counts_chunks_after_index() {
        let Some(b) = live_backend("mcpfs_bm25pg_stats2").await else { return };
        // Two files, each one chunk at this size.
        b.index_path("v1", "/a.md", "alpha beta gamma", 200, 0).await.unwrap();
        b.index_path("v1", "/b.md", "delta epsilon zeta", 200, 0).await.unwrap();
        let s = b.stats("v1").await.unwrap();
        assert_eq!(s.bm25_docs, 2, "expected 2 chunks");
        assert!(s.bm25_warm, "non-empty volume must be warm");
    }

    #[tokio::test]
    async fn live_idempotent_reindex() {
        let Some(b) = live_backend("mcpfs_bm25pg_idem").await else { return };
        b.index_path("v1", "/a.md", "first version content", 200, 0).await.unwrap();
        // Re-index the same path: should replace, not duplicate.
        b.index_path("v1", "/a.md", "second version content", 200, 0).await.unwrap();
        let s = b.stats("v1").await.unwrap();
        assert_eq!(s.bm25_docs, 1, "re-index must replace, not append");
        // The old content must be gone.
        let old = b.query_bm25("v1", "first", 10).await.unwrap();
        assert!(old.is_empty(), "old content must not appear after re-index");
        // The new content must be present.
        let new = b.query_bm25("v1", "second", 10).await.unwrap();
        assert!(!new.is_empty(), "new content must be findable");
    }

    #[tokio::test]
    async fn live_volume_isolation() {
        let Some(b) = live_backend("mcpfs_bm25pg_iso").await else { return };
        b.index_path("vol_a", "/shared.md", "document in volume a", 200, 0).await.unwrap();
        b.index_path("vol_b", "/shared.md", "document in volume b", 200, 0).await.unwrap();
        // Query vol_a must not return vol_b rows.
        let ra = b.query_bm25("vol_a", "document", 10).await.unwrap();
        assert!(ra.iter().all(|r| r.path == "/shared.md"), "vol_a results must come from vol_a");
        let cnt_a = b.stats("vol_a").await.unwrap().bm25_docs;
        let cnt_b = b.stats("vol_b").await.unwrap().bm25_docs;
        assert_eq!(cnt_a, 1);
        assert_eq!(cnt_b, 1);
    }

    #[tokio::test]
    async fn live_delete_all_removes_every_chunk_of_the_volume() {
        let Some(b) = live_backend("mcpfs_bm25pg_delall").await else { return };
        b.index_path("v1", "/a.md", "alpha content", 200, 0).await.unwrap();
        b.index_path("v1", "/b.md", "beta content", 200, 0).await.unwrap();
        assert_eq!(b.stats("v1").await.unwrap().bm25_docs, 2);

        assert_eq!(b.delete_all("v1").await.unwrap(), 2, "delete_all reports what it removed");
        assert_eq!(b.stats("v1").await.unwrap().bm25_docs, 0);
        assert!(b.query_bm25("v1", "content", 10).await.unwrap().is_empty());
    }

    /// One database holds every volume, so a wipe without the volume_id filter
    /// would empty another project's index.
    #[tokio::test]
    async fn live_delete_all_is_scoped_to_one_volume() {
        let Some(b) = live_backend("mcpfs_bm25pg_delall_iso").await else { return };
        b.index_path("vol_a", "/x.md", "content a", 200, 0).await.unwrap();
        b.index_path("vol_b", "/x.md", "content b", 200, 0).await.unwrap();
        b.delete_all("vol_a").await.unwrap();
        assert_eq!(b.stats("vol_a").await.unwrap().bm25_docs, 0);
        assert_eq!(b.stats("vol_b").await.unwrap().bm25_docs, 1, "the other volume survives");
    }

    #[tokio::test]
    async fn live_delete_all_on_an_empty_volume_is_zero() {
        let Some(b) = live_backend("mcpfs_bm25pg_delall_empty").await else { return };
        assert_eq!(b.delete_all("never-seen").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn live_query_vector_returns_not_supported() {
        let Some(b) = live_backend("mcpfs_bm25pg_norag").await else { return };
        let err = b.query_vector("v1", "anything", 10).await.unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
    }
}
