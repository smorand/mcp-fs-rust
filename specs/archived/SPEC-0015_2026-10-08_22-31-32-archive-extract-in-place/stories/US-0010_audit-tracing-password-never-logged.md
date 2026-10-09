---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0009
min_tier: 2
Complexity: S
Files touched: crates/core/src/tools/archive.rs
---

# US-0010 — Audit entry; tracing; password-never-logged

## Objective

On success, record one audit entry and emit one `tracing::info!` event with the five named
fields, and guarantee the `password` field is never logged, in full or in part, at any tracing
level, under any circumstance, including on failure.

## Technical Context

### Stack

Rust 2024. Uses existing `safety.record_audit` and `tracing::info!`. No new dependency.

### Relevant File Structure

- `crates/core/src/core/fs_ops.rs:613` — `core::fs_ops::write_bytes`'s one-audit-entry-per-
  operation call, the pattern this story mirrors.
- `crates/core/src/tools/archive.rs` (from US-0009) — this story adds the audit call and the
  tracing event immediately after a successful write pass, and ensures no code path anywhere in
  the module formats the `password` parameter into a log line.

### Existing Patterns Referenced

`AGENTS.md`'s inherited `rust` skill rule: "MUST / NEVER trace a credential, token, or secret."
This story is the concrete enforcement of that rule for this spec's `password` parameter.

### Decisions That Govern This Story

None of §17's 11 decisions specifically govern observability; `FR-NEW-025`, `FR-NEW-026`,
`FR-NEW-027` are the governing requirements, and they are themselves §7.5's complete
observability surface.

### Applicable NFRs

§7.5 Observability: "one INFO tracing event on success with the five named fields, and an
absolute prohibition on logging the password at any level" — this story's entire scope.

### Bounded Context

Transport-adjacent but actually a cross-cutting concern over the archive-parsing context's own
call: the audit/tracing calls happen inside `tools::archive::extract_archive` itself, not in
either transport door, mirroring how `core::fs_ops::write_bytes` emits its own audit entry
regardless of which door called it.

## Functional Requirements

**FR-NEW-025** [EARS-U] On success, THE system SHALL call `safety.record_audit(person, mount_id,
"extract_archive", <normalized path>, <a detail string containing the destination, files_written,
and bytes_written>)` exactly once per call, mirroring the one-audit-entry-per-operation pattern
`core::fs_ops::write_bytes` uses (`fs_ops.rs:613`).

- Inputs: `person`, `mount_id`, the normalized `path`, the response fields from US-0009.
- Outputs: one `record_audit` call.
- Business Rules: exactly once, only on success.

**FR-NEW-026** [EARS-E] WHEN a call to `fs.extract_archive` succeeds THE system SHALL emit one
`tracing::info!` event carrying the fields `mount_id`, `path`, `destination`, `files_written`, and
`bytes_written`.

- Inputs: the same fields as `FR-NEW-025`.
- Outputs: one structured `tracing::info!` event with exactly these five fields.
- Business Rules: exactly these five field keys, no more, no fewer; no `password` key present
  even as an empty value.

**FR-NEW-027** [EARS-UB] The system SHALL NOT log the `password` field, in full or in part, at any
tracing level, under any circumstance, including on failure.

- Inputs: the `password` parameter, every code path (success and every failure mode from
  US-0005 through US-0009).
- Outputs: no log line, at any level, containing the password string.
- Business Rules: absolute, unconditional, applies to every error message too (e.g. the
  password-incorrect error message must never embed the attempted password).

## Acceptance Tests

### Test Data

| Scenario | password | expected log content |
|---|---|---|
| E2E-NEW-008 fixture, wrong password `"wrong"` | `"wrong"` | no log line contains `"wrong"` |
| E2E-NEW-004 fixture, correct password `"swordfish"` | `"swordfish"` | no log line contains `"swordfish"` |

#### E2E-NEW-028 — Failing call (wrong password) logs no literal password string

- Category: Failure. Requirements: FR-NEW-027.
- Preconditions: a `tracing_test`-style subscriber (or an in-memory `tracing` layer collecting
  formatted event text) installed for the duration of the test; the E2E-NEW-008 fixture (wrong
  password `"wrong"` against a password `"correct"` archive).
- Steps: Given the subscriber, When `extract_archive(.., password=Some("wrong"), ..)` fails,
  Then none of the collected log lines contain the literal substring `"wrong"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-029 — Succeeding call's log line also contains no literal password string

- Category: Edge. Requirements: FR-NEW-027.
- Preconditions: same subscriber mechanism, the E2E-NEW-004 fixture (correct password
  `"swordfish"`).
- Steps: Given the subscriber, When `extract_archive(.., password=Some("swordfish"), ..)`
  succeeds, Then none of the collected log lines contain the literal substring `"swordfish"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-044 — Tracing event has exactly the five named fields; password absent on success and failure

- Category: Edge. Requirements: FR-NEW-011,026,027.
- Preconditions: the subscriber mechanism of E2E-NEW-028/029, run against both the E2E-NEW-001
  (success) and E2E-NEW-008 (failure, wrong password `"wrong"`) fixtures.
- Steps: Given each fixture in turn, When `extract_archive` completes, Then the collected
  structured fields on the relevant event are exactly `mount_id`, `path`, `destination`,
  `files_written`, `bytes_written` for the success case (`FR-NEW-026`) — no `password` field
  key present at all, not merely an empty one — And no event at any level carries a `password`
  key for the failure case either.
- Cleanup: none. Priority: P1.

## Constraints

- Files Not to Touch: REST route/OpenAPI (US-0011), tool contract (US-0012).
- Dependencies Not to Add: none.
- Patterns to Avoid: do not interpolate the `password` parameter into any `format!`/`tracing`
  macro call anywhere in `tools::archive.rs`, even in a debug-only branch.
- Scope Boundary: audit entry and tracing event only.

## Non Regression

`core::fs_ops::write_bytes`'s own audit pattern is unmodified; this story only adds a new call
site following the same pattern.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
