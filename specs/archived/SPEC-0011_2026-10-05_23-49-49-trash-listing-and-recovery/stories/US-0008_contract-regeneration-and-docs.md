# US-0008: Contract regeneration & docs

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 8 (last — per FR-NEW-017 step 7, "once every tool/schema change from steps 4 and 6 is
> final")
> Depends On: US-0001, US-0002, US-0003, US-0004, US-0005, US-0006, US-0007
> Complexity: S
> min_tier: 2
> Files touched: 4

## Objective
Regenerate the frozen, machine-checked tool contract (`TOOL_CONTRACT.txt` / `tool-contract-golden.json`)
to include `fs.trash_list`, `fs.trash_restore`, and `admin.create_project`'s new params, and update
the project documentation that references the tool count and module list. This is the closing step
of the whole spec: it must run only after every tool and schema change from every prior story has
landed, never mid-sequence, or the golden-contract test fails against a partially-updated tool set.

## Technical Context

### Stack
Rust 2024, the project's own `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test` regeneration mechanism.

### Relevant File Structure
```
TOOL_CONTRACT.txt            # human-readable, frozen schema reference
tool-contract-golden.json    # machine-checked, serialized, three tests compare against it
AGENTS.md                    # module bullet list, tool count
.agent_docs/tools.md         # tool reference
```

### Existing Patterns
Regeneration command (AGENTS.md Key Commands): `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs
--lib tool_contract_golden_is_current`, then **review the diff** — it is never hand-edited (AGENTS.md:
*"Never hand edited: regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff
(`tools/contract_golden.rs`)."*).

### Decisions That Govern This Story
- This story implements FR-NEW-017's own step 7 directly; no other `DEC-XXX` governs it.

### Bounded Context
Spans all four contexts transitively (it documents the whole feature), but owns none of them itself
— it is process/documentation work, not a bounded-context implementation.

## Functional Requirements

### FR-NEW-017: required build order
- **EARS:** [EARS-U] "The system SHALL be implemented in this order: (1) FR-NEW-001 (`trash_entries`
  schema across sqlite/postgres/sqlserver); (2) the shared collision-retry helper used by
  FR-NEW-004/FR-NEW-005/FR-NEW-006; (3) FR-NEW-002, FR-NEW-003, FR-NEW-013 (`delete_path` and
  `sweep_project_files` writing or withholding `trash_entries` rows); (4) FR-NEW-007/FR-NEW-008/
  FR-NEW-011 (`fs.trash_list`) and FR-NEW-009/FR-NEW-010/FR-NEW-011 (`fs.trash_restore`); (5)
  FR-NEW-012 and FR-NEW-018 (the `/app/trash` screen, which calls only the functions built in step
  4); (6) FR-NEW-014/FR-NEW-015 (`admin.create_project` params, independent of steps 1-5); (7)
  `TOOL_CONTRACT.txt` and `tool-contract-golden.json` regeneration, last, once every tool/schema
  change from steps 4 and 6 is final."
- **Business Rules:** this story IS step 7. By the time it runs, US-0001 through US-0007 (steps 1-6)
  must already be `done` in `_index.md` — `/implement`'s dependency-ordered loop guarantees this, but
  verify it explicitly before regenerating: if any of US-0001..0007 is not `done`, STOP and do not
  regenerate against a partial tool set.

## Acceptance Tests

> **100% must pass.** This story has no spec-numbered `E2E-NEW-*` test — per the spec's own
> traceability matrix ("(process)" row, §11): *"n/a — structural: verified by the build succeeding
> in the stated order (§14) and by `cargo test --workspace` staying green at every step, not by a
> dedicated E2E test."*

### Verification (this story's acceptance criterion, in place of a numbered test)
- **Category:** Structural
- **Driver:** `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib
  tool_contract_golden_is_current`, then a manual, reviewed diff of `TOOL_CONTRACT.txt` and
  `tool-contract-golden.json`.
- **Steps:**
  - Given US-0001 through US-0007 are all `done`.
  - When the regeneration command runs.
  - Then `TOOL_CONTRACT.txt` gains `fs.trash_list` and `fs.trash_restore` entries (schema, params,
    required list, annotations, matching exactly what US-0004/US-0005 implemented) and
    `admin.create_project`'s entry gains the four new optional params (matching US-0007).
  - And the diff contains no change to any other tool's entry.
  - And `cargo test --workspace --all-features` passes in full (the three golden-contract tests plus
    every other test in the workspace).
- **Priority:** Critical

## Documentation Requirements
(spec §10, §9.4 — this story's actual deliverable)
- `AGENTS.md`: update the `crates/core/` module bullet list to add `trash_screen.rs`,
  `tools/trash.rs`, and a `trash_entries` bookkeeping note on the `purge.rs` bullet.
- `.agent_docs/tools.md`: add `fs.trash_list`/`fs.trash_restore` to the tool reference; update the
  tool count from 98 to 100.
- `TOOL_CONTRACT.txt`, `tool-contract-golden.json`: regenerated as above.

## Constraints

### Files Not to Touch
- No production code in `crates/core/src/` changes in this story — it is documentation and
  generated-artifact regeneration only.

### Dependencies Not to Add
- None.

### Patterns to Avoid
- Do not hand-edit `TOOL_CONTRACT.txt` or `tool-contract-golden.json` directly — always regenerate
  and review the diff.
- Do not run this story before US-0001..0007 are all done — the golden-contract test will fail
  against a partial tool set, and the diff would be impossible to review meaningfully.

### Scope Boundary
- Documentation and contract regeneration only.

## Non Regression

### Existing Tests That Must Pass
- The full `cargo test --workspace --all-features` suite, including every pre-existing
  golden-contract test.

### Behaviors That Must Not Change
- No existing tool's entry in `TOOL_CONTRACT.txt` changes.

### API Contracts to Preserve
- Every existing tool's frozen schema is byte-identical after regeneration; only the additions from
  US-0004/US-0005/US-0007 appear as a diff.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
