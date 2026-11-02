# Which branch's tags a fetch receives: functional specification

> Id: SPEC-0075
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable rule (which branch decides the tags received), one module family, 2 requirements, no persisted change. Override with --depth.
> Status: designed
> Generated on: 2026-11-02
> Project: existing
> From backlog: BL-0099
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
When a member fetches or pulls, the product brings in the remote tags that belong to one branch's history. SPEC-0052 says "the fetched branch" without saying which branch that is for a fetch. This increment writes the rule down and tests it: the branch checked out in the project decides, or the remote's default branch when none is checked out; a pull uses the branch it pulls. Tags that belong only to other branches are not brought in until a fetch or pull of that branch.

## 2. Current State
### 2.1 What users can do today
- A fetch brings in the remote tags that point into the history of the checked-out branch, and reports them as added.
- When no branch is checked out, the remote's default branch decides.
- A pull brings in the tags of the history of the branch it pulls.
- A remote tag that only another branch's history contains is not brought in and is not mentioned in the result.
- Every fetch updates all remote branches; only tags are limited to one branch.

### 2.2 Specs governing this area
SPEC-0052 FR-NEW-009 ("the fetched branch's history"), FR-NEW-010 (conflicting local tag), SC-005. Deviation: this increment states which branch is "the fetched branch" for a fetch (FR-NEW-001) and what a later fetch receives (FR-NEW-002). SPEC-0052 is not modified.

### 2.3 What is verified today
One test covers a tag on another branch being ignored by a fetch while on `main`. None covers the no-checked-out-branch case, the pull branch rule, a later fetch of the other branch, or a tag shared by two histories.

## 3. Scope
### 3.1 In scope
The branch rule for fetch and pull, and its tests.
### 3.2 Out of scope, and why
- Receiving tags from every remote branch, or every remote tag: not chosen by the owner (2026-11-02); the rule stays "one branch".
- An option to pick the branch for tags: not requested.
- Tag conflicts and reporting lists: unchanged (SPEC-0052 FR-NEW-009, FR-NEW-010).

## 5. Usage scenarios
### SC-001: fetch on the checked-out branch
**Actor:** a project member (Alice).
**Preconditions:** the remote has branches `main` and `experimental`; tag `v1.0` is on `main`'s history; tag `v9.0` is only on `experimental`; Alice has `main` checked out.
**Flow:** 1. Alice fetches → `v1.0` is reported as added; `v9.0` is neither brought in nor mentioned.
**Postconditions:** the project lists `v1.0` and not `v9.0`.
**Exceptions:**
- EXC-001a: the remote is down → the fetch fails and no tag is brought in.

### SC-002: fetch with no checked-out branch
**Actor:** Alice.
**Preconditions:** same remote; no branch is checked out; the remote's default branch is `main`.
**Flow:** 1. Alice fetches → `v1.0` is added; `v9.0` is not.

### SC-003: pull, and a later fetch of the other branch
**Actor:** Alice.
**Preconditions:** same remote.
**Flow:** 1. Alice pulls `experimental` → `v9.0` is added (the pulled branch decides). 2. Alice pulls `main` → `v9.0` is not removed and `main`'s tags are as before.
**Postconditions:** both `v1.0` and `v9.0` are listed.
**Exceptions:**
- EXC-003a: Alice pulls `main` first → `v9.0` is not brought in.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: the branch that decides on a fetch
> WHEN a member fetches THE Remote exchange context SHALL bring in the remote tags that point into the history of the checked-out branch, or into the history of the remote's default branch when no branch is checked out.
- **Business rules:** a tag whose commit is in that history is brought in even if it is also in another branch's history; a tag only in another branch's history is not brought in and not mentioned in the result; branches are updated for every remote branch regardless.
- **Scenario:** SC-001, SC-002. **Priority:** Must

#### FR-NEW-002 [EARS-E]: the branch that decides on a pull
> WHEN a member pulls a branch THE Remote exchange context SHALL bring in the remote tags that point into the history of that branch, and no others.
- **Business rules:** a later pull or fetch of another branch brings in that branch's tags; tags already brought in stay.
- **Scenario:** SC-003. **Priority:** Must

## 9. Acceptance tests
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001 | High |
| E2E-002 | New | failure | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | happy | SC-002 | FR-NEW-001 | High |
| E2E-004 | New | edge | SC-003 | FR-NEW-001, FR-NEW-002 | High |
| E2E-005 | New | happy | SC-003 | FR-NEW-002 | High |
| E2E-006 | New | failure | SC-003 | FR-NEW-002 | High |
| E2E-007 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-008 | New | side effect | SC-001 | FR-NEW-001 | Medium |
| E2E-009 | New | failure | SC-001 | FR-NEW-001 | Medium |
| E2E-010 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-011 | New | edge | SC-002 | FR-NEW-001 | Medium |
| E2E-012 | New | side effect | SC-002 | FR-NEW-001 | Medium |
| E2E-013 | New | side effect | SC-003 | FR-NEW-002 | Medium |
| E2E-014 | New | failure | SC-003 | FR-NEW-002 | Medium |
| E2E-015 | New | failure | SC-002 | FR-NEW-001 | Medium |

Coverage: happy 3, failure 6, side effect 3, edge 3, state transition 0 (none changes), security 0 (not a security change). Happy to failure ratio 3:6 (1:2).

