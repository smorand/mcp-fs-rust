# US-0007: Background loop + CLI purge verb (core)

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 7
> Depends On: US-0005, US-0006
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective

Wires US-0005's file sweep and US-0006's project sweep to their two drivers: an in-process
background loop (interval-based, started at server boot) and a new `mcp-fs purge` CLI verb. Both
drivers call the exact same sweep functions — this story adds no new sweep logic, only the two
callers and the CLI's gating rule.

## Technical Context

### Stack
Rust 2024, tokio, clap.

### Relevant File Structure
```
crates/core/src/
  cli.rs       (new `Purge` subcommand)
  app.rs       (spawn the loop at boot, stop it cleanly at shutdown)
  purge.rs     (add the loop-cycle orchestration function here, calling US-0005/US-0006's fns)
```

### Existing Patterns
- Background task precedent: `search/indexer.rs:16-23` (detached, log-on-failure).
- Graceful shutdown precedent: `app.rs:172-195 shutdown_signal` (selects on `ctrl_c()` and
  `SIGTERM`) — the new loop task must be cancellable the same way, not left to be killed by
  process exit.
- CLI precedent: `cli.rs`'s existing `Serve | Keys | Token | Migrate | Version` clap-derive
  subcommands (`cli.rs:44-112`) — `Purge` follows the identical shape.

### Decisions That Govern This Story
- **DEC-002** (Section 17): dual trigger — internal loop and CLI, selectable per project via
  `use_internal_purge`.
- **DEC-003**: CLI purge on a project not configured for autopurge fails unless `--on-demand` is
  passed.

### Applicable NFRs
Section 7.6 (Deployment): no new infra, in-process `tokio::spawn`, no new crate dependency.

## Functional Requirements

### FR-NEW-009 [EARS-U]: Background sweep loop
> The system SHALL run the FR-NEW-007/FR-NEW-008 sweep on a fixed interval given by
> `safety.purge_interval_secs`, via a detached background task started at server boot and stopped
> cleanly on shutdown.

- **Business Rules:** the grace-period sweep (FR-NEW-012, US-0008) also runs from this loop, but
  this story only needs to leave the hook for US-0008 to add it to — do not build the grace-sweep
  call here, only the file/project sweep call per cycle.

### FR-NEW-010 [EARS-U]: CLI `mcp-fs purge`
> The system SHALL provide a `purge` CLI verb accepting an optional `--project <id>` and an
> optional `--on-demand` flag.

### FR-NEW-011 [EARS-O]: CLI purge gating and global mode
> IF `--project <id>` is given and that project's `autopurge_enabled` is `false` and
> `--on-demand` is absent THEN THE system SHALL exit non-zero with no action taken. IF
> `--project <id>` is given and (`autopurge_enabled` is `true` OR `--on-demand` is present) THEN
> THE system SHALL run the FR-NEW-007/FR-NEW-008 logic synchronously for that project and print a
> summary. IF `--project` is absent THEN THE system SHALL run the grace-period sweep across every
> soft-deleted project (US-0008 implements the grace-sweep function itself; this story wires the
> no-`--project` CLI branch to call it once it exists).

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Compiled `mcp-fs` binary | for CLI tests via `assert_cmd` | build artifact | ready |

