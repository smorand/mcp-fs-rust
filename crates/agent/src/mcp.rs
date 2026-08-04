//! A minimal MCP client over streamable HTTP.
//!
//! The server is stateless: it answers a bare `tools/list` or `tools/call` with no
//! `initialize` handshake, so this client does not perform one. It accepts either a plain
//! JSON body or an SSE framed one, because the server picks the framing from the `Accept`
//! header and we advertise both.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// One tool as advertised by the server.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The outcome of `tools/call`.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    /// Concatenated text blocks, which is what the model is shown.
    pub text: String,
    pub is_error: bool,
}

/// A connected client. Cheap to clone the inner reqwest client, so calls are sequential
/// by construction here but the transport is reusable.
pub struct McpClient {
    http: reqwest::Client,
    url: String,
    auth_header: String,
    token: String,
}

impl McpClient {
    /// Build a client. No network traffic yet.
    pub fn new(url: &str, auth_header: &str, token: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            url: url.to_string(),
            auth_header: auth_header.to_string(),
            token: token.to_string(),
        })
    }

    /// The tool catalogue, keyed by name so a call can resolve exactly.
    pub async fn list_tools(&self) -> Result<BTreeMap<String, Tool>> {
        let res = self.rpc("tools/list", json!({})).await?;
        let arr = res["tools"]
            .as_array()
            .ok_or_else(|| anyhow!("tools/list returned no tools array"))?;
        let mut out = BTreeMap::new();
        for t in arr {
            let name = t["name"].as_str().unwrap_or_default().to_string();
            if name.is_empty() {
                continue;
            }
            out.insert(
                name.clone(),
                Tool {
                    name,
                    description: t["description"].as_str().unwrap_or_default().to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                },
            );
        }
        if out.is_empty() {
            bail!("the server advertised no tools");
        }
        Ok(out)
    }

    /// Invoke a tool. A tool level failure is reported in the outcome rather than as an
    /// error, because the model needs to see it and decide what to do next.
    pub async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolOutcome> {
        let res = self.rpc("tools/call", json!({"name": name, "arguments": args})).await?;
        Ok(ToolOutcome {
            text: collect_text(&res),
            is_error: res["isError"].as_bool().unwrap_or(false),
        })
    }

    /// One JSON-RPC round trip, returning the `result` object.
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let res = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/json")
            // Both framings are advertised, and `parse_body` handles whichever comes back.
            .header("Accept", "application/json, text/event-stream")
            .header(&self.auth_header, format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {} failed", self.url))?;

        let status = res.status();
        let text = res.text().await.context("reading the response body")?;
        if !status.is_success() {
            bail!("server returned {status}: {}", first_line(&text));
        }

        let v = parse_body(&text)?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let msg = err["message"].as_str().unwrap_or("unknown error");
            let code = err["code"].as_i64().unwrap_or(0);
            bail!("{method} failed ({code}): {msg}");
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("{method} returned no result: {}", first_line(&text)))
    }
}

/// Accept a plain JSON body or an SSE stream, returning the JSON-RPC envelope.
///
/// The SSE case can carry several `data:` lines; the last complete JSON object wins,
/// which is the envelope for the request we just made.
fn parse_body(text: &str) -> Result<Value> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed)
            .with_context(|| format!("invalid JSON body: {}", first_line(text)));
    }
    let mut last = None;
    for line in text.lines() {
        if let Some(payload) = line.strip_prefix("data:")
            && let Ok(v) = serde_json::from_str::<Value>(payload.trim())
        {
            last = Some(v);
        }
    }
    last.ok_or_else(|| anyhow!("no JSON payload in the response: {}", first_line(text)))
}

/// Join the text blocks of a `tools/call` result.
///
/// The server may return the payload as text blocks or, for a structured result, as a
/// bare JSON object; both are rendered as text for the model.
fn collect_text(result: &Value) -> String {
    if let Some(blocks) = result["content"].as_array() {
        let joined = blocks
            .iter()
            .filter(|b| b["type"].as_str().unwrap_or("text") == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return joined;
        }
    }
    if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
        return sc.to_string();
    }
    String::new()
}

