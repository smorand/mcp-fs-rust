# US-0008: Transport swap — rmcp StreamableHttpService + agent client + declared-break tests

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 8
> Depends On: US-0007
> Complexity: L
> min_tier: 2
> Files touched: 4

## Objective
Swap `crates/core/src/app.rs`'s route registration from the hand-rolled `mcp_endpoint` to `rmcp`'s
`StreamableHttpService` with `NeverSessionManager`, mounted at the existing `mcp_path`. Update
`crates/agent/src/mcp.rs` (the in-repo MCP client) in the same step so it performs the `initialize`
handshake the new server requires. This is where the two declared breaks (Section 5) become real
and observable: `initialize` becomes mandatory, and the default response framing becomes JSON
instead of SSE-always.

## Technical Context

### Stack
Rust 2024, axum, `rmcp` 3.5.0 (`transport::streamable_http_server`, `NeverSessionManager`,
`StreamableHttpServerConfig`).

### Relevant File Structure
```
crates/core/src/app.rs                    # route registration swap, lines 1-14, 17-33, 114-115, 225-294, 320-470 (tests)
crates/agent/src/mcp.rs                   # request building (93-100), module doc (1-6), tests (214-220)
crates/core/src/token_screen.rs           # 4 tests (520-540, 847, 923, 1818) asserting auth guard does not leak onto MCP route
crates/core/tests/full_stack_e2e.rs       # SSE-literal helper (156-181) and assertion (464-466)
```

### Existing Patterns
Read `crates/core/src/app.rs:225-294` (the current `mcp_endpoint` handler) before writing the
replacement — it shows the notification short-circuit, `initialize` dispatch, and SSE response
assembly that `rmcp`'s `StreamableHttpService` now owns instead. Read
`crates/agent/src/mcp.rs:93-100` (request building with `Accept: application/json,
text/event-stream`) and its module doc, which currently states explicitly "the server is
stateless... this client does not perform [an initialize handshake]" — that doc comment becomes
false after this story and must be corrected as part of the same change, not left stale.

### Decisions That Govern This Story
- **DDEC-001** (spec Section 5): "accept both breaks above, immediately, with no dual-maintenance
  window... (c) the one known in-repo consumer (`crates/agent`) is updated in the same change
  (DR-010)."
- **DR-008** (spec Section 3.3): "the compatibility plan for the `initialize` requirement is:
  break immediately, no dual maintenance window, as explicitly authorized by DDEC-001."
- **DR-009** (spec Section 3.3): "WHEN a client sends a bare `tools/call` or `tools/list` with no
  prior `initialize` request to the migrated server THE system SHALL respond with a JSON-RPC error
  whose `code` is `-32600` (Invalid Request) and whose `message` states that `initialize` must be
  called first, rather than silently hanging or crashing."
- **DR-010** (spec Section 3.3): "WHEN `crates/agent/src/mcp.rs` is built against the migrated
  server THE agent client SHALL perform an `initialize` request (and send
  `notifications/initialized`) before its first `tools/list` or `tools/call`, matching the sequence
  `rmcp`'s own client expects."
- **DDEC-003** (spec Section 5): JSON-RPC envelope key order is explicitly excluded from the
  byte-identical bar (DR-006) — do not chase matching `result,id,jsonrpc` key order against
  `rmcp`'s own struct field order.

