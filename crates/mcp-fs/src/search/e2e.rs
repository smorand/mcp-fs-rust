//! End-to-end integration tests for the `search.*` MCP tools.
//!
//! Tests drive the real tool-dispatch path (`registry.call`) through a real
//! `AppState`, for both SQLite-RAG and PostgreSQL-RAG backends. Each test creates
//! any required database state, runs the full scenario, and cleans up afterward.
//!
//! ## Module layout
//!
//! - `shared`     — helpers shared by both suites
//! - `sqlite_rag` — SQLite-RAG e2e tests (`#[cfg(feature = "rag")]`)
//! - `pg_rag`     — PostgreSQL-RAG e2e tests (`#[cfg(feature = "rag")]`)

#[cfg(test)]
mod shared {
    use crate::tools::testkit::Harness;

    /// The mount / project id used across all e2e tests — same as `testkit::MOUNT`.
    pub const MOUNT: &str = "proj";

    // ── fake embedding server ─────────────────────────────────────────────────

    /// Spawn an inline axum stub on an ephemeral port that returns a fixed
    /// `dims`-dimensional embedding for ANY POST to `/v1/embeddings`.
    /// Returns the base URL (`http://127.0.0.1:{port}`).
    pub async fn spawn_fake_embedding(dims: usize) -> String {
        use axum::routing::post;
        use axum::{Json, Router};

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

    // ── E2eHarness ───────────────────────────────────────────────────────────

    /// A thin wrapper around `testkit::Harness` that carries any extra resources
    /// needed by an e2e test (temp dirs, cleanup callbacks).
    pub struct E2eHarness {
        pub inner: Harness,
        /// Keeps the Tantivy temp dir alive for the duration of the test.
        pub _tantivy_dir: Option<tempfile::TempDir>,
        /// Called when the harness is dropped (used for PG schema cleanup).
        pub cleanup: Option<Box<dyn FnOnce() + Send>>,
    }

    impl E2eHarness {
        /// Dispatch a tool call through the registry, identical to `testkit::Harness::call`.
        pub async fn call(
            &self,
            tool: &str,
            args: serde_json::Value,
        ) -> crate::errors::Result<serde_json::Value> {
            self.inner.call(tool, args).await
        }
    }

    impl Drop for E2eHarness {
        fn drop(&mut self) {
            if let Some(cb) = self.cleanup.take() {
                cb();
            }
        }
    }

    // ── SQLite harness builder ────────────────────────────────────────────────

    /// Build a full harness wired with a real `SqliteVecBackend`.
    ///
    /// Requires the `rag` feature.
    #[cfg(feature = "rag")]
    pub async fn make_sqlite_rag_harness(embedding_url: &str, dims: u32) -> E2eHarness {
        use crate::config::EmbeddingConfig;
        use crate::search::vector_sqlite::SqliteVecBackend;
        use crate::tools::testkit::harness_with_search;
        use std::sync::Arc;

        // Ensure sqlite-vec extension is loaded.
        crate::storage::sqlite::register_sqlite_vec();

        let tantivy_dir = tempfile::tempdir().expect("failed to create tantivy temp dir");
        let client = Arc::new(reqwest::Client::new());
        let cfg = EmbeddingConfig {
            endpoint: format!("{embedding_url}/v1/embeddings"),
            model: "fake".to_string(),
            api_key_env: String::new(),
            dimensions: dims,
        };
        let backend = Arc::new(SqliteVecBackend::new(
            dims,
            tantivy_dir.path().to_str().expect("temp dir has valid utf-8 path"),
            client,
            cfg,
        )) as Arc<dyn crate::search::SearchBackend>;

        let endpoint_for_config = format!("{embedding_url}/v1/embeddings");
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "rag".into();
                c.search.embedding.endpoint = endpoint_for_config;
                c.search.embedding.dimensions = dims;
            },
            Some(backend),
        )
        .await;

