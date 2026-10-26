# Publish every local tag in one guarded action: functional specification

> Id: SPEC-0054
> Nature: FEAT
> Depth: M
> Depth evidence: 1 context (remote exchange), 16 anticipated requirements, no persisted change
> Status: designed
> Generated on: 2026-10-25
> Project: existing
> From backlog: BL-0076
> Depends on: SPEC-0052
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A member can send all the local tags of a project to a remote in one action, without sending a private mark by accident. The action first shows the member exactly which tags would be sent and what would happen to each of the others. Nothing is sent until the member confirms by stating how many tags are about to be sent. A tag that already exists differently on the remote is never replaced. Publishing tags one at a time, which SPEC-0052 delivered, becomes tedious after many releases; this is the follow up SPEC-0052 left out on purpose (its Section 3.2).

## 2. Current State (MANDATORY)
Empty repository: no. The product versions projects and exchanges them with remotes.

### 2.1 What users can do today
- A member can create, list and delete local tags, and publish **one named tag** to a remote. The tag is sent alone: no branch, no other tag, no commit. The commit behind the tag must already be on the remote.
- The remote can be named; when no name is given, or the name is empty, the remote called `origin` is used.
- Publishing a tag the remote already holds identically reports `up to date` and records nothing. A remote tag that differs is refused unless the member confirms the overwrite by stating the commit the remote tag has now.
- A tag whose commit the remote lacks is refused with `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.`
- Each tag actually sent leaves one entry in the audit trail: `published tag 'v1.0' at <full commit id>`.
- A paused merge, rebase, cherry pick or revert does not stop a tag from being published.
- There is no way to send several tags in one action; a member with 30 tags makes 30 calls.

### 2.2 Specs governing this area
- SPEC-0052/FR-NEW-007, FR-NEW-008, FR-NEW-012, FR-NEW-018, FR-NEW-019, FR-NEW-020: the rules for publishing one tag. This increment applies them tag by tag and does not modify them.
- SPEC-0053/FR-NEW-012: the remote connection failure texts, reused here so one vocabulary covers every remote action.
- Deviation: SPEC-0052 Section 3.2 declared publishing every tag out of scope; this spec brings it in with the guards below.

### 2.3 What is verified today
Publishing one tag is covered by the tag publish tests of SPEC-0052 (journeys, refusals, audit, connection failures). Nothing exercises several tags in one action, since no such action exists. Test command and files: Section 14.1.

## 3. Scope
### 3.1 In scope
- A preview of all local tags and what would happen to each.
- A confirmed send of the tags that are safe to send, guarded by the number the member states.
- A per tag report of the outcome.
- One audit entry per tag actually sent.
- Naming the remote, defaulting to `origin`.

### 3.2 Out of scope, and why
- Overwriting a differing remote tag in bulk: replacing a remote tag is a destructive remote act and stays a single tag decision with the confirmation of SPEC-0052/FR-NEW-008.
- Sending commits or branches: tags only, as for one tag (SPEC-0052/FR-NEW-019).
- Filtering tags by a name pattern or excluding names: the preview and the local deletion of a private mark cover the need; a pattern filter can follow if asked (no backlog entry, nobody asked).
- Selecting a subset by name in the same action: publishing one named tag already exists.
- Deleting remote tags: delivered by SPEC-0053.
- A cap on the number of tags or a rate limit: nobody asked for one.
- All or nothing sending: a partial result is reported tag by tag instead (decided with the requester).

### 3.3 Constraints imposed by the context
- Publishing tags must not make a private mark public by accident. Imposed by the requester (BL-0076 notes); met by the preview and the stated number.

## 4. Actors
- **Member**: a person who belongs to the project. Previews and publishes the tags.
- **Another member**: acts on the same project at the same time.
- **Remote**: the declared remote repository that receives the tags; it can refuse a tag.
- **Operator**: reads the audit trail to know who published which tag.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| Remote exchange | sending and receiving with a remote, remote refusals, connection failures | remote, publish, up to date, differs |
| Versioning | the local tags and their commits | tag, lightweight, annotated, commit |
| Audit | the record of who did what | audit entry |

## 5. Usage scenarios
### SC-001: Review then publish all tags
**Actor:** Member (Alice).
**Preconditions:** Alice belongs to `proj-a`. The project tracks the remote `origin`. Local tags: `v1.0` at `a1b2c3d`, `v1.1` at `b2c3d4e`, `v0.9` at `c3d4e5f`. The remote already holds `v0.9` identically. Commits `a1b2c3d`, `b2c3d4e` and `c3d4e5f` are on the remote.
**Flow:**
1. Alice asks to publish all tags without confirming → the product lists `v0.9 up to date`, `v1.0 to publish`, `v1.1 to publish` and says `Tags to publish: 2. Confirm by stating 2 to send them.` Nothing is sent.
2. Alice confirms stating 2 → the product sends `v1.0` and `v1.1` only, and reports `v0.9 up to date`, `v1.0 published`, `v1.1 published`.
**Postconditions:** the remote holds `v0.9`, `v1.0`, `v1.1`. The audit trail has two entries, one per tag sent. Local tags are unchanged.
**Exceptions:**
- EXC-001a: Alice states a number other than the one due → refused with the fresh list (FR-NEW-003).
- EXC-001b: the project tracks no remote or not the named one → refused (FR-NEW-010).
- EXC-001c: Alice is not a member → the usual refusal (FR-NEW-011).
- EXC-001d: the connection fails → refused, nothing reported sent (FR-NEW-012).
- EXC-001e: nothing is due → `No tag to publish.` (FR-NEW-004).
- EXC-001f: Bob confirmed the same tags a moment earlier → Alice is refused with the fresh list (FR-NEW-013).
**Interactions:** SC-002 for tags that are skipped.

