# MCP protocol: Design (as-built)

> Spec: SPEC-0028
> Status: as-built
> Generated on: 2026-10-14 at bc2b51a

## 1. Components

| Component | Responsibility | Location |
|---|---|---|
| MCP route | mounts the protocol service at `server.mcp_path` behind the identity middleware | `crates/core/src/app.rs:102-104`, `:132` |
| `mcp_auth` middleware | resolves the caller identity, answers 401 JSON on failure, passes the person to session creation | `crates/core/src/app.rs:301-325` |
| `build_mcp_service` | `rmcp::StreamableHttpService` with `LocalSessionManager` and `legacy_session_mode = true` | `crates/core/src/app.rs:283-292` |
| `McpServer` | per session tool router filtered by configuration, `#[tool_handler]` for list and call, `get_info` | `crates/core/src/mcp/server.rs:137-163`, `:3742-3747` |
| `to_call_result` | maps `Result<Value, ToolError>` to `CallToolResult` success or error | `crates/core/src/mcp/server.rs:46-56` |
| `add_optional_families` | adds the five optional families from their `ToolRegistry` via `ToolRoute::new_dyn` | `crates/core/src/mcp/server.rs:65-108` |
| Identity | bearer extraction from the configured header, then `Authorization` | `crates/core/src/identity.rs:116-125` |

## 2. Flows

- **SC-001**: request → `mcp_auth` (`app.rs:301`) → rmcp service (`app.rs:292`) → `initialize` handled by rmcp with `get_info` (`server.rs:3742`) → session id issued by `LocalSessionManager` → `tools/list` and `tools/call` from the session's router (`server.rs:137-163`) → handler → `to_call_result` (`server.rs:46`) → SSE frame.
- **SC-002**: missing token → 401 (`app.rs:317-325`); no `initialize` first → rmcp 422 (rmcp 3.5.0 `src/transport/common/server_side_http.rs:200-201`); unknown tool → rmcp `ToolRouter::call` `-32602 "tool not found"` (rmcp 3.5.0 `src/handler/server/router/tool.rs:566`, `:571`); undecodable body → rmcp 415 (`tower.rs:2014`).
- **SC-003**: `McpServer::new` removes `git.*` and `search.*` routes per flag (`server.rs:143-161`) and adds enabled families (`server.rs:65-108`).

## 3. Interfaces

- HTTP: `POST {server.mcp_path}` (default `/mcp`), headers `Accept: application/json, text/event-stream`, `Content-Type: application/json`, `Mcp-Session-Id` after `initialize`, bearer on `X-Forwarded-Authorization` or `Authorization`, `Host` checked by rmcp's allow list (`app.rs` module doc).
- JSON-RPC methods: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`; other spec methods get rmcp defaults.
- Tool result: `content[0].text` = serialized JSON; error result `isError: true`.

## 4. Data and state
- Sessions: in memory, `LocalSessionManager::default()` (`app.rs:292`); lost on restart.
- Caller identity: `tokio::task_local!` `CURRENT_PERSON` set by `mcp_auth` for the creating request (`app.rs` module and `:283-292`).

## 5. Configuration

| Name | Source | Default | Evidence |
|---|---|---|---|
| `server.mcp_path` | YAML | `/mcp` | `crates/core/src/config.rs:135-137` |
| `server.host`, `server.port` | YAML | `0.0.0.0`, `5002` | `crates/core/src/config.rs:129-134` |
| `auth.jwt.header` | YAML | `X-Forwarded-Authorization` | `crates/core/src/config.rs:162` |
| `git.enabled`, `search.enabled`, `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled`, `doc.enabled` | YAML; CLI flags force git and each optional family on (not search) | all `false` | `crates/core/src/config.rs:545`, `crates/core/src/cli.rs:57-73` |

## 6. Observability
Unauthenticated attempts logged by `logging::log_unauthenticated` (`app.rs:305`). No protocol level metrics or traces.

## 7. Decisions (observed)

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | Official `rmcp` 3.5.0 SDK over a hand written JSON-RPC layer | `Cargo.toml`, `crates/core/src/app.rs:283-292` | SPEC-0016 design (`sdlc/specs/SPEC-0016-rmcp-3x-migration-prep/sdlc-design.md:13-16`) |
| DEC-002 | `legacy_session_mode = true` with `LocalSessionManager`, which makes `initialize` mandatory and keeps SSE framing | `crates/core/src/app.rs:279-292` | SPEC-0016 design `:63-75` |
| DEC-003 | Domain errors are tool results with `isError: true`, the text format kept from the old transport | `crates/core/src/mcp/server.rs:44-56` | comment at `server.rs:44-45` |
| DEC-004 | Served tool set computed per session from configuration; static router kept as catalog | `crates/core/src/mcp/server.rs:137-163` | SPEC-0027 design DEC-001, DEC-002 |
| DEC-005 | Exact tool name matching only (no separator variants) | `crates/core/src/mcp/server.rs:137-163` (rmcp router) | owner decision 2026-10-14, BL-0028 |

## 8. Requirement to code map

| Requirement | Files |
|---|---|
| SPEC-0028/FR-001, FR-002 | `crates/core/src/app.rs:301-325`, `crates/core/src/identity.rs:116-125` |
| SPEC-0028/FR-003, FR-005, FR-006, FR-007, FR-014, FR-016 | `crates/core/src/app.rs:283-292` (rmcp service) |
| SPEC-0028/FR-004, FR-013 | `crates/core/src/mcp/server.rs:3742-3747` |
| SPEC-0028/FR-008, FR-011, FR-012 | `crates/core/src/mcp/server.rs:137-163`, `:65-108` |
| SPEC-0028/FR-009, FR-010 | `crates/core/src/mcp/server.rs:46-56` |
| SPEC-0028/FR-015 | `crates/core/src/app.rs:102-104`, `:132`, `crates/core/src/config.rs:135-137` |