        E2eHarness { inner: h, _tantivy_dir: Some(tantivy_dir), cleanup: None }
    }

    // ── PostgreSQL harness builder ────────────────────────────────────────────

    /// Build a full harness wired with a real `PostgresVectorBackend`.
    ///
    /// Returns `None` when `MCPFS_TEST_PG_DSN` is unset (skips the test).
    /// Requires the `rag` feature.
    #[cfg(feature = "rag")]
    pub async fn make_pg_rag_harness(
        dsn: &str,
        schema: &str,
        embedding_url: &str,
        dims: u32,
    ) -> E2eHarness {
        use crate::config::EmbeddingConfig;
        use crate::search::vector_pg::PostgresVectorBackend;
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        use crate::tools::testkit::harness_with_search;
        use std::sync::Arc;

        let db = PostgresRelationalDb::connect(dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but connection failed");
        let pool = db.pool().clone();

        let client = Arc::new(reqwest::Client::new());
        let cfg = EmbeddingConfig {
            endpoint: format!("{embedding_url}/v1/embeddings"),
            model: "fake".to_string(),
            api_key_env: String::new(),
            dimensions: dims,
        };
        let backend =
            Arc::new(PostgresVectorBackend::new(pool.clone(), dims, client, cfg).await.expect(
                "PostgresVectorBackend::new failed — check that pgvector is installed",
            )) as Arc<dyn crate::search::SearchBackend>;

        let endpoint_for_config = format!("{embedding_url}/v1/embeddings");
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "rag".into();
                c.search.embedding.endpoint = endpoint_for_config;
                c.search.embedding.dimensions = dims;
            },
            Some(backend),
        )
        .await;

        // Drop-cleanup: remove the schema so tests are fully isolated.
        let cleanup_pool = pool.clone();
        let cleanup_schema = schema.to_string();
        let cleanup: Box<dyn FnOnce() + Send> = Box::new(move || {
            let pool = cleanup_pool;
            let schema = cleanup_schema;
            // We can't call block_on on the existing Tokio runtime handle inside
            // Drop (that panics: "Cannot start a runtime from within a Tokio runtime").
            // Spawn a new OS thread with its own single-threaded runtime instead.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("cleanup runtime");
                rt.block_on(async move {
                    let sql =
                        format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE");
                    let _ = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                        .execute(&pool)
                        .await;
                });
            })
            .join()
            .ok();
        });

        E2eHarness { inner: h, _tantivy_dir: None, cleanup: Some(cleanup) }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SQLite-RAG e2e tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "rag")]
mod sqlite_rag {
    use super::shared::{MOUNT, make_sqlite_rag_harness, spawn_fake_embedding};
    use serde_json::json;

    /// Index a file and verify that `search.status` reports chunks.
    #[tokio::test]
    async fn e2e_sqlite_rag_index_creates_chunks() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        h.inner.seed("/doc.md", "rust is a systems programming language").await;

        let resp = h
            .call("search.index", json!({"mount_id": MOUNT, "path": "/doc.md"}))
            .await
            .expect("search.index must succeed");

