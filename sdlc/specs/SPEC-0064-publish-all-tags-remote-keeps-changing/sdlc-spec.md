# Publish all tags: the remote keeps changing during the send: functional specification

> Id: SPEC-0064
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: 1 context (remote exchange), 2 requirements, no persisted change.
> Status: designed
> Generated on: 2026-10-30
> Project: existing
> From backlog: BL-0093
> Depends on: SPEC-0054
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
When a member confirms "publish all tags" and the remote keeps changing under the send, the product already stops with a refusal instead of retrying forever. That refusal, its wording and the exact set of tags it names were never specified or tested. This increment specifies it, so a member (and whoever supports members) can rely on what they see and on what was and was not sent.

## 2. Current State (MANDATORY, every depth)
Empty repository: no. The product versions projects and exchanges them with remotes.

### 2.1 What users can do today
- A member previews, then confirms "publish all tags" by stating the number of tags to publish (SPEC-0054/FR-NEW-001, FR-NEW-002).
- If, between the listing the confirmation takes and the sending, the remote gains one of the tags to publish, the product lists the remote again, reports each such tag as `up to date` or `differs`, and sends the other tags (SPEC-0054/FR-NEW-020).
- The sending is all or nothing: when the remote has gained any of the tags being sent, nothing at all is sent.
- If, after listing the remote again, a tag the remote just turned down is still a tag to publish (the remote created and removed it again), the product stops and refuses with `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.`, naming every tag still to publish at that moment, in the order of the preview. Nothing is sent and no audit entry is recorded.
- After a re-listing that removed at least one tag from the tags to publish, the product sends again; after one that removed none, it refuses.
- No requirement of SPEC-0054 words this refusal, and no test exercises it.

### 2.2 Specs governing this area
SPEC-0054/FR-NEW-020 (a remote tag that changed after the listing: not replaced, reported, the others still sent), FR-NEW-009 (audit trail: one entry per tag sent, none for any refusal), FR-NEW-015 (order of refusals), FR-NEW-003 (wrong number). This increment adds the missing case and modifies none of them. SPEC-0054 is not edited.

### 2.3 What is verified today
The reclassification after a lost send is verified for tags that turn out `up to date` or `differs`. The case where a tag stays to publish after the re-listing has no test, because today no test can make a remote tag appear and vanish after a lost send.

## 3. Scope
### 3.1 In scope
- A requirement that words the refusal, the set of tags it names, and what was and was not sent.
- A requirement that fixes when the product gives up and when it sends again.
- The acceptance tests for both.

### 3.2 Out of scope, and why
- Retrying the send once more before refusing: not chosen. It changes when a member sees the refusal and still needs the same wording for the second failure. The refusal already tells the member exactly what to do.
- Any change to the report format of SPEC-0054.

### 3.3 Constraints imposed by the context
None.

## 5. Usage scenarios
### SC-001: The remote keeps changing during the send
**Actor:** Member (Alice).
**Preconditions:** local tags `v1.0` and `v2.0`, neither on the remote; the remote holds the commits of both.
**Flow:**
1. Alice previews → the product lists `v1.0 to publish`, `v2.0 to publish`, then `Tags to publish: 2. Confirm by stating 2 to send them.`
2. Alice confirms stating 2, and while the product sends, another source creates `v1.0` on the remote and removes it again before the product looks → the product refuses with `Remote tags kept changing while publishing: v1.0, v2.0. Review the list, then confirm again.`
3. Alice previews again → the product lists the real state of the remote.
**Postconditions:** the remote holds neither tag from this action; the audit trail has no new entry.
**Exceptions:**
- EXC-001a: the remote stops changing before Alice confirms again → the confirmation publishes as in SPEC-0054.
- EXC-001b: Alice states a wrong number in step 2 → the wrong number refusal comes first and nothing is sent (SPEC-0054/FR-NEW-003).

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-O]: The remote keeps changing during the send
> IF, during a confirmed send, the remote lost the product's attempt because a tag to send changed, and after the product listed the remote again a tag to send is still a tag to publish, THEN THE Remote exchange context SHALL send nothing, report no tag as sent, and refuse with `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.`
- **Business rules:** `<names>` are the tags still to publish at that moment, not only the tag that changed, in the order of the preview (byte by byte by name), separated by a comma and a space; a tag that the re-listing showed `up to date` or `differs` is not named and its outcome is not reported (the refusal replaces the report); no audit entry is recorded (SPEC-0054/FR-NEW-009: none for any refusal); the remote receives nothing from this action; the refusal comes after the number check of SPEC-0054/FR-NEW-003 and FR-NEW-019, so a wrong number is refused first and nothing is sent; a new preview afterwards shows the real state of the remote.
- **User visible text:** `Remote tags kept changing while publishing: v1.0, v2.0. Review the list, then confirm again.`
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-002 [EARS-UB]: No endless retry
> The Remote exchange context SHALL NOT send again after a re-listing that removed no tag from the tags to publish, and SHALL send again after a re-listing that removed at least one.
- **Business rules:** a tag leaves the tags to publish when the re-listing shows it `up to date` or `differs` (SPEC-0054/FR-NEW-020); the confirmed send therefore always ends, with the report of SPEC-0054/FR-NEW-002 or with the refusal of FR-NEW-001; a send that ended in the report of FR-NEW-002 after one or more re-listings sends each tag at most once.
- **Scenario:** SC-001. **Priority:** Must

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | failure | SC-001 | FR-NEW-001 | Critical |
| E2E-002 | New | failure | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-004 | New | recovery | SC-001 | FR-NEW-001 | High |
| E2E-005 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-006 | New | failure | SC-001 | FR-NEW-002 | High |
| E2E-007 | New | happy | SC-001 | FR-NEW-002 | High |
| E2E-008 | New | failure | SC-001 | FR-NEW-001 | Medium |

