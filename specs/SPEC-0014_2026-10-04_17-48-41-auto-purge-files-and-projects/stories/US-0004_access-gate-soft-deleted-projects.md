# US-0004: Access gate for soft-deleted projects

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 4
> Depends On: US-0002
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective

Blocks every `fs.*` and `git.*` call (MCP and REST alike) against a project whose `deleted_at` is
set, reusing the existing `ERR_PROJECT_NOT_FOUND` error rather than minting a new code. This is
the single-point enforcement that makes "soft-deleted" actually mean something, ahead of
US-0006 (which starts setting `deleted_at`) and US-0011 (undelete).

## Technical Context

### Stack
Rust 2024, the shared membership gate.

### Relevant File Structure
```
crates/core/src/
  storage/admin.rs   (require_member — add the deleted_at check here, and nowhere else)
```

### Existing Patterns
- `state.rs:59 authorize()` calls `storage/admin.rs require_member` (around `:293`), and this is
  the one point both the MCP dispatch (`mcp/server.rs`) and the REST plane (`api/dataplane.rs:191`,
  `state.authorize(&mount, &person)`) go through — verified by reading both call sites in the
  parent spec's Section 2.1. Adding the check here, and only here, makes both surfaces inherit it
  automatically.
- `errors.rs:11 ERR_PROJECT_NOT_FOUND` is the existing constant to reuse — AGENTS.md documents the
  14 `ERR_*` codes as frozen ("no new code").

### Decisions That Govern This Story
- **DEC-009** (Section 17): reuse `ERR_PROJECT_NOT_FOUND` for calls against a soft-deleted
  project rather than minting a 15th code — from a normal caller's perspective a soft-deleted
  project is indistinguishable from an absent one.

### Applicable NFRs
Section 7.2 (Security): no new auth mechanism; this story only adds a predicate to an existing
gate.

## Functional Requirements

### FR-NEW-013 [EARS-E]: Blocked access to a soft-deleted project
> WHEN any `fs.*` or `git.*` tool, or its REST equivalent, is called against a project with
> `deleted_at` non-null THE system SHALL return `ERR_PROJECT_NOT_FOUND`, enforced in
> `storage/admin.rs require_member` (called from `state.rs:59 authorize()`), so both the MCP and
> REST surfaces inherit it from the one shared gate.

- **Business Rules:** applies unconditionally, including to a platform admin (no implicit file
  access, per existing convention). The `deleted_at` filter SHALL be applied only inside
  `require_member` and nowhere else: `admin.get_project`, `admin.require_owner`, and
  `state.rs:47 require_owner_or_admin` SHALL continue to operate on a soft-deleted project's row
  unchanged — otherwise `admin.set_purge_config` (US-0009), `admin.list_deleted_projects`
  (US-0010), and `admin.undelete_project` (US-0011) would be locked out of exactly the rows they
  exist to manage.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Project with `deleted_at` set directly via DB write | No dependency on US-0006's sweep actually running | fixture | ready |

### E2E-NEW-040: fs.read blocked on soft-deleted project
- **Category:** failure
- **Requirements:** FR-NEW-013
- **Steps:** Given project `proj1` with `deleted_at` set (non-null, seeded directly) / When
  member `alice` calls `fs.read(proj1, "any.txt")` / Then fails `ERR_PROJECT_NOT_FOUND` — assert
  the exact error code constant value, not just "an error".
- **Priority:** Critical

### E2E-NEW-041: every fs.*/git.* tool blocked, table-driven
- **Category:** failure (table-driven)
- **Requirements:** FR-NEW-013
- **Steps:** Table-driven over `fs.write`, `fs.list`, `fs.stat`, `fs.grep`, `git.status`,
  `git.log`, `git.push` against the same soft-deleted project. Each fails `ERR_PROJECT_NOT_FOUND`.
- **Priority:** Critical

### E2E-NEW-042: REST inherits the gate automatically
- **Category:** failure (cross-scenario, security)
- **Requirements:** FR-NEW-013
- **Steps:** Given soft-deleted project / When `GET /api/fs/{mount}/read?path=any.txt` / Then
  fails with `ERR_PROJECT_NOT_FOUND`'s mapped HTTP status, proving the gate lives in the one
  shared `require_member`, not a duplicated REST-only check. This is the test that would fail if
  `api/dataplane.rs` ever grew its own separate membership check.
- **Priority:** Critical

### E2E-NEW-043: live project unaffected (control)
- **Category:** happy (regression guard)
- **Requirements:** FR-NEW-013
- **Steps:** Given project NOT soft-deleted (`deleted_at` NULL) / When `fs.read` called by a
  member / Then succeeds normally.
- **Priority:** Critical

### E2E-NEW-044: platform admin gets no bypass
- **Category:** edge (auth state)
- **Requirements:** FR-NEW-013
- **Steps:** Given project soft-deleted, caller is the platform admin (not a member) / When admin
  calls `fs.read` / Then STILL fails `ERR_PROJECT_NOT_FOUND` — the gate is unconditional on
  project-live-state, consistent with "platform admin does NOT get implicit file access"
  (AGENTS.md).
- **Priority:** High

### E2E-NEW-045: gate re-reads live state (closes the loop, phase 1)
- **Category:** state transition
- **Requirements:** FR-NEW-013
- **Steps:** Given project soft-deleted, `fs.read` fails (E2E-NEW-040) / When `deleted_at` is
  cleared directly via DB write (simulating US-0011's undelete, without depending on that story
  being built) / Then `fs.read` retried succeeds — proving the gate reads live `deleted_at` on
  every call, never a cached value.
- **Priority:** Critical

## Constraints

### Files Not to Touch
`tools/admin.rs`, `mcp/server.rs`, `api/dataplane.rs` — the fix belongs in exactly one place
(`storage/admin.rs`), which both surfaces already route through.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not add a second `deleted_at` check in `api/dataplane.rs` "for clarity" or "defensively" — that
duplicates the gate the spec explicitly requires to be singular (FR-NEW-013's exact wording), and
E2E-NEW-042 exists specifically to catch that mistake.

### Scope Boundary
`require_member` only. `get_project`/`require_owner`/`require_owner_or_admin` are explicitly
out of scope for this story's change (see Business Rules above) — touching them is a regression,
not an enhancement.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace`; in particular every existing `require_member`/membership test in
`storage/admin.rs` and `tools/admin.rs` must still pass for projects with `deleted_at = NULL`
(the overwhelming majority, including every project created before this feature existed).

### Behaviors That Must Not Change
Every live project's access behavior is byte-identical to before this story.

### API Contracts to Preserve
`ERR_PROJECT_NOT_FOUND`'s existing meaning and HTTP status mapping — this story adds a new cause
for it, never a new code or a new mapping.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
