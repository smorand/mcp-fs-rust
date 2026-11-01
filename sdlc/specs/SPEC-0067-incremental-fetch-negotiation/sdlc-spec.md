# Incremental fetch: functional specification

> Id: SPEC-0067
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: one area (the git service), 14 functional requirements and 3 qualities, no persisted data change; what a client observes during a fetch changes, which rules out S.
> Status: implemented
> Generated on: 2026-10-31
> Project: existing
> From backlog: BL-0114
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
A developer who already holds part of a project's history and runs `git fetch` receives only the commits and files it does not have, and the service conducts that exchange the way git's own server does. Today the service ignores what the developer says it holds and sends the whole history for every exchange of the negotiation. After this change a fetch with nothing new transfers nothing and a fetch of two new commits transfers those two commits. Cloning stays as it is.

The contract is a reference: for any request, the service answers as git's own server answers for the same project data, except for a short, closed list of declared differences (FR-NEW-003, 004, 007, 008, 009, 010, 012, 013 and 014). The tests compare the two answers; they do not enumerate the cases of the exchange by hand.

## 2. Current State (MANDATORY, every depth)
### 2.1 What users can do today
- A developer can clone a project over the git service and fetch from it. A fetch works, but the service ignores which commits the developer says it holds: every answer carries the whole history reachable from what was asked for (rows 1 and 2).
- When the developer's copy has history, the git client talks to the service in several exchanges. The service answers each with the complete history and the client discards all but the last answer. One fetch costs several times the cost of sending the project once (rows 1 to 3).
- Asking for something the project does not have gives an answer with no data and no error (row 4).
- A request that is empty, or cut in the middle of a line, is read as ending before the cut; the answer carries no data and no error (row 9).
- Reading follows the access rules of the project: a member can read; a caller with no identity can read only when anonymous reading is switched on; push always needs an identity (rows 5 and 6).
- A project of 20 000 objects took 1.6 to 1.8 seconds to answer one request in a development build, measured on 2026-10-30, before SPEC-0066 removed about 0.12 to 0.17 seconds of it (not re-measured since; row 7).

