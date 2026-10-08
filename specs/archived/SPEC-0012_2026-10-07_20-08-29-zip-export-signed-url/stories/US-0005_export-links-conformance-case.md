# US-0005: Converge gap — export_links multi-dialect conformance case

> Parent Spec: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Spec ID: SPEC-0012
> Epic: n/a
> Status: todo
> Priority: 5
> Depends On: US-0001
> Complexity: S
> min_tier: 2
> Files touched: 1

> **Origin:** appended by Phase 4.6 CONVERGE after all four original stories were done. The
> auditor found FR-NEW-015 ("migrated identically across SQLite, PostgreSQL and SQL Server")
> had only a SQLite-backed unit test (`storage/meta.rs`); the spec's own traceability matrix
> names `E2E-NEW-050` with driver `storage::conformance` for the multi-dialect claim, and that
> suite was never touched by US-0001. This story closes exactly that gap.

## Objective
Add one case to the existing backend-parametric conformance suite
(`crates/core/src/storage/conformance.rs`) proving `export_links` insert/read/delete behaves
identically on SQLite, PostgreSQL and SQL Server — the same proof every other table in that suite
already carries (see `git_objects_refs_remotes`, `oauth_round_trip`, etc. in the same file).

## Technical Context

### Stack
Rust 2024, the project's existing `storage::conformance` harness (no new dependency).

### Relevant File Structure
```
crates/core/src/storage/
  conformance.rs   <- add one case fn here + call it from run_suite
  meta.rs          <- read-only reference: insert_export_link, delete_export_link_if_live,
                        select_expired_export_links (already built by US-0001, do not modify)
```

### Existing Patterns
- `conformance.rs`'s module doc (`conformance.rs:1-19`) explains the exact pattern: one case
  function per table/feature, taking `&Engine` and a per-run `tag` for isolation, called once from
  `run_suite` (`conformance.rs:~680-696`), which is itself invoked by three `#[tokio::test]`
  functions — `sqlite_passes_the_conformance_suite` (always runs), `postgres_...`/`sqlserver_...`
  (self-skip, printing a message, when their DSN env var is unset or the feature is off).
- Follow the shape of a comparable existing case (e.g. whichever case nearest exercises a simple
  insert/read/delete table, such as the git refs/remotes case) for the exact calling convention:
  open a `RelationalMetaStore`/equivalent against `engine.db().await?`, run the three operations,
  assert the row round-trips and the delete actually removes it, return `Ok(())`.
- Use `insert_export_link`, `delete_export_link_if_live`, `select_expired_export_links` exactly as
  US-0001 built them (`crates/core/src/storage/meta.rs`) — do not add a fourth accessor or change
  their signatures.
- Derive the test's `token`/`volume_id` from the per-run `tag` the harness already threads through
  every case, so PostgreSQL/SQL Server runs (which share one live instance, not a fresh database per
  call) stay isolated from any other concurrent case.

### Data Model (excerpt)
The same `export_links` table US-0001 declared: `token` (`TextKey(36)`, PK), `volume_id`
(`TextKey(VOLUME_ID_LEN)`), `created_at` (`Text`), `expires_at` (`Text`). This story does not change
the schema; it only proves the existing declaration behaves identically across dialects.

### Decisions That Govern This Story
- **DEC-016:** `export_links` declared in the same schema module as `trash_entries`
  (`storage/meta.rs`), migrated via the dialect-neutral `Table`/`Column` DSL — this story's case is
  the proof that the DSL's dialect-neutrality claim holds for this specific table, the same way it
  already holds for every other table in the conformance suite.

### Applicable NFRs
None beyond what US-0001 already satisfied; this story adds verification, not new behavior.

### Bounded Context
**Lifecycle sweep** / cross-cutting with **Export creation** — this is a structural proof about the
storage substrate both contexts depend on.

## Functional Requirements

### FR-NEW-015 [EARS-U]: Schema (re-verified here, not re-implemented)
- **EARS:** THE system SHALL declare the `export_links` table in the same schema module as
  `trash_entries`, migrated identically across SQLite, PostgreSQL and SQL Server via the store's
  existing migration path.
- **Business Rules:** this story's case is the specific proof required by the spec's own
  traceability matrix (E2E-NEW-050), which named `storage::conformance` as the driver.

## Acceptance Tests

> **100% must pass.** Run through `cargo test --all-features conformance` (SQLite always runs;
> PostgreSQL/SQL Server self-skip without their DSN env vars — that is expected and not a failure).

### E2E-NEW-050: structural, export_links migration runs cleanly on a fresh database, all three dialects
- **Category:** Structural
- **Scenario:** SC-001
- **Requirements:** FR-NEW-015
- **Driver:** `storage::conformance` suite
- **Preconditions:** a brand-new SQLite in-memory database (always), a PostgreSQL connection when
  `MCPFS_TEST_PG_DSN` is set, a SQL Server connection when `MCPFS_TEST_MSSQL_DSN` is set
- **Steps:**
  - Given the engine under test (one case function, called once per engine by `run_suite`)
  - When `insert_export_link` writes one row, then `delete_export_link_if_live` reads it back and
    deletes it atomically
  - Then the row's columns round-trip exactly (`token`, `volume_id`, `created_at`, `expires_at`),
    the delete reports success exactly once, and a second call on the same token reports "not found"
  - And `select_expired_export_links` returns the row when `expires_at` is in the past and omits it
    when it is in the future (using a second inserted row for the live case)
- **Cleanup:** none (SQLite is in-memory per call; PostgreSQL/SQL Server rows are deleted by the
  case itself)
- **Priority:** Medium

## Constraints

### Files Not to Touch
- `crates/core/src/storage/meta.rs` — call the existing accessors, do not modify or add to them.
- `crates/core/src/exports.rs`, `crates/core/src/tools/export.rs`, `crates/core/src/purge.rs` — not
  this story's concern.

### Dependencies Not to Add
- None needed.

### Patterns to Avoid
- Do not write a new standalone test outside `conformance.rs`'s `run_suite` pattern — the whole
  point is running the same case against all three dialects through the existing harness, not a
  SQLite-only test (that already exists, in `meta.rs`, from US-0001).

### Scope Boundary
- This story adds exactly one conformance case. It does not touch any other table's case, any
  production code path, or any other story's files.

## Non Regression

### Existing Tests That Must Pass
- Every existing case in `conformance.rs`, unmodified.
- The full existing `storage::meta` test suite from US-0001, unmodified.

### Behaviors That Must Not Change
- No production behavior changes; this is test-only.

### API Contracts to Preserve
- None affected.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
