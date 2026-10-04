# US-0007: Finalize tool surface — golden contract green, 94-tool list

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 7
> Depends On: US-0003, US-0004, US-0005, US-0006
> Complexity: M
> min_tier: 2
> Files touched: 2-3

## Objective
With all 94 `#[tool]` methods now present on `McpServer` (fs: US-0003, admin+search: US-0004, git
core: US-0005, git.auth+git.pr: US-0006), prove the full tool surface is intact before the
transport swap (US-0008) touches anything client-visible: the golden contract test passes without
the regeneration escape hatch, and `tools/list` returns exactly the 94 names in `TOOL_CONTRACT.txt`.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0, `schemars` (inputSchema derivation).

### Relevant File Structure
```
crates/core/src/mcp/server.rs                  # read/finalized, not family-split anymore
crates/core/src/tools/contract_golden.rs        # test run against the new registrations
tool-contract-golden.json                       # regenerated ONLY if a named mismatch is found (see DR-005)
```

### Existing Patterns
`tool_contract_golden_is_current` (`crates/core/src/tools/contract_golden.rs:15,34,139`) currently
asserts every tool's exact serialized `inputSchema` against the frozen golden file. Run it with
`MCPFS_REWRITE_TOOL_CONTRACT` unset — DR-005 forbids using the regeneration flag to make it pass.

### Decisions That Govern This Story
- **DR-005** (spec Section 3.1): "`tool-contract-golden.json` and the test
  `tool_contract_golden_is_current` SHALL remain green under the new `#[tool]`-based registration,
  without using the `MCPFS_REWRITE_TOOL_CONTRACT=1` regeneration escape hatch. If the `rmcp`
  `schemars`-derived schema for any one of the 94 tools cannot be made to serialize identically to
  the frozen golden entry, that tool's golden entry SHALL be regenerated deliberately, the diff
  SHALL be reviewed by a human, and the tool SHALL be listed by name in Section 5 as a declared
  break."
- **DDRIFT-001 resolution** (US-0001): confirms whatever `schemars`/`rmcp` schema-emission
  behavior this story depends on.

### Applicable NFRs
DR-006 (restated per Invariant 3): byte-identical observable output for every non-declared-break
input. If any tool's schema cannot match byte-for-byte, it must be named explicitly here, not
silently absorbed.

## Functional Requirements

### DR-005: Golden contract stays green without the regeneration flag
- **EARS:** see "Decisions That Govern This Story" above (verbatim).
- **Inputs / Outputs:** Input: all 94 `#[tool]` methods on `McpServer`. Output:
  `cargo test -p mcp-fs --lib tool_contract_golden_is_current` passes with
  `MCPFS_REWRITE_TOOL_CONTRACT` unset.
- **Business Rules:** any tool whose schema cannot be made to match exactly is named explicitly in
  this story's report (not silently regenerated) and flagged back to the user as an amendment to
  the spec's Section 5 declared-break list — per Invariant 1, this story does not unilaterally
  expand the declared-break list; it reports the finding and waits for the spec to be amended if a
  new break is discovered.

## Acceptance Tests

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| `tool-contract-golden.json` | the existing frozen 94-tool schema file | repository root | ready |
| `TOOL_CONTRACT.txt` | the human-readable twin, 94 entries | repository root | ready |

### DT-005: Golden contract test passes without the regeneration flag
- **Category:** happy
- **Requirements:** DR-005
- **Preconditions:** all 94 `#[tool]` methods registered on `McpServer`.
- **Steps:** Given the migrated server's `#[tool]` registrations; When `cargo test -p mcp-fs --lib
  tool_contract_golden_is_current` runs with `MCPFS_REWRITE_TOOL_CONTRACT` unset; Then the test
  passes, proving the 94 `inputSchema` values serialize identically to `tool-contract-golden.json`.
- **Cleanup:** none.
- **Priority:** Critical.

### DT-007: tools/list returns exactly the 94 expected names
- **Category:** happy
- **Requirements:** DR-002 (closure check across all four migration stories)
- **Preconditions:** `McpServer`'s router fully populated.
- **Steps:** Given the migrated server; When calling `tools/list` (directly against the router, no
  HTTP transport needed yet — that is US-0008); Then the returned list has exactly 94 entries and
  the set of names is identical (as a set) to the 94 names currently listed in `TOOL_CONTRACT.txt`,
  verified by diffing the two name sets and asserting the diff is empty.
- **Cleanup:** none.
- **Priority:** Critical.

## Constraints

### Files Not to Touch
`crates/core/src/app.rs` (no route change — US-0008); `crates/core/src/mcp/{mod,registry,schema,args}.rs`
(still live, deleted only in US-0009).

### Dependencies Not to Add
None.

### Patterns to Avoid
Running with `MCPFS_REWRITE_TOOL_CONTRACT=1` to force a pass — DR-005 explicitly forbids this as
the normal path.

### Scope Boundary
No transport change. This story only closes out the tool-surface migration started across
US-0003..0006.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace`, specifically `tool_contract_golden_is_current` and every existing
`tools/*.rs` unit test (still exercising the old `ToolRegistry` path, unaffected).

### Behaviors That Must Not Change
DR-006, DR-007 (restated per Invariant 3, see US-0003). DR-005's byte-identical schema bar.

### API Contracts to Preserve
`TOOL_CONTRACT.txt`, `tool-contract-golden.json` — both for all 94 tools.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
