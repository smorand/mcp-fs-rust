# US-0007: `admin.create_project` retention parameters

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 7 (independent of US-0001..0006 — may be implemented in parallel; see spec FR-NEW-017
> step 6, "independent of steps 1-5")
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 3

## Objective
Let `admin.create_project` optionally set a project's purge/retention configuration at creation
time, instead of requiring a separate `admin.set_purge_config` call afterward. This story touches a
different subsystem (project provisioning) than every other story in this spec and has no
dependency on the trash feature itself.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0.

### Relevant File Structure
```
crates/core/src/mcp/server.rs      # CreateProjectArgs (struct at :1122) and admin_create_project (fn at :2079) — the PRODUCTION entry point
crates/core/src/storage/admin.rs   # the AdminBackend::create_project implementor
crates/core/src/storage/traits.rs # AdminBackend::create_project trait signature (:204)
```

**Important:** `crates/core/src/tools/admin.rs`'s own `create_project` function is `#[cfg(test)]`-
gated test-dispatch glue only (AGENTS.md: *"what remains of that machinery lives at
`tools::registry_support`... kept only because `Args`/`ToolCtx` are the test-dispatch types"*). The
real production code path to modify is `mcp/server.rs`'s `CreateProjectArgs` struct and
`admin_create_project` function.

### Existing Patterns
`admin.set_purge_config`'s existing validation (`tools/admin.rs:572-578`) is the exact pattern to
mirror:
```rust
// crates/core/src/tools/admin.rs:572-578 — mirror this validation exactly in admin_create_project
if file_retention_days == Some(0) || file_retention_days.is_some_and(|d| d < 0) {
    return Err(ToolError::invalid_argument("file_retention_days must be > 0"));
}
if project_retention_days == Some(0) || project_retention_days.is_some_and(|d| d < 0) {
    return Err(ToolError::invalid_argument("project_retention_days must be > 0"));
}
```
The existing `require_admin` gate at `mcp/server.rs:2084` inside `admin_create_project` is
**unchanged** by this story — project creation's authorization is not part of this spec's scope.

### Data Model (excerpt)
`PurgeConfig { autopurge_enabled: bool, use_internal_purge: bool, file_retention_days: Option<i64>,
project_retention_days: Option<i64> }` (`storage/traits.rs:119-124`) — no new fields, this story just
applies it at creation time instead of only post-creation.

### Decisions That Govern This Story
- **DEC-006** (§17): "`admin.create_project` gains the same four optional purge params as
  `admin.set_purge_config`, defaulting to today's `PurgeConfig::default()` when omitted...
  mirroring the existing tool's param names/types/validation avoids inventing a second convention."

### Applicable NFRs
None beyond what already governs `admin.create_project` — no new performance, security or
observability requirement.

### Bounded Context
**Project provisioning** (§4.5): "Creating a project and its initial purge configuration." Key
entities: `Project`, `PurgeConfig`.

## Functional Requirements

### FR-NEW-014: `admin.create_project` retention parameters
- **EARS:** [EARS-E] "WHEN `admin.create_project` is called with any of
  `autopurge_enabled:boolean=false, use_internal_purge:boolean=false,
  file_retention_days:number|null=null, project_retention_days:number|null=null` THE system SHALL
  apply the resulting `PurgeConfig` to the new project atomically at creation, identical to calling
  `admin.set_purge_config` with the same values immediately afterward."
- **Inputs / Outputs:** the four new optional params on `admin.create_project`, named and typed
  exactly as `admin.set_purge_config`'s equivalents.
- **Business Rules:** when all four are omitted, behavior is byte-identical to today
  (`PurgeConfig::default()`); every existing caller/test using the 2-arg form continues to compile
  and pass unmodified.
- **Exact names:** tool `admin.create_project`, new params as listed.

### FR-NEW-015: `admin.create_project` retention validation
- **EARS:** [EARS-O] "IF `file_retention_days` or `project_retention_days` is `0` or negative THEN
  THE system SHALL return `ERR_INVALID_ARGUMENT`, using the identical validation
  `admin.set_purge_config` already applies (`tools/admin.rs:572-578`)."

## Acceptance Tests

> **100% must pass.** Run via `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| none beyond standard project-creation fixtures | | existing `storage/admin.rs` test fixtures | ready |

### E2E-NEW-429: `admin.create_project` applies retention at creation
- **Category:** Happy / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- **Steps:** Given no project `proj-new-purge` exists / When
  `admin.create_project(project_id="proj-new-purge", owner="alice", autopurge_enabled=true,
  use_internal_purge=true, file_retention_days=7, project_retention_days=30)` is called / Then the
  project is created / And `admin.get_purge_config("proj-new-purge")` (internal call) returns
  `PurgeConfig{autopurge_enabled: true, use_internal_purge: true, file_retention_days: Some(7),
  project_retention_days: Some(30)}`.
- **Priority:** Critical

### E2E-NEW-430: omitted params keep today's default
- **Category:** Edge / **Scenario:** SC-005 / EXC-005b / **Requirements:** FR-NEW-014
- **Steps:** Given no project `proj-old-style` exists / When
  `admin.create_project(project_id="proj-old-style", owner="alice")` is called with no purge params
  (the existing 2-arg form) / Then the project is created with `PurgeConfig::default()` exactly.
- **Priority:** Critical

### E2E-NEW-431: negative retention at creation is rejected
- **Category:** Failure / **Scenario:** SC-005 / EXC-005a / **Requirements:** FR-NEW-015
- **Steps:** Given no project `proj-bad-retention` exists / When
  `admin.create_project(project_id="proj-bad-retention", owner="alice", autopurge_enabled=true,
  use_internal_purge=true, file_retention_days=-1, project_retention_days=null)` is called / Then
  the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** High

### E2E-NEW-459: partial retention params at creation don't implicitly enable autopurge
- **Category:** Edge / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- **Steps:** Given no project `proj-partial-purge` exists / When
  `admin.create_project(project_id="proj-partial-purge", owner="alice", file_retention_days=5)` is
  called with `autopurge_enabled`/`use_internal_purge` left at their `false` defaults / Then the
  resulting `PurgeConfig` has `file_retention_days=Some(5)` but `autopurge_enabled=false` (DB
  query) — confirming the fields apply independently, and the sweep remains gated off per the
  existing `purge.rs:51` check.
- **Priority:** Low

### E2E-NEW-460: negative `project_retention_days` at creation is rejected
- **Category:** Failure / **Scenario:** SC-005 / **Requirements:** FR-NEW-015
- **Steps:** Given no project `proj-bad-retention-2` exists / When
  `admin.create_project(project_id="proj-bad-retention-2", owner="alice", autopurge_enabled=true,
  use_internal_purge=true, file_retention_days=null, project_retention_days=-3)` is called / Then
  the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

### E2E-NEW-461: exact-zero retention at creation is rejected
- **Category:** Failure / **Scenario:** SC-005 / **Requirements:** FR-NEW-015
- **Steps:** Given no project `proj-zero-retention` exists / When
  `admin.create_project(project_id="proj-zero-retention", owner="alice", autopurge_enabled=true,
  use_internal_purge=true, file_retention_days=0, project_retention_days=null)` is called / Then the
  call fails with `ERR_INVALID_ARGUMENT`, mirroring `admin.set_purge_config`'s identical rejection of
  exact zero (`tools/admin.rs:572-578`).
- **Priority:** Low

## Constraints

### Files Not to Touch
- `crates/core/src/tools/admin.rs`'s `create_project` test-dispatch glue — update it only if the
  shared test harness requires matching the new signature for `#[cfg(test)]` builds to compile;
  do not add new production logic there.
- Every other story's files — this story is fully independent.

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not invent a second tool (e.g. `admin.create_project_with_retention`) — DEC-006 explicitly
  rejected that alternative.
- Do not change `admin.create_project`'s existing `require_admin` authorization gate — out of scope.

### Scope Boundary
- Only `admin.create_project`'s signature and the `AdminBackend::create_project` trait method (and
  its 3 dialect implementors) change. `admin.set_purge_config` itself is untouched.

## Non Regression

### Existing Tests That Must Pass
- Every existing test calling `admin.create_project(project_id, owner)` with the 2-arg form must
  keep compiling and passing unmodified — this is the specific regression this story must not break.

### Behaviors That Must Not Change
- A project created with no purge params gets `PurgeConfig::default()`, byte-identical to today.

### API Contracts to Preserve
- `admin.create_project`'s existing two required params (`project_id`, `owner`) keep their exact
  names, types and position; the four new params are additive and optional.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
