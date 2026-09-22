# User Stories Index

> Source Specification: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Nature: FEAT
> Depth: L
> Generated on: 2026-09-21
> Target tier: 1 (top frontier), resolved from an explicit instruction in the partitioning session
> Total: 19 stories in 0 epics (epics collapse into the story at tier 1)

## Slicing Verdict

| | |
|---|---|
| Verdict | **SLICEABLE** |
| Epics refused at this tier | none |
| Carried drift entries | DRIFT-003 (US-001), DRIFT-004 + DRIFT-005 (US-003), DRIFT-010 (US-006), DRIFT-009 (US-014), DRIFT-008 (US-015) — each homed into the story implementing the requirement it blocks, not as standalone head-of-backlog stories |
| Source spec audit verdict | `NOT-IMPLEMENTABLE` at round 3, escalated and accepted by the user. See the concern note below. |

## Implementation Order

| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|-------|----|----|-------|-----|-----------|-------|-------|------------|----------|--------|
| 1 | US-001 | n/a | The git.hosts map, its boot validation, and its single owner | 6 | 001, 006 | 17 | 5 | none | 1 | done |
| 2 | US-002 | n/a | Resolve a remote URL to a provider by parsing and exact match | 4 | 001, 004, 005, 009, 011 | 16 | 2 | US-001 | 1 | done |
| 3 | US-003 | n/a | Token identity becomes (person, host), in memory, on disk, and across backends | 4 | 002, 006, 008, 008, 009 | 13 | 6 | US-001 | 1 | done |
| 4 | US-004 | n/a | git.token_set: seed a token the caller already holds | 7 | 002, 003 | 20 | 2 | US-002, US-003 | 1 | done |
| 5 | US-005 | n/a | An expired token fails the operation before any socket opens | 3 | 004, 014 | 10 | 3 | US-002, US-003 | 1 | done |
| 6 | US-006 | n/a | git.auth takes a host, git.auth_status reports one entry per held host | 4 | 003, 007 | 19 | 2 | US-003 | 1 | done |
| 7 | US-007 | n/a | git.auth_revoke removes exactly one host and never reaches across hosts | 2 | 007 | 8 | 1 | US-003, US-006 | 1 | done |
| 8 | US-008 | n/a | Extract git/remote.rs, validate the URL, and record origin at clone | 6 | 004, 005, 009, 011, 012 | 19 | 3 | US-002, US-005 | 1 | done |
| 9 | US-009 | n/a | git.remote_push: send one branch, fast-forward only | 6 | 009, 010 | 17 | 3 | US-008 | 1 | done |
| 10 | US-010 | n/a | git.remote_fetch: objects and remote-tracking refs, nothing else | 5 | 011, 012 | 14 | 3 | US-008 | 1 | done |
| 11 | US-011 | n/a | git.remote_pull: fast-forward, applied atomically | 4 | 012, 013, 015 | 16 | 2 | US-010 | 1 | done |
| 12 | US-012 | n/a | A pull charges the write quota for the bytes it actually writes | 3 | 012, 015 | 6 | 2 | US-011 | 1 | done |
| 13 | US-013 | n/a | A diverged pull: refuse, or merge under one global strategy | 5 | 013, 015 | 11 | 2 | US-011 | 1 | done |
| 14 | US-014 | n/a | Remote timeout, lock release, and the frozen failure messages | 2 | 010, 012 | 7 | 4 | US-009, US-010, US-011, US-013 | 1 | done |
| 15 | US-015 | n/a | The token screen: routes, and identity from three sources | 3 | 006 | 14 | 3 | US-004, US-007 | 1 | done |
| 16 | US-016 | n/a | The screen lists held hosts, ordered, and never another person's | 3 | 006 | 9 | 2 | US-015, US-006 | 1 | done |
| 17 | US-017 | n/a | The screen seeds and revokes, protected by a single-use csrf_token | 4 | 006 | 16 | 2 | US-015, US-016, US-004, US-007 | 1 | done |
| 18 | US-018 | n/a | Every remote operation is audited and traced, and no token is ever emitted | 3 | 004 | 9 | 2 | US-008, US-009, US-010, US-011 | 1 | done |
| 19 | US-019 | n/a | Regenerate the frozen tool contract at 63 tools | 2 | 002, 009, 012 | 6 | 4 | US-004, US-006, US-007, US-008, US-009, US-010, US-011, US-013, US-014 | 1 | done |

## Dependency Graph

