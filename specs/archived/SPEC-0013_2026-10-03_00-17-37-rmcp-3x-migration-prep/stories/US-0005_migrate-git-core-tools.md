# US-0005: Migrate `git.*` core tools (39) to `#[tool]` methods

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 5
> Depends On: US-0002
> Complexity: L
> min_tier: 2
> Files touched: 1

## Objective
Re-express the 39 core `git.*` tools (clone/push/fetch/pull/branch/merge/diff etc., excluding the
`git.auth*` and `git.pr_*` families covered by US-0006) as `#[tool]` methods on `McpServer`,
calling the exact same `git::repo`/`git::merge`/`git::odb` code the old handlers called.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0 `#[tool]` macros, `git2` (libgit2) underneath (unchanged).

### Relevant File Structure
```
crates/core/src/mcp/server.rs       # extended with git.* #[tool] methods
crates/core/src/tools/git.rs        # reference only, not modified
crates/core/src/git/                # repo.rs, merge.rs, odb.rs — called, not modified
```

### Existing Patterns
Follow `crates/core/src/tools/git.rs`'s existing `register()` functions. A merge conflict is an
`Ok` response carrying `status: "conflict"` rather than an error, with every combine response
serialized through the types in `git/merge.rs`, never a hand-built `json!` (project convention,
`AGENTS.md`) — preserve this exactly in the new `#[tool]` methods.

### Decisions That Govern This Story
- **DR-002** (git.* core subset, same requirement text as US-0003/0004, scoped to these 39 tools).
- **DR-007**: no individual tool's behavior, authorization check, merge semantics, or return shape
  changes.

### Applicable NFRs
DR-006 (restated, per Invariant 3): byte-identical observable output once wired by US-0008.

### Bounded Context
`git.*` core: per-project repository + write lock (`git/repo.rs`), objects in the blob store under
`git:{sha}` (`git/odb.rs`), the shared merge engine (`git/merge.rs`).

## Functional Requirements

### DR-002 (git.* core subset)
- **EARS:** see US-0003's DR-002 text, scoped here to the 39 core `git.*` tool names in
  `TOOL_CONTRACT.txt` (excluding `git.auth*` and `git.pr_*`).
- **Inputs / Outputs:** same parameter sets and return shapes as the existing `tools/git.rs`
  handlers, including the `status: "conflict"` combine-operation shape.
- **Business Rules:** `max_pack_size_mb` enforced identically; pushed objects really indexed
  identically; `volume_id` present in every underlying query the handler triggers (unchanged,
  since no storage code moves).

## Acceptance Tests

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Existing git fixture repos | whatever `tools/git.rs` unit tests already construct (temp repos, test commits) | existing | ready |

### E2E-GIT-001: git.* core `#[tool]` methods dispatch identically to the old handlers
- **Category:** happy
- **Requirements:** DR-002, DR-007
- **Preconditions:** `McpServer` compiled with the 39 new methods present.
- **Steps:** Given the same fixture repo an existing `tools/git.rs` test uses; When the new
  `#[tool]` method is called directly with the same arguments; Then the result matches the old
  handler's existing asserted result exactly, including any `status: "conflict"` shape.
- **Cleanup:** temp repo teardown, as the existing fixture already does.
- **Priority:** Critical.

### E2E-GIT-002: Merge conflict response shape unchanged
- **Category:** edge
- **Requirements:** DR-002, DR-007
- **Preconditions:** two branches with a genuine conflicting change, as an existing conflict test
  already sets up.
- **Steps:** Given that conflict fixture; When the new combine `#[tool]` method runs it; Then the
  response is `Ok` with `status: "conflict"`, serialized through the same `git/merge.rs` types,
  matching the old handler's existing test byte-for-byte on that field.
- **Cleanup:** temp repo teardown.
- **Priority:** Critical.

### E2E-GIT-003: Tool names and descriptions preserved verbatim
- **Category:** happy
- **Requirements:** DR-002
- **Preconditions:** `McpServer`'s router introspectable.
- **Steps:** Given the router; When queried for the 39 core `git.*` entries; Then each `name` and
  `description` matches `TOOL_CONTRACT.txt` exactly.
- **Cleanup:** none.
- **Priority:** Critical.

## Constraints

### Files Not to Touch
`crates/core/src/tools/git.rs`, `crates/core/src/git/*` (reference only, logic does not move per
Decision D1); `crates/core/src/mcp/{mod,registry,schema,args}.rs`; `crates/core/src/app.rs`.

### Dependencies Not to Add
None.

### Patterns to Avoid
Hand-building a `json!` for any combine-operation response instead of using the shared types in
`git/merge.rs`.

### Scope Boundary
No transport change, no deletion, no change to `git2`/libgit2 usage.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` — unaffected.

### Behaviors That Must Not Change
DR-006, DR-007 (restated per Invariant 3, see US-0003). Specifically: no custom libgit2 ODB
backend is introduced (blob store stays the source of truth, bytes identical) — this story must
not change that.

### API Contracts to Preserve
`TOOL_CONTRACT.txt` entries for the 39 core `git.*` tools.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
