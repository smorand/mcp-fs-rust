# User Stories Index

> Spec ID: SPEC-0011
> Source Specification: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Nature: FEAT
> Depth: L
> Generated on: 2026-10-06
> Target tier: 2 (standard frontier), resolved from default (no `--tier` flag, no `.spec.json` found)
> Total: 8 stories in 0 epics (tier 2 collapses epic level)

## Slicing Verdict
| Verdict | SLICEABLE |
|---|---|
| Epics refused at this tier | none |
| Carried drift entries | none — source spec's Implementability Gate verdict is `IMPLEMENTABLE`, drift register empty |

## Implementation Order
| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-0001 | n/a | `trash_entries` persistence layer | FR-NEW-001 | (infra) | 0 (own unit verification) | 5 | none | 2 | done |
| 2 | US-0002 | n/a | `fs.delete` writes the trash entry, collision-safe | FR-NEW-002, 004, 005, 006, 013 | SC-004 | 15 | 1 | US-0001 | 2 | done |
| 3 | US-0003 | n/a | Sweep writes the trash entry | FR-NEW-003 | SC-003 | 5 | 1 | US-0001, US-0002 | 2 | done |
| 4 | US-0004 | n/a | `fs.trash_list` tool | FR-NEW-007, 008, 011, 016 | SC-001 | 16 | 2 | US-0001, US-0002, US-0003 | 2 | done |
| 5 | US-0005 | n/a | `fs.trash_restore` tool | FR-NEW-009, 010 | SC-002 | 14 | 2 | US-0004 | 2 | done |
| 6 | US-0006 | n/a | `/app/trash` GUI screen | FR-NEW-012, 018 | SC-006 | 7 | 2 | US-0004, US-0005 | 2 | done |
| 7 | US-0007 | n/a | `admin.create_project` retention parameters | FR-NEW-014, 015 | SC-005 | 6 | 3 | none (parallel-safe) | 2 | in-progress |
| 8 | US-0008 | n/a | Contract regeneration & docs | FR-NEW-017 | (process) | 0 (structural verification) | 4 | US-0001..0007 | 2 | todo |

**Cost caveat:** n/a at tier 2 (the caveat applies only to tiers 3/4).

**Budget note:** US-0002 (5 FRs) and US-0004 (16 tests) each land one unit over their nominal T2
ceiling (4 FRs, 15 tests respectively). Both are low-decision-density, fully-specified mechanical
work, well under the binding files-touched constraint — accepted rather than fragmented further.

## Dependency Graph
```
US-0001 (schema)
   |
   v
US-0002 (fs.delete writes + collision retry)
   |
   v
US-0003 (sweep writes)
   |
   v
US-0004 (fs.trash_list + backfill + ACL pattern)
   |
   v
US-0005 (fs.trash_restore)
   |
   v
US-0006 (GUI screen)
   |
   v
US-0008 (contract regen) <---- also depends on ----
   ^                                                |
   |                                          US-0007 (create_project params, no deps, parallel-safe)
   |______________________________________________|
```
US-0007 has no dependency on US-0001..0006 and may be implemented concurrently with them; it must
still complete before US-0008.

## Coverage Verification (Phase 5 gate)
- Requirements in spec (`FR-NEW-`): 18 | assigned: 18 | unassigned: none
- Tests in spec (`E2E-NEW-`): 63 | assigned: 63 | unassigned: none
- Scenarios in spec: 6 (SC-001..SC-006) | covered: 6 | uncovered: none
- SC-orphan FRs homed: none — every FR was homed by its direct FR-tag in spec §12.1, not by the
  (narrower) 4-column scenario matrix; the matrix's own omissions (e.g. FR-NEW-017 mapped to no
  scenario, ACL tests mapped to a synthetic "(ACL)" row, process/cross rows) were all resolved
  against the FR-refs column instead of guessed.
- Matrix-unassigned tests homed: 22 tests that the spec's 4-column traceability matrix does not
  list under any SC row (e.g. E2E-402, 412, 414, 425-428, 435-436, 440, 442, 446, 450-454, 456,
  458-461) are all homed below by their FR-refs tag (§12.1) and by which capability's story their
  test *driver* actually requires — notably E2E-424 and E2E-436 (tagged list-side FR-011/007) were
  homed to US-0005 (restore) because their drivers call `fs.trash_restore`, not `fs.trash_list`, and
  could not pass before that tool exists.

| Test ID(s) | Home | Why |
|---|---|---|
| 401, 402, 404, 405, 441, 443, 444, 446, 448, 449, 450, 453, 454, 456, 458 | US-0002 | driver is `fs.delete`/`delete_path` |
| 403, 440, 445, 447, 457 | US-0003 | driver is `sweep_project_files` |
| 406-416, 428, 438, 442, 451, 452 | US-0004 | driver is `fs.trash_list` (ACL tests 428/442 homed here: list alone suffices per their own description) |
| 417-427, 436, 437, 439 | US-0005 | driver is `fs.trash_restore`, or (424, 436) needs restore to exist even though tagged to a list-side FR |
| 432-435, 455, 462, 463 | US-0006 | driver is the HTTP GUI |
| 429-431, 459-461 | US-0007 | driver is `admin.create_project` |

## Notes for `/implement`
- US-0002 and US-0003 together introduce one new shared symbol, `rename_with_collision_retry`
  (`crates/core/src/core/fs_ops.rs`) — US-0002 builds it, US-0003 only calls it. Do not let US-0003's
  implementation re-derive or duplicate the retry logic.
- US-0004 and US-0005 share one module, `crates/core/src/tools/trash.rs`, and one backfill helper
  (FR-NEW-011) — US-0004 builds the helper for its bulk (list) case, US-0005 reuses it for its
  single-entry (restore) case. Do not let US-0005 re-implement the reconstruction algorithm.
- US-0007 is independent and may run in parallel with US-0001..0006 if the implementing loop
  supports concurrent dispatch; sequential execution in the order above is also correct and is the
  default assumption if not.
- US-0008 must verify US-0001 through US-0007 are all `Status: done` in this file before
  regenerating the tool contract — regenerating against a partial tool set produces a diff that
  cannot be meaningfully reviewed.
