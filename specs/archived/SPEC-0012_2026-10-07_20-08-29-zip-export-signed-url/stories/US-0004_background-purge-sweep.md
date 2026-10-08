# US-0004: Background purge sweep for expired exports

> Parent Spec: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Spec ID: SPEC-0012
> Epic: n/a
> Status: ready
> Priority: 4
> Depends On: US-0001
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Fold cleanup of expired, never-downloaded exports into the existing background purge cycle, so an
export that nobody ever opened doesn't linger forever. This is the only other place (besides
US-0003's opportunistic cleanup-on-GET) that deletes `export_links` rows and their blobs.

## Technical Context

### Stack
Rust 2024, `tokio` (async).

### Relevant File Structure
```
crates/core/src/
  purge.rs          <- add the third per-project sweep step + extend CycleSummary here
  storage/meta.rs    <- read-only reference: US-0001's select-expired accessor
  storage/volume.rs  <- read-only reference: VolumeClient.blob (public field)
```

### Existing Patterns
- `run_cycle`'s existing per-project loop: `purge.rs:174-203`. It calls `sweep_project_files` then
  `sweep_project` for each project from `admin.list_all_projects()`, isolating failures per project
  with the exact pattern `match ... .await { Ok(_) => ..., Err(e) => tracing::warn!(...) }` — never
  aborting the whole cycle on one project's failure. Add a **third** call in the same loop body,
  following the identical isolation pattern, this time isolating failures **per row** (one bad
  export row must not abort the sweep for the rest of that project, nor for other projects).
- `CycleSummary` struct: `purge.rs:163-167` (`pub struct CycleSummary { pub files_purged: usize, pub
  projects_soft_deleted: usize }`). Add a new field, e.g. `pub exports_swept: usize`.
- Signatures already fixed by the spec: `pub async fn run_cycle(stores: &StoreManager, admin: &dyn
  AdminBackend, safety: &SafetyManager) -> Result<CycleSummary>` (`purge.rs:174-178`); called from
  the background loop (`app.rs:210`) and reachable from the CLI `purge` verb. Do not change this
  signature; add the new sweep step inside the existing function body.
- Deletion mechanics: for each row returned by US-0001's select-expired accessor, delete the blob at
  `export:{token}` via `VolumeClient.blob.delete(...)` (public field, `storage/volume.rs:15`), then
  delete the row (or vice versa — order doesn't matter for correctness since both are idempotent
  deletes, but delete the blob first so a crash mid-sweep leaves an orphaned row rather than an
  orphaned blob, the cheaper failure mode to clean up on the next cycle).

### Data Model (excerpt)
Same `export_links` table from US-0001. This story **reads** (select-expired) and **deletes**; it
never inserts.

### Decisions That Govern This Story
- **DEC-009:** Cleanup folds into the existing `purge::run_cycle`, no new sweeper process — one
  cleanup entry point already exists and already has the right error-isolation shape. **Implemented
  by:** FR-NEW-014.
- **DEC-013:** Grace period for a never-downloaded export equals the link's own expiry — no separate
  grace window. The sweep's criterion is simply `expires_at < now`. **Implemented by:** FR-NEW-014.

### Applicable NFRs
- **§7.4 Reliability:** cleanup depends on the existing purge cycle running (background loop or CLI
  verb). If disabled, expired exports accumulate — the same existing limitation shared by every
  other purge-dependent cleanup in this codebase, not a new risk introduced here. This story does not
  need to add any new reliability mechanism beyond what `run_cycle` already has.
- **§7.5 Observability:** DEBUG on sweep: count of rows/blobs deleted per cycle (use the new
  `exports_swept` field).

### Bounded Context
**Lifecycle sweep**: "Time-based cleanup of artifacts no authentication layer ever gates." Key
entity: `purge::run_cycle`.

## Functional Requirements

### FR-NEW-014 [EARS-E]: Background sweep of expired exports
- **EARS:** WHEN `purge::run_cycle`'s per-project loop runs THE system SHALL, for each project,
  delete every `export_links` row whose `expires_at` has passed as of the sweep's start time, and
  delete the corresponding blob bytes at `export:{token}` for each deleted row, isolating failures
  per row the same way `sweep_project_files`/`sweep_project` isolate failures per project.
