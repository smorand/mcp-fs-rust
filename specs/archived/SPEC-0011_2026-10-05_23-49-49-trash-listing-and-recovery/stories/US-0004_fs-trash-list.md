# US-0004: `fs.trash_list` tool

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 4
> Depends On: US-0001, US-0002, US-0003
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective
Expose the first MCP tool that makes trash visible: `fs.trash_list`. It reads `trash_entries` for a
project, paginated and filterable by path prefix, and also backfills any legacy trashed file that
predates this feature (one that exists on disk under the trash directory but has no `trash_entries`
row yet) the first time anyone looks. This story also establishes the ACL pattern (plain
`state.authorize`, no per-file ownership) that US-0005 will reuse without re-deciding it.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0 (`#[tool]` macro), axum.

### Relevant File Structure
```
crates/core/src/tools/trash.rs   # NEW — typed functions, called by the #[tool] method and later by the GUI
crates/core/src/mcp/server.rs    # the #[tool] method for fs.trash_list goes here
```

### Existing Patterns
Every `fs.*` tool handler follows one fixed shape (AGENTS.md Conventions, verbatim): *"Every
`fs.*`/`git.*` handler: `state.authorize(mount_id, person)` first, then normalize every path through
`state.safety.normalize_path`, then call the engine in `core::fs_ops`. Never reimplement an operation
in the tool layer... both layers are thin adapters over it."* Follow this exact shape:
```rust
// crates/core/src/mcp/server.rs — the pattern every existing #[tool] method uses
#[tool(description = "...")]
async fn fs_trash_list(&self, /* typed args */) -> Result<CallToolResult, McpError> {
    self.state.authorize(&mount_id, &self.person).await?;
    let result = tools::trash::trash_list(&self.state, &mount_id, &path_prefix, limit, offset).await?;
    Ok(json_tool_result(result))
}
```
Pagination/listing style precedent to follow: `fs.audit_log` uses `limit:integer=20` with a cursor
(`since`), `fs.read` returns `next_offset`. This spec's own `fs.trash_list` deliberately uses
`limit`/`offset` instead (a documented deviation, not an inconsistency to "fix" — see spec §Round 4,
Q1/confirmed in conversation), matching exactly the parameter names and defaults in FR-NEW-007 below.

A `LIKE` pattern for the `path_prefix` filter is built with `descendant_pattern` /
`Dialect::escape_like_literal` (AGENTS.md Conventions) — never hand-rolled.

### Data Model (excerpt)
Reads `trash_entries` (US-0001's table) via its `list_trash_entries` trait method. No schema change
in this story.

### Decisions That Govern This Story
- **DEC-003** (§17): "`trash_entries` starts empty; legacy trashed files are backfilled lazily and
  idempotently on first `fs.trash_list`/`fs.trash_restore` touch, with best-effort `deleted_at`
  (parsed epoch prefix, else now)... no separate startup migration job." This story implements the
  `fs.trash_list` half of that decision (FR-NEW-011).
- **DEC-004** (§17): "No per-file ownership-based ACL; any project member can list/restore any
  trashed file in that project... no per-file attribution exists anywhere in the codebase today."
- **DEC-005** (§17): "Platform admin status grants no implicit file access; the existing
  `state.authorize` membership gate applies unchanged to the new tools."

### Applicable NFRs
- **7.1 Performance** (§7.1): "`fs.trash_list` is bounded by `limit` (default 200, max unspecified
  beyond `i32` range — no additional cap requested)."
- **7.2 Security** (§7.2): "No new authentication mechanism. Authorization reuses `state.authorize`
  exactly as every other `fs.*` tool."
- **7.5 Observability** (§7.5): "`fs.trash_list`/`fs.trash_restore` calls at INFO (API calls)."
- **7.7 Scalability** (§7.7): the `(volume_id, deleted_at)` secondary index US-0001 added supports
  this story's `ORDER BY deleted_at DESC`.

### Bounded Context
**Trash tracking** (§4.5).

## Functional Requirements

