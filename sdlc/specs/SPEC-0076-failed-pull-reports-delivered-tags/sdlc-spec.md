# A failed pull reports the tags it delivered: functional specification

> Id: SPEC-0076
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable behavior (the text of a failed pull), one module family, 3 anticipated requirements, no persisted change. Override with --depth.
> Status: implemented
> Generated on: 2026-11-02
> Project: existing
> From backlog: BL-0100
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A pull first brings in the remote tags, then merges. When the merge step fails (a merge error, a quota refusal, a deadline), the tags are already delivered, but the refusal says nothing about them, and a retry cannot list them as added because they are now local. This increment makes the refusal name the tags the failed pull delivered, so the operator can see them.

## 2. Current State
### 2.1 What users can do today
- A pull that succeeds returns the tags it added and the tags it kept local.
- A pull that pauses on a merge conflict is not a failure and returns both lists.
- A pull that fails after the tags were brought in returns a refusal with no mention of them. The tags stay in the project.
- A retry of that pull reports those tags as neither added nor kept local, since they are now identical locally.

### 2.2 Specs governing this area
SPEC-0052 FR-NEW-009 (receive tags on pull), FR-NEW-010 (conflicting tag), and the paused merge rule (`sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:201`). Deviation: a failed pull now names its added tags (FR-NEW-001). SPEC-0052 is not modified.

### 2.3 What is verified today
Tests cover a pull that succeeds with tags, a pull that pauses on a conflict and a failed exchange bringing no tag. No test covers a pull that fails after the tags were delivered.

## 3. Scope
### 3.1 In scope
The added tags named in the refusal of a pull that fails after they were delivered.
### 3.2 Out of scope, and why
- Tags kept local on a failed pull: they differ from the remote's, so a retry reports them again; nothing is lost.
- A structured error with the list as data: an error carries a code and a text only.
- The wasted listing for an empty set of incoming tags: the same change is already specified (SPEC-0073, DR-002).

## 5. Usage scenarios
### SC-001: the merge fails after the tags arrived
**Actor:** a project member (Alice).
**Preconditions:** the remote holds new tags `v1.0` and `v2.0` on its history; the pull will fail at the merge step.
**Flow:** 1. Alice pulls → the pull is refused with its usual text followed by `Tags received before the failure: v1.0, v2.0.`
**Postconditions:** `v1.0` and `v2.0` are listed in the project.
**Exceptions:**
- EXC-001a: the pull fails before any tag is delivered (the remote is down) → the usual refusal, no sentence.
- EXC-001b: no tag was delivered → the usual refusal, no sentence.

### SC-002: a long list
**Actor:** Alice.
**Preconditions:** the remote delivers 25 tags `t01` to `t25`, then the merge fails.
**Flow:** 1. Alice pulls → the refusal ends `Tags received before the failure: t01, t02, t03, t04, t05, t06, t07, t08, t09, t10, t11, t12, t13, t14, t15, t16, t17, t18, t19, t20 and 5 more.`

### SC-003: retry
**Actor:** Alice.
**Preconditions:** SC-001 happened and the cause is removed.
**Flow:** 1. Alice pulls again → the pull succeeds; neither tag is reported as added nor as kept local.
**Postconditions:** both tags are still listed.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: the refusal names the delivered tags
> WHEN a pull fails after it brought in at least one tag THE Remote exchange context SHALL end the refusal text with `Tags received before the failure: <names>.`
- **Business rules:** `<names>` are the added tags only, sorted by name, separated by `, `; the sentence follows the usual refusal text after one space; the failure code is unchanged.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-002 [EARS-O]: a long list is shortened
> IF a failed pull delivered more than 20 tags THEN THE Remote exchange context SHALL name the first 20 by name order and end the sentence with ` and <N> more.`, where N is the number of tags not named.
- **Business rules:** exactly 20 tags are all named, with no `and <N> more`.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-003 [EARS-UB]: no sentence when nothing was delivered
> The Remote exchange context SHALL NOT add the sentence to a refusal when the pull delivered no tag, and SHALL NOT add it to the answer of a pull that succeeds or pauses on a conflict.
- **Scenario:** SC-001, SC-003. **Priority:** Must

## 9. Acceptance tests
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-003 | FR-NEW-003 | High |
| E2E-002 | New | failure | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | failure | SC-001 | FR-NEW-001 | High |
| E2E-004 | New | failure | SC-001 | FR-NEW-003 | High |
| E2E-005 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-006 | New | edge | SC-002 | FR-NEW-002 | High |
| E2E-007 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-008 | New | edge | SC-003 | FR-NEW-001, FR-NEW-003 | High |
| E2E-009 | New | side effect | SC-001 | FR-NEW-001 | Medium |
| E2E-010 | New | failure | SC-001 | FR-NEW-001 | Medium |
| E2E-011 | New | failure | SC-002 | FR-NEW-002 | Medium |
| E2E-012 | New | side effect | SC-002 | FR-NEW-002 | Medium |
| E2E-013 | New | failure | SC-003 | FR-NEW-003 | Medium |
| E2E-014 | New | side effect | SC-003 | FR-NEW-003 | Medium |
| E2E-015 | New | edge | SC-002 | FR-NEW-002 | Medium |

Coverage: happy 1, failure 6, side effect 3, edge 5, state transition 0 (none changes), security 0 (not a security change). Happy to failure ratio 1:6.

