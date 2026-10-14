# SPEC-0027: The MCP tool surface ignores the feature flags, design

> Spec: sdlc/specs/SPEC-0027-mcp-tool-surface-ignores-flags/sdlc-spec.md
> Nature: BUG
> Depth: L
> Security: internal finding
> Designed on: 2026-10-14
> Target tier: 2 (default); both stories raised to 1, see section 11

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| MCP | rmcp, existing (`server`, `transport-streamable-http-server`) | 3.5.0 (`Cargo.lock`) | DEC-001, DEC-003 |
Skills loaded: rust, effect: `make check`, clippy `-D warnings`, `cargo fmt`. No dependency added.

## 2. Approaches
### Chosen: build the served router per configuration, at the root cause site
`McpServer::new` (`crates/core/src/mcp/server.rs:79-91`) starts from the static `Self::tool_router()`
and then (a) removes every `git.*` route when `git.enabled` is false and every `search.*` route when
`search.enabled` is false, with `ToolRouter::remove_route` (rmcp 3.5.0
`src/handler/server/router/tool.rs:456`), and (b) adds one route per tool of each enabled optional
family with `ToolRoute::new_dyn` (`tool.rs:197`) and `ToolRouter::add_route` (`tool.rs:436`), whose
call delegates to the family's existing handler through its `ToolRegistry`
(`crates/core/src/tools/registry_support/registry.rs:39-60`). `tools/list` and `tools/call` are generated
from that router (`server.rs:3659-3665`), so listing and dispatch change together.
- Pros: one place decides the served set; handlers, schemas and annotations of the families are reused
  as they are; the static catalog used by the golden contract and `/api/swagger.json` is untouched.
- Cons: the families keep their `Args` style handlers instead of typed `#[tool]` methods.
- Scope: `mcp/server.rs`, `tools/registry_support/registry.rs`, tests.
### Rejected: typed `#[tool]` methods for the 22 family tools
Restates 22 schemas by hand, duplicating the families' `register()` descriptions (contract drift risk)
and still needs the removal step for git and search. DEC-003.
### Rejected: refuse inside each git handler when git is disabled
Leaves the 49 tools listed (BR-003 requires absence) and touches 49 handlers. DEC-001.

## 3. Architecture
```
config ──► McpServer::new(state, person)
             router = Self::tool_router()            static 112 (108 contract + 4 search)
             if !git.enabled    remove git.*            (DEC-001)
             if !search.enabled remove search.*         (DEC-001)
             for family in [web, context7, sqlite, db, doc] if enabled:
                 reg = family::register(...)            existing production register()
                 for tool in reg: add_route(new_dyn(attr from schema, call -> reg.call(name, ctx, args)))   (DEC-003)
           ──► #[tool_handler] tools/list, tools/call
```
The served set is computed once per `McpServer` (per session), from a configuration fixed for the
process (spec assumption).

## 4. Data model
No change.

## 5. Contracts and interfaces
- Served tool names, descriptions, input schemas and annotations: exactly today's (contract tools) and
  exactly the families' `register()` schemas (optional tools).
