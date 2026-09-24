# User Stories Index

> Source Specification: `specs/2026-09-23_08-47-57-mcp-tool-annotation-hints.md`
> Nature: DEBT
> Depth: L
> Generated on: 2026-09-24
> Target tier: 2 (standard frontier), resolved from the default (no `--tier` flag, no `.spec.json`)
> Total: 11 stories in 0 epics (epics collapse into the story at tier 2)

## Slicing Verdict

| | |
|---|---|
| Verdict | **SLICEABLE** |
| Epics refused at this tier | none |
| Carried drift entries | none — the spec's own drift register (`DDRIFT-001`) is already `Status: resolved` in the source spec; nothing open to carry |

## Implementation Order

| Order | ID | Epic | Title | DRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-001 | n/a | Tool annotations schema infrastructure | DR-006, DR-007 | n/a | DT-001, DT-002 | 1 | — | 2 | todo |
| 2 | US-002 | n/a | Contract golden annotation awareness | DR-008 | n/a | DT-005 (prepared) | 1 | US-001 | 2 | todo |
| 3 | US-003 | n/a | Annotate admin, context7, db tools (17) | DR-001..005 (subset) | n/a | 1 | 3 | US-001 | 2 | todo |
| 4 | US-004 | n/a | Annotate doc, document, edit tools (10) | DR-001..005 (subset) | n/a | 1 | 3 | US-001 | 2 | todo |
| 5 | US-005 | n/a | Annotate editor, git_auth, git_pr tools (13) | DR-001..005 (subset) | n/a | 1 | 3 | US-001 | 2 | todo |
| 6 | US-006 | n/a | Annotate listing, metadata, read tools (13) | DR-001, DR-005 | n/a | 1 | 3 | US-001 | 2 | todo |
| 7 | US-007 | n/a | Annotate search, search_semantic, sqlite tools (16) | DR-001..005 (subset) | n/a | 1 | 3 | US-001 | 2 | todo |
| 8 | US-008 | n/a | Annotate web, write, lifecycle tools (15) | DR-001..005 (subset) | n/a | 1 | 3 | US-001 | 2 | todo |
| 9 | US-009 | n/a | Annotate git.rs tools (39) | DR-001..005 (subset) | n/a | 2 | 1 | US-001 | 2 | todo |
| 10 | US-010 | n/a | Registry-wide annotation invariant tests | (test-only) | n/a | DT-003, DT-004 | 1 | US-003, US-004, US-005, US-006, US-007, US-008, US-009 | 2 | todo |
| 11 | US-011 | n/a | Regenerate golden contract and update docs | DR-008 (completion) | n/a | DT-005 (completion) | 3 | US-010 | 2 | todo |

## Dependency Graph

```
US-001 (schema infra)
  ├── US-002 (contract golden awareness)
  ├── US-003 (admin/context7/db)      ─┐
  ├── US-004 (doc/document/edit)       │
  ├── US-005 (editor/git_auth/git_pr)  ├── all feed → US-010 (registry-wide invariants)
  ├── US-006 (listing/metadata/read)   │                  │
  ├── US-007 (search/search_semantic/sqlite)              │
  ├── US-008 (web/write/lifecycle)     │                  │
  └── US-009 (git.rs)                 ─┘                  │
                                                            ▼
                                              US-011 (golden regen + docs)
                                              [also depends on US-002 for
                                               contract_golden.rs readiness]
```

## Coverage Verification (Phase 5 gate)

- Requirements in spec (`DR-`): 8 total (DR-001..DR-008).
  - DR-006, DR-007 → US-001 (exactly one story each).
  - DR-008 → US-002 (infrastructure) and US-011 (completion/exercise). This
    single requirement spans two stories by design: US-002 prepares the
    mechanism, US-011 is where the golden file is actually regenerated and the
    gate turns green. Both stories cite DR-008's EARS text verbatim.
  - DR-001, DR-002, DR-003, DR-004, DR-005 → these five rules are the
    classification system applied identically across every one of the seven
    tool-annotation stories (US-003 through US-009), because each rule spans
    all 123 tools and no single story's file-budget can hold the full set.
    Each of US-003..US-009 quotes the five rules verbatim and applies only the
    subset relevant to its own per-tool table (Invariant 3: self-contained).
    This is a deliberate, documented exception to strict one-story-per-DR
    ownership, driven by the nature of DR-001..DR-005 being global
    classification rules rather than per-file requirements; no orphan exists
    (every DR text appears in at least one story, and every one of the 123
    tools is covered by exactly one story's table).
  - Zero unassigned DRs.
- Tests in spec (`DT-`): 5 total (DT-001..DT-005).
  - DT-001, DT-002 → US-001 (exactly one story each).
  - DT-003, DT-004 → US-010 (exactly one story each).
  - DT-005 → prepared in US-002 (the mechanism exists and passes vacuously),
    exercised to completion in US-011 (the assertion that actually matters:
    the golden file matches the fully annotated registry). Same documented
    dual-story pattern as DR-008, since DT-005 IS the executable form of
    DR-008.
  - Zero unassigned DTs.
- Scenarios in spec: none declared (this is a structural DEBT spec; Section 6
  states "Not applicable: no dual-maintained contract exists" for scenario
  based compatibility tests). No scenario coverage gap.
- SC-orphan FRs homed: DR-007 (behavior invariance) is a cross-cutting
  constraint with no dedicated file of its own; it is formally owned by
  US-001 (the story that first changes `to_list_entry()`, the highest-risk
  point for an accidental behavior change) and is additionally restated
  verbatim as a Non-Regression item in every one of the 11 stories, per the
  DEBT convention (Invariant 3).
- Matrix-unassigned tests homed: none found beyond the two dual-story tests
  above (DT-005 shares its home with DR-008 by construction, not by omission).
- 123 per-tool classifications (Section 3.1): distributed exactly once each
  across US-003 (17) + US-004 (10) + US-005 (13) + US-006 (13) + US-007 (16) +
  US-008 (15) + US-009 (39) = 123. Verified by direct source enumeration
  (`ToolSchema::new(` occurrences per file) during slicing, cross-checked
  against the spec's own family totals in Section 3.1's "Total check" line.
- 24 touched files (Section 2): `mcp/schema.rs` (US-001), `tools/contract_golden.rs`
  (US-002), the 19 `tools/*.rs` family files (US-003 through US-009),
  `tools/all.rs` (US-010), `tool-contract-golden.json` + `TOOL_CONTRACT.txt` +
  `.agent_docs/tools.md` (US-011) = 1+1+19+1+3 = 25. Note: this is one more
  than the spec's stated 24, because the spec's Section 2 lists `AGENTS.md` as
  a 25th row explicitly marked "0 (optional)" and not counted in its own "24
  files" total; this story set does not touch `AGENTS.md` either, matching the
  spec's own accounting (24 files with a mandatory change + 1 optional,
  untouched).

**DEBT non-regression restated (Invariant 3):** every story's Non Regression
section restates verbatim "the existing test suite passes unmodified" plus the
exact commands `cargo test --workspace`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo fmt --all -- --check`, with the single
documented exception that `tool_contract_golden_is_current` is expected to
fail from US-003 through US-010 (by the spec's own design, Section 7) and is
restored to green in US-011.
