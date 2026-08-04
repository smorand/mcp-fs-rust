//! Hand-rolled MCP layer (JSON-RPC 2.0 over streamable HTTP + SSE).
//!
//! Why hand-rolled instead of the `rmcp` SDK (DEC-107): the C# server runs the
//! ModelContextProtocol SDK with `Stateless = true`, which answers a bare
//! `tools/call` with no `initialize` handshake and no session header. `rmcp` 3.1
//! always requires `initialize` first (even with `NeverSessionManager`), so it
//! cannot reproduce that contract. A tools-only MCP surface is small, and owning
//! the wire format guarantees the 1:1 parity this port requires.
//!
//! Wire contract captured from the running C# server:
//! * response headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache,no-store`
//! * body framing: `event: message\ndata: {json}\n\n`
//! * success: `{"result":{"content":[{"type":"text","text":"<json>"}]},"id":N,"jsonrpc":"2.0"}`
//! * tool error: same + `"isError":true`, text `An error occurred invoking '<tool>': <CODE>: <msg>`
//! * unknown tool: `{"error":{"code":-32602,"message":"Unknown tool: '<name>'"},...}`
//! * `initialize` is also accepted, returning protocolVersion `2024-11-05`.

pub mod args;
pub mod registry;
pub mod schema;

pub use args::Args;
pub use registry::{ToolHandler, ToolRegistry};
pub use schema::ToolSchema;

use serde_json::{Value, json};

/// Protocol version advertised by `initialize`, matching the C# server.
pub const PROTOCOL_VERSION: &str = "2024-11-05";
/// Server name advertised by `initialize`, matching the C# server.
pub const SERVER_NAME: &str = "mcp-fs";

/// JSON-RPC error codes used by the C# surface.
pub mod rpc_error {
    pub const INVALID_PARAMS: i32 = -32602;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const PARSE_ERROR: i32 = -32700;
}

/// Build the SSE body for one JSON-RPC message, exactly as the C# SDK frames it.
pub fn sse_frame(payload: &Value) -> String {
    format!("event: message\ndata: {}\n\n", serde_json::to_string(payload).unwrap())
}

/// `{"result":<result>,"id":<id>,"jsonrpc":"2.0"}` with the C# key order.
pub fn rpc_result(id: Value, result: Value) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("result".into(), result);
    m.insert("id".into(), id);
    m.insert("jsonrpc".into(), json!("2.0"));
    Value::Object(m)
}

/// `{"error":{"code":c,"message":m},"id":<id>,"jsonrpc":"2.0"}`.
pub fn rpc_error(id: Value, code: i32, message: impl Into<String>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("error".into(), json!({"code": code, "message": message.into()}));
    m.insert("id".into(), id);
    m.insert("jsonrpc".into(), json!("2.0"));
    Value::Object(m)
}

/// A successful tool result: the tool's return value serialized as JSON text
/// inside a single text content block.
pub fn tool_ok(value: &Value) -> Value {
    json!({"content": [{"type": "text", "text": serde_json::to_string(value).unwrap()}]})
}

/// A failed tool result. The text reproduces the C# SDK's wrapper:
/// `An error occurred invoking '<tool>': <CODE>: <message>`.
pub fn tool_err(tool: &str, err: &crate::errors::ToolError) -> Value {
    json!({
        "content": [{"type": "text", "text": format!("An error occurred invoking '{tool}': {err}")}],
        "isError": true
    })
}

/// The `initialize` result, matching the C# server's advertised capabilities.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"logging": {}, "tools": {"listChanged": true}},
        "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ToolError;

    #[test]
    fn sse_framing_matches_csharp() {
        let f = sse_frame(&json!({"a": 1}));
        assert_eq!(f, "event: message\ndata: {\"a\":1}\n\n");
        assert!(f.starts_with("event: message\ndata: "));
        assert!(f.ends_with("\n\n"));
    }

    #[test]
    fn rpc_result_key_order_is_result_id_jsonrpc() {
        let v = rpc_result(json!(7), json!({"x": 1}));
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#"{"result":{"x":1},"id":7,"jsonrpc":"2.0"}"#);
    }

    #[test]
    fn rpc_error_shape_matches_unknown_tool() {
        let v = rpc_error(json!(9), rpc_error::INVALID_PARAMS, "Unknown tool: 'fs.nope'");
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(
            s,
            r#"{"error":{"code":-32602,"message":"Unknown tool: 'fs.nope'"},"id":9,"jsonrpc":"2.0"}"#
        );
    }

    #[test]
    fn tool_ok_wraps_json_as_text() {
        let v = tool_ok(&json!({"projects": []}));
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], r#"{"projects":[]}"#);
        assert!(v.get("isError").is_none());
    }

    #[test]
    fn tool_err_text_matches_csharp_wrapper() {
        let e = ToolError::forbidden("'admin@example.com' is not a member of 'p'");
        let v = tool_err("fs.list_allowed_roots", &e);
        assert_eq!(v["isError"], true);
        assert_eq!(
            v["content"][0]["text"],
            "An error occurred invoking 'fs.list_allowed_roots': ERR_FORBIDDEN: \
             'admin@example.com' is not a member of 'p'"
        );
    }

    #[test]
    fn initialize_advertises_csharp_shape() {
        let v = initialize_result();
        assert_eq!(v["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(v["capabilities"]["tools"]["listChanged"], true);
        assert!(v["capabilities"]["logging"].is_object());
        assert_eq!(v["serverInfo"]["name"], "mcp-fs");
    }
}
