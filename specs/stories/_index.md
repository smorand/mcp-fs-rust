# User Stories Index

> Source Specification: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Nature: FEAT
> Depth: L
> Generated on: 2026-09-21
> Target tier: 2 (standard frontier), resolved from the default (no `--tier` flag, no `.spec.json`)
> Total: 31 stories in 0 epics (epics collapse into the story at tier 2)

## Slicing Verdict

| | |
|---|---|
| Verdict | **SLICEABLE** |
| Epics refused at this tier | none |
| Carried drift entries | `DRIFT-001` homed into US-002, the story implementing the requirement it blocks, rather than a standalone head-of-backlog story (agreed with the user; a standalone drift story here would have nothing to verify) |
| Source spec audit verdict | `NOT-IMPLEMENTABLE` at round 4, with all four findings closed and the root cause fixed by `FR-NEW-199`. See the concern note below. |

## Implementation Order

| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-001 | n/a | git_operations table, state model and purge registration | 7 | 1 | 1 | 2 | — | 2 | todo |
| 2 | US-002 | n/a | Shared merge engine and git.merge, with the conflict response contract | 15 | 4 | 36 | 2 | US-001 | 2 | todo |
| 3 | US-003 | n/a | git.merge_resolve, git.merge_abort and resolution mechanics | 11 | 6 | 30 | 1 | US-002 | 2 | todo |
| 4 | US-004 | n/a | Conflict edge semantics: delete/modify, both-deleted, binary, type change | 4 | 2 | 13 | 1 | US-003 | 2 | todo |
| 5 | US-005 | n/a | In-progress guard and git.status operation reporting | 5 | 2 | 8 | 2 | US-003 | 2 | todo |
| 6 | US-006 | n/a | Squash merge | 1 | 1 | 5 | 1 | US-002 | 2 | todo |
| 7 | US-007 | n/a | git.remote_pull on the shared conflict model, on_conflict removed | 3 | 4 | 22 | 2 | US-003, US-004 | 2 | todo |
| 8 | US-008 | n/a | Branch creation and ref-name validation | 7 | 1 | 11 | 1 | US-005 | 2 | todo |
| 9 | US-009 | n/a | Branch switch and the dirty-volume guard | 3 | 2 | 14 | 1 | US-008 | 2 | todo |
| 10 | US-010 | n/a | Branch delete, branch_reset and tracking divergence | 8 | 2 | 14 | 1 | US-009 | 2 | todo |
| 11 | US-011 | n/a | Stash save, list and drop | 8 | 1 | 20 | 2 | US-009 | 2 | todo |
| 12 | US-012 | n/a | Stash apply and pop, with the conflict path | 5 | 2 | 13 | 1 | US-003, US-011 | 2 | todo |
| 13 | US-013 | n/a | Rebase todo validation and planning | 5 | 3 | 25 | 2 | US-005 | 2 | todo |
| 14 | US-014 | n/a | Rebase execution: pick, squash, drop, reword | 5 | 4 | 15 | 1 | US-013 | 2 | todo |
| 15 | US-015 | n/a | Rebase pause, continue and abort | 7 | 5 | 23 | 1 | US-003, US-014 | 2 | todo |
| 16 | US-016 | n/a | Cherry-pick with continue and abort | 6 | 3 | 27 | 1 | US-003, US-015 | 2 | todo |
| 17 | US-017 | n/a | git.reset soft: pointer-only move | 4 | 4 | 10 | 1 | US-005 | 2 | todo |
| 18 | US-018 | n/a | git.reset hard: volume rewrite and orphaning | 2 | 6 | 16 | 1 | US-017 | 2 | todo |
| 19 | US-019 | n/a | git.revert, including merge commits and the conflict path | 7 | 4 | 25 | 1 | US-003, US-017 | 2 | todo |
| 20 | US-020 | n/a | Remote add, remove and list | 6 | 1 | 14 | 2 | US-005 | 2 | todo |
| 21 | US-021 | n/a | Named remotes on push, fetch and pull, with local:remote refspec | 5 | 3 | 13 | 2 | US-020 | 2 | todo |
| 22 | US-022 | n/a | Force push with a mandatory lease | 6 | 6 | 28 | 2 | US-021 | 2 | todo |
| 23 | US-023 | n/a | OAuth scope request, validation and reporting | 7 | 5 | 17 | 4 | — | 2 | todo |
| 24 | US-024 | n/a | Provider client seam, base URL resolution and transport safety | 6 | 3 | 14 | 4 | US-023 | 2 | todo |
| 25 | US-025 | n/a | git.pr_create and the normalized pull-request model | 4 | 2 | 18 | 2 | US-024 | 2 | todo |
| 26 | US-026 | n/a | git.pr_list with state filtering | 1 | 2 | 15 | 2 | US-025 | 2 | todo |
| 27 | US-027 | n/a | git.pr_get and git.pr_diff | 3 | 1 | 14 | 2 | US-025 | 2 | todo |
| 28 | US-028 | n/a | git.pr_merge with provider refusals surfaced | 2 | 3 | 19 | 2 | US-027 | 2 | todo |
| 29 | US-029 | n/a | git.pr_review | 2 | 3 | 17 | 2 | US-027 | 2 | todo |
| 30 | US-030 | n/a | Tool registration, frozen contract and count assertions | 4 | 4 | 15 | 4 | US-029 | 2 | todo |
| 31 | US-031 | n/a | Blanket authorization coverage and documentation parity | 3 | 11 | 24 | 4 | US-030 | 2 | todo |

