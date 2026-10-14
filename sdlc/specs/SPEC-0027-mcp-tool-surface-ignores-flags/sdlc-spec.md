# The MCP tool surface ignores the feature flags: Bug Specification

> Generated on: 2026-10-14
> Id: SPEC-0027
> Nature: BUG
> Depth: L
> Depth evidence: 3 modules (MCP server listing and dispatch, the git tools, the five optional tool families), 6 requirements, public tool listing changes per configuration (restored), no data model change
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0032
> Security: internal finding
> CVSS: not scored
> Affected: every deployment since the move to the `rmcp` transport (SPEC-0013, `McpServer` tool router)
> Fixed in: unreleased

`--security` applied without the flag. An operator who disables git (`git.enabled: false`) still has
every `git.*` tool executable by any project member over MCP, remote clone, fetch and push included.
That is an operator control that is not enforced (CWE-284).

## 1. Defect

### BUG-001: `tools/list` and `tools/call` serve one fixed tool set, whatever the configuration
- **Observed behavior:**
  - **One fixed list.** The MCP surface is `McpServer`'s `rmcp` tool router, built once from the
    static `#[tool_router]` (`crates/core/src/mcp/server.rs:79-91`). `#[tool_handler]` generates
    `tools/list` and `tools/call` from it (`server.rs:3659-3665`). The router holds the 108 contract
    tools plus the 4 `search.*` tools whatever the configuration. Counts from
    `tool-contract-golden.json`: 44 `fs.*`, 15 `admin.*`, 40 `git.*`, 3 `git.auth*`, 6 `git.pr_*`.
    The router test asserts 108 non search names (`server.rs:4128-4150`).
  - **Disabled but executable.** With `git.enabled: false`, the default (`crates/core/src/config.rs:545`),
    the 49 git tools are listed and run. `authorize` checks membership only and opens the shared
    repository store (`crates/core/src/tools/git.rs:1769-1778`); no git tool reads `git.enabled`.
    Only the git HTTP routes and the token screen are gated (`crates/core/src/app.rs:155-167`).
  - **Disabled, listed, refused.** With `search.enabled: false`, the 4 `search.*` tools are listed;
    calling one is refused with `ERR_NOT_SUPPORTED`
    `search is not enabled; set search.enabled: true in config` (`server.rs:2837`, `:2905`, `:2993`, `:3015`).
  - **Enabled but absent.** With `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled` or
    `doc.enabled` true, their 22 tools are missing from `tools/list`, and `tools/call` answers
    JSON-RPC `-32602`, `tool not found`. The families are 5 `web.*`, 2 `context7.*`, 8 `sqlite.*`,
    5 `db.*` and 2 `doc.*`; their `register()` lives only in a `#[cfg(test)]` registry
    (`crates/core/src/tools/all.rs:7-55`). They are not reachable over REST either:
    `/api/swagger.json` documents only the REST operations of `api/dataplane.rs` (`Op.tool` names
    `fs.*` tools, `crates/core/src/api/openapi.rs:440`, `:538-576`), and the optional families'
    `catalog()` entries (`openapi.rs:145-160`) only supply description text no operation uses.
- **Expected per authority:**
  - SPEC-0009/FR-001: "IF an optional family's `enabled` flag is false THEN none of its tools SHALL be
    registered", and "An LLM must not see a tool it cannot call"
    (`sdlc/specs/SPEC-0009-optional-integrations/spec.md:170-176`). Its E2E-001 to E2E-020 call the
    enabled tools through `tools/call`. E2E-021: a disabled family's tools are absent, and calling one
    returns a JSON-RPC unknown tool error (`spec.md:453`).
  - SPEC-0012, cross cutting: "git disabled → tool not registered"
    (`sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:67`).
  - SPEC-0008/FR-001: the server "SHALL register `search.index`, `search.query`, `search.delete` and
    `search.status` when `search.enabled` is true, and SHALL register none of them otherwise"
    (`sdlc/specs/SPEC-0008-search-and-rag/spec.md:150-151`).
  - SPEC-0006: `doc.to_docx` and `doc.to_pptx` are registered conditionally
    (`sdlc/specs/SPEC-0006-documents/spec.md:52`).
