//! The MCP server: `rmcp`'s own tool router and `StreamableHttpService`
//! (`mcp::server::McpServer`), the one production dispatch path for every
//! tool call.
//!
//! The hand-rolled JSON-RPC/SSE framing layer this module used to own
//! (`sse_frame`, `rpc_result`, `tool_ok`, `tool_err`, `initialize_result`) and
//! the dispatch registry it dispatched through (`ToolRegistry`, `ToolSchema`,
//! `Args`) are gone: `app::build` wires `rmcp`'s `StreamableHttpService`
//! directly over [`server::McpServer`] now. What remains of that registry
//! machinery lives at [`crate::tools::registry_support`], kept only because
//! `Args`/`ToolCtx` are still the parameter types of the `tool_*` functions
//! `McpServer`'s `#[tool]` methods call directly, and `ToolRegistry`/
//! `ToolSchema` still back the five optional families' `catalog()` functions
//! read by `/api/swagger.json`.

pub mod server;
