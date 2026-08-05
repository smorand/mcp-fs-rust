//! Context7 documentation tools: resolve a library ID and fetch its docs.
//!
//! Calls the Context7 public API at https://context7.com/api. No API key
//! required. Registered only when `context7.enabled` is true.

use crate::errors::ToolError;
use crate::mcp::registry::handler;
use crate::mcp::{ToolRegistry, ToolSchema};
use serde_json::Value;

/// Register the two `context7.*` tools.
pub fn register(reg: &mut ToolRegistry, config: &crate::config::Context7Config) {
    let cfg1 = config.clone();
    reg.add(
        ToolSchema::new(
            "context7.resolve_library_id",
            "Resolve a library name to its Context7 library ID. Use this before calling context7.get_library_docs.",
        )
        .req_str("library_name", "Library name to search for (e.g. 'react', 'tokio', 'numpy')."),
        handler(move |_ctx, a| {
            let cfg = cfg1.clone();
            async move {
                let name = a.str("library_name")?;
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err(ToolError::invalid_argument("library_name must not be blank"));
                }
                resolve_library_id(&name, &cfg).await
            }
        }),
    );

    let cfg2 = config.clone();
    reg.add(
        ToolSchema::new(
            "context7.get_library_docs",
            "Fetch documentation for a library from Context7. Returns markdown documentation.",
        )
        .req_str(
            "library_id",
            "Context7 library ID as returned by context7.resolve_library_id (e.g. '/facebook/react').",
        )
        .opt_str_null("topic", "Topic or section to focus on (e.g. 'hooks', 'routing').")
        .opt_int("tokens", 10_000, "Maximum number of tokens to return (capped at 50000)."),
        handler(move |_ctx, a| {
            let cfg = cfg2.clone();
            async move {
                let library_id = a.str("library_id")?;
                let library_id = library_id.trim().to_string();
                if library_id.is_empty() {
                    return Err(ToolError::invalid_argument("library_id must not be blank"));
                }
                let topic = a.opt_str("topic");
                let tokens = cap_tokens(a.int_or("tokens", 0));
                get_library_docs(&library_id, topic.as_deref(), tokens, &cfg).await
            }
        }),
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Cap the token count: 0 or negative => default 10000, else min(n, 50000).
fn cap_tokens(requested: i64) -> usize {
    if requested <= 0 { 10_000 } else { (requested as usize).min(50_000) }
}

fn build_client(timeout_secs: u64) -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .user_agent("mcp-fs/1.0")
        .build()
        .map_err(|e| ToolError::internal(format!("http client error: {e}")))
}

async fn resolve_library_id(
    name: &str,
    config: &crate::config::Context7Config,
) -> crate::errors::Result<Value> {
    let encoded = urlencoding(name);
    let url = format!("{}/v1/search?query={encoded}", config.api_url);

    let client = build_client(config.request_timeout_secs)?;
    let resp = client
        .get(&url)
        .header("X-Context7-Source", "mcp-fs")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ToolError::internal("request timed out")
            } else {
                ToolError::internal(format!("context7 search failed: {e}"))
            }
        })?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::internal(format!("parse context7 response: {e}")))?;

    let results = body.get("results").and_then(|r| r.as_array());
    match results {
        None => Err(ToolError::internal(format!("no library found for '{name}'"))),
        Some(arr) if arr.is_empty() => Err(ToolError::internal(format!("no library found for '{name}'"))),
        Some(arr) => {
            Ok(Value::String(
                serde_json::to_string_pretty(&Value::Array(arr.clone()))
                    .unwrap_or_default(),
            ))
        }
    }
}

async fn get_library_docs(
    library_id: &str,
    topic: Option<&str>,
    tokens: usize,
    config: &crate::config::Context7Config,
) -> crate::errors::Result<Value> {
    let mut url = format!("{}/v1{}?tokens={tokens}", config.api_url, library_id);
    if let Some(t) = topic {
        url.push_str(&format!("&topic={}", urlencoding(t)));
    }

    let client = build_client(config.request_timeout_secs)?;
    let resp = client
        .get(&url)
        .header("X-Context7-Source", "mcp-fs")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ToolError::internal("request timed out")
            } else {
                ToolError::internal(format!("context7 docs request failed: {e}"))
            }
        })?;

    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(ToolError::internal(format!("library '{library_id}' not found")));
    }
    if !status.is_success() {
        return Err(ToolError::internal(format!("context7 error: {status}")));
    }

    let text = resp.text().await.map_err(|e| ToolError::internal(format!("read docs response: {e}")))?;
    Ok(Value::String(text))
}

/// Percent-encode a string for use in a URL query parameter.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Context7Config;
    use crate::mcp::ToolRegistry;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &Context7Config::default());
        r
    }

    #[test]
    fn two_context7_tools_register() {
        assert_eq!(reg().len(), 2);
        assert_eq!(reg().names(), ["context7.resolve_library_id", "context7.get_library_docs"]);
    }

    #[test]
    fn resolve_schema_requires_library_name() {
        let r = reg();
        let t = r.resolve("context7.resolve_library_id").unwrap();
        let req = t.schema.input_schema()["required"].as_array().unwrap().to_vec();
        assert_eq!(req, vec![serde_json::json!("library_name")]);
    }

    #[test]
    fn get_docs_schema_requires_library_id() {
        let r = reg();
        let t = r.resolve("context7.get_library_docs").unwrap();
        let req = t.schema.input_schema()["required"].as_array().unwrap().to_vec();
        assert_eq!(req, vec![serde_json::json!("library_id")]);
    }

    #[test]
    fn tokens_cap_is_50000() {
        assert_eq!(cap_tokens(0), 10_000);
        assert_eq!(cap_tokens(1_000), 1_000);
        assert_eq!(cap_tokens(100_000), 50_000);
    }
}
