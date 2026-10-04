# US-0001: Verify rmcp 3.5.0 API claims before proceeding (DDRIFT-001)

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 0 (research verification; no production code changes)

## Objective
The spec's Section 5 declared-break analysis (DDEC-001: `initialize` becomes mandatory, JSON
becomes the default framing) is sourced from context7-retrieved docs.rs pages, not from reading
`rmcp`'s own published source (Drift Register `DDRIFT-001`). Before any `#[tool]` method is
written, this story confirms or corrects those claims against the actual crate, because the whole
declared-break premise (DDEC-001) rests on them. If the claims are wrong in the direction of "a
no-initialize HTTP path does exist", DDEC-001 is void and the spec must be amended before any
further story proceeds.

## Technical Context

### Stack
Rust 2024, Cargo, `rmcp` crate (not yet a dependency — added in US-0002, which depends on this
story completing first with a PASS verdict).

### Relevant File Structure
No files are modified by this story. It produces a verification note (this story's own completion
record) confirming or correcting the claims below.

### Decisions That Govern This Story
- **DDEC-001** (spec Section 5): "accept both breaks above, immediately, with no dual-maintenance
  window" — this decision's premise is exactly what this story checks.
- **DDRIFT-001** (spec Section 10, Drift Register): "At DR-001 (Section 7, step 1), before writing
  a single `#[tool]` method, read `rmcp` 3.5.0's actual source (via `cargo doc --open -p rmcp` or
  the vendored `~/.cargo/registry` copy) for `StreamableHttpServerConfig`, `NeverSessionManager`,
  and the streamable-HTTP request-dispatch path, and confirm or correct every claim in Section 5
  before proceeding to step 2."

### Bounded Context
Spec perimeter: the claims under test are in Section 1 ("Target structure" / "Observable
behavior"), Section 2.1 (file-by-file occurrence table), and Section 5 ("Declared Breaks", points
1 and 2).

## Functional Requirements

### DR-004 (precondition check, no new work owed)
- **EARS:** "Every one of the 33 existing tests enumerated in Section 2.4 SHALL carry an explicit
  verdict in this document's Section 6.4 before implementation starts... A test with no verdict
  blocks Phase 6 under G10."
- **Check:** the spec's own Section 6.4 table already assigns a verdict (`removed` / `modified: ...`)
  to all 33 tests, and the running count (16 + 17 = 33) matches Section 2.4's guardrail count.
  Confirm this arithmetic holds by re-reading Section 6.4 and Section 2.4 side by side. This is a
  verification of the spec document, not an implementation task: do not edit the spec (Invariant
  4). If the counts do not reconcile, STOP and report to the user as a spec defect; do not proceed
  to US-0002.

### DDRIFT-001 (drift resolution)
- **EARS:** confirm or correct, against `rmcp` 3.5.0's actual source:
  1. Whether `rmcp::service::serve_directly` / `serve_directly_with_ct` documents a path for the
     **streamable-HTTP** transport (not only stdio/in-process) to skip the `initialize` handshake.
  2. The exact semantics of `StreamableHttpServerConfig::stateless_protocol_metadata_required`.
  3. The exact fallback trigger for `json_response` (does a plain `tools/call` with no preceding
     notification really default to `application/json`, switching to `text/event-stream` only when
     a notification precedes the final response).
- **Procedure:** add `rmcp = { version = "3.5.0", features = ["server",
  "transport-streamable-http-server"] }` to a scratch `Cargo.toml` (or temporarily to
  `crates/core/Cargo.toml`, reverted before this story's commit — this story ships no dependency
  change, that is US-0002's job), run `cargo doc --open -p rmcp` or read the vendored source
  under `~/.cargo/registry/src/`, and read the three items above directly.
- **Outcome, one of:**
  - **CONFIRMED**: all three claims hold as stated in Section 5. Report this; US-0002 proceeds
    unchanged.
  - **CONTRADICTED**: a no-`initialize` streamable-HTTP path, or an always-JSON-no-SSE-fallback
    mode, actually exists. **STOP.** Per Invariant 1, do not invent a workaround or silently adopt
    the newly discovered configuration — report the contradiction to the user as a spec defect
    requiring DDEC-001 to be revisited, and do not proceed to US-0002 until the spec is amended.

## Acceptance Tests

> **100% must pass.** No automated test exists for this story (DDRIFT-001 itself states: "There is
> no automated test for this, because it is a claim about a library's documented behavior"). The
> acceptance criterion is the reviewer's own reading of the `rmcp` 3.5.0 documentation/source,
> recorded as a PASS or CONTRADICTED verdict against each of the three claims in DDRIFT-001 above.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| rmcp 3.5.0 docs/source | `cargo doc -p rmcp` output or `~/.cargo/registry` vendored source, once a scratch dependency add makes it resolvable | fetched during this story | pending |

### DT-DRIFT-001: Confirm or contradict the three DDRIFT-001 claims
- **Category:** edge (process gate, not a code test)
- **Requirements:** DDRIFT-001
- **Preconditions:** none
- **Steps:** Given a scratch or temporary `rmcp = "3.5.0"` dependency resolvable via `cargo doc`;
  When the implementer reads `StreamableHttpServerConfig`, `NeverSessionManager`,
  `serve_directly`/`serve_directly_with_ct`, and the streamable-HTTP dispatch path in the actual
  crate source; Then each of the three claims in DDRIFT-001 is recorded as CONFIRMED or
  CONTRADICTED, with the exact source location (file path or docs.rs anchor) cited.
- **Cleanup:** revert any scratch `Cargo.toml` change made solely to resolve the crate for reading;
  US-0002 is the story that lands the real dependency.
- **Priority:** Critical — this story gates every other story in the spec.

## Constraints

### Files Not to Touch
`crates/core/Cargo.toml`, `Cargo.toml` (workspace) — any dependency change belongs to US-0002, not
here. If a scratch add is needed to make `cargo doc` resolve the crate, revert it before finishing
this story.

### Dependencies Not to Add
None landed permanently by this story.

### Patterns to Avoid
Do not substitute "the context7 citation is probably fine" for actually reading the source — that
is the exact gap DDRIFT-001 exists to close.

### Scope Boundary
This story verifies; it does not implement. No `#[tool]` method, no transport change, no test
rewrite happens here.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` is unaffected by this story (no code changes land).

### Behaviors That Must Not Change
None — no behavior changes in this story.

### API Contracts to Preserve
n/a.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5. In addition: confirm the verdict (PASS or
CONTRADICTED) was recorded with a literal source citation, not a restatement of the spec's own
claim.
