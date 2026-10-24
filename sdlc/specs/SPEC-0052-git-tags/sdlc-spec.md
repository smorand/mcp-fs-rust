# Git tags: functional specification

> Id: SPEC-0052
> Nature: FEAT
> Depth: L
> Depth evidence: started M (3 areas, about 12 requirements); escalated to L at audit round 1 when the count reached 18 (more than about 15); no new persisted concept (tags are already stored references)
> Status: implemented
> Generated on: 2026-10-24
> Project: existing
> From backlog: BL-0074
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
A project member can mark a version: create a tag (plain or annotated with a message), see tags with their kind and message, delete a local tag, and publish one named tag to the remote. Fetch, pull and clone now bring in the remote tags that belong to the fetched history, and never overwrite a local tag. Today tags can only be listed and never arrive from a remote, so a release cannot be marked or shared. This is the first ranked gap of the git divergence register.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today
- A member can list the tags of a project. The list shows each tag's name and the commit it points at, nothing else.
- No action creates a tag, moves a tag or deletes a tag.
- A fetch or a pull brings in the branch and no tag, on purpose; a test asserts it.
- A clone brings in the branch and records no tag.
- Publishing sends one branch to the remote; it cannot send a tag.
- While a merge, rebase, cherry pick or revert is paused, every action that changes a branch is refused with a message naming the operation in progress and how to finish or abandon it.
- Every project action by a non member is refused.

### 2.2 Specs governing this area
- SPEC-0011/FR-NEW-062: fetch takes no tag. **Deviation:** FR-MOD-002 below reverses it for tags inside the fetched history.
- SPEC-0012 listed "tag push" as out of scope. **Deviation:** this spec adds publishing one named tag. Deleting a remote tag and publishing every tag at once stay out of scope (3.2).
- SPEC-0012/FR-NEW-279 and FR-NEW-280: while an operation is paused, other ref changing actions are refused. FR-NEW-006 extends that rule to tag creation and deletion.
- SPEC-0011/FR-NEW-155 to FR-NEW-157: overwriting a remote branch needs the caller to state the remote's current commit. FR-NEW-008 applies the same rule to tags.
- No existing spec is modified.

### 2.3 What is verified today
Listing tags and the "fetch takes no tags" behaviour are exercised by existing tests; no test creates, deletes, publishes or receives a tag, because no such action exists. Details in 14.1.

## 3. Scope
### 3.1 In scope
- Create a plain tag or an annotated tag, at the checked out commit or at a named branch, tag or commit.
- Overwrite an existing tag only on explicit confirmation.
- List tags with kind and message.
- Delete a local tag.
- Publish one named tag, with overwrite on explicit confirmation.
- Receive tags on fetch, pull and clone, keeping any conflicting local tag.
- An audit entry for every create, delete and publish.

### 3.2 Out of scope, and why
- Deleting a tag on the remote: a destructive remote action, own decision (BL-0075).
- Publishing every tag in one action: invites accidental publication of private marks (BL-0076).
- Signed tags: no key management exists in the product.
- Receiving tags that point outside the fetched history: matches ordinary git behaviour and keeps fetch bounded.
- Renaming a tag: delete plus create covers it.

### 3.3 Constraints imposed by the context
- Tag names follow the same rules as branch names. Imposed by the user (2026-10-24 interview, default accepted).

## 4. Actors
- **Member**: a person who belongs to the project. Creates, deletes, publishes, lists, fetches.
- **Non member**: a person who does not belong to the project. Refused everywhere.
- **Remote**: the hosted repository the project tracks (for example a GitHub Enterprise repository).
- **Operator**: reads the audit trail.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| Versioning | tags, their kind and message, creation, deletion, listing | tag, plain tag, annotated tag, tagger |
| Remote exchange | publishing a tag, receiving tags, conflicts with local tags | publish, receive, kept local, overwrite confirmation |
| Audit | the trail of who changed a tag | audit entry |

## 5. Usage scenarios

### SC-001: Create a tag
**Actor:** member.
**Preconditions:** a project with at least one commit; no operation paused.
**Flow:**
1. The member asks for tag `v1.0`, optionally naming a target and optionally giving a message → the tag is created.
2. A tag with a message is annotated and records the member as tagger and the creation time; a tag without a message is plain.
**Postconditions:** the tag exists at the target (default: the checked out commit); one audit entry exists.
**Exceptions:**
- EXC-001a: the name is taken and overwrite is not confirmed → `Tag 'v1.0' already exists. Choose another name or confirm overwriting it.`
- EXC-001b: the name is invalid → `'my tag' is not a valid tag name: <reason>`.
- EXC-001c: the target is unknown → `Cannot tag 'nosuchref': no such branch, tag or commit.`
- EXC-001d: the project has no commit → `Cannot tag: this project has no commit yet.`
- EXC-001e: an operation is paused → the same refusal a branch change gets.
- EXC-001f: the requester is not a member → the usual refusal.
- EXC-001g: two members create the same name at once → one succeeds, the other gets EXC-001a (FR-NEW-015).
**Interactions:** SC-002, SC-004.

### SC-002: See tags
**Actor:** member.
**Preconditions:** none.
**Flow:** 1. The member lists tags → each tag shows name, commit, kind (`lightweight` or `annotated`) and, for annotated tags, the message and tagger.
**Postconditions:** nothing changes.
**Exceptions:** EXC-002a: non member → the usual refusal. EXC-002b: no tag → an empty list, not an error.
**Interactions:** works while an operation is paused.

### SC-003: Delete a local tag
**Actor:** member.
**Preconditions:** the tag exists locally; no operation paused.
**Flow:** 1. The member deletes `v1.0` → the tag disappears; commits and other tags are untouched; the remote is untouched.
**Postconditions:** one audit entry.
**Exceptions:** EXC-003a: unknown tag → `Tag 'v9.9' does not exist.` EXC-003b: operation paused → the usual refusal. EXC-003c: non member → the usual refusal. EXC-003d: two members delete at once → one succeeds, the other gets EXC-003a (FR-NEW-016).

### SC-004: Publish a tag
**Actor:** member.
**Preconditions:** the tag exists locally; the project tracks a remote; the member has a valid credential for it.
**Flow:** 1. The member publishes `v1.0` → only that tag is sent. 2. A remote tag identical to the local one is reported as `up to date`.
**Postconditions:** the remote has the tag; one audit entry, except when `up to date`.
**Exceptions:**
- EXC-004a: no such local tag → `Tag 'v9.9' does not exist.`
- EXC-004b: the remote has the name at another commit and overwrite is not confirmed → `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.`
- EXC-004c: overwrite is confirmed but the remote tag moved since the member last saw it → refused as an overwrite of a branch is, the remote untouched.
- EXC-004d: remote down, rate limited, credential expired or connection lost → the same refusal publishing a branch gives; no partial tag on the remote.
- EXC-004e: non member → the usual refusal.
- EXC-004f: the project tracks no remote → `Cannot publish tag 'v1.0': this project tracks no remote.`
- EXC-004h: the tag's commit is not on the remote → `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.`
- EXC-004g: the member states a commit that is not the remote tag's current commit → `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`

### SC-005: Receive tags
**Actor:** member.
**Preconditions:** the project tracks a remote with tags.
**Flow:** 1. The member fetches, pulls or clones → the tags pointing into the fetched branch's history arrive, and the result lists the tags `added` and the tags `kept local`.
**Postconditions:** new tags exist locally; no existing local tag moved.
**Exceptions:** EXC-005a: a local tag has the same name at another commit → kept, reported `kept local`. EXC-005b: the exchange fails → no tag arrives. EXC-005c: non member → the usual refusal. EXC-005d: a pull pauses on a merge conflict → the tags still arrive and are reported (FR-NEW-017).

## 6. Functional requirements