- **Reproduction:**
  1. Default configuration, project `atlas`, member Mia. `tools/list` returns 112 tools, among them
     `git.init` and `search.query`. `tools/call git.init {mount_id: "atlas"}` succeeds and creates a
     repository, while git is disabled.
  2. Start with `--sqlite`. `tools/call sqlite.list_tables {mount_id: "atlas", path: "/db.sqlite"}`
     answers `-32602 tool not found`.
- **Root cause:**
  - `crates/core/src/mcp/server.rs:82` builds the router from the static `Self::tool_router()` with
    no reference to `state.config`.
  - The optional families have no `#[tool]` method on `McpServer`; `tools/all.rs:7-55` is test only.
- **Contributing factors:** the router and golden tests check the static catalog, not what a
  configured server serves. No test starts a server with a flag on and calls a family tool.
- **Impact:**
  - Every deployment advertises and executes 49 git tools the operator disabled: repository
    creation, history rewrite, remote clone, fetch, pull and push to declared hosts, and token use.
  - It advertises 4 search tools that always fail.
  - Deployments that enable an optional family get none of its 22 tools over MCP.
  - Agents waste calls on tools that refuse, and miss tools that exist.
- **Rollback posture:** reverting restores today's surface; no stored data changes.

## 1bis. Class Sweep

**Class:** an operator configuration that disables or enables a capability is not applied where a
client discovers and invokes capabilities (CWE-284, improper access control of a disabled feature).

**Searches run:**
1. Feature flags against the code that enforces them on the tool surface:
   `rg -n 'config\.(git|search|web|context7|sqlite|db|doc)\.enabled' crates/core/src`. Hits are in
   `app.rs:155` (HTTP routes), `api/openapi.rs:77` and `:145-160` (catalog), `cli.rs:306-321` (overrides),
   `search/mod.rs:125` (no search backend is built when search is off) and `purge.rs:429` (git purge
   on project removal), and none in `mcp/server.rs` or `tools/git.rs`.
2. Refusals on disabled capabilities in tool handlers:
   `rg -n 'is not enabled' crates/core/src`. Only the four `search.*` handlers refuse
   (`server.rs:2837`, `:2905`, `:2993`, `:3015`), plus `admin.set_index_mode` (`:2687`).
3. Every surface that lists tools: `tools/list` (router). `/api/swagger.json` lists REST operations,
   not tools (`openapi.rs:219-233`, `:440`), so it lists no git, search or optional family entry.

Not searched: the REST data plane `/api/fs`, which exposes no git, search or optional family operation.

| Occurrence | Verdict | Disposition |
|---|---|---|
| git tools listed and executable while `git.enabled` is false (`server.rs:82`, `tools/git.rs:1769-1778`) | vulnerable | BR-003 |
| search tools listed while `search.enabled` is false (`server.rs:82`) | listed but safe from execution: refused at `server.rs:2837`, `:2905`, `:2993`, `:3015` | BR-004 |
| optional families not served while enabled (`server.rs:82`, `tools/all.rs:7-55`) | defective (missing capability, not exposure) | BR-001, BR-002 |
| `/api/swagger.json` (`openapi.rs:69-85`) | safe: documents `fs.*` REST operations only; git HTTP paths only when `git.enabled` (`openapi.rs:77`) | none |
| git HTTP routes and `/app/tokens` (`app.rs:155-167`) | safe: mounted only when `git.enabled` | BT-021 |
| git purge on project removal (`purge.rs:429`, reached from `admin.delete_project` and the grace purge) | safe: runs only when `git.enabled` (`purge.rs:429`) | none |
| search backend and auto indexing (`search/mod.rs:125`, `search/indexer.rs:187`) | safe: no backend is built when `search.enabled` is false, so no hook indexes | none |

## 2. Expected Behavior

Tool names, descriptions and input schemas of every tool stay exactly as they are; only which
tools are served changes with the configuration.