```
US-001  host map + boot validation + url dep      (no deps)
  |
  +-- US-002  host resolution, exact match
  +-- US-003  token identity (person, host) + migrate

US-004  git.token_set                <-- US-002, US-003
US-005  expiry enforced pre-network   <-- US-002, US-003
US-006  git.auth + git.auth_status    <-- US-003
US-007  git.auth_revoke               <-- US-003, US-006

US-008  extract git/remote.rs + URL safety + origin   <-- US-002, US-005
  +-- US-009  git.remote_push
  +-- US-010  git.remote_fetch
        +-- US-011  pull fast-forward + atomicity
              +-- US-012  pull write-quota delta
              +-- US-013  diverged pull / merge

US-014  failure contract: timeout + frozen prefixes  <-- US-009, US-010, US-011, US-013
US-015  token screen routes + identity               <-- US-004, US-007
  +-- US-016  screen listing + isolation             <-- also US-006
        +-- US-017  screen mutations + csrf
US-018  audit + tracing + redaction                  <-- US-008, US-009, US-010, US-011
US-019  tool contract regeneration  (LAST)           <-- every schema story
```

Ordering follows the specification's Section 14.1, which states these pairs are load-bearing: config and host map before every consumer; the token key change before the seeding tool and before the auth tool modifications; the schema migration in the same change as the key change; `git::remote` extraction before the three new tools; `origin` persistence before push, fetch and pull; fetch before pull; fast-forward pull before merge; contract regeneration last.

## Coverage Verification (Phase 5 gate)

- Requirements in spec (`FR-NEW`/`FR-MOD`): 76 | assigned: 76 | unassigned: none | duplicated: none
- Tests in spec (`E2E-NEW`): 247 | assigned: 247 | unassigned: none | duplicated: none
- Scenarios in spec: 15 | covered: 15 | uncovered: none
- SC-orphan FRs homed: none. The spec's Section 11 matrix maps all 76 requirements and all 247 tests; verified by command, not by eye.
- Matrix-unassigned tests homed: none. `FR-MOD-001` is the only near-miss: it is never the *first* requirement on any test, so it is homed in US-002, which is literally "provider resolution replaces substring detection".
- `E2E-MOD-001` to `E2E-MOD-026`: the 26 modified existing tests are enumerated by `file:line` in Section 9.3 and distributed as **Non Regression** items into the story touching their file — store.rs and persistence.rs to US-003, git_auth.rs to US-003/US-006/US-007, git.rs and contract_golden.rs to US-019.

### Test ownership rule

Each test is owned by the story holding the **first** requirement on that test's own `**Requirements:**` line in Section 12. This rule exists because Section 11's matrix rows overlap: E2E-NEW-071 appears under both SC-009 and SC-010, E2E-NEW-107 and E2E-NEW-108 under both SC-006 and cross-cutting identity, E2E-NEW-151 to E2E-NEW-153 under both cross-cutting security and FR-NEW-054/FR-NEW-072. Scenario-derived slicing alone would have duplicated them across stories.

## Budget Note

Tier 1 budget is 3 to 5 FRs, 8 to 20 tests and 5 to 8 files per story. Four stories sit outside the FR band, deliberately. US-004 (7 FRs) and US-008 / US-009 (6 FRs) are many low-density rules on a single tool surface, which decision density says to keep together. US-007, US-012, US-014 and US-019 (2 FRs) are small high-precision units: the revoke semantics, the write-quota basis that audit round 3 had to resolve a contradiction over, the frozen failure prefixes, and the golden contract. Files touched, the binding constraint, is within budget everywhere: every story lands in 1 to 6 files.

## Concern Carried From The Source Spec

Section 18 records verdict `NOT-IMPLEMENTABLE`: three audit rounds ran, the F count fell 13, 10, 5, no finding ever survived a round, and round 3's five findings were amended but **not** re-audited before the three-round limit was reached. The documented convergence pattern, all-new findings each round, indicates residual naming and response-shape gaps rather than functional gaps in the interview's coverage. The user was informed and chose to proceed. Practical consequence: if a story turns out to under-determine a response shape or a message string, that is the expected residue. Raise it as a spec defect rather than inventing the answer, exactly as each story's self-review item 3 requires.

Section 15 leaves two behaviour-neutral TBDs: the `expires_at` representation for a non-expiring token (relax the non-null column or adopt a sentinel; US-004 owns the observable behaviour), and whether dirty-volume detection needs a cheaper signal than a full tree walk (US-011 carries it). A third, punycode normalisation asserted by E2E-NEW-062, is owned by US-002.
