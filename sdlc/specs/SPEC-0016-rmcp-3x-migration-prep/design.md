> Id: SPEC-0016
> Nature: DEBT
> Status: implemented
> Migrated from: specs/archived/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0013; new id SPEC-0016 (collision avoidance, SPEC-0013 is used by trash-listing-and-recovery)

# Replace the hand-rolled MCP layer with rmcp 3.x

## Confidence note

This document is DEBT nature with design.md only, per the legacy spec's own front matter
(`Nature: DEBT`, not FEAT). The migration swaps the MCP transport/dispatch layer from a
860-line hand-rolled JSON-RPC/SSE stack to the official `rmcp` 3.5.0 crate. It preserves every
one of the 94 (now 102, +4 search.* in the current contract) tool names, descriptions,
`inputSchema` shapes and authorization/engine calls (DR-002, DR-006, DR-007). Two transport-level
protocol breaks are declared and accepted (DDEC-001, renumbered DEC-001 below): `initialize`
becomes mandatory, and two narrower error-path message/status differences surface. No new
user-observable product behavior was added or removed; this is why the legacy spec already
self-classified as DEBT rather than FEAT, and why this migration note carries only a design.md.

Confidence: high on content fidelity to the legacy spec.md and its 7 drift files (all read in
full); medium on whether every drift file's "resolved" status fully matches the live code today,
since this document is a migration of historical record, not a fresh code audit (the main
AGENTS.md of this repo independently confirms "rmcp 3.5.0" and "the one production MCP
transport" are live in production, which corroborates the terminal state recorded here).

## 1. Components

- **`crates/core/src/mcp/server.rs`** — `McpServer`, the single `#[tool_router]`/`#[tool]`
  struct exposing one method per tool in `TOOL_CONTRACT.txt`. Each method calls the same
  `core::fs_ops`/engine function the REST data plane calls (never a second implementation).
  Replaces the deleted hand-rolled dispatch (`mcp/{mod,registry,schema,args}.rs`, 860 lines).
- **`rmcp` 3.5.0** — the official Rust MCP SDK, added to `[workspace.dependencies]` and
  `crates/core/Cargo.toml` with only the `server` and `transport-streamable-http-server`
  features (DR-001). Supplies `StreamableHttpService`, `LocalSessionManager`, the
  `#[tool_router]`/`#[tool]` macros, and the JSON-RPC envelope types.
- **`crates/core/src/app.rs`** — mounts the MCP route as an `rmcp` `StreamableHttpService`
  wrapping `McpServer`, configured with `LocalSessionManager` + `legacy_session_mode: true`
  (corrected pairing, see Decisions). Replaces the old hand-rolled `mcp_endpoint` handler.
- **`tools/registry_support`** (formerly `mcp::{registry,schema,args}`) — what remains of the
  old machinery after the full decoupling (US-0010 through US-0014): `ToolCtx`/`Args`/
  `ToolRegistry`/`ToolSchema` survive only as `#[cfg(test)]`-gated glue for the shared test
  harness, and as genuine production code backing `catalog()` for the five optional, non-contract
  tool families (`web`/`context7`/`sqlite`/`db`/`doc`), read by `/api/swagger.json`.
- **`crates/agent/src/mcp.rs`** — the in-repo MCP client (`./agent.sh`). Updated in the same
  change (DR-010) to perform `initialize` + `notifications/initialized` before its first
  `tools/list`/`tools/call`, since the migrated server now requires it.
- **`api/openapi.rs`** — the `/api/swagger.json` catalog. Rewired (US-0013, discovered scope) onto
  `McpServer`'s rmcp tool router plus the new lightweight catalogs for the five optional families,
  replacing the old `AppState.registry`/`register_all`/`EnabledFeatures` wiring.
- **`TOOL_CONTRACT.txt` / `tool-contract-golden.json`** — unchanged in content; the golden test
  stays green under the new `#[tool]`-based registration without the
  `MCPFS_REWRITE_TOOL_CONTRACT=1` escape hatch (DR-005).

## 2. Flows

### 2.1 Tool call, pre-migration (deleted)
Client sends a bare `tools/call` JSON-RPC body, no `initialize` → hand-rolled `mcp_endpoint`
dispatches by name through `ToolRegistry` → response is always SSE-framed
(`event: message\ndata: {...}\n\n`), success or error, exact key order `result,id,jsonrpc`.

### 2.2 Tool call, post-migration (shipped)
1. Client opens a session and sends `initialize` first. Skipping this step gets an HTTP `422`
   with a plain-text body containing `"initialize"` (`"Unexpected message, expect initialize
   request"`) — not the JSON-RPC `-32600` envelope originally imagined in the legacy spec's
   DR-009, corrected via empirical testing (see Decisions, DEC-003).
2. Once a session is established, every subsequent tool call (`tools/call`) is dispatched through
   `McpServer`'s `ToolRouter`, which calls straight into the same `core::fs_ops`/engine functions
   the REST plane uses.
3. Response framing for an established session stays `text/event-stream` (SSE) — the
   `legacy_session_mode: true` pairing that restores mandatory `initialize` makes the SSE branch
   the only reachable one; the "JSON becomes default" break originally declared in the legacy
   spec's Section 4 row 2 never materializes in shipped behavior (DEC-003).
4. Unknown tool name → `INVALID_PARAMS`/`-32602` (code preserved), message text changes to
   rmcp's own `"tool not found"` (was `"Unknown tool: '<name>'"`).
5. Malformed JSON-RPC body → HTTP `415` plain-text body containing `"deserialize"` (was HTTP 500
   with a hand-built JSON error object).
6. A spec-defined method this server never advertises resources for (e.g. `resources/list`) gets
   rmcp's own empty-list default, handled at rmcp's protocol layer before `ToolRouter` ever sees
   it, rather than this server's old `-32601` method-not-found error.

### 2.3 git.*/git.auth*/git.pr_* dispatch (final shape)
All 49 git-family `#[tool]` methods call `pub(crate)` free functions directly with typed
`Parameters<T>` fields (promoted from private handlers in `tools/{git,git_auth,git_pr}.rs`),
matching the pattern `fs.*`/`admin.*`/`search.*` already use via `core::fs_ops`. The intermediate
state where these methods re-entered `ToolRegistry`/`Args` internally (a real discovered gap,
DEC-005/DEC-007 below) was closed by the Phase 4.6 converge audit before final delivery.

## 3. Interfaces

- **MCP wire contract**: JSON-RPC 2.0 over streamable HTTP, mounted at the configurable `mcp_path`
  (unchanged route). `initialize` is mandatory (restored intent, real mechanism corrected). Error
  codes for programmatic handling (`INVALID_PARAMS`/`-32602`) are preserved; human-readable
  message text and some HTTP status codes for protocol-level rejections differ from pre-migration.
- **Tool surface**: 102 tools (39 `fs.*`, 14 `admin.*`, 39 `git.*`, 4 `git.auth*`, 6 `git.pr_*`, +4
  `search.*`), names/descriptions/`inputSchema` byte-identical to `TOOL_CONTRACT.txt` and
  `tool-contract-golden.json` (DR-002, DR-005, DR-007).
- **`/api/swagger.json`**: unaffected in output shape; its backing source moved from
  `AppState.registry.list_payload()` to `McpServer::tool_router().list_all()` for the 94/102
  contract tools, plus lightweight catalogs for the five optional families.
- **`crates/agent`'s MCP client**: now performs the standard `initialize` + `notifications/
  initialized` handshake; no longer documents itself as talking to a stateless, handshake-free
  server.

## 4. Data and state

No persisted data model changes. The migration is transport/dispatch only (Decision D1/DEC-005):
it does not touch `core::fs_ops`, `core::storage`, or any of the 94/102 tools' actual logic.
Session state is now held by `rmcp`'s `LocalSessionManager` (in-memory, per-connection) rather
than being implicitly stateless; this is a new in-process state container introduced purely by
the transport swap, not a persisted store.

## 5. Configuration

- `[workspace.dependencies]` / `crates/core/Cargo.toml`: `rmcp = "3.5.0"`, features `server` +
  `transport-streamable-http-server` only (DR-001).
- Existing `mcp_path` config key: unchanged, still names the mount point for the MCP route.
- Session manager: `LocalSessionManager` + `legacy_session_mode: true` (corrected pairing; the
  originally planned `NeverSessionManager` + `legacy_session_mode: false` would have produced the
  opposite of the intended mandatory-`initialize` behavior — see Decisions, DEC-002).
- `MCPFS_REWRITE_TOOL_CONTRACT=1`: unchanged escape hatch for regenerating the golden contract;
  not needed to keep the suite green after this migration (DR-005 holds without it).

## 6. Observability

No new observability surface was introduced by this migration beyond what already existed
(tracing on tool dispatch, error logging). The error-path message/status differences (Flow 2.2
steps 1, 4, 5, 6) change what a client or a log line sees as literal text/status for protocol-
level failures, which is worth knowing when searching historical logs across the migration
boundary: a pre-migration `"Unknown tool: '<name>'"` string search will not match post-migration
`"tool not found"` logs for the same condition.

## 7. Decisions

- **DEC-001** (was DDEC-001): Accept both declared breaks (mandatory `initialize`; framing
  default) immediately, no dual-maintenance window. Reasons: no external client depended on
  either prior behavior (confirmed with user); a compatibility shim would defeat the point of the
  migration; the one in-repo consumer (`crates/agent`) was updated in the same change.
- **DEC-002** (was DDEC-002): This document (and its legacy spec) supersedes the conclusion of
  the prior exclusion decision (`specs/archived/SPEC-0012`'s DDEC-003, "exclude the rmcp
  migration, not pure debt") without modifying that document. The exclusion was correct against
  `rmcp` 3.1 and 2026-09-26 evidence; this migration's research found a newer release line and
  re-opened the question on fresh evidence, naming the breaks explicitly rather than treating any
  behavior change as disqualifying.
- **DEC-003** (merges DDEC-003 + the two empirical corrections in drift/2026-10-03_00-38-28.md
  and drift/2026-10-03_02-50-09.md): JSON-RPC envelope key order is a declared, narrow,
  practically-inert break (no structural JSON parser can observe it). More materially: the
  spec's originally planned session-manager pairing (`NeverSessionManager` +
  `legacy_session_mode: false`) was found, by reading rmcp 3.5.0's actual vendored source, to
  produce the OPPOSITE of the intended mandatory-`initialize` behavior. Corrected to
  `LocalSessionManager` + `legacy_session_mode: true`, which restores mandatory `initialize` as
  real behavior but makes the exact rejection shape an HTTP `422` + `"initialize"` substring
  (not a JSON-RPC `-32600` envelope) and keeps response framing SSE for an established session
  (so the "JSON becomes default" framing break from the original compatibility table never
  materializes in shipped behavior).
- **DEC-004** (was DDEC-004): The implementation run that produced the legacy spec disclosed it
  lacked the pipeline's intended separation between the requirements author and the Phase 0/4.0/6
  auditor sub-agent, because no sub-agent spawning primitive was available. Recorded as a process
  deviation, not hidden; later caught and corrected by an independent Phase 4.6 converge audit
  (see DEC-007 below), which validates that the disclosed weaker-evidence gate was in fact worth
  flagging.
- **DEC-005** (was Decision D1): Migration scoped to the MCP transport and dispatch layer only;
  does not touch `core::fs_ops`, `core::storage`, or any tool's actual logic (DR-007).
- **DEC-006** (was Decision D2): No feature flag or dual-build (old layer + new layer side by
  side) was introduced; a flag never flipped in production is itself debt, and Assumption A2
  (no external client depends on current behavior) removes the only reason a flag would exist.
- **DEC-007** (merges the git.*-family drift corrections, drift/2026-10-03_02-05-00.md and
  drift/2026-10-04_00-37-37.md): The first attempt at migrating the 49 `git.*`/`git.auth*`/
  `git.pr_*` tools dispatched internally through a live `Arc<ToolRegistry>` instead of calling
  engine functions directly, which would have blocked DR-003's "no `ToolRegistry`/`Args` symbol
  outside test code" bar. A later independent Phase 4.6 converge audit found this gap still open
  after all 14 stories were marked done (49 functions still typed `(ctx: ToolCtx, a: Args)`), and
  closed it: all 49 converted to typed `Parameters<T>` fields calling promoted `pub(crate)`
  functions directly, net -357 lines in `server.rs`, full suite (2102 tests) still green. `ToolCtx`
  itself was confirmed NOT forbidden by DR-003's literal text and was kept as the state/person
  carrier.
- **DEC-008** (merges drift/2026-10-03_04-05-29.md and the US-0010 through US-0014 stories it
  spawned): The legacy spec's Section 2.3 claim that `ToolRegistry`/`ToolSchema` were "free to
  remove, called only from `tools/*.rs` and `app.rs`" was found false during the originally-scoped
  US-0009: the true blast radius was ~20 files (not 4) and ~32,000 lines (not ~860), because
  `register_all` populated `AppState.registry` for every tool family (not only MCP-facing ones)
  and `api/openapi.rs`'s `/api/swagger.json` catalog read directly from it — a live production
  path the legacy spec's Section 2 perimeter never mentioned. User authorized expanded scope
  (US-0010 wean fs./admin./search.; US-0011 wean git family; US-0012 catalog-only for optional
  families; US-0013 rewire openapi.rs; re-run US-0009 to actually delete). All completed (stories
  index marks all "done").
- **DEC-009** (was drift/2026-10-03_03-52-49.md's finding): Two additional, undeclared error-path
  deviations were accepted as low-severity and left as-is rather than reconciled back to old
  behavior: unknown-tool message text (`"tool not found"` replacing `"Unknown tool: '<name>'"`,
  code unchanged) and malformed-body handling (HTTP 415 + `"deserialize"` substring, replacing
  HTTP 500 + hand-built JSON, at the decode step before any JSON-RPC envelope exists). No known
  in-repo consumer depends on the old text/status for either path.

## 8. Requirement to code map

| Requirement | Code |
|---|---|
| DR-001 (add rmcp 3.5.0, `server`+`transport-streamable-http-server` only) | `Cargo.toml` workspace deps, `crates/core/Cargo.toml` |
| DR-002 (every tool as a `#[tool]` method, names/descriptions/schemas preserved) | `crates/core/src/mcp/server.rs` |
| DR-003 (old `mcp/{mod,registry,schema,args}.rs` deleted; no `ToolRegistry`/`ToolSchema`/`Args`/`ToolHandler` outside test code) | deletion done via US-0009 (re-run) after US-0010..US-0014 decoupling; `tools::registry_support` carries the surviving `#[cfg(test)]`-gated + optional-family-catalog remainder |
| DR-004 (every one of the 33 existing wire-contract tests given an explicit verdict) | legacy spec Section 6.4; carried into §9 below |
| DR-005 (golden contract green without the rewrite escape hatch) | `crates/core/src/tools/contract_golden.rs` |
| DR-006 (byte-identical output for every non-declared-break input) | full `cargo test --workspace`, DT-005/DT-007 |
| DR-007 (no individual tool's behavior/name/auth/return shape changes) | `core::fs_ops` calls unchanged; enforced by the full existing per-tool test suite |
| DR-008 (break-immediately compatibility plan, no dual-maintenance window) | `crates/core/src/app.rs` route swap |
| DR-009 (missing-`initialize` rejection) | corrected shape: HTTP 422 + `"initialize"` substring (DEC-003), `app.rs` tests |
| DR-010 (agent client performs `initialize` handshake) | `crates/agent/src/mcp.rs` |

## 9. Legacy mapping

Source: specs/archived/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md (pre-move, renumbered to SPEC-0016); plus its drift/ directory (7 files)

| Old id | New id | Note |
|---|---|---|
| DBT-001 | §1 Components / §2 Flows | Hand-rolled MCP layer replaced by `rmcp` 3.5.0 adapter |
| DR-001..DR-010 | §8 Requirement to code map | Ids kept verbatim |
| DDEC-001 | DEC-001 | Accept both declared breaks immediately, no dual-maintenance |
| DDEC-002 | DEC-002 | Supersedes SPEC-0012's prior exclusion (not modified in place) |
| DDEC-003 | DEC-003 | Key order break; merged with the two empirical session-manager corrections below |
| DDEC-004 | DEC-004 | Process-deviation disclosure (no separate auditor sub-agent in the authoring run) |
| Decision D1 | DEC-005 | Scope bounded to transport/dispatch layer only |
| Decision D2 | DEC-006 | No feature flag / dual-build introduced |
| DDRIFT-001 (register entry) | §9 row below, resolved via US-0001-verification-note.md | Claims 2 and 3 confirmed verbatim against vendored source; claim 1 contradicted, see next row |
| drift/2026-10-03_00-38-28.md (session manager voids DDEC-001 premise) | DEC-003 | Resolved: `NeverSessionManager`+`false` would have produced the OPPOSITE of mandatory-`initialize`; corrected to `LocalSessionManager`+`legacy_session_mode: true` |
| drift/2026-10-03_02-05-00.md (git.* tools depend on ToolRegistry internally) | DEC-007 | Resolved: closed by the Phase 4.6 converge audit (drift/2026-10-04_00-37-37.md), all 49 functions converted to typed `Parameters<T>` |
| drift/2026-10-03_02-50-09.md (DT-001/DT-003 exact shapes unreachable under corrected config) | DEC-003 | Resolved: DT-001 relaxed to HTTP 4xx + `"initialize"` substring; DT-003 relaxed to SSE-stays-default for an established session |
| drift/2026-10-03_03-52-49.md (two undeclared error-path deviations) | DEC-009 | Resolved/accepted as low-severity, documented, no code change required |
| drift/2026-10-03_04-05-29.md (Section 2.3 claim false, ~20-file blast radius) | DEC-008 | Resolved via expanded scope: US-0010, US-0011, US-0012, US-0013, re-run US-0009 (all done per stories index) |
| drift/2026-10-04_00-37-37.md (Phase 4.6 converge: Args still used in production for git.* family) | DEC-007 | Resolved: all 49 functions converted; `ToolCtx` confirmed compliant, not forbidden |
| US-0001-verification-note.md | DDRIFT-001 row above | Resolved: proceeded to US-0002 carrying the session-manager correction forward |

## FINDINGS FOR BACKLOG

- **drift/2026-10-03_13-03-02.md** (optional `web.*`/`context7.*`/`sqlite.*`/`db.*`/`doc.*` tool
  families become unreachable via MCP after the transport swap): explicitly left open by the
  implementing run. All five families are off by default (no evidence of a production deployment
  enabling them), but if any deployment ever sets `--web`/`--context7`/`--sqlite`/`--db`, those
  tools are now silently unreachable over MCP — a real, undeclared gap not covered by any DR-006
  exclusion. Resolution requires either a dedicated follow-up spec building `#[tool]` methods for
  these optional families on `McpServer`, or an explicit decision that MCP access to them is no
  longer offered going forward. Not resolved by this migration; carried forward as backlog.
