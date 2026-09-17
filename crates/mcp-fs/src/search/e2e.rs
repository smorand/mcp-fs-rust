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

    /// A throwaway RSA keypair on disk, plus the public key path the harness
    /// config points `auth.jwt` at.
    ///
    /// The REST plane has no back door: `person_of` resolves the caller by
    /// verifying a bearer token, so an e2e harness that wants an HTTP surface
    /// needs a real key and real tokens rather than an injected identity.
    pub struct JwtKeys {
        /// Keeps the key files alive for the duration of the test.
        pub _dir: tempfile::TempDir,
        pub private: std::path::PathBuf,
        pub public: String,
    }

    /// Write a keypair for one harness.
    pub fn jwt_keys() -> JwtKeys {
        let dir = tempfile::tempdir().expect("failed to create key temp dir");
        let (private, public) =
            crate::keys::write_keypair(dir.path().join("keys")).expect("keypair must be writable");
        JwtKeys { _dir: dir, private, public: public.display().to_string() }
    }

    /// A thin wrapper around `testkit::Harness` that carries any extra resources
    /// needed by an e2e test (temp dirs, cleanup callbacks).
    pub struct E2eHarness {
        pub inner: Harness,
        /// Keeps the Tantivy temp dir alive for the duration of the test.
        pub _tantivy_dir: Option<tempfile::TempDir>,
        /// The keypair whose public half this harness's `AppState` verifies.
        pub keys: JwtKeys,
        /// Called when the harness is dropped (used for PG schema cleanup).
        pub cleanup: Option<Box<dyn FnOnce() + Send>>,
    }

    impl E2eHarness {
        /// A bearer token for `person`, signed by this harness's key.
        pub fn token_for(&self, person: &str) -> String {
            crate::keys::mint_token_from_file(
                &self.keys.private,
                person,
                crate::keys::DEFAULT_ISSUER,
                crate::keys::DEFAULT_CLAIM,
                3600,
            )
            .expect("the token must be mintable")
        }

        /// Drive one request through the REST data plane router, as `person`.
        ///
        /// The router is built from the very `Arc<AppState>` the tool calls use,
        /// so a REST hook and an MCP hook are observed against one index.
        /// `person` is a parameter so a test can send as somebody who must be
        /// refused, instead of only ever as the owner.
        pub async fn rest(
            &self,
            method: &str,
            uri: &str,
            content_type: &str,
            body: Vec<u8>,
            person: &str,
        ) -> (axum::http::StatusCode, serde_json::Value) {
            use tower::ServiceExt as _;

            let request = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("Authorization", format!("Bearer {}", self.token_for(person)))
                .header("Content-Type", content_type)
                .body(axum::body::Body::from(body))
                .expect("the request must be buildable");
            let response = crate::api::dataplane::router(self.inner.state.clone())
                .oneshot(request)
                .await
                .expect("the router is infallible");
            let status = response.status();
            let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("the response body must be readable");
            // A non JSON body (an empty 401, say) still has a status worth
            // asserting on, so it becomes Null rather than a panic.
            (status, serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null))
        }

        /// `POST /api/fs/{mount}/{sub}` with a JSON body, as `person`.
        pub async fn rest_post(
            &self,
            mount: &str,
            sub: &str,
            body: serde_json::Value,
            person: &str,
        ) -> (axum::http::StatusCode, serde_json::Value) {
            self.rest(
                "POST",
                &format!("/api/fs/{mount}/{sub}"),
                "application/json",
                body.to_string().into_bytes(),
                person,
            )
            .await
        }

        /// `POST /api/fs/{mount}/upload` with one file part, as `person`.
        ///
        /// The multipart body is written out by hand because its exact shape is
        /// the contract the handler parses, same as the data plane's own tests.
        pub async fn rest_upload(
            &self,
            mount: &str,
            directory: &str,
            file_name: &str,
            content: &str,
            person: &str,
        ) -> (axum::http::StatusCode, serde_json::Value) {
            const BOUNDARY: &str = "E2E-BOUNDARY";
            let body = format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"directory\"\r\n\r\n\
                 {directory}\r\n\
                 --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; \
                 filename=\"{file_name}\"\r\n\r\n{content}\r\n\
                 --{BOUNDARY}--\r\n"
            );
            self.rest(
                "POST",
                &format!("/api/fs/{mount}/upload"),
                &format!("multipart/form-data; boundary={BOUNDARY}"),
                body.into_bytes(),
                person,
            )
            .await
        }

        /// Dispatch a tool call through the registry, identical to `testkit::Harness::call`.
        pub async fn call(
            &self,
            tool: &str,
            args: serde_json::Value,
        ) -> crate::errors::Result<serde_json::Value> {
            self.inner.call(tool, args).await
        }

        /// Create a second project owned by the same person, volume included.
        pub async fn add_project(&self, project_id: &str) {
            self.inner
                .state
                .admin
                .create_project(project_id, crate::tools::testkit::PERSON)
                .await
                .expect("the second project must be creatable");
            self.inner
                .state
                .stores
                .provision_volume(project_id)
                .await
                .expect("the second volume must be provisionable");
        }

        /// Write a file into `mount` through the real `fs.write` tool.
        pub async fn write_file(&self, mount: &str, path: &str, content: &str) {
            self.call(
                "fs.write",
                serde_json::json!({
                    "mount_id": mount, "path": path, "content": content, "overwrite": true
                }),
            )
            .await
            .unwrap_or_else(|e| panic!("fs.write {path} must succeed: {e}"));
        }

        /// `admin.set_index_mode`, as the owner.
        pub async fn set_mode(
            &self,
            mount: &str,
            mode: &str,
        ) -> crate::errors::Result<serde_json::Value> {
            self.call(
                "admin.set_index_mode",
                serde_json::json!({"project_id": mount, "mode": mode}),
            )
            .await
        }

        /// The indexed chunk count reported by `search.status`.
        ///
        /// Summed across both halves because a BM25 backend reports `bm25_docs`
        /// and a vector backend `vector_chunks`, and the same scenario runs
        /// against either.
        pub async fn chunk_count(&self, mount: &str) -> u64 {
            let s = self
                .call("search.status", serde_json::json!({"mount_id": mount}))
                .await
                .expect("search.status must succeed");
            s["bm25_docs"].as_u64().unwrap_or(0) + s["vector_chunks"].as_u64().unwrap_or(0)
        }

        /// Wait until `search.status` reports exactly `expected` chunks.
        ///
        /// Auto indexing is fire and forget, so the index lags the write by one
        /// backend round trip. Polling to a bounded deadline is the only honest
        /// way to observe it: a fixed sleep would either be flaky or hide a hook
        /// that never fired. A timeout FAILS the test, it does not pass quietly.
        pub async fn await_chunks(&self, mount: &str, expected: u64) {
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(AUTO_INDEX_TIMEOUT_SECS);
            let mut last = u64::MAX;
            while std::time::Instant::now() < deadline {
                last = self.chunk_count(mount).await;
                if last == expected {
                    // Settled: give a straggler task a beat to add a chunk we
                    // would then see as a wrong count rather than as a pass.
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let again = self.chunk_count(mount).await;
                    assert_eq!(
                        again, expected,
                        "{mount}: the chunk count moved to {again} after settling at {expected}"
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!(
                "{mount}: timed out after {AUTO_INDEX_TIMEOUT_SECS}s waiting for {expected} \
                 indexed chunks, last saw {last}"
            );
        }
    }

    /// How long an auto index hook may take before the test calls it broken.
    /// Generous next to a fake embedding server on loopback, which answers in
    /// microseconds, so a timeout here means the hook never fired.
    pub const AUTO_INDEX_TIMEOUT_SECS: u64 = 15;

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
        let keys = jwt_keys();
        let public_key_path = keys.public.clone();
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "rag".into();
                c.search.embedding.endpoint = endpoint_for_config;
                c.search.embedding.dimensions = dims;
                c.auth.jwt.public_key_path = public_key_path;
            },
            Some(backend),
        )
        .await;

        E2eHarness { inner: h, _tantivy_dir: Some(tantivy_dir), keys, cleanup: None }
    }

    /// Build a full harness wired with a real Tantivy BM25 backend and NO
    /// embedding endpoint, which is what makes it the fixture for the rag/both
    /// rejection case as well as for plain BM25 auto indexing.
    pub async fn make_sqlite_bm25_harness() -> E2eHarness {
        use crate::search::bm25_sqlite::TantivyBm25Backend;
        use crate::tools::testkit::harness_with_search;
        use std::sync::Arc;

        let tantivy_dir = tempfile::tempdir().expect("failed to create tantivy temp dir");
        let backend = Arc::new(TantivyBm25Backend::new(
            tantivy_dir.path().to_str().expect("temp dir has valid utf-8 path"),
        )) as Arc<dyn crate::search::SearchBackend>;

        let keys = jwt_keys();
        let public_key_path = keys.public.clone();
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "bm25".into();
                c.auth.jwt.public_key_path = public_key_path;
            },
            Some(backend),
        )
        .await;

        E2eHarness { inner: h, _tantivy_dir: Some(tantivy_dir), keys, cleanup: None }
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
        let backend = Arc::new(
            PostgresVectorBackend::new(pool.clone(), dims, client, cfg)
                .await
                .expect("PostgresVectorBackend::new failed — check that pgvector is installed"),
        ) as Arc<dyn crate::search::SearchBackend>;

        let endpoint_for_config = format!("{embedding_url}/v1/embeddings");
        let keys = jwt_keys();
        let public_key_path = keys.public.clone();
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "rag".into();
                c.search.embedding.endpoint = endpoint_for_config;
                c.search.embedding.dimensions = dims;
                c.auth.jwt.public_key_path = public_key_path;
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
                    let sql = format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE");
                    let _ = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).execute(&pool).await;
                });
            })
            .join()
            .ok();
        });

        E2eHarness { inner: h, _tantivy_dir: None, keys, cleanup: Some(cleanup) }
    }

    /// The PostgreSQL twin of [`make_sqlite_bm25_harness`]: a real tsvector
    /// backend, no embedding endpoint.
    #[cfg(feature = "postgres")]
    pub async fn make_pg_bm25_harness(dsn: &str, schema: &str) -> E2eHarness {
        use crate::search::bm25_pg::PostgresBm25Backend;
        use crate::storage::rel::{PoolSettings, PostgresRelationalDb};
        use crate::tools::testkit::harness_with_search;
        use std::sync::Arc;

        let db = PostgresRelationalDb::connect(dsn, schema, PoolSettings::default())
            .await
            .expect("MCPFS_TEST_PG_DSN is set but connection failed");
        let pool = db.pool().clone();
        let backend = Arc::new(
            PostgresBm25Backend::new(pool.clone()).await.expect("bm25 backend must build"),
        ) as Arc<dyn crate::search::SearchBackend>;

        let keys = jwt_keys();
        let public_key_path = keys.public.clone();
        let h = harness_with_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "bm25".into();
                c.auth.jwt.public_key_path = public_key_path;
            },
            Some(backend),
        )
        .await;

        E2eHarness {
            inner: h,
            _tantivy_dir: None,
            keys,
            cleanup: Some(pg_schema_cleanup(pool, schema)),
        }
    }

    /// Drop the test schema when the harness goes, so runs stay isolated.
    ///
    /// A new OS thread with its own runtime, because `block_on` inside `Drop` on
    /// the ambient runtime panics with "Cannot start a runtime from within a
    /// Tokio runtime".
    #[cfg(feature = "postgres")]
    pub fn pg_schema_cleanup(pool: sqlx::PgPool, schema: &str) -> Box<dyn FnOnce() + Send> {
        let schema = schema.to_string();
        Box::new(move || {
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("cleanup runtime");
                rt.block_on(async move {
                    let sql = format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE");
                    let _ = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).execute(&pool).await;
                });
            })
            .join()
            .ok();
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Auto indexing scenarios, written once and run against every backend
// ─────────────────────────────────────────────────────────────────────────────

