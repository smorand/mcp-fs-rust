# US-0004: Migrate `admin.*` (10) + `search.*` (4) tools to `#[tool]` methods

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 4
> Depends On: US-0002
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Re-express the 10 `admin.*` tools and the 4 optional `search.*` tools as `#[tool]` methods on the
same `McpServer` struct US-0003 started, calling the same `core::fs_ops`/admin engine code the old
handlers called. Additive only — no route wiring yet.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0 `#[tool]` macros.

### Relevant File Structure
```
crates/core/src/mcp/server.rs       # extended with admin.* and search.* #[tool] methods
crates/core/src/tools/admin.rs      # reference only, not modified
crates/core/src/tools/search.rs
crates/core/src/tools/search_semantic.rs
```

### Existing Patterns
Follow `crates/core/src/tools/admin.rs`'s existing `register()` functions for the exact
authorization and dispatch sequence per `admin.*` tool. Platform admin manages projects and
membership — it does NOT get implicit file access (project convention, `AGENTS.md`); preserve that
separation in the new methods exactly as the old handlers enforce it.

### Decisions That Govern This Story
- **DR-002** (admin.*+search.* subset, same requirement text as US-0003, scoped to these 14
  tools).
- **DR-007**: no individual tool's behavior, authorization check, or return shape changes.

### Applicable NFRs
DR-006 (restated, per Invariant 3): byte-identical observable output for every non-declared-break
input, once wired by US-0008.

### Bounded Context
`admin.*` (project/membership management, platform scope) and `search.*` (BM25/RAG search,
enabled only when the search feature is on — see `.agent_docs/search.md`; if search is disabled in
the build configuration used for this story's tests, the 4 `search.*` methods still compile but
their tests are conditional on the same feature gate the existing `tools/search.rs` registration
already uses).

## Functional Requirements

### DR-002 (admin.*+search.* subset)
- **EARS:** see US-0003's DR-002 text, scoped here to the 10 `admin.*` and 4 `search.*` tool
  names in `TOOL_CONTRACT.txt`.
- **Inputs / Outputs:** same parameter sets and return shapes as the existing `tools/admin.rs` and
  `tools/search.rs` handlers.
- **Business Rules:** platform admin authorization (not file-mount authorization) for `admin.*`;
  whatever per-project index-mode gating `search.rs`'s existing handlers already enforce for
  `search.*`.

## Acceptance Tests

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Existing admin/search test fixtures | project/member/index fixtures already used by `tools/admin.rs` and `tools/search.rs` unit tests | existing | ready |

### E2E-ADM-001: admin.* and search.* `#[tool]` methods dispatch identically to the old handlers
- **Category:** happy
- **Requirements:** DR-002, DR-007
- **Preconditions:** `McpServer` compiled with the 14 new methods present.
- **Steps:** Given the same fixtures an existing `tools/admin.rs`/`tools/search.rs` test uses; When
  the new `#[tool]` method is called directly with the same arguments; Then the result matches the
  old handler's existing asserted result exactly.
- **Cleanup:** none beyond existing fixture teardown.
- **Priority:** Critical.

### E2E-ADM-002: Platform admin gets no implicit file access
- **Category:** failure
- **Requirements:** DR-002, DR-007
- **Preconditions:** a platform admin person with no explicit project membership.
- **Steps:** Given that person; When an `admin.*` `#[tool]` method lists or modifies project
  membership (allowed) versus when an `fs.*` method (from US-0003) is called against a project
  they are not a member of (not allowed); Then the `admin.*` call succeeds and the `fs.*` call
  returns the same unauthorized `ToolError` the old handler's test already asserts.
- **Cleanup:** none.
- **Priority:** High.

### E2E-ADM-003: Tool names and descriptions preserved verbatim
- **Category:** happy
- **Requirements:** DR-002
- **Preconditions:** `McpServer`'s router introspectable.
- **Steps:** Given the router; When queried for the 10 `admin.*` and 4 `search.*` entries; Then
  each `name` and `description` matches `TOOL_CONTRACT.txt` exactly.
- **Cleanup:** none.
- **Priority:** Critical.

## Constraints

### Files Not to Touch
`crates/core/src/tools/admin.rs`, `search.rs`, `search_semantic.rs` (reference only);
`crates/core/src/mcp/{mod,registry,schema,args}.rs`; `crates/core/src/app.rs`.

### Dependencies Not to Add
None.

### Patterns to Avoid
Reimplementing admin ACL logic or search ranking in `mcp/server.rs` instead of calling the
existing engine functions.

### Scope Boundary
No transport change, no deletion.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` — unaffected.

### Behaviors That Must Not Change
DR-006, DR-007 (restated per Invariant 3, see US-0003).

### API Contracts to Preserve
`TOOL_CONTRACT.txt` entries for the 10 `admin.*` and 4 `search.*` tools.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
