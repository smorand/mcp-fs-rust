---
Id: BL-0032
Title: Optional tool families (web/context7/sqlite/db/doc) unreachable over MCP after rmcp migration
Nature: BUG
Source: retro-spec 2026-10-09, SPEC-0016 (rmcp-3x-migration-prep) split
Created: 2026-10-09
Severity: Medium
Fingerprint: regression:crates/core/src/mcp/server.rs:optional-families-not-in-tool-router
---

## What
The five optional, off-by-default tool families (`web.*`, `context7.*`, `sqlite.*`, `db.*`,
`doc.*`) became unreachable over the MCP transport once it swapped to `rmcp`'s
`StreamableHttpService` wrapping `McpServer`, whose `#[tool_router]` only contains the 102-tool
contract. They remain reachable via the REST `/api/swagger.json` catalog (their `register()`/
`catalog()` is genuine production code there), but not as MCP tools.

## Why
Low practical risk today (nothing in the shipped config enables these flags), but it is a real,
undeclared regression for any deployment that does enable `--web`/`--context7`/`--sqlite`/`--db`/
`--doc`: those tools silently vanish from `tools/list` with no error.

## Evidence
`crates/core/src/mcp/server.rs` (`#[tool_router]`, 102-tool contract only);
`crates/core/src/tools/all.rs` (`register_all`, still registers the five optional families for
the REST catalog); legacy `specs/archived/SPEC-0013.../drift/2026-10-03_13-03-02.md` (now
superseded, content captured here).

## Notes
Needs either a follow-up spec adding `#[tool]` methods for the five optional families on
`McpServer`, or an explicit decision to drop MCP access to them (REST would remain the only
door). Not resolved by the rmcp migration; carried forward unresolved.
