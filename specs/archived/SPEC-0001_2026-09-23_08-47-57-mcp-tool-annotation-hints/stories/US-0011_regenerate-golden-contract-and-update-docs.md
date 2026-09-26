# US-0011: Regenerate the golden contract and update TOOL_CONTRACT.txt / tools.md

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 11
> Depends On: US-0010
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Regenerate `tool-contract-golden.json` from the now fully annotated live
registry, review the diff to confirm it shows only added `annotations` keys,
then update the two human-readable contract mirrors,
`TOOL_CONTRACT.txt` and `.agent_docs/tools.md`, so every tool's annotations
are documented for a human reader. This closes DR-008/DT-005 and completes
the spec's implementation order (Section 7, steps 4-7).

## Technical Context

### Stack
Rust 2024 test harness (`cargo test`), no new library code.

### Relevant File Structure
```
tool-contract-golden.json
TOOL_CONTRACT.txt
.agent_docs/tools.md
```

### Existing Patterns
Regeneration command, exact as documented in `contract_golden.rs`'s module
doc and in the spec's Section 7 step 4:
```
MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current
```
`TOOL_CONTRACT.txt` is the hand-maintained human mirror (referenced in
`crates/mcp-fs/src/tools/admin.rs:486` comment and `AGENTS.md:26`); it is not
test-generated. Each tool entry there already lists `params`/`required`;
extend each entry with its annotations, e.g.:
```
admin.delete_project
  Delete a project and recursively tear down its volume (owner or platform admin).
  params: project_id (required)
  annotations: destructiveHint=true, readOnlyHint=false, idempotentHint=true
```
Only emit the hints that are actually set (mirroring `to_list_entry()`'s
omit-when-`None` behavior); never invent a hint value not in the per-tool
tables from US-0003 through US-0009.
`.agent_docs/tools.md` header currently states "94 tools"; add an
"Annotations" column (or a legend section) to its per-family reference tables,
covering the same 123 tools this spec annotates.

### Data Model (excerpt)
None; this story edits two documentation files and regenerates one JSON
fixture via the existing test harness, no new struct or function.

### Decisions That Govern This Story
None invented. This story performs exactly the mechanical regeneration and
documentation update the spec's Section 7 steps 4, 6 and 7 describe, with no
new classification decision (every value was already decided in US-0003
through US-0009's tables).

### Applicable NFRs
None beyond DR-007 and DR-008.

### Bounded Context
Documentation and the frozen contract fixture; no source code.

## Functional Requirements

### DR-008: Contract regeneration covers annotations (completion)
- **EARS:** The `render()` function in `contract_golden.rs` SHALL include the
  `annotations` field (when present) in every rendered tool entry, and the
  `tool_contract_golden_is_current` test SHALL fail when a registered tool's
  annotations differ from the frozen `tool-contract-golden.json` entry.
- **Inputs / Outputs:** Running
  `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`
  rewrites `tool-contract-golden.json`; the diff MUST show only added
  `annotations` keys on the 94 frozen entries (the 94 tools covered by the
  contract: `fs.*`, `admin.*`, `git.*`, `git.auth*`, `git.pr_*`), zero changes
  to any `name`, `description`, or `inputSchema` value.
- **Business Rules:** After regeneration, re-run
  `cargo test -p mcp-fs --lib tool_contract_golden_is_current` (without the
  env var) to confirm it now passes.

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| fully annotated registry (all 123 tools, US-0001 through US-0010 merged) | live registry | in-repo | ready |
| pre-regeneration `tool-contract-golden.json` | 94 entries, no `annotations` key | repo fixture | ready |

### DT-005: contract golden regeneration is annotation-aware (completion)
- **Category:** happy
- **Scenario:** n/a (structural test, DEBT spec)
- **Requirements:** DR-008
- **Preconditions:** every tool family annotated (US-0003 through US-0009), DT-003/DT-004 in place (US-0010).
- **Steps:** Given the live registry with every tool annotated / When
  `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`
  is run / Then `tool-contract-golden.json` is rewritten with an
  `"annotations"` key added to every one of its 94 entries, and a subsequent
  `cargo test -p mcp-fs --lib tool_contract_golden_is_current` (no env var)
  passes.
- **Cleanup:** none; the regenerated file is the intended, committed change.
- **Priority:** Critical

### E2E-US011-01: full quality gate is green
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-007 (behavior invariance, final check across the whole
  spec)
- **Preconditions:** golden file regenerated, `TOOL_CONTRACT.txt` and
  `.agent_docs/tools.md` updated.
- **Steps:** Given the fully merged change / When
  `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D
  warnings`, `cargo fmt --all -- --check` are run / Then all three succeed
  with zero failures and zero warnings.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No `crates/mcp-fs/src/**/*.rs` file: every annotation value was already set
  in US-0001 through US-0010. This story only regenerates the golden fixture
  and updates the two documentation files.

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not hand-edit `tool-contract-golden.json`; it is regenerated only via the
  `MCPFS_REWRITE_TOOL_CONTRACT=1` command, and the diff is reviewed, not
  authored.
- Do not add, remove, or reclassify any hint value in `TOOL_CONTRACT.txt` or
  `.agent_docs/tools.md` beyond what US-0003 through US-0009's tables already
  fixed; this story documents, it does not decide.

### Scope Boundary
- Only `tool-contract-golden.json` (regenerated), `TOOL_CONTRACT.txt`,
  `.agent_docs/tools.md`.

## Non Regression

### Existing Tests That Must Pass
- `tool_contract_golden_is_current` passes at the end of this story (the one
  test that was expected to be red throughout US-0003 through US-0010 is now
  green, per spec Section 7's stated design).
- Every other test in the workspace, unmodified and green.
- **The existing test suite passes unmodified.** Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- The regenerated golden file's diff contains zero changes to any `name`,
  `description`, or `inputSchema` value; only `annotations` keys are added.

### API Contracts to Preserve
- `tools/list` JSON-RPC response shape: additive only, per Section 4 of the
  spec. `tool-contract-golden.json` schema: additive only.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