### 9.2 New tests
#### E2E-001: a pull that succeeds
- **Given** Alice and the remote tag `v1.0`. **When** she pulls and it succeeds. **Then** `tags_added` is `["v1.0"]` and no sentence appears anywhere in the answer.
#### E2E-002: quota refusal after the tags
- **Given** the remote delivers `v1.0` and `v2.0`; the project cannot take the merged files within its quota. **When** Alice pulls. **Then** the refusal is its usual text followed by ` Tags received before the failure: v1.0, v2.0.` **And** both tags are listed in the project.
#### E2E-003: merge error after the tags
- **Given** the remote delivers `v1.0` and a merge that fails with an error (not a conflict). **When** Alice pulls. **Then** the refusal ends ` Tags received before the failure: v1.0.`
#### E2E-004: the remote is down
- **Given** a remote that does not exist. **When** Alice pulls. **Then** the refusal is the connection failure text with no sentence.
#### E2E-005: no tag delivered, merge fails
- **Given** the remote delivers no tag and the merge fails. **When** Alice pulls. **Then** the refusal has no sentence.
#### E2E-006: 25 tags
- **Given** the remote delivers `t01` to `t25` and the merge fails. **When** Alice pulls. **Then** the refusal ends with the sentence of SC-002 naming `t01` to `t20` and ` and 5 more.`
#### E2E-007: exactly 20 tags
- **Given** the remote delivers `t01` to `t20`. **When** Alice pulls and the merge fails. **Then** the sentence names all 20 and has no `and`.
#### E2E-008: retry reports nothing new
- **Given** E2E-002 and the cause removed. **When** Alice pulls again. **Then** it succeeds, `tags_added` is `[]`, `tags_kept_local` is `[]`, and both tags are still listed.
#### E2E-009: nothing else changes
- **Side effect.** **Given** E2E-002. **Then** the remote is unchanged, and the project's tags are exactly `v1.0` and `v2.0`.
#### E2E-010: a deadline after the tags
- **Given** the remote delivers `v1.0`, and the merge step does not finish before the time allowed for a remote operation. **When** Alice pulls. **Then** the refusal is the usual deadline text; it ends with ` Tags received before the failure: v1.0.` when the tag was delivered before the deadline, and has no sentence otherwise.
#### E2E-011: the shortened list is sorted
- **Given** the remote delivers `t25` and `t01` to `t24` in that order and the merge fails. **Then** the sentence names `t01` to `t20`.
#### E2E-012: a shortened list leaves the others delivered
- **Side effect.** **Given** E2E-006. **Then** all 25 tags are listed in the project.
#### E2E-013: a pause on conflict is not a failure
- **Given** the remote delivers `v1.0` and the merge conflicts. **When** Alice pulls. **Then** the answer reports `v1.0` as added and the conflict, with no sentence.
#### E2E-014: no duplicate sentence on a second failure
- **Side effect.** **Given** E2E-002 and the merge fails again on a retry. **When** Alice pulls. **Then** the refusal has no sentence, since no tag is added this time.
#### E2E-015: boundary of 21
- **Given** the remote delivers `t01` to `t21` and the merge fails. **Then** the sentence names `t01` to `t20` and ends ` and 1 more.`

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-003 | E2E-013 (pause is not failure) | E2E-002, 003, 004, 010 | E2E-005 |
| SC-002 | FR-NEW-002 | E2E-001 (a pull that delivers no list needs no shortening) | E2E-011 | E2E-006, 007, 015 |
| SC-003 | FR-NEW-001, FR-NEW-003 | E2E-001 | E2E-013, 014 | E2E-008 |

Side effects: SC-001 E2E-009; SC-002 E2E-012; SC-003 E2E-014. Requirement coverage: FR-NEW-001 has E2E-002, 003, 008, 009, 010 (5); FR-NEW-002 has E2E-006, 007, 011, 012, 015 (5); FR-NEW-003 has E2E-001, 004, 005, 008, 013, 014 (6).

## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Delivered tag | a remote tag a pull has brought in and recorded | every scenario |
| Kept local | a tag that differs from the remote's of the same name and stays as it is | out of scope |
| Paused on a conflict | a pull that stops at a merge conflict and reports it as a result, not a failure | E2E-013 |

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
| Tags are recorded after the import and before the merge decision | `crates/core/src/tools/git.rs:6818-6823` |
| The tag lists are read only after the call returns without error | `crates/core/src/tools/git.rs:6995-6998` (`.await?` before `mem::take`) |
| A successful pull returns both lists | `crates/core/src/tools/git.rs:7021` |
| An error carries a code and a text | `crates/core/src/errors.rs:30-40` |
| A retry sees the tags as identical and reports nothing | `crates/core/src/tools/git_tags.rs:451-455` (`tags_identical`, neither list) |
| The paused-conflict rule | `sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:201` |
Test command: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`

### 14.3 Design inputs
- The error leaves the thread closure through `.await?` before the tag slot is read (`git.rs:6995`); the slot must be read on the error path too (signal: E2E-002 fails with no sentence).
- On a deadline the worker thread may still be running when the call returns (`remote.rs:1598`, `with_remote_deadline`); whether the slot is set then depends on timing, hence the "when delivered" wording of E2E-010 (signal: the test is timing dependent; a seam like `after_listing` in `git_tags.rs` can make it deterministic).
- The 20 name cap is a user visible number and belongs to the product, not to the design.