Coverage: happy 1, recovery 1, failure 4, edge 2, side effect 0 (the no audit entry and nothing on the remote assertions are inside E2E-001 to 003), state transition 0, security 0. Happy and recovery to failure and edge ratio 1:3 (2 against 6).

### 9.2 New tests
#### E2E-001: One tag keeps changing
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0`, not on the remote, whose commit the remote holds; Alice previewed `Tags to publish: 1`. **When** Alice confirms stating 1 and, each time the product is about to send, another source creates `v1.0` on the remote and removes it again before the product looks. **Then** the refusal is exactly `Remote tags kept changing while publishing: v1.0. Review the list, then confirm again.` **And** the remote holds no `v1.0` from this action and the audit trail has no new entry.
- **Priority:** Critical

#### E2E-002: A stable tag is named and not sent
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0` and `v2.0`, neither on the remote; Alice previewed `Tags to publish: 2`; the remote changes `v1.0` as in E2E-001 and never touches `v2.0`. **When** Alice confirms stating 2. **Then** the refusal is exactly `Remote tags kept changing while publishing: v1.0, v2.0. Review the list, then confirm again.` **And** `v2.0` is not on the remote and the audit trail has no new entry.
- **Priority:** Critical

#### E2E-003: A tag that became identical is not named
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** local `a1`, `b1`, `c1`, none on the remote; Alice previewed `Tags to publish: 3`; before the first send another source creates `a1` on the remote identical to the local tag, and the remote changes `b1` as in E2E-001 on every send; `c1` is never touched. **When** Alice confirms stating 3. **Then** the refusal is exactly `Remote tags kept changing while publishing: b1, c1. Review the list, then confirm again.` **And** the remote holds `a1` (from the other source) and neither `b1` nor `c1`, and the audit trail has no new entry.
- **Priority:** High

#### E2E-004: Retry after the refusal
- **Category:** recovery. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the refusal of E2E-001 has just been given and the remote has stopped changing. **When** Alice previews, then confirms stating the number the preview gives. **Then** the preview says `v1.0 to publish` and `Tags to publish: 1. Confirm by stating 1 to send them.`, and the confirmation reports exactly `v1.0 published`. **And** the remote holds `v1.0` and the audit trail has one new entry (`v1.0`).
- **Priority:** High

#### E2E-005: Names in byte order
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `alpha` and `Zeta`, neither on the remote, both to publish; the remote changes `alpha` as in E2E-001. **When** Alice confirms stating 2. **Then** the refusal is exactly `Remote tags kept changing while publishing: Zeta, alpha. Review the list, then confirm again.` (capital `Z` sorts before lower case `a`). **And** nothing was sent.
- **Priority:** Medium

#### E2E-006: One attempt, then the refusal
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** local `v1.0`, not on the remote; the remote changes `v1.0` as in E2E-001 on every send. **When** Alice confirms stating 1. **Then** the product made exactly one sending attempt and listed the remote exactly twice (once for the confirmation, once after the lost attempt), and the answer is the refusal of E2E-001.
- **Priority:** High

#### E2E-007: A re-listing that removes a tag allows another send
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** local `a1` and `b1`, neither on the remote; Alice previewed `Tags to publish: 2`; before the first send another source creates `a1` on the remote identical to the local tag. **When** Alice confirms stating 2. **Then** the report is exactly `a1 up to date`, `b1 published`. **And** the product made two sending attempts, the first lost and the second accepted, `b1` is on the remote once, and the audit trail has one new entry (`b1`).
- **Priority:** High

