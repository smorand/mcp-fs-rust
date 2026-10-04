# US-0003: Migrate `fs.*` tools (35) to `#[tool]` methods

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 3
> Depends On: US-0002
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective
Re-express every `fs.*` tool (35 of the 94 total, per the project's own tool-family breakdown) as
one `#[tool]` method on the new `McpServer` struct, calling the exact same `core::fs_ops` and
authorization code the old `ToolRegistry`-based handlers called. No logic moves: only the dispatch
wrapper changes. The old `crates/core/src/mcp/{mod,registry,schema,args}.rs` module stays untouched
and still serves traffic — this story's new code is additive and not yet wired into any route
(that happens in US-0008).

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0 (`#[tool_router]` / `#[tool]` macros from the `server` feature), `schemars`
(via `rmcp`'s own re-export, used to derive `inputSchema`).

### Relevant File Structure
```
crates/core/src/mcp/server.rs      # new file: McpServer struct, #[tool_router], fs.* #[tool] methods
crates/core/src/tools/read.rs      # existing fs_ops calls referenced, not modified
crates/core/src/tools/write.rs
crates/core/src/tools/edit.rs
crates/core/src/tools/editor.rs
crates/core/src/tools/listing.rs
crates/core/src/tools/metadata.rs
crates/core/src/tools/lifecycle.rs
crates/core/src/tools/document.rs
crates/core/src/tools/doc.rs
crates/core/src/tools/sqlite.rs
crates/core/src/tools/db.rs
crates/core/src/tools/web.rs
crates/core/src/tools/context7.rs
```
Only `crates/core/src/mcp/server.rs` is written to by this story. The `tools/*.rs` files are read
for reference (the exact `name`, `description`, authorization call and `core::fs_ops` call each
existing `register()` function makes) but are NOT modified or deleted — the spec's Implementation
Order (Section 7, steps 2-4) keeps the old tool surface fully intact until the transport swap
(US-0008) and deletion (US-0009) land.

### Existing Patterns
Read one existing `register(&mut ToolRegistry)` function in `crates/core/src/tools/read.rs` (e.g.
the handler for `fs.read_bytes`) to see the exact shape: `state.authorize(mount_id, person)` first,
path normalization via `state.safety.normalize_path`, then the `core::fs_ops` call, then the return
shape. The new `#[tool]` method must perform the identical sequence — authorize, normalize, call
the same `fs_ops` function, return the same JSON shape — with only the registration mechanism
(macro attribute vs. `ToolRegistry::register`) changing.

### Decisions That Govern This Story
- **DR-002** (spec Section 3.1): "Every tool currently registered via `register(&mut
  ToolRegistry)` in `crates/core/src/tools/*.rs` SHALL be re-expressed as one `#[tool]` method on a
  single server struct (name: `McpServer`, placed at `crates/core/src/mcp/server.rs`), preserving,
  for every one of the 94 tools frozen in `TOOL_CONTRACT.txt` and `tool-contract-golden.json`: the
  exact tool `name` string (dot-separated, e.g. `fs.read_bytes`), the exact `description` string,
  and an `inputSchema` that serializes identically to the one `tool-contract-golden.json` already
  records for that tool." (this story covers the 35 `fs.*` entries of the 94)
- **DR-007** (spec Section 3.2): "The migration SHALL NOT change the behavior, name, authorization
  check or return shape of any individual tool. Only the transport and dispatch layer carrying
  those tools SHALL change."
- **Decision D1** (spec Section 8): migration scoped to transport/dispatch only, `core::fs_ops` is
  not touched.

### Applicable NFRs
- **DR-006** (restated verbatim, per Invariant 3 — the DEBT success criterion this story must not
  violate): "For every input the existing suite (`cargo test --workspace`) covers today that is NOT
  one of the declared breaks enumerated in Section 5, the system SHALL produce byte-identical
  observable output before and after this migration: the same HTTP status code, the same response
  headers, the same response body bytes." This story adds dead code (not yet routed), so it cannot
  itself violate DR-006, but the new methods must be written so that once US-0008 wires them in,
  DR-006 holds for every `fs.*` tool call.

### Bounded Context
Tool family: `fs.*` (35 tools per `TOOL_CONTRACT.txt` / the project's own `AGENTS.md` tool count
breakdown). Key entity: none new — these methods call existing `core::fs_ops` functions.

## Functional Requirements

### DR-002 (fs.* subset): Re-express all 35 `fs.*` tools as `#[tool]` methods
- **EARS:** see "Decisions That Govern This Story" above (verbatim).
- **Inputs / Outputs:** For each of the 35 `fs.*` tool names in `TOOL_CONTRACT.txt`, the new
  `#[tool]` method accepts the same parameter set (snake_case, per the frozen `inputSchema`) and
  returns the same JSON shape the old handler returned, byte-for-byte on the `result` payload
  (DR-006, excluding transport framing which is not yet live).
- **Business Rules:** `state.authorize(mount_id, person)` first, then `state.safety.normalize_path`
  on every path parameter, then the call into `core::fs_ops`, exactly as the current `tools/*.rs`
  handlers do. Never reimplement an operation in this new dispatch layer — call the same
  `core::fs_ops` function the old handler called (project convention, `AGENTS.md`).

## Acceptance Tests

> **100% must pass.** Run through `make test` / `cargo test --workspace`, never ad hoc.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Sample mount/project fixtures | whatever existing `tools/read.rs` etc. unit tests already use | existing test fixtures | ready |

### E2E-FS-001: Each fs.* `#[tool]` method dispatches identically to the old handler
- **Category:** happy
- **Requirements:** DR-002, DR-007
- **Preconditions:** `McpServer` compiled with all 35 `fs.*` methods present.
- **Steps:** Given a `McpServer` instance constructed with the same `AppState` the old
  `ToolRegistry`-based handler used in its own existing unit test; When the new `#[tool]` method
  function is called directly (not through any HTTP route — no route exists yet) with the same
  arguments an existing `tools/*.rs` test uses; Then the returned value is identical to what the
  old handler's existing test already asserts for that same input.
- **Cleanup:** none (no persisted state beyond what the existing fixture already cleans up).
- **Priority:** Critical.

### E2E-FS-002: Tool name and description strings are preserved verbatim
- **Category:** happy
- **Requirements:** DR-002
- **Preconditions:** `McpServer`'s `#[tool_router]` is introspectable (rmcp's router exposes a
  `list_tools()` or equivalent without a live transport).
- **Steps:** Given the `McpServer` struct; When its tool router is queried for the 35 `fs.*`
  entries; Then each returned `name` string matches `TOOL_CONTRACT.txt` exactly (dot-separated,
  e.g. `fs.read_bytes`) and each `description` string matches verbatim.
- **Cleanup:** none.
- **Priority:** Critical.

### E2E-FS-003: Authorization is checked before dispatch, identically
- **Category:** failure
- **Requirements:** DR-002, DR-007
- **Preconditions:** a `mount_id` the test person is not authorized for.
- **Steps:** Given an unauthorized `(mount_id, person)` pair; When any `fs.*` `#[tool]` method is
  called directly with that pair; Then it returns the same `ToolError::<code>` (same `ERR_*` code,
  same message shape) the old handler's existing unauthorized-access test already asserts.
- **Cleanup:** none.
- **Priority:** High.

## Constraints

### Files Not to Touch
`crates/core/src/tools/*.rs` (the existing `register()` functions stay as-is until US-0009);
`crates/core/src/mcp/{mod,registry,schema,args}.rs` (deleted only in US-0009); `crates/core/src/app.rs`
(route wiring is US-0008's job, not this story's).

### Dependencies Not to Add
None beyond what US-0002 already landed.

### Patterns to Avoid
Reimplementing any `fs.*` operation's logic inside `mcp/server.rs` instead of calling the existing
`core::fs_ops` function — the project convention (`AGENTS.md`) is that `core::fs_ops` is the only
place an operation is written.

### Scope Boundary
No transport change, no route registration, no deletion of old code. This story only adds
`crates/core/src/mcp/server.rs` with the 35 `fs.*` `#[tool]` methods.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` — unaffected (additive-only story; no existing code is modified).

### Behaviors That Must Not Change
DR-006 (byte-identical output, restated per Invariant 3): does not yet apply at the transport
level (nothing new is routed), but the new methods' logic must already match it once wired.
DR-007: no individual tool's behavior, name, authorization check, or return shape changes.

### API Contracts to Preserve
`TOOL_CONTRACT.txt` entries for all 35 `fs.*` tools: name, description, inputSchema.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