### SC-002: Tags that are not sent
**Actor:** Member (Alice).
**Preconditions:** local tags `v1.0` (remote holds it at `f6a7b8c`, local at `a1b2c3d`), `wip-alice` (commit `d4e5f6a` not on the remote), `v2.0` (commit on the remote), `refused-tag` (commit on the remote, a name the remote will refuse with the reason `protected tag name`).
**Flow:**
1. Alice previews → the product lists, in this order, `refused-tag to publish`, `v1.0 differs (remote at f6a7b8c)`, `v2.0 to publish`, `wip-alice commit not on remote`, then `Tags to publish: 2. Confirm by stating 2 to send them.`
2. Alice confirms stating 2 → the product reports, in this order, `refused-tag refused by remote: protected tag name`, `v1.0 differs (remote at f6a7b8c)`, `v2.0 published`, `wip-alice commit not on remote`.
**Postconditions:** the remote has `v2.0`; `v1.0` on the remote is untouched; the commit `d4e5f6a` was not sent; one audit entry (`v2.0`).
**Exceptions:** EXC-002a: the remote refuses a tag after the preview → reported, the others still sent (FR-NEW-008).

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: Preview
> WHEN a member asks to publish all tags and does not state a number, THE Remote exchange context SHALL list every local tag with its outcome and SHALL NOT send anything.
- **Business rules:** tags are listed by name in ascending byte order; outcomes are `to publish`, `up to date`, `differs (remote at <7 characters>)` and `commit not on remote`; each tag has exactly one line, `<name> <outcome>`; when N is at least 1 the last line is `Tags to publish: <N>. Confirm by stating <N> to send them.` where N counts the tags marked `to publish`; when N is 0 FR-NEW-004 gives the last line; the preview records nothing in the audit trail.
- **User visible text:** `Tags to publish: 2. Confirm by stating 2 to send them.`
- **Scenario:** SC-001, SC-002. **Priority:** Must

#### FR-NEW-002 [EARS-E]: Confirmed send
> WHEN a member states a number equal to the count of tags to publish, THE Remote exchange context SHALL send exactly those tags and no other object, and report the outcome of every local tag.
- **Business rules:** the report has exactly one line per local tag, `<name> <outcome>`, and no other line; it uses the outcomes of FR-NEW-001 with `published` in place of `to publish`; it lists tags in the same order as the preview; the local tags are unchanged; no branch and no commit is sent.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-003 [EARS-O]: Wrong number
> IF the number a member states differs from the count of tags to publish at that moment, THEN THE Remote exchange context SHALL refuse, show the fresh list and leave the remote untouched.
- **User visible text:** `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by the list of FR-NEW-001 (M is the current count, N the stated number); the refusal line is always the first line and the list follows, in every case.
- **Business rules:** zero is a number: stating `0` while 2 tags are due is refused; the refusal records nothing in the audit trail.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-004 [EARS-O]: Nothing to publish
> IF no local tag is to publish and the member states no number or the number 0, THEN THE Remote exchange context SHALL answer `No tag to publish.` with the list of FR-NEW-001 and send nothing.
- **Business rules:** this covers a project with no tag, only identical tags, only differing tags and only tags whose commit is not on the remote; the list is empty when there is no tag; the answer is the list of FR-NEW-001 (empty when there is no tag) followed, as its last line, by `No tag to publish.`; the line `Tags to publish: 0. Confirm by stating 0 to send them.` is never shown; stating any other number is refused by FR-NEW-003 (`Tags to publish: 0, not <N>. Review the list, then confirm again.` followed by the list and no `No tag to publish.` line); nothing is recorded in the audit trail.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-005 [EARS-UB]: Differing remote tag
> The Remote exchange context SHALL NOT replace a remote tag that differs from the local tag of the same name when publishing all tags.
- **Business rules:** two tags are identical or differ by the definition of SPEC-0052/FR-NEW-007 (name, kind, commit and, for annotated tags, message); a differing tag is reported `differs (remote at <7 characters>)`, in the preview and in the report, where the 7 characters are the first 7 of the commit the remote tag points at (for an annotated tag, the commit it names); tagger and time are not compared; the other tags are still sent.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-006 [EARS-O]: Commit not on the remote
> IF the commit of a local tag is not on the remote, THEN THE Remote exchange context SHALL skip that tag, report it `commit not on remote` and send no commit.
- **Business rules:** "on the remote" has the meaning of SPEC-0052/FR-NEW-019; the other tags are still sent.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-007 [EARS-U]: Identical tag
> The Remote exchange context SHALL report a tag the remote already holds identically as `up to date` and leave no audit entry for it.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-008 [EARS-O]: Refusal by the remote
> IF the remote refuses a tag, THEN THE Remote exchange context SHALL report it `refused by remote: <the reason the remote gave>`, send the other tags anyway and leave the refused local tag unchanged.
- **Business rules:** a refused tag leaves no audit entry; tags sent before and after it stay sent; a local tag whose name the remote cannot accept as a reference (for example `a//b`, which the naming rules allow) is listed `to publish` and counted in N like any tag the remote has not yet refused, and in the report it is `a//b refused by remote: invalid ref name`.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-009 [EARS-E]: Audit trail
> WHEN a tag is sent, THE Audit context SHALL record the member and the line `published tag '<name>' at <full commit id>`.
- **Business rules:** one entry per tag sent; none for `up to date`, `differs`, `commit not on remote`, `refused by remote`, a preview, a wrong number or any refusal.
- **Scenario:** SC-001, SC-002. **Priority:** Must

#### FR-NEW-010 [EARS-O]: Remote lookup
> IF the project tracks no remote, or not the remote the member names, THEN THE Remote exchange context SHALL refuse and send nothing.
- **User visible text:** `Cannot publish tags: this project tracks no remote.`; `Cannot publish tags: this project does not track a remote named 'staging'.`
- **Business rules:** the remote name is compared exactly, letter case included, and shown as given; when no name is given the remote called `origin` is used (and refused with the second text if the project tracks remotes but not `origin`); the preview and the confirmed send are refused alike.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-011 [EARS-O]: Non member
> IF the requester is not a member of the project, THEN THE system SHALL refuse with the refusal it gives for any other project action and reveal no tag name.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-012 [EARS-O]: Connection failure
> IF the remote cannot be reached, answers too slowly, rate limits or the credential is missing or expired, THEN THE Remote exchange context SHALL refuse with the texts publishing one tag gives and report no tag as sent.
- **User visible text:** `fetch from host 'github.ibm.com' failed: <the reason the transport or the remote gave>`; `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`; `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`; `remote timeout: host 'github.ibm.com' did not respond within <n>s`.
- **Business rules:** the connection can drop after the remote applied some tags; those tags are not reported and leave no audit entry, and a retry reports them `up to date` (the rule of SPEC-0052/FR-NEW-007).
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-013 [EARS-E]: Concurrent confirmations
> WHEN two members confirm at the same time, THE Remote exchange context SHALL send each tag once and give the second member the refusal of FR-NEW-003, because the count of tags to publish is then smaller than the number the second member stated.
- **Business rules:** the second member's list shows the tags the first sent as `up to date`; the audit trail holds one entry per tag sent, whoever sent it.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-014 [EARS-UB]: Paused operation
> The Remote exchange context SHALL NOT refuse to publish all tags because a merge, rebase, cherry pick or revert is paused, and SHALL leave that paused operation as it was.
- **Scenario:** SC-001. **Priority:** Should

