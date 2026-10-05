# US-0006: Project soft-delete sweep logic

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 6
> Depends On: US-0005
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Implements the project-level staleness sweep: after US-0005's file sweep has run for a project,
compute whether the project itself is stale and, if so, soft-delete it (set `deleted_at`) without
touching any of its data. This is sweep logic only; US-0007 wires it to the loop and the CLI,
US-0004's gate is what makes the resulting `deleted_at` actually block access.

## Technical Context

### Stack
Rust 2024.

### Relevant File Structure
```
crates/core/src/
  purge.rs            (add the project-sweep function here, alongside US-0005's file-sweep fn)
  storage/admin.rs     (the conditional UPDATE query for project.deleted_at)
```

### Existing Patterns
`project.index_mode`'s column-migration precedent (`storage/admin.rs:63-70`) is the schema
precedent for `deleted_at` (already added in US-0002); this story adds the first query that
writes to it.

### Decisions That Govern This Story
- **DEC-004** (Section 17): project staleness reference = `max(project.created_at, max(atime)
  over live files)`, never an independent project-level activity timestamp — metadata-only
  operations must not keep a project artificially "alive".
- **DEC-007**: project soft-delete is two-phase (soft-delete, reversible → grace period →
  permanent removal), with its own grace clock distinct from BL-0003's file-trash retention.
- **DEC-013**: no new locking primitive — project soft-delete uses a conditional
  `UPDATE ... WHERE deleted_at IS NULL`, so a double-trigger from the loop and an on-demand CLI
  run (US-0007) collapses to a no-op rather than racing.

### Applicable NFRs
None beyond Section 7.4 already covered by DEC-013's idempotency design.

## Functional Requirements

### FR-NEW-008 [EARS-S]: Internal project soft-delete sweep
> WHILE a project has `autopurge_enabled && use_internal_purge && project_retention_days.is_some()`
> THE system SHALL, on every background sweep cycle after FR-NEW-007 has run for that project,
> soft-delete the project (set `project.deleted_at = now` via `UPDATE ... WHERE deleted_at IS
> NULL`) when `now - max(project.created_at, max(atime) over its live files) >
> project_retention_days`.

- **Inputs / Outputs:** input `project_id`; output: boolean (soft-deleted this cycle or not),
  logged.
- **Business Rules:**
  - Falls back to `created_at` alone when the project has no live files.
  - The conditional update makes a repeated trigger a no-op: `deleted_at` never advances past its
    first-set value.
  - Soft-delete never deletes any underlying data — only sets the flag.
  - "Live file" excludes the trash subtree, same definition as FR-NEW-007 (US-0005).

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Project with seeded `created_at` and file `atime`s | Direct DB seed | fixture | ready |

### E2E-NEW-032: Happy path, project soft-deleted
- **Category:** happy
- **Scenario:** SC-003
- **Requirements:** FR-NEW-008
- **Steps:** Given `created_at = now - 50 days`, most-recent live-file `atime = now - 45 days`,
  `project_retention_days = 30` (reference = `max(now-50d, now-45d) = now-45d`, age 45 > 30) /
  When sweep runs / Then `project.deleted_at` is set to "now" (non-null, within a few seconds).
- **Priority:** Critical

### E2E-NEW-033: Boundary, exactly at threshold — not soft-deleted
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-008
- **Steps:** Reference age exactly `= project_retention_days` / When sweep runs / Then NOT
  soft-deleted (strict `>`).
- **Priority:** Critical

### E2E-NEW-034: Boundary, just past threshold — soft-deleted
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-008
- **Steps:** Reference age `= project_retention_days + 1 second` / When sweep runs / Then
  soft-deleted.
- **Priority:** High

### E2E-NEW-035: No live files — falls back to created_at alone
- **Category:** edge (data variation)
- **Requirements:** FR-NEW-008
- **Steps:** Given project has NO live files, only `created_at = now - 40 days`,
  `project_retention_days = 30` / When sweep runs / Then reference falls back to `created_at`
  alone, age 40 > 30, soft-deleted.
- **Priority:** High

### E2E-NEW-036: `max()` semantics — recent activity resets staleness
- **Category:** edge (data variation) — the crux of FR-NEW-008
- **Requirements:** FR-NEW-008
- **Steps:** Given `created_at = now - 200 days`, one live file `atime = now - 5 days`,
  `project_retention_days = 30` / When sweep runs / Then reference = `max(created_at, max atime) =
  now - 5 days`, age 5 < 30, NOT soft-deleted — recent activity resets the clock even for an old
  project.
- **Priority:** Critical

### E2E-NEW-037: use_internal_purge=false gates this off too
- **Category:** state (regression)
- **Requirements:** FR-NEW-008
- **Steps:** Given `use_internal_purge=false`, `autopurge_enabled=true`, otherwise-stale project /
  When sweep runs / Then NOT soft-deleted (gated the same way as the file sweep).
- **Priority:** Critical

### E2E-NEW-038: Repeated trigger does not advance deleted_at
- **Category:** idempotency
- **Requirements:** FR-NEW-008
- **Steps:** Given project already has `deleted_at` set from a prior sweep / When sweep runs again
  on the same stale project / Then `deleted_at`'s VALUE is unchanged (not bumped to a new "now") —
  assert value equality, not just non-null.
- **Priority:** Critical

### E2E-NEW-039: Soft-delete never deletes data
- **Category:** side-effect
- **Requirements:** FR-NEW-008
- **Steps:** Given project soft-deleted / When checked immediately after / Then all node rows and
  blob refs still exist (row counts unchanged pre/post).
- **Priority:** Critical

## Constraints

### Files Not to Touch
`storage/admin.rs`'s `require_member` (already changed in US-0004 — this story only adds the
UPDATE query, never touches the read-gate).

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not add a lock around the soft-delete decision — DEC-013 explicitly relies on the conditional
`UPDATE ... WHERE deleted_at IS NULL` instead.

### Scope Boundary
Setting `deleted_at` only. Permanent removal after the grace period is US-0008's story
(FR-NEW-012), not this one.

## Non Regression

### Existing Tests That Must Pass
`storage/admin.rs` existing project tests, unaffected since this story only adds a new
conditional UPDATE path, never touches existing queries.

### Behaviors That Must Not Change
A project with `deleted_at = NULL` behaves exactly as before this story.

### API Contracts to Preserve
None — internal sweep logic only, no tool/CLI surface yet.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
