//! Tool registry support: `ToolRegistry`, `ToolSchema`, `Args` and `ToolCtx`.
//!
//! Relocated from the old hand-rolled MCP dispatch layer (`mcp::{registry,
//! schema, args}`), which no longer dispatches any real request: `rmcp`'s
//! `McpServer::tool_router` is the one production dispatch path now. These
//! types survive for two real reasons, neither of which is "dispatch":
//! * `ToolCtx` is still the parameter type of every `tool_*` function
//!   `mcp::server::McpServer`'s `#[tool]` methods call directly (`git`,
//!   `git_auth`, `git_pr`, `doc`), so it is a genuine production type, not
//!   test-only. `Args` itself is production-only for `doc` (US-0009 moved
//!   `git`/`git_auth`/`git_pr`'s `tool_*` functions onto the typed
//!   `Parameters<T>` structs `mcp::server` already builds, so those
//!   `#[tool]` methods pass fields straight through instead of
//!   round-tripping through `Args`); every other family's use of `Args` is
//!   test-only.
//! * `ToolRegistry`/`ToolSchema` back the `catalog()` function of the five
//!   optional families (`web`, `context7`, `sqlite`, `db`, `doc`), which the
//!   REST `/api/swagger.json` catalog reads in production (`api/openapi.rs`).
//!
//! Every other family's `register()` is exercised by tests only (the shared
//! `tools::testkit::Harness`, `contract_golden.rs`, and each family's own
//! schema pinning tests), which is why most `register()` functions in this
//! tree are `#[cfg(test)]`.

pub mod args;
pub mod registry;
pub mod schema;

pub use args::Args;
pub use registry::{ToolCtx, ToolHandler, ToolRegistry, disabled, handler};
pub use schema::ToolSchema;