#### E2E-008: A wrong number comes first
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0`, not on the remote, and a remote that changes `v1.0` as in E2E-001. **When** Alice confirms stating 5. **Then** the refusal is the wrong number refusal of SPEC-0054/FR-NEW-003, not the one of FR-NEW-001. **And** the product made no sending attempt and the audit trail has no new entry.
- **Priority:** Medium

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002 | E2E-004, E2E-007 | E2E-001, E2E-002, E2E-006, E2E-008 | E2E-003, E2E-005 |

FR-NEW-001: E2E-001, 002, 003, 004, 005, 008. FR-NEW-002: E2E-003, 006, 007. Every requirement has at least three tests.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Tag | a name attached to a commit; lightweight or annotated | Versioning |
| To publish | outcome of a tag the remote lacks and whose commit it holds | Remote exchange |
| Up to date | outcome of a tag the remote holds identically | Remote exchange |
| Differs | outcome of a tag the remote holds in another form | Remote exchange |
| Preview | the list of every local tag with its outcome, sent to nobody | Remote exchange |
| Sending attempt | one try to send the tags to publish together; all or nothing | Remote exchange |
| Re-listing | the product asks the remote for its tags again after a lost sending attempt | Remote exchange |
| Remote | a repository the project exchanges with | Remote exchange |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist) |

**Amendments applied:** none. Checklist run on the file as written: no forced choice (names order, separator, text, when to stop all spelled); every message verbatim; no technology, path, function or code name in Sections 1 to 13 (checked with rg); every test Given/When/Then with concrete tags; FR-NEW-001 has 6 tests, FR-NEW-002 has 3; failure and edge tests (6) outnumber happy and recovery (2); EARS patterns matched, no forbidden modal, no nested condition; right nature (FEAT: an operator-visible refusal was never defined); every code claim cited with a line and recounted; one capability the harness lacks recorded as a design input with its signal; length within the depth S budget of 80 to 200 lines (the file is slightly above because of the 8 tests).
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)
### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| The refusal exists with this text and names the tags still to publish | `crates/core/src/tools/git_tags.rs:1217-1222` (`Err(ToolError::no_clobber(format!("Remote tags kept changing while publishing: {}. Review the list, then confirm again.", names.join(", "))))`, `names` built from `due` at `:1217`) |
| The tags named are all those still to publish, in preview order | `due` holds indexes into `tags`, built in order at `git_tags.rs:1160-1176`; `names` at `:1217` |
| The send is all or nothing | `crates/core/src/git/remote.rs:826-836` (every refspec is a creation leased on "absent"; a lost lease aborts the whole push before anything is sent) |
| After a re-listing that removed a tag, the send goes again; after one that removed none, it stops | `git_tags.rs:1178` (`for _ in 0..due.len()`), `:1205-1209` (`let before = due.len(); reclassify_lost(..); if due.len() == before { break; }`) |
| A tag showing `up to date` or `differs` after the re-listing leaves the tags to publish | `git_tags.rs:1227-1261` (`reclassify_lost`, `kept` at the end replaces `due`) |
| No audit entry and nothing reported on this refusal | the refusal returns from `send_due_tags` (`git_tags.rs:1217-1222`) after the only place an outcome is marked `Published` (`:1199-1203`, reached only when no lease was lost), and the audit lines are written only for `Outcome::Published` tags (`:911`) |
| SPEC-0054 words only the `up to date` or `differs` case | `sdlc/specs/SPEC-0054-publish-all-tags/sdlc-spec.md:190-195` (FR-NEW-020) |
| No test covers the refusal | `rg -n 'kept changing' crates/core/src` matches only `git_tags.rs:1219` (recount at implementation time) |
Test command: `cargo test -p mcp-fs-core --lib e2e_tag`

### 14.3 Design inputs
- A capability the test harness lacks: no test can make a remote tag appear and vanish after a lost sending attempt, because a file remote cannot (BL-0093). The existing `bulk_seams` test seam (`crates/core/src/git/remote.rs:946-974`, `take_drop`, `take_rejections`) is the pattern: a seam that makes the push report a lost lease for named tags a given number of times, and one that counts sending attempts and listings (E2E-006, E2E-007). Signal: E2E-001 and E2E-006 cannot be written without it (compile error or no way to reach the branch).
- The refusal today carries the "no overwrite" error code (`ToolError::no_clobber`, `crates/core/src/errors.rs:74`); a design must keep it, because members and clients may already match on it.
- The loop bound is the number of tags to publish and the no progress break; any change to either changes FR-NEW-002.
