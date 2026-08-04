//! mcp-fs: a streamable-HTTP MCP server exposing a simulated multi-project
//! filesystem. Rust port of the C#/.NET 9 implementation, with strict 1:1
//! external parity (tool names, parameters, `ERR_*` codes, JSON shapes, SQLite
//! schemas, git wire protocol, REST routes).

pub mod config;
pub mod errors;
pub mod identity;
pub mod mcp;
pub mod safety;
pub mod state;
pub mod storage;
pub mod util;

pub use errors::{Result, ToolError};
