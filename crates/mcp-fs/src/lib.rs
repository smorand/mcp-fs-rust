//! mcp-fs: a streamable-HTTP MCP server exposing a simulated multi-project
//! filesystem, over SQLite, PostgreSQL or SQL Server.
//!
//! It began as a strict 1:1 port of a C#/.NET 9 implementation, which is why many
//! comments still explain a shape by pointing at it. That lineage is history, not a
//! constraint: parity was retired deliberately (see `.agent_docs/lineage.md`) and the
//! frozen external contract is now this project's own, pinned by
//! `tool-contract-golden.json`. A reference to the C# explains where a decision came
//! from; it is never a reason to keep or change behaviour.

pub mod api;
pub mod app;
pub mod cli;
pub mod config;
pub mod core;
pub mod docs;
pub mod errors;
pub mod git;
pub mod identity;
pub mod keys;
pub mod logging;
pub mod mcp;
pub mod migrate;
pub mod safety;
pub mod search;
pub mod state;
pub mod storage;
pub mod tools;
pub mod util;

pub use errors::{Result, ToolError};
