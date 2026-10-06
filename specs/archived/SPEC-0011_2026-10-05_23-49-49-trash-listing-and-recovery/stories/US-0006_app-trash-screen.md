# US-0006: `/app/trash` GUI screen

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 6
> Depends On: US-0004, US-0005
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective
Add a browser screen, `/app/trash`, that lets a member browse and restore trashed files without
using an MCP client: a project picker when no project is selected, and the trash listing with a
restore button per row once one is. It renders the exact same data `fs.trash_list`/`fs.trash_restore`
produce — it never reimplements listing or restoring.

## Technical Context

### Stack
Rust 2024, axum, hand-built HTML (no templating engine, no JS framework), session-cookie identity.

### Relevant File Structure
```
crates/core/src/trash_screen.rs           # NEW — modeled on deleted_projects_screen.rs
crates/core/src/deleted_projects_screen.rs # the pattern to mirror, read it first
crates/core/src/app.rs                    # mount the new router here
```

### Existing Patterns
`deleted_projects_screen.rs` is the exact pattern to copy the shape of. Its router:
```rust
// crates/core/src/deleted_projects_screen.rs:41-47
pub fn router(state: Arc<AppState>) -> Router {
    let screen_state = ScreenState { app: state, csrf: Arc::new(CsrfStore::default()) };
    Router::new()
        .route("/app/deleted-projects", get(show))
        .route("/app/deleted-projects/undelete", post(undelete))
        .with_state(screen_state)
}
```
Your router has the same shape, with its own `CsrfStore` (a screen-scoped store, not shared with
`deleted_projects_screen.rs`'s):
```rust
pub fn router(state: Arc<AppState>) -> Router {
    let screen_state = ScreenState { app: state, csrf: Arc::new(CsrfStore::default()) };
    Router::new()
        .route("/app/trash", get(show))
        .route("/app/trash/restore", post(restore))
        .with_state(screen_state)
}
```
Identity resolution: reuse `resolve_person` exactly as `deleted_projects_screen.rs` does (header,
then cookie, returning which `IdentitySource` authenticated the request).

CSRF: issue a fresh token only once the page is about to render (`state.csrf.issue(&person)`,
mirroring `deleted_projects_screen.rs:111`); enforce it on the restore POST **only when** the
identity source was the cookie (`deleted_projects_screen.rs:167-178`) — a header/bearer-authenticated
POST carries no ambient credential and is exempt. On mismatch, `403 FORBIDDEN`
(`deleted_projects_screen.rs:180-186`'s `forbidden_csrf()` pattern).

HTML rendering: raw `String`/`format!`, `escape_html` on every interpolated value
(`deleted_projects_screen.rs:244-261`'s `page_shell` pattern), `data-*` attributes on rows for test
scraping.

**New membership source this story needs, not present in `deleted_projects_screen.rs`:**
`AdminBackend::list_projects_for(person) -> Result<Vec<Project>>` (`storage/traits.rs:209`,
implemented at `storage/admin.rs:286`) — the same source `admin.list_projects` already uses. Call it
directly for the picker; do not try to reuse `deleted_projects_screen.rs`'s own membership check,
which iterates already-deleted projects and is not reusable for this purpose.

### Data Model (excerpt)
No schema change. Calls into `tools::trash::trash_list`/`tools::trash::trash_restore` (US-0004/0005)
and `AdminBackend::list_projects_for` (existing).

### Decisions That Govern This Story
No new `DEC-XXX` governs this story specifically — it implements FR-NEW-012/018 by citing the
screen-building pattern already established by `deleted_projects_screen.rs`, not a new decision.

### Applicable NFRs
- **7.2 Security**: "The GUI reuses the existing header-then-cookie identity resolution and CSRF
  convention verbatim from `deleted_projects_screen.rs`."
- **7.3 Usability**: "No new accessibility/i18n requirements beyond what `deleted_projects_screen.rs`
  already establishes (plain escaped HTML, no JS framework)."

### Bounded Context
**Browser surface** (§4.5): "The `/app/trash` screen, session identity and CSRF." Key entities:
`ScreenState`, `CsrfStore`.

## Functional Requirements

### FR-NEW-012: `/app/trash` GUI screen
- **EARS:** [EARS-U] "The system SHALL expose `GET /app/trash?mount_id=<id>` and `POST
  /app/trash/restore`, modeled on `deleted_projects_screen.rs`'s identity resolution and CSRF
  convention, rendering the result of the same `fs.trash_list`/`fs.trash_restore` logic the MCP
  tools call — never a second implementation."
- **Exact names:** routes `GET /app/trash`, `POST /app/trash/restore`; query param `mount_id`; new
  module `crates/core/src/trash_screen.rs`, mounted in `app.rs` alongside
  `deleted_projects_screen::router`.
- **Business Rules:** CSRF enforced on the restore POST only when the identity source was the
  cookie; on authorization failure for the requested `mount_id`, the screen renders an error page
  (the `ERR_FORBIDDEN` the underlying call raises), not an empty list.

### FR-NEW-018: `/app/trash` with no `mount_id`
- **EARS:** [EARS-O] "IF `GET /app/trash` is requested without a `mount_id` query parameter THEN
  THE system SHALL render a picker listing the viewer's own project memberships, obtained via
  `AdminBackend::list_projects_for(person)` (`storage/traits.rs:209`, implemented at
  `storage/admin.rs:286`), each linking to `GET /app/trash?mount_id=<id>`, instead of calling
  `fs.trash_list` or rendering an error."
- **Business Rules:** this is a new call site for `list_projects_for` inside `trash_screen.rs`.
  IF `list_projects_for` returns an empty list THEN the picker SHALL render with an explicit
  empty-state message (e.g. "you are not a member of any project"), HTTP 200, not an error.
- **Exact names:** route `GET /app/trash` with `mount_id` absent; trait method
  `AdminBackend::list_projects_for`.

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace` (HTTP-client-driven tests against the test
> server, matching the existing pattern for `deleted_projects_screen.rs`'s own tests).

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| multi-project membership fixture | a viewer who is a member of 0, 1, or 2 projects | existing admin fixtures | ready |
| cookie-authenticated test session | ambient-credential session for CSRF tests | existing `deleted_projects_screen.rs` test pattern | ready |

### E2E-NEW-432: `/app/trash` renders entries
- **Category:** GUI / Happy / **Scenario:** SC-006 / **Requirements:** FR-NEW-012
- **Preconditions:** project `proj-gui-1`, viewer-member `alice`, two files trashed.
- **Steps:** Given the two trashed entries and `alice`'s authenticated session / When `alice`
  performs `GET /app/trash?mount_id=proj-gui-1` / Then the response is `200`, HTML containing both
  entries' `original_path` values (HTML-escaped) and a restore form per row.
- **Priority:** High

### E2E-NEW-433: restore POST without a valid CSRF token is rejected
- **Category:** GUI / Failure (security) / **Scenario:** SC-006 / **Requirements:** FR-NEW-012
- **Preconditions:** project `proj-gui-1` with a trashed entry, `alice` authenticated via the
  cookie.
- **Steps:** Given a cookie-authenticated session with no/invalid CSRF token / When `alice`'s
  session POSTs a restore to `/app/trash/restore` with a missing or wrong CSRF token / Then the
  response is `403 FORBIDDEN` / And the `trash_entries` row is NOT deleted (DB query).
- **Priority:** Critical

### E2E-NEW-434: non-member `mount_id` renders an error page
- **Category:** GUI / Failure / **Scenario:** SC-006 / **Requirements:** FR-NEW-012
- **Preconditions:** project `proj-gui-1` with member `alice` only.
- **Steps:** Given `bob` is authenticated but not a member of `proj-gui-1` / When `bob` performs
  `GET /app/trash?mount_id=proj-gui-1` / Then the response renders the `ERR_FORBIDDEN` error page
  (not an empty success list).
- **Priority:** High

### E2E-NEW-435: GUI restore uses the same underlying function as the MCP tool
- **Category:** GUI / Side Effect / **Scenario:** SC-006 / **Requirements:** FR-NEW-012
- **Preconditions:** two identical trashed entries in two otherwise-identical projects, one
  restored via the GUI, one via `fs.trash_restore` directly.
- **Steps:** Given the two identical setups / When one is restored via the GUI POST and the other
  via the MCP tool / Then the resulting DB/filesystem state (restored path, collision-suffix
  behavior) is identical between the two (direct state comparison), confirming no divergent
  reimplementation.
- **Priority:** Low

### E2E-NEW-455: GUI renders an empty state for a project with no trashed files
- **Category:** GUI / Edge / **Scenario:** SC-006 / **Requirements:** FR-NEW-012
- **Preconditions:** project `proj-gui-empty` with member `alice`, no deletions ever performed.
- **Steps:** Given the empty-trash project / When `alice` performs `GET
  /app/trash?mount_id=proj-gui-empty` / Then the response is `200` with an explicit empty-state
  message in the HTML (e.g. "no trashed files"), not an error and not a blank/broken table.
- **Priority:** Medium

### E2E-NEW-462: `/app/trash` with no `mount_id` renders a project picker
- **Category:** GUI / Happy / **Scenario:** SC-006 / **Requirements:** FR-NEW-018
- **Preconditions:** `alice` is a member of `proj-gui-1` and `proj-gui-2`.
- **Steps:** Given `alice`'s authenticated session / When `alice` performs `GET /app/trash` (no
  `mount_id`) / Then the response is `200` with HTML listing links to `GET
  /app/trash?mount_id=proj-gui-1` and `GET /app/trash?mount_id=proj-gui-2` (her two memberships),
  and `fs.trash_list` is not invoked (instrumentation/call-count assertion).
- **Priority:** High

### E2E-NEW-463: picker with zero memberships
- **Category:** GUI / Edge / **Scenario:** SC-006 / **Requirements:** FR-NEW-018
- **Preconditions:** `dave` is authenticated but a member of no project.
- **Steps:** Given `dave`'s authenticated session, zero memberships / When `dave` performs `GET
  /app/trash` (no `mount_id`) / Then the response is `200` with an explicit "you are not a member
  of any project" message (or equivalent), not an error.
- **Priority:** Low

## Constraints

### Files Not to Touch
- `crates/core/src/deleted_projects_screen.rs` — reference it, do not modify it (it is a separate,
  unrelated screen for soft-deleted projects).
- `crates/core/src/tools/trash.rs` — call its functions, do not duplicate their logic here.

### Dependencies Not to Add
- No templating engine, no JS framework — matches the existing screen's plain HTML approach.

### Patterns to Avoid
- Do not call `fs.trash_list`/`fs.trash_restore`'s logic a second, independent way — call the exact
  same `tools::trash` functions US-0004/US-0005 built.
- Do not enforce CSRF on a header-authenticated request — only on cookie-authenticated ones.

### Scope Boundary
- This story is the screen only. No MCP tool changes.

## Non Regression

### Existing Tests That Must Pass
- Every existing `deleted_projects_screen.rs` test stays green — this story adds a sibling screen,
  it does not touch that file.

### Behaviors That Must Not Change
- n/a — new screen.

### API Contracts to Preserve
- n/a — new routes.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
