# US-0001 [drift]: Detached best-effort DB-write helper (closes DRIFT-001)

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

This spec requires every content read and every write to bump `atime`/`mtime` "asynchronously
and best-effort" (FR-NEW-001, FR-NEW-002), but no fire-and-forget **database write** idiom exists
anywhere in this codebase today — only fire-and-forget idioms that spawn a server task, a device
flow poller, or an external embedding call (DRIFT-001, Section 19 of the parent spec). This story
builds the one small, reusable helper that does that, so US-0003 (atime/mtime wiring) can consume
it without re-deciding this.

## Technical Context

### Stack
Rust 2024, tokio, rusqlite/sqlx/tiberius-ng behind `storage::rel::RelationalDb`.

### Relevant File Structure
```
crates/core/src/
  core/fs_ops.rs        (new helper lives here, or in a new purge.rs — see Decision below)
  search/indexer.rs      (the closest existing precedent, read-only reference)
```

### Existing Patterns
The closest precedent is the search indexer's detached-task idiom
(`search/indexer.rs:16-23`): a write triggers `tokio::spawn(async move { ... })`, and a failure
inside that task is logged, never propagated back to the caller. This story's helper follows the
same shape, applied to a DB write instead of an external embedding call.

### Decisions That Govern This Story
- **DRIFT-001** (parent spec Section 19): "build the detached atime/mtime bump as a small helper
  in `purge.rs` or `core/fs_ops.rs`, following `search/indexer.rs`'s log-on-failure convention,
  and expose a test-only completion hook (e.g. a `tokio::sync::Notify` fired after the spawned
  task finishes) so tests can await completion deterministically instead of polling with a fixed
  timeout." This story implements that resolution verbatim. Place the helper in `core/fs_ops.rs`
  (not a new `purge.rs`, since `purge.rs` doesn't exist until US-0005 and this helper has nothing
  to do with purge sweeps — it is used by every read/write path).
- **DEC-010** (parent spec Section 17): atime/mtime updates are async, best-effort, matching
  `search/indexer.rs:16-23`'s detached-task idiom, specifically to avoid write-amplifying the
  hot read path.

### Applicable NFRs
Section 7.5 (Observability): failures inside the detached task are logged at the existing
`tracing` DEBUG/ERROR levels (no OpenTelemetry collector exists in this codebase — see parent
spec Section 7.5), never silently swallowed without a log line.

## Functional Requirements

This story has no `FR-NEW-` of its own — it is infrastructure DRIFT-001 requires before FR-NEW-001
and FR-NEW-002 (owned by US-0003) can be implemented as specified.

### DRIFT-001 resolution
- **Spec says:** "asynchronously and best-effort, a failed bump never surfaces as a tool error"
  (FR-NEW-001/FR-NEW-002 normative text).
- **Business Rules:**
  - The helper SHALL accept an async closure/future performing the DB write and spawn it via
    `tokio::spawn`, returning immediately to the caller.
  - A failure inside the spawned future SHALL be logged (`tracing::debug!` or `warn!`, matching
    the search indexer's log-on-failure convention) and SHALL NOT propagate to the caller in any
    form.
  - A **test-only** completion hook SHALL be exposed (feature-gated behind `#[cfg(test)]`, e.g. a
    `tokio::sync::Notify` fired once the spawned future resolves, success or failure) so tests can
    `notify.notified().await` instead of polling with a fixed sleep/timeout.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Injectable failing future | A closure that always errors, used to exercise the log-on-failure path | fixture, test-only | ready |

### E2E-NEW-007 (owned here, consumed by US-0003's acceptance of FR-NEW-001)
- **Category:** Edge Case (best-effort semantics)
- **Scenario:** SC-002 (precondition for US-0003/US-0005)
- **Requirements:** DRIFT-001 resolution (feeds FR-NEW-001, implemented in US-0003)
- **Preconditions:** the helper is called with a future that always returns `Err(..)`.
- **Steps:**
  - Given the detached bump task is forced to fail (test double future)
  - When the caller invokes the helper and awaits only the helper's own return (not the spawned
    task)
  - Then the helper's call returns immediately with no error, and the test-only `Notify` fires
    once the spawned future has run and logged its failure
- **Cleanup:** none.
- **Priority:** Critical — this is the literal content of the "best-effort" wording DRIFT-001
  exists to close.

**Test Implementation (unit test, not an E2E harness test — this story has no HTTP/MCP surface
of its own)**
```rust
#[tokio::test]
async fn detached_write_never_surfaces_as_caller_error() {
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let n = notify.clone();
    spawn_best_effort(async move {
        n.notify_one();
        Err::<(), _>(anyhow::anyhow!("simulated DB failure"))
    });
    // The call above must already have returned by this point (no await on the spawned future).
    tokio::time::timeout(std::time::Duration::from_secs(2), notify.notified())
        .await
        .expect("spawned task should complete and notify within 2s");
}
```

**Signatures to implement**
```rust
/// Spawns `fut` detached; any error it returns is logged, never propagated.
/// `#[cfg(test)]` builds additionally fire `notify` (if provided via the test hook)
/// once `fut` resolves, so tests can await completion deterministically.
pub(crate) fn spawn_best_effort<F>(fut: F)
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static;
```

## Constraints

### Files Not to Touch
`storage/meta.rs`, `tools/admin.rs` — this story is the helper only, not its callers (US-0003
wires it into the read/write paths).

### Dependencies Not to Add
None — `tokio::sync::Notify` and `tokio::spawn` are already dependencies.

### Patterns to Avoid
Do not make the helper generic over a "notify on success vs failure" distinction the spec does
not ask for; it logs and notifies on completion regardless of outcome (US-0003's test E2E-NEW-007
exercises the failure case specifically, but the hook itself is outcome-agnostic).

### Scope Boundary
No atime/mtime-specific code here — that is US-0003's job, which consumes this helper.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` — this story adds a new, isolated module; it cannot regress anything
since nothing calls it yet.

### Behaviors That Must Not Change
N/A — new code only.

### API Contracts to Preserve
N/A.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
