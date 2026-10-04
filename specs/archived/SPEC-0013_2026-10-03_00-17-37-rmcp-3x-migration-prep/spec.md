# Replace the hand-rolled MCP layer with rmcp 3.x — Technical Debt Specification

> Generated on: 2026-10-03
> Id: SPEC-0013
> Nature: DEBT
> Depth: L
> Depth evidence: 4 modules touched directly (`crates/core/src/mcp/{mod,registry,schema,args}.rs`) plus 3 integration points (`crates/core/src/app.rs`, `crates/core/src/token_screen.rs`, `crates/agent/src/mcp.rs`) plus the frozen contract artifacts (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`); 94 tool schemas re-emitted through a new builder; at least 2 declared, observable wire-format breaks (Section 5); comfortably over the ~15 anticipated-requirement budget once structural + invariance + compatibility + declared-break requirements are counted. Escalated from the default M guess once the rmcp 3.5.0 `StreamableHttpServerConfig` research (Section 2) showed the swap is not a drop-in replacement.
> Status: Implemented (2026-10-03, branch feat/SPEC-0013-rmcp-3x-migration-prep, 14/14 stories done)
> Behavior change: BREAKING on the MCP wire contract — see Section 5
> From backlog: n/a
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 0. How this document was produced

This is a DEBT specification produced by an automated pipeline run with no interactive user
available mid-run (a sub-agent execution of `/spec-debt`). Per the pipeline's own rules, every
question a live interview would have asked was answered from the context supplied by the
invoking task plus direct repository research, and is marked `Assumption:` where it is a judgment
call rather than a cited fact. No `Explore` sub-agents were spawned (the executing environment has
no sub-agent spawning primitive available to this agent); the Phase 0 occurrence sweep, the Phase
4.0 test design and the Phase 6 implementability audit were all performed directly by the same
agent that wrote the requirements, which is a deviation from the pipeline's intended
separation-of-authorship defence. This is recorded here rather than silently, and is itself the
subject of **DDEC-004** below.

## 1. Debt

### DBT-001: Hand-rolled MCP JSON-RPC/SSE layer, kept past the reason it was hand-rolled

**Current structure.** `crates/core/src/mcp/mod.rs:1-16` documents the original decision
(DEC-107): the project's C# reference server ran the ModelContextProtocol SDK with
`Stateless = true`, answering a bare `tools/call` with no `initialize` handshake and no session
header. The decision record claims `rmcp` 3.1 "always requires `initialize` first (even with
`NeverSessionManager`)", so a byte-identical port was judged impossible, and the team wrote its own
JSON-RPC 2.0 + SSE framing layer instead (`crates/core/src/mcp/mod.rs`, `registry.rs`, `schema.rs`,
`args.rs`, 860 lines total, counted at `wc -l`).

**Why it is debt.** The decision record is stale on its own terms. `rmcp` is now at 3.5.0 (not
3.1), and the 3.0.x line, released 2026-07-28, shipped exactly the capability DEC-107 said did not
exist: `rmcp::service::serve_directly` / `serve_directly_with_ct`, which "skip the initialization
process when starting a service" (docs.rs, confirmed via context7 query against
`/websites/rs_rmcp_rmcp`, 2026-10-03), and `StreamableHttpServerConfig` now carries a
`stateless_protocol_metadata_required: bool` field alongside `NeverSessionManager`, both absent
from the version DEC-107 was written against. A hand-rolled JSON-RPC/SSE/schema stack of 860 lines,
kept alive only by a decision whose premise no longer holds, is the exact shape of debt this nature
exists to name: duplicated protocol logic the ecosystem now provides, with the one thing blocking
removal being a stale fact.

**Target structure.** `crates/core/src/mcp/{mod,registry,schema,args}.rs` are replaced by a thin
adapter over `rmcp` 3.5.0: one `#[tool_router]`/`#[tool]` server type exposing the 94 tools
currently in `TOOL_CONTRACT.txt`, served over `rmcp::transport::streamable_http_server` with
`NeverSessionManager`, mounted at the same configurable path the current `mcp_path` config key
names (`crates/core/src/app.rs:115`). `crates/core/src/app.rs`'s `mcp_endpoint` handler, and the
`ToolRegistry`/`ToolSchema`/`Args` types it depends on, are deleted; the 94 tool registrations in
`crates/core/src/tools/*.rs` (`register(&mut ToolRegistry)` functions) are rewritten as `#[tool]`
methods, one per existing registration, preserving every existing `name`, `description`,
`inputSchema` and authorization call exactly as `TOOL_CONTRACT.txt` and
`tool-contract-golden.json` already freeze them (requirement DR-002 below).

**Observable behavior — this is where the "move the inside, outside holds still" rule breaks**,
and the full accounting is in Section 5. In outline: the JSON-RPC envelope, the error codes and the
tool dispatch semantics carry over unchanged; the **transport framing does not**. rmcp 3.x's
non-legacy streamable-HTTP mode defaults to `application/json` responses for a simple
request/response tool call and only falls back to `text/event-stream` when the handler emits a
notification before its final response (docs.rs, `StreamableHttpServerConfig::json_response`,
confirmed via context7, 2026-10-03). The current contract is SSE-always
(`crates/core/src/app.rs:13-14`, `crates/core/src/mcp/mod.rs:11-12`). This is declared as a break,
not absorbed as pure debt, per DDEC-001.

## 2. Perimeter — the Current State

### 2.1 Source occurrences

| File | Lines | What is there | Occurrences of the wire contract or dispatch layer |
|---|---|---|---|
| `crates/core/src/mcp/mod.rs` | 145 | Module doc recording DEC-107 (`1-16`); `sse_frame` (`42`); `rpc_result` (`45-52`, C# key order `result,id,jsonrpc`); `rpc_error` (`54-62`); `initialize_result` (`78-85`); 7 inline tests (`93-145`) asserting the exact frame string, the exact key order and the exact `initialize` shape | 16 |
| `crates/core/src/mcp/registry.rs` | 166 | `ToolRegistry`, `ToolHandler`, `ToolCtx`, dot/underscore tolerant name resolution, `handler()` helper; 3 inline tests | all 166 lines replaced |
| `crates/core/src/mcp/schema.rs` | 329 | `ToolSchema` declarative builder emitting the exact frozen `inputSchema` JSON; 6 inline tests pinning exact field order and `destructive`/`read_only` annotation defaults | all 329 lines replaced |
| `crates/core/src/mcp/args.rs` | 220 | `Args`, typed/tolerant argument accessors used by every one of the 94 tool handlers | all 220 lines replaced; the accessor API itself may or may not have an `rmcp` equivalent, see DRIFT-001 |
| `crates/core/src/app.rs` | file is larger; MCP-specific span `1-14` (module doc, wire contract summary), `17-33` (imports from `crate::mcp`), `114-115` (route registration), `225-294` (the `mcp_endpoint` handler: notification short-circuit, `initialize` dispatch, SSE response assembly), `320-470` (12 `#[tokio::test]` functions asserting exact headers, exact SSE prefix, exact JSON-RPC key order, the 401 unauthenticated shape and the unknown-tool error shape) | the whole MCP-facing slice of this file, 12 tests, is rewritten against whatever `rmcp`'s axum integration returns |
| `crates/core/src/token_screen.rs` | `520-540`, `847`, `923`, `1818` | 4 more tests sending raw `{"jsonrpc":"2.0",...}` bodies at the same endpoint, asserting the SSE-framed response, to prove the `/app/tokens` auth guard does not leak onto the MCP route | 4 |
| `crates/agent/src/mcp.rs` | 1-6 (module doc), `93-100` (request building, `Accept: application/json, text/event-stream`), `214-220` (2 unit tests parsing both a plain-JSON and an SSE-framed body) | the in-repo MCP **client** used by `./agent.sh`; its doc comment states explicitly "the server is stateless: it answers a bare `tools/list` or `tools/call` with no `initialize` handshake, so this client does not perform one" — this is the one in-repo consumer of the exact behavior DEC-107 protected |
| `crates/core/tests/full_stack_e2e.rs` | `156-181` (helper decoding the SSE frame literally), `464-466` | 1 helper + 1 assertion relying on the SSE envelope |

Total: **8 files**, **≈43 cited wire-contract-or-dispatch occurrences**, of which 860 lines
(`mcp/{mod,registry,schema,args}.rs`) are deleted outright and the rest are call sites or tests
that must be rewritten against the new transport's actual response shape.

### 2.2 Non-source occurrences

- `TOOL_CONTRACT.txt` (repository root): human-readable freeze of all 94 tool schemas and return
  shapes. Unaffected in content (the tools themselves do not change), but is the document
  `tool-contract-golden.json` is checked against and the one a migration must not silently drift
  from.
- `tool-contract-golden.json`: the machine-checked twin, exercised by
  `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`
  (`crates/core/src/tools/contract_golden.rs:15,34,139`). This test asserts the exact serialized
  `inputSchema` for all 94 tools; it is a structural guardrail this migration must keep green
  **without** the `MCPFS_REWRITE_TOOL_CONTRACT=1` escape hatch, or explicitly declare why it cannot
  (DR-005).
- No occurrence found in CI config (there is no `.github/workflows/` directory in this repo at the
  time of writing), `Makefile`, Dockerfile, or deployment manifests: grepped for `rmcp` and for
  `event: message` outside `crates/` and `specs/`, zero hits.
- `specs/archived/SPEC-0012_2026-09-26_16-04-00-rust-skill-compliance-debt/spec.md:36,44,479,481,594`:
  the prior compliance-debt spec explicitly excluded this same migration as **DDEC-003**, "not pure
  debt", citing the same DEC-107 premise this document now finds stale. That decision is not
  modified (Invariant 9); this document supersedes its conclusion going forward and says so in
  DDEC-002.
- `specs/SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md:21,373` and
  `specs/archived/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md:494`: both
  describe the hand-rolled layer as current, retro-specified fact. Neither is modified; both become
  stale in the sense that the layer they describe is gone after this migration, which is the normal
  fate of a retro-spec describing code that later changes.

### 2.3 Contracts touched

- **Public, external-facing**: the MCP wire contract itself — response `Content-Type`, SSE framing,
  exact JSON-RPC envelope shape, the `initialize` requirement — is served at the network boundary
  any MCP client (not only the in-repo agent) connects to. This is "public" in the sense Round 2
  requires: an HTTP route's response shape, even with zero known external consumers today
  (confirmed with the user before this run started).
- **Internal**: `ToolRegistry`/`ToolSchema`/`Args`/`ToolHandler` are called only from
  `crates/core/src/tools/*.rs` and `crates/core/src/app.rs`, all within this codebase. Free to
  rename or remove; no compatibility plan owed.

### 2.4 Guardrail coverage

Exact non-regression command, from `test.sh:5` and `Makefile` (`make test`):

```
cargo test --workspace
```

This is the command the invariance requirement (DR-001) is measured against, with the explicit
carve-outs in Section 5 and Section 6.4.

Existing tests directly covering the MCP wire contract: **23** (`mcp/mod.rs`: 7; `app.rs`: 12;
`token_screen.rs`: 4), all counted in Section 2.1. Existing tests covering the tool dispatch and
schema layer: 3 (`registry.rs`) + 6 (`schema.rs`) = 9. `full_stack_e2e.rs` adds 1 more SSE-literal
assertion against a real running server. Total guardrail: **33 tests** directly exercise the
surface this migration touches, which is why DR-004 below requires every one of them to have a
named verdict (kept unmodified, modified with reason, or removed with reason) rather than treating
"the suite is green" as sufficient on its own: a test that was asserting SSE framing and is
silently rewritten to assert JSON framing would stay green while quietly re-certifying a breaking
change as a non-event.

## 3. Requirements

### 3.1 Structural

- **DR-001** [EARS-U]: The MCP tool-serving layer SHALL be implemented using the `rmcp` crate,
  version `3.5.0` pinned in `[workspace.dependencies]`, added to `crates/core/Cargo.toml` with the
  `server` and `transport-streamable-http-server` features enabled and no others.
- **DR-002** [EARS-U]: Every tool currently registered via `register(&mut ToolRegistry)` in
  `crates/core/src/tools/*.rs` SHALL be re-expressed as one `#[tool]` method on a single server
  struct (name: `McpServer`, placed at `crates/core/src/mcp/server.rs`), preserving, for every one
  of the 94 tools frozen in `TOOL_CONTRACT.txt` and `tool-contract-golden.json`: the exact tool
  `name` string (dot-separated, e.g. `fs.read_bytes`), the exact `description` string, and an
  `inputSchema` that serializes identically to the one `tool-contract-golden.json` already records
  for that tool.
- **DR-003** [EARS-U]: `crates/core/src/mcp/mod.rs`, `registry.rs`, `schema.rs` and `args.rs` SHALL
  be deleted in full once DR-002 is complete; no `ToolRegistry`, `ToolSchema`, `Args` or
  `ToolHandler` symbol SHALL remain referenced outside test code documenting the old contract for
  historical comparison (none is required; none SHALL be added).
- **DR-004** [EARS-U]: Every one of the 33 existing tests enumerated in Section 2.4 SHALL carry an
  explicit verdict in this document's Section 6.4 before implementation starts: `unmodified`,
  `modified: <reason tied to a Section 5 declared break>`, or `removed: <reason>`. A test with no
  verdict blocks Phase 6 under G10 (undeclared behavior change).
- **DR-005** [EARS-U]: `tool-contract-golden.json` and the test
  `tool_contract_golden_is_current` (`crates/core/src/tools/contract_golden.rs:139`) SHALL remain
  green under the new `#[tool]`-based registration, without using the
  `MCPFS_REWRITE_TOOL_CONTRACT=1` regeneration escape hatch. If the `rmcp` `schemars`-derived
  schema for any one of the 94 tools cannot be made to serialize identically to the frozen golden
  entry, that tool's golden entry SHALL be regenerated deliberately, the diff SHALL be reviewed by
  a human, and the tool SHALL be listed by name in Section 5 as a declared break.

### 3.2 Invariance (MANDATORY)

- **DR-006** [EARS-U]: For every input the existing suite (`cargo test --workspace`) covers today
  that is NOT one of the declared breaks enumerated in Section 5, the system SHALL produce
  byte-identical observable output before and after this migration: the same HTTP status code, the
  same response headers (name and value), the same response body bytes.
- **DR-007** [EARS-UB]: The migration SHALL NOT change the behavior, name, authorization check or
  return shape of any individual tool (the 94 entries in `TOOL_CONTRACT.txt`). Only the transport
  and dispatch layer carrying those tools SHALL change.

### 3.3 Compatibility

- **DR-008** [EARS-U]: Because Round 1 confirms there is no external consumer of the current
  no-`initialize` wire behavior, and because that behavior cannot be preserved under `rmcp`'s
  stateless mode without also preserving the thing that cannot be preserved (Section 5, DDEC-001),
  the compatibility plan for the `initialize` requirement is: **break immediately, no dual
  maintenance window**, as explicitly authorized by DDEC-001.
- **DR-009** [EARS-E]: WHEN a client sends a bare `tools/call` or `tools/list` with no prior
  `initialize` request to the migrated server THE system SHALL respond with a JSON-RPC error whose
  `code` is `-32600` (Invalid Request) and whose `message` states that `initialize` must be called
  first, rather than silently hanging or crashing. This is the concrete, specified shape of the
  declared break: not "it changes somehow" but the exact response an un-migrated client receives.
- **DR-010** [EARS-E]: WHEN `crates/agent/src/mcp.rs` is built against the migrated server THE
  agent client SHALL perform an `initialize` request (and send `notifications/initialized`) before
  its first `tools/list` or `tools/call`, matching the sequence `rmcp`'s own client expects. This is
  the one in-repo consumer identified in Section 2.1 and it SHALL be updated in the same change,
  not left broken.

## 4. Compatibility Plan

| Public contract | Today | Dual-maintain or break | Plan |
|---|---|---|---|
| No-`initialize`-required `tools/call` | Any bare `tools/call`/`tools/list` answered directly | **Break immediately** (DDEC-001) | `initialize` becomes mandatory; a client skipping it gets the `-32600` error specified in DR-009, not silence. No deprecation window: there is no way to run both behaviors on one `rmcp` service, and the one identified in-repo consumer (`crates/agent`) is fixed in the same change (DR-010). |
| SSE-always response framing (`event: message\ndata: {json}\n\n` on every response) | Every response, success or error, is SSE-framed | **Break immediately** (DDEC-001) | `rmcp`'s non-legacy stateless mode returns `application/json` for a simple request/response tool call and only uses SSE when a notification precedes the final response (Section 2, cited). The migrated server advertises this via its actual `Content-Type` header; no client-visible SSE emulation is added on top, because doing so would reintroduce the hand-rolled framing this migration exists to remove. |
| JSON-RPC success/error envelope shape (`{"result":...,"id":N,"jsonrpc":"2.0"}` / `{"error":{"code":c,"message":m},"id":N,"jsonrpc":"2.0"}`) | Exact key order `result,id,jsonrpc` / `error,id,jsonrpc`, C# derived | **Dual-maintain is not applicable; this one is preserved, not broken** | `rmcp`'s `JsonRpcError`/response types serialize the same three top-level keys (`jsonrpc`, `id`, `result`/`error`), confirmed via context7 against `/websites/rs_rmcp_rmcp` (`model::JsonRpcError`, `model::ErrorData`). Key **order** is a `serde_json` struct-field-order artifact and is not guaranteed identical; DR-006's byte-identical bar therefore does not extend to key order, which is called out explicitly as its own narrow declared break (DDEC-003) rather than silently waived under the general SSE break. |
| `initialize` response shape (protocol version, server name, capabilities) | `crates/core/src/mcp/mod.rs:78-85`, pinned to `2024-11-05` | **Break, scoped** | `rmcp` advertises its own negotiated protocol version and `ServerInfo`/`ServerCapabilities` shape; DR-002 requires the tool list to stay identical, not the `initialize` envelope around it. Declared under DDEC-001. |

No contract in this table receives a deprecation window: Round 1's confirmation that no external
client depends on the current behavior removes the only reason dual maintenance would be the
default (per the shared specification body's own rule, dual maintenance is the default **unless**
breaking immediately is an explicit, logged user decision — DDEC-001 is that decision).

## 5. Declared Breaks

This migration is **not pure debt**. The investigation this document was built on (Section 2, DBT-001
"Observable behavior") found that `rmcp` 3.x's stateless HTTP mode changes two things a client can
observe, not one:

1. **`initialize` becomes mandatory.** DEC-107's narrower premise ("rmcp always requires
   initialize") turns out to still be true for the no-handshake case specifically: the new
   `serve_directly`/`serve_directly_with_ct` functions that looked, at first read of the 3.5.0
   changelog claim in this task's own briefing, like they removed the requirement, instead skip
   the **service-level** initialization handshake for in-process or stdio transports; they do not
   documented a path for the **streamable-HTTP** transport to answer a bare `tools/call` with no
   prior `initialize` over the wire. The HTTP-specific knob that does exist,
   `stateless_protocol_metadata_required`, is documented as requiring "stateless JSON-RPC requests
   to include specific protocol signals before handler dispatch" — i.e. even in the most permissive
   stateless HTTP configuration found, a request still has to carry protocol metadata the current
   contract never asks for. No configuration was found, across the context7-indexed `rmcp` 3.5.0
   docs, that reproduces "accept a bare `tools/call`, no `initialize`, no per-request metadata" over
   HTTP. **DDEC-001** (below) accepts this as a real, permanent break rather than continuing to
   chase a stateless-HTTP configuration that may not exist.

2. **The default response framing is JSON, not SSE.** Independently of the `initialize` question,
   `rmcp`'s non-legacy streamable-HTTP mode (`legacy_session_mode: false`, the mode this migration
   must use since `NeverSessionManager` only makes sense outside legacy per-session mode) returns
   `application/json` for an ordinary tool call and only switches to `text/event-stream` when the
   handler itself emits a notification ahead of the final response. The current contract is
   SSE-always, both for success and for error responses, and three test files (Section 2.1) assert
   that literally. This is a second, independent break, not a consequence of the first.

**DDEC-001.** Decision: accept both breaks above, immediately, with no dual-maintenance window, for
the reasons: (a) Round 1 confirmed no external client depends on either current behavior; (b)
reproducing either behavior on top of `rmcp` would mean writing a compatibility shim at the exact
layer this migration exists to delete, which defeats the point of the migration; (c) the one known
in-repo consumer (`crates/agent`) is updated in the same change (DR-010). Authorized per the task's
own framing: "if the migration ends up requiring `initialize`... breaking immediately rather than
dual-maintaining is an acceptable choice." Implemented by: DR-008, DR-009, DR-010, and every row of
Section 4's compatibility table marked "Break immediately".

**DDEC-002.** Decision: this document supersedes the conclusion of `specs/archived/SPEC-0012
..., DDEC-003` ("exclude the rmcp migration, not pure debt") without modifying that document
(Invariant 9). The prior exclusion was correct against `rmcp` 3.1 and the information available
2026-09-26; this document's Section 2 research, dated 2026-10-03, found a newer release line and
re-opens the question on fresh evidence. The two documents are not in conflict: they are dated
conclusions against different `rmcp` versions, and this one wins going forward because it is later
and because its Section 5 does what SPEC-0012 declined to do, which is name the breaks explicitly
instead of treating "any behavior change" as disqualifying.

**DDEC-003.** Decision: JSON-RPC envelope **key order** (`result,id,jsonrpc` vs whatever
`rmcp`'s struct field order produces) is declared as its own narrow break, separate from the SSE
framing break, because a consumer that parses the envelope with a JSON-order-sensitive tool (a
byte-diff test, not a JSON-aware client) would observe it independently of whether SSE or JSON
framing is in use. No JSON-RPC 2.0 client that parses JSON structurally (as any reasonable
implementation does) can observe key order, so this is flagged for completeness rather than
expected to matter in practice. Implemented by: DR-006 (the byte-identical bar explicitly excludes
this), Section 4 row 3.

**DDEC-004.** Decision: this document records, in Section 0, that it was produced without the
pipeline's intended separation between the agent authoring the requirements and the agent
performing Phase 0 occurrence discovery, Phase 4.0 test design and the Phase 6 implementability
audit, because the executing environment had no sub-agent spawning primitive available. This is
recorded as a process deviation, not hidden; it is the reason Section 9's gate pass is weaker
evidence than a fresh-context auditor's would be, and any implementer picking this spec up SHOULD
treat Section 9's `PASS` with that in mind rather than as equivalent to a cross-checked audit.

## 6. Tests

### 6.1 Non-regression command

```
cargo test --workspace
```

Must pass unmodified except for the 33 tests given an explicit verdict in 6.4, and the new tests in
6.2/6.3 must be added and green.

### 6.2 Compatibility tests

| Test | Validates | Given / When / Then |
|---|---|---|
| `DT-001` | DR-009 (the declared `initialize`-required break) | Given the migrated server running with no prior request on a fresh connection; When a client sends `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.glob","arguments":{}}}` with no prior `initialize`; Then the response is a JSON-RPC error with `error.code == -32600` and `error.message` containing the substring `"initialize"`, verified by parsing the response body as JSON (not by string match on framing, since the framing itself is under test elsewhere) |
| `DT-002` | DR-010 (the agent client is fixed) | Given `crates/agent/src/mcp.rs`'s client pointed at the migrated server; When the agent runs its normal startup sequence (the one exercised by `./agent.sh`'s own integration path); Then a `tools/list` call after that sequence succeeds and returns the 94-tool list, verified by asserting the returned tool count equals 94 and that `fs.read_bytes` is present by name |
| `DT-003` | Section 4 row 2 (JSON default framing) | Given the migrated server; When a client sends a well-formed `tools/call` for a side-effect-free tool (`admin.list_projects`) after a valid `initialize`; Then the response `Content-Type` header is `application/json` (not `text/event-stream`), verified by reading the header directly on the HTTP response, and the body parses as a bare JSON object (no `event:`/`data:` prefix) |
| `DT-004` | Section 4 row 3 (SSE still used when a notification precedes the response) | Given the migrated server and a tool implementation that emits a progress notification before returning (if none of the 94 existing tools do this today, this test is written against a test-only tool registered solely for this assertion, named `t.notifies_then_returns`, added under `#[cfg(test)]` only, never shipped); When that tool is called; Then the response `Content-Type` is `text/event-stream` and the body contains at least two `data:` frames |
| `DT-005` | DR-005 (golden contract survives without the regeneration flag) | Given the migrated server's `#[tool]` registrations; When `cargo test -p mcp-fs --lib tool_contract_golden_is_current` runs with `MCPFS_REWRITE_TOOL_CONTRACT` unset; Then the test passes, proving the 94 `inputSchema` values serialize identically to `tool-contract-golden.json` |

### 6.3 Structural tests

| Test | Validates | Given / When / Then |
|---|---|---|
| `DT-006` | DR-003 (old symbols fully removed) | Given the migrated codebase; When running `rg -n "ToolRegistry|ToolSchema::new|mcp::args::Args" crates/ --type rust`; Then the only matches are inside this document's own history (none expected in source), verified by the grep returning zero matches outside `target/` |
| `DT-007` | DR-002 (every one of the 94 tools is reachable) | Given the migrated server; When calling `tools/list` after `initialize`; Then the returned list has exactly 94 entries and the set of names is identical (as a set) to the 94 names currently listed in `TOOL_CONTRACT.txt`, verified by diffing the two name sets and asserting the diff is empty |
| `DT-008` | DR-006/DR-007 (no individual tool's behavior moved) | Given the migrated server; When re-running every existing tool-level test currently passing against the old dispatcher (the bulk of `crates/core/src/tools/*.rs`'s test modules) against the new one; Then every one is unmodified and green, which is the existing suite itself and is already covered by 6.1 — listed here only to make explicit that no new per-tool test is owed by this migration, because DR-007 forbids per-tool behavior change |

### 6.4 Existing tests to modify

The 33 tests from Section 2.4, each with a verdict. **This list is not empty**, because the break is
real; an empty list here would itself be a false claim this document exists to avoid.

| Test(s) | File | Verdict | Reason |
|---|---|---|---|
| 7 tests | `crates/core/src/mcp/mod.rs:93-145` | **removed** | The module and its `sse_frame`/`rpc_result`/`rpc_error`/`initialize_result` functions are deleted whole under DR-003; these tests exist only to pin that module's internals |
| 3 tests | `crates/core/src/mcp/registry.rs` | **removed** | `ToolRegistry` deleted under DR-003 |
| 6 tests | `crates/core/src/mcp/schema.rs` | **removed** | `ToolSchema` deleted under DR-003 |
| 12 tests | `crates/core/src/app.rs:320-470` | **modified: reflect DDEC-001** | Each currently asserts an exact SSE frame and/or the no-`initialize` success path; each is rewritten to assert the new, declared behavior (JSON `Content-Type` for a plain call, `-32600` for a missing `initialize`, per DT-001/DT-003) rather than deleted, because the route and its auth guard (the 401 unauthenticated case, `app.rs`'s one other MCP-adjacent assertion) still need coverage |
| 4 tests | `crates/core/src/token_screen.rs:520-540,847,923,1818` | **modified: reflect DDEC-001** | Same reason as the `app.rs` dozen: these exist to prove the token-screen auth guard does not leak onto the MCP route, which is still true after the migration, but the literal JSON-RPC body/response shape they send and parse changes |
| 2 tests | `crates/agent/src/mcp.rs:214,220` | **modified: reflect DR-010** | One parses a plain-JSON body (kept, now the common case rather than the fallback), one parses an SSE-framed body (kept, now the notification-path case instead of the default, repurposed to match DT-004's shape) |
| 2 occurrences | `crates/core/tests/full_stack_e2e.rs:156-181,464-466` | **modified: reflect DDEC-001** | The helper's SSE-stripping logic becomes conditional on the response `Content-Type`, matching the new default |

Count, against the Section 2.4 guardrail of 33: 7 (`mcp/mod.rs`) + 3 (`registry.rs`) + 6
(`schema.rs`) = **16 removed**; 12 (`app.rs`) + 4 (`token_screen.rs`) + 1 (`full_stack_e2e.rs`) =
**17 modified**. 16 + 17 = 33, matching Section 2.4 exactly. The 2 `crates/agent/src/mcp.rs` tests
are additional to that 33 (Section 2.1 lists them separately, as the one in-repo client, not as
part of the server-side wire-contract guardrail) and are also **modified**, bringing the total
across both counts to 16 removed + 19 modified = 35 test sites touched. 0 left untouched that
touch the wire contract directly, which is consistent with Section 5: this is a declared-break
migration, not a zero-behavior-change refactor, and the test ledger says so plainly rather than by
omission.

## 7. Implementation Order

1. **DR-001**: add the `rmcp` 3.5.0 dependency, `server` + `transport-streamable-http-server`
   features only, to `[workspace.dependencies]` and `crates/core/Cargo.toml`. Build must succeed
   with zero other code changed (proves the dependency itself is clean before anything else moves).
2. **DR-002 + DR-005**: write `crates/core/src/mcp/server.rs` with all 94 `#[tool]` methods,
   calling the exact same `core::fs_ops`/authorization code the old handlers called (no logic
   moves, only the dispatch wrapper). Run `DT-007` and `DT-005` before touching the transport: this
   proves the tool surface is intact while the old transport is still live, so a failure here is
   isolated from the transport swap.
3. **DR-008/DR-009/DR-010 (Section 5 breaks)**: swap `crates/core/src/app.rs`'s route registration
   from the hand-rolled `mcp_endpoint` to `rmcp`'s `StreamableHttpService` with
   `NeverSessionManager`, mounted at the existing `mcp_path`. Update `crates/agent/src/mcp.rs` in
   the same step (DR-010), because step 4's test suite exercises the agent against the new server.
4. **DR-003**: delete `crates/core/src/mcp/{mod,registry,schema,args}.rs` only once step 3's tests
   (`DT-001` through `DT-004`) are green against the new transport; deleting first would leave the
   build broken with no way to bisect a transport bug from a removed-code bug.
5. Apply every verdict in Section 6.4, in the same change: tests cannot be left asserting the old
   framing once the server no longer produces it, and DR-004 forbids leaving any of the 33 without
   a verdict.
6. Run `cargo test --workspace` (Section 6.1) and confirm the only failures before the fix were the
   ones Section 6.4 names; confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo fmt --all -- --check` are clean.

Order rationale: tool surface (step 2) before transport (step 3) before deletion (step 4), because
each step's tests gate the next and a failure at step 3 must not be confused with a failure at step
2 that deletion in step 4 would make unbisectable.

## 8. Decisions & Assumptions

- **Assumption A1**: the task briefing's claim that "rmcp was never added as an actual dependency
  in this repo" and "the 3.0.x line... targets a spec revision with sessionless HTTP / no required
  initialize" was treated as a lead to verify, not as settled fact, per the task's own instruction.
  Verification (Section 5) found the sessionless-HTTP claim does not hold at the wire level for
  streamable HTTP the way the briefing's high-level framing suggested; the service-level
  `serve_directly` bypass exists but is documented for non-HTTP transports in the evidence gathered.
  Flagged rather than assumed correct.
- **Assumption A2**: "no existing external clients depend on the current no-initialize wire
  behavior" is taken as given, per the task's statement that this was already confirmed with the
  user. This document does not re-derive it.
- **Assumption A3**: Round 1's "why now" (the concrete cost) is the staleness of DEC-107 itself
  (Section 1) plus the fact that SPEC-0012's exclusion (DDEC-003 there) is now the single largest
  tracked compliance gap in this codebase per that document's own §"Rust skill compliance debt"
  closing note (`spec.md:594`, cited). No further "why now" was asked because no interactive user
  was available; this is recorded as the answer this document proceeds on.
- **Decision D1**: the migration is scoped to the MCP transport and dispatch layer only. It does
  **not** touch `core::fs_ops`, `core::storage`, or any of the 94 tools' actual logic (DR-007). This
  keeps the lot bounded to what Section 2 perimeters, rather than growing into a second refactor.
- **Decision D2**: no feature flag or dual-build (old layer + new layer side by side) is introduced.
  Round 1's confirmation (A2) removes the reason a flag would exist, and a flag that is never
  flipped in production is itself debt (YAGNI, Invariant 17).
- **Backlog**: none raised. Every idea surfaced during this run (the test-only notification-emitting
  tool for `DT-004`, the key-order question in DDEC-003) is small enough to fold directly into this
  spec's own tests/decisions rather than deferred.

## 9. Implementability Gate

Run as the Phase 6 **inline checklist** (6.4 of the shared body), because Section 0 already
discloses that no independent auditor sub-agent was available in this run; running the depth-L
auditor procedure with the same agent that wrote the requirements would not produce the
separation-of-authorship benefit that procedure exists for. This substitution is itself flagged
(DDEC-004) rather than silently presented as equivalent.

| # | Check | Verdict |
|---|---|---|
| 1 | No orphan decision | PASS — DDEC-001 → DR-008/009/010; DDEC-003 → DR-006 exclusion, Section 4 row 3 |
| 2 | No buried change | PASS — every code change (dependency add, server rewrite, transport swap, deletion, test ledger) owns a numbered DR or a Section 6.4 row |
| 3 | Every name spelled | PASS — tool count (94), file names, struct name (`McpServer`), test ids, error code (`-32600`), feature flags (`server`, `transport-streamable-http-server`), version (`3.5.0`) all literal |
| 4 | No forced choice | PASS — framing default (JSON vs SSE) is rmcp's documented behavior, not an open choice; key order explicitly excluded from the invariance bar (DDEC-003) rather than left ambiguous |
| 5 | Order stated | PASS — Section 7, with rationale for each ordering constraint |
| 6 | Every test specified | PASS — DT-001 through DT-008 have concrete Given/When/Then; Section 6.4's 33 existing tests each have a named verdict |
| 7 | No out-of-spec prerequisite | PASS — the one new test-only tool (`t.notifies_then_returns`) is specified in DT-004 itself, not assumed to pre-exist |
| 8 | EARS clean | PASS — all DR-* use `[EARS-U]`, `[EARS-E]` or `[EARS-UB]`; no forbidden modal found on review |
| 9 | Right tool (DEBT: behavior unchanged or every break declared) | PASS, **with the break declared rather than absent** — Section 5 names exactly two breaks and Section 4 maps every touched contract to one of them; DR-007 forbids any break beyond those named |
| 10 | Length within budget | Document runs long for depth L's "as needed" budget by design: a declared-break DEBT spec with a 33-test ledger cannot compress further without losing a citation, and the drift register (§10) is excluded from the budget by rule |
| 11 | Every code claim cited | PASS, with one caveat — every file:line citation in Sections 1, 2 and 5 was opened directly during this run (shown in the tool-call transcript this document was produced from); the `rmcp` API claims are cited to context7-retrieved docs.rs pages rather than to the crate's own source, which is weaker evidence than reading the published source directly. Recorded as **DDRIFT-001** below rather than silently accepted |
| 12 | No requirement rests on a missing capability | PASS — DR-009's `-32600` relies on `rmcp` surfacing a JSON-RPC error for a pre-`initialize` request, which is standard JSON-RPC 2.0 behavior `rmcp` is built on top of; not independently verified against `rmcp` source in this run (same caveat as check 11) |
| 13 | Security misclassification | PASS — this document describes no authn/authz bypass, injection, data exposure, deserialization/traversal flaw, or advisory-driven bump; `Security: n/a` stands |
| 14 | `--security` | n/a — not invoked |
| 15 | DEBT perimeter exhaustive, renames both-sided, contracts planned, non-regression literal | PASS — Section 2 lists non-source occurrences (none found outside `specs/`) and the perimeter is file-and-line exhaustive for source; there is no rename in this migration (DR-002 preserves every tool name), so "both sides spelled" is not applicable and is noted as such here rather than silently skipped; Section 4 plans every touched contract; Section 6.1's command is copy-pasteable |

**Verdict: IMPLEMENTABLE-WITH-DRIFT.**

One A finding, registered rather than chased, per check 11/12's caveat:

## 10. Drift Register

| DDRIFT-001 | The `rmcp` 3.5.0 API claims in Sections 1, 2 and 5 (`stateless_protocol_metadata_required`'s exact semantics, `serve_directly`'s exact transport applicability, `json_response`'s exact fallback trigger) are sourced from context7-retrieved docs.rs documentation, not from reading `rmcp`'s published source directly, because no sub-agent or second verification pass was available in this run (Section 0, DDEC-004) |
| Spec says | Section 5, point 1 and 2: streamable-HTTP cannot answer a bare `tools/call` with no `initialize` and no per-request metadata under any 3.5.0 configuration found; the non-legacy mode defaults to JSON and falls back to SSE only on a preceding notification |
| Code does | Not yet applicable — `rmcp` is not a dependency of this repository yet (Cargo.lock has zero `rmcp` entries, confirmed by direct grep during this run); there is no "code" to check this claim against until DR-001 lands the dependency |
| Nature | missing capability (possibly) / false statement (possibly) — cannot be told apart until the dependency is actually added and its real source read |
| Resolution during implementation | At DR-001 (Section 7, step 1), before writing a single `#[tool]` method, read `rmcp` 3.5.0's actual source (via `cargo doc --open -p rmcp` or the vendored `~/.cargo/registry` copy) for `StreamableHttpServerConfig`, `NeverSessionManager`, and the streamable-HTTP request-dispatch path, and confirm or correct every claim in Section 5 before proceeding to step 2. If the claims are wrong in the direction of "a no-initialize HTTP path does exist", DDEC-001 is void and this document must be amended before Section 7 continues, because the whole declared-break premise rests on it |
| Detected by | `cargo build` after DR-001 lands the dependency will make the real API surface inspectable via `cargo doc`; the implementer's own reading of that output, required explicitly by the "Resolution" cell above, is what detects a wrong claim. There is no automated test for this, because it is a claim about a library's documented behavior, not about this codebase's own code — flagged explicitly rather than left to "the compiler will catch it", since the compiler accepts any transport configuration that type-checks whether or not it reproduces the contract this document assumes |
| Status | resolved (2026-10-03, implementation run) — the claim was CONTRADICTED, not confirmed: reading `rmcp` 3.5.0's actual vendored source (US-0001) found the spec's planned `NeverSessionManager`+`legacy_session_mode: false` pairing produces NO `initialize` requirement at all, the opposite of Section 5's premise. User-decided correction (`LocalSessionManager`+`legacy_session_mode: true`) restores the mandatory-`initialize` intent but with real wire shapes DR-009/DT-001/DT-003 didn't anticipate (empirically verified in US-0008, second attempt). Full record: `drift/2026-10-03_00-38-28.md`, `drift/2026-10-03_02-50-09.md`. Implemented and shipped in commit `fbc193d`. |
