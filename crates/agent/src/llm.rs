//! An OpenAI compatible chat client with tool calling and streaming.
//!
//! Only the streaming path is implemented, because the agent shows tokens as they arrive.
//! The wire format is `POST {base_url}/chat/completions` with `stream: true`, answered as
//! SSE where each `data:` line is a chunk and the stream ends with `data: [DONE]`.

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use serde_json::{Value, json};

/// How many times to retry a failed LLM call before giving up.
pub const MAX_RETRIES: u32 = 10;
/// Base delay in milliseconds; doubles on every attempt (capped at 30 s).
const RETRY_BASE_MS: u64 = 1_000;
const RETRY_CAP_MS: u64 = 30_000;

/// Whether an error is worth retrying (network/gateway transients only).
fn is_retriable(msg: &str) -> bool {
    // HTTP 429 or any 5xx, plus lower level connection failures.
    msg.contains("429")
        || msg.contains("500")
        || msg.contains("502")
        || msg.contains("503")
        || msg.contains("504")
        || msg.contains("connection")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("reset")
        || msg.contains("broken")
}

/// A message in the conversation, in wire shape.
#[derive(Debug, Clone)]
pub enum Message {
    System(String),
    User(String),
    /// Plain assistant prose.
    Assistant(String),
    /// An assistant turn that asked for tools.
    ToolCalls(Vec<ToolCall>),
    /// One tool's answer, tied back by call id.
    ToolResult { call_id: String, content: String },
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON text, because the model streams it in fragments and it may be invalid.
    pub arguments: String,
}

impl ToolCall {
    /// Parse the arguments, falling back to an empty object.
    ///
    /// A model sometimes emits nothing, `null`, or a truncated fragment. A tool that takes
    /// no argument is the common case, so an empty object is the useful interpretation;
    /// a genuinely malformed payload then fails at the server with a real error message.
    pub fn args(&self) -> Value {
        let t = self.arguments.trim();
        if t.is_empty() || t == "null" {
            return json!({});
        }
        serde_json::from_str(t).unwrap_or_else(|_| json!({}))
    }
}

/// What one streamed turn produced.
#[derive(Debug, Default)]
pub struct Turn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// A chat endpoint bound to one model.
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
}