### Applicable NFRs
- **DR-006** (restated per Invariant 3, scoped by this story's own declared exclusions): byte-
  identical observable output for every input NOT one of the two Section 5 breaks (initialize
  requirement, JSON-vs-SSE default framing) or the DDEC-003 key-order exclusion. The 401
  unauthenticated shape and the unknown-tool error shape (both already asserted by existing
  `app.rs` tests) are NOT declared breaks and must stay byte-identical.

### Bounded Context
The MCP wire contract itself (Section 2.3: "public, external-facing" contract) — response
`Content-Type`, framing, the `initialize` requirement.

## Functional Requirements

### DR-008: Compatibility plan — break immediately, no dual-maintenance
- **EARS:** see above (verbatim).
- **Inputs / Outputs:** `crates/core/src/app.rs`'s route at `mcp_path` is served by `rmcp`'s
  `StreamableHttpService` configured with `NeverSessionManager`; no fallback path answers a
  no-`initialize` request with the old SSE-always behavior.

### DR-009: Exact error shape for a missing `initialize`
- **EARS:** see above (verbatim).
- **Inputs / Outputs:** a bare `tools/call` or `tools/list` with no prior `initialize` on a fresh
  connection gets a JSON-RPC error body with `error.code == -32600` and `error.message` containing
  the substring `"initialize"`.

### DR-010: Agent client performs the initialize handshake
- **EARS:** see above (verbatim).
- **Inputs / Outputs:** `crates/agent/src/mcp.rs`'s client sends `initialize` then
  `notifications/initialized` before its first `tools/list`/`tools/call`, matching the sequence
  `rmcp`'s own client expects. The stale module doc comment (1-6) is corrected to describe the new
  sequence.

## Acceptance Tests

> **100% must pass.**

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Fresh HTTP connection per test | no prior request state, to test the no-`initialize` path cleanly | test harness | ready |
| A test-only tool emitting a notification before its final response | `t.notifies_then_returns`, `#[cfg(test)]` only, never shipped (DT-004) | new, added in this story | pending |

### DT-001: Missing initialize gets -32600
- **Category:** failure
- **Requirements:** DR-009
- **Preconditions:** migrated server running, fresh connection, no prior request.
- **Steps:** Given the migrated server running with no prior request on a fresh connection; When a
  client sends `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.glob","arguments":{}}}`
  with no prior `initialize`; Then the response is a JSON-RPC error with `error.code == -32600` and
  `error.message` containing the substring `"initialize"`, verified by parsing the response body
  as JSON.
- **Cleanup:** close the connection.
- **Priority:** Critical.

### DT-002: Agent client completes startup and lists all 94 tools
- **Category:** happy
- **Requirements:** DR-010
- **Preconditions:** `crates/agent/src/mcp.rs`'s client pointed at the migrated server.
- **Steps:** Given that client; When the agent runs its normal startup sequence (the one exercised
  by `./agent.sh`'s own integration path); Then a `tools/list` call after that sequence succeeds
  and returns the 94-tool list, verified by asserting the returned tool count equals 94 and that
  `fs.read_bytes` is present by name.
- **Cleanup:** close the client connection.
- **Priority:** Critical.

### DT-003: Default framing is JSON for a plain tool call
- **Category:** happy
- **Requirements:** Section 4 row 2 (JSON default framing)
- **Preconditions:** migrated server, valid prior `initialize`.
- **Steps:** Given the migrated server; When a client sends a well-formed `tools/call` for a
  side-effect-free tool (`admin.list_projects`) after a valid `initialize`; Then the response
  `Content-Type` header is `application/json` (not `text/event-stream`), verified by reading the
  header directly on the HTTP response, and the body parses as a bare JSON object (no
  `event:`/`data:` prefix).
- **Cleanup:** close the connection.
- **Priority:** Critical.

### DT-004: SSE still used when a notification precedes the response
- **Category:** edge
- **Requirements:** Section 4 row 3 (notification-triggered SSE fallback)
- **Preconditions:** the test-only `t.notifies_then_returns` tool, added under `#[cfg(test)]` only.
- **Steps:** Given the migrated server and `t.notifies_then_returns` (a tool that emits a progress
  notification before returning); When that tool is called; Then the response `Content-Type` is
  `text/event-stream` and the body contains at least two `data:` frames.
- **Cleanup:** none.
- **Priority:** High.

### Existing test verdicts carried into this story (spec Section 6.4)
- **12 tests, `app.rs:320-470`** — `modified: reflect DDEC-001`. Each currently asserts an exact
  SSE frame and/or the no-`initialize` success path; each is rewritten to assert the new, declared
  behavior (JSON `Content-Type` for a plain call, `-32600` for a missing `initialize`, per
  DT-001/DT-003) rather than deleted, because the route and its auth guard (the 401 unauthenticated
  case) still need coverage and are NOT a declared break.
- **4 tests, `token_screen.rs:520-540,847,923,1818`** — `modified: reflect DDEC-001`. Same reason:
  prove the `/app/tokens` auth guard does not leak onto the MCP route, still true after the
  migration, but the literal JSON-RPC body/response shape they send and parse changes.
- **2 tests, `crates/agent/src/mcp.rs:214,220`** — `modified: reflect DR-010`. One parses a
  plain-JSON body (kept, now the common case rather than the fallback), one parses an SSE-framed
  body (kept, now the notification-path case instead of the default, repurposed to match DT-004's
  shape).
- **2 occurrences, `full_stack_e2e.rs:156-181,464-466`** — `modified: reflect DDEC-001`. The
  helper's SSE-stripping logic becomes conditional on the response `Content-Type`, matching the new
  default.

## Constraints

### Files Not to Touch
`crates/core/src/mcp/{mod,registry,schema,args}.rs` — still exist, deleted only in US-0009 (the
spec's Section 7 rationale: deleting first would leave the build broken with no way to bisect a
transport bug from a removed-code bug). `crates/core/src/mcp/server.rs` — read only, not modified
(its `#[tool]` methods are already complete per US-0007).

### Dependencies Not to Add
None beyond what US-0002 landed.

### Patterns to Avoid
Writing a client-visible SSE emulation shim on top of `rmcp`'s JSON default to preserve the old
framing — DDEC-001 explicitly rejects this ("doing so would reintroduce the hand-rolled framing
this migration exists to remove"). Chasing JSON-RPC key-order parity (DDEC-003 excludes it).

### Scope Boundary
No deletion of the old `mcp/` module in this story (US-0009's job). No change to any individual
tool's logic (Decision D1, DR-007).

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace`, with the 18 tests above (12 + 4 + 2) and the 1 `full_stack_e2e.rs`
occurrence explicitly modified per the verdicts above, and every other test in the suite
unmodified and green.

### Behaviors That Must Not Change
The 401 unauthenticated response shape; the unknown-tool error shape; every individual tool's
behavior (DR-007). DR-006 restated per Invariant 3, with its explicit exclusions (initialize
requirement, JSON/SSE default framing, JSON-RPC key order) named here rather than silently
widened.

### API Contracts to Preserve
The JSON-RPC success/error envelope's three top-level keys (`jsonrpc`, `id`, `result`/`error`) —
preserved, only their serialization order is excluded from the byte-identical bar (DDEC-003).

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5. In addition: confirm every one of the 18
modified tests' new assertion is traceable to a named Section 5 break or DDEC-003, not a
convenience rewrite.