- A tool not served: JSON-RPC `-32602`, `tool not found` (rmcp `tool.rs:566`, `:571`).
- A family handler error maps through the same `to_call_result` as contract tools (DEC-004).
- Internal: `fn served_router(state: &AppState) -> ToolRouter<McpServer>` in `mcp/server.rs`;
  `ToolRegistry` exposes what a route needs (`schema_tool(name) -> rmcp::model::Tool` or equivalent).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/mcp/server.rs` | filter git and search routes | BR-003, BR-004, BR-005, BR-006 | US-0030 |
| `crates/core/src/tools/tool_surface_e2e.rs` | new, MCP over HTTP tests | spec section 4 | US-0030, US-0031 |
| `crates/core/src/tools/mod.rs` | register test module | DEC-005 | US-0030 |
| existing tests calling git tools with git disabled (`tools/git.rs`, `app.rs`, `tools/quota_e2e.rs`, others found by the run) | enable git in their configuration | spec 4.4 | US-0030 |
| `crates/core/src/mcp/server.rs` | add optional family routes | BR-001, BR-002, BR-006 | US-0031 |
| `crates/core/src/tools/registry_support/registry.rs` | expose schema to rmcp `Tool` | DEC-003 | US-0031 |
| `.agent_docs/tools.md` | served set per configuration, 2026-10-14 | BR-006 | US-0031 |
Affected existing tests: golden and router catalog tests kept (spec 4.4).

## 7. Compatibility and risks
- Clients of a default deployment stop seeing 53 tools (49 git, 4 search) they could not use or should
  not have used. Intended (BR-003, BR-004).
- Tests and harnesses that called git tools while git was off must enable git; this is the only
  expected test change.
- Ordering: US-0030 lands the removal and its exploit tests; US-0031 adds the families.
- Rollback: revert.

## 8. Security design
- Threat: a project member invokes git operations (repository writes, remote network access, token
  use) on a server whose operator disabled git. Trust boundary: operator configuration versus member.
- Occurrence fixed: `server.rs:82` (router built without the configuration), mechanism: removal of
  every `git.*` route from the served router when `git.enabled` is false (DEC-001). The git handlers'
  membership check (`tools/git.rs:1769-1778`) stays as the second line.
- Exploit tests: BT-005 sends the literal `tools/call git.init` JSON-RPC and reads the absence of effect
  on disk (`config.git_repo_dir("atlas")` absent) and in the git index (no row); BT-006 sends
  `git.remote_clone` to a recording local HTTP server and reads its request log (0 requests). Both are
  red before the fix because the call is dispatched.
- Nothing new is logged; no secret is handled.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-005, BT-006, BT-007 | exploit | JSON-RPC over HTTP MCP (`app.rs:505-690` pattern) | `tools/tool_surface_e2e.rs` | US-0030 |
| BT-020, BT-021, BT-022 | non regression | same; HTTP GET; golden tests | same, existing | US-0030 |
| BT-001, BT-010, BT-011, BT-012, BT-014 | functional e2e | same | `tools/tool_surface_e2e.rs` | US-0031 |
Fixtures: server built through the app router with a config closure; `web.fetch` against a local stub
HTTP server; sqlite on a temp volume. `doc.*` expectations depend on pandoc presence (BR-006).

## 10. Decisions log
- **DEC-001:** remove `git.*` (including `git.auth*`, `git.pr_*`) routes when `git.enabled` is false,
  and `search.*` routes when `search.enabled` is false, from the router in `McpServer::new`.
  **Obliged by:** BR-003, BR-004, BR-005. **Rationale:** listing and dispatch come from the router, so
  one removal fixes both. **Alternatives considered:** per handler refusal. **Source:** design.
  **Evidence:** rmcp 3.5.0 `tool.rs:456`.
- **DEC-002:** the served router is computed per `McpServer` from `state.config`; the static
  `McpServer::tool_router()` stays the catalog for the golden contract and `/api/swagger.json`.
  **Obliged by:** BR-005 (contract unchanged), BT-022. **Source:** existing code `openapi.rs:136`,
  `server.rs:4128`.
- **DEC-003:** each enabled optional family's tools are added with `ToolRoute::new_dyn`, the route's
  `Tool` built from the family's `ToolSchema` (name, description, input schema, annotations), the call
  delegating to `ToolRegistry::call(name, ToolCtx { person, state }, Args(arguments))`.
  **Obliged by:** BR-001, BR-006. **Rationale:** reuses the production `register()`; no hand copied
  schema. **Alternatives considered:** 22 typed `#[tool]` methods. **Source:** existing code
  `registry.rs:39-60`, `catalog.rs:10-40`. **Evidence:** rmcp 3.5.0 `tool.rs:197`, `:436`.
- **DEC-004:** a family call's `Result<Value>` goes through `to_call_result` like contract tools.
  **Obliged by:** BR-001. **Source:** existing code `server.rs` (every `#[tool]` method).
- **DEC-005:** tests in a new `#[cfg(test)]` module `tools/tool_surface_e2e.rs`, JSON-RPC over the HTTP
  MCP endpoint. **Obliged by:** spec section 4 drivers. **Source:** existing code `app.rs:505-690`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0030 | Disabled git and search tools are not served | P1 | BR-003, BR-004, BR-005, BR-006 | main | 1 | no |
| US-0031 | Enabled optional families are served over MCP | P1 | BR-001, BR-002, BR-006 | US-0030 | 1 | yes |
min_tier 1: US-0030 must find and fix every existing test relying on the bypass across the tree;
US-0031 bridges two tool frameworks.