/// Each function here is one scenario from the per-project index mode plan. They
/// take a harness rather than building one, so the SQLite and PostgreSQL suites
/// below run the SAME assertions against their own backend instead of two
/// copies that drift apart.
///
/// `mode` is the index mode the scenario activates, which is also the query mode
/// the harness's backend can serve.
#[cfg(test)]
mod scenarios {
    use super::shared::{E2eHarness, MOUNT};
    use crate::tools::testkit::PERSON;
    use serde_json::json;

    /// Write a file with the mode active, then find it through `search.query`.
    pub async fn auto_index_write_then_query_finds_it(h: &E2eHarness, mode: &str) {
        let set = h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        assert_eq!(set["index_mode"], mode);
        assert_eq!(set["previous_mode"], "none");

        h.write_file(MOUNT, "/auto.md", "the quick brown fox jumps over the lazy dog").await;
        h.await_chunks(MOUNT, 1).await;

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "fox", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        let results = resp["results"].as_array().expect("results must be an array");
        assert!(!results.is_empty(), "the auto indexed file must be findable, got {resp}");
        assert_eq!(results[0]["path"], "/auto.md");
    }

    /// Deleting the file removes it from the index too.
    pub async fn auto_index_delete_then_query_is_empty(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/gone.md", "unique content about foxes").await;
        h.await_chunks(MOUNT, 1).await;

        h.call("fs.delete", json!({"mount_id": MOUNT, "path": "/gone.md"}))
            .await
            .expect("fs.delete must succeed");
        h.await_chunks(MOUNT, 0).await;

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "foxes", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        assert!(
            resp["results"].as_array().expect("results must be an array").is_empty(),
            "a deleted file must not be findable, got {resp}"
        );
    }

    /// Going back to none empties the index immediately.
    pub async fn set_mode_none_wipes_the_index(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/a.md", "alpha content").await;
        h.write_file(MOUNT, "/b.md", "beta content").await;
        h.await_chunks(MOUNT, 2).await;

        let off = h.set_mode(MOUNT, "none").await.expect("set_index_mode none must succeed");
        assert_eq!(off["index_mode"], "none");
        assert_eq!(off["previous_mode"], mode);
        assert_eq!(off["reindex_started"], false, "none must start no reindex");
        // The wipe is awaited by the tool, so this needs no polling at all.
        assert_eq!(h.chunk_count(MOUNT).await, 0, "the index must be empty right away");
    }

    /// The initial full index: files written while the mode was none become
    /// searchable when the mode is turned on, with no `search.index` call.
    pub async fn set_mode_active_backfills_existing_files(h: &E2eHarness, mode: &str) {
        for (path, text) in [
            ("/one.md", "alpha document about rust"),
            ("/two.md", "beta document about rust"),
            ("/sub/three.md", "gamma document about rust"),
        ] {
            h.write_file(MOUNT, path, text).await;
        }
        assert_eq!(h.chunk_count(MOUNT).await, 0, "mode none must have indexed nothing");

        let set = h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        assert_eq!(set["reindex_started"], true, "an active mode must start the backfill");

        h.await_chunks(MOUNT, 3).await;
        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "rust", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        let found: Vec<&str> = resp["results"]
            .as_array()
            .expect("results must be an array")
            .iter()
            .filter_map(|r| r["path"].as_str())
            .collect();
        for path in ["/one.md", "/two.md", "/sub/three.md"] {
            assert!(found.contains(&path), "{path} must be searchable, found {found:?}");
        }
    }