### 2.2 Specs governing this area
- SPEC-0066 (preparing the project's data before answering, implemented): unchanged; the answer is still built from the project's objects after that step.
- No existing spec defines negotiation. The project records "no negotiation of what the client already holds" as a known divergence from standard git (row 3), so this increment closes a documented gap and violates no requirement.

### 2.3 What is verified today
- Unit tests exercise the answer for a clone of a single commit, for a request with nothing wanted, and for a push followed by a fetch. None sends commits the client holds.
- An end to end suite runs three real `git clone`s. It needs external services and is excluded from the default run. No test runs a real `git fetch` into a copy that already has history.
- Test command and files: Section 14.1.

## 3. Scope
### 3.1 In scope
- The service answers a fetch the way git's own server does: the same acknowledgement lines in the same order, the same closing line, and the same set of objects.
- Clone, empty project, and access rules behave as today.
- The declared differences (FR-NEW-003, 004, 007, 008, 009, 010, 012, 013 and 014) keep the behavior the service has today where the reference answer would stop, send nothing or honour an option the service does not offer.

### 3.2 Out of scope, and why
- Shallow clones, partial clones, filters and any option the service does not offer today: nobody asked for them; a request using one is answered as today (FR-NEW-009).
- Any way of reducing the transfer size other than not sending what the client holds.
- Any change to what push sends or does.

### 3.3 Constraints imposed by the context
- The service must keep working with the git clients already in use (the version on the owner's machine is recorded in row 8): the client decides how the negotiation runs.
- "The reference answer" means the answer that git's own server program gives for the same project data and the same request, using only the options this service offers today (project owner: git compatibility). Where this document says "equals the reference answer", the acknowledgement lines are compared line by line and the objects as a set; the order and the packing of the objects are not compared.
- The declared differences are exactly FR-NEW-003, 004, 007, 008, 009, 010, 012, 013 and 014; every other difference from the reference answer is a defect (project owner).
- No data migration and no new setting: the change is on by default (project owner).

## 4. Actors
- **Developer:** uses the git client against a project; clones, fetches, pulls.
- **Operator:** runs the service; cares about load and about reads changing nothing.
- **Anonymous reader:** a caller with no identity, allowed only when anonymous reading is on.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| Git service | Answering clone and fetch for a project | wanted commits, held commits, acknowledgement, reference answer |
| Project access | Who may read a project | member, anonymous reading |

## 5. Usage scenarios
### SC-001: Clone
**Actor:** Developer
**Preconditions:** project `demo` holds 4 commits on `main`.
**Flow:**
1. The developer runs `git clone` on `demo` → the service sends the 4 commits with all their files.
**Postconditions:** the copy has the 4 commits and passes the integrity check.
**Exceptions:**
- EXC-001a: `demo` has no commit → the clone ends with an empty copy and git's usual message `You appear to have cloned an empty repository.`

### SC-002: Fetch new commits
**Actor:** Developer
**Preconditions:** the copy holds `main` at `c4`; the project later gained `c5` and `c6`.
**Flow:**
1. The developer runs `git fetch` → the client states the commits it holds → the service answers as the reference answer → the client ends the negotiation → the service sends only `c5` and `c6` with their new files.
**Postconditions:** the copy holds 6 commits; the objects received are exactly those of `c5` and `c6`.
**Exceptions:**
- EXC-002a: a push is arriving at the same time → the fetch ends with each branch at its old or its new tip, never with a commit missing its parents.

### SC-003: Fetch with nothing new
**Actor:** Developer
**Preconditions:** the copy holds everything on the project.
**Flow:**
1. The developer runs `git fetch` → git sees nothing to ask for, or the service answers an explicit request for a held commit with no objects.
**Postconditions:** the copy is unchanged.

### SC-004: Fetch when the copy has other history
**Actor:** Developer
**Preconditions:** the copy holds commits the project does not have (local work), or a long history of which little is on the project.
**Flow:**
1. The developer runs `git fetch` → over one or several exchanges the service acknowledges the commits in common and ignores the others → the service sends what is missing.
**Postconditions:** the copy is correct and complete; its local commits are kept.

### SC-005: Invalid or incomplete request
**Actor:** a git client or a script.
**Preconditions:** a request names an id that is not valid or not held by the project, is empty, or is cut in the middle of a line.
**Flow:**
1. The request is sent → the service answers as it does today (FR-NEW-004, 007, 008, 009, 010, 012, 013, 014).
**Postconditions:** the project is unchanged; no error is reported.

### SC-006: Access
**Actor:** Developer, Anonymous reader
**Preconditions:** the access rules of the project.
**Flow:**
1. A member, a non member or a caller with no identity asks to clone or fetch → access is granted or refused exactly as before this change.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: Same acknowledgements as the reference
> WHEN a client states wanted and held commits without ending the negotiation, THE git service SHALL answer with the acknowledgement lines of the reference answer, in the same order, and no object data.
- **Business rules:** nothing is sent beyond the lines the reference answer contains; a held id that is not a valid commit is not acknowledged (see FR-NEW-004).
- **Scenario:** SC-002, SC-003, SC-004
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: Same closing line and same objects as the reference
> WHEN the client ends the negotiation, THE git service SHALL answer with the acknowledgement lines and the closing line of the reference answer, followed by the same set of objects as the reference answer.
- **Business rules:** an object that belongs to a held commit's own contents is not sent unless the reference answer sends it; the order and the packing of the objects are not compared.
- **Scenario:** SC-002, SC-003, SC-004
- **Priority:** Must

#### FR-NEW-003 [EARS-E]: Clone and wants only send everything
> WHEN the request contains no `have` line, THE git service SHALL send every object reachable from the wanted commits, whether or not the client has ended the negotiation.
- **Business rules:** this is today's behavior for a clone and for a request that states wants and nothing else, kept. The count of `have` lines is taken before FR-NEW-004 and FR-NEW-007 discard any id. It is a declared difference: the reference program sends nothing for wants without an end marker.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Held id that is not well formed or not held
> IF a held id is not a well formed object id or is the id of no object of the project, THEN THE git service SHALL treat it as not stated and SHALL NOT report an error.
- **Business rules:** declared difference: the reference program stops the exchange on a badly formed id.
- **Scenario:** SC-003, SC-004, SC-005
- **Priority:** Must

#### FR-NEW-005 [EARS-UB]: Reads change nothing
> THE git service SHALL NOT change any data of the project while answering a clone or a fetch.
- **Scenario:** SC-001 to SC-004
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: Same access rules
> THE git service SHALL apply to a clone and to a fetch the same access rules as before this change.
- **Business rules:** member required; no identity allowed only when anonymous reading is on.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Held id of an object that is not a commit
> IF a held id is a well formed id of an object the project holds and that object is not a commit, THEN THE git service SHALL NOT acknowledge it and SHALL treat it as not stated, without reporting an error.
- **Business rules:** declared difference: the reference program acknowledges any held object, whatever its kind. Kinds: a file version, a folder listing, an annotated tag.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-008 [EARS-O]: Wanted id that cannot be served
> IF a wanted id is malformed, is the id of no object of the project, or is the id of an object that no branch or tag reaches, THEN THE git service SHALL send no data for it and SHALL NOT report an error.
- **Business rules:** declared difference: the reference program refuses a request for an object no branch or tag reaches, and stops on a malformed id.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Options the service does not offer, as request lines
> IF a request carries a shallow, deepen or filter line, THEN THE git service SHALL answer the whole request as it does today: the single acknowledgement line `NAK`, no `ACK` line, no `shallow` line, and every object reachable from the wanted commits, whatever the held ids and whether or not the client ended the negotiation.
- **Business rules:** declared difference: the reference program honours these options.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-010 [EARS-O]: Empty, cut or want-less request
> IF a request is empty, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request ends in the middle of a line, THEN THE git service SHALL discard the cut line and everything after it and answer the lines before it by every other requirement, with no error. IF a request, once cut, contains no wanted line, or contains none at all, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request carries a shallow, deepen or filter line and contains no wanted line, THEN THE git service SHALL answer as it does today for a request that wants nothing (a bare flush, no `NAK`), this requirement taking precedence over FR-NEW-009.
- **Business rules:** declared difference. A request that states held ids and ends the negotiation but wants nothing is answered like an empty one.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-011 [EARS-E]: Unusable held ids without end of negotiation
> WHEN a request wants at least one valid commit, contains at least one `have` line none of which is acknowledged, and does not end the negotiation, THE git service SHALL answer as the reference answer does: the closing acknowledgement line and no object data.
- **Business rules:** once the negotiation is ended, such a request is answered under FR-NEW-002 with every object reachable from the wanted commits.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-012 [EARS-O]: Options of the request that the service does not offer
> IF the first wanted line of a request carries an option other than `multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta` or `agent`, THEN THE git service SHALL ignore that option and answer the request as it would without it.
- **Business rules:** declared difference: the reference program honours options such as `include-tag`, `no-done`, `thin-pack`, `no-progress` and `allow-tip-sha1-in-want`.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-013 [EARS-O]: No wanted id can be served
> IF every wanted id of a request is malformed, is the id of no object of the project, or is the id of an object no branch or tag reaches, THEN THE git service SHALL answer as it does today: the single line `NAK`, then an answer with zero objects, whatever the held ids and whether or not the client ended the negotiation, and SHALL NOT report an error.
- **Business rules:** declared difference. When at least one wanted id can be served, the others are ignored (FR-NEW-008) and the request is answered on the ones that can.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-014 [EARS-O]: Request the reference program rejects for any other reason
> IF the reference program rejects or stops on a request for a reason that no other declared difference covers (a missing flush after the wanted lines, a request that ends between two lines, lines in an unexpected order), THEN THE git service SHALL treat the lines present as complete, read a missing flush as present, end the negotiation only if the request contains the end marker, answer by every other requirement, and SHALL NOT report an error.
- **Business rules:** declared difference, and the catch-all of the list: the service never reports an error for a request the reference program rejects, and never changes data of the project for it.
- **Scenario:** SC-005
- **Priority:** Must

### 6.2 Modified
None.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
### 7.1 Performance
- NFR-001 [EARS-E]: WHEN a client asks for a commit it holds on a project of 20 000 objects, THE git service SHALL answer in under 1 second and send no object (baseline: 1.6 to 1.8 seconds before SPEC-0066, development build; not re-measured since).
- NFR-003 [EARS-E]: WHEN a client fetches one new commit changing 1 file on a project of 20 000 objects, THE git service SHALL answer in under 1 second and send exactly the 3 objects of that commit.
### 7.2 Security and access
- Covered by FR-NEW-006.
### 7.3 Privacy and compliance
- Not applicable: no new data is exposed; the service sends objects the caller could already receive in a clone.
### 7.4 Usability and accessibility
- Not applicable.
### 7.5 Reliability
- NFR-002 [EARS-S]: WHILE a push updating one or several branches is being received, THE git service SHALL answer a fetch with the state of each branch either entirely before or entirely after the push, never with a commit missing its parents.
- **Business rules:** a push of several branches sets them one by one, so the branches need not move together.
### 7.6 Operability
- Not applicable: no new message or setting.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | project with no commit | empty copy, usual message | E2E-013 |
| Boundary values | client holds every wanted commit; wants only | no objects; everything | E2E-003, E2E-023, E2E-012 |
| Concurrent actions | fetch during a push; a push that fails; two branches | consistent per branch | E2E-025, E2E-026, E2E-027 |
| Slow or lost connectivity | negotiation interrupted; request empty, cut, or without a wanted line | acknowledgements only / as today | E2E-010, E2E-017, E2E-018, E2E-037 |
| Access states | member, non member, anonymous allowed, anonymous refused | as today | E2E-019, E2E-020, E2E-021 |
| Data variations | requests the reference rejects (no flush, odd order); no servable wanted id; unusable held ids without an end; unknown options; invalid, short or non commit held id; invalid, unknown or unreachable wanted id; unknown held ids; shallow, deepen or filter lines | ignored / no data / answered as today | E2E-011, E2E-014 to E2E-016, E2E-029 to E2E-036, E2E-041 to E2E-053 |
| Navigation | not applicable | n/a | n/a |
| Partial operations | stopped before the end | acknowledgements only | E2E-010 |
| Dependency failures | not applicable (no external dependency on this path) | n/a | n/a |
| Ordering and timing | merge history; file deleted and added back; tags | same objects as the reference | E2E-006, E2E-007, E2E-008, E2E-009 |
| Exhaustion | 20 000 objects, repeated, with unknown ids, several commits | under 1 second | E2E-023, E2E-024, E2E-028, E2E-038 to E2E-040 |

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Each test runs against a project served by the git service, through a real git client or, where a case needs an exact exchange, a request written by hand with the lines a git client sends. Where a Then says "equals the reference answer", the test also obtains the reference answer (3.3) for the same project data and request and compares as 3.3 says. /sdlc-design maps each test to a way of running it. "Transfer" means the objects the client received. The integrity check is git's own.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-003, FR-NEW-005 | Critical |
| E2E-002 | New | journey | SC-002 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-003 | New | journey | SC-003 | FR-NEW-001, FR-NEW-002, FR-NEW-005 | High |
| E2E-004 | New | journey | SC-004 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-005 | New | journey | SC-004 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-006 | New | edge | SC-002 | FR-NEW-002 | High |
| E2E-007 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-008 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-009 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-010 | New | edge | SC-004 | FR-NEW-001 | Critical |
| E2E-011 | New | edge | SC-004 | FR-NEW-001, FR-NEW-002, FR-NEW-004 | High |
| E2E-012 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-013 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-014 | New | failure | SC-005 | FR-NEW-004 | High |
| E2E-015 | New | failure | SC-005 | FR-NEW-008 | High |
| E2E-016 | New | failure | SC-005 | FR-NEW-008 | High |
| E2E-017 | New | failure | SC-005 | FR-NEW-010 | Medium |
| E2E-018 | New | failure | SC-005 | FR-NEW-010 | Medium |
| E2E-019 | New | failure | SC-006 | FR-NEW-006 | High |
| E2E-020 | New | failure | SC-006 | FR-NEW-006 | High |
| E2E-021 | New | journey | SC-006 | FR-NEW-006 | High |
| E2E-022 | New | side effect | SC-001 to SC-004 | FR-NEW-005 | Medium |
| E2E-023 | New | edge | SC-003 | NFR-001 | High |
| E2E-024 | New | edge | SC-002 | NFR-003, FR-NEW-002 | High |
| E2E-025 | New | edge | SC-002 | NFR-002 | High |
| E2E-026 | New | failure | SC-002 | NFR-002 | High |
| E2E-027 | New | edge | SC-002 | NFR-002 | Medium |
| E2E-028 | New | edge | SC-003 | NFR-001 | Medium |
| E2E-029 | New | failure | SC-005 | FR-NEW-007 | High |
| E2E-030 | New | failure | SC-005 | FR-NEW-008 | High |
| E2E-031 | New | failure | SC-005 | FR-NEW-009 | Medium |
| E2E-032 | New | failure | SC-005 | FR-NEW-004 | Medium |
| E2E-033 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-034 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-035 | New | failure | SC-005 | FR-NEW-009 | Medium |
| E2E-036 | New | failure | SC-005 | FR-NEW-009 | Medium |
| E2E-037 | New | failure | SC-005 | FR-NEW-010 | Medium |
| E2E-038 | New | edge | SC-003 | NFR-001, FR-NEW-004 | Medium |
| E2E-039 | New | edge | SC-002 | NFR-003 | Medium |
| E2E-040 | New | edge | SC-002 | NFR-003 | Medium |
| E2E-041 | New | failure | SC-004 | FR-NEW-011, FR-NEW-004 | Critical |
| E2E-042 | New | failure | SC-004 | FR-NEW-011, FR-NEW-007 | High |
| E2E-043 | New | edge | SC-004 | FR-NEW-011, FR-NEW-004 | High |
| E2E-044 | New | failure | SC-005 | FR-NEW-012 | Medium |
| E2E-045 | New | failure | SC-005 | FR-NEW-012 | Medium |
| E2E-046 | New | failure | SC-005 | FR-NEW-012 | Medium |
| E2E-047 | New | failure | SC-005 | FR-NEW-013 | Medium |
| E2E-048 | New | failure | SC-005 | FR-NEW-013 | Medium |
| E2E-049 | New | edge | SC-005 | FR-NEW-008, FR-NEW-013 | Medium |
| E2E-050 | New | failure | SC-005 | FR-NEW-009, FR-NEW-010 | Medium |
| E2E-051 | New | failure | SC-005 | FR-NEW-014 | Medium |
| E2E-052 | New | failure | SC-005 | FR-NEW-014 | Medium |
| E2E-053 | New | edge | SC-005 | FR-NEW-014, FR-NEW-003 | Medium |

Coverage: happy 6 (journey), failure 27, edge 19, side effect 1; total 53. Happy to failure ratio 6:27; failure plus edge outnumber happy 46:6. State transition tests: none, the service keeps nothing between the requests of one fetch.

### 9.2 New tests
#### E2E-001: clone of a four commit project
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-005
- **Given** project `demo` with commits `c1` to `c4` on `main` (`a.txt` added in `c1`, `b.txt` in `c3`) and a developer who is a member
- **When** the developer runs `git clone` on `demo`
- **Then** the copy has 4 commits with `main` at `c4` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's; the project is unchanged
- **Priority:** Critical

#### E2E-002: fetch of two new commits
- **Category:** journey; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy of `demo` at `c4`; the project then gains `c5` (adds `c.txt`) and `c6` (edits `a.txt`)
- **When** the developer runs `git fetch`
- **Then** the copy holds 6 commits, `origin/main` is at `c6`, the integrity check reports no problem
- **And** the objects sent are exactly the 6 of `c5` and `c6` (for each: the commit, its folder listing, the changed file), the same set as the reference answer's
- **Priority:** Critical

#### E2E-003: fetch with nothing new
- **Category:** journey; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-005
- **Given** a copy of `demo` at `c4` and the project still at `c4`
- **When** the developer runs `git fetch`; then a request wanting `c4`, stating `c4` as held, then ending the negotiation is sent
- **Then** git reports nothing to fetch and the copy is unchanged; the answer to the request equals the reference answer and carries 0 objects
- **And** the project is unchanged
- **Priority:** High

#### E2E-004: the copy holds commits the project lacks
- **Category:** journey; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy at `c4` with 3 local commits `l1`, `l2`, `l3` on top; the project gains `c5` (adds `d.txt`)
- **When** the developer runs `git fetch`
- **Then** the copy holds `c5` on `origin/main` and still holds `l1` to `l3`
- **And** the objects sent are exactly the 3 of `c5`, the same set as the reference answer's
- **Priority:** Critical

#### E2E-005: forty local commits, two in common
- **Category:** journey; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy with 40 local commits of which only `c3` and `c4` are on the project; the project is at `c6`
- **When** the developer runs `git fetch`
- **Then** the copy holds `c5` and `c6` and keeps its 40 commits; the integrity check reports no problem
- **And** every exchange equals the reference exchange, line by line; the objects sent are exactly those of `c5` and `c6`
- **Priority:** Critical

#### E2E-006: merge history with one side held
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `main` at merge commit `m1` joining `c4` and side commit `s1` (forks from `c3`, adds `side.txt`); a copy that holds `c4` only
- **When** the developer runs `git fetch`
- **Then** the copy holds `s1` and `m1`
- **And** the objects sent are the same set as the reference answer's and include no object that belongs to the contents of `c4`
- **Priority:** High

#### E2E-007: a file deleted long ago and added back
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `c1` adds `old.txt` (content `old`), `c2` deletes it, `c3` adds `other.txt`; a copy holds `c3`; the project gains `c4`, which adds `old.txt` with content `old` again
- **When** the developer runs `git fetch`
- **Then** the copy holds `c4` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's, whatever it is (this case is where git and a simpler walk differ)
- **Priority:** Medium

#### E2E-008: annotated tag on a held commit
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a copy at `c4`; the project gains annotated tag `v1` (message `first release`) on `c4`
- **When** the developer runs `git fetch --tags`
- **Then** the copy has tag `v1` with message `first release` pointing at `c4`
- **And** the objects sent are the same set as the reference answer's (the tag only)
- **Priority:** Medium

#### E2E-009: annotated tag on a new commit
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a copy at `c4`; the project gains `c5` (adds `d.txt`) and annotated tag `v2` on `c5`
- **When** the developer runs `git fetch --tags`
- **Then** the copy has `v2` pointing at `c5` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's (the tag, `c5`, its folder listing, `d.txt`)
- **Priority:** Medium

#### E2E-010: negotiation stopped before the end
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6` and stating `c3` then `c4` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer line by line and carries no object data
- **And** the project is unchanged
- **Priority:** Critical

#### E2E-011: twelve unknown held commits
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004
- **Given** a request wanting `c6` that states 12 well formed commit ids the project never held, then ends the negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer: the closing line and every object reachable from `c6`
- **And** none of the 12 ids appears in an acknowledgement line
- **Priority:** High

#### E2E-012: wants and nothing else
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003
- **Given** a request wanting `c6`, with no held id and no end of negotiation
- **When** the request is sent
- **Then** the answer carries every object reachable from `c6`, as before this change
- **And** no error is reported
- **Priority:** High

#### E2E-013: empty project
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003
- **Given** a project with no commit
- **When** the developer runs `git clone`
- **Then** git prints `You appear to have cloned an empty repository.` and the copy has no commit
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-014: a held id that is not a valid id
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-004
- **Given** a request wanting `c6` that states one held id which is not a valid commit id (the text `not-an-id`), then ends the negotiation
- **When** the request is sent
- **Then** the answer is as if that id had not been stated, with no error
- **And** the id appears in no line of the answer
- **Priority:** High

#### E2E-015: wanted object unknown to the project
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** a request wanting an object id that is well formed and not held by the project
- **When** the negotiation ends
- **Then** the answer is the line `NAK` with zero objects and no error, exactly as before this change
- **And** the project is unchanged
- **Priority:** High

#### E2E-016: wanted object that is not an id
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** a request wanting the text `xyz`
- **When** the negotiation ends
- **Then** the answer is the line `NAK` with zero objects and no error
- **And** the project is unchanged
- **Priority:** High

#### E2E-017: empty request
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request with no content
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing: no object, no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-018: request cut in the middle of a line
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request wanting `c6`, stating `c4` as held, then a second held line interrupted partway; the request ends there
- **When** the request is sent
- **Then** the answer equals the answer to the same request cut before the cut line (here: the reference answer for wanting `c6` and holding `c4` with no end of negotiation), with no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-019: a non member is refused
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** a caller with an identity who is not a member of `demo`
- **When** the caller runs `git fetch` on `demo`
- **Then** access is refused exactly as before this change
- **And** no object is sent
- **Priority:** High

#### E2E-020: anonymous caller refused when anonymous reading is off
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** anonymous reading off and a caller with no identity
- **When** the caller runs `git clone` on `demo`
- **Then** access is refused exactly as before this change
- **And** no object is sent
- **Priority:** High

#### E2E-021: anonymous caller allowed when anonymous reading is on
- **Category:** journey; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** anonymous reading on, a copy at `c4` made without identity, the project at `c6`
- **When** the caller runs `git fetch`
- **Then** the copy holds `c5` and `c6`
- **And** the objects sent are exactly the 6 of `c5` and `c6`
- **Priority:** High

#### E2E-022: reads change nothing
- **Category:** side effect; **Scenario:** SC-001 to SC-004; **Requirements:** FR-NEW-005
- **Given** project `demo` at `c4`, with its branches, tags, audit entries and trash listed
- **When** a clone, then a fetch with nothing new, then a fetch with 2 new commits are done
- **Then** branches and tags are identical apart from the 2 commits the test added, and the trash is unchanged
- **And** the audit log has no new entry from the three reads
- **Priority:** Medium

#### E2E-023: up to date request on a large project
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001
- **Given** a project of 20 000 objects with tip `c20000`; a request wanting `c20000` and stating `c20000` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer arrives in under 1 second, equals the reference answer and carries 0 objects
- **And** the project is unchanged
- **Priority:** High

#### E2E-024: one new commit on a large project
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003, FR-NEW-002
- **Given** a project of 20 000 objects, a copy that holds all of it, one new commit changing 1 file
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 3 (the commit, its top folder listing, the file)
- **Priority:** High

#### E2E-025: fetch during a push
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** a copy at `c4` and a push of `c5` to `c8` in progress
- **When** the developer runs `git fetch` at the same time
- **Then** the copy ends at `c4` or at `c8`, never in between (no `c6` without `c5`)
- **And** the integrity check reports no problem
- **Priority:** High

#### E2E-026: a push that fails during a fetch
- **Category:** failure; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** a copy at `c4` and a push of `c5` that the service rejects (the pusher is not a member)
- **When** the developer runs `git fetch` while the push is rejected
- **Then** the copy ends at `c4` with no integrity problem
- **And** the project still holds `c4` as its tip
- **Priority:** High

#### E2E-027: two fetches during a push moving two branches
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** two copies at `c4` (`main`) and `f1` (`feature`), and a push moving `main` to `c8` and `feature` to `f3` in progress
- **When** both developers run `git fetch` at the same time
- **Then** each branch in each copy ends at its old or its new tip, with no integrity problem
- **And** it is not asserted that the two branches move together
- **Priority:** Medium

#### E2E-028: five up to date requests in a row
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001
- **Given** the project and the request of E2E-023
- **When** the request is sent 5 times one after the other
- **Then** each answer arrives in under 1 second and carries 0 objects
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-029: held id of a file version
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** `demo` with `c1` to `c4`, and `a.txt` in `c4` with content `a`; a request wanting `c4`, stating the id of that `a.txt` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the objects sent are every object reachable from `c4`; the project is unchanged
- **Priority:** High

#### E2E-030: wanted commit that no branch reaches
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** `demo` at `c4`, plus an orphan commit `o1` that no branch or tag reaches; a request wanting `o1`, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects and no error
- **And** the project is unchanged
- **Priority:** High

#### E2E-031: deepen option with a held id
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, adding the line `deepen 1`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line and no `shallow` line
- **And** it carries every object reachable from `c4`; the project is unchanged
- **Priority:** Medium

#### E2E-032: held id with only ten characters
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-004
- **Given** a request wanting `c6`, stating the first 10 characters of the id of `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer is as if that id had not been stated, with no error
- **And** the id appears in no line of the answer
- **Priority:** Medium

#### E2E-033: held id of a folder listing
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** a request wanting `c4`, stating the id of the top folder listing of `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the objects sent are every object reachable from `c4`
- **Priority:** Medium

#### E2E-034: held id of an annotated tag
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** annotated tag `v1` on `c4`; a request wanting `c6`, stating the id of the tag `v1` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-035: filter option
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, adding the line `filter blob:none`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line
- **And** it carries every object reachable from `c4`
- **Priority:** Medium

#### E2E-036: shallow line from the client
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, stating a line `shallow` followed by the id of `c3`, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line and no `shallow` line
- **And** it carries every object reachable from `c4`
- **Priority:** Medium

#### E2E-037: held ids but nothing wanted
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request stating `c4` as held and ending the negotiation, with no wanted line
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing, with no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-038: up to date request with unknown ids besides the held tip
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001, FR-NEW-004
- **Given** a project of 20 000 objects with tip `c20000`; a request wanting `c20000` and stating `c20000` and two well formed ids the project never held, then ending the negotiation
- **When** the request is sent
- **Then** the answer arrives in under 1 second and equals the reference answer, with 0 objects
- **And** the unknown ids appear in no line
- **Priority:** Medium

#### E2E-039: two new commits on a large project
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003
- **Given** a project of 20 000 objects, a copy that holds all of it, and 2 new commits each changing 1 file in the top folder
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 6
- **Priority:** Medium

#### E2E-040: new commit adding a file in a new folder on a large project
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003
- **Given** a project of 20 000 objects, a copy that holds all of it, and one new commit adding `docs/readme.txt`
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 4 (the commit, the top folder listing, the listing of `docs`, the file)
- **Priority:** Medium

#### E2E-041: unusable held ids, no end of negotiation
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** a request wanting `c6` and stating 12 well formed ids the project never held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer: the closing line and no object data
- **And** the project is unchanged
- **Priority:** Critical

#### E2E-042: held id of a file version, no end of negotiation
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-007
- **Given** a request wanting `c6` and stating only the id of `a.txt` in `c4` as held, with no end of negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id; the answer equals the reference answer and carries no object data
- **And** the project is unchanged
- **Priority:** High

#### E2E-043: malformed held id, no end of negotiation
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** a request wanting `c6` and stating the text `not-an-id` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer carries no object and no error
- **And** the id appears in no line of the answer
- **Priority:** High

#### E2E-044: option `no-done` that the service does not offer
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** a request wanting `c4`, carrying the option `no-done` and stating `c3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer has the acknowledgement lines of the same request without `no-done`, and no object data
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-045: option `include-tag` that the service does not offer
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** annotated tag `v1` on `c3`; a request wanting `c4` and carrying the option `include-tag`, then ending the negotiation
- **When** the request is sent
- **Then** the objects sent are exactly those the same request without `include-tag` sends
- **And** the tag object is not among them
- **Priority:** Medium

#### E2E-046: an unknown option
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** a request wanting `c4` and carrying the option `x-foo`, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the same as for the request without `x-foo`
- **And** no error is reported
- **Priority:** Medium

#### E2E-047: unreachable wanted commit with a held id, no end
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-013
- **Given** `demo` at `c4` and an orphan commit `o1` that no branch or tag reaches; a request wanting `o1`, stating `c3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects, no `ACK` line for `c3` and no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-048: malformed wanted id with a held id
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-013
- **Given** a request wanting the text `xyz`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects, no `ACK` line for `c3` and no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-049: one servable and one unservable wanted id
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-013
- **Given** a request wanting `c6` and the orphan commit `o1`, stating `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer for the request wanting only `c6`
- **And** nothing of `o1` is sent
- **Priority:** Medium

#### E2E-050: deepen line with no wanted line
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009, FR-NEW-010
- **Given** a request carrying the line `deepen 1` and stating `c3` as held, with no wanted line, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing, with no `NAK` line and no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-051: no flush after the wanted line
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-014
- **Given** a request wanting `c6` and stating `c4` as held, with no flush after the wanted line and no end marker
- **When** the request is sent
- **Then** the answer equals the reference answer for the same request completed with the flush after the wanted line, with no error
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-052: end marker without a flush after the wanted line
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-014
- **Given** a request wanting `c6` and stating `c4` as held, then the end marker, with no flush after the wanted line
- **When** the request is sent
- **Then** the answer equals the reference answer for the same request completed with the flush after the wanted line, and carries every object of `c5` and `c6`
- **And** no error is reported
- **Priority:** Medium

#### E2E-053: wanted line only, no flush
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-014, FR-NEW-003
- **Given** a request wanting `c6` only, with no flush and no held id
- **When** the request is sent
- **Then** the answer equals the answer to the same request completed with the flush after the wanted line: every object reachable from `c6`
- **And** no error is reported
- **Priority:** Medium

### 9.3 Modified tests
None. The existing unit tests and the three real clones keep passing unchanged.

### 9.4 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-003, 005 | E2E-001 | none: a clone of a valid project cannot fail; access failures are SC-006 | E2E-012, 013 |
| SC-002 | FR-NEW-001, 002, 005, NFR-002, NFR-003 | E2E-002 | E2E-026 | E2E-006, 007, 008, 009, 024, 025, 027, 039, 040 |
| SC-003 | FR-NEW-001, 002, 004, 005, NFR-001 | E2E-003 | none: nothing is asked, so nothing can fail (the cut and empty requests are SC-005) | E2E-023, 028, 038 |
| SC-004 | FR-NEW-001, 002, 004, 007, 011 | E2E-004, 005 | E2E-041, 042 | E2E-010, 011, 043 |
| SC-005 | FR-NEW-004, 007, 008, 009, 010, 012, 013 | none: nothing valid is served | E2E-014 to 018, 029 to 037, 044 to 048, 050 to 052 | E2E-049, 053 |
| SC-006 | FR-NEW-006 | E2E-021 | E2E-019, 020 | none: access is granted or refused |

Side effect: E2E-022 (SC-001 to SC-004).

Tests per requirement (each at least 3): FR-NEW-001 (002, 003, 004, 005, 010, 011); FR-NEW-002 (002, 003, 004, 005, 006, 007, 008, 009, 011, 024); FR-NEW-003 (001, 012, 013, 053); FR-NEW-004 (011, 014, 032, 038, 041, 043); FR-NEW-005 (001, 003, 022); FR-NEW-006 (019, 020, 021); FR-NEW-007 (029, 033, 034, 042); FR-NEW-008 (015, 016, 030, 049); FR-NEW-009 (031, 035, 036, 050); FR-NEW-010 (017, 018, 037, 050); FR-NEW-011 (041, 042, 043); FR-NEW-012 (044, 045, 046); FR-NEW-013 (047, 048, 049); FR-NEW-014 (051, 052, 053); NFR-001 (023, 028, 038); NFR-002 (025, 026, 027); NFR-003 (024, 039, 040).

## 11. Open questions
- Whether the service ever tells the client early that it has enough: left to the reference answer; if the reference sends it, the service sends it (no option to switch it off). Not behavior changing beyond the number of rounds.
- The tests were designed by the requirements author, not an independent designer (depth M run in one context). Not behavior changing.
- This document replaces an earlier draft that listed every protocol case by hand; five audits of that draft each found new cases. Not behavior changing.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Copy | the developer's local clone of a project | all |
| Project | the repository served by the git service | all |
| Member | a person allowed to read and write a project | access |
| Anonymous reading | a setting letting a caller with no identity read | access |
| Anonymous reader | a caller with no identity | access |
| Branch, tag | a movable and a fixed name for a commit | fetch |
| Tip | the newest commit of a branch | fetch |
| Held commit | a commit a client says it already has | negotiation |
| Wanted commit | a commit the client asks for | fetch, clone |
| Wanted object | a commit or tag the client asks for | fetch, clone |
| Acknowledge | the service confirms that it also holds a stated commit | negotiation |
| Acknowledgement line | one line of the answer confirming a held commit, or saying none is held | negotiation |
| Closing line | the last acknowledgement line before the objects | negotiation |
| Folder listing | the stored list of the files and folders in one folder at one commit | all |
| Negotiation | the exchange where client and service agree on what the client lacks | fetch |
| Ends the negotiation | the client sends its final request, after which it expects the objects | fetch |
| Round | one request and answer of a negotiation | fetch |
| Declared difference | a case where the service deliberately answers differently from the reference answer, listed in Section 3.3 | all |
| Held id, wanted id | the id of an object a client states as held, or asks for | negotiation |
| Reference program | git's own server program, which produces the reference answer | tests |
| Shallow, deepen, filter line | a request line asking for a truncated or partial copy | fetch |
| Reference answer | the answer git's own server program gives for the same project data and request, with the options this service offers | all |
| Object | a commit, a file version, a folder listing or a tag stored by the project | all |
| Transfer | the objects the client receives for one clone or fetch | tests |
| Integrity check | git's own verification of a copy | tests |
| Merge commit | a commit joining two lines of history | fetch |
| Audit log, trash | the project's record of actions and its list of deleted files | tests |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 3 | 3 | NOT-IMPLEMENTABLE |
| 2 | 1 | 0 | NOT-IMPLEMENTABLE |
| 3 | 3 | 2 | NOT-IMPLEMENTABLE |
| 4 | 2 | 0 | NOT-IMPLEMENTABLE |
| 5 | not run | not run | WAIVED by the owner (round 4 findings amended, not re-audited) |

**Gate waiver (2026-10-31):** rounds 1 to 4 of this rewrite returned NOT-IMPLEMENTABLE (F: 3, 1, 3, 2, all amended). The owner waived a fifth audit; FR-NEW-014 is the catch-all for requests the reference program rejects. Residual risk: an unforeseen request shape is found by the comparison tests, not by the gate.
**Amendments applied (round 4, after the cap):** FR-NEW-010 gives a want-less request a bare flush even with a deepen, shallow or filter line (precedence over FR-NEW-009); FR-NEW-014 added as the catch-all declared difference for any request the reference program rejects (missing flush, end between lines, unexpected order); E2E-050 to 053.
**Amendments applied (round 3, after the cap):** FR-NEW-009 keeps the single line `NAK`; FR-NEW-010 split by case (empty, cut, want-less); FR-NEW-013 (no servable wanted id) added to the declared differences; E2E-047 to 049; E2E-015, 016, 018, 030, 031, 035, 036 made exact.
**Amendments applied (round 2):** FR-NEW-011 (unusable held ids without an end; pins the reference answer, not a difference) and FR-NEW-012 (options not offered) added; FR-NEW-003 keyed on the absence of any `have` line; E2E-041 to 046 added; FR-NEW-004 scenarios; glossary; wording of the reference refusal and the client version.
**Amendments applied (round 1):** declared differences completed (FR-NEW-007 non commit held ids, 008 unreachable or malformed wanted ids, 009 shallow, deepen and filter, 010 empty, cut or want-less request); FR-NEW-004 narrowed; NFR-001 split; tests 029 to 040 added; traceability, glossary, citations and wording corrected.
**No implementation detail outside Section 14:** PASS (protocol lines and the git commands a developer types are confined to the stated constraint of Section 3.3; checked by the consistency reviewer in rounds 1 and 2, wording fixed)

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)
### 14.1 Current State evidence
| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | the service ignores held commits, sends everything reachable | `crates/core/src/git/http/mod.rs:429` ("No negotiation: acknowledge nothing and send everything reachable"), `:398-417` (the `have` lines are parsed and not used) |
| 2 | every exchange gets the full answer | `crates/core/src/git/http/mod.rs:421-449` (`handle_upload_pack` builds the pack on every call; no branch on `done`) |
| 3 | documented as a known gap | `.agent_docs/git.md:147` ("There is no have/want negotiation") |
| 4 | unknown wanted object gives an answer with no data | `crates/core/src/git/http/mod.rs:471` (skipped), `:490-492` (empty pack when none) |
| 5 | access rules | `.agent_docs/git.md:133-136`; `crates/core/src/git/http/mod.rs:249` (`!(is_read && anonymous_read)`) |
| 6 | push needs an identity | `crates/core/src/git/http/mod.rs:191-193` |
| 7 | 1.6 to 1.8 s at 20 000 objects | `sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/sdlc-design.md:38-44`, measured 2026-10-30 |
| 8 | git client in use | `git --version` prints `git version 2.50.1 (Apple Git-155)` on 2026-10-31 |
| 9 | empty or cut request read as ending | `crates/core/src/git/http/pktline.rs:77-81,102-105` (a short or invalid packet returns an end), `crates/core/src/git/http/mod.rs:424-427` (no wanted line gives a bare flush) |

Verified tests: unit tests call `handle_upload_pack` at `crates/core/src/git/http/mod.rs:851,870,1504,1535` (the last two send wants with no held id and no end marker and expect the whole history); real clones at `crates/core/tests/full_stack_e2e.rs:545,592,660` (`#[ignore]`, needs infra; command `cargo test -p mcp-fs-core --all-features --test full_stack_e2e -- --ignored --nocapture`, Makefile `:23`). Default test command: `cargo test --workspace` (`make test`).

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- The oracle: run git's own server program in its stateless mode (`git upload-pack --stateless-rpc`) on a bare copy of the project's data, with the same request bytes and only the capabilities the service advertises (`crates/core/src/git/http/mod.rs:66-67`: `multi_ack multi_ack_detailed side-band-64k ofs-delta`, plus the `agent` string, which is not negotiated), and compare the acknowledgement lines and the object set (`git index-pack`/`git verify-pack` on the pack). Whether the reference sends `ACK <id> ready` is decided by the reference; the service follows it.
- A real `git fetch` with nothing new, or with only a new branch on a held commit, sends no object request at all (the client sees it already has everything; verified with `GIT_TRACE_PACKET` on 2026-10-31 over a local transport), so E2E-003 and E2E-023 need a hand written request for that part.
- libgit2 `insert_walk` marks only the trees of directly hidden commits uninteresting (`libgit2-sys-0.18.8+1.9.7/libgit2/src/libgit2/pack-objects.c:1684-1697`), whereas git's pack selection may differ for a file present in an older ancestor and absent from the held commit (E2E-007). Signal: E2E-007 and E2E-006 against the oracle. Any residual difference must be added to FR-NEW-004 or fixed.
- A wanted annotated tag goes through the non commit branch of the pack builder (`crates/core/src/git/http/mod.rs:473-480`, `insert_recursive`), which hides no held commit. Signal: E2E-008, E2E-009.
- The request parser stops at the first flush (`crates/core/src/git/http/mod.rs:403-405`), so the held commits git sends after the wanted commits and their flush are never read today. Signal: E2E-004 (it would receive 6 objects instead of 3).
- A held id absent from the object store must not fail the walk (revwalk `hide` of a missing commit errors in libgit2). Signal: E2E-011, E2E-014.
- NFR-002 relies on the push importing its objects (`crates/core/src/git/http/mod.rs:580-582`) before it sets each branch (`process_ref_update`), under the project's write lock (`:623`), while the fetch takes only the repository lock (`:433`). Signal: E2E-025, E2E-026, E2E-027.
- The reference program was run on 2026-10-31 (git 2.50.1) by the auditor: it acknowledges a held id of a file version, stops with `not our ref` for an unreachable wanted commit, honours `deepen`, and answers wants without an end marker with nothing; these are the declared differences. Cases left to the reference (uppercase ids, a repeated held id, a held id after the end marker, the lines for a stated id the project holds twice) are compared by the tests, not listed. Signal: E2E-029 to E2E-037 and the comparison tests.
- No limit exists today on the number of `have` lines in a request (the whole body is read, subject to the request size cap in the git service); a very large list costs work proportional to its length. Decision left to /sdlc-design; no behavior is required here.
- The service keeps no state between the requests of one fetch: the answer must be derived from each request alone.
- Timing thresholds (1 second) are for the development build used in SPEC-0066; /sdlc-design must state how they are measured.