### E2E-NEW-046: CLI happy path
- **Category:** happy
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010, FR-NEW-011
- **Steps:** Given project `proj1`, `autopurge_enabled=true`, a stale file and a stale-project
  condition (per US-0005/US-0006's fixtures) / When `mcp-fs purge --project proj1` / Then exits 0,
  prints a summary with non-zero counts for files purged and the project outcome.
- **Priority:** Critical

### E2E-NEW-047: CLI refuses unconfigured project without --on-demand
- **Category:** failure
- **Requirements:** FR-NEW-011
- **Steps:** Given project `proj1`, `autopurge_enabled=false` / When `mcp-fs purge --project
  proj1` (no `--on-demand`) / Then exits non-zero, message names the project and the reason, DB
  unchanged.
- **Priority:** Critical

### E2E-NEW-048: CLI --on-demand override
- **Category:** happy (override)
- **Requirements:** FR-NEW-011
- **Steps:** Same project as E2E-NEW-047 but with `--on-demand` / When `mcp-fs purge --project
  proj1 --on-demand` / Then exits 0, purge logic runs regardless of `autopurge_enabled`.
- **Priority:** Critical

### E2E-NEW-050: CLI on nonexistent project
- **Category:** failure
- **Requirements:** FR-NEW-011
- **Steps:** `mcp-fs purge --project ghost` / Then exits non-zero with an
  `ERR_PROJECT_NOT_FOUND`-equivalent message.
- **Priority:** High

### E2E-NEW-051: Global sweep, empty state
- **Category:** edge (empty state)
- **Requirements:** FR-NEW-011
- **Steps:** `mcp-fs purge` (no `--project`) with zero soft-deleted projects anywhere / Then exits
  0, zero-count summary, no error. (This exercises only the CLI branch dispatch to US-0008's
  function, not the grace logic itself, owned by US-0008.)
- **Priority:** Low

### E2E-NEW-052: Global sweep ignores autopurge_enabled — dispatch only
- **Category:** cross-scenario
- **Requirements:** FR-NEW-011
- **Steps:** Confirms the no-`--project` CLI branch calls the grace-sweep function unconditionally
  (not filtered by `autopurge_enabled`) — the actual permanent-removal assertion is US-0008's
  E2E-NEW-080/053-057; this test only asserts the CLI wiring reaches that function.
- **Priority:** High

### E2E-NEW-077: Loop performs real work end-to-end
- **Category:** happy (real interval, not direct function call)
- **Scenario:** SC-002/003
- **Requirements:** FR-NEW-009
- **Steps:** Given server started with `purge_interval_secs=1` (test override) and a stale
  file/project fixture set up BEFORE start / When server runs for 2.5s / Then the file is trashed
  and the project is soft-deleted without an explicit manual trigger — proving the loop itself,
  not a directly-invoked function, performs the work.
- **Priority:** Critical

### E2E-NEW-078: Graceful shutdown
- **Category:** edge
- **Requirements:** FR-NEW-009
- **Steps:** Given the loop is mid-cycle when shutdown is signaled (same mechanism as
  `app.rs:172-195`) / When shutdown fires / Then the loop task exits cleanly, no panic, no
  orphaned task, join completes within a bounded timeout.
- **Priority:** High

### E2E-NEW-079: Many projects, bounded time
- **Category:** edge (scale)
- **Requirements:** FR-NEW-009
- **Steps:** Given 50 projects with a mix of stale/non-stale files and purge configs / When one
  sweep cycle runs / Then completes within a bounded time budget, every expected outcome occurred
  once, none skipped, none duplicated.
- **Priority:** Low

## Constraints

### Files Not to Touch
`purge.rs`'s file-sweep and project-sweep functions themselves (US-0005/US-0006 — this story only
calls them).

### Dependencies Not to Add
clap is already a dependency; no new crate.

### Patterns to Avoid
Do not implement the grace-period sweep's actual logic here (US-0008 owns FR-NEW-012) — only wire
the CLI's no-`--project` branch to call whatever function US-0008 exposes.

### Scope Boundary
Loop orchestration and CLI argument handling/gating only.

## Non Regression

### Existing Tests That Must Pass
`cli.rs`'s existing subcommand tests (`no_verb_means_serve`, etc.) must still pass — the new
`Purge` variant must not change existing dispatch behavior.

### Behaviors That Must Not Change
Every existing CLI verb (`serve`, `keys`, `token`, `migrate`, `version`) unaffected.

### API Contracts to Preserve
Existing CLI flag shapes unchanged.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