#### FR-NEW-015 [EARS-U]: Order of refusals
> The Remote exchange context SHALL give refusals in this order: non member, remote lookup, connection and credential, then a number that is not a count (FR-NEW-019) or a wrong number (FR-NEW-003).
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-016 [EARS-UB]: Publishing one tag is unchanged
> The Remote exchange context SHALL NOT change the behavior of publishing one named tag.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-018 [EARS-U]: One outcome per tag
> The Remote exchange context SHALL give each tag exactly one outcome, chosen in this order: `commit not on remote`, then `up to date`, then `differs (remote at <7 characters>)`, then `to publish`.
- **Business rules:** a tag whose commit the remote lacks is never reported `differs`, even when the remote holds a tag of that name; this is the order in which publishing one tag checks them (SPEC-0052/FR-NEW-020); it is applied to the remote as listed by the confirmed action.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-019 [EARS-O]: Number that is not a count
> IF the stated number is not a whole number of zero or more, THEN THE Remote exchange context SHALL refuse with `The number to confirm must be a whole number, zero or more.` and send nothing.
- **Business rules:** the stated value is a text of one to nine characters, each an ASCII digit 0 to 9; nothing else is a number (no sign, no space, no decimal point, no exponent, no other script), and a longer text is refused with the same text; leading zeros are accepted (`02` means 2); an empty text is not a number and is refused with the same text (only an absent value means no number stated); a text such as `two` reaches this refusal and is not rejected earlier with another message; the refusal comes at the position of FR-NEW-003 in the order of FR-NEW-015 and records nothing in the audit trail.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-020 [EARS-O]: Remote tag changed after the listing
> IF, between the listing taken by a confirmed action and its sending, the remote came to hold a tag of a name to publish that is not identical to the local tag (the FR-NEW-005 definition), THEN THE Remote exchange context SHALL NOT replace it, SHALL report that tag `differs (remote at <7 characters>)` where the 7 characters are the first 7 of the commit the remote tag points at now, and SHALL still send the other tags.
- **Business rules:** the listing meant here is the one the confirmed action itself takes; a change before that listing is handled by FR-NEW-003; if the remote tag came to be identical to the local tag, the tag is reported `up to date` with no audit entry (FR-NEW-007); no audit entry for a diverted tag; the report keeps the order and the one line per tag of FR-NEW-002; the remote tag is left as the remote has it; the outcome of a tag that was not to publish at the listing (`up to date`, `differs`, `commit not on remote`) is the one of that listing and is not examined again; the number the member stated stays valid when a tag due at the listing is diverted, and the report then holds fewer `published` lines than that number.
- **Scenario:** SC-002. **Priority:** Must

### 6.2 Modified
None.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
### 7.1 Performance
#### FR-NEW-017 [EARS-U]: Bounded cost
> WHEN 50 tags are due, THE Remote exchange context SHALL finish the preview and the confirmed send each without a timeout refusal under the same configured remote deadline that applies to publishing one tag.
- **Business rules:** the 50 tags may be a mix of lightweight and annotated tags.
- **Scenario:** SC-001. **Priority:** Should

### 7.2 Security and access
Covered by FR-NEW-011, FR-NEW-005 and the stated number of FR-NEW-002/003 (the guard against accidental publication).
### 7.3 Privacy and compliance
Tags hold names and messages the member typed; no new personal data class. The preview is the control that keeps a private mark from leaving the project.
### 7.4 Usability and accessibility
The same report order and wording in the preview and the report, so a member can compare them.
### 7.5 Reliability
FR-NEW-008 and FR-NEW-012: a partial result is explicit, and a retry is safe.
### 7.6 Operability
FR-NEW-009: an operator can find who published which tag and at which commit.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | project with no tag | `No tag to publish.` and an empty list | E2E-024 |
| Empty states | only identical tags | `No tag to publish.`, `0` accepted | E2E-025 |
| Boundary values | one tag due; 50 tags due | normal flow | E2E-004, E2E-026 |
| Boundary values | stated number 0 while tags are due | refused | E2E-011 |
| Concurrent actions | two members confirm at once | each tag once | E2E-035 |
| Slow or lost connectivity | down, rate limited, no answer, drop mid send | refused, retry safe | E2E-020, 021, 022 |
| Access states | non member; expired or no credential | refused | E2E-019, 022 |
| Data variations | uppercase, accents, a slash in a name | ordered by byte, sent as named | E2E-006, E2E-027 |
| Partial operations | the remote refuses one tag | others sent | E2E-016 |
| Dependency failures | remote down | refused | E2E-020 |
| Ordering and timing | a tag published between preview and confirm | refused with the fresh list | E2E-012 |
| Exhaustion | not applicable: no quota applies to tags | n/a | n/a |
| Navigation | not applicable: no screens | n/a | n/a |

