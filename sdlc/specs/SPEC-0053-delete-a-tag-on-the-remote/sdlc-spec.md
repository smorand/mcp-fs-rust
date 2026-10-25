# Delete a tag on the remote: functional specification

> Id: SPEC-0053
> Nature: FEAT
> Depth: M
> Depth evidence: 2 areas (remote exchange, audit), 15 requirements, no persisted change
> Status: designed
> Generated on: 2026-10-24
> Project: existing
> From backlog: BL-0075
> Depends on: SPEC-0052
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A project member can withdraw one tag from the remote, for example a tag published by mistake. The action is destructive, so the member must state the commit the remote tag points at now; a wrong or missing commit is refused with a message that shows the real one. The local tag is never touched, nothing else on the remote is touched, and every real deletion leaves one audit line. Today a published tag can only be replaced (SPEC-0052), never removed.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today
- A member can publish one named tag to the remote, and replace a differing remote tag only by stating the commit it points at now.
- A member can delete a tag locally; that never touches the remote.
- No action removes a tag on the remote.
- Every project action by a non member is refused, and a paused merge, rebase, cherry pick or revert does not block publishing a tag.
- Publishing refuses with `Cannot publish tag '<n>': this project tracks no remote.` when the project tracks no remote, and gives one refusal per cause for a remote that is down, rate limited, whose credential expired or whose connection is lost.

### 2.2 Specs governing this area
- SPEC-0052/FR-NEW-005: a local deletion leaves the remote untouched. **Not changed:** the new action is separate, and the local deletion keeps that rule.
- SPEC-0052/FR-NEW-008 and FR-NEW-018: replacing a remote tag needs the commit it points at now, stated as a full id or at least 7 hexadecimal characters. This spec reuses that confirmation rule and its refusal wording for a commit that differs.
- SPEC-0052/FR-NEW-012: audit lines. This spec adds one line shape.
- SPEC-0052 section 3.2 left remote deletion out of scope on purpose (BL-0075); that is the only deviation. No existing spec is modified.

### 2.3 What is verified today
Publishing, replacing and local deletion are exercised by existing tests; nothing exercises a remote deletion because none exists. Details in 14.1.

## 3. Scope
### 3.1 In scope
- Delete one named tag on a named remote (default `origin`), confirmed by the commit it points at now.
- Refusals for: not a member, no remote, remote not tracked, remote or connection failure, tag absent, commit not stated, commit differs, remote declines.
- One audit line per real deletion.

### 3.2 Out of scope, and why
- Deleting several tags in one action: each deletion deserves its own confirmation.
- Deleting a remote branch: not a tag.
- Deleting the local tag together with the remote one: the two stay independent (SPEC-0052/FR-NEW-005).
- A dry run or an undo: re publishing the tag restores it, which SPEC-0052 already offers.

### 3.3 Constraints imposed by the context
- The remote can protect tags and decline a deletion; the product relays its reason. Imposed by the hosting providers.

## 4. Actors
- **Member**: belongs to the project; deletes remote tags.
- **Non member**: refused.
- **Remote**: the hosted repository the project tracks.
- **Operator**: reads the audit trail.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| Remote exchange | deleting a tag on the remote, its confirmation and refusals | remote tag, confirmation commit, moved, declined |
| Audit | the trail of who deleted what | audit line |

## 5. Usage scenarios

### SC-001: Delete a tag on the remote
**Actor:** member.
**Preconditions:** the project tracks a remote; the member holds a valid credential for it; the remote has the tag.
**Flow:**
1. The member names the tag and states the commit it points at now → the tag disappears from the remote and only that tag.
2. The result tells the member the tag, the remote and the full id of the commit the tag had.
**Postconditions:** the remote has no such tag; every other remote tag, every branch, every local tag is as before; one audit line exists.
**Exceptions:**
- EXC-001a: the requester is not a member → `'carol@example.com' is not a member of 'proj-a'`.
- EXC-001b: the project tracks no remote at all → `Cannot delete tag 'v1.0': this project tracks no remote.`; it tracks others but not the named one → `Cannot delete tag 'v1.0': this project does not track a remote named 'upstream'.`
- EXC-001c: the remote is down, rate limits, the credential expired or is missing, the remote does not answer in time, or the connection is lost → the refusal of FR-NEW-012; nothing changes and nothing is audited, even when the remote had already applied the deletion (FR-NEW-013).
- EXC-001d: the remote has no such tag → `Remote tag 'v9.9' does not exist.`
- EXC-001e: no commit stated → `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`
- EXC-001f: the stated commit is not the remote tag's commit (or the tag moved meanwhile) → `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`
- EXC-001g: the remote declines → `Remote refused to delete tag 'v1.0': <the remote's reason>`
- EXC-001h: two members delete the same tag at once → one succeeds, the other gets EXC-001d, or EXC-001f when the tag was moved instead.
**Interactions:** SPEC-0052 scenarios SC-003 (local deletion) and SC-004 (publishing).

## 6. Functional requirements

### 6.1 New
#### FR-NEW-001 [EARS-E]: Delete the confirmed remote tag
> WHEN a member deletes a named tag on the remote and states the commit the remote tag points at now, THE Remote exchange context SHALL remove that tag from the remote and only that tag.

