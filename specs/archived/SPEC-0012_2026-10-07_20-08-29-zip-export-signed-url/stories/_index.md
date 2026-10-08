# User Stories Index

> Spec ID: SPEC-0012
> Source Specification: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Nature: FEAT
> Depth: L
> Generated on: 2026-10-07
> Target tier: 2 (standard frontier), resolved from default (no `--tier` flag, no `.spec.json`)
> Total: 4 stories in 0 epics (tier 2, epics are tier 3/4 only), plus 1 converge story (US-0005) appended by Phase 4.6

## Slicing Verdict
| Verdict | SLICEABLE-WITH-EXCEPTIONS |
|---|---|
| Epics refused at this tier | none |
| Carried drift entries | none (source spec's Implementability Gate shows zero open drift) |

**Exceptions:** US-0002 and US-0003 both exceed tier 2's nominal budget (2-4 FRs, 5-15 tests) and are
marked `min_tier: 1` instead — each is one function's exhaustive branch enumeration, and splitting
either further would fail the independent-verifiability floor check (2.6). Reviewed and confirmed
with the user at the Phase 3 slicing gate before generation.

## Implementation Order

| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-0001 | n/a | Foundation: `export_links` schema + storage primitives | 1 (FR-NEW-015) | — (structural) | 2 | 1 | none | 2 | done |
| 2 | US-0002 | n/a | Export creation: `fs.export_zip` tool + REST route + URL config | 9 (FR-NEW-001,002,003,004,005,006,007,008,016) | SC-001 | 21 | 3 | US-0001 | 1 | done |
| 3 | US-0003 | n/a | Signed URL download: route, atomic consume, concurrency, unconditional mount | 8 (FR-NEW-009,009b,010,011,012,013,017,018) | SC-002, SC-003, SC-004 | 30 | 3 | US-0001, US-0002 | 1 | done |
| 4 | US-0004 | n/a | Background purge sweep for expired exports | 1 (FR-NEW-014) | SC-005 | 6 | 1 | US-0001 | 2 | done |
| 5 | US-0005 | n/a | Converge gap: export_links multi-dialect conformance case | 1 (FR-NEW-015) | SC-001 | 1 | 1 | US-0001 | 2 | done |

## Dependency Graph

```
US-0001 (foundation: schema + storage primitives)
   |
   +--> US-0002 (export creation)
   |        |
   |        v
   +--> US-0003 (signed URL download)  [depends on US-0001 directly, and on US-0002
   |                                     for its full end-to-end tests E2E-NEW-059 etc.]
   |
   +--> US-0004 (purge sweep)  [independent of US-0002/US-0003, placed last for a
                                 simple linear loop, not because it is blocked]
```

## Coverage Verification (Phase 5 gate)

- Requirements in spec (`FR-NEW-`): 19 (FR-NEW-001..018 + FR-NEW-009b) | assigned: 19 | unassigned: none
  - US-0001: FR-NEW-015 (1)
  - US-0002: FR-NEW-001, 002, 003, 004, 005, 006, 007, 008, 016 (9)
  - US-0003: FR-NEW-009, 009b, 010, 011, 012, 013, 017, 018 (8)
  - US-0004: FR-NEW-014 (1)
  - Total: 1+9+8+1 = 19 ✓, no duplicates (each FR appears in exactly one story's "Functional
    Requirements" section)
- Tests in spec (`E2E-NEW-`): 59 | assigned: 59 | unassigned: none
  - US-0001: E2E-NEW-040, 050 (2)
  - US-0002: E2E-NEW-001, 002, 003, 004, 005, 006, 007, 008, 009, 010, 011, 012, 013, 014, 022, 039,
    041, 042, 043, 044, 045, 051, 053, 056 (24) — *wait, recount below*
  - US-0003: E2E-NEW-015, 016, 017, 018, 019, 020, 021, 023, 024, 025, 026, 027, 028, 029, 030, 037,
    038, 046, 047, 048, 049, 052, 054, 055, 057, 058, 059 (27) — *recount below*
  - US-0004: E2E-NEW-031, 032, 033, 034, 035, 036 (6)

  **Recounted exactly from the story files as written** (US-0002 and US-0003's test lists, counted
  from their own "Acceptance Tests" sections): US-0002 carries 001, 002, 003, 004, 005, 006, 007, 008,
  009, 010, 011, 012, 013, 014, 022, 039, 041, 042, 043, 044, 045, 051, 053, 056 = **24 tests**.
  US-0003 carries 015, 016, 017, 018, 019, 020, 021, 023, 024, 025, 026, 027, 028, 029, 030, 037, 038,
  046, 047, 048, 049, 052, 054, 055, 057, 058, 059 = **27 tests**. Total: 2 + 24 + 27 + 6 = **59** ✓,
  matching the spec's own count exactly, no duplicates (each test id appears in exactly one story's
  "Acceptance Tests" section; cross-checked against the spec's §12.1 table and §11 traceability
  matrix).
- Scenarios in spec: 5 (SC-001..SC-005) | covered: 5 | uncovered: none
  - SC-001: US-0002 | SC-002: US-0003 | SC-003: US-0003 | SC-004: US-0003 | SC-005: US-0004
- SC-orphan FRs homed: FR-NEW-015/016/018 were cross-cutting in the spec's own traceability matrix
  (all three routed to SC-001 "for traceability" even though FR-NEW-018 semantically spans the whole
  feature); routed here by **which story creates the capability**: FR-NEW-015 and FR-NEW-016 stay
  with their creation-side home (US-0001 and US-0002 respectively, since §16's config field is built
  in US-0002), FR-NEW-018 is split across US-0002 (tool-contract/CLI verification, E2E-039/053) and
  US-0003 (the DELETE-route verification, E2E-054, and formal FR ownership, since "no revocation" is
  ultimately a property of the download route's surface).
- Matrix-unassigned tests homed: none were unassigned at the test-row level (every one of the 59 rows
  in the spec's §12.1 table already carries a Scenario and FR-refs value) — the only sweep performed
  here was re-homing FR-NEW-018's two SC-001-tagged tests (039, 053) to US-0002 alongside its other
  SC-001 tests, consistent with where the tool-contract/CLI checks actually run.
