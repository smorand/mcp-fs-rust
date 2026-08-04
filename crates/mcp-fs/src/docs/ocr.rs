//! Pluggable image to text (OCR). Port of the C# `Core/Ocr.cs`.
//!
//! The default is a no-op provider: no native Tesseract dependency, so the build
//! stays a single static binary. Swapping in image understanding is a config
//! change (`extract.ocr.provider: multimodal`), nothing else in the extraction
//! pipeline moves.
//!
//! Security: the configured prompt and the API key are request material only.
//! They are never logged, never echoed in an error message and never returned to
//! the caller, because tool output flows straight into an LLM context.

use crate::config::OcrConfig;
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use base64::Engine as _;
use std::sync::Arc;
use std::time::Duration;

/// A source of text for an image. Object safe so the extractor can hold a
/// `&dyn OcrProvider` chosen at composition root time.
#[async_trait]
pub trait OcrProvider: Send + Sync {
    /// False when no provider is configured: the extractor then emits a note
    /// instead of attempting a call.
    fn enabled(&self) -> bool;

    /// Recovered text, or an empty string when nothing could be read.
    async fn extract_text(&self, image: &[u8], mime_type: &str) -> Result<String>;
}

/// No OCR configured: image extraction yields an empty result plus a note.
/// Mirrors the C# `NullOcrProvider` (empty string, never an error).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullOcrProvider;

#[async_trait]
impl OcrProvider for NullOcrProvider {
    fn enabled(&self) -> bool {
        false
    }

    async fn extract_text(&self, _image: &[u8], _mime_type: &str) -> Result<String> {
        Ok(String::new())
    }
}

/// Delegates OCR to a multimodal vision model over an OpenAI compatible
/// `chat/completions` endpoint, sending the image as a data URL. Point
/// `extract.ocr.endpoint` / `model` at your provider and put the key in the
/// environment variable named by `api_key_env`.
pub struct MultimodalOcrProvider {
    config: OcrConfig,
    http: reqwest::Client,
}

impl MultimodalOcrProvider {
    /// A three minute timeout matches the C#: vision models on large scans are
    /// slow, and a stalled extraction is worse than a slow one.
    pub fn new(config: OcrConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Inject a client (tests point it at a local stub server).
    pub fn with_client(config: OcrConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }
}

#[async_trait]
impl OcrProvider for MultimodalOcrProvider {
    fn enabled(&self) -> bool {
        !self.config.endpoint.trim().is_empty()
    }

    async fn extract_text(&self, image: &[u8], mime_type: &str) -> Result<String> {
        if !self.enabled() {
            return Ok(String::new());
        }
        let api_key = std::env::var(&self.config.api_key_env).unwrap_or_default();
        let b64 = base64::engine::general_purpose::STANDARD.encode(image);
        let data_url = format!("data:{mime_type};base64,{b64}");
        let payload = serde_json::json!({
            "model": self.config.model,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": self.config.prompt },
                    { "type": "image_url", "image_url": { "url": data_url } },
                ],
            }],
        });

        let mut request = self.http.post(&self.config.endpoint).json(&payload);
        if !api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {api_key}"));
        }
        let response = request
            .send()
            .await
            // Only the transport class of failure is surfaced: the request body
            // carries the prompt, so it must never reach the caller.
            .map_err(|e| ToolError::internal(format!("ocr request failed: {}", transport_kind(&e))))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ToolError::internal(format!("ocr provider returned HTTP {}", status.as_u16())));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| ToolError::internal("ocr provider returned a non JSON body"))?;
        Ok(body
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string())
    }
}

/// Classify a transport failure without leaking the URL or the payload.
fn transport_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connection refused"
    } else if e.is_decode() {
        "malformed response"
    } else {
        "transport error"
    }
}

/// Build the provider named by the config. Anything other than `multimodal`
/// (including the default `none`) yields the no-op provider, like the C#
/// `MultimodalOcrProvider.FromConfig`.
pub fn provider_from_config(config: &OcrConfig) -> Arc<dyn OcrProvider> {
    match config.provider.as_str() {
        "multimodal" => Arc::new(MultimodalOcrProvider::new(config.clone())),
        _ => Arc::new(NullOcrProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: &str, endpoint: &str) -> OcrConfig {
        OcrConfig {
            provider: provider.into(),
            endpoint: endpoint.into(),
            model: "vision-1".into(),
            api_key_env: "MCPFS_TEST_OCR_KEY".into(),
            prompt: "transcribe".into(),
        }
    }

    #[tokio::test]
    async fn null_provider_is_disabled_and_returns_empty() {
        let p = NullOcrProvider;
        assert!(!p.enabled());
        assert_eq!(p.extract_text(b"\x89PNG", "image/png").await.unwrap(), "");
    }

    #[test]
    fn from_config_defaults_to_null() {
        assert!(!provider_from_config(&cfg("none", "")).enabled());
        assert!(!provider_from_config(&OcrConfig::default()).enabled());
        assert!(!provider_from_config(&cfg("tesseract", "http://x")).enabled());
    }

    #[test]
    fn from_config_builds_multimodal_when_named_and_endpoint_set() {
        assert!(provider_from_config(&cfg("multimodal", "http://localhost:1/v1")).enabled());
    }

    #[test]
    fn multimodal_without_endpoint_is_disabled() {
        // an endpoint-less multimodal config must behave like the null provider
        assert!(!provider_from_config(&cfg("multimodal", "   ")).enabled());
    }

    #[tokio::test]
    async fn multimodal_disabled_returns_empty_without_calling_out() {
        let p = MultimodalOcrProvider::new(cfg("multimodal", ""));
        assert_eq!(p.extract_text(b"x", "image/png").await.unwrap(), "");
    }

    #[tokio::test]
    async fn multimodal_transport_error_does_not_leak_the_prompt() {
        // port 1 is closed, so this fails at connect time
        let p = MultimodalOcrProvider::new(cfg("multimodal", "http://127.0.0.1:1/v1/chat"));
        let err = p.extract_text(b"x", "image/png").await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INTERNAL_ERROR);
        assert!(!err.message.contains("transcribe"), "prompt must not leak: {}", err.message);
        assert!(!err.message.contains("127.0.0.1"), "endpoint must not leak: {}", err.message);
    }
}
