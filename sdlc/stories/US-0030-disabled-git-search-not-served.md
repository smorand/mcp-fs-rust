# US-0030: Disabled git and search tools are not served

> Spec: SPEC-0027 (sdlc/specs/SPEC-0027-mcp-tool-surface-ignores-flags/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
With `git.enabled: false` (the default) every project member can still list and run the 49 git tools over MCP, remote clone and push included; with `search.enabled: false` the 4 search tools are listed but always refuse. After this story a server serves no git tool when git is off and no search tool when search is off. Security: internal finding (CWE-284).

## Requirements covered
- SPEC-0027/BR-003: IF `git.enabled` is false THEN THE MCP server SHALL list no `git.*` tool (`git.auth*` and `git.pr_*` included), and `tools/call` on one SHALL answer `-32602` `tool not found` without touching any repository, token or remote.
- SPEC-0027/BR-004: IF `search.enabled` is false THEN THE MCP server SHALL list no `search.*` tool, and `tools/call` on one SHALL answer `-32602` `tool not found`.
- SPEC-0027/BR-005: WHEN `git.enabled` or `search.enabled` is true THE MCP server SHALL list and execute their tools exactly as today.
- SPEC-0027/BR-006: The MCP server SHALL list 59 tools with the default configuration (44 `fs.*`, 15 `admin.*`), 108 with `git.enabled`, 112 with `git.enabled` and `search.enabled`, and 134 with every flag on and pandoc on `PATH` (132 without pandoc, since `doc.*` registers nothing then, `crates/core/src/tools/doc.rs:3`).
(BR-006 partial: the 59, 108 and 112 counts; the optional families are US-0031.)

## Independent Test
`make check` green; `cargo test -p mcp-fs-core --lib -- tool_surface_e2e` green: BT-005 reads the repository directory and the git index, BT-006 reads the recording server's request count.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0027): in `McpServer::new` (`crates/core/src/mcp/server.rs:79-91`, router built at `:82`), remove every route whose name starts with `git.` when `state.config.git.enabled` is false, and every `search.` route when `state.config.search.enabled` is false, with `ToolRouter::remove_route` (rmcp 3.5.0 `src/handler/server/router/tool.rs:456`). `tools/list` and `tools/call` come from that router (`server.rs:3659-3665`).
- DEC-002 (SPEC-0027): the served router is per `McpServer`; the static `McpServer::tool_router()` stays the catalog used by `tool_router_lists_exactly_the_100_contract_names` (`server.rs:4128`), the golden contract and `/api/swagger.json` (`crates/core/src/api/openapi.rs:136`). Do not change those.
- DEC-005 (SPEC-0027): Driver: JSON-RPC over the real HTTP MCP endpoint (initialize, `tools/list`, `tools/call`), as `crates/core/src/app.rs:505-690` does, against an app built with the stated configuration; new `#[cfg(test)]` module `crates/core/src/tools/tool_surface_e2e.rs` (DEC-005).
### Security
Occurrence: `server.rs:82` (router ignores `git.enabled`); git handlers only check membership (`crates/core/src/tools/git.rs:1769-1778`), which stays. Exploit tests carry the literal JSON-RPC and read the absence of effect on a second channel (repository directory `config.git_repo_dir("atlas")`, `crates/core/src/config.rs:967`; git index rows; a recording HTTP server's request log).
### Existing tests to adapt
Every test that calls a `git.*` tool through `McpServer` or the HTTP MCP endpoint while `git.enabled` is false must enable git in its configuration (spec 4.4). Find them by running the suite after the change; list each one changed in the commit body. Do not weaken any assertion.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/mcp/server.rs` | served router: remove git and search routes per config |
| `crates/core/src/tools/tool_surface_e2e.rs` | new |
| `crates/core/src/tools/mod.rs` | `#[cfg(test)] mod tool_surface_e2e;` |
| tests relying on the bypass | enable git in their config |
| `.agent_docs/architecture.md` | served tool set rule, timestamped 2026-10-14 |

## Acceptance tests
> 100% must pass, through `make check`.

### SPEC-0027/BT-005: A disabled git tool cannot create a repository
- **Covers:** the git occurrence at `server.rs:82` and `tools/git.rs:1769-1778`
- **Red before the fix because:** `git.init` runs and creates a repository.
- Given the default configuration (`git.enabled: false`), project `atlas`, member Mia
- When Mia sends `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"git.init","arguments":{"mount_id":"atlas"}}}`
- Then the answer is JSON-RPC error `-32602`, message `tool not found`
- And the git repository directory of `atlas` (`config.git_repo_dir("atlas")`, `config.rs:967`) does not exist, and no git index row exists for `atlas`

### SPEC-0027/BT-006: A disabled git tool cannot reach a remote
- **Covers:** same occurrence, remote path
- **Red before the fix because:** `git.remote_clone` is dispatched and attempts the clone.
- Given the default configuration and a local bare repository served at `http://127.0.0.1:<port>/r.git` that records every request
- When Mia sends `tools/call git.remote_clone {mount_id: "atlas", url: "http://127.0.0.1:<port>/r.git"}`
- Then `-32602 tool not found`
- And the recording server saw 0 requests, and `atlas` holds no file

### SPEC-0027/BT-007: Disabled tools are not listed
- **Covers:** listing occurrences
- Given the default configuration / When `tools/list` / Then 59 names, none starting with `git.`, `search.`, `web.`, `context7.`, `sqlite.`, `db.`, `doc.`

### SPEC-0027/BT-020: Enabled git and search unchanged
- **Protects:** BR-005, SPEC-0012, SPEC-0008
- Given `git.enabled` and `search.enabled` true / Then `tools/list` returns 112 names, and `git.init` then `git.status` on `atlas` succeed as today.

### SPEC-0027/BT-021: Git HTTP and token screen gating unchanged
- **Protects:** `app.rs:155-167`
- Default configuration: `GET /git/atlas/info/refs?service=git-upload-pack` and `GET /app/tokens` answer 404, as today.

### SPEC-0027/BT-022: Contract unchanged
- **Protects:** `tool-contract-golden.json`
- The golden contract tests pass unmodified; every served tool's name, description and input schema equal today's.

## Constraints
- Files not to touch: `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, `crates/core/src/api/openapi.rs`, the optional family modules (US-0031).
- Dependencies not to add: any.
- Patterns to avoid: a refusal inside each git handler instead of route removal.
- BT-005 and BT-006 must be observed RED before the fix (the call is dispatched today).

## Definition of done
- [ ] RED for BT-005, BT-006, BT-007 before the fix.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: golden contract tests, `tool_router_lists_exactly_the_100_contract_names`, git HTTP gating.
- [ ] Docs updated: `.agent_docs/architecture.md`.
- [ ] Self review: full, plus the security rules.
- [ ] One commit `US-0030: Disabled git and search tools are not served`, body `Spec: SPEC-0027`, listing every adapted test.