## Dependency Graph

```
US-001 (git_operations table)
  └─ US-002 (merge engine + git.merge + conflict contract)   <- carries DRIFT-001
       ├─ US-003 (merge_resolve/abort + resolution mechanics)
       │    ├─ US-004 (conflict edge semantics)
       │    │    └─ US-007 (remote_pull rework, on_conflict removed)
       │    └─ US-005 (in-progress guard + git.status)
       │         ├─ US-008 -> US-009 -> US-010   (branch create / switch / delete+reset)
       │         │                └─ US-011 -> US-012   (stash save-list-drop / apply-pop)
       │         ├─ US-013 -> US-014 -> US-015 -> US-016 (rebase plan / exec / pause / cherry-pick)
       │         ├─ US-017 -> US-018   (reset soft / reset hard)
       │         │        └─ US-019   (revert)
       │         └─ US-020 -> US-021 -> US-022   (remotes / named remotes / force-push lease)
       └─ US-006 (squash merge)

US-023 (OAuth scope)          <- second dependency-free entry point, parallel track
  └─ US-024 (provider client seam)
       └─ US-025 (pr_create + normalized model)
            ├─ US-026 (pr_list)
            └─ US-027 (pr_get + pr_diff)
                 ├─ US-028 (pr_merge)
                 └─ US-029 (pr_review)
                      └─ US-030 (registration + contract + counts)
                           └─ US-031 (blanket authz + docs parity)
```

Two dependency-free entry points: **US-001** (git core) and **US-023** (OAuth/provider track). They
touch disjoint files and can proceed in parallel; they converge only at US-030.

## Coverage Verification (Phase 5 gate)

- Requirements in spec (`FR-`): **162** | assigned: **162** | unassigned: **none** | duplicated: **none**
- Tests in spec (`E2E-`): **536** | assigned: **536** | unassigned: **none** | duplicated: **none**
- Scenarios in spec: **30** | covered: **30** | uncovered: **none**
- SC-orphan FRs homed: **18** (the wire-shape contract group added at audit, plus `FR-NEW-276`/`348`/`349`/`218`/`222`) — homed into US-001, US-002, US-003, US-030 and US-031, all ahead of their consumers
- Matrix-unassigned tests homed: **143** (26% of the suite: the `E2E-MOD`/`E2E-DEL` delta set and the whole `E2E-NEW-800..961` gap-closure band)
- Floor check: **0 violations** — no story's tests invoke a tool built by a later story
- File budget (tier 2: 3-5): max **4**, no breach

## Concerns Carried From the Specification

1. **`FR-NEW-276` is only fully observable on a shared relational backend.** On SQLite, `purge_repo`
   (`crates/mcp-fs/src/git/repo.rs:146-161`) deletes the index file outright and never reads `TABLES`,
   so US-001's purge consequence is verified in the opt-in PostgreSQL/SQL Server suites plus a
   `TABLES.len() == 4` assertion in the default build. US-001 is deliberately thin (1 test) for this reason.
2. **US-002 and US-003 exceed the nominal tier-2 test budget** (36 and 30 tests against 5-15), accepted
   by the user. Splitting them re-creates a floor breach: the conflict contract has no verifiable
   surface until a tool exposes it. Files touched stays at 1-2 and decision density is near zero,
   because every response shape is pinned by `FR-NEW-199`.
3. **The parent spec's audit ended `NOT-IMPLEMENTABLE` at round 4** with all findings closed and the
   recurring root cause fixed structurally (`FR-NEW-199`). No fifth audit was run. The residual risk is
   a surviving wire-shape disagreement in a test body. **Mitigation: US-002 should generate the response
   structs directly from the `FR-NEW-199` table**, so any disagreement fails to compile rather than shipping.
4. **`DRIFT-001` is open** and is carried by US-002. It must be closed before the implementation branch merges.