#### BR-001 [EARS-S]: An enabled optional family is served
> WHILE `web.enabled`, `context7.enabled`, `sqlite.enabled`, `db.enabled` or `doc.enabled` is true THE
> MCP server SHALL list that family's tools in `tools/list` and execute them through `tools/call`.
- **Source:** restored from SPEC-0009/FR-001 and E2E-001 to E2E-020; SPEC-0006 (`doc.*`).
- **Exact names:** 5 `web.*`, 2 `context7.*`, 8 `sqlite.*`, 5 `db.*`, 2 `doc.*`, names and schemas as in the families' `catalog()`.

#### BR-002 [EARS-O]: A disabled optional family is absent
> IF an optional family's flag is false THEN THE MCP server SHALL list none of its tools, and
> `tools/call` on one SHALL answer JSON-RPC error `-32602` with message `tool not found`.
- **Source:** restored from SPEC-0009/FR-001 and E2E-021; error text as today for an unknown tool (`app.rs:664-687`).

#### BR-003 [EARS-O]: Disabled git tools are absent and inert
> IF `git.enabled` is false THEN THE MCP server SHALL list no `git.*` tool (`git.auth*` and
> `git.pr_*` included), and `tools/call` on one SHALL answer `-32602` `tool not found` without
> touching any repository, token or remote.
- **Source:** restored from SPEC-0012 ("git disabled → tool not registered").

#### BR-004 [EARS-O]: Disabled search tools are absent
> IF `search.enabled` is false THEN THE MCP server SHALL list no `search.*` tool, and `tools/call` on
> one SHALL answer `-32602` `tool not found`.
- **Source:** restored from SPEC-0008/FR-001.

#### BR-005 [EARS-E]: Enabled contract families keep working
> WHEN `git.enabled` or `search.enabled` is true THE MCP server SHALL list and execute their tools exactly as today.
- **Source:** restored from SPEC-0012 and SPEC-0008.

#### BR-006 [EARS-U]: Counts per configuration
> The MCP server SHALL list 59 tools with the default configuration (44 `fs.*`, 15 `admin.*`), 108
> with `git.enabled`, 112 with `git.enabled` and `search.enabled`, and 134 with every flag on and
> pandoc on `PATH` (132 without pandoc, since `doc.*` registers nothing then, `crates/core/src/tools/doc.rs:3`).
- **Source:** restored from the authorities above; counts recounted from `tool-contract-golden.json` and the five `catalog()` functions.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| flag set by CLI override only (`--sqlite`) | served, as with the YAML flag | BR-001 |
| `doc.enabled` true but pandoc absent | as SPEC-0006 states: conditional on pandoc's presence | BR-001 |
| two sessions on one server | both see the same list (configuration is per process) | BR-006 |
| a disabled family's tool called with valid arguments | `-32602 tool not found`, no side effect | BR-002, BR-003 |

## 4. Tests

Driver everywhere: JSON-RPC over the real HTTP MCP endpoint (initialize, `tools/list`, `tools/call`),
as `app.rs:505-690` does, against a server built with the stated configuration.

### 4.1 End to end reproduction (mandatory)
#### BT-001: An enabled optional family is callable
- **Reproduces:** BUG-001 (enabled but absent)
- **Fails today because:** the families have no `#[tool]` method (`server.rs:82`).
- Given a server with `sqlite.enabled: true` and project `atlas` (member Mia)
- When Mia calls `tools/call sqlite.execute {mount_id: "atlas", path: "/db.sqlite", sql: "CREATE TABLE t(a)"}`, then `sqlite.list_tables {mount_id: "atlas", path: "/db.sqlite"}`
- Then both succeed and the listing contains `t`; `tools/list` contains the 8 `sqlite.*` names

### 4.1bis Exploit tests
#### BT-005: A disabled git tool cannot create a repository
- **Covers:** the git occurrence at `server.rs:82` and `tools/git.rs:1769-1778`
- **Red before the fix because:** `git.init` runs and creates a repository.
- Given the default configuration (`git.enabled: false`), project `atlas`, member Mia
- When Mia sends `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"git.init","arguments":{"mount_id":"atlas"}}}`
- Then the answer is JSON-RPC error `-32602`, message `tool not found`
- And the git repository directory of `atlas` (`config.git_repo_dir("atlas")`, `config.rs:967`) does not exist, and no git index row exists for `atlas`

