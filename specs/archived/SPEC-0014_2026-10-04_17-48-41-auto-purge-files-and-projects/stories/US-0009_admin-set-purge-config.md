# US-0009: `admin.set_purge_config`

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 9
> Depends On: US-0002
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

Adds the one admin-facing tool that lets an owner or platform admin configure autopurge for a
project: enable flag, internal-vs-external driver choice, and the two independent retention
thresholds. This is the entry point every downstream sweep story (US-0005, US-0006) reads its
configuration from.

## Technical Context

### Stack
Rust 2024, rmcp `#[tool]` macros.

### Relevant File Structure
```
crates/core/src/
  tools/admin.rs    (new tool function, alongside the existing admin.* functions)
  mcp/server.rs      (new #[tool] method, following the existing admin.* pattern)
```

### Existing Patterns
- `require_owner_or_admin` (`state.rs:47-54`) is the existing gate for "owner or platform admin"
  — already used by other `admin.*` mutations; reuse it verbatim, do not write a new gate.
- No existing `admin.*` tool has a REST equivalent (`api/dataplane.rs:103-147` routes only `fs.*`)
  — this tool is MCP-only, consistent with `admin.create_project`/`delete_project`/`list_projects`
  (`mcp/server.rs:2039-2280`).

### Data Model (excerpt)
`project_purge_config(project_id, autopurge_enabled, use_internal_purge, file_retention_days,
project_retention_days)` — from US-0002.

### Decisions That Govern This Story
No new decision beyond what the FRs below state directly; `require_owner_or_admin` reuse is
dictated by US-0004's Business Rules (the `deleted_at` filter lives only in `require_member`,
so this tool's authorization path is unaffected by soft-delete state).

### Applicable NFRs
Section 7.2 (Security): no new auth mechanism, reuses `require_owner_or_admin`.

## Functional Requirements

### FR-NEW-004 [EARS-E]: `admin.set_purge_config`
> WHEN an owner or platform admin calls `admin.set_purge_config(project_id,
> autopurge_enabled: bool, use_internal_purge: bool, file_retention_days: Option<u32>,
> project_retention_days: Option<u32>)` with every present retention value `> 0` THE system
> SHALL persist the configuration and return it.

- **Business Rules:** either retention field SHALL be permitted to be `None`/absent (that axis
  disabled independently of the other). No REST route is added — MCP-only, consistent with every
  other `admin.*` tool.

### FR-NEW-005 [EARS-O]: Rejected `set_purge_config` arguments
> IF a present `file_retention_days` or `project_retention_days` is `<= 0` THEN THE system SHALL
> return `ERR_INVALID_ARGUMENT` and persist nothing.

### FR-NEW-006 [EARS-O]: `set_purge_config` authorization and existence
> IF the caller is neither the project owner nor a platform admin THEN THE system SHALL return
> `ERR_FORBIDDEN`; IF the project does not exist THEN THE system SHALL return
> `ERR_PROJECT_NOT_FOUND`.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Project owned by `alice`, member `mallory` | fixture | fixture | ready |

### E2E-NEW-015: Owner sets config
- **Category:** happy
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Steps:** Given project `proj1` owned by `alice` / When `alice` calls
  `admin.set_purge_config(proj1, true, true, 7, 30)` / Then succeeds, config persisted exactly.
- **Priority:** Critical

### E2E-NEW-016: Platform admin (non-owner) sets config
- **Category:** happy
- **Requirements:** FR-NEW-004
- **Steps:** Given platform admin `root` (not owner) / When `root` calls the same / Then succeeds
  (platform-admin bypass via `require_owner_or_admin`).
- **Priority:** High

### E2E-NEW-017: Second happy-path variant (no REST route exists)
- **Category:** happy
- **Requirements:** FR-NEW-004
- **Steps:** Repeat E2E-NEW-015 with a different valid combination (e.g.
  `use_internal_purge=false`) to exercise the tool a second time end-to-end via MCP, since no REST
  equivalent exists for this tool (parent spec Section 9.1 correction).
- **Priority:** Medium

### E2E-NEW-018: Non-owner, non-admin forbidden
- **Category:** failure
- **Requirements:** FR-NEW-006
- **Steps:** Given member `mallory` (not owner, not admin) / When `mallory` calls
  `admin.set_purge_config` / Then fails `ERR_FORBIDDEN`, config unchanged.
- **Priority:** Critical

### E2E-NEW-019: Unknown project
- **Category:** failure
- **Requirements:** FR-NEW-006
- **Steps:** When `alice` calls with `project_id = "ghost"` / Then fails `ERR_PROJECT_NOT_FOUND`,
  no row created.
- **Priority:** Critical

### E2E-NEW-020: Boundary — zero retention values rejected
- **Category:** edge (boundary), table-driven
- **Requirements:** FR-NEW-005
- **Steps:** `file_retention_days = 0` → `ERR_INVALID_ARGUMENT`; `project_retention_days = 0` →
  `ERR_INVALID_ARGUMENT`.
- **Priority:** Critical

### E2E-NEW-021: Both retentions None is valid
- **Category:** edge (data variation)
- **Requirements:** FR-NEW-004
- **Steps:** `file_retention_days = None`, `project_retention_days = None`,
  `autopurge_enabled=true`, `use_internal_purge=false` / Then persists with NULL retentions, no
  validation error.
- **Priority:** High

### E2E-NEW-022: Boundary — minimum valid positive value
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-005
- **Steps:** `file_retention_days = 1` / Then succeeds.
- **Priority:** Medium

### E2E-NEW-023: Idempotent re-set
- **Category:** idempotency
- **Requirements:** FR-NEW-004
- **Steps:** Given config already set / When called again with identical values / Then succeeds,
  no duplicate row, same final state.
- **Priority:** Low

## Constraints

### Files Not to Touch
`purge.rs` (US-0005/0006), `cli.rs` (US-0007) — this tool only persists config, never runs a
sweep as a side effect.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not add a REST route "for consistency" — no `admin.*` tool has one; see Technical Context.

### Scope Boundary
Persist and validate config only. Does not trigger a sweep.

## Non Regression

### Existing Tests That Must Pass
Existing `admin.*` tool tests, unaffected — this is additive.

### Behaviors That Must Not Change
`require_owner_or_admin`'s existing behavior for every other `admin.*` tool.

### API Contracts to Preserve
None affected.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