## 9. Acceptance tests (the contract)
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, 007 | Critical |
| E2E-002 | New | journey | SC-001 | FR-NEW-002, 009 | Critical |
| E2E-003 | New | journey | SC-001 | FR-NEW-002 | High |
| E2E-004 | New | edge | SC-001 | FR-NEW-001, 002 | Medium |
| E2E-005 | New | journey | SC-001 | FR-NEW-010, 002 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-007 | New | journey | SC-002 | FR-NEW-001, 002 | High |
| E2E-008 | New | cross scenario | SC-001 | FR-NEW-016, 007 | High |
| E2E-009 | New | failure | SC-001 | FR-NEW-003 | Critical |
| E2E-010 | New | failure | SC-001 | FR-NEW-003 | High |
| E2E-011 | New | failure | SC-001 | FR-NEW-003 | High |
| E2E-012 | New | failure | SC-001 | FR-NEW-003, 013 | High |
| E2E-013 | New | failure | SC-002 | FR-NEW-005, 018 | Critical |
| E2E-014 | New | edge | SC-002 | FR-NEW-005 | High |
| E2E-015 | New | failure | SC-002 | FR-NEW-006 | Critical |
| E2E-016 | New | failure | SC-002 | FR-NEW-008 | High |
| E2E-017 | New | failure | SC-001 | FR-NEW-010 | High |
| E2E-018 | New | failure | SC-001 | FR-NEW-010 | High |
| E2E-019 | New | security | SC-001 | FR-NEW-011 | Critical |
| E2E-020 | New | failure | SC-001 | FR-NEW-012 | High |
| E2E-021 | New | failure | SC-001 | FR-NEW-012, 007 | High |
| E2E-022 | New | failure | SC-001 | FR-NEW-012 | High |
| E2E-023 | New | failure | SC-002 | FR-NEW-004, 005, 006 | High |
| E2E-024 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-025 | New | edge | SC-001 | FR-NEW-004, 007 | Medium |
| E2E-026 | New | edge | SC-001 | FR-NEW-002, 017 | Medium |
| E2E-027 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-028 | New | failure | SC-001 | FR-NEW-015, 010 | Medium |
| E2E-029 | New | failure | SC-001 | FR-NEW-015, 012 | Medium |
| E2E-030 | New | side effect | SC-001 | FR-NEW-009 | Critical |
| E2E-031 | New | side effect | SC-001 | FR-NEW-009, 007 | High |
| E2E-032 | New | side effect | SC-001 | FR-NEW-009, 011 | High |
| E2E-033 | New | side effect | SC-002 | FR-NEW-008, 009 | High |
| E2E-034 | New | side effect | SC-001 | FR-NEW-002 | High |
| E2E-035 | New | state transition | SC-001 | FR-NEW-013, 009 | High |
| E2E-036 | New | edge | SC-001 | FR-NEW-014 | Medium |
| E2E-037 | New | failure | SC-002 | FR-NEW-018 | High |
| E2E-038 | New | failure | SC-001 | FR-NEW-019 | High |
| E2E-039 | New | failure | SC-001 | FR-NEW-011, 015 | Medium |
| E2E-040 | New | edge | SC-002 | FR-NEW-006 | Medium |
| E2E-041 | New | failure | SC-002 | FR-NEW-008 | High |
| E2E-042 | New | failure | SC-001 | FR-NEW-013, 003 | High |
| E2E-043 | New | edge | SC-001 | FR-NEW-014 | Medium |
| E2E-044 | New | failure | SC-002 | FR-NEW-016, 005 | High |
| E2E-045 | New | edge | SC-001 | FR-NEW-017, 004 | Medium |
| E2E-046 | New | edge | SC-001 | FR-NEW-014 | Medium |
| E2E-047 | New | edge | SC-001 | FR-NEW-017 | Medium |
| E2E-048 | New | side effect | SC-002 | FR-NEW-016, 005 | Medium |
| E2E-049 | New | edge | SC-001 | FR-NEW-019 | Medium |
| E2E-050 | New | failure | SC-001 | FR-NEW-019, 015 | Medium |
| E2E-051 | New | edge | SC-002 | FR-NEW-018, 005 | Medium |
| E2E-052 | New | edge | SC-001 | FR-NEW-019, 002 | Medium |
| E2E-053 | New | failure | SC-001 | FR-NEW-019 | Medium |
| E2E-054 | New | failure | SC-002 | FR-NEW-020, 005 | High |
| E2E-055 | New | failure | SC-002 | FR-NEW-020 | Medium |
| E2E-056 | New | failure | SC-002 | FR-NEW-020, 007 | Medium |
| E2E-057 | New | failure | SC-002 | FR-NEW-008 | Medium |
| E2E-058 | New | edge | SC-001 | FR-NEW-010 | Medium |
| E2E-059 | New | failure | SC-001 | FR-NEW-019 | Medium |

Coverage: journey 5, cross scenario 1 (happy 6), failure 28, security 1, edge 17, side effect 6, state transition 1 = 59. Happy to failure plus security plus edge ratio 1:8 (beats 1:1).

### 9.2 New tests
Fixtures used throughout: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` all on the remote unless stated.

#### E2E-001: Preview lists every tag and sends nothing
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-007
- **Given** local tags `v1.0` at `a1b2c3d`, `v1.1` at `b2c3d4e`, `v0.9` at `c3d4e5f`; the remote holds `v0.9` identically. **When** Alice asks to publish all tags without a number. **Then** Alice sees, in this order, `v0.9 up to date`, `v1.0 to publish`, `v1.1 to publish`, then `Tags to publish: 2. Confirm by stating 2 to send them.` **And** the remote still holds only `v0.9`, and the audit trail has no new entry.
- **Priority:** Critical

#### E2E-002: Confirmed send publishes the due tags
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-009
- **Given** the state of E2E-001. **When** Alice confirms stating 2. **Then** Alice sees exactly the three lines `v0.9 up to date`, `v1.0 published`, `v1.1 published` and no other line. **And** the remote holds `v0.9`, `v1.0` at `a1b2c3d` and `v1.1` at `b2c3d4e`, the local tags are unchanged, and the audit trail holds exactly two new entries, both by Alice.
- **Priority:** Critical

#### E2E-003: Annotated tags keep their kind and message
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the local annotated tag `v2.0` with the message `Release 2.0` at `a1b2c3d` and the lightweight tag `v2.1` at `b2c3d4e`, none on the remote. **When** Alice previews, then confirms stating 2. **Then** the report says `v2.0 published` and `v2.1 published`. **And** the remote tag `v2.0` is annotated with the message `Release 2.0` and `v2.1` is lightweight.
- **Priority:** High

#### E2E-004: A single tag due
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** the single local tag `v1.0` at `a1b2c3d`, not on the remote. **When** Alice previews. **Then** she sees `v1.0 to publish` and `Tags to publish: 1. Confirm by stating 1 to send them.` **And** when she confirms stating 1 the report is `v1.0 published` and the remote holds `v1.0`.
- **Priority:** Medium

#### E2E-005: A named remote
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-002
- **Given** the project tracks `origin` and `backup`; local tag `v1.0` at `a1b2c3d`. **When** Alice previews and confirms stating 1 against `backup`. **Then** the report says `v1.0 published`. **And** `backup` holds `v1.0` and `origin` still has no `v1.0`.
- **Priority:** High

#### E2E-006: Order is by name, byte by byte
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the local tags `Zeta`, `alpha`, `été` and `v1` on commits that are on the remote. **When** Alice previews. **Then** the list order is `Zeta`, `alpha`, `v1`, `été`.
- **Priority:** Medium

#### E2E-007: A private mark is seen, removed, and not published
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** the local tags `v1.0` at `a1b2c3d` and `wip-alice` at `b2c3d4e`, none on the remote. **When** Alice previews and sees `wip-alice to publish`, deletes `wip-alice` locally, previews again and confirms stating 1. **Then** the second preview says `Tags to publish: 1. Confirm by stating 1 to send them.` **And** the remote holds `v1.0` and has no `wip-alice`.
- **Priority:** High

#### E2E-008: Bulk then single publish
- **Category:** cross scenario. **Scenario:** SC-001. **Requirements:** FR-NEW-016, FR-NEW-007
- **Given** Alice published all tags with `v1.0` due. **When** she publishes `v1.0` alone with the one tag action. **Then** the answer is `up to date`. **And** no further audit entry exists; publishing a tag `v3.0` alone still sends only `v3.0`, with one new entry.
- **Priority:** High

#### E2E-009: Number too low
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** two tags due (`v1.0`, `v1.1`). **When** Alice confirms stating 1. **Then** she is refused with `Tags to publish: 2, not 1. Review the list, then confirm again.` followed by the list. **And** the remote has neither tag and the audit trail has no new entry.
- **Priority:** Critical

#### E2E-010: Number too high
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** two tags due. **When** Alice confirms stating 5. **Then** she is refused with `Tags to publish: 2, not 5. Review the list, then confirm again.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