### FR-NEW-007: `fs.trash_list`
- **EARS:** [EARS-E] "WHEN `fs.trash_list(mount_id:string, path_prefix:string="", limit:integer=200,
  offset:integer=0)` is called THE system SHALL return `{"entries": [...], "total": N}`, where each
  entry is `{trash_path, original_path, size, kind, deleted_at, deleted_by, purge_in_days}`, filtered
  to entries whose `original_path` starts with `path_prefix`, ordered by `deleted_at` descending, and
  paginated by `limit`/`offset`."
- **Inputs / Outputs:** `mount_id:string` (required), `path_prefix:string=""`, `limit:integer=200`,
  `offset:integer=0` → `{"entries": [{trash_path, original_path, size, kind, deleted_at, deleted_by,
  purge_in_days}, ...], "total": N}`.
- **Business Rules:** `purge_in_days` is `file_retention_days - floor((now - deleted_at)/86400)` when
  the project's `PurgeConfig.file_retention_days` is set (can be negative), else `null`. Before
  building the response, runs the backfill of FR-NEW-011 for every untracked node directly under the
  project's `trash_dir`. Authorization via `state.authorize(mount_id, person)`.
- **Exact names:** tool `fs.trash_list`; response keys `entries`, `total`, and the per-entry keys
  above; error codes `ERR_PROJECT_NOT_FOUND`, `ERR_FORBIDDEN`, `ERR_INVALID_ARGUMENT`.

### FR-NEW-008: `fs.trash_list` input validation
- **EARS:** [EARS-O] "IF `limit` or `offset` is negative THEN THE system SHALL return
  `ERR_INVALID_ARGUMENT`."

### FR-NEW-011: lazy backfill of legacy trash entries
- **EARS:** [EARS-E] "WHEN `fs.trash_list` or `fs.trash_restore` encounters a node directly under
  the project's `trash_dir` with no corresponding `trash_entries` row THE system SHALL insert one,
  idempotently, with `original_path` reconstructed by stripping the leading `{epoch_ms}__` prefix
  from the trash path's final segment and replacing remaining `__` with `/`, `deleted_at` parsed
  from that `epoch_ms` prefix (falling back to the current time if unparseable), and
  `deleted_by: null`."
- **Business Rules:** idempotent — calling this twice on the same untracked node inserts exactly
  one row, never a duplicate. This reconstruction is best-effort: a legacy `original_path` that
  itself contained a literal `__` substring is ambiguous with a legacy path whose separator was `/`
  (both flatten identically) — a known, accepted limitation of pre-existing data only. Implement the
  backfill as a shared helper both this story and US-0005 call; US-0005 restores by looking up what
  this story's backfill wrote, it does not re-implement the reconstruction logic.

### FR-NEW-016: ACL on trash tools
- **EARS:** [EARS-E] "WHEN `fs.trash_list` or `fs.trash_restore` is called THE system SHALL
  authorize the caller via `state.authorize(mount_id, person)`, the same membership gate every other
  `fs.*` tool uses, with no additional per-file ownership check."
- **Business Rules:** `deleted_by` is informational display only; it SHALL NOT gate
  `fs.trash_restore` (this story sets the pattern US-0005 reuses by citation, not re-decision).
  Platform admin status SHALL NOT grant implicit access to a project the admin is not a member of.

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| legacy trash node | a node seeded directly via the volume client under `trash_dir`, no `trash_entries` row | new test fixture this story adds | pending |
| multi-project membership fixture | projects with varying membership for ACL tests | existing project/admin test fixtures | ready |

### E2E-NEW-406: `fs.trash_list` orders by `deleted_at` descending
- **Category:** Happy / **Scenario:** SC-001 / **Requirements:** FR-NEW-007
- **Preconditions:** project `proj-list-1`, three deletes in sequence: `/a.txt` at `t0`, `/b.txt` at
  `t0+1s`, `/c.txt` at `t0+2s`.
- **Steps:** Given the three deletes above / When `fs.trash_list(mount_id="proj-list-1",
  path_prefix="", limit=200, offset=0)` is called / Then `entries` is `[c.txt-entry, b.txt-entry,
  a.txt-entry]` in that order, `total=3`, each entry containing all seven keys.
- **Priority:** Critical

### E2E-NEW-407: `fs.trash_list` on an unknown project
- **Category:** Failure / **Scenario:** SC-001 / EXC-001a / **Requirements:** FR-NEW-007
- **Steps:** Given no project `nonexistent-proj` exists / When `alice` calls
  `fs.trash_list(mount_id="nonexistent-proj", ...)` / Then the call fails with
  `ERR_PROJECT_NOT_FOUND`.
- **Priority:** Critical

### E2E-NEW-408: `fs.trash_list` forbidden for a non-member
- **Category:** Failure / **Scenario:** SC-001 / EXC-001b / **Requirements:** FR-NEW-016
- **Steps:** Given project `proj-list-1` with member `alice` only / When `eve` (not a member, not a
  platform admin) calls `fs.trash_list(mount_id="proj-list-1", ...)` / Then the call fails with
  `ERR_FORBIDDEN`.
- **Priority:** Critical

### E2E-NEW-409: empty trash
- **Category:** Edge / **Scenario:** SC-001 / EXC-001c / **Requirements:** FR-NEW-007
- **Steps:** Given project `proj-empty` with no deletions ever performed / When
  `fs.trash_list(mount_id="proj-empty", path_prefix="", limit=200, offset=0)` is called / Then the
  response is `{"entries": [], "total": 0}`, no error.
- **Priority:** High

### E2E-NEW-410: pagination boundary
- **Category:** Edge / **Scenario:** SC-001 / **Requirements:** FR-NEW-007
- **Preconditions:** project `proj-page` with exactly 5 trashed files `f1.txt`..`f5.txt` (`f5` most
  recent).
- **Steps:** Given the 5 entries / When `fs.trash_list(mount_id="proj-page", path_prefix="",
  limit=2, offset=4)` is called / Then `entries` has exactly 1 element (`f1.txt`'s entry), `total=5`
  / And when called again with `limit=2, offset=5`, `entries` is `[]`, `total=5`.
- **Priority:** High

### E2E-NEW-411: `path_prefix` matches nothing
- **Category:** Edge / **Scenario:** SC-001 / EXC-001c / **Requirements:** FR-NEW-007
- **Steps:** Given project `proj-list-1` with entries under `/data/` / When
  `fs.trash_list(mount_id="proj-list-1", path_prefix="/nomatch/", limit=200, offset=0)` is called /
  Then the response is `{"entries": [], "total": 0}`, not an error.
- **Priority:** High

### E2E-NEW-412: `path_prefix` filters correctly
- **Category:** Happy / **Scenario:** SC-001 / **Requirements:** FR-NEW-007
- **Steps:** Given project `proj-list-1` with trashed `/data/a.txt` and `/other/b.txt` / When
  `fs.trash_list(mount_id="proj-list-1", path_prefix="/data/", limit=200, offset=0)` is called /
  Then `entries` contains exactly the `/data/a.txt` entry.
- **Priority:** High

### E2E-NEW-413: backfill of a pre-existing untracked trash node
- **Category:** Edge / **Scenario:** cross / **Requirements:** FR-NEW-011
- **Preconditions:** project `proj-backfill`, node seeded at
  `/.mcp_trash/1700000000000__legacy__report.txt` with no `trash_entries` row.
- **Steps:** Given the untracked node / When `fs.trash_list(mount_id="proj-backfill", path_prefix="",
  limit=200, offset=0)` is called / Then the response includes an entry with
  `original_path="legacy/report.txt"` and `deleted_at` derived from epoch `1700000000000` ms / And a
  `trash_entries` row now exists in the DB for that `trash_path` with `deleted_by=null` (DB query).
- **Priority:** Critical

### E2E-NEW-414: backfill is idempotent
- **Category:** Idempotency / **Scenario:** cross / **Requirements:** FR-NEW-011
- **Preconditions:** state immediately after E2E-NEW-413's first call.
- **Steps:** Given one backfilled row already exists / When `fs.trash_list(mount_id="proj-backfill",
  ...)` is called a second time / Then the response still contains exactly one entry for
  `legacy/report.txt` / And a DB query confirms exactly one `trash_entries` row for that `trash_path`.
- **Priority:** Critical

### E2E-NEW-415: `purge_in_days` can go negative
- **Category:** Edge / **Scenario:** cross / **Requirements:** FR-NEW-007
- **Preconditions:** project `proj-retention`, `PurgeConfig.file_retention_days = Some(3)`, an entry
  with `deleted_at` backdated 10 days.
- **Steps:** Given the backdated entry / When `fs.trash_list(mount_id="proj-retention", ...)` is
  called / Then the entry's `purge_in_days` is `-7`.
- **Priority:** High

### E2E-NEW-416: `purge_in_days` is `null` without retention configured
- **Category:** Edge / **Scenario:** cross / **Requirements:** FR-NEW-007
- **Preconditions:** project `proj-no-retention`, `PurgeConfig.file_retention_days = None`, a file
  deleted 1 day ago.
- **Steps:** Given the entry / When `fs.trash_list(mount_id="proj-no-retention", ...)` is called /
  Then the entry's `purge_in_days` is `null`.
- **Priority:** High

### E2E-NEW-428: platform admin without membership is forbidden
- **Category:** Failure / ACL / **Scenario:** ACL / **Requirements:** FR-NEW-016
- **Preconditions:** project `proj-acl` with member `alice` only; platform admin `admin-carol` not a
  member.
- **Steps:** Given `admin-carol` is a platform admin but not a member of `proj-acl` / When
  `admin-carol` calls `fs.trash_list` on `proj-acl` / Then the call fails with `ERR_FORBIDDEN`.
- **Priority:** High

### E2E-NEW-438: negative `limit`/`offset` rejected
- **Category:** Failure / **Scenario:** SC-001 / EXC-001d / **Requirements:** FR-NEW-008
- **Steps:** Given project `proj-list-1` / When `fs.trash_list(mount_id="proj-list-1", path_prefix="",
  limit=-1, offset=0)` is called / Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

### E2E-NEW-442: a removed member loses trash access
- **Category:** Failure / ACL / **Scenario:** ACL / **Requirements:** FR-NEW-016
- **Preconditions:** project `proj-acl-2`, member `dave` deletes `/x.txt`, then `admin.remove_member`
  removes `dave` from `proj-acl-2`.
- **Steps:** Given `dave` is no longer a member / When `dave` calls
  `fs.trash_list(mount_id="proj-acl-2", ...)` / Then the call fails with `ERR_FORBIDDEN` — "any
  current project member" does not mean "whoever was ever a member".
- **Priority:** High

### E2E-NEW-451: negative `offset` alone is rejected
- **Category:** Failure / **Scenario:** SC-001 / **Requirements:** FR-NEW-008
- **Steps:** Given project `proj-list-1` / When `fs.trash_list(mount_id="proj-list-1", path_prefix="",
  limit=10, offset=-1)` is called / Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

### E2E-NEW-452: `limit=0` is accepted, not treated as negative
- **Category:** Happy / **Scenario:** SC-001 / **Requirements:** FR-NEW-008
- **Preconditions:** project `proj-list-1` with at least one trashed entry.
- **Steps:** Given at least one entry exists / When `fs.trash_list(mount_id="proj-list-1",
  path_prefix="", limit=0, offset=0)` is called / Then the call succeeds with `{"entries": [],
  "total": N}` (N > 0) — `limit=0` returns no rows but is not an error, distinguishing it from a
  negative value.
- **Priority:** Low

## Constraints

### Files Not to Touch
- `crates/core/src/trash_screen.rs` does not exist yet (US-0006) — nothing to touch here.
- `crates/core/src/core/fs_ops.rs`, `purge.rs` — already correct from US-0002/US-0003.

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not hand-roll the `LIKE` pattern for `path_prefix` — use `descendant_pattern`/
  `Dialect::escape_like_literal`.
- Do not gate the backfill behind a feature flag or a separate admin trigger — it is unconditional
  and automatic per FR-NEW-011.

### Scope Boundary
- `fs.trash_restore` is US-0005's. This story builds `fs.trash_list` and the shared backfill helper
  FR-NEW-011 requires, nothing else.

## Non Regression

### Existing Tests That Must Pass
- `cargo test --workspace` stays green; no existing tool's schema or behavior changes.

### Behaviors That Must Not Change
- n/a — new tool.

### API Contracts to Preserve
- n/a — new tool, added to `TOOL_CONTRACT.txt` only in US-0008.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
