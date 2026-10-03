# User Stories Index

> Spec ID: SPEC-0013
> Source Specification: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Nature: DEBT
> Depth: L
> Generated on: 2026-10-03
> Target tier: 2 (standard frontier), resolved from default (no --tier flag, no .spec.json)
> Total: 9 stories in 0 epics (tier 2: epics collapsed into stories per budget rule)

## Slicing Verdict
| Verdict | SLICEABLE |
|---|---|
| Epics refused at this tier | none |
| Carried drift entries | DDRIFT-001 as US-0001 |

## Implementation Order
| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-0001 | n/a | Verify rmcp 3.5.0 API claims before proceeding (DDRIFT-001) | DDRIFT-001, DR-004 | n/a | DT-DRIFT-001 | 0 | none | 2 | done |
| 2 | US-0002 | n/a | Add rmcp 3.5.0 dependency | DR-001 | n/a | DT-DEP-001, DT-DEP-002 | 2 | US-0001 | 2 | done |
| 3 | US-0003 | n/a | Migrate fs.* tools (35) to #[tool] methods | DR-002, DR-006, DR-007 | n/a | E2E-FS-001..003 | 1-2 | US-0002 | 2 | done |
| 4 | US-0004 | n/a | Migrate admin.* (10) + search.* (4) tools | DR-002 | n/a | E2E-ADM-001..003 | 1 | US-0002 | 2 | done |
| 5 | US-0005 | n/a | Migrate git.* core tools (39) | DR-002 | n/a | E2E-GIT-001..003 | 1 | US-0002 | 2 | done |
| 6 | US-0006 | n/a | Migrate git.auth* (4) + git.pr_* (6) tools | DR-002 | n/a | E2E-GITAUTH-001..003 | 1 | US-0002 | 2 | done |
| 7 | US-0007 | n/a | Finalize tool surface: golden contract green, 94-tool list | DR-005 | n/a | DT-005, DT-007 | 2-3 | US-0003,US-0004,US-0005,US-0006 | 2 | done |
| 8 | US-0008 | n/a | Transport swap: rmcp StreamableHttpService + agent client + break tests | DR-008,DR-009,DR-010 | n/a | DT-001..004 + 18 existing-test verdicts | 4 | US-0007 | 2 | done |
| 9 | US-0009 | n/a | Delete old mcp layer + final suite/clippy/fmt gate | DR-003 | n/a | DT-006, DT-FINAL-001, DT-008 (ack) | 4 | US-0010,US-0011,US-0012,US-0013 | 2 | todo |
| 10 | US-0010 | n/a | Wean fs.*/admin.*/search.* tool families off ToolRegistry (discovered drift) | DR-003 (surplus) | n/a | none new, preserve existing coverage | ~10 | US-0009-orig-attempt | 2 | done (partial, see report) |
| 11 | US-0011 | n/a | Wean git.*/git_auth.*/git_pr.* tool families off ToolRegistry (discovered drift) | DR-003 (surplus) | n/a | none new, preserve existing coverage | 4 | US-0009-orig-attempt | 2 | done |
| 12 | US-0012 | n/a | Replace web.*/context7.*/sqlite.*/db.*/doc.* ToolRegistry registration with a lightweight catalog-only structure (discovered drift) | DR-003 (surplus) | n/a | none new, preserve existing coverage | ~5 | US-0009-orig-attempt | 2 | done |
| 13 | US-0013 | n/a | Rewire api/openapi.rs catalog onto McpServer's rmcp router + the new lightweight catalogs; remove AppState.registry/register_all/EnabledFeatures (discovered drift) | DR-003 (surplus) | n/a | none new, preserve openapi.rs output shape | 2 | US-0010,US-0011,US-0012 | 2 | done |
| 14 | US-0014 | n/a | Fully decouple every tool family + testkit from ToolRegistry/ToolSchema/Args (literal DR-003 compliance) | DR-003 (surplus) | n/a | none new, preserve ~700 existing tests | 16 | US-0010,US-0011,US-0012 | 2 | todo |

## Dependency Graph
```
US-0001 (drift gate)
   └── US-0002 (dependency)
          ├── US-0003 (fs.*)
          ├── US-0004 (admin.*+search.*)
          ├── US-0005 (git.* core)
          └── US-0006 (git.auth*+git.pr_*)
                 └── US-0007 (golden contract closes out all 4 families)
                        └── US-0008 (transport swap, declared breaks)
                               └── US-0009 (first attempt: discovered the ~20-file blast radius,
                                            logged as drift/2026-10-03_04-05-29.md, did not
                                            delete anything, user authorized expanded scope)
                                      ├── US-0010 (wean fs.*/admin.*/search.* off ToolRegistry)
                                      ├── US-0011 (wean git family off ToolRegistry)
                                      ├── US-0012 (web/context7/sqlite/db/doc -> catalog-only)
                                      └── US-0013 (rewire openapi.rs onto the new catalogs)
                                             └── US-0009 (re-run: delete old layer, final gate)
```

> Note: US-0010/0011/0012 touch disjoint file sets (no two write the same file) and may run in
> parallel. US-0013 depends on all three. US-0009 is re-run once US-0013 lands.

## Coverage Verification (Phase 5 gate)
- Requirements in spec (`DR-`): 10 | assigned: 10 (DR-001→US-0002, DR-002→US-0003/0004/0005/0006,
  DR-003→US-0009, DR-004→US-0001, DR-005→US-0007, DR-006→restated in every story per Invariant 3
  owned by US-0003, DR-007→owned by US-0003 restated elsewhere, DR-008/009/010→US-0008) |
  unassigned: none
- Tests in spec (`DT-`): 8 (DT-001..008) | assigned: 8 (DT-001..004→US-0008, DT-005→US-0007,
  DT-006→US-0009, DT-007→US-0007, DT-008→acknowledged in US-0009, no new work owed) | unassigned: none
- Existing guardrail tests (spec Section 2.4/6.4): 33 + 2 agent + 1 full_stack_e2e occurrence = 35
  test sites | assigned: 35 (16 removed→US-0009; 12 app.rs + 4 token_screen.rs + 1 full_stack_e2e +
  2 agent/mcp.rs = 19 modified→US-0008) | unassigned: none
- Scenarios in spec: none (`SC-` ids not used in this DEBT spec's structure) | N/A, not an orphan
- SC-orphan FRs homed: n/a — this DEBT spec carries no scenario/FR traceability matrix; every
  `DR-`/`DT-` id was assigned directly against the spec's own Section 7 Implementation Order and
  Section 6.4 test ledger, cross-checked line by line (Phase 2.2 sweep substitute for this nature)
- Matrix-unassigned tests homed: n/a (no matrix in this spec's structure)