#### E2E-011: Zero is a number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** two tags due. **When** Alice confirms stating 0. **Then** she is refused with `Tags to publish: 2, not 0. Review the list, then confirm again.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

#### E2E-012: The count changed after the preview
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** Alice previewed 2 tags due (`v1.0`, `v1.1`). **When** Bob publishes `v1.1` alone, then Alice confirms stating 2. **Then** Alice is refused with `Tags to publish: 1, not 2. Review the list, then confirm again.` and the list shows `v1.0 to publish`, `v1.1 up to date`. **And** the remote holds `v1.1` (Bob's) and not `v1.0`.
- **Priority:** High

#### E2E-013: A differing remote tag is skipped, the others sent
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-005
- **Given** local `v1.0` at `a1b2c3d` while the remote `v1.0` is at `f6a7b8c`; local `v2.0` at `b2c3d4e`, not on the remote. **When** Alice previews, then confirms stating 1. **Then** the preview is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 to publish`, `Tags to publish: 1. Confirm by stating 1 to send them.`; the report is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 published`. **And** the remote `v1.0` is still at `f6a7b8c`, and the audit trail has one new entry (`v2.0`).
- **Priority:** Critical

#### E2E-014: A differing kind is skipped
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-005
- **Given** the local tag `v1.0` is lightweight at `a1b2c3d`, the remote `v1.0` is annotated at `a1b2c3d`. **When** Alice previews. **Then** the answer is exactly `v1.0 differs (remote at a1b2c3d)` then `No tag to publish.` **And** the remote tag is unchanged.
- **Priority:** High

#### E2E-015: A tag whose commit the remote lacks is skipped
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-006
- **Given** local `wip-alice` at `d4e5f6a`, a commit not on the remote; local `v2.0` at `b2c3d4e`. **When** Alice previews, then confirms stating 1. **Then** the report is exactly `v2.0 published`, `wip-alice commit not on remote`. **And** the remote does not hold the commit `d4e5f6a` nor a `wip-alice` tag.
- **Priority:** Critical

#### E2E-016: The remote refuses one tag
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-008
- **Given** local `v1.0`, `refused-tag` and `v2.0`, all on remote commits; the remote refuses `refused-tag` with the reason `protected tag name`. **When** Alice confirms stating 3. **Then** the report is exactly `refused-tag refused by remote: protected tag name`, `v1.0 published`, `v2.0 published`. **And** the local `refused-tag` is unchanged and the audit trail has two new entries (`v1.0`, `v2.0`).
- **Priority:** High

#### E2E-017: The project tracks no remote
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-010
- **Given** a project with local tags and no remote. **When** Alice previews, then confirms stating 1. **Then** both are refused with `Cannot publish tags: this project tracks no remote.`
- **Priority:** High

#### E2E-018: The named remote is not tracked
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-010
- **Given** the project tracks only `origin`. **When** Alice previews against `staging`, then against `Origin`. **Then** the refusals are `Cannot publish tags: this project does not track a remote named 'staging'.` and `Cannot publish tags: this project does not track a remote named 'Origin'.` **And** `origin` is untouched.
- **Priority:** High

#### E2E-019: A non member cannot probe or publish
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-011
- **Given** Mallory is not a member of `proj-a`, which has the tag `secret-release`. **When** Mallory previews, then confirms stating 1. **Then** both are refused with `'mallory@example.com' is not a member of 'proj-a'`, and no tag name appears in either answer. **And** the remote and the audit trail are unchanged.
- **Priority:** Critical

#### E2E-020: The remote is unreachable
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** two tags due and the remote down. **When** Alice previews. **Then** she is refused with `fetch from host 'github.ibm.com' failed: connection refused` (the remote refuses connections). **And** nothing is reported as sent and the audit trail is unchanged.
- **Priority:** High

#### E2E-021: The connection drops while sending, retry is safe
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012, FR-NEW-007
- **Given** two tags due, `v1.0` and `v1.1`. **When** Alice confirms stating 2 and the connection drops after the remote applied `v1.0` only. **Then** Alice is refused with `fetch from host 'github.ibm.com' failed: connection reset` (the remote drops the connection) and no tag is reported as sent. **And** a retry (preview then confirm stating 1) reports `v1.0 up to date`, `v1.1 published`, and the audit trail then has exactly one entry for `v1.1`.
- **Priority:** High

#### E2E-022: Credential or answer problems
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** two tags due. **When** Alice previews with an expired credential, with no stored credential, with a remote that rate limits, and with a remote that does not answer within 1 s. **Then** the refusals are, in turn, `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `fetch from host 'github.ibm.com' failed: rate limit exceeded`, `remote timeout: host 'github.ibm.com' did not respond within 1s`. **And** nothing is sent.
- **Priority:** High

#### E2E-023: Nothing is sendable
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-004, FR-NEW-005, FR-NEW-006
- **Given** local `v1.0` (remote at `f6a7b8c`, differs) and `wip-alice` (commit not on the remote). **When** Alice previews, then asks to confirm stating 0. **Then** both answers are exactly `v1.0 differs (remote at f6a7b8c)`, `wip-alice commit not on remote`, `No tag to publish.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

#### E2E-024: A project with no tag
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** a project with no tag. **When** Alice previews. **Then** the answer is exactly the one line `No tag to publish.` **And** nothing is recorded in the audit trail.
- **Priority:** Medium

#### E2E-025: Everything is up to date
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, FR-NEW-007
- **Given** the local tags `v1.0` and `v1.1` held identically by the remote. **When** Alice previews, then confirms stating 0. **Then** both answers are exactly `v1.0 up to date`, `v1.1 up to date`, `No tag to publish.` **And** the audit trail is unchanged.
- **Priority:** Medium

#### E2E-026: Fifty tags, annotated and lightweight
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-017
- **Given** 25 annotated tags `a01` to `a25` (message `Release a01` and so on) already on the remote at the same commits with the same messages, and 25 lightweight tags `r26` to `r50` on remote commits, none on the remote. **When** Alice previews and confirms stating 25. **Then** neither answer is a timeout refusal; the report has 50 lines in name order, `a01 up to date` to `a25 up to date` then `r26 published` to `r50 published`. **And** the remote holds 50 tags and the audit trail holds 25 new entries.
- **Priority:** Medium

#### E2E-027: A name with a slash
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the local tag `release/1.0` at `a1b2c3d`. **When** Alice previews and confirms stating 1. **Then** the report says `release/1.0 published`. **And** the remote holds `release/1.0` at `a1b2c3d`.
- **Priority:** Medium

#### E2E-028: No remote beats a wrong number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-010
- **Given** a project with no remote and 2 tags. **When** Alice confirms stating 7. **Then** the answer is `Cannot publish tags: this project tracks no remote.`
- **Priority:** Medium

#### E2E-029: A connection failure beats a wrong number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-012
- **Given** 2 tags due and the remote down. **When** Alice confirms stating 7. **Then** the answer is `fetch from host 'github.ibm.com' failed: connection refused` and not the wrong number text.
- **Priority:** Medium

#### E2E-030: Audit lines
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** `v1.0` at `a1b2c3d` and `v1.1` at `b2c3d4e` due. **When** Alice confirms stating 2. **Then** the audit trail holds, for Alice, `published tag 'v1.0' at <full id of a1b2c3d>` and `published tag 'v1.1' at <full id of b2c3d4e>`. **And** no other entry was added.
- **Priority:** Critical

#### E2E-031: No audit for an up to date tag
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-009, FR-NEW-007
- **Given** `v0.9` identical on the remote and `v1.0` due. **When** Alice confirms stating 1. **Then** the audit trail gains one entry, for `v1.0`, and none for `v0.9`.
- **Priority:** High

#### E2E-032: No audit for any refusal or preview
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** two tags due. **When** Alice previews, states a wrong number, and a non member previews. **Then** the audit trail has no new entry after any of them.
- **Priority:** High

#### E2E-033: No audit for a tag the remote refused
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-008, FR-NEW-009
- **Given** the state of E2E-016. **When** Alice confirms stating 3. **Then** no audit entry names `refused-tag`.
- **Priority:** High

#### E2E-034: Only tags are sent
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the remote branch `main` at `a1b2c3d`, the local branch `main` ahead at `b2c3d4e`, the local tag `v1.0` at `a1b2c3d`. **When** Alice confirms stating 1. **Then** the remote branch `main` is still at `a1b2c3d` and the remote holds no other new commit.
- **Priority:** High

#### E2E-035: Two members confirm at once
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-013, FR-NEW-009
- **Given** `v1.0` and `v1.1` due; Alice and Bob both previewed and saw `Tags to publish: 2`. **When** both confirm stating 2 at the same moment. **Then** exactly one of them receives `v1.0 published`, `v1.1 published`. **And** the other receives `Tags to publish: 0, not 2. Review the list, then confirm again.` followed by the lines `v1.0 up to date`, `v1.1 up to date`; the remote holds both tags once, and the audit trail has exactly two new entries, one per tag, both by the member who sent.
- **Priority:** High

#### E2E-036: A paused merge does not block
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** a merge paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report says `v1.0 published`. **And** the merge is still paused with the same conflicts.
- **Priority:** Medium

#### E2E-037: A missing commit wins over a differing remote tag
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-018
- **Given** local `v1.0` at `d4e5f6a`, a commit not on the remote, while the remote holds `v1.0` at `f6a7b8c`. **When** Alice previews. **Then** the answer is exactly `v1.0 commit not on remote`, `No tag to publish.` and never `differs`.
- **Priority:** High

#### E2E-038: A negative number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-019
- **Given** two tags due. **When** Alice confirms stating `-1`. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

#### E2E-039: Non member beats no remote
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-011, FR-NEW-015
- **Given** Mallory is not a member of `proj-a`, which tracks no remote. **When** Mallory confirms stating 7. **Then** the answer is `'mallory@example.com' is not a member of 'proj-a'` and not the no remote text.
- **Priority:** Medium

#### E2E-040: Two tags whose commit the remote lacks
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-006
- **Given** local `wip-alice` and `wip-bob` both at `d4e5f6a`, not on the remote. **When** Alice previews. **Then** the answer is exactly `wip-alice commit not on remote`, `wip-bob commit not on remote`, `No tag to publish.` **And** nothing is sent.
- **Priority:** Medium

#### E2E-041: The remote refuses every tag
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-008
- **Given** `v1.0` and `v1.1` due and the remote refusing every tag with the reason `protected tag name`. **When** Alice confirms stating 2. **Then** the report is exactly `v1.0 refused by remote: protected tag name`, `v1.1 refused by remote: protected tag name`. **And** the audit trail is unchanged and the local tags are unchanged.
- **Priority:** High

#### E2E-042: A new tag appears after the preview
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013, FR-NEW-003
- **Given** Alice previewed `Tags to publish: 2` (`v1.0`, `v1.1`). **When** Bob creates `v1.2` at `c3d4e5f` locally in the same project and Alice confirms stating 2. **Then** Alice is refused with `Tags to publish: 3, not 2. Review the list, then confirm again.` and the list shows `v1.0 to publish`, `v1.1 to publish`, `v1.2 to publish`. **And** the remote holds none of them.
- **Priority:** High

#### E2E-043: A paused rebase does not block
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** a rebase paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report is exactly `v1.0 published`. **And** the rebase is still paused with the same conflicts.
- **Priority:** Medium

#### E2E-044: One tag publish keeps its overwrite rule after a bulk send
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-016, FR-NEW-005
- **Given** the state of E2E-013 after the confirmed send (the remote `v1.0` is at `f6a7b8c`, local at `a1b2c3d`). **When** Alice publishes `v1.0` alone without confirming an overwrite. **Then** she is refused with `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** High

#### E2E-045: Fifty tags, all up to date
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-017, FR-NEW-004
- **Given** 50 lightweight tags `r01` to `r50` held identically by the remote. **When** Alice previews. **Then** the answer is 50 lines `<name> up to date` in name order followed by `No tag to publish.`, without a timeout refusal.
- **Priority:** Medium

#### E2E-046: A paused cherry pick does not block
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** a cherry pick paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report is exactly `v1.0 published`. **And** the cherry pick is still paused.
- **Priority:** Medium

#### E2E-047: Fifty tags due, preview only
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-017
- **Given** 50 lightweight tags `r01` to `r50` on remote commits, none on the remote. **When** Alice previews. **Then** the answer is 50 lines `<name> to publish` in name order followed by `Tags to publish: 50. Confirm by stating 50 to send them.`, without a timeout refusal. **And** the remote has no new tag.
- **Priority:** Medium

#### E2E-048: Overwrite of a skipped tag stays a single tag decision
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-016, FR-NEW-005
- **Given** the state of E2E-013 after the confirmed send. **When** Alice publishes `v1.0` alone confirming the overwrite and stating `f6a7b8c`. **Then** the remote `v1.0` is at `a1b2c3d`. **And** the audit trail has one new entry `published tag 'v1.0' at <full id of a1b2c3d>`.
- **Priority:** Medium

#### E2E-049: A decimal number
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-019
- **Given** two tags due. **When** Alice confirms stating `2.5`. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** Medium

#### E2E-050: A word instead of a number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-019, FR-NEW-015
- **Given** a project with no remote and two tags. **When** Alice confirms stating `two`. **Then** the answer is `Cannot publish tags: this project tracks no remote.` **And** with a remote tracked and two tags due, stating `two` gives `The number to confirm must be a whole number, zero or more.` **And** with the remote down, stating `two` gives `fetch from host 'github.ibm.com' failed: connection refused`.
- **Priority:** Medium

#### E2E-051: A differing message makes an annotated tag differ
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-018, FR-NEW-005
- **Given** the local annotated tag `v2.0` with the message `Release 2.0` at `a1b2c3d`, and the remote annotated tag `v2.0` at `a1b2c3d` with the message `Other`. **When** Alice previews. **Then** the answer is exactly `v2.0 differs (remote at a1b2c3d)`, `No tag to publish.`
- **Priority:** Medium

#### E2E-052: Leading zeros are a number
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-019, FR-NEW-002
- **Given** two tags due. **When** Alice confirms stating `02`. **Then** the two tags are sent and the report has two `published` lines. **And** stating `0000000002` (ten characters) gives `The number to confirm must be a whole number, zero or more.`
- **Priority:** Medium

#### E2E-053: Signs and spaces are not a number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-019
- **Given** two tags due. **When** Alice confirms stating `+2`, then ` 2` (a space then 2). **Then** each answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** Medium

#### E2E-054: A remote tag appears between the listing and the send
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-020, FR-NEW-005
- **Given** local `v1.0` at `a1b2c3d` and `v2.0` at `b2c3d4e`, none on the remote; Alice previewed `Tags to publish: 2`. **When** Alice confirms stating 2 and, after the product listed the remote for this confirmation and before it sends, the remote receives `v1.0` at `f6a7b8c` from another source. **Then** the report is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 published`. **And** the remote `v1.0` is still at `f6a7b8c` and the audit trail has one new entry (`v2.0`).
- **Priority:** High

#### E2E-055: A remote tag moves between the listing and the send
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-020
- **Given** local `v1.0` at `a1b2c3d` and the remote `v1.0` also at `a1b2c3d` at the listing, so `v1.0 up to date`; and `v2.0` at `b2c3d4e` due. **When** Alice confirms stating 1 and, after the product listed the remote for this confirmation and before it sends, the remote moves `v1.0` to `f6a7b8c`. **Then** the report is exactly `v1.0 up to date`, `v2.0 published` (the line states the outcome of the listing; it is not examined again). **And** the remote `v1.0` is at `f6a7b8c` and the audit trail has one new entry (`v2.0`).
- **Priority:** Medium

#### E2E-056: A due tag is created on the remote by another member before the send
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-020, FR-NEW-007
- **Given** local `v1.0` at `a1b2c3d` and `v2.0` at `b2c3d4e`, none on the remote; Alice previewed 2. **When** Alice confirms stating 2 and, after the product listed the remote for this confirmation and before it sends, the remote receives `v1.0` at `a1b2c3d` (the same tag). **Then** the report is exactly `v1.0 up to date`, `v2.0 published`. **And** the audit trail has one new entry (`v2.0`).
- **Priority:** Medium

#### E2E-057: A name the remote refuses as a reference
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-008
- **Given** local `a//b` and `v1.0`, on remote commits, none on the remote. **When** Alice previews, then confirms stating 2. **Then** the preview is exactly `a//b to publish`, `v1.0 to publish`, `Tags to publish: 2. Confirm by stating 2 to send them.` and the report is exactly `a//b refused by remote: invalid ref name`, `v1.0 published`. **And** the audit trail has one new entry (`v1.0`).
- **Priority:** Medium

#### E2E-058: An empty remote name means origin
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010
- **Given** the project tracks `origin` and `v1.0` is due. **When** Alice previews with the remote name left empty. **Then** the answer is exactly `v1.0 to publish`, `Tags to publish: 1. Confirm by stating 1 to send them.` **And** with a project tracking only `backup`, the same request is refused with `Cannot publish tags: this project does not track a remote named 'origin'.`
- **Priority:** Medium

#### E2E-059: An empty number is not a number
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-019
- **Given** two tags due. **When** Alice confirms stating an empty text. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged; leaving the number out entirely gives the preview of E2E-001's shape.
- **Priority:** Medium

### 9.4 Modified tests
None. No existing test is modified.

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge | Side effect and state |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001..004, 007, 009..017, 019 | E2E-001, 002, 003, 005, 008 | E2E-009..012, 017..022, 028, 029, 038, 039, 042, 050, 053, 059 | E2E-004, 006, 024..027, 036, 043, 045..047, 049, 052, 058 | E2E-030..032, 034, 035 |
| SC-002 | FR-NEW-001, 002, 004..006, 008, 009, 016, 018, 020 | E2E-007 | E2E-013, 015, 016, 023, 037, 041, 044, 054..057 | E2E-014, 040, 051 | E2E-033, 048 |

Every requirement appears in at least three tests: FR-NEW-001 (001, 004, 006, 047), 002 (002, 003, 026, 034, 052), 003 (009..012, 042), 004 (023..025, 045), 005 (013, 014, 044, 048), 006 (015, 023, 040), 007 (001, 021, 025, 031), 008 (016, 033, 041, 057), 009 (002, 030..033), 010 (005, 017, 018, 028, 058), 011 (019, 032, 039), 012 (020..022, 029), 013 (035, 012, 042), 014 (036, 043, 046), 015 (028, 029, 039), 016 (008, 044, 048), 017 (026, 045, 047), 018 (037, 051, 013), 019 (038, 049, 050, 052, 053, 059), 020 (054, 055, 056).


## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Tag | a name attached to a commit; lightweight (name only) or annotated (with message and author) | Versioning |
| Publish | send a local tag to a remote under the same name | Remote exchange |
| Preview | the list of every local tag with its outcome, sent to nobody | Remote exchange |
| To publish | outcome of a tag the remote lacks and whose commit it holds | Remote exchange |
| Up to date | outcome of a tag the remote holds identically, as seen by the listing of the same action | Remote exchange |
| Differs | outcome of a tag the remote holds in another form (commit, kind or message) | Remote exchange |
| Private mark | a tag a member keeps for personal use and does not want public | Versioning |
| Remote | a repository the project exchanges with | Remote exchange |

## 13. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 5 | 2 | NOT-IMPLEMENTABLE |
| 2 | 2 | 3 (1 blocking) | NOT-IMPLEMENTABLE |
| 3 | 1 (minor, behavior visible) | 1 | NOT-IMPLEMENTABLE |
| 4 | 3 blocking, 1 minor (all on FR-NEW-020 and its tests, amended) | 3 minor | NOT-IMPLEMENTABLE |
| 5 | 2 minor (empty values, name refused by the remote) | 4 minor | amended directly (precision only), not re audited |

**Amendments applied:** round 1: order of lines, outcome precedence (FR-NEW-018), number syntax (FR-NEW-019), exact Then clauses, 12 added tests. Round 2: refusal line first, digit syntax, remote commit meaning, audit wording of E2E-048, two capability inputs in 14.3, 3 added tests. Round 3: FR-NEW-020 and E2E-054..056 drafted, citation fixed. Round 5: empty remote name and empty number rules, remote refused name case, E2E-057..059, traceability, 14.2. Round 4: FR-NEW-020 scope and precedence, listing outcome not re examined, seam input in 14.3, When clauses of E2E-054..056, traceability and statistics fixes.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix
### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| publishing one tag sends only that tag, takes a remote name | `crates/core/src/tools/git_tags.rs:467-505`, `crates/core/src/mcp/server.rs:3530-3538` |
| no paused operation check on publish | `crates/core/src/tools/git_tags.rs:552-600` (no call to the paused check) |
| one audit line per tag sent, none when up to date | `crates/core/src/tools/git_tags.rs:596-600` |
| refusal for commit not on the remote | `crates/core/src/tools/git_tags.rs:652` |
| no tool publishes several tags | `TOOL_CONTRACT.txt:535` (single tag tool only) |
Test command: `cargo test -p mcp-fs-core --lib e2e_tag_`

### 14.2 Class sweep
Not applicable: not a security change.

### 14.3 Design inputs
- The single publish engine (`publish_tag`, `publish_on_repo`) works on one tag under the project write lock with one listing and at most one push connection, plus one extra connection per annotated tag sharing its commit with the remote tag (`probe_identical`, `git_tags.rs:673-737`) and one more listing on a lost lease (`git_tags.rs:719`); a bulk send needs one listing and one push for many refs, with per ref rejection reporting (`PushFailure::Rejected`, see `git/remote.rs`). FR-NEW-017 asks for a connection count that does not grow with the tag count.
- Capability missing: a push of several refs with a per ref status (name and reason). `push_refs` takes one source, destination and lease and keeps a single rejection message (`git/remote.rs:596-604`, `610-677`: a single rejection cell), so a rejection cannot be attributed to a tag. Detection signal: E2E-016, E2E-033 and E2E-041 fail (the reason is attached to the wrong tag, or the whole send is refused).
- Capability missing: a test seam that applies the first N refs of a push and then fails the transport; the existing seams cover single ref deletion only (`git/remote.rs:738-751`). Detection signal: E2E-021 cannot be written without it.
- Capability missing: a test seam firing between the listing and the push of a bulk send; the existing equivalent for one tag is `git_tags.rs:645`. Detection signal: E2E-054, 055 and 056 cannot be written without it.
- The audit line for a sent tag is the one the single publish writes (`git_tags.rs:596-600`), also after a confirmed single overwrite (E2E-048).
- The preview and the send must classify tags with the same code so the counts agree; the classification (identical, differs, commit not on remote) exists per tag in the single publish engine.
- FR-NEW-017 holds only if the bulk design classifies annotated tags without a connection per tag growing past the deadline; signal: E2E-026 (25 annotated identical tags) and E2E-045/047 (50 tags) fail with a timeout refusal.
- FR-NEW-012 and 008 interact: after a dropped connection the per ref results are unknown, hence nothing is reported sent.
- A new tool or a mode of the existing one is a design choice; the tool contract golden and all count sites change if a tool is added.
- Whether the confirmation number is optional text or an integer argument is a design choice; FR-NEW-003 requires that 0 is distinguishable from "not stated".
