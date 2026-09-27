//! HTTP embedding call to an OpenAI-compatible `/v1/embeddings` endpoint.
//!
//! The endpoint contract:
//! - POST `{"model": "...", "input": ["text"]}`
//! - Response `{"data": [{"embedding": [float, ...], "index": 0}], ...}`
//!
//! API keys are read from the environment at call time via `api_key_env`, not
//! stored in any struct that could be logged. Embedding vectors are never logged.

#![cfg(feature = "rag")]

use crate::config::EmbeddingConfig;
use crate::errors::{Result, ToolError};
use reqwest::Client;
use serde_json::Value;

/// Call the embedding endpoint and return the embedding for `text`.
///
/// `client` is the shared `reqwest::Client`; build it once at boot.
pub async fn embed(client: &Client, config: &EmbeddingConfig, text: &str) -> Result<Vec<f32>> {
    let api_key =
        if config.api_key_env.is_empty() { None } else { std::env::var(&config.api_key_env).ok() };

    let mut req = client.post(&config.endpoint).header("Content-Type", "application/json");

    if let Some(key) = &api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let body = serde_json::json!({
        "model": config.model,
        "input": [text],
    });

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::internal(format!("embedding request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(ToolError::internal(format!("embedding endpoint returned HTTP {status}")));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::internal(format!("parse embedding response: {e}")))?;

    let embedding = json
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|e| e.get("embedding"))
        .and_then(|e| e.as_array())
        .ok_or_else(|| ToolError::internal("embedding response missing data[0].embedding array"))?;

    let floats: Vec<f32> = embedding.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect();

    Ok(floats)
}
