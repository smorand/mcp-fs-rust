# US-0011: `admin.undelete_project`

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 11
> Depends On: US-0006, US-0004
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

The reversal half of the project lifecycle: clears `deleted_at`, immediately restoring normal
access (verified against US-0004's gate). This is the tool the browser screen (US-0012) calls.

## Technical Context

### Stack
Rust 2024, rmcp `#[tool]` macros.

### Relevant File Structure
```
crates/core/src/
  tools/admin.rs    (new tool function)
  mcp/server.rs      (new #[tool] method)
```

### Existing Patterns
`require_owner_or_admin` (`state.rs:47-54`), reused verbatim — and per US-0004's Business Rules,
this tool's authorization path (`get_project`/`require_owner`) is explicitly exempted from the
`deleted_at` filter, so it keeps working on soft-deleted rows.

### Decisions That Govern This Story
None new; relies directly on US-0004's explicit exemption of `require_owner_or_admin` from the
`deleted_at` filter (parent spec FR-NEW-013 Business Rules).

### Applicable NFRs
None beyond existing auth conventions.

## Functional Requirements

### FR-NEW-015 [EARS-E]: `admin.undelete_project`
> WHEN an owner or platform admin calls `admin.undelete_project(project_id)` on a project with
> `deleted_at` non-null THE system SHALL set `deleted_at` to `NULL` and return success.

- **Business Rules:** `atime` values are untouched by undelete — an idle project can become
  stale again quickly if nothing is read afterward. No REST route (MCP-only).

### FR-NEW-016 [EARS-O]: `undelete_project` rejections
> IF the project's `deleted_at` is already `NULL` THEN THE system SHALL return
> `ERR_INVALID_ARGUMENT`; IF the project does not exist THEN THE system SHALL return
> `ERR_PROJECT_NOT_FOUND`; IF the caller is neither owner nor platform admin THEN THE system
> SHALL return `ERR_FORBIDDEN`.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Soft-deleted project, seeded `deleted_at` | Direct DB seed | fixture | ready |

### E2E-NEW-063: Owner undeletes
- **Category:** happy
- **Scenario:** SC-006
- **Requirements:** FR-NEW-015
- **Steps:** Given project soft-deleted / When owner calls `admin.undelete_project(proj1)` / Then
  succeeds, `deleted_at` cleared to NULL.
- **Priority:** Critical

### E2E-NEW-064: Platform admin undeletes
- **Category:** happy
- **Requirements:** FR-NEW-015
- **Steps:** Given project soft-deleted / When platform admin (non-owner) calls it / Then
  succeeds.
- **Priority:** High

### E2E-NEW-065: Already-live project rejected
- **Category:** failure
- **Requirements:** FR-NEW-016
- **Steps:** Given project NOT soft-deleted (`deleted_at` already NULL) / When owner calls
  `admin.undelete_project` / Then fails `ERR_INVALID_ARGUMENT`, no state change.
- **Priority:** Critical

### E2E-NEW-066: Unknown project
- **Category:** failure
- **Requirements:** FR-NEW-016
- **Steps:** Given project does not exist at all / When called with `project_id="ghost"` / Then
  fails `ERR_PROJECT_NOT_FOUND`, no row created.
- **Priority:** Critical

### E2E-NEW-067: Non-owner, non-admin forbidden
- **Category:** failure
- **Requirements:** FR-NEW-016
- **Steps:** Given project soft-deleted, caller is a non-owner non-admin member / When they call
  `admin.undelete_project` / Then fails `ERR_FORBIDDEN`, `deleted_at` unchanged.
- **Priority:** Critical

### E2E-NEW-068: Undelete restores real access (closes the loop, phase 2)
- **Category:** state transition
- **Scenario:** SC-006
- **Requirements:** FR-NEW-015, FR-NEW-016
- **Steps:** Given project soft-deleted, `fs.read` fails per US-0004's E2E-NEW-040 / When
  `admin.undelete_project` is called / Then `fs.read` on that project succeeds again immediately
  after — confirms this tool's effect is observed through US-0004's live gate, not a cached state.
- **Priority:** Critical

## Constraints

### Files Not to Touch
`storage/admin.rs require_member` (US-0004's gate — this story only clears the flag it reads).

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not re-implement the `deleted_at` check here "defensively" — `get_project`/`require_owner` are
already exempted per US-0004, and duplicating the check would contradict that design.

### Scope Boundary
Clearing `deleted_at` only. Does not touch `atime` or any file data.

## Non Regression

### Existing Tests That Must Pass
US-0004's gate tests, unaffected except where this story's own E2E-NEW-068 explicitly exercises
the transition.

### Behaviors That Must Not Change
`require_owner_or_admin`'s existing behavior for every other tool.

### API Contracts to Preserve
None affected.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
