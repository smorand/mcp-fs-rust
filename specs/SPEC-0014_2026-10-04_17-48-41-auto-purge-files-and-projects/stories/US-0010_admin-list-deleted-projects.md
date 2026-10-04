# US-0010: `admin.list_deleted_projects`

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 10
> Depends On: US-0006
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

Lets a caller see which soft-deleted projects are visible to them, with a countdown to permanent
removal — the read side of the admin surface this spec adds, consumed later by US-0012's browser
screen.

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
`admin.list_projects` (`tools/admin.rs:247` → `storage/admin.rs:236 list_projects_for(person)`)
is membership-filtered per caller, platform admin sees all — this tool follows the identical
filtering convention, verified by reading the existing implementation rather than assumed.

### Decisions That Govern This Story
**DEC-012** (Section 17): `admin.list_deleted_projects()` is membership-filtered per caller
(platform admin sees all), matching `admin.list_projects`'s verified convention, not
platform-admin-only.

### Applicable NFRs
None beyond the existing membership-filter convention.

## Functional Requirements

### FR-NEW-014 [EARS-E]: `admin.list_deleted_projects`
> WHEN a caller invokes `admin.list_deleted_projects()` THE system SHALL return every
> soft-deleted project visible to that caller — filtered exactly as `admin.list_projects` filters
> by membership, with a platform admin seeing all — as `{project_id, owner, deleted_at,
> days_until_permanent_removal}`.

- **Business Rules:** `days_until_permanent_removal = project_purge_grace_days -
  floor((now - deleted_at) in days)`, not clamped below zero. No REST route (MCP-only).

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Soft-deleted projects owned by different callers, seeded `deleted_at` | Direct DB seed, no dependency on US-0006's sweep having run | fixture | ready |

### E2E-NEW-058: Platform admin sees all
- **Category:** happy
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Steps:** Given 2 soft-deleted projects (`deleted_at` 5 and 10 days ago),
  `project_purge_grace_days=30` / When platform admin calls `admin.list_deleted_projects()` /
  Then returns 2 entries with `days_until_permanent_removal` computed as 25 and 20 respectively.
- **Priority:** Critical

### E2E-NEW-059: Second happy-path variant (no REST route exists)
- **Category:** happy
- **Requirements:** FR-NEW-014
- **Steps:** Repeat E2E-NEW-058 with a single soft-deleted project to exercise the single-result
  shape, via MCP only (parent spec Section 9.1 correction — no REST equivalent).
- **Priority:** Medium

### E2E-NEW-060: Membership-filtered result for a non-admin caller
- **Category:** edge (auth scoping) — corrected per DEC-012
- **Requirements:** FR-NEW-014
- **Steps:** Given caller is owner/member of ONE of the two soft-deleted projects, not platform
  admin / When they call `admin.list_deleted_projects()` / Then they see only their own
  owned/member soft-deleted project, not the other (filtered, same as `admin.list_projects`'s
  convention) — NOT `ERR_FORBIDDEN`.
- **Priority:** Critical

### E2E-NEW-061: Empty state
- **Category:** edge (empty)
- **Requirements:** FR-NEW-014
- **Steps:** Given zero soft-deleted projects exist / When called / Then returns empty array, no
  error.
- **Priority:** Low

### E2E-NEW-062: Boundary — countdown when grace period already elapsed
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-014
- **Steps:** Given a soft-deleted project already past its grace period but the grace-sweep has
  not yet run this cycle (race window) / When listed / Then `days_until_permanent_removal` is 0 or
  negative, not clamped.
- **Priority:** Low

## Constraints

### Files Not to Touch
US-0006/US-0008's sweep functions — this tool only reads, never triggers a sweep.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not implement this as platform-admin-only — DEC-012 explicitly corrects that assumption
against the verified `list_projects_for` convention.

### Scope Boundary
Read-only listing.

## Non Regression

### Existing Tests That Must Pass
`admin.list_projects`'s existing tests, unaffected.

### Behaviors That Must Not Change
`list_projects_for`'s existing filtering behavior for the non-deleted listing tool.

### API Contracts to Preserve
None affected.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