### 9.2 New tests
#### E2E-001: fetch on the checked-out branch
- **Given** Alice with `main` checked out; remote tag `v1.0` on `main`'s history. **When** she fetches. **Then** `v1.0` is reported as added and listed in the project.
#### E2E-002: tag on another branch is ignored
- **Given** the E2E-001 remote plus `experimental` holding tag `v9.0` (plain) and `v9.1` (annotated, message `outside`) only. **When** Alice, on `main`, fetches. **Then** neither is listed and neither name appears in the result.
#### E2E-003: no checked-out branch (happy)
- **Given** no branch checked out; the remote's default branch is `main`. **When** Alice fetches. **Then** `v1.0` is added and `v9.0` is not.
#### E2E-004: a later fetch of the other branch
- **Given** E2E-002's end state, then Alice checks out `experimental`. **When** she fetches. **Then** `v9.0` and `v9.1` are added; `v1.0` stays and is not reported again.
#### E2E-005: pull decides by the pulled branch
- **Given** `main` checked out, the E2E-002 remote. **When** Alice pulls `experimental`. **Then** `v9.0` and `v9.1` are reported as added.
#### E2E-006: pull of `main` does not bring in the other branch's tags
- **Given** the E2E-002 remote, Alice with nothing received yet. **When** she pulls `main`. **Then** `v1.0` is added and `v9.0` is not listed.
#### E2E-007: a tag in both histories
- **Given** tag `v0.5` on the commit where `experimental` branched off `main`. **When** Alice fetches on `main`, and on a fresh project pulls `experimental`. **Then** `v0.5` is added both times.
#### E2E-008: nothing else changes
- **Side effect.** **Given** E2E-002's set up. **When** Alice fetches. **Then** the remote's tags are unchanged, the project's tag list is exactly `v1.0`, and the result lists no kept-local tag.
#### E2E-009: remote down
- **Given** Alice with `main` checked out and a remote that does not exist. **When** she fetches. **Then** the fetch fails with the connection failure text and no tag is listed.

#### E2E-010: no checked-out branch, other branch's tag
- **Failure.** **Given** no branch checked out, the remote's default branch `main`, and `v9.0` only on `experimental`. **When** Alice fetches. **Then** `v9.0` is not listed and not mentioned in the result.
#### E2E-011: a detached position behaves like no branch
- **Edge.** **Given** Alice has checked out the commit `a1b2c3d` rather than a branch, and the remote's default branch is `main`. **When** she fetches. **Then** `v1.0` is added and `v9.0` is not.
#### E2E-012: a fetch with no checked-out branch changes nothing else
- **Side effect.** **Given** E2E-010's set up. **When** Alice fetches. **Then** the project's tag list is exactly `v1.0` and the remote's tags are unchanged.
#### E2E-013: pulling both branches
- **Side effect.** **Given** the E2E-002 remote. **When** Alice pulls `experimental`, then `main`, then `experimental` again. **Then** the project lists `v1.0`, `v9.0` and `v9.1` once each, and the third pull reports no tag added.
#### E2E-014: a pull fails
- **Failure.** **Given** Alice and a remote that does not exist. **When** she pulls `experimental`. **Then** the pull fails with the connection failure text and no tag is listed.
#### E2E-015: the remote has no default branch
- **Failure.** **Given** no branch checked out in the project and a remote whose default position names no branch. **When** Alice fetches. **Then** the fetch succeeds, the remote's branches are updated, and no tag is added, with no tag error.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001 | E2E-001 | E2E-002, 009 | E2E-007 |
| SC-002 | FR-NEW-001 | E2E-003 (default branch decides) | E2E-010, 015 | E2E-011 |
| SC-003 | FR-NEW-002 | E2E-005 | E2E-006, 014 | E2E-004 |

Side effects: SC-001 E2E-008; SC-002 E2E-012; SC-003 E2E-013. Per scenario: SC-001 has E2E-001, 002, 007, 008, 009 (5); SC-002 has E2E-003, 010, 011, 012, 015 (5); SC-003 has E2E-004, 005, 006, 013, 014 (5).
Requirement coverage: FR-NEW-001 has E2E-001, 002, 003, 004, 007, 008, 009, 010, 011, 012, 015 (11 tests); FR-NEW-002 has E2E-004, 005, 006, 007, 013, 014 (6 tests).

## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Checked-out branch | the branch the project is currently on | SC-001 |
| Remote's default branch | the branch the remote names as its default | SC-002 |
| History of a branch | the branch's commits and all their ancestors | every scenario |
| Tag brought in | a remote tag now listed in the project | every scenario |

## 13. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix
### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| The rule says "the fetched branch's history" | `sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:176-180` |
| A fetch uses the checked-out branch, else the remote default | `crates/core/src/tools/git.rs:3915-3918` (`TagScope::Branch` / `RemoteDefault`) |
| A pull uses the pulled branch | `crates/core/src/tools/git.rs:6811` |
| The fetch picks the tip of that branch | `crates/core/src/git/remote.rs:1374-1381` |
| Only tags are limited, branches are all updated | `crates/core/src/git/remote.rs:1311-1366` (`fetch_refspec`, `download_tags(None)`) |
| A tag on another branch is ignored and unmentioned | `crates/core/src/tools/git.rs:25800-25817` (E2E-042) |
Test command: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`

### 14.3 Design inputs
- E2E-004 and E2E-005 need a way to check out `experimental` and to pull a named branch; both already exist as tools (signal: the test helpers fail to compile if absent).
- A fetch with a detached HEAD has no checked-out branch; today it falls to the remote default (`git.rs:3917-3918`). This is the same as no branch, and is not changed (signal: E2E-003).