    /// Switching between two active modes wipes first and rebuilds after.
    pub async fn mode_change_wipes_and_rebuilds(h: &E2eHarness, from: &str, to: &str) {
        h.set_mode(MOUNT, from).await.expect("the first mode must be settable");
        h.write_file(MOUNT, "/a.md", "alpha content").await;
        h.write_file(MOUNT, "/b.md", "beta content").await;
        h.await_chunks(MOUNT, 2).await;

        let changed = h.set_mode(MOUNT, to).await.expect("the second mode must be settable");
        assert_eq!(changed["previous_mode"], from);
        assert_eq!(changed["index_mode"], to);
        assert_eq!(changed["reindex_started"], true);

        // The wipe already happened, so the count can only come back up through
        // the rebuild, which is what makes the final 2 meaningful.
        h.await_chunks(MOUNT, 2).await;
    }

    /// Setting the mode a project already has must keep the index it has.
    pub async fn same_mode_twice_is_a_noop(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/a.md", "alpha content").await;
        h.await_chunks(MOUNT, 1).await;

        let again = h.set_mode(MOUNT, mode).await.expect("the second set must succeed");
        assert_eq!(again["previous_mode"], mode);
        assert_eq!(again["reindex_started"], false, "a no-op must not start a reindex");
        assert_eq!(h.chunk_count(MOUNT).await, 1, "a no-op must not wipe the index");
    }

    /// With mode none, writes reach the volume and nothing else.
    pub async fn mode_none_does_not_index_writes(h: &E2eHarness, mode: &str) {
        let read = h
            .call("admin.get_index_mode", json!({"project_id": MOUNT}))
            .await
            .expect("admin.get_index_mode must succeed");
        assert_eq!(read["index_mode"], "none", "a fresh project starts at none");

        h.write_file(MOUNT, "/a.md", "content that must not be indexed").await;
        h.write_file(MOUNT, "/b.md", "more content that must not be indexed").await;

        // Nothing to wait for, so give a hook that should not exist the time to
        // fire before concluding that it did not.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 0, "mode none must index nothing");

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "content", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        assert!(resp["results"].as_array().expect("results must be an array").is_empty());
    }

    /// Two projects on one backend: an active mode on one must not index, wipe or
    /// surface anything belonging to the other.
    pub async fn volume_isolation_under_auto_index(h: &E2eHarness, mode: &str) {
        const OTHER: &str = "other-proj";
        h.add_project(OTHER).await;

        h.set_mode(MOUNT, mode).await.expect("the first project must take the mode");
        h.write_file(MOUNT, "/shared.md", "alpha specific content").await;
        h.write_file(OTHER, "/shared.md", "beta specific content").await;
        h.await_chunks(MOUNT, 1).await;

        assert_eq!(
            h.chunk_count(OTHER).await,
            0,
            "a project left at none must index nothing while its neighbour indexes"
        );
        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "specific", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        let results = resp["results"].as_array().expect("results must be an array");
        assert_eq!(results.len(), 1, "only this volume's chunk may match, got {resp}");
        assert!(
            results[0]["chunk"].as_str().unwrap_or_default().contains("alpha"),
            "the other volume's content leaked: {resp}"
        );

        // Now the other project indexes too, and the first one is untouched.
        h.set_mode(OTHER, mode).await.expect("the second project must take the mode");
        h.await_chunks(OTHER, 1).await;
        assert_eq!(h.chunk_count(MOUNT).await, 1, "the neighbour's backfill must not wipe us");

        // And turning the other one off must not empty this one.
        h.set_mode(OTHER, "none").await.expect("the second project must go back to none");
        assert_eq!(h.chunk_count(OTHER).await, 0);
        assert_eq!(h.chunk_count(MOUNT).await, 1, "a neighbour's wipe must stay in its volume");
    }

    /// A vector mode with no embedding endpoint would index nothing and report
    /// success, so it is refused.
    pub async fn set_index_mode_rejects_rag_without_embedding_endpoint(h: &E2eHarness) {
        for mode in ["rag", "both"] {
            let err = h.set_mode(MOUNT, mode).await.expect_err("a vector mode must be refused");
            assert_eq!(
                err.code,
                crate::errors::code::INVALID_ARGUMENT,
                "expected ERR_INVALID_ARGUMENT for {mode}, got {err}"
            );
            assert!(err.message.contains("search.embedding.endpoint"), "{}", err.message);
        }
        let read = h
            .call("admin.get_index_mode", json!({"project_id": MOUNT}))
            .await
            .expect("admin.get_index_mode must succeed");
        assert_eq!(read["index_mode"], "none", "a refused call must change nothing");
    }