- **Business rules:** the stated commit is the full commit id or at least 7 hexadecimal characters of it, upper or lower case, surrounding spaces ignored; every local tag, every branch, every commit and every other remote tag stay untouched; the member can delete a remote tag that has no local copy; an annotated tag is confirmed by the commit it points at, not by its own identifier; the remote is the one named by the member, `origin` when none is named.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-002 [EARS-O]: Commit not stated
> IF the member states no commit, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.

- **User visible text:** `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.` (the commit shown is the remote tag's, 7 characters)
- **Business rules:** an empty text, or a text of only spaces, counts as not stated.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-003 [EARS-O]: Commit differs or tag moved
> IF the stated commit is not the commit the remote tag points at now, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.

- **User visible text:** `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` (the commit shown is the remote tag's, 7 characters)
- **Business rules:** a stated text shorter than 7 characters, or containing a character that is not hexadecimal, never matches; the same refusal applies when the remote tag moved between the moment it was looked at and the moment it was to be deleted, so a stale confirmation never deletes a tag that now points elsewhere; when it moved and was then deleted so that no commit can be shown, the refusal is the FR-NEW-004 one.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-004 [EARS-O]: Remote tag absent
> IF the remote has no tag of that name, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v9.9' does not exist.`

- **Business rules:** a tag that exists only locally counts as absent on the remote; a name that is empty, only spaces, or not a valid tag name cannot exist on the remote and counts as absent, with the name shown as given.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-005 [EARS-O]: No remote
> IF the project tracks no remote at all, THEN THE Remote exchange context SHALL refuse with `Cannot delete tag 'v1.0': this project tracks no remote.`

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-006 [EARS-O]: The remote declines
> IF the remote declines the deletion, THEN THE Remote exchange context SHALL refuse with `Remote refused to delete tag 'v1.0': <the remote's reason>` and leave the remote tag in place.

- **Business rules:** the reason is relayed as the remote gave it; the audit trail is untouched.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-007 [EARS-E]: Order of refusals
> WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list: not a member (FR-NEW-009); the project tracks no remote at all (FR-NEW-005); the named remote is not tracked (FR-NEW-014); the remote or the connection fails (FR-NEW-012, FR-NEW-013); the remote tag is absent (FR-NEW-004); no commit is stated (FR-NEW-002); the stated commit differs (FR-NEW-003); the remote declines (FR-NEW-006).

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-008 [EARS-E]: Audit line
> WHEN a remote tag is really deleted, THE Audit context SHALL record the member and one line of text.

- **User visible text:** `deleted tag 'v1.0' on remote 'origin' (was at <full commit id>)`
- **Business rules:** the line shows the full commit id even when 7 characters were stated; a refusal, a non member attempt and a connection failure leave no entry.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-009 [EARS-O]: Non member
> IF the requester is not a member of the project, THEN THE system SHALL refuse with `'carol@example.com' is not a member of 'proj-a'` (the requester's identifier and the project name), and reveal no tag name, commit or sign that the tag exists.

- **Business rules:** this is the refusal the product gives a non member for any other project action.

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-010 [EARS-S]: Paused operation
> WHILE a merge, rebase, cherry pick or revert is paused, THE Remote exchange context SHALL still delete a confirmed remote tag, and leave the paused operation as it was.

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-011 [EARS-E]: Concurrent deletion
> WHEN two members delete the same remote tag at the same time, THE Remote exchange context SHALL let exactly one succeed and refuse the other.

- **Business rules:** the refused member gets the FR-NEW-004 refusal when the winner's deletion is visible, and the FR-NEW-003 refusal when the tag was moved instead.

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-012 [EARS-O]: Remote or connection failure
> IF the remote is down, rate limits the request, does not answer in time, or the member's credential expired or is missing, THEN THE Remote exchange context SHALL refuse, leave the remote tag in place and record nothing.

- **User visible text:** remote down or rate limited: `fetch from host 'github.ibm.com' failed: <the reason the transport or the remote gave>`; credential expired: `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`; no credential stored: `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`; no answer in time: `remote timeout: host 'github.ibm.com' did not respond within <n>s`. These are the refusals publishing a tag gives, so one vocabulary covers every remote action.
- **Business rules:** the host name is the remote's.

- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-013 [EARS-O]: Answer lost after the request was sent
> IF the connection is lost after the deletion request was sent and before the remote's answer arrived, THEN THE Remote exchange context SHALL refuse with the connection refusal of FR-NEW-012 and record nothing, even when the remote did delete the tag.

- **Business rules:** the remote tag can already be gone; a retry reveals it with `Remote tag 'v1.0' does not exist.`
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-014 [EARS-O]: Named remote not tracked
> IF the project tracks at least one remote but not the one the member named, THEN THE Remote exchange context SHALL refuse with `Cannot delete tag 'v1.0': this project does not track a remote named 'upstream'.`

- **Business rules:** remote names are case sensitive; the name shown is the one the member gave; when the member names no remote the name `origin` is used and shown, so a project tracking only `upstream` gets the refusal for `origin`; a remote name that is empty or only spaces counts as given and is refused with the name shown as given.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-015 [EARS-E]: Result of a deletion
> WHEN a remote tag is really deleted, THE Remote exchange context SHALL tell the member the tag name as given, the remote name used and the full 40 character id of the commit the tag had.

- **Business rules:** the remote name is the one the member named, `origin` when none was named; seven stated characters still yield the full id; the exact layout of the answer is the designer's, its content is fixed here.
- **Scenario:** SC-001. **Priority:** Must

## 7. Non functional requirements (user observable)
### 7.1 Performance
No new time target; deleting one tag among 5000 on the remote completes and leaves 4999 (E2E-037). Design measures the baseline.
### 7.2 Security and access
FR-NEW-009 is the access rule; FR-NEW-001's confirmation and FR-NEW-003 are the protection against a stale or careless deletion.
### 7.3 Privacy and compliance
Not applicable: no new personal data class.
### 7.4 Usability and accessibility
Not applicable: no screen is added.
### 7.5 Reliability
FR-NEW-011 to FR-NEW-013: a refused deletion leaves the trail empty; when the answer of a deletion is lost the trail stays empty even if the remote applied it, and a retry reveals the real state; a retry never produces two entries for one deletion.
### 7.6 Operability
FR-NEW-008 is the operator's view: who deleted which tag on which remote, and which commit it had.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | no commit stated; no remote; tag absent; project with no tag locally | the refusals of FR-NEW-002, 005, 004; a remote tag without local copy deletes | E2E-010, 011, 012, 014, 017, 036 |
| Boundary values | stated 6 / 7 / 40 characters, uppercase, spaces; 5000 tags | 6 never matches; the rest match | E2E-002, 003, 004, 006, 037 |
| Concurrent actions | two members delete at once; move versus delete; retry after success | one wins; never delete a moved tag | E2E-030, 031, 040 |
| Slow or lost connectivity | connection lost before or after the remote applied the deletion | refusal, nothing audited | E2E-033, 042, 043 |
| Access states | non member | the usual refusal, nothing revealed | E2E-022, 023, 024 |
| Data variations | slash and accent names; annotated tag | handled; annotated confirmed by its commit | E2E-027, 028 |
| Navigation | not applicable: no screen or link | | |
| Partial operations | not applicable: one tag is one atomic deletion; the lost connection case is E2E-033 | | |
| Dependency failures | remote down, rate limited, credential expired, remote declines | the refusals of FR-NEW-012 and FR-NEW-006 | E2E-019, 020, 032 |
| Ordering and timing | tag moved after viewing; order of refusals; paused operation | FR-NEW-003, 007, 010 | E2E-009, 015, 018, 021, 024, 029, 034, 038, 039 |
| Exhaustion | thousands of remote tags | one removed | E2E-037 |

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Users: Alice and Bob are members of `proj-a`; Carol is not. The remote is a GitHub Enterprise repository tracked by `proj-a` under the name `origin`. Unless stated, the remote tag `v1.0` points at the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5` (shown as `f6a7b8c`) and the local tag `v1.0` points at `c3d4e5f`; the checked out commit is `a1b2c3d`.

### 9.1 Summary
Coverage by primary category (recounted from the table): journey happy 1, failure 11, security 3 (all refusals), edge 27, state transition 6, side effect 3, cross scenario 1; total 52. By outcome: 11 tests end only in a deletion, 35 end only in refusals, 5 mix both (E2E-027, E2E-028, E2E-030, E2E-033, E2E-037) and E2E-031 allows either; the happy to failure ratio stays above 1:3, which beats 1:1.

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey happy | SC-001 | FR-NEW-001, FR-NEW-015 | Critical |
| E2E-002 | New | edge | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-004 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-005 | New | side effect | SC-001 | FR-NEW-001 | Critical |
| E2E-006 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-007 | New | failure | SC-001 | FR-NEW-003 | Critical |
| E2E-008 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-009 | New | state transition | SC-001 | FR-NEW-003 | Critical |
| E2E-010 | New | failure | SC-001 | FR-NEW-002 | Critical |
| E2E-011 | New | edge | SC-001 | FR-NEW-002 | High |
| E2E-012 | New | edge | SC-001 | FR-NEW-002 | High |
| E2E-013 | New | side effect | SC-001 | FR-NEW-002 | Medium |
| E2E-014 | New | failure | SC-001 | FR-NEW-004 | Critical |
| E2E-015 | New | edge | SC-001 | FR-NEW-004, FR-NEW-007 | High |
| E2E-016 | New | edge | SC-001 | FR-NEW-004 | High |
| E2E-017 | New | failure | SC-001 | FR-NEW-005 | Critical |
| E2E-018 | New | edge | SC-001 | FR-NEW-005, FR-NEW-007 | Medium |
| E2E-019 | New | failure | SC-001 | FR-NEW-006 | Critical |
| E2E-020 | New | edge | SC-001 | FR-NEW-006 | Medium |
| E2E-021 | New | edge | SC-001 | FR-NEW-006, FR-NEW-007 | Medium |
| E2E-022 | New | security | SC-001 | FR-NEW-009, FR-NEW-007 | Critical |
| E2E-023 | New | security | SC-001 | FR-NEW-009 | High |
| E2E-024 | New | security | SC-001 | FR-NEW-007, FR-NEW-009 | Medium |
| E2E-025 | New | side effect | SC-001 | FR-NEW-008 | Critical |
| E2E-026 | New | failure | SC-001 | FR-NEW-008 | High |
| E2E-027 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-008 | High |
| E2E-028 | New | edge | SC-001 | FR-NEW-001, FR-NEW-004 | Medium |
| E2E-029 | New | state transition | SC-001 | FR-NEW-010, FR-NEW-001 | High |
| E2E-030 | New | state transition | SC-001 | FR-NEW-011, FR-NEW-004 | Critical |
| E2E-031 | New | state transition | SC-001 | FR-NEW-011, FR-NEW-003 | High |
| E2E-032 | New | failure | SC-001 | FR-NEW-012 | Critical |
| E2E-033 | New | failure | SC-001 | FR-NEW-012, FR-NEW-013 | High |
| E2E-034 | New | edge | SC-001 | FR-NEW-007, FR-NEW-012 | Medium |
| E2E-035 | New | cross scenario | SC-001 | FR-NEW-001, FR-NEW-004 | High |
| E2E-036 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-037 | New | edge | SC-001 | FR-NEW-001, FR-NEW-004 | Low |
| E2E-038 | New | state transition | SC-001 | FR-NEW-010, FR-NEW-004 | Medium |
| E2E-039 | New | state transition | SC-001 | FR-NEW-010, FR-NEW-003 | Medium |
| E2E-040 | New | edge | SC-001 | FR-NEW-011 | High |
| E2E-041 | New | edge | SC-001 | FR-NEW-005 | Medium |
| E2E-042 | New | failure | SC-001 | FR-NEW-013 | High |
| E2E-043 | New | edge | SC-001 | FR-NEW-013 | Medium |
| E2E-044 | New | failure | SC-001 | FR-NEW-014 | High |
| E2E-045 | New | edge | SC-001 | FR-NEW-014 | Medium |
| E2E-046 | New | edge | SC-001 | FR-NEW-014, FR-NEW-007 | Medium |
| E2E-047 | New | edge | SC-001 | FR-NEW-015 | Medium |
| E2E-048 | New | edge | SC-001 | FR-NEW-015 | Medium |
| E2E-049 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-050 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-051 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-052 | New | failure | SC-001 | FR-NEW-014 | High |

### 9.2 New tests
#### E2E-001: Delete with the full commit id
- **Category:** journey happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-015
- **Given** Alice is a member of `proj-a`; the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`; the remote also has tags `v1.1` and `release-2026`. **When** Alice deletes the remote tag `v1.0` and states `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** the remote tags `v1.1` and `release-2026` still exist, the local tag `v1.0` still points at `c3d4e5f`, and Alice is told the tag `v1.0`, the remote `origin` and the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`.
- **Priority:** Critical

#### E2E-002: Seven character abbreviation
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Bob, a member, deletes the remote tag `v1.0` and states `f6a7b8c`. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** every branch and commit of the remote is unchanged.
- **Priority:** Critical

#### E2E-003: Uppercase id with surrounding spaces
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Alice states `  F6A7B8C9D0E1F2A3B4C5D6E7F8A9B0C1D2E3F4A5  `. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** only `v1.0` is removed from the remote.
- **Priority:** High

#### E2E-004: Mixed case nine character abbreviation
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Alice states ` F6a7B8c9D `. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** the other remote tags are unchanged.
- **Priority:** Medium

#### E2E-005: Only the named tag is removed
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the remote has branches `main` and `dev` and tags `v1.0`, `v1.1`, `v2.0`; the local project has tags `v1.0` and `v1.1` and the commits `a1b2c3d` and `e5f6a7b`. **When** Alice deletes the remote tag `v1.0` stating `f6a7b8c`. **Then** the remote keeps `v1.1`, `v2.0`, `main` and `dev` at their previous commits. **And** the local tags `v1.0` and `v1.1` and all local commits are unchanged.
- **Priority:** Critical

#### E2E-006: Six characters never match
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice states `f6a7b8` (six characters, a true beginning of the commit). **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still exists and no audit entry was added.
- **Priority:** High

#### E2E-007: Stated commit is another commit
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the same state as E2E-006. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

#### E2E-008: Same first seven characters, different afterwards
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the same state as E2E-006. **When** Alice states `f6a7b8cffffffffffffffffffffffffffffffff0`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** `v1.0` remains on the remote.
- **Priority:** Medium

#### E2E-009: Tag moved after the member looked at it
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** Alice looked at the remote tag `v1.0` at `f6a7b8c`, then someone moved it to `e5f6a7b`. **When** Alice states `f6a7b8c`. **Then** the refusal is `Remote tag 'v1.0' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still points at `e5f6a7b` and no audit entry was added.
- **Priority:** Critical

#### E2E-010: No commit stated
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes the remote tag `v1.0` and states no commit. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

#### E2E-011: Empty text counts as none
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the same state as E2E-010. **When** Alice states an empty text. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched.
- **Priority:** High

#### E2E-012: Only spaces counts as none
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the same state as E2E-010. **When** Alice states five spaces. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched.
- **Priority:** High

#### E2E-013: The hint shows seven characters
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Bob states no commit. **Then** the refusal contains `f6a7b8c`. **And** the refusal does not contain the rest of the commit, `9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`.
- **Priority:** Medium

#### E2E-014: Remote tag does not exist
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has no tag `v9.9`. **When** Alice deletes the remote tag `v9.9` and states `f6a7b8c`. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** no audit entry was added.
- **Priority:** Critical

#### E2E-015: Absent tag outranks a missing commit
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, FR-NEW-007
- **Given** the remote has no tag `v9.9`. **When** Alice deletes `v9.9` and states no commit. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** the remote is untouched.
- **Priority:** High

#### E2E-016: Tag exists only locally
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** a local tag `release-2026` points at `e5f6a7b` and the remote has no `release-2026`. **When** Alice deletes the remote tag `release-2026` and states `e5f6a7b`. **Then** the refusal is `Remote tag 'release-2026' does not exist.`. **And** the local tag `release-2026` still exists.
- **Priority:** High

#### E2E-017: The project tracks no remote
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-005
- **Given** `proj-a` tracks no remote at all. **When** Alice deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project tracks no remote.`. **And** no audit entry was added.
- **Priority:** Critical

#### E2E-018: No remote outranks absent tag and missing commit
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-007
- **Given** `proj-a` tracks no remote. **When** Alice deletes `v9.9` and states no commit. **Then** the refusal is `Cannot delete tag 'v9.9': this project tracks no remote.`. **And** no audit entry was added.
- **Priority:** Medium

#### E2E-019: The remote declines a protected tag
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** the remote protects `v1.0` and answers `protected tag` to a deletion; `v1.0` points at `f6a7b8c`. **When** Alice deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `Remote refused to delete tag 'v1.0': protected tag`. **And** the remote tag `v1.0` still points at `f6a7b8c` and no audit entry was added.
- **Priority:** Critical

#### E2E-020: The remote's reason is relayed as given
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** the remote declines the deletion of `release-2026` with the reason `deletion of release tags is forbidden by policy`. **When** Bob deletes `release-2026` stating its correct commit. **Then** the refusal is `Remote refused to delete tag 'release-2026': deletion of release tags is forbidden by policy`. **And** the remote tag is unchanged.
- **Priority:** Medium

#### E2E-021: A wrong commit outranks the remote's protection
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006, FR-NEW-007
- **Given** `v1.0` is protected on the remote and points at `f6a7b8c`. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`, not the protection refusal. **And** the remote tag is unchanged.
- **Priority:** Medium

#### E2E-022: Non member is refused
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-009, FR-NEW-007
- **Given** Carol is not a member of `proj-a`. **When** Carol deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `'carol@example.com' is not a member of 'proj-a'`, with no tag name, commit or hint that the tag exists. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

#### E2E-023: Existence cannot be probed by a non member
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** Carol is not a member; the remote has `v1.0` but no `v9.9`. **When** Carol deletes `v9.9` with no commit, then `v1.0` with no commit. **Then** both refusals are exactly `'carol@example.com' is not a member of 'proj-a'`. **And** nothing about the remote is revealed.
- **Priority:** High

#### E2E-024: Non member outranks no remote
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-007, FR-NEW-009
- **Given** Carol is not a member and `proj-a` tracks no remote. **When** Carol deletes `v1.0`. **Then** the refusal is `'carol@example.com' is not a member of 'proj-a'`, not the no remote one. **And** no audit entry was added.
- **Priority:** Medium

#### E2E-025: Audit entry for a real deletion
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes `v1.0` stating `f6a7b8c`. **Then** exactly one audit entry exists, by Alice, reading `deleted tag 'v1.0' on remote 'origin' (was at f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5)`. **And** the entry shows the full id although seven characters were stated.
- **Priority:** Critical

#### E2E-026: No audit entry for any refusal
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** the audit trail holds some number of entries. **When** Alice tries once with no commit, once with a wrong commit, once on the absent tag `v9.9`, once on a protected tag; Carol tries once. **Then** each attempt is refused. **And** the audit trail holds the same number of entries as before.
- **Priority:** High

#### E2E-027: Annotated tag is confirmed by its commit
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-008
- **Given** the remote annotated tag `release-2026` has its own identifier `9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b` and points at the commit `e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4`. **When** Alice states no commit, then states `9a8b7c6`, then states `e5f6a7b`. **Then** the first attempt is refused with `Remote tag 'release-2026' points at e5f6a7b. State that commit to confirm its deletion.`, the second with `Remote tag 'release-2026' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`, and the third succeeds. **And** the audit line shows the full commit id `e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4`.
- **Priority:** High

#### E2E-028: Names with a slash and an accent
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** the remote tags `rel/2026` and `été-1` both point at `f6a7b8c`. **When** Alice deletes `rel/2026` stating `f6a7b8c`, then `été-1`, then `rel/2027`. **Then** the first two succeed and only those tags are removed; the third is refused with `Remote tag 'rel/2027' does not exist.`. **And** the other remote tags are unchanged.
- **Priority:** Medium

#### E2E-029: A paused operation does not block the deletion
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** a merge is paused in `proj-a`; then, in three further runs, a rebase, a cherry pick and a revert. **When** Alice deletes the remote tag `v1.0` stating `f6a7b8c` in each run. **Then** each deletion succeeds. **And** the paused operation is still paused afterwards, with its state unchanged.
- **Priority:** High

#### E2E-030: Two members delete the same remote tag
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** Alice and Bob both looked at the remote tag `v1.0` at `f6a7b8c`. **When** both delete it at the same moment stating `f6a7b8c`. **Then** exactly one succeeds and the other is refused with `Remote tag 'v1.0' does not exist.`. **And** exactly one audit entry exists.
- **Priority:** Critical

#### E2E-031: Stale view while another member moves the tag
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-011, FR-NEW-003
- **Given** Alice looked at `v1.0` at `f6a7b8c`; at the same moment Bob moves the remote tag to `e5f6a7b` by publishing with confirmation. **When** Alice states `f6a7b8c`. **Then** either Alice's deletion came first and succeeds, or she is refused with `Remote tag 'v1.0' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`. **And** the tag Bob placed at `e5f6a7b` is never deleted by Alice's stale confirmation; the trail holds one entry if Alice's deletion came first and none if she was refused.
- **Priority:** High

#### E2E-032: Remote down, rate limited, credential expired, no answer
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** the remote tag `v1.0` points at `f6a7b8c` on the host `github.ibm.com`. **When** Alice deletes `v1.0` stating `f6a7b8c` while the remote is unreachable, then while it rate limits, then with an expired credential, then with no stored credential, then while it does not answer in time. **Then** the refusals are, in turn, `fetch from host 'github.ibm.com' failed: <the transport's reason>`, `fetch from host 'github.ibm.com' failed: <the rate limit reason the remote gave>`, `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, and `remote timeout: host 'github.ibm.com' did not respond within <n>s`. **And** the remote tag `v1.0` stays and no audit entry was added.
- **Priority:** Critical

#### E2E-033: Connection lost before the deletion was applied
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012, FR-NEW-013
- **Given** the remote tag `v1.0` points at `f6a7b8c`. **When** the connection drops while Alice deletes `v1.0`, before the remote applied the request. **Then** Alice receives `fetch from host 'github.ibm.com' failed: <the transport's reason>` and the remote tag `v1.0` is still there. **And** no audit entry was added; a retry by Alice stating `f6a7b8c` succeeds and leaves exactly one entry.
- **Priority:** High

#### E2E-034: Connection failure outranks absent tag and missing commit
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** the remote is unreachable. **When** Alice deletes `v9.9` and states no commit. **Then** she receives the connection refusal, not the absent tag or missing commit refusal. **And** no audit entry was added.
- **Priority:** Medium

#### E2E-035: Publish, delete remotely, delete locally, publish again
- **Category:** cross scenario. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** Alice has the local tag `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0`, deletes it on the remote stating `a1b2c3d`, deletes it locally, then Bob creates a local `v1.0` at `e5f6a7b` and publishes it, and Alice deletes that one on the remote stating `e5f6a7b`. **Then** after the remote deletion the local `v1.0` is still there; after the local deletion the remote still has no `v1.0`; the republication succeeds; the last deletion succeeds. **And** each step leaves exactly its own audit entry.
- **Priority:** High

#### E2E-036: Remote tag without a local copy
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the remote tag `v2.0` points at `f6a7b8c` and `proj-a` has no local `v2.0`. **When** Bob deletes `v2.0` stating `f6a7b8c`. **Then** the deletion succeeds. **And** no local tag is created or changed.
- **Priority:** High

#### E2E-037: Many remote tags
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** the remote has 5000 tags `t0001` to `t5000`. **When** Alice deletes `t2500` stating its commit. **Then** exactly `t2500` is removed. **And** 4999 remote tags remain, and a second attempt on `t2500` is refused with `Remote tag 't2500' does not exist.`.
- **Priority:** Low

#### E2E-038: Paused operation and an unknown tag
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-004
- **Given** a merge is paused in `proj-a`; the remote has no `v9.9`. **When** Alice deletes `v9.9` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** the merge is still paused with its state unchanged.
- **Priority:** Medium

#### E2E-039: Paused operation and a wrong commit
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-003
- **Given** a merge is paused in `proj-a`; the remote tag `v1.0` points at `f6a7b8c`. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the merge is still paused and the remote tag is unchanged.
- **Priority:** Medium

#### E2E-040: A retry after a successful deletion
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-011
- **Given** Alice has just deleted the remote tag `v1.0`. **When** Bob, who looked at it earlier, deletes `v1.0` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v1.0' does not exist.`. **And** the trail holds exactly one entry for `v1.0`.
- **Priority:** High

#### E2E-041: No remote while a local tag exists
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005
- **Given** `proj-a` tracks no remote and has the local tag `v1.0`. **When** Alice deletes `v1.0` stating `a1b2c3d`. **Then** the refusal is `Cannot delete tag 'v1.0': this project tracks no remote.`. **And** the local tag `v1.0` still exists.
- **Priority:** Medium

#### E2E-042: Connection lost after the remote applied the deletion
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** the remote tag `v1.0` points at `f6a7b8c`. **When** Alice deletes `v1.0` stating `f6a7b8c`; the remote applies the deletion, then the connection drops before the answer reaches her. **Then** Alice receives `fetch from host 'github.ibm.com' failed: <the transport's reason>`. **And** no audit entry was added although the remote tag is gone; a retry by Alice is refused with `Remote tag 'v1.0' does not exist.`.
- **Priority:** High

#### E2E-043: Another member finds the deletion Alice was unable to confirm
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** the same state as E2E-042. **When** Bob deletes `v1.0` stating `f6a7b8c`. **Then** Bob is refused with `Remote tag 'v1.0' does not exist.`. **And** the trail holds no entry for `v1.0`.
- **Priority:** Medium

#### E2E-044: The project does not track the named remote
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v1.0` on the remote `upstream` stating `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'upstream'.`. **And** the remote `origin` is untouched and no audit entry was added.
- **Priority:** High

#### E2E-045: Remote names are case sensitive
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v1.0` on the remote `Origin` stating `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'Origin'.`. **And** the remote `origin` is untouched.
- **Priority:** Medium

#### E2E-046: A named remote that is not tracked outranks an absent tag and a missing commit
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014, FR-NEW-007
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v9.9` on the remote `upstream` stating no commit. **Then** the refusal is `Cannot delete tag 'v9.9': this project does not track a remote named 'upstream'.`. **And** no audit entry was added.
- **Priority:** Medium

#### E2E-047: The reported commit is the full id
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes `v1.0` stating only `f6a7b8c`. **Then** Alice is told the tag `v1.0`, the remote `origin` and the full commit id `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`, not the seven characters. **And** the audit line shows the same full id.
- **Priority:** Medium

#### E2E-048: The reported remote is the one named
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** `proj-a` tracks the remotes `origin` and `upstream`; `upstream` has `v1.0` at `f6a7b8c`. **When** Alice deletes `v1.0` on `upstream` stating `f6a7b8c`. **Then** Alice is told the tag `v1.0`, the remote `upstream` and the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`; the audit line reads `deleted tag 'v1.0' on remote 'upstream' (was at f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5)`. **And** the tag `v1.0` of `origin` is untouched.
- **Priority:** Medium

#### E2E-049: An empty or blank tag name counts as absent
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has tags `v1.0` and `v1.1`. **When** Alice deletes a tag named with an empty text, then a tag named with three spaces, stating `f6a7b8c`. **Then** the refusals are `Remote tag '' does not exist.` and `Remote tag '   ' does not exist.`. **And** both remote tags are untouched and no audit entry was added.
- **Priority:** Medium

#### E2E-050: An invalid tag name counts as absent
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has the tag `v1.0`. **When** Alice deletes the tag named `v1..0` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v1..0' does not exist.`. **And** the remote tag `v1.0` is untouched and no audit entry was added.
- **Priority:** Medium

#### E2E-051: A stated text that is not hexadecimal never matches
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice states `zzzzzzz`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still exists and no audit entry was added.
- **Priority:** Medium

#### E2E-052: No remote named, and origin is not tracked
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `upstream`. **When** Alice deletes `v1.0` naming no remote and states `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'origin'.`. **And** no audit entry was added.
- **Priority:** High

### 9.4 Modified tests
None.

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Happy | Failure | Edge | State, side effect, cross |
|---|---|---|---|---|
| SC-001 | E2E-001 | E2E-007, E2E-010, E2E-014, E2E-017, E2E-019, E2E-022, E2E-023, E2E-024, E2E-026, E2E-032, E2E-033, E2E-042, E2E-044, E2E-052 | E2E-002, E2E-003, E2E-004, E2E-006, E2E-008, E2E-011, E2E-012, E2E-015, E2E-016, E2E-018, E2E-020, E2E-021, E2E-027, E2E-028, E2E-034, E2E-036, E2E-037, E2E-040, E2E-041, E2E-043, E2E-045, E2E-046, E2E-047, E2E-048, E2E-049, E2E-050, E2E-051 | E2E-005, E2E-009, E2E-013, E2E-025, E2E-029, E2E-030, E2E-031, E2E-035, E2E-038, E2E-039 |

Requirement to tests (recounted; every requirement has at least 3):

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-027, E2E-028, E2E-029, E2E-035, E2E-036, E2E-037 (11) |
| FR-NEW-002 | E2E-010, E2E-011, E2E-012, E2E-013, E2E-027 (5) |
| FR-NEW-003 | E2E-006, E2E-007, E2E-008, E2E-009, E2E-031, E2E-039, E2E-051 (7) |
| FR-NEW-004 | E2E-014, E2E-015, E2E-016, E2E-028, E2E-030, E2E-035, E2E-037, E2E-038, E2E-049, E2E-050 (10) |
| FR-NEW-005 | E2E-017, E2E-018, E2E-041 (3) |
| FR-NEW-006 | E2E-019, E2E-020, E2E-021 (3) |
| FR-NEW-007 | E2E-015, E2E-018, E2E-021, E2E-022, E2E-024, E2E-034, E2E-046 (7) |
| FR-NEW-008 | E2E-025, E2E-026, E2E-027 (3) |
| FR-NEW-009 | E2E-022, E2E-023, E2E-024 (3) |
| FR-NEW-010 | E2E-029, E2E-038, E2E-039 (3) |
| FR-NEW-011 | E2E-030, E2E-031, E2E-040 (3) |
| FR-NEW-012 | E2E-032, E2E-033, E2E-034 (3) |
| FR-NEW-013 | E2E-033, E2E-042, E2E-043 (3) |
| FR-NEW-014 | E2E-044, E2E-045, E2E-046, E2E-052 (4) |
| FR-NEW-015 | E2E-001, E2E-047, E2E-048 (3) |

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Remote tag | a tag held by the remote, not by the project | Remote exchange |
| Confirmation commit | the commit the member states to confirm a deletion: the full id or at least 7 hexadecimal characters, case insensitive, spaces ignored | Remote exchange |
| Moved | a remote tag whose commit is no longer the one the member saw or stated | Remote exchange |
| Declined | a deletion the remote refuses (for example a protected tag) | Remote exchange |
| Tag | a named mark on a commit | Remote exchange |
| Local tag | a tag held by the project | Remote exchange |
| Commit | a recorded state of the project's files | Remote exchange |
| Credential | the stored token a member's account holds for the remote | Remote exchange |
| Audit trail | the sequence of audit lines | Audit |
| Remote | the hosted repository the project tracks, known by a name (default `origin`) | Remote exchange |
| Branch | a named line of work in the project; deleting a tag never moves one | Remote exchange |
| Project | the set of files, tags and remotes the members work on (`proj-a` in the tests) | Actors |
| Host | the server name of a remote, shown in connection refusals | Remote exchange |
| Refusal | the message a member receives when an action is not carried out | Actors |
| Audit entry | one audit line with the member who caused it | Audit |
| Member | a person who belongs to the project | Actors |
| Non member | a person who does not belong to the project | Actors |
| Operator | the person who reads the audit trail | Audit |
| Audit line | the operator visible record of a deletion | Audit |
| Paused operation | a merge, rebase, cherry pick or revert waiting to be finished or abandoned | Remote exchange |
| Annotated tag | a tag with a message of its own, confirmed here by the commit it points at | Remote exchange |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 5 | 3 | NOT-IMPLEMENTABLE |
| 2 | 3 | 0 | NOT-IMPLEMENTABLE |
| 3 | 4 | 3 | NOT-IMPLEMENTABLE |
| 4 | 0 | 3 | IMPLEMENTABLE |
| consistency review | 3 precision points | 0 | fixed |

**Amendments applied:** round 1 (non member and connection texts spelled, FR-NEW-013 lost answer, FR-NEW-014 named remote, traceability column, E2E-042 to 046), round 2 (FR-NEW-015 result content, invalid tag names count as absent, non hexadecimal stated text, E2E-047 to 051), round 3 (default remote `origin`, E2E-052, annotated tag test E2E-027, race wording), round 4 (outcome statistics, gate record, timeout setting design input), final review (outcome statistics recounted, missing credential named as a trigger of FR-NEW-012, glossary completed, edge case table row).
**No implementation detail outside Section 14:** PASS (the tool names inside two verbatim messages are existing user visible text)

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Publishing sends one tag, replaces a differing remote tag only with the stated commit | `crates/core/src/tools/git_tags.rs:534-726` (`publish_tag`, `publish_inner`, `publish_on_repo`), `:799` (`stated_commit_matches`, 7 character minimum at `:29`) |
| The "moved" refusal wording | `crates/core/src/tools/git_tags.rs:774` (`moved`) |
| No remote refusal for publishing | `crates/core/src/tools/git_tags.rs:476-483` |
| Advertised refs listing and tag push | `crates/core/src/git/remote.rs:793` (`list_advertised_refs`), `:696` (`push_tag_to_remote`), `:767` (`tag_push_error`), `:784` (`remote_refused_tag`) |
| Publishing is not blocked by a paused operation | `crates/core/src/tools/git_tags.rs:466-499` (no `reject_if_operation_in_progress` call, unlike `tag_create` at `:44`) |
| Local deletion never touches the remote | `crates/core/src/tools/git_tags.rs:64-78` (`tool_tag_delete`), `:191` (`tag_delete`), SPEC-0052/FR-NEW-005 |
| Audit lines | `crates/core/src/tools/git_tags.rs:592-600` (publication) |
| Non member refusal text | `crates/core/src/storage/admin.rs:393` (`'{person}' is not a member of '{project_id}'`) |
| Connection, credential and timeout refusal texts | `crates/core/src/git/remote.rs:1226-1229` (`connection_error`), `:1269-1273` (`timeout_error`); `crates/core/src/git/oauth/store.rs:254-258` |
| Remote deletion is out of scope in SPEC-0052 | `sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:52` |
Test command: `cargo test --workspace`

### 14.3 Design inputs
- `<n>` in the no answer refusal is the existing remote timeout setting used by `timeout_error` (`crates/core/src/git/remote.rs:1269`); no new setting; signal: E2E-032.
- A new action is a contract change: new tool, golden regeneration (`MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`), count sites (114 to 115 total and the `git.*` family count, recount with `rg -n '114' crates` and the family constants), `TOOL_CONTRACT.txt`, `AGENTS.md` and `.agent_docs/tools.md` parity (`e2e_new_957`, `e2e_new_958`); signal: the golden and parity tests fail until updated.
- Deleting a ref on the remote needs a delete form of the push refspec through the existing push core (`git/remote.rs:609` `push_refs`, `:696` `push_tag_to_remote`); the lease on the advertised tag object (as the forced publication does) is what makes FR-NEW-003's "moved meanwhile" safe; signal: E2E-009, E2E-031.
- Reuse `list_advertised_refs` (`git/remote.rs:793`) for the remote tag state and the peeled commit (annotated tags: the `^{}` entry); reuse `stated_commit_matches` and `short7` (`git_tags.rs:799,812`); signal: E2E-002 to E2E-004, E2E-006, E2E-027.
- The action label and location under which the audit line is recorded are the designer's choice, following the labels of SPEC-0052 (one label per action); signal: E2E-025.
- `crates/agent/src/mcp.rs:317-319` asserts the full 114 tool catalogue and must become 115; signal: that test fails until updated.
- The audit must not use `run_remote_operation` (it audits failures); use `trace_remote_operation` and write the line after a real deletion, as publication does (SPEC-0052 DEC-016); signal: E2E-026, E2E-033.
- Local file transport runs no hooks, so a protecting remote (E2E-019, E2E-020) must be simulated through the push status callback, as E2E-085 of SPEC-0052 was; connection loss is simulated with a transport that drops connections (E2E-033); signal: these tests need a seam.
- Refusal order (FR-NEW-007) follows the order of the calls: authorize, remote lookup, credential and listing, then the tag state checks; signal: E2E-015, 018, 021, 024, 034.
- A distinct refusal function is needed for the remote's declining of a deletion (`remote_refused_tag`, `git/remote.rs:784`, words a name refusal); signal: E2E-019, E2E-020.
- Result shape: tag, remote, the deleted commit (full id) and the credential kind, like publishing; design decides the exact keys.

