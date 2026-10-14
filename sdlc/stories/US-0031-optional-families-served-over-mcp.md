# US-0031: Enabled optional families are served over MCP

> Spec: SPEC-0027 (sdlc/specs/SPEC-0027-mcp-tool-surface-ignores-flags/)
> Nature: BUG
> Priority: P1
> Depends on: US-0030
> min_tier: 1
> Files touched: 4
> Closes spec: yes

## Objective
The 22 tools of the optional families (`web.*`, `context7.*`, `sqlite.*`, `db.*`, `doc.*`) are never served over MCP, even when their flag is on, so a deployment that enables one gets `-32602 tool not found`. After this story each enabled family is listed and callable, with the schemas its existing `register()` declares.

## Requirements covered
- SPEC-0027/BR-001: WHILE `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled` or `doc.enabled` is true THE MCP server SHALL list that family's tools in `tools/list` and execute them through `tools/call`.
- SPEC-0027/BR-002: IF an optional family's flag is false THEN THE MCP server SHALL list none of its tools, and `tools/call` on one SHALL answer JSON-RPC error `-32602` with message `tool not found`.
- SPEC-0027/BR-006: The MCP server SHALL list 59 tools with the default configuration (44 `fs.*`, 15 `admin.*`), 108 with `git.enabled`, 112 with `git.enabled` and `search.enabled`, and 134 with every flag on and pandoc on `PATH` (132 without pandoc, since `doc.*` registers nothing then, `crates/core/src/tools/doc.rs:3`).

## Independent Test
`make check` green; `cargo test -p mcp-fs-core --lib -- tool_surface_e2e` green: BT-001 creates a table with `sqlite.execute` and reads it back with `sqlite.list_tables`.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0027): in the served router built by US-0030 (`crates/core/src/mcp/server.rs`, `McpServer::new`), for each enabled family (`state.config.web.enabled`, `context7`, `sqlite`, `db`, `doc`), build its `ToolRegistry` with the family's production `register()` (`crates/core/src/tools/web.rs:25`, `context7.rs`, `sqlite.rs`, `db.rs`, `doc.rs`; `doc.rs:3` registers nothing without pandoc), and for each tool `add_route(ToolRoute::new_dyn(tool, call))` (rmcp 3.5.0 `src/handler/server/router/tool.rs:197`, `:436`). The route's `rmcp::model::Tool` comes from the family's `ToolSchema` (name, description, input schema, annotations; see `crates/core/src/tools/catalog.rs:10-40`). The call delegates to `ToolRegistry::call(name, ToolCtx { person, state }, Args(arguments))` (`crates/core/src/tools/registry_support/registry.rs:39-60`).
- DEC-004 (SPEC-0027): the family call's `Result<Value>` goes through `to_call_result` like every contract tool in `server.rs`.
- DEC-005 (SPEC-0027): tests in `crates/core/src/tools/tool_surface_e2e.rs` (from US-0030). Driver: JSON-RPC over the real HTTP MCP endpoint (initialize, `tools/list`, `tools/call`), as `crates/core/src/app.rs:505-690` does, against an app built with the stated configuration; new `#[cfg(test)]` module `crates/core/src/tools/tool_surface_e2e.rs` (DEC-005). `web.fetch` hits a local stub HTTP server; sqlite uses a temp volume.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/mcp/server.rs` | add the family routes |
| `crates/core/src/tools/registry_support/registry.rs` | expose what a route needs (schema to `Tool`, call) |
| `crates/core/src/tools/tool_surface_e2e.rs` | add tests |
| `.agent_docs/tools.md` | served set per configuration, timestamped 2026-10-14 |

## Acceptance tests
> 100% must pass, through `make check`.

### SPEC-0027/BT-001: An enabled optional family is callable
- **Reproduces:** BUG-001 (enabled but absent)
- **Fails today because:** the families have no `#[tool]` method (`server.rs:82`).
- Given a server with `sqlite.enabled: true` and project `atlas` (member Mia)
- When Mia calls `tools/call sqlite.execute {mount_id: "atlas", path: "/db.sqlite", sql: "CREATE TABLE t(a)"}`, then `sqlite.list_tables {mount_id: "atlas", path: "/db.sqlite"}`
- Then both succeed and the listing contains `t`; `tools/list` contains the 8 `sqlite.*` names

### SPEC-0027/BT-010: Each optional family listed when enabled
- **Validates:** BR-001, BR-006
- For each of `--web`, `--context7`, `--sqlite`, `--db`, `--doc` alone: `tools/list` returns 59 plus 5, 2, 8, 5, 2 names respectively, each with the description and input schema its `catalog()` gives.

### SPEC-0027/BT-011: All flags on
- **Validates:** BR-006, BR-005
- Given every flag on and pandoc on `PATH` / When `tools/list` / Then 134 names; without pandoc, 132.

### SPEC-0027/BT-012: A disabled family call is unknown
- **Validates:** BR-002, BR-004
- Default configuration: `tools/call web.search {query: "x"}` and `tools/call search.query {mount_id: "atlas", query: "x"}` each answer `-32602 tool not found`.

### SPEC-0027/BT-014: Web family callable through MCP
- **Validates:** BR-001
- Given `web.enabled: true` and a stub HTTP server / When `tools/call web.fetch {url: <stub>/page}` / Then the stub body is returned.

## Constraints
- Files not to touch: `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, `crates/core/src/api/openapi.rs`, the families' handler logic.
- Dependencies not to add: any.
- Patterns to avoid: hand written `#[tool]` copies of the 22 schemas.
- Count assertions for `doc.*` depend on pandoc on `PATH`: assert 2 when `which pandoc` succeeds, 0 otherwise.

## Definition of done
- [ ] RED for BT-001, BT-010, BT-014 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: US-0030 tests, family unit tests, golden contract.
- [ ] Docs updated: `.agent_docs/tools.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0027 green; BR-001..BR-006 cited by US-0030 or US-0031.
- [ ] One commit `US-0031: Enabled optional families are served over MCP`, body `Spec: SPEC-0027`.
