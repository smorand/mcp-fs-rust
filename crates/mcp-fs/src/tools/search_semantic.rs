//! Semantic search tool family: `search.index`, `search.query`, `search.delete`,
//! `search.status`.
//!
//! These tools are optional: they are only registered when `search.enabled` is
//! true in the server config. Each tool authorizes via `state.authorize` before
//! any storage access, exactly like the `fs.*` family.
//!
//! Mode dispatch:
//! - `bm25`  : Tantivy on SQLite (always available)
//! - `rag`   : vector KNN, requires `rag` feature and embedding config
//! - `both`  : BM25 + vector, RRF merge, optional reranking

use crate::config::SearchConfig;
use crate::errors::ToolError;
use crate::mcp::registry::handler;
use crate::mcp::{ToolRegistry, ToolSchema};
use crate::search::fusion::rrf_merge;
use serde_json::{Value, json};

/// Register the four `search.*` tools.
pub fn register(reg: &mut ToolRegistry, _config: &SearchConfig) {
    reg.add(
        ToolSchema::new(
            "search.index",
            "Index a file or directory into the search engine for this volume.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path to index (file or directory).")
        .opt_bool("recursive", false, "Recurse into subdirectories when path is a directory.")
        .opt_int("chunk_size", 1000, "Maximum character size of each indexed chunk.")
        .opt_int("chunk_overlap", 100, "Character overlap between consecutive chunks."),
        handler(|ctx, a| async move {
            let mount = a.str("mount_id")?;
            ctx.state.authorize(&mount, &ctx.person).await?;

            let path = ctx.state.safety.normalize_path(&a.str("path")?)?;
            let recursive = a.bool_or("recursive", false);
            let chunk_size = a.int_or("chunk_size", 1000) as usize;
            let chunk_overlap = a.int_or("chunk_overlap", 100) as usize;

            let backend = ctx
                .state
                .search
                .as_ref()
                .ok_or_else(|| ToolError::not_supported("search is not enabled; set search.enabled: true in config"))?;

            let client = ctx.state.stores.client(&mount).await?;

            let mut indexed = 0usize;
            let mut skipped = 0usize;

            // Collect paths to index.
            let paths_to_index: Vec<String> = if recursive {
                crate::core::fs_ops::iter_files(&client, &path, &[])
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, _mtime)| p)
                    .collect()
            } else {
                vec![path.clone()]
            };

            for file_path in &paths_to_index {
                // Try to read text content; skip binary or unreadable files.
                match client.read_text(file_path).await {
                    Ok(text) => {
                        match backend.index_path(&mount, file_path, &text, chunk_size, chunk_overlap).await {
                            Ok(n) => indexed += n,
                            Err(_) => skipped += 1,
                        }
                    }
                    Err(_) => {
                        skipped += 1;
                    }
                }
            }

            Ok(json!({
                "indexed": indexed,
                "skipped": skipped,
                "path": path,
            }))
        }),
    );

    reg.add(
        ToolSchema::new(
            "search.query",
            "Search indexed content. mode overrides the server default (bm25, rag, both).",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("query", "Search query text.")
        .opt_str_null("mode", "Query mode: bm25, rag, or both. Defaults to server config.")
        .opt_int("top_k", 10, "Maximum number of results to return.")
        .opt_bool("rerank", true, "Apply reranking when configured and available."),
        handler(|ctx, a| async move {
            let mount = a.str("mount_id")?;
            ctx.state.authorize(&mount, &ctx.person).await?;

            let query = a.str("query")?;
            let top_k = a.int_or("top_k", 10) as usize;
            let _rerank = a.bool_or("rerank", true);

            // Resolve mode: explicit param beats the config default.
            let config_mode = &ctx.state.config.search.mode;
            let mode = a.opt_str("mode").unwrap_or_else(|| config_mode.clone());

            let backend = ctx
                .state
                .search
                .as_ref()
                .ok_or_else(|| ToolError::not_supported("search is not enabled; set search.enabled: true in config"))?;

            // Validate that the requested mode is supported.
            if matches!(mode.as_str(), "rag" | "both") {
                if ctx.state.config.search.embedding.endpoint.is_empty() {
                    return Err(ToolError::not_supported(
                        "mode rag/both requires search.embedding.endpoint to be configured",
                    ));
                }
                if !backend.supported_modes().contains(&mode.as_str()) {
                    return Err(ToolError::not_supported(format!(
                        "mode '{mode}' is not supported by the current search backend"
                    )));
                }
            }

            let (results, mode_used) = match mode.as_str() {
                "bm25" => {
                    let r = backend.query_bm25(&mount, &query, top_k).await?;
                    (r, "bm25".to_string())
                }
                "rag" => {
                    let r = backend.query_vector(&mount, &query, top_k).await?;
                    (r, "rag".to_string())
                }
                "both" => {
                    let bm25 = backend.query_bm25(&mount, &query, top_k).await.unwrap_or_default();
                    let vec = backend.query_vector(&mount, &query, top_k).await.unwrap_or_default();
                    let merged = rrf_merge(&bm25, &vec);
                    (merged, "both".to_string())
                }
                other => {
                    return Err(ToolError::invalid_argument(format!(
                        "unknown mode '{other}', expected bm25, rag, or both"
                    )));
                }
            };

            // Check whether BM25 was warm, to add a warning.
            let warning: Option<&str> = if matches!(mode_used.as_str(), "bm25" | "both") {
                match backend.stats(&mount).await {
                    Ok(s) if !s.bm25_warm => {
                        Some("BM25 index is not warm; run search.index to populate it")
                    }
                    _ => None,
                }
            } else {
                None
            };

            let result_values: Vec<Value> = results
                .iter()
                .map(|r| json!({
                    "path": r.path,
                    "score": r.score,
                    "chunk": r.chunk,
                    "rank": r.rank,
                }))
                .collect();

            Ok(json!({
                "results": result_values,
                "mode_used": mode_used,
                "warning": warning,
            }))
        }),
    );

    reg.add(
        ToolSchema::new("search.delete", "Remove a path from the search index.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path to remove from the index.")
            .opt_bool("recursive", false, "Recurse into subdirectories when path is a directory."),
        handler(|ctx, a| async move {
            let mount = a.str("mount_id")?;
            ctx.state.authorize(&mount, &ctx.person).await?;

            let path = ctx.state.safety.normalize_path(&a.str("path")?)?;
            let _recursive = a.bool_or("recursive", false);

            let backend = ctx
                .state
                .search
                .as_ref()
                .ok_or_else(|| ToolError::not_supported("search is not enabled; set search.enabled: true in config"))?;

            let deleted = backend.delete_path(&mount, &path).await?;

            Ok(json!({ "deleted": deleted }))
        }),
    );

    reg.add(
        ToolSchema::new("search.status", "Report index statistics for this volume.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(|ctx, a| async move {
            let mount = a.str("mount_id")?;
            ctx.state.authorize(&mount, &ctx.person).await?;

            let backend = ctx
                .state
                .search
                .as_ref()
                .ok_or_else(|| ToolError::not_supported("search is not enabled; set search.enabled: true in config"))?;

            let stats = backend.stats(&mount).await?;

            Ok(json!({
                "bm25_docs": stats.bm25_docs,
                "vector_chunks": stats.vector_chunks,
                "mode": stats.mode,
                "bm25_warm": stats.bm25_warm,
            }))
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SearchConfig;
    use crate::mcp::ToolRegistry;
    use crate::search::bm25_sqlite::TantivyBm25Backend;
    use crate::tools::testkit::MOUNT;
    use std::sync::Arc;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &SearchConfig::default());
        r
    }

    #[test]
    fn search_tools_register_four_tools() {
        assert_eq!(
            reg().names(),
            ["search.index", "search.query", "search.delete", "search.status"]
        );
    }

    /// Build a harness with a real Tantivy BM25 backend injected.
    async fn search_harness() -> (crate::tools::testkit::Harness, tempfile::TempDir) {
        let search_dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TantivyBm25Backend::new(search_dir.path().to_str().unwrap()));
        let h = crate::tools::testkit::harness_with_search(
            |c| { c.search.enabled = true; },
            Some(backend as Arc<dyn crate::search::SearchBackend>),
        )
        .await;
        (h, search_dir)
    }

    #[tokio::test]
    async fn bm25_index_and_query_sqlite() {
        let (h, _dir) = search_harness().await;
        // Seed a file in the volume.
        h.seed("/doc.md", "the quick brown fox jumps over the lazy dog").await;

        // Index through the backend directly.
        let n = h
            .state
            .search
            .as_ref()
            .unwrap()
            .index_path(MOUNT, "/doc.md", "the quick brown fox jumps over the lazy dog", 200, 0)
            .await
            .unwrap();
        assert!(n > 0, "expected at least one chunk indexed");

        let results = h
            .state
            .search
            .as_ref()
            .unwrap()
            .query_bm25(MOUNT, "fox", 10)
            .await
            .unwrap();
        assert!(!results.is_empty(), "expected at least one result");
        assert_eq!(results[0].path, "/doc.md");
    }

    #[tokio::test]
    async fn bm25_delete_removes_doc() {
        let (h, _dir) = search_harness().await;
        h.state
            .search
            .as_ref()
            .unwrap()
            .index_path(MOUNT, "/a.md", "hello world search test", 200, 0)
            .await
            .unwrap();
        h.state
            .search
            .as_ref()
            .unwrap()
            .delete_path(MOUNT, "/a.md")
            .await
            .unwrap();
        let results = h
            .state
            .search
            .as_ref()
            .unwrap()
            .query_bm25(MOUNT, "hello", 10)
            .await
            .unwrap();
        assert!(results.is_empty(), "deleted doc must not appear in results");
    }

    #[tokio::test]
    async fn status_zero_on_empty_volume() {
        let (h, _dir) = search_harness().await;
        let stats = h.state.search.as_ref().unwrap().stats(MOUNT).await.unwrap();
        assert_eq!(stats.bm25_docs, 0);
        assert!(!stats.bm25_warm);
    }

    #[tokio::test]
    async fn rag_mode_without_embedding_config_returns_not_supported() {
        let (h, _dir) = search_harness().await;
        // BM25 backend does not support vector queries.
        let backend = h.state.search.as_ref().unwrap();
        let err = backend.query_vector(MOUNT, "fox", 10).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_SUPPORTED);
    }

    #[test]
    fn search_schema_matches_contract_index() {
        let r = reg();
        let t = r.resolve("search.index").unwrap();
        let schema = t.schema.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required[0], "mount_id");
        assert_eq!(required[1], "path");
    }

    #[test]
    fn search_schema_matches_contract_query() {
        let r = reg();
        let t = r.resolve("search.query").unwrap();
        let schema = t.schema.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required[0], "mount_id");
        assert_eq!(required[1], "query");
    }

    #[test]
    fn search_schema_matches_contract_delete() {
        let r = reg();
        let t = r.resolve("search.delete").unwrap();
        let schema = t.schema.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required[0], "mount_id");
        assert_eq!(required[1], "path");
    }

    #[test]
    fn search_schema_matches_contract_status() {
        let r = reg();
        let t = r.resolve("search.status").unwrap();
        let schema = t.schema.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required[0], "mount_id");
    }
}