impl LlmClient {
    /// Build a client. The timeout applies to one whole turn.
    pub fn new(base_url: &str, model: &str, api_key: &str, timeout_seconds: u64) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_seconds))
            .build()
            .context("building the LLM HTTP client")?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
        })
    }

    /// Stream one turn, invoking `on_text` for each text fragment as it arrives.
    ///
    /// The callback is how the caller renders tokens live and knows when to stop a
    /// spinner, so it is called before anything is accumulated.
    pub async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[Value],
        mut on_text: impl FnMut(&str),
    ) -> Result<Turn> {
        let body = json!({
            "model": self.model,
            "messages": messages.iter().map(wire_message).collect::<Vec<_>>(),
            "tools": tools,
            "stream": true,
        });

        let res = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("the chat request failed")?;

        let status = res.status();
        if !status.is_success() {
            let detail = res.text().await.unwrap_or_default();
            bail!("chat endpoint returned {status}: {}", squash(&detail));
        }

        let mut turn = Turn::default();
        // Tool call fragments arrive keyed by index, and the name and arguments can be
        // split across any number of chunks, so accumulate per index and flatten at the end.
        let mut partial: Vec<(String, String, String)> = Vec::new();
        let mut buf = String::new();
        let mut stream = res.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("the chat stream broke")?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // SSE events are separated by a blank line, but a `data:` line is complete as
            // soon as we see its newline, so process line by line and keep the remainder.
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim_end_matches('\r').to_string();
                buf.drain(..=nl);
                let Some(payload) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() {
                    continue;
                }
                if payload == "[DONE]" {
                    return Ok(finish(turn, partial));
                }
                let Ok(v) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                    bail!("chat endpoint error: {}", squash(&err.to_string()));
                }
                apply_chunk(&v, &mut turn, &mut partial, &mut on_text);
            }
        }
        Ok(finish(turn, partial))
    }

    /// Like [`stream_turn`] but retries on transient errors (network failures, 5xx, 429).
    ///
    /// Retry is only safe before the first token arrives: once the model starts streaming,
    /// stopping and restarting would produce duplicate or contradictory output. The callback
    /// receives a boolean that is `true` on a retry so the caller can reset any display state.
    ///
    /// Backoff: 1 s, 2 s, 4 s … capped at 30 s. Up to [`MAX_RETRIES`] attempts total.
    pub async fn stream_turn_with_retry(
        &self,
        messages: &[Message],
        tools: &[Value],
        mut on_text: impl FnMut(&str),
        mut on_retry: impl FnMut(u32, &str),
    ) -> Result<Turn> {
        let mut attempt = 0u32;
        loop {
            let mut got_token = false;
            // Shadow on_text to detect whether any token arrived before an error.
            let result = self
                .stream_turn(messages, tools, |chunk| {
                    got_token = true;
                    on_text(chunk);
                })
                .await;

            match result {
                Ok(turn) => return Ok(turn),
                Err(e) => {
                    let msg = e.to_string();
                    // Never retry if tokens already came through: the stream is partial.
                    if got_token || !is_retriable(&msg) || attempt >= MAX_RETRIES - 1 {
                        return Err(e);
                    }
                    attempt += 1;
                    let delay_ms =
                        (RETRY_BASE_MS * (1u64 << attempt.min(5))).min(RETRY_CAP_MS);
                    on_retry(attempt, &msg);
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
}

/// Fold one SSE chunk into the turn under construction.
fn apply_chunk(
    v: &Value,
    turn: &mut Turn,
    partial: &mut Vec<(String, String, String)>,
    on_text: &mut impl FnMut(&str),
) {
    let Some(delta) = v["choices"].get(0).map(|c| &c["delta"]) else {
        return;
    };

    if let Some(text) = delta["content"].as_str().filter(|s| !s.is_empty()) {
        on_text(text);
        turn.text.push_str(text);
    }

    let Some(calls) = delta["tool_calls"].as_array() else {
        return;
    };
    for c in calls {
        // The index positions the fragment; some endpoints omit it when there is one call.
        let idx = c["index"].as_u64().unwrap_or(0) as usize;
        while partial.len() <= idx {
            partial.push((String::new(), String::new(), String::new()));
        }
        let slot = &mut partial[idx];
        if let Some(id) = c["id"].as_str().filter(|s| !s.is_empty()) {
            slot.0 = id.to_string();
        }
        if let Some(name) = c["function"]["name"].as_str().filter(|s| !s.is_empty()) {
            slot.1.push_str(name);
        }
        if let Some(args) = c["function"]["arguments"].as_str() {
            slot.2.push_str(args);
        }
    }
}

/// Flatten the accumulated fragments into the finished turn.
fn finish(mut turn: Turn, partial: Vec<(String, String, String)>) -> Turn {
    turn.tool_calls = partial
        .into_iter()
        .enumerate()
        .filter(|(_, (_, name, _))| !name.is_empty())
        .map(|(i, (id, name, arguments))| ToolCall {
            // A missing id still needs to tie the result back, so synthesise a stable one.
            id: if id.is_empty() { format!("call_{i}") } else { id },
            name,
            arguments,
        })
        .collect();
    turn
}

/// One message in wire shape.
fn wire_message(m: &Message) -> Value {
    match m {
        Message::System(t) => json!({"role": "system", "content": t}),
        Message::User(t) => json!({"role": "user", "content": t}),
        Message::Assistant(t) => json!({"role": "assistant", "content": t}),
        Message::ToolCalls(calls) => json!({
            "role": "assistant",
            // An assistant turn that only asks for tools still needs the key present:
            // some endpoints reject a missing content field outright.
            "content": null,
            "tool_calls": calls.iter().map(|c| json!({
                "id": c.id,
                "type": "function",
                "function": {
                    "name": c.name,
                    "arguments": if c.arguments.trim().is_empty() { "{}" } else { c.arguments.as_str() },
                }
            })).collect::<Vec<_>>(),
        }),
        Message::ToolResult { call_id, content } => {
            json!({"role": "tool", "tool_call_id": call_id, "content": content})
        }
    }
}

/// Describe one MCP tool as a chat function declaration.
pub fn tool_declaration(name: &str, description: &str, input_schema: &Value) -> Value {
    // The schema is passed through as the parameter object. An absent or non object
    // schema still has to be a valid object schema, or the endpoint rejects the request.
    let params = if input_schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        input_schema.clone()
    } else {
        json!({"type": "object", "properties": {}})
    };
    json!({
        "type": "function",
        "function": {"name": name, "description": description, "parameters": params},
    })
}

/// Collapse whitespace and cap, so an HTML error page becomes one readable line.
fn squash(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 300 {
        let cut: String = flat.chars().take(297).collect();
        format!("{cut}...")
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed chunks through the accumulator the way the stream loop does.
    fn fold(chunks: &[Value]) -> (Turn, String) {
        let mut turn = Turn::default();
        let mut partial = Vec::new();
        let mut seen = String::new();
        for c in chunks {
            apply_chunk(c, &mut turn, &mut partial, &mut |t| seen.push_str(t));
        }
        (finish(turn, partial), seen)
    }

    fn text_chunk(t: &str) -> Value {
        json!({"choices": [{"delta": {"content": t}}]})
    }

    #[test]
    fn text_fragments_are_streamed_then_accumulated() {
        let (turn, seen) = fold(&[text_chunk("Hel"), text_chunk("lo "), text_chunk("world")]);
        assert_eq!(turn.text, "Hello world");
        assert_eq!(seen, "Hello world", "the callback saw every fragment in order");
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn an_empty_content_fragment_is_not_streamed() {
        let (_, seen) = fold(&[text_chunk(""), text_chunk("x")]);
        assert_eq!(seen, "x");
    }

    #[test]
    fn a_tool_call_split_across_chunks_is_reassembled() {
        let (turn, _) = fold(&[
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "c1", "function": {"name": "fs.re", "arguments": ""}}
            ]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"name": "ad", "arguments": "{\"mount_"}}
            ]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "id\":\"p\"}"}}
            ]}}]}),
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        let c = &turn.tool_calls[0];
        assert_eq!(c.id, "c1");
        assert_eq!(c.name, "fs.read", "the name arrived in two pieces");
        assert_eq!(c.args(), json!({"mount_id": "p"}));
    }

    #[test]
    fn two_parallel_tool_calls_stay_separate() {
        let (turn, _) = fold(&[json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "a", "function": {"name": "fs.read", "arguments": "{}"}},
            {"index": 1, "id": "b", "function": {"name": "fs.stat", "arguments": "{}"}}
        ]}}]})]);
        let names: Vec<_> = turn.tool_calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["fs.read", "fs.stat"]);
        assert_eq!(turn.tool_calls[1].id, "b");
    }

    #[test]
    fn a_call_arriving_out_of_index_order_lands_in_the_right_slot() {
        let (turn, _) = fold(&[
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 1, "id": "second", "function": {"name": "b", "arguments": "{}"}}
            ]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "first", "function": {"name": "a", "arguments": "{}"}}
            ]}}]}),
        ]);
        assert_eq!(turn.tool_calls[0].name, "a");
        assert_eq!(turn.tool_calls[1].name, "b");
    }

    #[test]
    fn a_missing_id_gets_a_synthetic_one_so_the_result_can_tie_back() {
        let (turn, _) = fold(&[json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "function": {"name": "fs.read", "arguments": "{}"}}
        ]}}]})]);
        assert_eq!(turn.tool_calls[0].id, "call_0");
    }

    #[test]
    fn a_fragment_with_no_name_is_dropped() {
        // Some endpoints emit a trailing empty tool_calls delta; it must not become a call.
        let (turn, _) = fold(&[json!({"choices": [{"delta": {"tool_calls": [
            {"index": 0, "function": {"arguments": "{}"}}
        ]}}]})]);
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn text_and_a_tool_call_in_one_turn_both_survive() {
        let (turn, _) = fold(&[
            text_chunk("let me look"),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "x", "function": {"name": "fs.read", "arguments": "{}"}}
            ]}}]}),
        ]);
        assert_eq!(turn.text, "let me look");
        assert_eq!(turn.tool_calls.len(), 1);
    }

    #[test]
    fn a_chunk_without_choices_is_ignored() {
        let (turn, seen) = fold(&[json!({"id": "x", "object": "chat.completion.chunk"})]);
        assert!(turn.text.is_empty() && seen.is_empty());
    }

    #[test]
    fn empty_or_null_arguments_parse_as_an_empty_object() {
        for raw in ["", "   ", "null"] {
            let c = ToolCall { id: "1".into(), name: "n".into(), arguments: raw.into() };
            assert_eq!(c.args(), json!({}), "for {raw:?}");
        }
    }

    #[test]
    fn malformed_arguments_degrade_to_an_empty_object() {
        let c = ToolCall { id: "1".into(), name: "n".into(), arguments: "{\"a\":".into() };
        assert_eq!(c.args(), json!({}));
    }

    #[test]
    fn a_tool_calls_message_carries_a_null_content_and_valid_arguments() {
        let m = Message::ToolCalls(vec![ToolCall {
            id: "c1".into(),
            name: "fs.read".into(),
            arguments: String::new(),
        }]);
        let w = wire_message(&m);
        assert_eq!(w["role"], "assistant");
        assert!(w["content"].is_null(), "the key must be present");
        assert_eq!(w["tool_calls"][0]["function"]["arguments"], "{}", "never an empty string");
        assert_eq!(w["tool_calls"][0]["type"], "function");
    }

    #[test]
    fn a_tool_result_message_ties_back_by_id() {
        let w = wire_message(&Message::ToolResult {
            call_id: "c1".into(),
            content: "ok".into(),
        });
        assert_eq!(w["role"], "tool");
        assert_eq!(w["tool_call_id"], "c1");
        assert_eq!(w["content"], "ok");
    }

    #[test]
    fn the_three_prose_roles_map_straight_through() {
        assert_eq!(wire_message(&Message::System("s".into()))["role"], "system");
        assert_eq!(wire_message(&Message::User("u".into()))["content"], "u");
        assert_eq!(wire_message(&Message::Assistant("a".into()))["role"], "assistant");
    }

    #[test]
    fn a_tool_declaration_passes_an_object_schema_through() {
        let schema = json!({"type": "object", "properties": {"path": {"type": "string"}}});
        let d = tool_declaration("fs.read", "Read a file.", &schema);
        assert_eq!(d["function"]["name"], "fs.read");
        assert_eq!(d["function"]["description"], "Read a file.");
        assert_eq!(d["function"]["parameters"], schema);
    }

    #[test]
    fn a_non_object_schema_is_replaced_by_a_valid_empty_one() {
        for bad in [json!(null), json!("nonsense"), json!({"type": "array"})] {
            let d = tool_declaration("t", "", &bad);
            assert_eq!(d["function"]["parameters"]["type"], "object", "for {bad}");
        }
    }

    #[test]
    fn squash_flattens_and_caps() {
        assert_eq!(squash("<html>\n  <body>  boom </body>"), "<html> <body> boom </body>");
        let out = squash(&"x ".repeat(400));
        assert_eq!(out.chars().count(), 300);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn is_retriable_matches_gateway_and_network_errors() {
        assert!(is_retriable("chat endpoint returned 502 Bad Gateway"));
        assert!(is_retriable("chat endpoint returned 503 Service Unavailable"));
        assert!(is_retriable("chat endpoint returned 429 Too Many Requests"));
        assert!(is_retriable("connection refused"));
        assert!(is_retriable("request timed out"));
        assert!(!is_retriable("chat endpoint returned 400 Bad Request"));
        assert!(!is_retriable("unknown tool 'fs.read'"));
    }
}
