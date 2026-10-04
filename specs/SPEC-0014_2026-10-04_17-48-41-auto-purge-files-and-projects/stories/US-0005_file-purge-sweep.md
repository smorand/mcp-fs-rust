# US-0005: File-purge sweep logic

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 5
> Depends On: US-0002, US-0003
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Implements the core file-purge sweep: for a project with autopurge configured, soft-delete into
the existing trash every live file whose `atime` is older than its configured
`file_retention_days`. This is sweep logic only — callable directly in tests; US-0007 wires it to
the background loop and the CLI.

## Technical Context

### Stack
Rust 2024, the existing trash/soft-delete mechanism.

### Relevant File Structure
```
crates/core/src/
  purge.rs            (new module — the sweep functions live here)
  storage/meta.rs     (query helper: files with atime older than a threshold)
  safety.rs           (trash_path — reused unchanged, read-only reference)
```

### Existing Patterns
- Soft-delete (trash) is a metadata-only path rename: `safety.rs:180-191 trash_path` produces
  `/{trash_dir}/{epoch_ms}__{flattened path}`; blob and refcount untouched. This story calls the
  exact same path `fs.delete` already uses in its default (trash) mode — never a parallel
  mechanism.
- `descendant_pattern`/`Dialect::escape_like_literal` is the one correct way to build any `LIKE`
  pattern over a subtree (AGENTS.md) — use it for the trash-subtree exclusion below, never a
  hand-built pattern.

### Data Model (excerpt)
Reads `project_purge_config` (US-0002) for `autopurge_enabled`, `use_internal_purge`,
`file_retention_days`. Reads `nodes.atime` (US-0003) per file.

### Decisions That Govern This Story
- **DEC-006** (Section 17): file-level purge reuses the existing trash mechanism as-is, single
  phase — no new per-file retention clock in this spec (permanent trash cleanup is BL-0003's
  separate, unspec'd scope).
- **DEC-013**: no new locking primitive — a file soft-delete is naturally idempotent (purging an
  already-trashed file is a no-op), so concurrent triggers from the loop and the CLI (US-0007)
  collapse to one effect without a lock.

### Applicable NFRs
Section 7.4 (Reliability): no retry beyond "next cycle picks it up"; each file commits
independently so a crash mid-sweep loses no already-committed progress.

## Functional Requirements

### FR-NEW-007 [EARS-S]: Internal file-purge sweep
> WHILE a project has `autopurge_enabled && use_internal_purge && file_retention_days.is_some()`
> THE system SHALL, on every background sweep cycle, soft-delete into the existing trash every
> live file whose `now - atime > file_retention_days`, committing each file independently (a
> failure on one file SHALL NOT block any other file in the same cycle).

- **Inputs / Outputs:** input: `project_id`; output: count of files purged, logged.
- **Business Rules:**
  - "Live file" excludes any node whose path falls under the project's `trash_dir` subtree
    (matched via `descendant_pattern`/`Dialect::escape_like_literal`); such nodes are never purge
    targets.
  - Re-purging an already-trashed file is a no-op, not an error.
  - One commit per file: a failure on file N does not roll back files `1..N-1` nor block
    `N+1..end`.
  - This function is called directly by both US-0007 (loop) and US-0007's CLI form, synchronously
    or from the spawned loop task — this story exposes it as a plain async function, no transport
    wiring.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Project with `file_retention_days=7`, files with varying seeded `atime` | Direct DB seed, no dependency on the real interval loop | fixture | ready |

### E2E-NEW-024: Happy path, file purged
- **Category:** happy
- **Scenario:** SC-002
- **Requirements:** FR-NEW-007
- **Steps:** Given project with `autopurge_enabled=true, use_internal_purge=true,
  file_retention_days=7`; file `old.txt` has `atime = now - 8 days` / When the sweep function is
  invoked directly for this project / Then `old.txt` is soft-deleted (present in trash listing).
- **Priority:** Critical

### E2E-NEW-025: Boundary, exactly at threshold — not purged
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-007
- **Steps:** File `atime = now - 7 days` exactly / When sweep runs / Then NOT purged (strict `>`,
  not `>=`).
- **Priority:** Critical

### E2E-NEW-026: Boundary, just past threshold — purged
- **Category:** edge (boundary)
- **Requirements:** FR-NEW-007
- **Steps:** File `atime = now - 7 days - 1 second` / When sweep runs / Then purged.
- **Priority:** High

### E2E-NEW-027: use_internal_purge=false gates the sweep off
- **Category:** state (regression)
- **Requirements:** FR-NEW-007
- **Steps:** Given `use_internal_purge=false`, `autopurge_enabled=true`, stale file (age 10 days,
  `file_retention_days=7`) / When sweep runs / Then file NOT purged (gated strictly on
  `use_internal_purge`, independent of `autopurge_enabled`).
- **Priority:** Critical

### E2E-NEW-028: Per-file commit, not all-or-nothing
- **Category:** side-effect / error recovery
- **Requirements:** FR-NEW-007
- **Steps:** Given 3 stale files in the same project, file 2's commit forced to fail (locked row)
  / When sweep runs / Then file 1 purged, file 2 not, file 3 STILL purged.
- **Priority:** Critical

### E2E-NEW-029: Idempotent re-purge
- **Category:** error recovery / idempotency
- **Requirements:** FR-NEW-007
- **Steps:** Given `old.txt` already trashed by a prior sweep / When sweep runs again on the same
  file / Then no-op: no error, no duplicate trash entry.
- **Priority:** Critical

### E2E-NEW-030: Empty project
- **Category:** edge (empty state)
- **Requirements:** FR-NEW-007
- **Steps:** Given project has zero files / When sweep runs / Then completes with zero purges, no
  error.
- **Priority:** Low

### E2E-NEW-031: file_retention_days=None skips this step independently
- **Category:** edge (data variation)
- **Requirements:** FR-NEW-007
- **Steps:** Given `file_retention_days = None` but `project_retention_days` set / When sweep runs
  / Then the file-purge step is skipped entirely for this project (no files touched); the
  project-level step (US-0006) is independently evaluated.
- **Priority:** High

## Constraints

### Files Not to Touch
`cli.rs`, `app.rs` — the loop/CLI wiring is US-0007's job; this story only exposes the callable
function.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not build a second retention clock for trashed files (DEC-006) — purge = reuse `fs.delete`'s
default trash path exactly, nothing more.

### Scope Boundary
File-level sweep only. Project-level staleness (FR-NEW-008) is US-0006's story, even though it
shares this module (`purge.rs`) and runs in the same cycle.

## Non Regression

### Existing Tests That Must Pass
Existing trash/soft-delete tests (`core/fs_ops.rs:2639` and neighbors) must pass unmodified — this
story calls the same trash path, never a new one.

### Behaviors That Must Not Change
Manual `fs.delete` behavior is completely unaffected by this story.

### API Contracts to Preserve
None — `purge.rs` is new, internal-only at this point (no tool/CLI surface yet).

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