### 6.1 New
#### FR-NEW-001 [EARS-E]: Create a plain tag
> WHEN a member creates a tag with a name and no message, THE Versioning context SHALL create a plain tag at the checked out commit, or at the branch, tag or commit the member names.
- **Business rules:** the target keeps pointing where it pointed; naming a tag as target tags that tag's commit; moving a branch later does not move the tag.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-002 [EARS-E]: Create an annotated tag
> WHEN a member creates a tag with a message, THE Versioning context SHALL create an annotated tag recording the message exactly as given, the member as tagger and the creation time.
- **Business rules:** the message is kept byte for byte, including accents, emoji, markup and several lines; an empty message counts as no message and creates a plain tag; a message made only of spaces, tabs or line breaks is a message and creates an annotated tag recording it as given.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-003 [EARS-O]: Existing name
> IF a tag with the requested name exists and the member has not confirmed overwriting, THEN THE Versioning context SHALL refuse and change nothing.
- **User visible text:** `Tag 'v1.0' already exists. Choose another name or confirm overwriting it.`
- **Business rules:** with confirmation, the tag moves to the new target and takes the new kind (a plain tag becomes annotated if a message is given, and an annotated tag becomes plain if none is); the tag count is unchanged.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-004 [EARS-O]: Invalid name or unknown target
> IF the tag name is invalid or the target is unknown, THEN THE Versioning context SHALL refuse and create nothing.
- **User visible text:** `'<name>' is not a valid tag name: <reason>` and `Cannot tag '<target>': no such branch, tag or commit.`
- **Business rules:** the naming rules are the branch naming rules, checked in this order, the first that applies giving the reason: `it is empty`; `it is <n> bytes of UTF-8, over the 255 byte limit`; `it contains the forbidden character '<c>'` (forbidden: control characters, space, `~`, `^`, `:`, `?`, `*`, `[`, `\`); `it contains '..'`; `it begins or ends with '/'`; `one of its '/' separated components begins with '.'`; `it ends with '.lock'`. Accents and emoji are valid. When several forbidden characters are present the reason names the leftmost one; a control character is written as its escape between single quotes (`'\t'` for a tab, `'\n'` for a line break, `'\u{7f}'` for the delete character).
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-005 [EARS-E]: Delete a local tag
> WHEN a member deletes a tag, THE Versioning context SHALL remove that tag only, leaving every commit, every branch, every other tag and the remote untouched.
- **User visible text:** unknown tag → `Tag 'v9.9' does not exist.`
- **Scenario:** SC-003. **Priority:** Must

#### FR-NEW-006 [EARS-S]: Paused operation
> WHILE a merge, rebase, cherry pick or revert is paused, THE Versioning context SHALL refuse to create or delete a tag with the same refusal a branch change gets.
- **Business rules:** the refusal names the operation in progress and how to finish or abandon it. Listing tags is not refused.
- **Scenario:** SC-001, SC-003. **Priority:** Must

#### FR-NEW-007 [EARS-E]: Publish a tag
> WHEN a member publishes a named tag, THE Remote exchange context SHALL send that tag and no other.
- **Business rules:** two tags are identical when they have the same name, the same kind, the same commit and, for annotated tags, the same message (tagger and time are not compared); a remote tag that differs from the local one in any respect, kind and message included, gets the FR-NEW-008 refusal, showing the remote's commit; a tag already identical on the remote changes nothing, is reported `up to date` and leaves no audit entry; a remote that refuses the tag because of its name gives `Remote refused tag '<name>': <the remote's reason>`, leaves the remote untouched, the local tag unchanged and no audit entry; a publish that ends in a connection failure leaves no audit entry, and a later retry that finds the tag landed is `up to date`; an unknown local tag is refused with `Tag 'v9.9' does not exist.`; a failure leaves no partial tag on the remote and the local tag unchanged.
- **Scenario:** SC-004. **Priority:** Must

#### FR-NEW-008 [EARS-O]: Remote tag at another commit
> IF the remote already has the tag at a different commit and overwriting is not confirmed, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.
- **User visible text:** `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` (the commit shown is the remote's, abbreviated to 7 characters)
- **Business rules:** confirming means stating the commit the remote tag has now; if the remote tag has moved since, the overwrite is refused as an overwrite of a branch is, and the remote stays untouched.
- **Scenario:** SC-004. **Priority:** Must

#### FR-NEW-009 [EARS-E]: Receive tags on fetch and pull
> WHEN a fetch or a pull succeeds, THE Remote exchange context SHALL bring in the remote tags that point into the fetched branch's history, and report which were added.
- **Business rules:** tags pointing outside that history are not brought in; a tag identical locally (same definition as FR-NEW-007) is neither added nor reported; a remote tag that differs in any respect from a local tag of the same name is `kept local`; annotated tags keep kind, message and tagger; a tag pointing at something that is not a commit (a file or a folder) is not brought in; an annotated tag pointing at another annotated tag is brought in when the final commit is in history; a failed exchange brings no tag.
- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-010 [EARS-O]: Conflicting local tag
> IF a remote tag has the name of a local tag at a different commit, THEN THE Remote exchange context SHALL keep the local tag and report the tag as `kept local`.
- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-011 [EARS-E]: Receive tags on clone
> WHEN a project is cloned, THE Remote exchange context SHALL bring in the tags that point into the cloned branch's history.
- **Business rules:** the clone result lists the tags added and the tags kept local, as a fetch does; a remote with no tag gives two empty lists; a failed clone leaves no tag behind; a local tag of the same name at another commit is kept (FR-NEW-010).
- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-012 [EARS-E]: Audit trail
> WHEN a tag is created, overwritten, deleted or published, THE Audit context SHALL record the member and one line of text.
- **User visible text:** `created tag 'v1.0' at <full commit id>`; `overwrote tag 'v1.0' at <new full commit id> (was <old full commit id>)`; `deleted tag 'v1.0' (was at <full commit id>)`; `published tag 'v1.0' at <full commit id>`.
- **Business rules:** a refused action, a non member attempt and an `up to date` publish leave no entry; a confirmed overwrite always records an `overwrote` line, even when the new commit equals the old one; a confirmed overwrite of a name that does not exist records a `created` line.
- **Scenario:** SC-001, SC-003, SC-004. **Priority:** Must

#### FR-NEW-013 [EARS-O]: Project with no commit
> IF the project has no commit and the member creates a tag without a target, THEN THE Versioning context SHALL refuse with `Cannot tag: this project has no commit yet.`
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-017 [EARS-O]: Pull that pauses on a conflict
> IF a pull completes its exchange with the remote but pauses on a merge conflict, THEN THE Remote exchange context SHALL still bring in the tags of FR-NEW-009 and report them as added or kept local.
- **Business rules:** a pull whose exchange with the remote fails brings no tag; a paused merge does not undo the received tags.
- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-018 [EARS-O]: Publish edge cases
> IF the project tracks no remote when a member publishes a tag, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag 'v1.0': this project tracks no remote.`
> IF a member confirms an overwrite while the remote has no tag of that name, THEN THE Remote exchange context SHALL publish the tag as a creation.
> IF a member confirms an overwrite while the remote tag is identical to the local one, THEN THE Remote exchange context SHALL report `up to date` and leave no audit entry.
> IF a member confirms an overwrite without stating the remote tag's current commit while the remote has the tag at a different commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` and leave the remote untouched.
> IF the commit the member states is not the remote tag's current commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` and leave the remote untouched.
- **Business rules:** the commit shown is the remote's, abbreviated to 7 characters; the refusals are given in the order of FR-NEW-020: no remote first, then FR-NEW-019, then the overwrite refusals of this requirement and of FR-NEW-008.
- **Scenario:** SC-004. **Priority:** Must

#### FR-NEW-021 [EARS-U]: Target resolution
> WHEN a member names a target, THE Versioning context SHALL resolve it in this order: a branch of that exact name, then a tag of that exact name, then a commit whose full id equals it, then the only commit whose id begins with it when the text is at least 7 characters.
- **Business rules:** a name that is both a branch and a tag resolves to the branch; a prefix of 7 or more characters matching several commits, or any text under 7 characters that is not a branch or tag, is an unknown target and gets `Cannot tag '<target>': no such branch, tag or commit.`
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-019 [EARS-O]: Commit not on the remote
> IF the commit a member publishes a tag for is not on the remote, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` and leave the remote untouched.
- **Business rules:** a commit is on the remote when it is a branch tip the remote advertises, or an ancestor of such a tip in the history the project already holds; a publish never fetches, so a commit the remote has only through history the project has not fetched is refused; publishing a tag never sends commits; the commit shown is the tag's, abbreviated to 7 characters.
- **Scenario:** SC-004. **Priority:** Must

#### FR-NEW-020 [EARS-U]: Order of refusals
> WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list.
- **Business rules:** creating a tag: not a member (FR-NEW-014); paused operation (FR-NEW-006); invalid name (FR-NEW-004); project has no commit (FR-NEW-013); unknown target (FR-NEW-004); existing name (FR-NEW-003). Deleting a tag: not a member; paused operation; unknown tag (FR-NEW-005). Publishing a tag: not a member; unknown local tag; project tracks no remote (FR-NEW-018); commit not on the remote (FR-NEW-019); remote tag differs, has moved or was confirmed without a stated commit (FR-NEW-008, FR-NEW-018); then connection failures.
- **Scenario:** SC-001, SC-003, SC-004. **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-U]: Listing shows kind and message (modifies the existing tag listing)
> The Versioning context SHALL show, for each listed tag, its name, its commit, its kind (`lightweight` or `annotated`) and, for an annotated tag, its message and tagger.
- **Before:** name and commit only (Section 2.1). **After:** the statement above; name and commit are unchanged so existing readers keep working; for an annotated tag the tagger is shown as the member's identifier exactly as it appears on that member's commits, and the creation time in UTC as `YYYY-MM-DDTHH:MM:SSZ`; tags are listed in ascending order of name, compared byte by byte. **Reason:** a tag's meaning is its message.

#### FR-MOD-002 [EARS-E]: Fetch takes tags (modifies SPEC-0011/FR-NEW-062)
> WHEN a fetch succeeds, THE Remote exchange context SHALL apply FR-NEW-009 and FR-NEW-010.
- **Before:** no tag ever follows a fetch. **After:** in history tags follow; branches behave as before. **Reason:** shared tags are the point of a release mark.

## 7. Non functional requirements (user observable)
### 7.1 Performance
No new time target: listing a project holding 5000 tags returns all of them (E2E-022). Design measures the baseline; a threshold is a follow up if the owner wants one.
### 7.2 Security and access
#### FR-NEW-014 [EARS-O]
> IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.
### 7.3 Privacy and compliance
Not applicable: tags hold names and messages the member typed; no new personal data class.
### 7.4 Usability and accessibility
Not applicable: no screen is added.
### 7.5 Reliability
#### FR-NEW-015 [EARS-E]: Concurrent creation
> WHEN two members create the same tag name at the same time, THE Versioning context SHALL let exactly one succeed and give the other the FR-NEW-003 refusal.
#### FR-NEW-016 [EARS-E]: Concurrent deletion
> WHEN two members delete the same tag at the same time, THE Versioning context SHALL let exactly one succeed and give the other the FR-NEW-005 refusal.
### 7.6 Operability
FR-NEW-012 is the operator's view: who, which tag, which commit.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | no tag; no commit; delete last tag | empty list; FR-NEW-013 refusal; second delete refused | E2E-014, 021, 028, 047, 053, 067 |
| Boundary values | name of 255 and 256 bytes; 0, 1, many tags | 255 accepted, 256 refused | E2E-011, 021, 022, 048 |
| Concurrent actions | same name created or deleted twice at once; different names at once | exactly one succeeds; different names both succeed | E2E-016, 032, 061, 065, 066, 068 |
| Slow or lost connectivity | connection lost while publishing | usual refusal, retry works | E2E-039, 047 |
| Access states | non member on each action | usual refusal | E2E-015, 023, 030, 040, 049 |
| Data variations | accents, emoji, markup, several line message; unicode names; tags on a folder or on a tag | kept verbatim; non commit tags skipped | E2E-005, 013, 025, 062 |
| Navigation | not applicable: no screen or link |  |  |
| Partial operations | failed fetch, pull, publish or clone; pull paused on a conflict | no partial tag; tags still arrive on a paused pull | E2E-038, 039, 045, 047, 054, 063 |
| Dependency failures | remote down, rate limited, credential expired | the refusal branches get | E2E-038, 045 |
| Ordering and timing | remote tag moved between two fetches; remote tag moved before an overwrite; listing order | local kept; overwrite refused; ascending by name | E2E-025, 037, 050, 057, 064 |
| Exhaustion | thousands of tags | all listed and received | E2E-022, 048, 059 |

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Users: Alice and Bob are members of `proj-a`; Carol is not. Remote: a GitHub Enterprise repository tracked by `proj-a`. `main` is checked out at `a1b2c3d`; `dev` is at `b2c3d4e`; other commits `c3d4e5f`, `d4e5f6a`, `e5f6a7b`.

### 9.1 Summary
Coverage by primary category (recounted from the table): journey happy 11, failure 31, security 7 (all refusals), edge 22, state transition 11, side effect 4, cross scenario 1; total 87 = 85 new plus 2 modified, 0 removed. Happy to failure ratio 1:3.5 (11 happy against 31 failure plus 7 security refusals), which beats 1:1.

| Test | Action | Primary category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey happy | SC-001 | FR-NEW-001 | Critical |
| E2E-002 | New | journey happy | SC-001 | FR-NEW-001 | High |
| E2E-003 | New | journey happy | SC-001 | FR-NEW-001 | Medium |
| E2E-004 | New | journey happy | SC-001 | FR-NEW-002 | Critical |
| E2E-005 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-006 | New | failure | SC-001 | FR-NEW-003, 012 | Critical |
| E2E-007 | New | state transition | SC-001 | FR-NEW-003, 001 | High |
| E2E-008 | New | state transition | SC-001 | FR-NEW-003, 002 | Medium |
| E2E-009 | New | failure | SC-001 | FR-NEW-004 | High |
| E2E-010 | New | failure | SC-001 | FR-NEW-004 | High |
| E2E-011 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-012 | New | failure | SC-001 | FR-NEW-004 | High |
| E2E-013 | New | edge | SC-001 | FR-NEW-004, 001 | Low |
| E2E-014 | New | failure | SC-001 | FR-NEW-013 | Medium |
| E2E-015 | New | security | SC-001 | FR-NEW-014, 001 | Critical |
| E2E-016 | New | failure | SC-001 | FR-NEW-015, 003 | Critical |
| E2E-017 | New | state transition | SC-001 | FR-NEW-006 | High |
| E2E-018 | New | state transition | SC-001 | FR-NEW-006 | Medium |
| E2E-019 | New | side effect | SC-001 | FR-NEW-012 | High |
| E2E-020 | New | journey happy | SC-002 | FR-MOD-001 | Critical |
| E2E-021 | New | edge | SC-002 | FR-MOD-001 | Medium |
| E2E-022 | New | edge | SC-002 | FR-MOD-001 | High |
| E2E-023 | New | security | SC-002 | FR-NEW-014 | High |
| E2E-024 | New | state transition | SC-002 | FR-NEW-006, FR-MOD-001 | Medium |
| E2E-026 | New | journey happy | SC-003 | FR-NEW-005 | Critical |
| E2E-027 | New | failure | SC-003 | FR-NEW-005 | High |
| E2E-028 | New | edge | SC-003 | FR-NEW-005 | Medium |
| E2E-029 | New | state transition | SC-003 | FR-NEW-006 | High |
| E2E-030 | New | security | SC-003 | FR-NEW-014, 005 | Critical |
| E2E-031 | New | side effect | SC-003 | FR-NEW-005, 012 | High |
| E2E-032 | New | failure | SC-003 | FR-NEW-016, 005 | Medium |
| E2E-033 | New | journey happy | SC-004 | FR-NEW-007 | Critical |
| E2E-034 | New | failure | SC-004 | FR-NEW-007 | High |
| E2E-035 | New | edge | SC-004 | FR-NEW-007, 012 | High |
| E2E-036 | New | failure | SC-004 | FR-NEW-008 | Critical |
| E2E-037 | New | state transition | SC-004 | FR-NEW-008 | High |
| E2E-038 | New | failure | SC-004 | FR-NEW-007 | High |
| E2E-039 | New | failure | SC-004 | FR-NEW-007 | Medium |
| E2E-040 | New | security | SC-004 | FR-NEW-014, 012, 007 | Critical |
| E2E-041 | New | journey happy | SC-005 | FR-NEW-009, FR-MOD-002 | Critical |
| E2E-042 | New | edge | SC-005 | FR-NEW-009, FR-MOD-002 | High |
| E2E-043 | New | failure | SC-005 | FR-NEW-010, 009 | Critical |
| E2E-044 | New | edge | SC-005 | FR-NEW-010, 009 | Medium |
| E2E-045 | New | failure | SC-005 | FR-NEW-009 | High |
| E2E-046 | New | journey happy | SC-005 | FR-NEW-011 | Critical |
| E2E-047 | New | failure | SC-005 | FR-NEW-011 | High |
| E2E-048 | New | edge | SC-005 | FR-NEW-009 | Medium |
| E2E-049 | New | security | SC-005 | FR-NEW-014, 009 | High |
| E2E-050 | New | state transition | SC-005 | FR-NEW-009, 010 | High |
| E2E-051 | New | cross scenario | SC-001, 004, 005 | FR-NEW-002, 007, 009, 012 | High |
| E2E-052 | New | journey happy | SC-001 | FR-NEW-013, 001 | Medium |
| E2E-053 | New | failure | SC-003 | FR-NEW-005 | Medium |
| E2E-025 | New | edge | SC-002 | FR-MOD-001 | Medium |
| E2E-054 | New | edge | SC-005 | FR-NEW-017, FR-NEW-009 | High |
| E2E-055 | New | failure | SC-004 | FR-NEW-018 | High |
| E2E-056 | New | edge | SC-004 | FR-NEW-018, FR-NEW-007 | Medium |
| E2E-057 | New | failure | SC-004 | FR-NEW-018, FR-NEW-008 | High |
| E2E-058 | New | security | SC-004 | FR-NEW-008, FR-NEW-014 | High |
| E2E-059 | New | edge | SC-005 | FR-NEW-011 | Medium |
| E2E-060 | New | failure | SC-005 | FR-NEW-011, FR-NEW-010 | High |
| E2E-061 | New | edge | SC-001 | FR-NEW-015 | Medium |
| E2E-062 | New | edge | SC-005 | FR-NEW-009 | Medium |
| E2E-063 | New | failure | SC-005 | FR-NEW-017, FR-NEW-009 | High |
| E2E-064 | New | failure | SC-005 | FR-NEW-017, FR-NEW-010 | High |
| E2E-065 | New | edge | SC-003 | FR-NEW-016 | Medium |
| E2E-066 | New | side effect | SC-003 | FR-NEW-016, FR-NEW-012 | Medium |
| E2E-067 | New | failure | SC-001 | FR-NEW-013 | Medium |
| E2E-068 | New | state transition | SC-001 | FR-NEW-015, FR-NEW-003 | Medium |
| E2E-069 | New | failure | SC-004 | FR-NEW-007, FR-NEW-008 | High |
| E2E-070 | New | failure | SC-001 | FR-NEW-020 | High |
| E2E-071 | New | failure | SC-004 | FR-NEW-019 | High |
| E2E-072 | New | journey happy | SC-004 | FR-NEW-019 | Medium |
| E2E-073 | New | edge | SC-004 | FR-NEW-019 | Medium |
| E2E-074 | New | security | SC-004 | FR-NEW-020, FR-NEW-014 | Medium |
| E2E-075 | New | failure | SC-004 | FR-NEW-020, FR-NEW-007 | Medium |
| E2E-076 | New | failure | SC-005 | FR-NEW-010, FR-NEW-009 | Medium |
| E2E-077 | New | edge | SC-004 | FR-NEW-007, FR-NEW-012 | Medium |
| E2E-078 | New | edge | SC-001 | FR-NEW-002, FR-NEW-001 | Medium |
| E2E-079 | New | side effect | SC-001 | FR-NEW-012, FR-NEW-003 | Medium |
| E2E-080 | New | edge | SC-001 | FR-NEW-021, FR-NEW-001 | Medium |
| E2E-081 | New | failure | SC-001 | FR-NEW-021, FR-NEW-004 | Medium |
| E2E-082 | New | failure | SC-001 | FR-NEW-021, FR-NEW-004 | Medium |
| E2E-083 | New | failure | SC-004 | FR-NEW-018, FR-NEW-008 | High |
| E2E-084 | New | edge | SC-004 | FR-NEW-019 | Medium |
| E2E-085 | New | failure | SC-004 | FR-NEW-007 | Medium |
| E2E-MOD-001 | Modified | state transition | SC-002 | FR-MOD-001 | High |
| E2E-MOD-002 | Modified | state transition | SC-005 | FR-MOD-002, FR-NEW-009 | Critical |

### 9.2 New tests

#### E2E-001: Create a plain tag at the checked out commit
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** Alice is a member of `proj-a`, `main` is checked out at `a1b2c3d`, no tag exists. **When** Alice creates `v1.0` with no target and no message. **Then** `v1.0` exists at `a1b2c3d`. **And** the list shows `v1.0`, commit `a1b2c3d`, kind `lightweight`, no message.
- **Priority:** Critical

#### E2E-002: Create a tag at a named branch
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** `dev` is at `b2c3d4e`, `main` checked out at `a1b2c3d`. **When** Alice creates `release-2026` targeting `dev`. **Then** `release-2026` is at `b2c3d4e`. **And** the checked out commit is still `a1b2c3d`.
- **Priority:** High

#### E2E-003: Create a tag at a tag or a commit
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** `v1.0` is at `a1b2c3d`. **When** Alice creates `v1.0-copy` targeting `v1.0`, then `v1.1` targeting `c3d4e5f`. **Then** `v1.0-copy` is at `a1b2c3d` and `v1.1` at `c3d4e5f`. **And** both are `lightweight`.
- **Priority:** Medium

#### E2E-004: Create an annotated tag
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** Alice is a member, `main` at `a1b2c3d`. **When** Alice creates `v2.0` with message `Release 2.0`. **Then** the list shows `v2.0`, kind `annotated`, message `Release 2.0`, commit `a1b2c3d`. **And** the tagger is Alice, shown as her identifier on her commits, `alice@example.com`, and the creation time is shown in UTC as `YYYY-MM-DDTHH:MM:SSZ` within 5 seconds of now.
- **Priority:** Critical

#### E2E-005: Message kept verbatim
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** Alice is a member. **When** Alice creates `v2.1` with a two line message whose first line is `Été 🚀 <b>gras</b> & "quotes"`. **Then** the list returns both lines exactly, with no escaping and no truncation.
- **Priority:** Medium

#### E2E-006: Existing name refused without confirmation
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-012
- **Given** `v1.0` is at `a1b2c3d` and `main` has moved to `c3d4e5f`. **When** Alice creates `v1.0` without confirming. **Then** the refusal is `Tag 'v1.0' already exists. Choose another name or confirm overwriting it.` **And** `v1.0` is still at `a1b2c3d` and no audit entry was added.
- **Priority:** Critical

#### E2E-007: Overwrite confirmed moves the tag
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** `v1.0` at `a1b2c3d`, `main` at `c3d4e5f`. **When** Alice creates `v1.0` confirming the overwrite. **Then** `v1.0` is at `c3d4e5f`. **And** the number of tags is unchanged.
- **Priority:** High

#### E2E-008: Overwrite changes the kind
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-002
- **Given** plain `v1.0` exists. **When** Bob creates `v1.0` with message `Re-cut` confirming the overwrite. **Then** `v1.0` is `annotated`, message `Re-cut`, tagger Bob.
- **Priority:** Medium

#### E2E-009: Name with a space
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** Alice creates `my tag`. **Then** the refusal is `'my tag' is not a valid tag name: it contains the forbidden character ' '`. **And** for the name `a b~` the reason names the space (the leftmost forbidden character), and for a name holding a tab between `a` and `b` the reason is `it contains the forbidden character '\t'`. **And** no tag is created.
- **Priority:** High

#### E2E-010: Other invalid names
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** she tries, one at a time, an empty name, `a..b`, `/v1`, `v1/`, `rel/.hidden`, `v1.lock`. **Then** each is refused with `'<name>' is not a valid tag name: <reason>` where the reasons are, in order: `it is empty`; `it contains '..'`; `it begins or ends with '/'` (for `/v1` and for `v1/`); `one of its '/' separated components begins with '.'`; `it ends with '.lock'`. **And** the list is unchanged after each attempt.
- **Priority:** High

#### E2E-011: Name length boundary
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** she creates a name of `v` plus 254 letters `a` (255 bytes), then a name of 256 bytes, then a name of 100 accented letters `é` (200 bytes) preceded by 101 letters `a` (301 bytes). **Then** the first is created; the second is refused with `it is 256 bytes of UTF-8, over the 255 byte limit` and the third with `it is 301 bytes of UTF-8, over the 255 byte limit`. **And** only one tag exists.
- **Priority:** Medium

#### E2E-012: Unknown target
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** nothing is named `nosuchref`. **When** Alice creates `v3.0` targeting `nosuchref`. **Then** the refusal is `Cannot tag 'nosuchref': no such branch, tag or commit.` **And** `v3.0` does not exist.
- **Priority:** High

#### E2E-013: Unicode names accepted
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, FR-NEW-001
- **Given** Alice is a member. **When** she creates `été-2026` and `rel/2026-🚀`. **Then** both are created and listed with exactly those names.
- **Priority:** Low

#### E2E-014: Project with no commit
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** `proj-empty` has no commit and Alice is a member. **When** Alice creates `v1.0` with no target. **Then** the refusal is `Cannot tag: this project has no commit yet.` **And** no tag exists.
- **Priority:** Medium

#### E2E-015: Non member cannot create
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-014, FR-NEW-001
- **Given** Carol is not a member of `proj-a`. **When** Carol creates `v1.0`. **Then** she receives the refusal she gets for any other project action. **And** no tag exists and no audit entry was added.
- **Priority:** Critical

#### E2E-016: Concurrent creation
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-003
- **Given** `v5.0` does not exist. **When** Alice (target `a1b2c3d`) and Bob (target `b2c3d4e`) create `v5.0` at the same time without confirming. **Then** exactly one succeeds and the other receives `Tag 'v5.0' already exists. Choose another name or confirm overwriting it.` **And** `v5.0` is at the winner's target and exactly one creation entry exists.
- **Priority:** Critical

#### E2E-017: Creation refused during a paused merge
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** a merge of `dev` into `main` is paused on a conflict. **When** Alice creates `v1.0`. **Then** the refusal is the one a branch change gets: it names the merge in progress and how to finish or abandon it. **And** no tag is created.
- **Priority:** High

#### E2E-018: Creation refused during other paused operations
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** a paused rebase, then separately a paused cherry pick, then a paused revert. **When** Bob creates `v1.0` in each case. **Then** each refusal names its own operation. **And** after the operation is abandoned, the same creation succeeds.
- **Priority:** Medium

#### E2E-019: Audit entry on creation and overwrite
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** Alice is a member. **When** she creates `v1.0` at `a1b2c3d`, then overwrites it to `c3d4e5f`. **Then** two entries exist, both by Alice: `created tag 'v1.0' at <full id of a1b2c3d>` and `overwrote tag 'v1.0' at <full id of c3d4e5f> (was <full id of a1b2c3d>)`.
- **Priority:** High

#### E2E-020: List shows kind and message
- **Category:** journey (happy). **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** plain `v1.0` at `a1b2c3d` and annotated `v2.0` at `c3d4e5f` with message `Release 2.0`. **When** Bob lists tags. **Then** two entries appear: `v1.0` `lightweight` with no message; `v2.0` `annotated` with message `Release 2.0`. **And** the entries appear in the order `v1.0`, `v2.0`.
- **Priority:** Critical

#### E2E-021: Zero and one tag
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** `proj-a` has no tag. **When** Alice lists tags, creates `v1.0`, lists again. **Then** the first list is empty with no error and the second holds exactly `v1.0`.
- **Priority:** Medium

#### E2E-022: 5000 tags
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** `proj-a` holds 5000 tags, plain and annotated. **When** Alice lists tags. **Then** all 5000 are returned with kind and message, in ascending name order.
- **Priority:** High

#### E2E-023: Non member cannot list
- **Category:** security. **Scenario:** SC-002. **Requirements:** FR-NEW-014
- **Given** Carol is not a member and `v1.0` exists. **When** Carol lists tags of `proj-a`. **Then** she receives the usual refusal. **And** her answer contains no tag name.
- **Priority:** High

#### E2E-024: Listing works during a paused operation
- **Category:** state transition. **Scenario:** SC-002. **Requirements:** FR-NEW-006, FR-MOD-001
- **Given** a merge is paused and `v1.0` exists. **When** Alice lists tags. **Then** `v1.0` is shown with its kind.
- **Priority:** Medium

#### E2E-026: Delete a local tag
- **Category:** journey (happy). **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** tags `v1.0` (`a1b2c3d`), `v2.0` (`c3d4e5f`), `v3.0`. **When** Alice deletes `v1.0`. **Then** the list shows `v2.0` and `v3.0` only. **And** commit `a1b2c3d` is still readable and `main` is unchanged.
- **Priority:** Critical

#### E2E-027: Delete an unknown tag
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** no `v9.9`. **When** Alice deletes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** the list is unchanged.
- **Priority:** High

#### E2E-028: Delete the last tag, twice
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** only `v1.0` exists. **When** Alice deletes it, then deletes it again. **Then** the first succeeds and the list is empty; the second is refused with `Tag 'v1.0' does not exist.`
- **Priority:** Medium

#### E2E-029: Deletion refused during a paused operation
- **Category:** state transition. **Scenario:** SC-003. **Requirements:** FR-NEW-006
- **Given** a cherry pick is paused and `v1.0` exists. **When** Bob deletes `v1.0`. **Then** the refusal names the cherry pick in progress and how to finish or abandon it. **And** `v1.0` still exists.
- **Priority:** High

#### E2E-030: Non member cannot delete
- **Category:** security. **Scenario:** SC-003. **Requirements:** FR-NEW-014, FR-NEW-005
- **Given** Carol is not a member and `v1.0` exists. **When** Carol deletes `v1.0`. **Then** she receives the usual refusal. **And** `v1.0` remains and no audit entry was added.
- **Priority:** Critical

#### E2E-031: Deletion is local and audited
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-005, FR-NEW-012
- **Given** `v1.0` at `a1b2c3d` exists locally and on the remote. **When** Alice deletes `v1.0`. **Then** the remote still has `v1.0`. **And** an audit entry by Alice reads `deleted tag 'v1.0' (was at <full id of a1b2c3d>)`.
- **Priority:** High

#### E2E-032: Concurrent deletion
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-016, FR-NEW-005
- **Given** `v1.0` and `v2.0` exist. **When** Alice and Bob delete `v1.0` at the same time. **Then** one succeeds and the other receives `Tag 'v1.0' does not exist.` **And** `v2.0` is untouched.
- **Priority:** Medium

#### E2E-033: Publish one tag only
- **Category:** journey (happy). **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v1.0` (`a1b2c3d`) and `v2.0` (`c3d4e5f`); the remote has neither. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `a1b2c3d`. **And** the remote has no `v2.0`.
- **Priority:** Critical

#### E2E-034: Publish an unknown tag
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** no local `v9.9`. **When** Alice publishes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** the remote is untouched.
- **Priority:** High

#### E2E-035: Publish an identical tag
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** the remote and the local project both have `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** the result says `up to date`. **And** the remote is unchanged and no audit entry was added.
- **Priority:** High

#### E2E-036: Remote tag at another commit, no confirmation
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes `v1.0` without confirming. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** Critical

#### E2E-037: Confirmed overwrite, and a moved remote tag
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming the overwrite and stating `f6a7b8c`. **Then** the remote `v1.0` is at `a1b2c3d`. **And** if the remote tag had first been moved to `0a1b2c3`, the same request is refused and the remote stays at `0a1b2c3`.
- **Priority:** High

#### E2E-038: Remote unavailable, rate limited or credential expired
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v1.0` exists. **When** Alice publishes while the remote is down, then while it rate limits, then with an expired credential. **Then** each is refused with the refusal publishing a branch gives for that cause. **And** the remote has no partial tag and the local tag is unchanged.
- **Priority:** High

#### E2E-039: Connection lost while publishing
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v2.0` exists. **When** the connection drops during the publish before the tag reaches the remote, then Alice publishes again. **Then** the first attempt, which dropped before the tag reached the remote, gives the usual connection refusal; the second succeeds. **And** the remote never holds a partial tag and one audit entry exists.
- **Priority:** Medium

#### E2E-040: Non member cannot publish; the audit names the publisher
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-014, FR-NEW-012, FR-NEW-007
- **Given** Carol is not a member; `v1.0` exists locally. **When** Carol publishes `v1.0`, then Alice publishes it. **Then** Carol receives the usual refusal and the remote is untouched. **And** exactly one audit entry exists, by Alice, reading `published tag 'v1.0' at <full id of a1b2c3d>`.
- **Priority:** Critical

#### E2E-041: Fetch brings in history tags
- **Category:** journey (happy). **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote `main` has `v1.0` (`a1b2c3d`) and `v1.1` (`c3d4e5f`) in its history; the project has neither. **When** Alice fetches `main`. **Then** both tags exist locally. **And** the result lists `v1.0` and `v1.1` as added.
- **Priority:** Critical

#### E2E-042: Tags outside the history are ignored
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has `v9.0` at `e5f6a7b` reachable only from branch `experimental`. **When** Alice fetches `main`. **Then** `v9.0` does not exist locally. **And** the result does not mention it.
- **Priority:** High

#### E2E-043: Pull keeps a conflicting local tag
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local `v1.0` at `a1b2c3d`; remote `v1.0` at `c3d4e5f`. **When** Alice pulls `main`. **Then** the pull succeeds and local `v1.0` stays at `a1b2c3d`. **And** the result lists `v1.0` as kept local.
- **Priority:** Critical

#### E2E-044: Same name at the same commit
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local and remote `v1.0` are both at `a1b2c3d`. **When** Alice fetches. **Then** `v1.0` is neither added nor kept local in the result. **And** nothing changes.
- **Priority:** Medium

#### E2E-045: Failed fetch brings no tag
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has a new tag `v3.0` in history. **When** Alice fetches while the remote is down, then rate limited, then with an expired credential. **Then** each gives the usual fetch refusal. **And** `v3.0` does not exist locally.
- **Priority:** High

#### E2E-046: Clone brings in history tags
- **Category:** journey (happy). **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** the remote `main` has `v1.0` and annotated `v2.0` (message `Release 2.0`) in its history, and `v9.0` only on `experimental`. **When** Alice clones it into `proj-new`. **Then** `proj-new` lists `v1.0` and `v2.0` with the right kind and message. **And** `v9.0` is absent. **And** the clone result lists `v1.0` and `v2.0` as added and nothing as kept local.
- **Priority:** Critical

#### E2E-047: Clone with no tag, and a failed clone
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** remote A has no tag; remote B goes down during the clone. **When** Alice clones A, then B. **Then** A succeeds with an empty tag list; B is refused with the usual clone refusal. **And** B leaves no tag behind.
- **Priority:** High

#### E2E-048: 2000 tags
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote `main` history holds 2000 tags, some annotated with messages. **When** Alice fetches. **Then** all 2000 arrive, annotated ones keeping kind, message and tagger. **And** all 2000 are listed as added.
- **Priority:** Medium

#### E2E-049: Non member cannot receive
- **Category:** security. **Scenario:** SC-005. **Requirements:** FR-NEW-014, FR-NEW-009
- **Given** Carol is not a member. **When** Carol fetches, then pulls. **Then** each gets the usual refusal. **And** no tag arrives in `proj-a`.
- **Priority:** High

#### E2E-050: Remote tag moved between two fetches
- **Category:** state transition. **Scenario:** SC-005. **Requirements:** FR-NEW-009, FR-NEW-010
- **Given** Alice fetched and has `v1.0` at `a1b2c3d`; the remote then moves `v1.0` to `c3d4e5f`. **When** Alice fetches again. **Then** local `v1.0` stays at `a1b2c3d`. **And** the result lists it as kept local and no other tag changed.
- **Priority:** High

#### E2E-051: Create, publish, receive round trip
- **Category:** cross scenario. **Scenario:** SC-001, SC-004, SC-005. **Requirements:** FR-NEW-002, 007, 009, 012
- **Given** Alice and Bob each have a clone of the project. **When** Alice creates annotated `release-2026` with message `RC`, publishes it, and Bob fetches. **Then** Bob's list shows `release-2026`, `annotated`, message `RC`, same commit. **And** the audit shows Alice's creation and publication.
- **Priority:** High

#### E2E-052: Tagging works after the first commit
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-013, FR-NEW-001
- **Given** `proj-empty` has no commit. **When** Alice commits her first file at `f0a1b2c`, then creates `v0.1` with no target. **Then** `v0.1` exists at `f0a1b2c`.
- **Priority:** Medium

#### E2E-053: Delete in a project with no commit
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** `proj-empty` has no commit and no tag. **When** Alice deletes `v1.0`. **Then** the refusal is `Tag 'v1.0' does not exist.` **And** nothing changes.
- **Priority:** Medium

#### E2E-025: Listing order
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** tags `v2.0`, `v10.0`, `Alpha`, `été`, `v1.0` exist. **When** Bob lists tags. **Then** the order is `Alpha`, `v1.0`, `v10.0`, `v2.0`, `été` (ascending, compared byte by byte).
- **Priority:** Medium

#### E2E-054: Pull paused on a conflict still receives tags
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-009
- **Given** the remote `main` has `v1.0` (`c3d4e5f`) in its history and a change that conflicts with Alice's local change. **When** Alice pulls `main`. **Then** the pull ends paused on the conflict. **And** `v1.0` exists locally at `c3d4e5f` and the result lists it as added.
- **Priority:** High

#### E2E-055: Publish with no remote
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018
- **Given** `proj-local` tracks no remote and has `v1.0`. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': this project tracks no remote.` **And** no audit entry was added.
- **Priority:** High

#### E2E-056: Confirmed overwrite where the remote has no such tag, or an identical one
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-007
- **Given** local `v1.0` at `a1b2c3d`; the remote has no `v1.0`. **When** Alice publishes confirming an overwrite and stating `f6a7b8c`, then publishes again the same way. **Then** the first publish creates `v1.0` at `a1b2c3d` on the remote; the second says `up to date`. **And** exactly one audit entry exists.
- **Priority:** Medium

#### E2E-057: Stated commit differs from the remote tag's commit
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming an overwrite and stating `0a1b2c3`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` **And** the remote stays at `f6a7b8c`.
- **Priority:** High

#### E2E-058: Non member cannot overwrite a remote tag
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-008, FR-NEW-014
- **Given** Carol is not a member; the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Carol publishes `v1.0` confirming an overwrite and stating `f6a7b8c`. **Then** she receives the usual refusal. **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** High

#### E2E-059: Clone with many tags
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** the remote `main` history holds 500 tags, 100 of them annotated with messages. **When** Alice clones it into `proj-new`. **Then** `proj-new` lists 500 tags, the annotated ones keeping kind, message and tagger. **And** the clone result lists 500 as added.
- **Priority:** Medium

#### E2E-060: Clone meets a conflicting local tag
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-011, FR-NEW-010
- **Given** `proj-new` already has `v1.0` at `a1b2c3d`; the remote `v1.0` is at `c3d4e5f`. **When** Alice clones the remote into `proj-new`. **Then** local `v1.0` stays at `a1b2c3d`. **And** the clone result lists `v1.0` as kept local.
- **Priority:** High

#### E2E-061: Concurrent creation of different names
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** neither `v6.0` nor `v6.1` exists. **When** Alice creates `v6.0` and Bob creates `v6.1` at the same time. **Then** both exist. **And** exactly two creation entries exist.
- **Priority:** Medium

#### E2E-062: Tags that do not point at a commit
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has `t-tree` pointing at a folder, and `v8.0` annotated pointing at another annotated tag whose commit is in `main`'s history. **When** Alice fetches `main`. **Then** `v8.0` arrives. **And** `t-tree` does not, and the result does not mention it.
- **Priority:** Medium

#### E2E-063: Failed pull brings no tag
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-009
- **Given** the remote has a new tag `v3.0` in history. **When** Alice pulls while the remote is down, then with an expired credential. **Then** each gives the usual pull refusal. **And** `v3.0` does not exist locally.
- **Priority:** High

#### E2E-064: Paused pull keeps a conflicting local tag
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-010
- **Given** local `v1.0` at `a1b2c3d`; remote `v1.0` at `c3d4e5f`; the pull pauses on a conflict. **When** Alice pulls `main`. **Then** local `v1.0` stays at `a1b2c3d`. **And** the result lists `v1.0` as kept local.
- **Priority:** High

#### E2E-065: Concurrent deletion of different tags
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-016
- **Given** `v1.0` and `v2.0` exist. **When** Alice deletes `v1.0` and Bob deletes `v2.0` at the same time. **Then** both are deleted and the list is empty.
- **Priority:** Medium

#### E2E-066: One audit entry for a concurrent deletion
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-016, FR-NEW-012
- **Given** `v1.0` exists. **When** Alice and Bob delete `v1.0` at the same time. **Then** exactly one deletion entry exists. **And** it names the member whose deletion succeeded.
- **Priority:** Medium

#### E2E-067: Annotated tag in a project with no commit
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** `proj-empty` has no commit. **When** Alice creates `v1.0` with message `First` and no target. **Then** the refusal is `Cannot tag: this project has no commit yet.` **And** no tag exists.
- **Priority:** Medium

#### E2E-068: The loser of a concurrent creation can retry under another name
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-003
- **Given** Alice and Bob created `v5.0` at the same time and Bob received the already exists refusal. **When** Bob creates `v5.1` at `b2c3d4e`. **Then** `v5.1` exists at `b2c3d4e`. **And** `v5.0` is unchanged.
- **Priority:** Medium

#### E2E-069: Remote tag differs only by kind
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-008
- **Given** local annotated `v1.0` (message `A`) at `a1b2c3d`; the remote has plain `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0` without confirming. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (a1b2c3d). Confirm overwriting to replace it.` **And** the remote is untouched.
- **Priority:** High

#### E2E-070: Paused operation outranks an invalid name and unknown target
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-020
- **Given** a merge is paused and Alice is a member. **When** she creates `my tag` targeting `nosuchref`. **Then** she gets the paused merge refusal only. **And** no tag is created.
- **Priority:** High

#### E2E-071: Tag on a commit the remote lacks
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `v1.0` is at `a1b2c3d`, a commit on local branch `dev` never published. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` **And** the remote has no `v1.0` and no new commit.
- **Priority:** High

#### E2E-072: Tag on a published commit
- **Category:** journey (happy). **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `dev` was published and `v1.0` is at its tip `b2c3d4e`. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `b2c3d4e`. **And** the remote gained no other reference.
- **Priority:** Medium

#### E2E-073: Commit reachable from another remote branch
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `v1.0` is at `c3d4e5f`, which belongs to the history of remote branch `release` but not of the tracked branch. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `c3d4e5f`.
- **Priority:** Medium

#### E2E-074: Non member outranks every other publish refusal
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-020, FR-NEW-014
- **Given** `proj-local` tracks no remote; Carol is not a member. **When** Carol publishes `v9.9`. **Then** she receives the usual refusal, not `Tag 'v9.9' does not exist.` **And** nothing about the project is revealed.
- **Priority:** Medium

#### E2E-075: Unknown tag outranks no remote
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-020, FR-NEW-007
- **Given** `proj-local` tracks no remote and has no `v9.9`. **When** Alice publishes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** it does not mention the remote.
- **Priority:** Medium

#### E2E-076: Same commit, different message
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local annotated `v1.0` (message `A`) at `a1b2c3d`; the remote has annotated `v1.0` (message `B`) at `a1b2c3d`. **When** Alice fetches. **Then** local `v1.0` keeps message `A`. **And** the result lists `v1.0` as kept local.
- **Priority:** Medium

#### E2E-077: Connection lost after the remote accepted the tag
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** local `v2.0` exists and the connection drops after the remote accepted the tag but before Alice saw the answer. **When** Alice sees the connection refusal, then publishes again. **Then** the second publish says `up to date`. **And** no audit entry exists for either attempt.
- **Priority:** Medium

#### E2E-078: Empty message gives a plain tag
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-001
- **Given** Alice is a member and `main` is at `a1b2c3d`. **When** she creates `v7.0` with an empty message, then `v7.1` with a message of three spaces. **Then** `v7.0` is `lightweight` with no message. **And** `v7.1` is `annotated` with a message of three spaces.
- **Priority:** Medium

#### E2E-079: Overwrite to the same commit is audited
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012, FR-NEW-003
- **Given** plain `v1.0` at `a1b2c3d`. **When** Alice creates `v1.0` at `a1b2c3d` confirming the overwrite, then creates `v8.0` at `a1b2c3d` confirming the overwrite. **Then** two new entries exist: `overwrote tag 'v1.0' at <full id of a1b2c3d> (was <full id of a1b2c3d>)` and `created tag 'v8.0' at <full id of a1b2c3d>`.
- **Priority:** Medium

#### E2E-080: Branch and tag share a name
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-001
- **Given** branch `rel` at `b2c3d4e` and tag `rel` at `a1b2c3d`. **When** Alice creates `v1` targeting `rel`. **Then** `v1` is at `b2c3d4e`.
- **Priority:** Medium

#### E2E-081: Short commit prefix
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-004
- **Given** commit `a1b2c3d` exists and no branch or tag is named `a1b2c`. **When** Alice creates `v1` targeting `a1b2c`. **Then** the refusal is `Cannot tag 'a1b2c': no such branch, tag or commit.` **And** no tag is created.
- **Priority:** Medium

#### E2E-082: Ambiguous commit prefix
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-004
- **Given** two commits whose ids begin with `a1b2c3d`. **When** Alice creates `v1` targeting `a1b2c3d`. **Then** the refusal is `Cannot tag 'a1b2c3d': no such branch, tag or commit.` **And** no tag is created.
- **Priority:** Medium

#### E2E-083: Confirmed overwrite without a stated commit
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-008
- **Given** the remote `v1.0` is at `f6a7b8c` and local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming the overwrite and stating no commit. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote stays at `f6a7b8c`.
- **Priority:** High

#### E2E-084: Commit only on an unfetched remote tip
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** the remote `main` advanced to `d4e5f6a`, which Alice has not fetched, and her `v1.0` is at `e5f6a7b`, an ancestor of `d4e5f6a` on the remote but absent from Alice's history. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit e5f6a7b is not on the remote. Publish its branch first.` **And** the remote has no `v1.0`.
- **Priority:** Medium

#### E2E-085: Remote refuses the tag name
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** Alice created the local tag `a//b` (accepted by the naming rules) and the remote rejects that name with the reason `invalid ref name`. **When** Alice publishes `a//b`. **Then** the refusal is `Remote refused tag 'a//b': invalid ref name`. **And** the remote has no `a//b`, the local tag is unchanged and no audit entry was added.
- **Priority:** Medium

### 9.4 Modified tests
#### E2E-MOD-001: Listing now shows kind and message (was: listing returns name and commit only)
- **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Validated before:** a listing returned name and commit only.
- **Validates now:** the listing also shows kind and message.
- **Given** annotated `v2.0` with message `Release 2.0` at `c3d4e5f`. **When** Alice lists tags. **Then** `v2.0` shows commit `c3d4e5f`, kind `annotated`, message `Release 2.0`. **And** name and commit are exactly what they were before.
- **Category:** state transition. **Priority:** High

#### E2E-MOD-002: Fetch takes in history tags (was: the existing test "fetch brings branches and takes no tags", see 14.1)
- **Scenario:** SC-005. **Requirements:** FR-MOD-002, FR-NEW-009
- **Validated before:** after fetching a remote with `v1.0`, no tag existed.
- **Validates now:** branches arrive as before and `v1.0` also arrives.
- **Given** the remote `main` has `v1.0` at `a1b2c3d` in its history. **When** Alice fetches `main`. **Then** the branch's remote position is updated as before. **And** `v1.0` exists locally and is listed as added.
- **Category:** state transition. **Priority:** Critical

### 9.5 Removed tests
None. The "no tag follows" assertion of the existing test is replaced by E2E-MOD-002 and E2E-042.

## 10. Traceability
| Scenario | Happy | Failure (incl. refusals) | Edge | State, side effect, cross |
|---|---|---|---|---|
| SC-001 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-052 | E2E-006, E2E-009, E2E-010, E2E-012, E2E-014, E2E-015, E2E-016, E2E-067, E2E-070, E2E-081, E2E-082 | E2E-005, E2E-011, E2E-013, E2E-061, E2E-078, E2E-080 | E2E-007, E2E-008, E2E-017, E2E-018, E2E-019, E2E-051, E2E-068, E2E-079 |
| SC-002 | E2E-020 | E2E-023 | E2E-021, E2E-022, E2E-025 | E2E-024, E2E-MOD-001 |
| SC-003 | E2E-026 | E2E-027, E2E-030, E2E-032, E2E-053 | E2E-028, E2E-065 | E2E-029, E2E-031, E2E-066 |
| SC-004 | E2E-033, E2E-072 | E2E-034, E2E-036, E2E-038, E2E-039, E2E-040, E2E-055, E2E-057, E2E-058, E2E-069, E2E-071, E2E-074, E2E-075, E2E-083, E2E-085 | E2E-035, E2E-056, E2E-073, E2E-077, E2E-084 | E2E-037 |
| SC-005 | E2E-041, E2E-046 | E2E-043, E2E-045, E2E-047, E2E-049, E2E-060, E2E-063, E2E-064, E2E-076 | E2E-042, E2E-044, E2E-048, E2E-054, E2E-059, E2E-062 | E2E-050, E2E-MOD-002 |

Requirement to tests (recounted from the 9.1 table; every requirement has at least 3):

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, E2E-002, E2E-003, E2E-007, E2E-013, E2E-015, E2E-052, E2E-078, E2E-080 (9) |
| FR-NEW-002 | E2E-004, E2E-005, E2E-008, E2E-051, E2E-078 (5) |
| FR-NEW-003 | E2E-006, E2E-007, E2E-008, E2E-016, E2E-068, E2E-079 (6) |
| FR-NEW-004 | E2E-009, E2E-010, E2E-011, E2E-012, E2E-013, E2E-081, E2E-082 (7) |
| FR-NEW-005 | E2E-026, E2E-027, E2E-028, E2E-030, E2E-031, E2E-032, E2E-053 (7) |
| FR-NEW-006 | E2E-017, E2E-018, E2E-024, E2E-029 (4) |
| FR-NEW-007 | E2E-033, E2E-034, E2E-035, E2E-038, E2E-039, E2E-040, E2E-051, E2E-056, E2E-069, E2E-075, E2E-077, E2E-085 (12) |
| FR-NEW-008 | E2E-036, E2E-037, E2E-057, E2E-058, E2E-069, E2E-083 (6) |
| FR-NEW-009 | E2E-041, E2E-042, E2E-043, E2E-044, E2E-045, E2E-048, E2E-049, E2E-050, E2E-051, E2E-054, E2E-062, E2E-063, E2E-076, E2E-MOD-002 (14) |
| FR-NEW-010 | E2E-043, E2E-044, E2E-050, E2E-060, E2E-064, E2E-076 (6) |
| FR-NEW-011 | E2E-046, E2E-047, E2E-059, E2E-060 (4) |
| FR-NEW-012 | E2E-006, E2E-019, E2E-031, E2E-035, E2E-040, E2E-051, E2E-066, E2E-077, E2E-079 (9) |
| FR-NEW-013 | E2E-014, E2E-052, E2E-067 (3) |
| FR-NEW-014 | E2E-015, E2E-023, E2E-030, E2E-040, E2E-049, E2E-058, E2E-074 (7) |
| FR-NEW-015 | E2E-016, E2E-061, E2E-068 (3) |
| FR-NEW-016 | E2E-032, E2E-065, E2E-066 (3) |
| FR-NEW-017 | E2E-054, E2E-063, E2E-064 (3) |
| FR-NEW-018 | E2E-055, E2E-056, E2E-057, E2E-083 (4) |
| FR-NEW-019 | E2E-071, E2E-072, E2E-073, E2E-084 (4) |
| FR-NEW-020 | E2E-070, E2E-074, E2E-075 (3) |
| FR-NEW-021 | E2E-080, E2E-081, E2E-082 (3) |
| FR-MOD-001 | E2E-020, E2E-021, E2E-022, E2E-024, E2E-025, E2E-MOD-001 (6) |
| FR-MOD-002 | E2E-041, E2E-042, E2E-MOD-002 (3) |

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Tag | a named mark on a commit | Versioning |
| Plain tag | a tag with no message; shown as `lightweight` | Versioning |
| Annotated tag | a tag with a message, a tagger and a creation time | Versioning |
| Tagger | the member who created an annotated tag | Versioning |
| Kind | `lightweight` or `annotated` | Versioning |
| Overwrite confirmation | the member's explicit statement that an existing tag can be replaced | Versioning, Remote exchange |
| Publish | send one named tag to the remote | Remote exchange |
| Receive | bring remote tags into the project on fetch, pull or clone | Remote exchange |
| In history | reachable from the fetched branch | Remote exchange |
| Kept local | a remote tag skipped because a local tag of that name points elsewhere | Remote exchange |
| Up to date | the remote already has the identical tag | Remote exchange |
| Paused operation | a merge, rebase, cherry pick or revert waiting to be finished or abandoned | Versioning |
| Audit entry | the operator visible record of a change | Audit |
| Member | a person who belongs to the project | Actors |
| Non member | a person who does not belong to the project | Actors |
| Remote | the hosted repository the project tracks | Remote exchange |
| Operator | the person who reads the audit trail | Audit |
| Usual refusal | the refusal given today to a non member for any other project action | Actors |
| Target | the commit, branch or tag a new tag is placed on; default the checked out commit | Versioning |
| Fetch, pull, clone | bring remote content into the project; pull also merges it; clone creates the project from the remote | Remote exchange |
| Commit | a recorded state of the project's files | Versioning |
| `lightweight` | the label shown for a plain tag | Versioning |
| Branch | a named line of work in the project | Versioning |
| Checked out commit | the commit the project's files currently match | Versioning |
| Credential | the stored token a member's account holds for the remote | Remote exchange |
| Added | a tag newly brought in by a fetch, pull or clone and listed as such in its result | Remote exchange |
| Identical tags | same name, kind and commit and, for annotated tags, same message | Remote exchange |
| On the remote | a commit belonging to the history of any branch the remote holds | Remote exchange |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 6 | 1 | NOT-IMPLEMENTABLE |
| 2 | 5 | 3 | NOT-IMPLEMENTABLE |
| 3 | 2 | 2 | NOT-IMPLEMENTABLE |
| 4 | 4 | 2 | NOT-IMPLEMENTABLE |
| 5 | 4 (all minor precision) | 2 | NOT-IMPLEMENTABLE |
| waiver | 0 open | 0 open | WAIVED by the owner on 2026-10-24: round 5 amendments applied, not re-audited |

**Amendments applied:** rounds 1 and 2 amendments verified by the next round; round 3 amendments verified by round 4; round 4 amendments verified by round 5; round 5 amendments (forbidden character wording, refusal order in FR-NEW-018, remote name refusal and E2E-085, tagger and time format, two design inputs) applied, not audited.
**No implementation detail outside Section 14:** FAIL at round 2 (minor), amended at round 3; later rounds raised none. **Accepted concern:** the amendments of rounds 5 were not re-audited.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Listing shows name and commit only | `crates/core/src/tools/git.rs:2016-2022` |
| Only a listing tag tool exists | `crates/core/src/mcp/server.rs:3290`, `TOOL_CONTRACT.txt:599-603` |
| Fetch takes no tag, by design | `crates/core/src/git/remote.rs:745-747` |
| A test asserts it (test name `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags`, the one E2E-MOD-002 replaces) | `crates/core/src/tools/git.rs:10808-10830` |
| Clone records no tag | `crates/core/src/tools/git.rs:3105-3200` (no `refs/tags` write; recounted by search) |
| Publish sends one branch refspec | `crates/core/src/git/remote.rs:588-592` |
| Branch changes are refused while an operation is paused | `crates/core/src/tools/git.rs:1169,1226,1245,1264,1283,1302`, message at `:4201-4202` |
| Overwrite of a remote branch needs the remote's current commit | `crates/core/src/git/remote.rs:545-592`, `tools/git.rs:1513` |
| Branch naming rules | `crates/core/src/tools/git.rs:2200-2240` |
| FR-NEW-062 | `sdlc/specs/SPEC-0011-github-enterprise-and-token-store/spec.md:243` |
| Tag push out of scope earlier | `sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:54` |
| Membership is checked before any tag action | `crates/core/src/tools/git.rs:1781-1788` (`open` calls `authorize` first), listing uses it at `:1318` |
| Branch name limit is 255 bytes | `crates/core/src/tools/git.rs:2194` |
| FR-NEW-155 to 157 lease rules and FR-NEW-279/280 paused operation rule | `crates/core/src/git/remote.rs:545-592`; `sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:215` |
| Short commit display today is 8 characters | `crates/core/src/tools/git.rs:1866-1868` (this spec shows 7; see 14.3) |
| First ranked gap of the register | `.agent_docs/git-divergences.md` row D15 |
| Pull ending in a conflict is an `Ok` result with `status: "conflict"` | `AGENTS.md` section "Behaviour worth knowing"; `crates/core/src/tools/git.rs:1747` (`tool_remote_pull`) |
Test command: `cargo test --workspace`

### 14.2 Existing tests to modify
| Test | Site | Replaced by |
|---|---|---|
| `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags` | `crates/core/src/tools/git.rs:10808-10830` | E2E-MOD-002 (branch assertions kept, "no tag" assertions removed) |
| `e2e_new_217_pull_will_inherit_the_fetch_refspec` | `crates/core/src/tools/git.rs:10843-10870` (asserts `refs/tags/v1` absent through the shared fetch primitive) | E2E-MOD-002 on the shared fetch primitive; its "no tag" assertions are removed and `v1` asserted present; signal: `cargo test --workspace` fails on it |

### 14.3 Design inputs
- Proposed action names: tag create, tag delete, publish via the existing remote push action with a tag parameter; the frozen tool contract (`tool-contract-golden.json`, regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1`) and `TOOL_CONTRACT.txt` must be updated; detection signal: the three golden contract tests fail until regenerated.
- Overwrite on publish: the existing force path ties force to a lease (`git/remote.rs:545-592`, `tools/git.rs:1513`). FR-NEW-008 maps to it; signal: a force flag without a lease is refused today, so a test of E2E-037 fails if mapped naively.
- Fetch uses one explicit refspec with tag following off (`git/remote.rs:745-751`, `tools/git.rs:3504` cited by SPEC-0011 is stale). "In history" needs either an automatic follow or a computed reachability check; signal: E2E-042.
- Clone path imports refs by hand (`tools/git.rs:3118-3125`); it must also write tag refs; signal: E2E-046.
- Annotated tag objects must survive the object import and export between the blob store and the libgit2 cache (`git/odb.rs`); signal: E2E-048 kind and tagger after receive.
- Tagger identity: the caller's person id, as for commits (`tools/git.rs:368-371`).
- Audit entries use the existing audit recorder (`tools/git.rs:1414`); an `up to date` publish records none.
- Concurrency: tag creation must run under the repository write lock already used by commit (`tools/git.rs:2121-2135`); signal: E2E-016.
- Messages `Tag '...' already exists...` map to the existing no clobber refusal code, `Tag '...' does not exist` to the not found code, invalid names to the invalid argument code (`errors.rs:15,16,20`); the codes are design's choice.
- Stash references live under the same reference store and never appear as tags (`tools/git.rs:17310` test); keep that.
- The 5000 tag listing time has no baseline; measure in design. No threshold is promised (7.1).
- Abbreviated commits: the existing helper shows 8 characters (`tools/git.rs:1866`), this spec's messages show 7. Design either aligns the helper call for these messages to 7 or raises the spec; signal: E2E-036 and E2E-057 assert the exact text.
- Listing order is ascending by name compared byte by byte (FR-MOD-001); today's order comes from `list_refs()` (`tools/git.rs:2009-2022`) and is not documented; signal: E2E-025.
- Pull that pauses on a conflict returns `Ok` with `status: "conflict"`; tag import must happen before that return; signal: E2E-054.
- Clone into an already populated project is allowed today (`tools/git.rs:3174-3176`, re-cloning replaces the origin row), which E2E-060 relies on; signal: E2E-060.
- "On the remote" (FR-NEW-019) needs the remote's advertised branch tips at publish time; the push transport already reads them before sending (`git/remote.rs:545-592`); signal: E2E-071, E2E-073.
- Audit lines (FR-NEW-012) go through the existing recorder with action, path and detail; design picks the action labels; signal: E2E-019.
- The listing, publish and receive comparisons must peel annotated tags (a tag of a tag included) to the commit; `refs_under` returns the unpeeled target today (`crates/core/src/tools/git.rs:2008-2022`); signal: E2E-004 (commit shown), E2E-062, E2E-069.
- The published list of actions is a machine checked contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`); adding tag actions is a deliberate change of it; signal: the three golden contract tests.
- `git.tags` entries keep `name`, `full_ref` and `sha`; kind, message and tagger are added keys; signal: the existing listing assertions and the golden contract tests.
- Update the comments citing FR-NEW-062 at `git/remote.rs:745-746` and `tools/git.rs:10593` (review only, NOTHING DETECTS THIS automatically).
- Tagger identifier: use the caller's commit identity (`tools/git.rs:368-371`); signal: E2E-004.
- Tags that point at a non commit (a tree or a blob) must be filtered on receive; signal: E2E-062.
- `tools/git.rs:3504` cited by SPEC-0011 for FR-NEW-062 is stale; the live site is `git/remote.rs:745-747`.
