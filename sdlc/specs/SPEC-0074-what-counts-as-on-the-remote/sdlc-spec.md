# What counts as "on the remote" for a tag publish: functional specification

> Id: SPEC-0074
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable rule, one module family, 2 requirements, no persisted change. Override with --depth.
> Status: designed
> Generated on: 2026-11-02
> Project: existing
> From backlog: BL-0103
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
Before a member publishes a tag, the product checks that the tag's commit already exists on the remote, and refuses otherwise. SPEC-0052 describes that check as "a branch tip the remote advertises", but the product accepts every place the remote advertises a commit (branches, pull request references, tags, the remote's default position). This increment writes the wider rule down, so the rule a member can observe is the rule that is specified and tested.

## 2. Current State
### 2.1 What users can do today
- A member can publish a tag whose commit is a branch tip on the remote, or an ancestor of one in history the project already holds.
- A member can also publish a tag whose commit is reachable only from a pull request reference, a remote tag, or the remote's default position; the product accepts it.
- A commit the remote advertises nowhere is refused with `Cannot publish tag '<tag>': commit <short id> is not on the remote. Publish its branch first.` and the remote is left untouched.
- A tip the project does not hold is skipped, never fetched: a commit whose only link to the remote is such a tip is refused.

### 2.2 Specs governing this area
SPEC-0052 FR-NEW-019 and DEC-020 say "a branch tip". Deviation: this increment replaces that wording by the wider rule (FR-MOD-001). SPEC-0052 is not modified.

### 2.3 What is verified today
One test covers a commit the remote lacks, one a published commit, one a commit reachable from another remote branch, one a tip held only on the remote. None covers a pull request reference, a remote tag, or the default position as the only link.

## 3. Scope
### 3.1 In scope
The rule that decides whether a tag's commit is on the remote, and its tests.
### 3.2 Out of scope, and why
- The cost of a refusal that walks every advertised reference: no observable effect at the sizes tested; not requested.
- The wording `Publish its branch first.` for a commit that is on no reference: unchanged.
- Publishing a tag never sends commits: unchanged.

## 5. Usage scenarios
### SC-001: publish a tag on a commit the remote holds under any reference
**Actor:** a project member (Alice).
**Preconditions:** local tag `v1.0` on commit `a1b2c3d`; the remote advertises `a1b2c3d` as the tip of a pull request reference `pull/7/head` and as no branch.
**Flow:** 1. Alice publishes `v1.0` → the tag is published; the remote holds `v1.0` at `a1b2c3d`.
**Postconditions:** the remote's branches are unchanged.
**Exceptions:**
- EXC-001a: the tip is not held by the project and is not fetched → see SC-002.

### SC-002: publish a tag on a commit the remote does not hold
**Actor:** Alice.
**Preconditions:** local tag `v1.0` on commit `a1b2c3d`; no reference of the remote leads to it in history the project holds.
**Flow:** 1. Alice publishes `v1.0` → refused with `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.`
**Postconditions:** the remote and the audit log are unchanged.

## 6. Functional requirements
### 6.2 Modified
#### FR-MOD-001 [EARS-U]: what counts as on the remote (modifies SPEC-0052/FR-NEW-019)
> The Remote exchange context SHALL treat a commit as on the remote when it equals a position the remote advertises under any reference (branch, pull request, tag or default position), or is an ancestor of such a position whose history the project already holds.
- **Before:** the wording said "a branch tip"; the behavior accepted every advertised position (Section 2.1). **After:** the statement above. **Reason:** a commit a pull request reference holds exists on the remote, so refusing it is a false refusal.
- **Business rules:** a position the project does not hold is skipped and never fetched; publishing never sends commits.

