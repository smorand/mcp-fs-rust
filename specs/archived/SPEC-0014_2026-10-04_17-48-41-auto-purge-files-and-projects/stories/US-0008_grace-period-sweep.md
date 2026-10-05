# US-0008: Grace-period sweep + CLI global mode

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 8
> Depends On: US-0007
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Closes the project lifecycle's second phase: once a soft-deleted project's grace period elapses,
permanently remove it via the existing, unmodified `admin.delete_project` cascade (FR-027). Runs
from the same background-loop cycle as US-0005/US-0006 (unconditionally, unlike them) and from
`mcp-fs purge`'s no-`--project` global mode (wired by US-0007, this story supplies the function it
calls).

## Technical Context

### Stack
Rust 2024.

### Relevant File Structure
```
crates/core/src/
  purge.rs           (grace-sweep function, calls the existing cascade)
  cli.rs             (no-`--project` branch calls this function — wire the call site here)
  tools/admin.rs     (admin.delete_project / delete_project — reused unchanged, read-only reference)
```

### Existing Patterns
`admin.delete_project` (`tools/admin.rs:228`) implements FR-027's cascade
(`specs/SPEC-0002.../spec.md:494-499`) — this story calls it exactly as-is, never a parallel or
simplified cascade.

### Decisions That Govern This Story
- **DEC-007** (Section 17): project soft-delete is two-phase; this story is phase two.
- **DEC-008**: grace period is one global config value (`project_purge_grace_days`, from
  US-0002), not per-project.

### Applicable NFRs
None beyond FR-027's own guarantees (Section 9.5: rollback = none beyond what FR-027 already
defines on cascade failure).

## Functional Requirements

### FR-NEW-012 [EARS-E]: Grace-period permanent removal
> WHEN a soft-deleted project's `now - deleted_at > project_purge_grace_days` (global config) THE
> system SHALL permanently remove it via the existing `admin.delete_project` cascade (FR-027,
> `specs/SPEC-0002.../spec.md:494-499`), unmodified, regardless of that project's
> `use_internal_purge` value.

- **Business Rules:** runs unconditionally once a project is soft-deleted — `use_internal_purge`
  only gates whether the *file/project* sweeps (US-0005/US-0006) run automatically, never whether
  an already-soft-deleted project eventually gets permanently removed.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Soft-deleted project with seeded `deleted_at` | Direct DB seed, independent of US-0006 having actually run | fixture | ready |

### E2E-NEW-049: Global CLI sweep removes an expired project, leaves another
- **Category:** happy (cross-scenario)
- **Scenario:** SC-004, SC-007
- **Requirements:** FR-NEW-012 (and the CLI dispatch from FR-NEW-011/US-0007)
- **Steps:** Given two projects: `proj2` soft-deleted `deleted_at = now - 31 days`
  (`project_purge_grace_days=30`); `proj3` soft-deleted `deleted_at = now - 5 days` / When
  `mcp-fs purge` (no `--project`) / Then `proj2` permanently removed (project row and every
  cascade-deleted row gone), `proj3` left soft-deleted unchanged; printed summary counts match.
- **Priority:** Critical

### E2E-NEW-053: Happy path, direct sweep
- **Category:** happy
- **Scenario:** SC-007
- **Requirements:** FR-NEW-012
- **Steps:** Given project soft-deleted `deleted_at = now - 31 days`,
  `project_purge_grace_days = 30` / When the grace-sweep function runs directly / Then project
  permanently removed via the cascade.
- **Priority:** Critical

### E2E-NEW-054: Boundary, exactly at grace period — not yet removed
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-012
- **Steps:** `deleted_at = now - 30 days` exactly / When grace-sweep runs / Then NOT yet removed
  (strict `>`).
- **Priority:** Critical

### E2E-NEW-055: Boundary, just past grace period — removed
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-012
- **Steps:** `deleted_at = now - 30 days - 1 second` / When grace-sweep runs / Then removed.
- **Priority:** High

### E2E-NEW-056: Unconditional on use_internal_purge
- **Category:** state — the distinguishing clause of FR-NEW-012
- **Requirements:** FR-NEW-012
- **Steps:** Given project soft-deleted with `use_internal_purge=false`, past grace period / When
  grace-sweep runs / Then STILL permanently removed.
- **Priority:** Critical

### E2E-NEW-057: Reuses FR-027's cascade exactly
- **Category:** side-effect
- **Requirements:** FR-NEW-012
- **Steps:** Given permanent removal just occurred / Then its side effects (trash entries
  cleared, git repo removed, blob refcounts decremented) match a baseline `admin.delete_project`
  call on an equivalent live project — proving the same cascade, not a parallel one.
- **Priority:** High

### E2E-NEW-080: Grace-sweep cascade failure surfaces like any FR-027 failure
- **Category:** failure
- **Requirements:** FR-NEW-012
- **Steps:** Given a soft-deleted project past grace period, cascade forced to fail partway
  (locked child row) / When grace-sweep invokes the cascade / Then the failure surfaces exactly as
  an unmodified `admin.delete_project` call would fail today — no silent swallow, no partial state
  beyond what FR-027's own cascade already guarantees.
- **Priority:** High

## Constraints

### Files Not to Touch
`tools/admin.rs`'s `delete_project` body itself — call it, never modify it.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not build a simplified or partial cascade "just for purge" — reuse `admin.delete_project`
exactly (DEC-007's design intent, and E2E-NEW-057 exists specifically to catch a divergence here).

### Scope Boundary
Grace-period evaluation and dispatch to the existing cascade only.

## Non Regression

### Existing Tests That Must Pass
Every existing `admin.delete_project` test (`tools/admin.rs:854,711,920`, `storage/admin.rs:442`,
`storage/conformance.rs:367,412`, `tools/git.rs:14986`) must pass unmodified — this story adds a
new caller, never a new cascade implementation.

### Behaviors That Must Not Change
A manually-triggered `admin.delete_project` call is completely unaffected by this story.

### API Contracts to Preserve
FR-027's cascade contract, byte-for-byte.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
