# US-0002: Add the rmcp 3.5.0 dependency

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 2
> Depends On: US-0001
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective
Add `rmcp` 3.5.0 as a dependency with only the `server` and `transport-streamable-http-server`
features, and confirm the workspace builds with zero other code changed. This isolates "the
dependency itself is clean" from every later step, per the spec's Section 7 implementation-order
rationale: a build failure here cannot be confused with a tool-migration or transport bug.

## Technical Context

### Stack
Rust 2024, Cargo workspace with `crates/core` and `crates/agent` members.

### Relevant File Structure
```
Cargo.toml                  # [workspace.dependencies] table
crates/core/Cargo.toml      # dependency reference
```

### Existing Patterns
Other optional/pinned dependencies in `[workspace.dependencies]` (e.g. `aws-sdk-s3`,
`tiberius-ng`) are declared once at the workspace level and referenced with `workspace = true`
from `crates/core/Cargo.toml`. Follow that existing convention for `rmcp` — do not declare the
version string twice.

### Decisions That Govern This Story
- **DR-001** (spec Section 3.1): "The MCP tool-serving layer SHALL be implemented using the `rmcp`
  crate, version `3.5.0` pinned in `[workspace.dependencies]`, added to `crates/core/Cargo.toml`
  with the `server` and `transport-streamable-http-server` features enabled and no others."
- **Decision D1** (spec Section 8): "the migration is scoped to the MCP transport and dispatch
  layer only. It does not touch `core::fs_ops`, `core::storage`, or any of the 94 tools' actual
  logic."

## Functional Requirements

### DR-001: Pin rmcp 3.5.0 with exactly two features
- **EARS:** The MCP tool-serving layer SHALL be implemented using the `rmcp` crate, version
  `3.5.0` pinned in `[workspace.dependencies]`, added to `crates/core/Cargo.toml` with the
  `server` and `transport-streamable-http-server` features enabled and no others.
- **Inputs / Outputs:** Input: `Cargo.toml` and `crates/core/Cargo.toml`. Output: `cargo build
  --workspace --all-features` succeeds; `cargo metadata` shows `rmcp 3.5.0` resolved with exactly
  the two named features active (no default features beyond what those two pull in transitively).
- **Business Rules:** No other feature flag of `rmcp` is enabled (not `client`, not
  `transport-sse-server`, not `macros` unless `server`/`transport-streamable-http-server` already
  require it transitively — check `rmcp`'s own `Cargo.toml` to confirm which features are
  implied).

## Acceptance Tests

> **100% must pass.** Run through `make test` / `cargo test --workspace`, never ad hoc.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| none | no runtime data needed; this is a build-only story | n/a | ready |

### DT-DEP-001: Workspace builds with rmcp added, zero other code changed
- **Category:** happy
- **Requirements:** DR-001
- **Preconditions:** `rmcp` is not currently a dependency (confirmed: `Cargo.lock` has zero
  `rmcp` entries).
- **Steps:** Given `rmcp = "3.5.0"` with features `["server", "transport-streamable-http-server"]`
  added to `[workspace.dependencies]` and referenced from `crates/core/Cargo.toml`; When
  `cargo build --workspace --all-features` runs; Then the build succeeds with exit code 0 and no
  other source file in the repository differs from `main`.
- **Cleanup:** none.
- **Priority:** Critical.

### DT-DEP-002: No unintended feature flags resolved
- **Category:** edge
- **Requirements:** DR-001
- **Preconditions:** dependency added as above.
- **Steps:** Given the dependency is added; When `cargo tree -p rmcp -e features` (or `cargo
  metadata --format-version 1 | jq` filtered to the `rmcp` package) is inspected; Then the active
  feature set contains exactly `server` and `transport-streamable-http-server` plus whatever they
  transitively require per `rmcp`'s own manifest, and does not contain `client` or
  `transport-sse-server`.
- **Cleanup:** none.
- **Priority:** High.

## Constraints

### Files Not to Touch
No file under `crates/core/src/` or `crates/agent/src/` changes in this story — it is a manifest-
only change (Decision D1 scope).

### Dependencies Not to Add
Any `rmcp` feature beyond `server` and `transport-streamable-http-server`.

### Patterns to Avoid
Declaring the `rmcp` version string in both `Cargo.toml` and `crates/core/Cargo.toml` instead of
using `workspace = true` in the latter.

### Scope Boundary
No `#[tool]` method, no server struct, no route change. That is US-0003 onward.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` (Section 6.1) — unaffected, since no source file changes.

### Behaviors That Must Not Change
DR-006: byte-identical observable output for every input not a declared break (Section 5) — holds
trivially here since no behavior-affecting code exists yet.

### API Contracts to Preserve
n/a — no API surface touched.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