    /// An unknown mode is the caller's mistake, not a server error.
    pub async fn set_index_mode_rejects_an_unknown_mode(h: &E2eHarness) {
        let err = h.set_mode(MOUNT, "fancy").await.expect_err("an unknown mode must be refused");
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT, "got {err}");
    }

    /// A plain member may read the mode but never set it.
    pub async fn non_owner_cannot_set_index_mode(h: &E2eHarness, mode: &str) {
        h.inner
            .state
            .admin
            .add_member(MOUNT, "member@x.y", crate::tools::testkit::PERSON)
            .await
            .expect("the member must be addable");

        let ctx = crate::mcp::registry::ToolCtx {
            person: "member@x.y".to_string(),
            state: h.inner.state.clone(),
        };
        let err = h
            .inner
            .state
            .registry
            .call(
                "admin.set_index_mode",
                ctx.clone(),
                crate::mcp::Args::new(json!({"project_id": MOUNT, "mode": mode})),
            )
            .await
            .expect("the tool must be registered")
            .expect_err("a plain member must not set the mode");
        assert_eq!(err.code, crate::errors::code::FORBIDDEN, "got {err}");

        // Reading is allowed, which is what makes the refusal a gate rather than
        // an accident of the project being invisible to them.
        let read = h
            .inner
            .state
            .registry
            .call("admin.get_index_mode", ctx, crate::mcp::Args::new(json!({"project_id": MOUNT})))
            .await
            .expect("the tool must be registered")
            .expect("a member may read the mode");
        assert_eq!(read["index_mode"], "none", "the refused call changed nothing");
    }

    /// A stranger cannot even read it.
    pub async fn non_member_cannot_read_index_mode(h: &E2eHarness) {
        let ctx = crate::mcp::registry::ToolCtx {
            person: "stranger@x.y".to_string(),
            state: h.inner.state.clone(),
        };
        let err = h
            .inner
            .state
            .registry
            .call("admin.get_index_mode", ctx, crate::mcp::Args::new(json!({"project_id": MOUNT})))
            .await
            .expect("the tool must be registered")
            .expect_err("a non member must be forbidden");
        assert_eq!(err.code, crate::errors::code::FORBIDDEN, "got {err}");
    }

    /// Every write surface feeds the index, not just `fs.write`.
    pub async fn every_write_tool_feeds_the_index(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");

        h.write_file(MOUNT, "/w.md", "written through fs write").await;
        h.await_chunks(MOUNT, 1).await;

        h.call(
            "fs.append",
            json!({"mount_id": MOUNT, "path": "/w.md", "content": "\nappended marker zebra"}),
        )
        .await
        .expect("fs.append must succeed");
        await_query_hit(h, mode, "zebra", "/w.md").await;

        h.call(
            "fs.edit",
            json!({"mount_id": MOUNT, "path": "/w.md",
                   "old_string": "zebra", "new_string": "quokka"}),
        )
        .await
        .expect("fs.edit must succeed");
        await_query_hit(h, mode, "quokka", "/w.md").await;

        h.call(
            "fs.apply_patch",
            json!({"mount_id": MOUNT, "patch_text":
                "*** Begin Patch\n*** Add File: /patched.md\n+patched marker narwhal\n*** End Patch"}),
        )
        .await
        .expect("fs.apply_patch must succeed");
        await_query_hit(h, mode, "narwhal", "/patched.md").await;
    }

    /// A `dry_run` edit writes nothing, so it must index nothing.
    pub async fn a_dry_run_edit_does_not_index(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/d.md", "original marker walrus").await;
        h.await_chunks(MOUNT, 1).await;

        h.call(
            "fs.edit",
            json!({"mount_id": MOUNT, "path": "/d.md", "old_string": "walrus",
                   "new_string": "manatee", "dry_run": true}),
        )
        .await
        .expect("a dry run edit must succeed");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 1, "a dry run must not add a chunk");

        // The stored chunk must still be the original text. Asserting on an empty
        // result set would prove nothing here: the fake embedding server answers
        // every input with the same vector, so a RAG query matches regardless of
        // the words in it.
        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "walrus", "mode": mode, "top_k": 10}),
            )
            .await
            .expect("search.query must succeed");
        let chunk = resp["results"][0]["chunk"].as_str().unwrap_or_default().to_string();
        assert!(chunk.contains("walrus"), "the indexed text must be the original, got {resp}");
        assert!(!chunk.contains("manatee"), "a dry run must not reach the index, got {resp}");
    }

    /// A recursive delete must clear every file it removed, not just the root.
    pub async fn recursive_delete_clears_the_whole_subtree(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/tree/a.md", "alpha in the tree").await;
        h.write_file(MOUNT, "/tree/deep/b.md", "beta in the tree").await;
        h.await_chunks(MOUNT, 2).await;

        h.call("fs.delete", json!({"mount_id": MOUNT, "path": "/tree", "recursive": true}))
            .await
            .expect("a recursive delete must succeed");
        h.await_chunks(MOUNT, 0).await;
    }

    /// A move takes the index entry with it: the old path stops matching, the new
    /// one starts, and nothing is added or lost on the way.
    pub async fn move_a_file_moves_its_index_entry(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/from.md", "content about wandering albatross").await;
        h.await_chunks(MOUNT, 1).await;

        h.call(
            "fs.move",
            json!({"mount_id": MOUNT, "source": "/from.md", "destination": "/to.md"}),
        )
        .await
        .expect("fs.move must succeed");

        await_query_hit(h, mode, "albatross", "/to.md").await;
        await_no_query_hit(h, mode, "albatross", "/from.md").await;
        h.await_chunks(MOUNT, 1).await;
    }

    /// A copy leaves the source indexed and indexes the destination too.
    pub async fn copy_a_file_indexes_the_destination(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/orig.md", "content about spotted salamander").await;
        h.await_chunks(MOUNT, 1).await;

        h.call(
            "fs.copy",
            json!({"mount_id": MOUNT, "source": "/orig.md", "destination": "/dup.md"}),
        )
        .await
        .expect("fs.copy must succeed");

        h.await_chunks(MOUNT, 2).await;
        await_query_hit(h, mode, "salamander", "/dup.md").await;
        await_query_hit(h, mode, "salamander", "/orig.md").await;
    }

    /// A tree move must follow every file of the subtree, at every depth, not
    /// just the named root.
    pub async fn move_a_tree_moves_every_entry(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        for (path, text) in [
            ("/src/a.md", "alpha about pangolin"),
            ("/src/b.md", "beta about pangolin"),
            ("/src/deep/c.md", "gamma about pangolin"),
        ] {
            h.write_file(MOUNT, path, text).await;
        }
        h.await_chunks(MOUNT, 3).await;

        h.call("fs.move", json!({"mount_id": MOUNT, "source": "/src", "destination": "/dst"}))
            .await
            .expect("a tree move must succeed");

        for path in ["/dst/a.md", "/dst/b.md", "/dst/deep/c.md"] {
            await_query_hit(h, mode, "pangolin", path).await;
        }
        for path in ["/src/a.md", "/src/b.md", "/src/deep/c.md"] {
            await_no_query_hit(h, mode, "pangolin", path).await;
        }
        h.await_chunks(MOUNT, 3).await;
    }

    /// `overwrite: true` onto an existing tree DESTROYS that tree: the engine
    /// deletes the destination before the rename. A file that lived only under
    /// the old destination is therefore gone from the volume, and must be gone
    /// from the index too instead of staying searchable at a dead path.
    ///
    /// The content assertions matter as much as the counts: a file present on
    /// both sides is cleared AND re-indexed by the same hook, so a chunk still
    /// carrying the destination's old text, or no chunk at all, is the
    /// clear-after-reindex race rather than a stale read.
    pub async fn move_onto_an_existing_tree_strands_nothing(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        for (path, text) in [
            ("/d/a.md", "destination alpha about okapi"),
            ("/d/b.md", "destination beta about okapi"),
            ("/d/c.md", "destination gamma about okapi"),
            ("/s/a.md", "source alpha about okapi"),
            ("/s/b.md", "source beta about okapi"),
        ] {
            h.write_file(MOUNT, path, text).await;
        }
        h.await_chunks(MOUNT, 5).await;

        h.call(
            "fs.move",
            json!({"mount_id": MOUNT, "source": "/s", "destination": "/d", "overwrite": true}),
        )
        .await
        .expect("an overwriting tree move must succeed");

        // The file that existed only under the old destination went with it.
        await_no_query_hit(h, mode, "okapi", "/d/c.md").await;
        for path in ["/d/a.md", "/d/b.md"] {
            await_chunk_contains(h, mode, "okapi", path, "source").await;
        }
        h.await_chunks(MOUNT, 2).await;
    }

    /// The single file case of the same rule: the destination's entry must end
    /// up carrying the source's text, with nothing added and nothing lost.
    pub async fn move_a_file_onto_an_existing_file_reindexes_it(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/dst.md", "destination text about tapir").await;
        h.write_file(MOUNT, "/src.md", "source text about tapir").await;
        h.await_chunks(MOUNT, 2).await;

        h.call(
            "fs.move",
            json!({"mount_id": MOUNT, "source": "/src.md", "destination": "/dst.md",
                   "overwrite": true}),
        )
        .await
        .expect("an overwriting file move must succeed");

        await_no_query_hit(h, mode, "tapir", "/src.md").await;
        await_chunk_contains(h, mode, "tapir", "/dst.md", "source").await;
        h.await_chunks(MOUNT, 1).await;
    }

    /// A recursive copy must index every destination file and keep every source one.
    pub async fn copy_a_tree_indexes_every_destination(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        for (path, text) in [
            ("/src/a.md", "alpha about axolotl"),
            ("/src/b.md", "beta about axolotl"),
            ("/src/deep/c.md", "gamma about axolotl"),
        ] {
            h.write_file(MOUNT, path, text).await;
        }
        h.await_chunks(MOUNT, 3).await;

        h.call(
            "fs.copy",
            json!({"mount_id": MOUNT, "source": "/src", "destination": "/dst", "recursive": true}),
        )
        .await
        .expect("a tree copy must succeed");

        h.await_chunks(MOUNT, 6).await;
        for path in
            ["/src/a.md", "/src/b.md", "/src/deep/c.md", "/dst/a.md", "/dst/b.md", "/dst/deep/c.md"]
        {
            await_query_hit(h, mode, "axolotl", path).await;
        }
    }

    /// Mode none means the move hook reads the project row and stops there.
    pub async fn move_under_mode_none_indexes_nothing(h: &E2eHarness) {
        h.write_file(MOUNT, "/from.md", "content that must not be indexed").await;
        h.call(
            "fs.move",
            json!({"mount_id": MOUNT, "source": "/from.md", "destination": "/to.md"}),
        )
        .await
        .expect("fs.move must succeed with the mode off");

        // Nothing to wait for, so give a hook that should not exist the time to
        // fire before concluding that it did not.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 0, "mode none must index nothing on a move");
    }

    /// A refused move must not touch the index: the volume did not change, so
    /// neither may the entries describing it.
    pub async fn a_failed_move_leaves_the_index_alone(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/from.md", "content about numbat").await;
        h.write_file(MOUNT, "/blocker.md", "more content about numbat").await;
        h.await_chunks(MOUNT, 2).await;

        let err = h
            .call(
                "fs.move",
                json!({"mount_id": MOUNT, "source": "/from.md", "destination": "/blocker.md"}),
            )
            .await
            .expect_err("a no clobber move onto an existing path must fail");
        assert_eq!(err.code, crate::errors::code::NO_CLOBBER, "got {err}");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 2, "a failed move must add or drop nothing");
        await_query_hit(h, mode, "numbat", "/from.md").await;
        await_query_hit(h, mode, "numbat", "/blocker.md").await;
    }

    /// A binary write has no text to index, and must not fail the write either.
    pub async fn a_binary_write_is_skipped(h: &E2eHarness, mode: &str) {
        use base64::Engine as _;
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        let payload = base64::engine::general_purpose::STANDARD.encode([0xFFu8, 0xFE, 0x00, 0x01]);
        h.call(
            "fs.write_bytes",
            json!({"mount_id": MOUNT, "path": "/blob.bin", "base64": payload}),
        )
        .await
        .expect("fs.write_bytes must succeed with an active index mode");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 0, "a non utf-8 file must be skipped");
    }

    // ── the same hooks, driven through the REST data plane ───────────────────
    //
    // The REST plane is a second door onto the same engine. These run the HTTP
    // handlers against the harness's own `AppState`, so a hook that only exists
    // on the MCP side shows up here as a failure rather than as silence.

    /// A REST delete clears the index entry, exactly as `fs.delete` does.
    pub async fn rest_delete_clears_the_index(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/gone.md", "content about the greater bilby").await;
        h.await_chunks(MOUNT, 1).await;

        let (status, body) =
            h.rest_post(MOUNT, "delete", json!({"path": "/gone.md"}), PERSON).await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST delete failed: {body}");

        await_no_query_hit(h, mode, "bilby", "/gone.md").await;
        h.await_chunks(MOUNT, 0).await;
    }

    /// A recursive REST delete must clear every file it removed, at every depth.
    pub async fn rest_recursive_delete_clears_the_whole_subtree(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/tree/a.md", "alpha about the quoll").await;
        h.write_file(MOUNT, "/tree/deep/b.md", "beta about the quoll").await;
        h.await_chunks(MOUNT, 2).await;

        let (status, body) =
            h.rest_post(MOUNT, "delete", json!({"path": "/tree", "recursive": true}), PERSON).await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST recursive delete failed: {body}");

        for path in ["/tree/a.md", "/tree/deep/b.md"] {
            await_no_query_hit(h, mode, "quoll", path).await;
        }
        h.await_chunks(MOUNT, 0).await;
    }

    /// A REST move takes the index entry with it, adding and losing nothing.
    pub async fn rest_move_moves_the_index_entry(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/from.md", "content about the kakapo").await;
        h.await_chunks(MOUNT, 1).await;

        let (status, body) = h
            .rest_post(
                MOUNT,
                "move",
                json!({"source": "/from.md", "destination": "/to.md"}),
                PERSON,
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST move failed: {body}");

        await_query_hit(h, mode, "kakapo", "/to.md").await;
        await_no_query_hit(h, mode, "kakapo", "/from.md").await;
        h.await_chunks(MOUNT, 1).await;
    }

    /// A REST copy leaves the source indexed and indexes the destination too.
    pub async fn rest_copy_indexes_the_destination(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/orig.md", "content about the saiga antelope").await;
        h.await_chunks(MOUNT, 1).await;

        let (status, body) = h
            .rest_post(
                MOUNT,
                "copy",
                json!({"source": "/orig.md", "destination": "/dup.md"}),
                PERSON,
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST copy failed: {body}");

        h.await_chunks(MOUNT, 2).await;
        await_query_hit(h, mode, "saiga", "/dup.md").await;
        await_query_hit(h, mode, "saiga", "/orig.md").await;
    }

    /// An upload is a write, so it must become searchable like one.
    pub async fn rest_upload_indexes_the_file(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");

        let (status, body) =
            h.rest_upload(MOUNT, "/", "up.md", "content about the capybara", PERSON).await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST upload failed: {body}");
        assert_eq!(body["count"], 1, "the upload must have written one file, got {body}");

        h.await_chunks(MOUNT, 1).await;
        await_query_hit(h, mode, "capybara", "/up.md").await;
    }

    /// Mode none means the REST delete hook reads the project row and stops.
    pub async fn rest_delete_under_mode_none_indexes_nothing(h: &E2eHarness) {
        h.write_file(MOUNT, "/gone.md", "content that must not be indexed").await;
        let (status, body) =
            h.rest_post(MOUNT, "delete", json!({"path": "/gone.md"}), PERSON).await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST delete failed: {body}");

        // Nothing to wait for, so give a hook that should not exist the time to
        // fire before concluding that it did not.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 0, "mode none must index nothing on a delete");
    }

    /// A refused REST delete must not touch the index: the volume did not change,
    /// so neither may the entries describing it.
    pub async fn a_failed_rest_delete_leaves_the_index_alone(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        h.write_file(MOUNT, "/tree/a.md", "alpha about the vaquita").await;
        h.write_file(MOUNT, "/tree/deep/b.md", "beta about the vaquita").await;
        h.await_chunks(MOUNT, 2).await;

        let (status, body) = h.rest_post(MOUNT, "delete", json!({"path": "/tree"}), PERSON).await;
        assert_ne!(
            status,
            axum::http::StatusCode::OK,
            "deleting a non empty directory without recursive must fail, got {body}"
        );

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(h.chunk_count(MOUNT).await, 2, "a failed delete must drop nothing");
        for path in ["/tree/a.md", "/tree/deep/b.md"] {
            await_query_hit(h, mode, "vaquita", path).await;
        }
    }

    /// The REST door onto the same rule: an overwriting tree move through
    /// `POST /api/fs/{mount}/move` must strand nothing either.
    pub async fn rest_move_onto_an_existing_tree_strands_nothing(h: &E2eHarness, mode: &str) {
        h.set_mode(MOUNT, mode).await.expect("set_index_mode must succeed");
        for (path, text) in [
            ("/d/a.md", "destination alpha about markhor"),
            ("/d/b.md", "destination beta about markhor"),
            ("/d/c.md", "destination gamma about markhor"),
            ("/s/a.md", "source alpha about markhor"),
            ("/s/b.md", "source beta about markhor"),
        ] {
            h.write_file(MOUNT, path, text).await;
        }
        h.await_chunks(MOUNT, 5).await;

        let (status, body) = h
            .rest_post(
                MOUNT,
                "move",
                json!({"source": "/s", "destination": "/d", "overwrite": true}),
                PERSON,
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "REST overwriting move failed: {body}");

        await_no_query_hit(h, mode, "markhor", "/d/c.md").await;
        for path in ["/d/a.md", "/d/b.md"] {
            await_chunk_contains(h, mode, "markhor", path, "source").await;
        }
        h.await_chunks(MOUNT, 2).await;
    }

    /// Poll until `path`'s indexed chunk contains `needle`, or fail.
    ///
    /// Stronger than [`await_query_hit`]: it proves WHICH text is indexed, the
    /// only way to tell a real re-index from a leftover entry when the path is
    /// the same on both sides of a move.
    async fn await_chunk_contains(
        h: &E2eHarness,
        mode: &str,
        query: &str,
        path: &str,
        needle: &str,
    ) {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(super::shared::AUTO_INDEX_TIMEOUT_SECS);
        let mut last = serde_json::Value::Null;
        while std::time::Instant::now() < deadline {
            last = h
                .call(
                    "search.query",
                    json!({"mount_id": MOUNT, "query": query, "mode": mode, "top_k": 50}),
                )
                .await
                .expect("search.query must succeed");
            let hit = last["results"]
                .as_array()
                .and_then(|r| r.iter().find(|x| x["path"] == path))
                .and_then(|x| x["chunk"].as_str());
            if hit.is_some_and(|c| c.contains(needle)) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {path} to be indexed with '{needle}', last: {last}");
    }

    /// Poll until `query` finds `path`, or fail: the index lags the write.
    async fn await_query_hit(h: &E2eHarness, mode: &str, query: &str, path: &str) {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(super::shared::AUTO_INDEX_TIMEOUT_SECS);
        let mut last = serde_json::Value::Null;
        while std::time::Instant::now() < deadline {
            last = h
                .call(
                    "search.query",
                    json!({"mount_id": MOUNT, "query": query, "mode": mode, "top_k": 10}),
                )
                .await
                .expect("search.query must succeed");
            if last["results"].as_array().is_some_and(|r| r.iter().any(|x| x["path"] == path)) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for '{query}' to find {path}, last results: {last}");
    }

    /// The negative twin of `await_query_hit`: poll until `path` has left the
    /// results, or fail. The removal is fire and forget like the write, so a
    /// single immediate query would be racy in the passing direction.
    async fn await_no_query_hit(h: &E2eHarness, mode: &str, query: &str, path: &str) {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(super::shared::AUTO_INDEX_TIMEOUT_SECS);
        let mut last = serde_json::Value::Null;
        while std::time::Instant::now() < deadline {
            last = h
                .call(
                    "search.query",
                    json!({"mount_id": MOUNT, "query": query, "mode": mode, "top_k": 50}),
                )
                .await
                .expect("search.query must succeed");
            if !last["results"].as_array().is_some_and(|r| r.iter().any(|x| x["path"] == path)) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for '{query}' to stop finding {path}, last results: {last}");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SQLite-RAG e2e tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "rag")]
mod sqlite_rag {
    use super::scenarios;
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

        assert!(resp["indexed"].as_u64().unwrap_or(0) > 0, "indexed must be > 0, got {resp}");
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
        assert!(results[0]["score"].as_f64().unwrap_or(0.0) > 0.0, "score must be positive");
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
        assert!(del["deleted"].as_u64().unwrap_or(0) > 0, "deleted must be > 0, got {del}");

        let resp = h
            .call(
                "search.query",
                json!({"mount_id": MOUNT, "query": "fox", "mode": "rag", "top_k": 10}),
            )
            .await
            .expect("query after delete must succeed");
        let results = resp["results"].as_array().expect("results must be array");
        assert!(results.is_empty(), "results must be empty after delete, got {resp}");

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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
            .call("search.index", json!({"mount_id": MOUNT, "path": "/docs", "recursive": true}))
            .await
            .expect("recursive index must succeed");
        assert!(resp["indexed"].as_u64().unwrap_or(0) >= 2, "indexed must be >= 2, got {resp}");

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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
        assert!(results.is_empty(), "results must be empty on an unindexed volume, got {resp}");
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
        assert!(results.len() <= 2, "results.len() must be <= top_k=2, got {}", results.len());
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

        assert_eq!(err.code, crate::errors::code::FORBIDDEN, "expected ERR_FORBIDDEN, got {err}");
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

    // ── per-project index mode, auto indexing ────────────────────────────────

    /// One harness per scenario: a mode change wipes the whole volume index, so
    /// sharing one would let scenarios overwrite each other's state.
    async fn h() -> super::shared::E2eHarness {
        let base = spawn_fake_embedding(3).await;
        make_sqlite_rag_harness(&base, 3).await
    }

    #[tokio::test]
    async fn e2e_sqlite_auto_index_write_then_query_finds_it() {
        scenarios::auto_index_write_then_query_finds_it(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_auto_index_delete_then_query_is_empty() {
        scenarios::auto_index_delete_then_query_is_empty(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_set_mode_none_wipes_the_index() {
        scenarios::set_mode_none_wipes_the_index(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_set_mode_active_backfills_existing_files() {
        scenarios::set_mode_active_backfills_existing_files(&h().await, "rag").await;
    }

    /// bm25 -> rag on a RAG backend: the point is the wipe and the rebuild, which
    /// the transition table requires of every change between two active modes.
    #[tokio::test]
    async fn e2e_sqlite_mode_change_bm25_to_rag_wipes_and_rebuilds() {
        scenarios::mode_change_wipes_and_rebuilds(&h().await, "bm25", "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_same_mode_twice_is_a_noop() {
        scenarios::same_mode_twice_is_a_noop(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_mode_none_does_not_index_writes() {
        scenarios::mode_none_does_not_index_writes(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_volume_isolation_under_auto_index() {
        scenarios::volume_isolation_under_auto_index(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_non_owner_cannot_set_index_mode() {
        scenarios::non_owner_cannot_set_index_mode(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_non_member_cannot_read_index_mode() {
        scenarios::non_member_cannot_read_index_mode(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_set_index_mode_rejects_an_unknown_mode() {
        scenarios::set_index_mode_rejects_an_unknown_mode(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_every_write_tool_feeds_the_index() {
        scenarios::every_write_tool_feeds_the_index(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_a_dry_run_edit_does_not_index() {
        scenarios::a_dry_run_edit_does_not_index(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_recursive_delete_clears_the_whole_subtree() {
        scenarios::recursive_delete_clears_the_whole_subtree(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_a_binary_write_is_skipped() {
        scenarios::a_binary_write_is_skipped(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_move_a_file_moves_its_index_entry() {
        scenarios::move_a_file_moves_its_index_entry(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_copy_a_file_indexes_the_destination() {
        scenarios::copy_a_file_indexes_the_destination(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_move_a_tree_moves_every_entry() {
        scenarios::move_a_tree_moves_every_entry(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_move_onto_an_existing_tree_strands_nothing() {
        scenarios::move_onto_an_existing_tree_strands_nothing(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_move_a_file_onto_an_existing_file_reindexes_it() {
        scenarios::move_a_file_onto_an_existing_file_reindexes_it(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_move_onto_an_existing_tree_strands_nothing() {
        scenarios::rest_move_onto_an_existing_tree_strands_nothing(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_copy_a_tree_indexes_every_destination() {
        scenarios::copy_a_tree_indexes_every_destination(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_move_under_mode_none_indexes_nothing() {
        scenarios::move_under_mode_none_indexes_nothing(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_a_failed_move_leaves_the_index_alone() {
        scenarios::a_failed_move_leaves_the_index_alone(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_delete_clears_the_index() {
        scenarios::rest_delete_clears_the_index(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_recursive_delete_clears_the_whole_subtree() {
        scenarios::rest_recursive_delete_clears_the_whole_subtree(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_move_moves_the_index_entry() {
        scenarios::rest_move_moves_the_index_entry(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_copy_indexes_the_destination() {
        scenarios::rest_copy_indexes_the_destination(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_upload_indexes_the_file() {
        scenarios::rest_upload_indexes_the_file(&h().await, "rag").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_rest_delete_under_mode_none_indexes_nothing() {
        scenarios::rest_delete_under_mode_none_indexes_nothing(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_a_failed_rest_delete_leaves_the_index_alone() {
        scenarios::a_failed_rest_delete_leaves_the_index_alone(&h().await, "rag").await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SQLite-BM25 e2e tests: the same scenarios against Tantivy, with no embedding
// endpoint configured, which is also what makes the rag rejection case real.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod sqlite_bm25 {
    use super::scenarios;
    use super::shared::make_sqlite_bm25_harness as h;

    #[tokio::test]
    async fn e2e_sqlite_bm25_auto_index_write_then_query_finds_it() {
        scenarios::auto_index_write_then_query_finds_it(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_auto_index_delete_then_query_is_empty() {
        scenarios::auto_index_delete_then_query_is_empty(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_set_mode_none_wipes_the_index() {
        scenarios::set_mode_none_wipes_the_index(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_set_mode_active_backfills_existing_files() {
        scenarios::set_mode_active_backfills_existing_files(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_same_mode_twice_is_a_noop() {
        scenarios::same_mode_twice_is_a_noop(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_mode_none_does_not_index_writes() {
        scenarios::mode_none_does_not_index_writes(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_volume_isolation_under_auto_index() {
        scenarios::volume_isolation_under_auto_index(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_set_index_mode_rejects_rag_without_embedding_endpoint() {
        scenarios::set_index_mode_rejects_rag_without_embedding_endpoint(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_non_owner_cannot_set_index_mode() {
        scenarios::non_owner_cannot_set_index_mode(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_recursive_delete_clears_the_whole_subtree() {
        scenarios::recursive_delete_clears_the_whole_subtree(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_move_a_file_moves_its_index_entry() {
        scenarios::move_a_file_moves_its_index_entry(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_copy_a_file_indexes_the_destination() {
        scenarios::copy_a_file_indexes_the_destination(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_move_a_tree_moves_every_entry() {
        scenarios::move_a_tree_moves_every_entry(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_move_onto_an_existing_tree_strands_nothing() {
        scenarios::move_onto_an_existing_tree_strands_nothing(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_move_a_file_onto_an_existing_file_reindexes_it() {
        scenarios::move_a_file_onto_an_existing_file_reindexes_it(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_move_onto_an_existing_tree_strands_nothing() {
        scenarios::rest_move_onto_an_existing_tree_strands_nothing(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_copy_a_tree_indexes_every_destination() {
        scenarios::copy_a_tree_indexes_every_destination(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_move_under_mode_none_indexes_nothing() {
        scenarios::move_under_mode_none_indexes_nothing(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_a_failed_move_leaves_the_index_alone() {
        scenarios::a_failed_move_leaves_the_index_alone(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_delete_clears_the_index() {
        scenarios::rest_delete_clears_the_index(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_recursive_delete_clears_the_whole_subtree() {
        scenarios::rest_recursive_delete_clears_the_whole_subtree(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_move_moves_the_index_entry() {
        scenarios::rest_move_moves_the_index_entry(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_copy_indexes_the_destination() {
        scenarios::rest_copy_indexes_the_destination(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_upload_indexes_the_file() {
        scenarios::rest_upload_indexes_the_file(&h().await, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_rest_delete_under_mode_none_indexes_nothing() {
        scenarios::rest_delete_under_mode_none_indexes_nothing(&h().await).await;
    }

    #[tokio::test]
    async fn e2e_sqlite_bm25_a_failed_rest_delete_leaves_the_index_alone() {
        scenarios::a_failed_rest_delete_leaves_the_index_alone(&h().await, "bm25").await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PostgreSQL-RAG e2e tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "rag")]
mod pg_rag {
    use super::scenarios;
    use super::shared::{MOUNT, make_pg_rag_harness, spawn_fake_embedding};
    use serde_json::json;

    /// Read `MCPFS_TEST_PG_DSN`. Returns `None` when unset (skip the test).
    pub fn pg_dsn() -> Option<String> {
        std::env::var("MCPFS_TEST_PG_DSN").ok().filter(|v| !v.trim().is_empty())
    }

    /// Generate a unique schema name so parallel tests don't collide.
    pub fn unique_schema() -> String {
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

        assert!(resp["indexed"].as_u64().unwrap_or(0) > 0, "indexed must be > 0, got {resp}");
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

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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
            .call("search.index", json!({"mount_id": MOUNT, "path": "/docs", "recursive": true}))
            .await
            .expect("recursive index must succeed");
        assert!(resp["indexed"].as_u64().unwrap_or(0) >= 2, "indexed must be >= 2, got {resp}");

        let status =
            h.call("search.status", json!({"mount_id": MOUNT})).await.expect("status must succeed");
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
        assert!(results.is_empty(), "results must be empty on an unindexed volume, got {resp}");
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
        assert!(results.len() <= 2, "results.len() must be <= top_k=2, got {}", results.len());
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

        assert_eq!(err.code, crate::errors::code::FORBIDDEN, "expected ERR_FORBIDDEN, got {err}");
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
        let sa = ha.call("search.status", json!({"mount_id": MOUNT})).await.expect("status in A");
        let sb = hb.call("search.status", json!({"mount_id": MOUNT})).await.expect("status in B");

        assert_eq!(sa["vector_chunks"], 1, "A must have exactly 1 chunk, got {sa}");
        assert_eq!(sb["vector_chunks"], 1, "B must have exactly 1 chunk, got {sb}");
    }

    // ── per-project index mode, auto indexing ────────────────────────────────

    /// One harness (and one schema) per scenario: a mode change wipes the whole
    /// volume index, so sharing one would let scenarios overwrite each other.
    /// Returns `None` when `MCPFS_TEST_PG_DSN` is unset, which skips the test.
    async fn h() -> Option<super::shared::E2eHarness> {
        let dsn = pg_dsn()?;
        let base = spawn_fake_embedding(3).await;
        Some(make_pg_rag_harness(&dsn, &unique_schema(), &base, 3).await)
    }

    #[tokio::test]
    async fn e2e_pg_auto_index_write_then_query_finds_it() {
        let Some(h) = h().await else { return };
        scenarios::auto_index_write_then_query_finds_it(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_auto_index_delete_then_query_is_empty() {
        let Some(h) = h().await else { return };
        scenarios::auto_index_delete_then_query_is_empty(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_set_mode_none_wipes_the_index() {
        let Some(h) = h().await else { return };
        scenarios::set_mode_none_wipes_the_index(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_set_mode_active_backfills_existing_files() {
        let Some(h) = h().await else { return };
        scenarios::set_mode_active_backfills_existing_files(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_mode_change_bm25_to_rag_wipes_and_rebuilds() {
        let Some(h) = h().await else { return };
        scenarios::mode_change_wipes_and_rebuilds(&h, "bm25", "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_same_mode_twice_is_a_noop() {
        let Some(h) = h().await else { return };
        scenarios::same_mode_twice_is_a_noop(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_mode_none_does_not_index_writes() {
        let Some(h) = h().await else { return };
        scenarios::mode_none_does_not_index_writes(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_volume_isolation_under_auto_index() {
        let Some(h) = h().await else { return };
        scenarios::volume_isolation_under_auto_index(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_non_owner_cannot_set_index_mode() {
        let Some(h) = h().await else { return };
        scenarios::non_owner_cannot_set_index_mode(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_non_member_cannot_read_index_mode() {
        let Some(h) = h().await else { return };
        scenarios::non_member_cannot_read_index_mode(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_set_index_mode_rejects_an_unknown_mode() {
        let Some(h) = h().await else { return };
        scenarios::set_index_mode_rejects_an_unknown_mode(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_every_write_tool_feeds_the_index() {
        let Some(h) = h().await else { return };
        scenarios::every_write_tool_feeds_the_index(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_a_dry_run_edit_does_not_index() {
        let Some(h) = h().await else { return };
        scenarios::a_dry_run_edit_does_not_index(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_recursive_delete_clears_the_whole_subtree() {
        let Some(h) = h().await else { return };
        scenarios::recursive_delete_clears_the_whole_subtree(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_a_binary_write_is_skipped() {
        let Some(h) = h().await else { return };
        scenarios::a_binary_write_is_skipped(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_move_a_file_moves_its_index_entry() {
        let Some(h) = h().await else { return };
        scenarios::move_a_file_moves_its_index_entry(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_copy_a_file_indexes_the_destination() {
        let Some(h) = h().await else { return };
        scenarios::copy_a_file_indexes_the_destination(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_move_a_tree_moves_every_entry() {
        let Some(h) = h().await else { return };
        scenarios::move_a_tree_moves_every_entry(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_move_onto_an_existing_tree_strands_nothing() {
        let Some(h) = h().await else { return };
        scenarios::move_onto_an_existing_tree_strands_nothing(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_move_a_file_onto_an_existing_file_reindexes_it() {
        let Some(h) = h().await else { return };
        scenarios::move_a_file_onto_an_existing_file_reindexes_it(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_move_onto_an_existing_tree_strands_nothing() {
        let Some(h) = h().await else { return };
        scenarios::rest_move_onto_an_existing_tree_strands_nothing(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_copy_a_tree_indexes_every_destination() {
        let Some(h) = h().await else { return };
        scenarios::copy_a_tree_indexes_every_destination(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_move_under_mode_none_indexes_nothing() {
        let Some(h) = h().await else { return };
        scenarios::move_under_mode_none_indexes_nothing(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_a_failed_move_leaves_the_index_alone() {
        let Some(h) = h().await else { return };
        scenarios::a_failed_move_leaves_the_index_alone(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_delete_clears_the_index() {
        let Some(h) = h().await else { return };
        scenarios::rest_delete_clears_the_index(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_recursive_delete_clears_the_whole_subtree() {
        let Some(h) = h().await else { return };
        scenarios::rest_recursive_delete_clears_the_whole_subtree(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_move_moves_the_index_entry() {
        let Some(h) = h().await else { return };
        scenarios::rest_move_moves_the_index_entry(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_copy_indexes_the_destination() {
        let Some(h) = h().await else { return };
        scenarios::rest_copy_indexes_the_destination(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_upload_indexes_the_file() {
        let Some(h) = h().await else { return };
        scenarios::rest_upload_indexes_the_file(&h, "rag").await;
    }

    #[tokio::test]
    async fn e2e_pg_rest_delete_under_mode_none_indexes_nothing() {
        let Some(h) = h().await else { return };
        scenarios::rest_delete_under_mode_none_indexes_nothing(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_a_failed_rest_delete_leaves_the_index_alone() {
        let Some(h) = h().await else { return };
        scenarios::a_failed_rest_delete_leaves_the_index_alone(&h, "rag").await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PostgreSQL-BM25 e2e tests: the same scenarios against tsvector, with no
// embedding endpoint configured.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "rag")]
mod pg_bm25 {
    use super::pg_rag::{pg_dsn, unique_schema};
    use super::scenarios;
    use super::shared::make_pg_bm25_harness;

    /// `None` when `MCPFS_TEST_PG_DSN` is unset, which skips the test.
    async fn h() -> Option<super::shared::E2eHarness> {
        let dsn = pg_dsn()?;
        Some(make_pg_bm25_harness(&dsn, &unique_schema()).await)
    }

    #[tokio::test]
    async fn e2e_pg_bm25_auto_index_write_then_query_finds_it() {
        let Some(h) = h().await else { return };
        scenarios::auto_index_write_then_query_finds_it(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_auto_index_delete_then_query_is_empty() {
        let Some(h) = h().await else { return };
        scenarios::auto_index_delete_then_query_is_empty(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_set_mode_none_wipes_the_index() {
        let Some(h) = h().await else { return };
        scenarios::set_mode_none_wipes_the_index(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_set_mode_active_backfills_existing_files() {
        let Some(h) = h().await else { return };
        scenarios::set_mode_active_backfills_existing_files(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_same_mode_twice_is_a_noop() {
        let Some(h) = h().await else { return };
        scenarios::same_mode_twice_is_a_noop(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_mode_none_does_not_index_writes() {
        let Some(h) = h().await else { return };
        scenarios::mode_none_does_not_index_writes(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_volume_isolation_under_auto_index() {
        let Some(h) = h().await else { return };
        scenarios::volume_isolation_under_auto_index(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_set_index_mode_rejects_rag_without_embedding_endpoint() {
        let Some(h) = h().await else { return };
        scenarios::set_index_mode_rejects_rag_without_embedding_endpoint(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_non_owner_cannot_set_index_mode() {
        let Some(h) = h().await else { return };
        scenarios::non_owner_cannot_set_index_mode(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_recursive_delete_clears_the_whole_subtree() {
        let Some(h) = h().await else { return };
        scenarios::recursive_delete_clears_the_whole_subtree(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_move_a_file_moves_its_index_entry() {
        let Some(h) = h().await else { return };
        scenarios::move_a_file_moves_its_index_entry(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_copy_a_file_indexes_the_destination() {
        let Some(h) = h().await else { return };
        scenarios::copy_a_file_indexes_the_destination(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_move_a_tree_moves_every_entry() {
        let Some(h) = h().await else { return };
        scenarios::move_a_tree_moves_every_entry(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_move_onto_an_existing_tree_strands_nothing() {
        let Some(h) = h().await else { return };
        scenarios::move_onto_an_existing_tree_strands_nothing(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_move_a_file_onto_an_existing_file_reindexes_it() {
        let Some(h) = h().await else { return };
        scenarios::move_a_file_onto_an_existing_file_reindexes_it(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_move_onto_an_existing_tree_strands_nothing() {
        let Some(h) = h().await else { return };
        scenarios::rest_move_onto_an_existing_tree_strands_nothing(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_copy_a_tree_indexes_every_destination() {
        let Some(h) = h().await else { return };
        scenarios::copy_a_tree_indexes_every_destination(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_move_under_mode_none_indexes_nothing() {
        let Some(h) = h().await else { return };
        scenarios::move_under_mode_none_indexes_nothing(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_a_failed_move_leaves_the_index_alone() {
        let Some(h) = h().await else { return };
        scenarios::a_failed_move_leaves_the_index_alone(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_delete_clears_the_index() {
        let Some(h) = h().await else { return };
        scenarios::rest_delete_clears_the_index(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_recursive_delete_clears_the_whole_subtree() {
        let Some(h) = h().await else { return };
        scenarios::rest_recursive_delete_clears_the_whole_subtree(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_move_moves_the_index_entry() {
        let Some(h) = h().await else { return };
        scenarios::rest_move_moves_the_index_entry(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_copy_indexes_the_destination() {
        let Some(h) = h().await else { return };
        scenarios::rest_copy_indexes_the_destination(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_upload_indexes_the_file() {
        let Some(h) = h().await else { return };
        scenarios::rest_upload_indexes_the_file(&h, "bm25").await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_rest_delete_under_mode_none_indexes_nothing() {
        let Some(h) = h().await else { return };
        scenarios::rest_delete_under_mode_none_indexes_nothing(&h).await;
    }

    #[tokio::test]
    async fn e2e_pg_bm25_a_failed_rest_delete_leaves_the_index_alone() {
        let Some(h) = h().await else { return };
        scenarios::a_failed_rest_delete_leaves_the_index_alone(&h, "bm25").await;
    }
}