/// The first line, capped, for a one line error message.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 200 {
        let cut: String = line.chars().take(197).collect();
        format!("{cut}...")
    } else {
        line.to_string()
    }
}

/// Resolve a name the model produced against the catalogue.
///
/// Models routinely emit `admin_list_projects` for `admin.list_projects`, because many
/// function calling schemas forbid a dot. Rather than fail, match on the shape with dots
/// and underscores treated as equivalent, so the user sees the real server behaviour.
pub fn resolve_name<'a>(tools: &'a BTreeMap<String, Tool>, wanted: &str) -> Option<&'a str> {
    if let Some((k, _)) = tools.get_key_value(wanted) {
        return Some(k.as_str());
    }
    let flat = |s: &str| s.replace('.', "_");
    let target = flat(wanted);
    tools.keys().find(|k| flat(k) == target).map(|k| k.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue(names: &[&str]) -> BTreeMap<String, Tool> {
        names
            .iter()
            .map(|n| {
                (
                    n.to_string(),
                    Tool {
                        name: n.to_string(),
                        description: String::new(),
                        input_schema: json!({"type": "object"}),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_plain_json_body_parses() {
        let v = parse_body("{\"jsonrpc\":\"2.0\",\"result\":{\"ok\":true}}").unwrap();
        assert_eq!(v["result"]["ok"], true);
    }

    #[test]
    fn an_sse_framed_body_parses() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"result\":{\"n\":1}}\n\n";
        let v = parse_body(sse).unwrap();
        assert_eq!(v["result"]["n"], 1);
    }

    #[test]
    fn the_last_sse_payload_wins() {
        let sse = "data: {\"result\":{\"n\":1}}\n\ndata: {\"result\":{\"n\":2}}\n\n";
        assert_eq!(parse_body(sse).unwrap()["result"]["n"], 2);
    }

    #[test]
    fn a_body_with_no_payload_is_an_error_quoting_it() {
        let e = parse_body("upstream exploded").unwrap_err();
        assert!(e.to_string().contains("upstream exploded"), "got {e}");
    }

    #[test]
    fn text_blocks_are_joined_and_non_text_blocks_ignored() {
        let r = json!({"content": [
            {"type": "text", "text": "one"},
            {"type": "image", "data": "zzz"},
            {"type": "text", "text": "two"}
        ]});
        assert_eq!(collect_text(&r), "one\ntwo");
    }

    #[test]
    fn a_structured_result_falls_back_to_its_json() {
        let r = json!({"content": [], "structuredContent": {"total": 3}});
        assert_eq!(collect_text(&r), "{\"total\":3}");
    }

    #[test]
    fn an_empty_result_is_an_empty_string() {
        assert_eq!(collect_text(&json!({})), "");
    }

    #[test]
    fn an_exact_tool_name_resolves() {
        let t = catalogue(&["fs.read", "admin.list_projects"]);
        assert_eq!(resolve_name(&t, "fs.read"), Some("fs.read"));
    }

    #[test]
    fn an_underscored_name_resolves_to_the_dotted_one() {
        let t = catalogue(&["admin.list_projects", "fs.read"]);
        assert_eq!(resolve_name(&t, "admin_list_projects"), Some("admin.list_projects"));
    }

    #[test]
    fn an_unknown_name_does_not_resolve() {
        let t = catalogue(&["fs.read"]);
        assert_eq!(resolve_name(&t, "fs.teleport"), None);
    }

    #[test]
    fn first_line_caps_a_long_body() {
        let long = "x".repeat(500);
        let out = first_line(&long);
        assert_eq!(out.chars().count(), 200);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn first_line_does_not_split_a_multibyte_char() {
        let long = "é".repeat(500);
        let out = first_line(&long);
        assert_eq!(out.chars().count(), 200, "counted in chars, not bytes");
    }
}