        assert!(
            resp["indexed"].as_u64().unwrap_or(0) > 0,
            "indexed must be > 0, got {resp}"
        );
        assert_eq!(resp["skipped"], 0, "skipped must be 0, got {resp}");

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("search.status must succeed");
        assert!(
            status["vector_chunks"].as_u64().unwrap_or(0) > 0,
            "vector_chunks must be > 0 after indexing, got {status}"
        );
    }

    /// Query after indexing returns ranked results.
    #[tokio::test]
    async fn e2e_sqlite_rag_query_returns_ranked_results() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        h.inner.seed("/a.md", "the quick brown fox jumps over the lazy dog").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("search.index must succeed");

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "fox", "mode": "rag", "top_k": 5}),
            )
            .await
            .expect("search.query must succeed");

        let results = resp["results"].as_array().expect("results must be an array");
        assert!(!results.is_empty(), "results must be non-empty");
        assert_eq!(results[0]["path"], "/a.md", "top result must be /a.md");
        assert_eq!(results[0]["rank"], 1, "top result must have rank 1");
        assert!(
            results[0]["score"].as_f64().unwrap_or(0.0) > 0.0,
            "score must be positive"
        );
        assert_eq!(resp["mode_used"], "rag", "mode_used must be rag");
    }

    /// Delete removes the document from results and reduces vector_chunks to 0.
    #[tokio::test]
    async fn e2e_sqlite_rag_delete_removes_from_results() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        h.inner.seed("/a.md", "unique content about foxes and dogs").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("index must succeed");

        let del = h
            .call("search.delete", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("search.delete must succeed");
        assert!(
            del["deleted"].as_u64().unwrap_or(0) > 0,
            "deleted must be > 0, got {del}"
        );

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "fox", "mode": "rag", "top_k": 10}),
            )
            .await
            .expect("query after delete must succeed");
        let results = resp["results"].as_array().expect("results must be array");
        assert!(results.is_empty(), "results must be empty after delete, got {resp}");

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(99),
            0,
            "vector_chunks must be 0 after delete, got {status}"
        );
    }

    /// Re-indexing the same path replaces, not appends (idempotent).
    #[tokio::test]
    async fn e2e_sqlite_rag_idempotent_reindex() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        h.inner.seed("/a.md", "first content about systems programming").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("first index must succeed");

        // Overwrite with different content and re-index.
        h.inner.seed("/a.md", "second content about web development").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("second index must succeed");

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(99),
            1,
            "re-index must replace, not append: got {status}"
        );
    }

    /// Index 3 files separately; status reports 3 chunks.
    #[tokio::test]
    async fn e2e_sqlite_rag_multi_file_index() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        for (path, text) in [
            ("/a.md", "content about alpha"),
            ("/b.md", "content about beta"),
            ("/c.md", "content about gamma"),
        ] {
            h.inner.seed(path, text).await;
            h.call("search.index", json!({"mount_id": MOUNT, "path": path}))
                .await
                .expect("index must succeed");
        }

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(0),
            3,
            "expected 3 vector_chunks, got {status}"
        );
    }

    /// Recursive indexing of a directory indexes all files underneath it.
    #[tokio::test]
    async fn e2e_sqlite_rag_recursive_index() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        h.inner.seed("/docs/a.md", "documentation about alpha features").await;
        h.inner.seed("/docs/b.md", "documentation about beta features").await;

        let resp = h
            .call(
                "search.index",
                json!({"mount_id": MOUNT, "path": "/docs", "recursive": true}),
            )
            .await
            .expect("recursive index must succeed");
        assert!(
            resp["indexed"].as_u64().unwrap_or(0) >= 2,
            "indexed must be >= 2, got {resp}"
        );

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert!(
            status["vector_chunks"].as_u64().unwrap_or(0) >= 2,
            "vector_chunks must be >= 2 after recursive index, got {status}"
        );
    }

    /// Querying an empty index returns empty results (no error).
    #[tokio::test]
    async fn e2e_sqlite_rag_query_empty_index_returns_empty_results() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "anything", "mode": "rag", "top_k": 10}),
            )
            .await
            .expect("query on empty index must succeed (no error)");

        let results = resp["results"].as_array().expect("results must be array");
        assert!(
            results.is_empty(),
            "results must be empty on an unindexed volume, got {resp}"
        );
    }

    /// `top_k` is respected: results.len() <= top_k.
    #[tokio::test]
    async fn e2e_sqlite_rag_top_k_respected() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        for i in 0..5u32 {
            let path = format!("/file{i}.md");
            h.inner.seed(&path, &format!("content for document number {i} about systems")).await;
            h.call("search.index", json!({"mount_id": MOUNT, "path": path}))
                .await
                .expect("index must succeed");
        }

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "systems", "mode": "rag", "top_k": 2}),
            )
            .await
            .expect("query must succeed");

        let results = resp["results"].as_array().expect("results must be array");
        assert!(
            results.len() <= 2,
            "results.len() must be <= top_k=2, got {}",
            results.len()
        );
    }

    /// A non-member is forbidden before any storage access.
    #[tokio::test]
    async fn e2e_sqlite_rag_non_member_is_forbidden() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        let ctx = crate::mcp::registry::ToolCtx {
            person: "stranger@x.y".to_string(),
            state: h.inner.state.clone(),
        };
        let err = h
            .inner
            .state
            .registry
            .call(
                "search.index",
                ctx,
                crate::mcp::Args::new(json!({"mount_id": MOUNT, "path": "/doc.md"})),
            )
            .await
            .expect("tool must be registered")
            .expect_err("non-member must be forbidden");

        assert_eq!(
            err.code,
            crate::errors::code::FORBIDDEN,
            "expected ERR_FORBIDDEN, got {err}"
        );
    }

    /// Indexing a binary (non-UTF-8) file does not return an error.
    ///
    /// NOTE: the volume's `read_text` uses `from_utf8_lossy`, so binary files
    /// are never rejected at read time — they are indexed as garbled text.
    /// This test verifies that the tool completes without panicking or returning
    /// an error code. `indexed + skipped` must equal the number of paths attempted.
    #[tokio::test]
    async fn e2e_sqlite_rag_index_binary_file_skips_gracefully() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        // Write raw non-UTF-8 bytes directly to the volume.
        h.inner
            .client()
            .await
            .write_bytes_atomic("/binary.bin", &[0xFF, 0xFE, 0x00, 0x01])
            .await
            .expect("write_bytes_atomic must succeed");

        // The call must succeed (no error returned), regardless of whether the
        // file is indexed or skipped — the exact behaviour depends on whether
        // the embedding call tolerates garbled input.
        let resp = h
            .call("search.index", json!({"mount_id": MOUNT, "path": "/binary.bin"}))
            .await
            .expect("search.index on binary must succeed (not error)");

        // indexed + skipped must equal 1 (one path was attempted).
        let indexed = resp["indexed"].as_u64().unwrap_or(0);
        let skipped = resp["skipped"].as_u64().unwrap_or(0);
        assert_eq!(
            indexed + skipped,
            1,
            "indexed + skipped must equal 1, got indexed={indexed} skipped={skipped}"
        );
    }

    /// Requesting `mode=bm25` against a RAG-only backend returns NOT_SUPPORTED.
    #[tokio::test]
    async fn e2e_sqlite_rag_query_mode_bm25_returns_not_supported() {
        let base = spawn_fake_embedding(3).await;
        let h = make_sqlite_rag_harness(&base, 3).await;

        let err = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "anything", "mode": "bm25", "top_k": 10}),
            )
            .await
            .expect_err("bm25 mode on rag backend must return an error");

        assert_eq!(
            err.code,
            crate::errors::code::NOT_SUPPORTED,
            "expected ERR_NOT_SUPPORTED, got {err}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PostgreSQL-RAG e2e tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "rag")]
mod pg_rag {
    use super::shared::{MOUNT, make_pg_rag_harness, spawn_fake_embedding};
    use serde_json::json;

    /// Read `MCPFS_TEST_PG_DSN`. Returns `None` when unset (skip the test).
    fn pg_dsn() -> Option<String> {
        std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
    }

    /// Generate a unique schema name so parallel tests don't collide.
    fn unique_schema() -> String {
        let id = uuid::Uuid::new_v4().to_string().replace('-', "");
        format!("mcpfs_e2e_{}", &id[..12])
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn e2e_pg_rag_index_creates_chunks() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner.seed("/doc.md", "rust is a systems programming language").await;
        let resp = h
            .call("search.index", json!({"mount_id": MOUNT, "path": "/doc.md"}))
            .await
            .expect("search.index must succeed");

        assert!(
            resp["indexed"].as_u64().unwrap_or(0) > 0,
            "indexed must be > 0, got {resp}"
        );
        assert_eq!(resp["skipped"], 0);

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("search.status must succeed");
        assert!(
            status["vector_chunks"].as_u64().unwrap_or(0) > 0,
            "vector_chunks must be > 0, got {status}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_query_returns_ranked_results() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner.seed("/a.md", "the quick brown fox jumps over the lazy dog").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("index must succeed");

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "fox", "mode": "rag", "top_k": 5}),
            )
            .await
            .expect("query must succeed");

        let results = resp["results"].as_array().expect("results must be array");
        assert!(!results.is_empty(), "results must be non-empty");
        assert_eq!(results[0]["path"], "/a.md");
        assert_eq!(results[0]["rank"], 1);
        assert!(results[0]["score"].as_f64().unwrap_or(0.0) > 0.0, "score must be > 0");
        assert_eq!(resp["mode_used"], "rag");
    }

    #[tokio::test]
    async fn e2e_pg_rag_delete_removes_from_results() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner.seed("/a.md", "unique content for deletion test").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("index must succeed");

        let del = h
            .call("search.delete", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("delete must succeed");
        assert!(del["deleted"].as_u64().unwrap_or(0) > 0, "deleted must be > 0, got {del}");

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "deletion", "mode": "rag", "top_k": 10}),
            )
            .await
            .expect("query after delete must succeed");
        let results = resp["results"].as_array().expect("results must be array");
        assert!(results.is_empty(), "results must be empty after delete, got {resp}");

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(99),
            0,
            "vector_chunks must be 0 after delete, got {status}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_idempotent_reindex() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner.seed("/a.md", "first content about systems programming").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("first index must succeed");

        h.inner.seed("/a.md", "second content about web development").await;
        h.call("search.index", json!({"mount_id": MOUNT, "path": "/a.md"}))
            .await
            .expect("second index must succeed");

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(99),
            1,
            "re-index must replace, not append: got {status}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_multi_file_index() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        for (path, text) in [
            ("/a.md", "content about alpha"),
            ("/b.md", "content about beta"),
            ("/c.md", "content about gamma"),
        ] {
            h.inner.seed(path, text).await;
            h.call("search.index", json!({"mount_id": MOUNT, "path": path}))
                .await
                .expect("index must succeed");
        }

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert_eq!(
            status["vector_chunks"].as_u64().unwrap_or(0),
            3,
            "expected 3 vector_chunks, got {status}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_recursive_index() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner.seed("/docs/a.md", "documentation about alpha features").await;
        h.inner.seed("/docs/b.md", "documentation about beta features").await;

        let resp = h
            .call(
                "search.index",
                json!({"mount_id": MOUNT, "path": "/docs", "recursive": true}),
            )
            .await
            .expect("recursive index must succeed");
        assert!(
            resp["indexed"].as_u64().unwrap_or(0) >= 2,
            "indexed must be >= 2, got {resp}"
        );

        let status = h
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status must succeed");
        assert!(
            status["vector_chunks"].as_u64().unwrap_or(0) >= 2,
            "vector_chunks must be >= 2, got {status}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_query_empty_index_returns_empty_results() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "anything", "mode": "rag", "top_k": 10}),
            )
            .await
            .expect("query on empty index must succeed (no error)");

        let results = resp["results"].as_array().expect("results must be array");
        assert!(
            results.is_empty(),
            "results must be empty on an unindexed volume, got {resp}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_top_k_respected() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        for i in 0..5u32 {
            let path = format!("/file{i}.md");
            h.inner.seed(&path, &format!("content for document number {i} about systems")).await;
            h.call("search.index", json!({"mount_id": MOUNT, "path": path}))
                .await
                .expect("index must succeed");
        }

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "systems", "mode": "rag", "top_k": 2}),
            )
            .await
            .expect("query must succeed");

        let results = resp["results"].as_array().expect("results must be array");
        assert!(
            results.len() <= 2,
            "results.len() must be <= top_k=2, got {}",
            results.len()
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_non_member_is_forbidden() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        let ctx = crate::mcp::registry::ToolCtx {
            person: "stranger@x.y".to_string(),
            state: h.inner.state.clone(),
        };
        let err = h
            .inner
            .state
            .registry
            .call(
                "search.index",
                ctx,
                crate::mcp::Args::new(json!({"mount_id": MOUNT, "path": "/doc.md"})),
            )
            .await
            .expect("tool must be registered")
            .expect_err("non-member must be forbidden");

        assert_eq!(
            err.code,
            crate::errors::code::FORBIDDEN,
            "expected ERR_FORBIDDEN, got {err}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_index_binary_file_skips_gracefully() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        h.inner
            .client()
            .await
            .write_bytes_atomic("/binary.bin", &[0xFF, 0xFE, 0x00, 0x01])
            .await
            .expect("write_bytes_atomic must succeed");

        // Must succeed (no error). indexed + skipped must equal 1.
        let resp = h
            .call("search.index", json!({"mount_id": MOUNT, "path": "/binary.bin"}))
            .await
            .expect("search.index on binary must succeed (not error)");

        let indexed = resp["indexed"].as_u64().unwrap_or(0);
        let skipped = resp["skipped"].as_u64().unwrap_or(0);
        assert_eq!(
            indexed + skipped,
            1,
            "indexed + skipped must equal 1, got indexed={indexed} skipped={skipped}"
        );
    }

    #[tokio::test]
    async fn e2e_pg_rag_query_mode_bm25_returns_not_supported() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;
        let schema = unique_schema();
        let h = make_pg_rag_harness(&dsn, &schema, &base, 3).await;

        let err = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "anything", "mode": "bm25", "top_k": 10}),
            )
            .await
            .expect_err("bm25 mode on rag backend must return an error");

        assert_eq!(
            err.code,
            crate::errors::code::NOT_SUPPORTED,
            "expected ERR_NOT_SUPPORTED, got {err}"
        );
    }

    /// Two independent harnesses (different schemas) — index content in each,
    /// verify queries are isolated to their own volume.
    #[tokio::test]
    async fn e2e_pg_rag_volume_isolation() {
        let Some(dsn) = pg_dsn() else { return };
        let base = spawn_fake_embedding(3).await;

        // Use two separate schemas (and hence two separate harnesses).
        let schema_a = unique_schema();
        let schema_b = unique_schema();

        let ha = make_pg_rag_harness(&dsn, &schema_a, &base, 3).await;
        let hb = make_pg_rag_harness(&dsn, &schema_b, &base, 3).await;

        ha.inner.seed("/shared.md", "alpha specific content").await;
        ha.call("search.index", json!({"mount_id": MOUNT, "path": "/shared.md"}))
            .await
            .expect("index in A must succeed");

        hb.inner.seed("/shared.md", "beta specific content").await;
        hb.call("search.index", json!({"mount_id": MOUNT, "path": "/shared.md"}))
            .await
            .expect("index in B must succeed");

        // Volume A must report exactly 1 chunk; Volume B must independently report 1.
        let sa = ha
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status in A");
        let sb = hb
            .call("search.status", json!({"mount_id": MOUNT}))
            .await
            .expect("status in B");

        assert_eq!(sa["vector_chunks"], 1, "A must have exactly 1 chunk, got {sa}");
        assert_eq!(sb["vector_chunks"], 1, "B must have exactly 1 chunk, got {sb}");
    }
}
