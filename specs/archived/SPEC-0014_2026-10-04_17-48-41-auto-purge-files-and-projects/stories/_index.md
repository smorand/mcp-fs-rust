# User Stories Index

> Spec ID: SPEC-0014
> Source Specification: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Nature: FEAT
> Depth: L
> Generated on: 2026-10-04
> Target tier: 2 (standard frontier), resolved from default (no `--tier` flag, no `.spec.json`)
> Total: 12 stories in 0 epics (epic grouping applies only at tier 3/4)

## Slicing Verdict
| Verdict | SLICEABLE |
|---|---|
| Epics refused at this tier | none |
| Carried drift entries | DRIFT-001 as US-0001 |

## Implementation Order
| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-0001 | n/a | Detached best-effort DB-write helper (drift) | — | — | 1 (E2E-NEW-007) | 1-2 | none | 2 | done |
| 2 | US-0002 | n/a | Schema & configuration foundation | FR-NEW-018 | — | 3 | 2 | none | 2 | done |
| 3 | US-0003 | n/a | Atime/mtime tracking on read & write | FR-NEW-001,002,003 | SC-002/003 (precond.) | 14 | 2-3 | US-0001 | 2 | done |
| 4 | US-0004 | n/a | Access gate for soft-deleted projects | FR-NEW-013 | cross-cutting | 6 | 1 | US-0002 | 2 | done |
| 5 | US-0005 | n/a | File-purge sweep logic | FR-NEW-007 | SC-002 | 8 | 2 | US-0002, US-0003 | 2 | done |
| 6 | US-0006 | n/a | Project soft-delete sweep logic | FR-NEW-008 | SC-003 | 8 | 2 | US-0005 | 2 | done |
| 7 | US-0007 | n/a | Background loop + CLI purge verb (core) | FR-NEW-009,010,011 | SC-004 | 9 | 3 | US-0005, US-0006 | 2 | done |
| 8 | US-0008 | n/a | Grace-period sweep + CLI global mode | FR-NEW-012 | SC-007 | 7 | 2 | US-0007 | 2 | done |
| 9 | US-0009 | n/a | `admin.set_purge_config` | FR-NEW-004,005,006 | SC-001 | 9 | 2 | US-0002 | 2 | done |
| 10 | US-0010 | n/a | `admin.list_deleted_projects` | FR-NEW-014 | SC-005 | 5 | 2 | US-0006 | 2 | done |
| 11 | US-0011 | n/a | `admin.undelete_project` | FR-NEW-015,016 | SC-006 | 6 | 2 | US-0006, US-0004 | 2 | done |
| 12 | US-0012 | n/a | Browser screen `/app/deleted-projects` | FR-NEW-017 | SC-005/006 | 5 | 2 | US-0010, US-0011 | 2 | done |
| 13 | US-0013 | n/a | Converge gap: test the production `admin.set_purge_config` handler | FR-NEW-004,005,006 | SC-001 | n/a | 1 | US-0009 | 2 | done |
| 14 | US-0014 | n/a | Converge gap: end-to-end access-gate test through a real fs.*/REST call | FR-NEW-013 | cross-cutting | n/a | 1 | US-0004 | 2 | done |

Total tests: 80, each owned by exactly one story (US-0001 owns E2E-NEW-007; US-0003's prose
references it but does not claim it — see Coverage Verification below for the gate check).

## Dependency Graph
```
US-0001 ─────────────────────┐
                              v
US-0002 ──┬──────────────> US-0003 ──┐
          │                           v
          ├──> US-0004              US-0005 ──> US-0006 ──┬──> US-0007 ──> US-0008
          │       ^                                       │
          │       └───────────────────────────────────────┤
          ├──> US-0009                                     ├──> US-0010 ──┐
          │                                                 └──> US-0011 ─┴──> US-0012
          └─────────────────────────────────────────────────────> (US-0011 also needs US-0004)
```
Plain reading: US-0001 and US-0002 have no dependencies and can start immediately. US-0003 needs
US-0001. US-0004 and US-0009 need only US-0002. US-0005 needs US-0002+US-0003. US-0006 needs
US-0005. US-0007 needs US-0005+US-0006. US-0008 needs US-0007. US-0010 and US-0011 need US-0006
(US-0011 additionally needs US-0004). US-0012 needs US-0010+US-0011.

## Coverage Verification (Phase 5 gate)
- Requirements in spec (`FR-NEW-`): 18 | assigned: 18 | unassigned: none
- Tests in spec (`E2E-NEW-`): 80 | assigned: 80 | unassigned: none (verified by diffing the full
  `E2E-NEW-001..080` range against every `### E2E-NEW-XXX:` header across all 12 story files —
  exact match, zero gaps, zero duplicates)
- Scenarios in spec: 7 | covered: 7 | uncovered: none
  (SC-001→US-0009; SC-002→US-0003+US-0005; SC-003→US-0003+US-0006; SC-004→US-0007+US-0008;
  SC-005→US-0010+US-0012; SC-006→US-0011+US-0012; SC-007→US-0008)
- SC-orphan FRs homed: none — the parent spec's own Section 12.1 area table already partitions
  all 80 tests cleanly by area with no matrix-unassigned remainder, so no separate orphan sweep
  story was needed.
- Matrix-unassigned tests homed: 0 (none existed to home)