- **Business Rules:** a failure deleting one row's blob (e.g. already missing) does not abort the
  sweep for other rows or other projects.
- **Exact names:** extends `crates/core/src/purge.rs`'s `run_cycle` (`purge.rs:174-203`) with a
  third step; `CycleSummary` (`purge.rs:163-167`) gains a new counter field, e.g. `exports_swept:
  usize`.

## Acceptance Tests

> **100% must pass.** Run through `cargo test --workspace`. Loop fix/run/check until zero failures.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| token `T`, `T1`, `T2`, `T_expired`, `T_live` | inserted directly via US-0001's insert accessor with controlled `expires_at` values (no need to go through US-0002's tool) | test fixture | ready |

### E2E-NEW-031: SC-005 happy path, sweep deletes expired never-downloaded export
- **Category:** Happy path / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given token `T` created, `expires_at` directly set to `now - 1h` / When `purge::run_cycle` runs
  for `proj` / Then it returns success
- **Priority:** Critical

### E2E-NEW-032: side effect, export_links row removed by sweep
- **Category:** Side effect / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given state from E2E-NEW-031, row confirmed present before / When `purge::run_cycle` runs / Then
  row count for `T` is `0` after
- **Priority:** Critical

### E2E-NEW-033: side effect, export blob removed by sweep
- **Category:** Side effect / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given state from E2E-NEW-031, blob confirmed present before / When `purge::run_cycle` runs / Then
  `export:T` read after returns the not-found variant
- **Priority:** Critical

### E2E-NEW-034: failure isolation, one bad row doesn't abort the sweep for others
- **Category:** Failure / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given expired tokens `T1`, `T2`; `T1`'s blob removed out-of-band before the sweep (simulating
  corruption), `T2`'s blob intact / When `purge::run_cycle` runs for `proj` / Then both rows are
  removed regardless of `T1`'s blob-delete outcome; `T2`'s blob confirmed deleted; the sweep still
  returns success
- **Priority:** Critical

### E2E-NEW-035: state transition, sweep leaves unexpired rows untouched
- **Category:** State transition / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given `T_expired` (`expires_at = now-1h`) and `T_live` (`expires_at = now+4min`) / When
  `purge::run_cycle` runs / Then `T_expired` row+blob deleted; `T_live` row still present with
  unchanged `expires_at`, blob still readable with original bytes
- **Priority:** High

### E2E-NEW-036: edge, sweep with zero export_links rows is a no-op
- **Category:** Edge / **Scenario:** SC-005 / **Requirements:** FR-NEW-014
- Given zero `export_links` rows for `proj` / When `purge::run_cycle` runs / Then it returns success,
  no panic, row count remains `0`
- **Priority:** High

## Constraints

### Files Not to Touch
- `crates/core/src/exports.rs` / `crates/core/src/tools/export.rs` — this story does not touch either
  consumer; it only adds the sweep step and reads/deletes via US-0001's accessors.
- `crates/core/src/storage/meta.rs` — call the existing select-expired accessor, do not add a new
  query path.

### Dependencies Not to Add
- No new crate.

### Patterns to Avoid
- Do not let one row's deletion failure propagate and abort the whole cycle — this is exactly the
  isolation bug FR-NEW-014 and E2E-NEW-034 exist to prevent.
- Do not touch unexpired rows — the predicate is `expires_at < now`, strictly.

### Scope Boundary
- This story does not change `run_cycle`'s public signature, its callers in `app.rs`/`cli.rs`, or any
  other existing sweep step (`sweep_project_files`, `sweep_project`).

## Non Regression

### Existing Tests That Must Pass
- Every existing `purge.rs` test, unmodified — `sweep_project_files`/`sweep_project`'s own test
  coverage must stay green exactly as is.

### Behaviors That Must Not Change
- `run_cycle`'s existing two sweep steps and their isolation behavior.
- The CLI `purge` verb's existing behavior and output shape, apart from the new counter.

### API Contracts to Preserve
- `run_cycle`'s signature `(stores: &StoreManager, admin: &dyn AdminBackend, safety: &SafetyManager)
  -> Result<CycleSummary>` is unchanged; only `CycleSummary`'s fields grow (additive).

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
