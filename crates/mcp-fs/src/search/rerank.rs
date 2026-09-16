//! HTTP rerank call to a Cohere/Jina-compatible rerank endpoint.
//!
//! Request: POST `{"model": "...", "query": "...", "documents": ["text1", ...]}`
//! Response: `{"results": [{"index": 0, "relevance_score": 0.9}, ...]}`
//!
//! The reranked results are returned in relevance-score order. API keys are read
//! from the environment at call time and never stored or logged.

#![cfg(feature = "rag")]

use crate::config::RerankConfig;
use crate::errors::{Result, ToolError};
use crate::search::SearchResult;
use reqwest::Client;
use serde_json::Value;

/// Rerank `results` by relevance to `query` using the configured HTTP endpoint.
///
/// Returns the results reordered with freshly assigned rank indices.
/// If `top_n` is zero the config value is used.
pub async fn rerank(
    client: &Client,
    config: &RerankConfig,
    query: &str,
    results: Vec<SearchResult>,
    top_n: usize,
) -> Result<Vec<SearchResult>> {
    if results.is_empty() || config.endpoint.is_empty() {
        return Ok(results);
    }

    let api_key = if config.api_key_env.is_empty() {
        None
    } else {
        std::env::var(&config.api_key_env).ok()
    };

    let n = if top_n == 0 { config.top_n } else { top_n };
    let documents: Vec<&str> = results.iter().map(|r| r.chunk.as_str()).collect();

    let mut req = client
        .post(&config.endpoint)
        .header("Content-Type", "application/json");

    if let Some(key) = &api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let body = serde_json::json!({
        "model": config.model,
        "query": query,
        "documents": documents,
        "top_n": n,
    });

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::internal(format!("rerank request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        // Reranking failures are non-fatal: return the original order.
        tracing::warn!(status, "rerank endpoint returned HTTP {status}, using original order");
        return Ok(results);
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::internal(format!("parse rerank response: {e}")))?;

    let reranked = json
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or_else(|| ToolError::internal("rerank response missing results array"))?;

    let mut out: Vec<SearchResult> = reranked
        .iter()
        .filter_map(|entry| {
            let idx = entry.get("index")?.as_u64()? as usize;
            let score = entry.get("relevance_score").and_then(|s| s.as_f64()).unwrap_or(0.0) as f32;
            results.get(idx).map(|r| SearchResult {
                path: r.path.clone(),
                score,
                chunk: r.chunk.clone(),
                rank: 0, // assigned below
            })
        })
        .collect();

    for (i, r) in out.iter_mut().enumerate() {
        r.rank = i + 1;
    }

    Ok(out)
}