### 6.1 New
#### FR-NEW-001 [EARS-O]: commit not on the remote
> IF the commit of the tag being published is not on the remote under FR-MOD-001, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag '<tag>': commit <short id> is not on the remote. Publish its branch first.` and leave the remote and the audit log untouched.
- **Business rules:** `<short id>` is the commit's first 7 characters; this refusal comes before any refusal about the remote tag itself (SPEC-0052 FR-NEW-020).
- **Scenario:** SC-002. **Priority:** Must

## 9. Acceptance tests
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-MOD-001 | High |
| E2E-002 | New | happy | SC-001 | FR-MOD-001 | Critical |
| E2E-003 | New | failure | SC-002 | FR-NEW-001 | Critical |
| E2E-004 | New | edge | SC-001 | FR-MOD-001 | High |
| E2E-005 | New | edge | SC-001 | FR-MOD-001 | High |
| E2E-006 | New | failure | SC-001, SC-002 | FR-MOD-001, FR-NEW-001 | High |
| E2E-007 | New | edge | SC-001 | FR-MOD-001 | Medium |
| E2E-008 | New | side effect | SC-002 | FR-NEW-001 | Medium |
| E2E-009 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-001 | High |

Coverage: happy 2, failure 4, side effect 1, edge 3, state transition 0 (none changes), security 0 (not a security change). Happy to failure ratio 1:2.

### 9.2 New tests
#### E2E-001: commit at a branch tip
- **Given** Alice, local tag `v1.0` on `a1b2c3d`, and the remote's branch `main` at `a1b2c3d`. **When** she publishes `v1.0`. **Then** it is published and the remote holds `v1.0` at `a1b2c3d`. **And** the remote's branches are unchanged.
#### E2E-002: commit only under a pull request reference
- **Given** `v1.0` on `a1b2c3d`; the remote advertises `a1b2c3d` only as `pull/7/head`; the project holds the commit. **When** Alice publishes `v1.0`. **Then** it is published. **And** the remote holds `v1.0` at `a1b2c3d`.
#### E2E-003: commit the remote does not hold
- **Given** `v1.0` on `a1b2c3d`; the remote has branch `main` at `9e8d7c6`, unrelated to `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` **And** the remote has no `v1.0`.
#### E2E-004: commit pointed at only by a remote tag
- **Given** `v1.0` on `a1b2c3d`; the remote has no branch holding it but its tag `old` points at `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** it is published.
#### E2E-005: ancestor of a pull request tip
- **Given** `v1.0` on `a1b2c3d`, a parent of `b2c3d4e`; the remote advertises `b2c3d4e` only as `pull/7/head`; the project holds both. **When** Alice publishes `v1.0`. **Then** it is published.
#### E2E-006: tip the project does not hold
- **Given** `v1.0` on `a1b2c3d`, a parent of `b2c3d4e`, which the project does not hold; the remote advertises `b2c3d4e` as `pull/7/head`. **When** Alice publishes `v1.0`. **Then** the refusal is the E2E-003 text. **And** nothing was fetched: the project still lacks `b2c3d4e`.
#### E2E-007: the default position only
- **Given** the remote's default position is `a1b2c3d` and no branch carries it. **When** Alice publishes `v1.0` on `a1b2c3d`. **Then** it is published.
#### E2E-008: a refusal changes nothing
- **Side effect.** **Given** the E2E-003 set up. **When** Alice publishes `v1.0`. **Then** the remote's references and the audit log equal their state before. **And** local tag `v1.0` still exists.
#### E2E-009: the commit refusal comes first
- **Given** `v1.0` on `a1b2c3d` (not on the remote) and a different remote tag `v1.0` at `f6a7b8c`. **When** Alice publishes `v1.0` stating `f6a7b8c`. **Then** the refusal is the E2E-003 text, not a refusal about the remote tag.
#### E2E-010: a remote that advertises nothing
- **Given** an empty remote. **When** Alice publishes `v1.0` on `a1b2c3d`. **Then** the refusal is the E2E-003 text.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-MOD-001 | E2E-001, 002 | E2E-006 | E2E-004, 005, 007 |
| SC-002 | FR-NEW-001, FR-MOD-001 | E2E-001 (published commit passes) | E2E-003, 006, 009, 010 | E2E-005 (ancestor counts) |

Requirement coverage: FR-MOD-001 has E2E-001, 002, 004, 005, 006, 007 (6 tests); FR-NEW-001 has E2E-003, 006, 008, 009, 010 (5 tests).

## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Advertised position | a commit the remote lists under a reference when asked | every scenario |
| Pull request reference | a reference such as `pull/7/head` that a hosting service keeps | SC-001 |
| Default position | the commit the remote's default view points at | E2E-007 |

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
| SPEC-0052 says "a branch tip the remote advertises" | `sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:220-222`; DEC-020 at `sdlc/specs/SPEC-0052-git-tags/sdlc-design.md:214` |
| The tip set is every advertised position | `crates/core/src/tools/git_tags.rs:1474-1477` (`advertised.iter().map(|(_, oid)| *oid)`) |
| The listing returns every advertised reference | `crates/core/src/git/remote.rs:1170` (`advertise`, `heads.iter()` unfiltered) |
| A tip the project lacks is skipped | `crates/core/src/tools/git_tags.rs:1481-1486` (`repo.find_commit(tip).is_ok()`) |
| The refusal text and place in the order | `crates/core/src/tools/git_tags.rs:651-656` |
| Existing tests | `crates/core/src/tools/git.rs:26573` (E2E-071), `:26600` (E2E-072), `:26619` (E2E-073), `:26695` (E2E-084) |
Test command: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`

### 14.3 Design inputs
- Publish all (`git_tags.rs:1086`) shares the same reachability function, so it follows the same rule; add a case to confirm (signal: a publish all test with a pull request only commit).
- Whether the hidden or peeled entries (`^{}`) enter the set as commits: they do today; the rule counts them (signal: E2E-004 with an annotated remote tag).
- The refusal wording says "Publish its branch first", unchanged.