#### BT-006: A disabled git tool cannot reach a remote
- **Covers:** same occurrence, remote path
- **Red before the fix because:** `git.remote_clone` is dispatched and attempts the clone.
- Given the default configuration and a local bare repository served at `http://127.0.0.1:<port>/r.git` that records every request
- When Mia sends `tools/call git.remote_clone {mount_id: "atlas", url: "http://127.0.0.1:<port>/r.git"}`
- Then `-32602 tool not found`
- And the recording server saw 0 requests, and `atlas` holds no file

#### BT-007: Disabled tools are not listed
- **Covers:** listing occurrences
- Given the default configuration / When `tools/list` / Then 59 names, none starting with `git.`, `search.`, `web.`, `context7.`, `sqlite.`, `db.`, `doc.`

### 4.2 New behavior
#### BT-010: Each optional family listed when enabled
- **Validates:** BR-001, BR-006
- For each of `--web`, `--context7`, `--sqlite`, `--db`, `--doc` alone: `tools/list` returns 59 plus 5, 2, 8, 5, 2 names respectively, each with the description and input schema its `catalog()` gives.

#### BT-011: All flags on
- **Validates:** BR-006, BR-005
- Given every flag on and pandoc on `PATH` / When `tools/list` / Then 134 names; without pandoc, 132.

#### BT-012: A disabled family call is unknown
- **Validates:** BR-002, BR-004
- Default configuration: `tools/call web.search {query: "x"}` and `tools/call search.query {mount_id: "atlas", query: "x"}` each answer `-32602 tool not found`.

#### BT-014: Web family callable through MCP
- **Validates:** BR-001
- Given `web.enabled: true` and a stub HTTP server / When `tools/call web.fetch {url: <stub>/page}` / Then the stub body is returned.

### 4.3 Non regression
#### BT-020: Enabled git and search unchanged
- **Protects:** BR-005, SPEC-0012, SPEC-0008
- Given `git.enabled` and `search.enabled` true / Then `tools/list` returns 112 names, and `git.init` then `git.status` on `atlas` succeed as today.

#### BT-021: Git HTTP and token screen gating unchanged
- **Protects:** `app.rs:155-167`
- Default configuration: `GET /git/atlas/info/refs?service=git-upload-pack` and `GET /app/tokens` answer 404, as today.

#### BT-022: Contract unchanged
- **Protects:** `tool-contract-golden.json`
- The golden contract tests pass unmodified; every served tool's name, description and input schema equal today's.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/mcp/server.rs:4128` | `tool_router_lists_exactly_the_100_contract_names` | Kept | asserts the static catalog, which is unchanged |
| `crates/core/src/tools/contract_golden.rs` | golden tests | Kept | names, descriptions, schemas unchanged |
| `crates/core/src/app.rs:505-690` | MCP transport tests (`tools/list` over a session) | Modified only if they assert a count | the served count follows the configuration |
| `crates/core/src/tools/all.rs:100` | `web_and_context7_tools_register_when_enabled` | Kept | test registry |
| tests calling git tools with `git.enabled` false | any | Modified: enable git in their configuration | they relied on the bypass |

## 5. Existing Specs Affected
None deviated. Restores SPEC-0006, SPEC-0008/FR-001, SPEC-0009/FR-001 and E2E-021, and SPEC-0012's
"git disabled" rule on the `rmcp` transport.

## 6. Assumptions
- **Assumption:** the error for a tool not served is today's unknown tool answer, `-32602 tool not
  found` (`app.rs:664-687`), matching SPEC-0009 E2E-021's "JSON-RPC unknown-tool error".
- **Assumption:** the configuration is fixed for the life of the process, so the served set is
  computed per server, not per call.
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS (flags, tool names, counts, error code and message) |
| 4 | No forced choice | PASS |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001 over MCP) |
| 7ante | Security misclassification | PASS (`internal finding`) |
| 7bis | Class sweep real | PASS (class without location, 3 searches, safe verdicts cited, exploit tests BT-005 and BT-006 with second channels: repository directory and index row, remote request log) |
| 8 | No hidden prerequisite | PASS |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (restores specified behavior; no new capability) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS |
| 13 | Writer and reader read | PASS (router `server.rs:82`, swagger operations `openapi.rs:219-233`, flags `config.rs:545`) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
