# US-0012: Browser screen `/app/deleted-projects`

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 12
> Depends On: US-0010, US-0011
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

The one UI surface this spec adds: an admin-session-gated page listing soft-deleted projects with
an undelete action, modeled directly on the existing `/app/tokens` screen's auth/CSRF convention.

## Technical Context

### Stack
Rust 2024, axum, the existing session-cookie + CSRF screen pattern.

### Relevant File Structure
```
crates/core/src/
  deleted_projects_screen.rs   (new, modeled on token_screen.rs)
  app.rs                       (mount the new route, alongside the existing /app/tokens mount)
```

### Existing Patterns
`token_screen.rs` (`/app/tokens`): `GET /app/tokens` issues a `csrf_token`, `POST
/app/tokens/revoke` consumes it, cookie-authenticated POSTs require CSRF, bearer-authenticated
ones are exempt. This story follows that shape exactly: `GET /app/deleted-projects` lists,
`POST /app/deleted-projects/undelete` (or the exact route `token_screen.rs`'s convention
suggests) consumes a CSRF token and calls US-0011's tool function in-process.

### Decisions That Govern This Story
None new — this story is a direct application of the `token_screen.rs` precedent cited in the
parent spec's FR-NEW-017.

### Applicable NFRs
Section 7.3 (Usability): mirrors `/app/tokens`'s look and interaction pattern for consistency.

## Functional Requirements

### FR-NEW-017 [EARS-U]: Browser screen `/app/deleted-projects`
> The system SHALL serve an admin-session-gated `/app/deleted-projects` screen, following the
> same auth and CSRF conventions as the existing `/app/tokens` screen (`token_screen.rs`), listing
> every soft-deleted project visible to the caller with an undelete action that calls FR-NEW-015
> (never a second implementation).

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Admin session cookie, valid CSRF token | Same fixtures as existing `/app/tokens` tests | fixture | ready |

### E2E-NEW-069: Happy path, lists soft-deleted projects
- **Category:** happy
- **Scenario:** SC-005
- **Requirements:** FR-NEW-017
- **Steps:** Given platform admin has a valid admin session (cookie, same mechanism as
  `token_screen.rs`) / When `GET /app/deleted-projects` / Then 200, HTML body lists each
  soft-deleted project with an undelete control per row.
- **Priority:** Critical

### E2E-NEW-070: Unauthenticated access rejected
- **Category:** failure (auth)
- **Requirements:** FR-NEW-017
- **Steps:** Given no session / invalid session / When `GET /app/deleted-projects` / Then
  redirected/401, matching `/app/tokens`'s unauthenticated behavior exactly.
- **Priority:** Critical

### E2E-NEW-071: Undelete action calls the real tool
- **Category:** happy (action parity)
- **Scenario:** SC-006
- **Requirements:** FR-NEW-017
- **Steps:** Given the page lists `proj1` (soft-deleted) / When the undelete route is posted with
  a valid CSRF token issued by the page / Then succeeds, `proj1.deleted_at` is NULL — cross-check
  against US-0011's E2E-NEW-063 to confirm identical effect (same tool function, not a second
  implementation).
- **Priority:** Critical

### E2E-NEW-072: CSRF rejected
- **Category:** failure (CSRF)
- **Requirements:** FR-NEW-017
- **Steps:** Given the undelete POST is sent via the cookie-authenticated path without a valid
  CSRF token / Then rejected (403 or equivalent), no state change.
- **Priority:** High

### E2E-NEW-073: Empty state
- **Category:** edge (empty)
- **Requirements:** FR-NEW-017
- **Steps:** Given zero soft-deleted projects / When `GET /app/deleted-projects` / Then 200,
  renders an empty/placeholder list, no error.
- **Priority:** Low

## Constraints

### Files Not to Touch
`tools/admin.rs`'s undelete tool function itself (US-0011) — this screen calls it, never
reimplements its logic.

### Dependencies Not to Add
None — reuses `token_screen.rs`'s existing session/CSRF machinery.

### Patterns to Avoid
Do not write a second undelete implementation inside the screen handler — it must call US-0011's
tool function in-process (E2E-NEW-071 exists to catch a divergence here).

### Scope Boundary
This screen only; no other admin UI page is added.

## Non Regression

### Existing Tests That Must Pass
`/app/tokens`'s existing tests, unaffected — this is a new, separate route.

### Behaviors That Must Not Change
`/app/tokens`'s auth/CSRF behavior, untouched.

### API Contracts to Preserve
None affected.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
