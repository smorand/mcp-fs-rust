# Project storage quota enforcement: functional specification

> Id: SPEC-0020
> Nature: FEAT
> Depth: L
> Depth evidence: FEAT / depth L: 6 areas touched (file writes, archive extraction, conversion, version control, trash, project administration), a new observable refusal contract, 15 requirements.
> Status: implemented
> Generated on: 2026-10-11
> Project: existing
> From backlog: BL-0039
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

A platform admin can already set a maximum storage size on a project, but nothing honors it: any
member can keep writing past it, and nobody can see how much a project uses. This increment makes
the limit real. Every operation that would push a project's stored bytes over its limit is refused
with an exact message, multi file operations are all or nothing, and an admin or the project owner
can read the limit and the current usage. It closes the part of SPEC-0010 that was scoped and never
built.

## 2. Current State

### 2.1 What users can do today

- A platform admin sets or clears a project's maximum storage size in megabytes; zero is refused
  with `max_mb must be > 0`; lowering it below what the project stores is accepted and removes
  nothing.
- The stored limit has no effect on any operation: writes, copies, archive extraction, conversions
  and version control operations all succeed whatever the limit.
- Nobody can read back a project's limit or its usage; no operation computes usage.
- The only limit that refuses writes is each member's session write budget, with the message
  `session write quota of <N> bytes exceeded`; a copy and a version control push are not charged to
  it.
- Deleting a file moves it to the trash, where it stays stored until the purge removes it.
- An owner or admin gate exists for other project settings; for a non admin naming a missing
  project it answers `project '<id>' not found` before checking ownership.

### 2.2 Specs governing this area

- SPEC-0010 (project storage quota, as built): FR-NEW-001 to 004 and 014 delivered; FR-NEW-005 to
  012 not delivered; FR-NEW-013 true only because nothing reads the limit. This spec delivers the
  intent of FR-NEW-005, 006, 009, 010, 011, 012 and modifies FR-NEW-013 (FR-MOD-001). FR-NEW-007 and
  008 (size sorted breakdown) stay out of scope (BL-0041). SPEC-0010 is not modified.
- SPEC-0019 (document artifacts): artifacts are charged to the session budget only; this spec
  excludes them from project usage (FR-NEW-002).
- SPEC-0015 (archive extraction): one quota check before any write; this spec keeps that shape.

### 2.3 What is verified today

Eight tests cover setting and clearing the limit and its refusals. No test covers a write refused by
the limit, reading usage, or reading the limit back, because none of it exists.

## 3. Scope

### 3.1 In scope

- Refusing any operation that would make a project's usage exceed its limit, with an exact message.
- All or nothing for multi file operations.
- Reading a project's limit and usage, by a platform admin or the project owner.
- Lowering a limit below usage: nothing evicted, growing operations refused from then on.

### 3.2 Out of scope, and why

| Non goal | Reason | Backlog |
|---|---|---|
| Size sorted usage breakdown | separate capability, needed once limits bite | BL-0041 |
| Counting version control history, zip exports, document artifacts | not project files from the user's view | none, decided |
| Charging a version control push | a push stores history only, no project file | none, decided |
| Eviction when a limit is lowered | SPEC-0010 non goal, kept | none, decided |
| Alerts near the limit, platform wide quota | SPEC-0010 non goals, kept | none, decided |

### 3.3 Constraints imposed by the context

- Imposed by the project conventions: AI assistants and HTTP API consumers get the same behavior.

## 4. Actors

| Actor | Description |
|---|---|
| Platform admin (Ada) | Sets limits; reads any project's quota. |
| Project owner (Olivia) | Owns a project; reads its quota; writes like any member. |
| Member (Mia) | Belongs to the project; writes. |
| Outsider (Otto) | Does not belong to the project. |

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Storage | what a project holds and the limit on it | usage, limit, bytes requested, growing operation, multi file operation, trash, purge |
| Administration | who may read and set a project's limit | platform admin, owner |
| Session budget | the bytes a member may write in a session | session write budget |

## 5. Usage scenarios

### SC-001: A member writes while the project has a limit
**Actor:** Member
**Preconditions:** project `Atlas` has a limit.
**Flow:**
1. The member writes, appends, uploads, edits, copies, creates a Word document or converts a
   document → the system computes the bytes the operation adds and accepts it when usage stays
   within the limit.
**Postconditions:** usage grew by the bytes added.
**Exceptions:**
- EXC-001a: usage would exceed the limit → `project storage quota of <N> MB exceeded: <U> bytes
  used, <W> bytes requested`; nothing changes; the session budget is not charged.
- EXC-001b: the session budget would be exceeded and the limit would not → today's session refusal.

### SC-002: A multi file operation
**Actor:** Member
**Flow:**
1. The member extracts an archive, copies a folder, or clones, pulls, merges or checks out a version
   → the system computes the total bytes added once, before writing anything.
**Exceptions:**
- EXC-002a: the total would exceed the limit → the refusal of EXC-001a with the total; no file of
  the operation is written.

### SC-003: Reading a project's quota
**Actor:** Platform admin or project owner
**Flow:**
1. The actor asks for a project's quota → the system shows the project id, the limit in MB (empty
   when unlimited) and the bytes used, computed at that moment.
**Exceptions:**
- EXC-003a: neither admin nor owner → `'<person>' is not the owner of '<project>'`, also when the
  project does not exist.
- EXC-003b: an admin names a missing project → `project '<project>' not found`.

### SC-004: The admin lowers the limit below usage
**Actor:** Platform admin, then members
**Flow:**
1. The admin sets a limit below current usage → accepted, nothing removed.
2. Members keep reading, deleting, moving, restoring from the trash, pushing and shrinking files.
**Exceptions:**
- EXC-004a: any growing operation → refused per EXC-001a.

### SC-005: Two members write at the same moment near the limit
**Actor:** Two members
**Flow:**
1. Both write; each write fits alone, not both → exactly one is refused.
**Postconditions:** usage never ends above the limit.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: Refuse an operation over the limit
> IF an operation would make a project's usage exceed its limit THEN THE Storage context SHALL
> refuse it with `project storage quota of <N> MB exceeded: <U> bytes used, <W> bytes requested`.
- **Business rules:** N is the limit in MB as set; U the usage before the operation; W the bytes
  requested (FR-NEW-003). The refusal belongs to its own category, distinct from the session
  budget refusal.
- **Scenario:** SC-001, SC-002, SC-004 **Priority:** Must

#### FR-NEW-002 [EARS-U]: What usage counts
> The Storage context SHALL count as a project's usage the sum of the declared sizes of all its
> files, trashed files included, each path counted once even when files share content.
- **Business rules:** version control history, zip exports and document artifacts (images and
  tables kept from conversions) are not counted. A permanent purge removes a file's bytes from usage.
- **Scenario:** SC-001, SC-002, SC-003, SC-004 **Priority:** Must

#### FR-NEW-003 [EARS-U]: Which operations are checked and what they request
> The Storage context SHALL check the limit on writes, appends, uploads, every kind of edit, copies,
> Word document creation, a conversion's Markdown file, archive extraction, and every version control
> operation that changes project files: clone, pull, merge, merge conflict resolution, cherry pick
> and its continuation, rebase and its continuation, revert and its continuation, reset, branch
> creation, branch switch, branch reset, checkout of a version or of a file, stash save, stash apply,
> stash pop, and a pull request merge into the project.
- **Business rules:** the bytes requested are the net change the operation makes to usage: for each
  path the operation writes, its new size minus its previous size (zero for a new path), plus, for
  each path a version control operation removes, minus that path's size; summed over the operation.
  A path removed by a version control operation leaves usage at once and does not go to the trash.
- **Scenario:** SC-001, SC-002, SC-004 **Priority:** Must

#### FR-NEW-004 [EARS-O]: Shrinking operations always pass
> IF an operation requests zero or fewer bytes THEN THE Storage context SHALL accept it whatever the
> limit and the usage.
- **Scenario:** SC-001, SC-002, SC-004 **Priority:** Must

#### FR-NEW-005 [EARS-U]: The boundary
> The Storage context SHALL accept an operation that brings usage exactly to the limit.
- **Business rules:** one byte over is refused; 1 MB is 1 048 576 bytes.
- **Scenario:** SC-001, SC-002, SC-004 **Priority:** Must

#### FR-NEW-006 [EARS-U]: The limit before the session budget
> The Storage context SHALL check the project limit before the session budget, and charge nothing to
> the session budget for an operation the limit refuses.
- **Scenario:** SC-001, SC-002, SC-005 **Priority:** Must

#### FR-NEW-007 [EARS-E]: All or nothing
> WHEN a multi file operation is checked THE Storage context SHALL check its total requested bytes
> once, before writing any of its files.
- **Business rules:** refused, no file of the operation is written.
- **Scenario:** SC-002, SC-005 **Priority:** Must

#### FR-NEW-008 [EARS-UB]: Never above the limit under concurrency
> The Storage context SHALL NOT let concurrent operations leave a project's usage above its limit.
- **Business rules:** of two operations that fit alone but not together, exactly one is refused.
- **Scenario:** SC-005 **Priority:** Must

#### FR-NEW-009 [EARS-E]: Read a project's quota
> WHEN a platform admin or the project owner reads a project's quota THE Administration context
> SHALL show the project id, the limit in MB, and the bytes used computed at that moment.
- **Business rules:** the quota shows exactly three values: the project id as stored, the limit as
  the whole number of MB the admin set (no value, not zero and not text, when unlimited), and the
  usage as a whole number of bytes; usage per FR-NEW-002.
- **Scenario:** SC-003 **Priority:** Must

#### FR-NEW-010 [EARS-UB]: Who may read
> The Administration context SHALL NOT show a project's quota to anyone who is neither a platform
> admin nor the project owner.
- **User visible text:** `'<person>' is not the owner of '<project>'`, given also when the project
  does not exist; an admin naming a missing project gets `project '<project>' not found`.
- **Scenario:** SC-003 **Priority:** Must

#### FR-NEW-011 [EARS-S]: Unlimited projects
> WHILE a project has no limit THE Storage context SHALL refuse no operation for the project limit.
- **Scenario:** SC-001, SC-003 **Priority:** Must

#### FR-NEW-012 [EARS-U]: Operations never refused by the limit
> The Storage context SHALL accept moves, deletes to the trash, trash restores and version control
> pushes whatever the limit and the usage.
- **Business rules:** a delete keeps the file's bytes in usage until its permanent purge. Merge
  abort, rebase abort, cherry pick abort and revert abort are also always accepted: each restores the
  project to its state before the operation.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-013 [EARS-UB]: A refusal changes nothing
> The Storage context SHALL NOT change any file or the usage when it refuses an operation for the
> limit.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-014 [EARS-O]: The conversion's Markdown file in a documented upload
> IF the Markdown file of a conversion requested with an upload would exceed the limit THEN THE
> Storage context SHALL keep the uploaded file and report the refusal of FR-NEW-001 as the
> documentation outcome.
- **Business rules:** the upload itself is checked first, on its own bytes. For the Markdown file the
  project limit is checked before the session budget; the documentation outcome reports the project
  limit refusal when both would be exceeded, and `session write quota of <N> bytes exceeded` when
  only the session budget would.
- **Scenario:** SC-001 **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-E]: Lowering the limit below usage (modifies SPEC-0010/FR-NEW-013)
> WHEN a platform admin sets a limit below a project's usage THE Administration context SHALL accept
> it, remove nothing, and from then on refuse every growing operation.
- **Before:** accepted and nothing removed, with no further effect (Section 2.1).
- **After:** the statement above.
- **Reason:** the limit becomes enforced.
- **Scenario:** SC-004 **Priority:** Must

## 7. Non functional requirements (user observable)

### 7.1 Performance
Reading a quota and checking an operation stay within the time of the operation itself for a
project of 100 000 files (no noticeable delay for the member).

### 7.2 Security and access
FR-NEW-010; the refusal order prevents probing which projects exist.

### 7.5 Reliability
A refused operation leaves nothing behind (FR-NEW-013); concurrent operations never overshoot
(FR-NEW-008).

### 7.6 Operability
The refusal names the limit, the usage and the bytes requested, so a member or operator knows how
much to free.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | empty project; empty file | 0 bytes used; a 0 byte write accepted | E2E-043, E2E-056, E2E-057 |
| Boundary values | exactly the limit; one byte over | accepted; refused | E2E-003, E2E-004, E2E-029 |
| Concurrent actions | two writes near the limit | exactly one refused | E2E-049 to E2E-052 |
| Slow or lost connectivity | not applicable: checks are local to the product | n/a | n/a |
| Access states | admin, owner, member, outsider, missing project | per FR-NEW-010 | E2E-037 to E2E-041 |
| Data variations | copies sharing content | counted once per path | E2E-015, E2E-016 |
| Navigation | not applicable: no navigation | n/a | n/a |
| Partial operations | refused multi file operation | nothing written | E2E-028, E2E-030 to E2E-034 |
| Dependency failures | not applicable: no new dependency | n/a | n/a |
| Ordering and timing | both limits exceeded | project limit answers first | E2E-005, E2E-056 |
| Exhaustion | over limit after lowering; both members refused at the limit | growing refused, shrinking accepted | E2E-044 to E2E-048, E2E-061, E2E-064 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

Shared setup unless stated: project `Atlas` has a limit of 1 MB (1 048 576 bytes) and uses
1 000 000 bytes (48 576 bytes left). Mia (`mia@example.com`) and Olivia (`olivia@example.com`,
owner) are members with no session budget used; Ada (`ada@example.com`) is a platform admin; Otto
(`otto@example.com`) belongs to no project. Each member's session budget is 262 144 bytes.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | 001, 003 | High |
| E2E-002 | New | failure | SC-001 | 001, 013 | Critical |
| E2E-003 | New | edge | SC-001 | 005 | Critical |
| E2E-004 | New | failure | SC-001 | 005, 001 | Critical |
| E2E-005 | New | failure | SC-001 | 006, 001 | Critical |
| E2E-006 | New | side effect | SC-001 | 006, 013 | Critical |
| E2E-007 | New | failure | SC-001 | 006 | High |
| E2E-008 | New | failure | SC-001 | 003 | High |
| E2E-009 | New | happy | SC-001 | 003, 005 | High |
| E2E-010 | New | failure | SC-001 | 003 | High |
| E2E-011 | New | edge | SC-001 | 004 | High |
| E2E-012 | New | edge | SC-001 | 004 | High |
| E2E-013 | New | failure | SC-001 | 003, 013 | High |
| E2E-014 | New | failure | SC-001 | 003 | High |
| E2E-015 | New | failure | SC-001 | 002, 003 | Critical |
| E2E-016 | New | edge | SC-001 | 002 | High |
| E2E-017 | New | failure | SC-001 | 003 | Medium |
| E2E-018 | New | failure | SC-001 | 014, 013 | High |
| E2E-019 | New | side effect | SC-001 | 002 | Medium |
| E2E-020 | New | side effect | SC-003 | 002 | Low |
| E2E-021 | New | side effect | SC-003 | 002 | Low |
| E2E-022 | New | side effect | SC-004 | 002, 012 | High |
| E2E-023 | New | side effect | SC-004 | 012, 002 | High |
| E2E-024 | New | edge | SC-004 | 012 | Medium |
| E2E-025 | New | edge | SC-004 | 012 | Medium |
| E2E-026 | New | edge | SC-004 | 012 | Medium |
| E2E-027 | New | happy | SC-002 | 007, 003 | High |
| E2E-028 | New | failure | SC-002 | 007, 013 | Critical |
| E2E-029 | New | edge | SC-002 | 007, 005 | Medium |
| E2E-030 | New | failure | SC-002 | 007, 003 | High |
| E2E-031 | New | failure | SC-002 | 007, 003 | High |
| E2E-032 | New | failure | SC-002 | 007, 013 | High |
| E2E-033 | New | failure | SC-002 | 007, 003 | Medium |
| E2E-034 | New | failure | SC-002 | 007, 003 | Medium |
| E2E-035 | New | edge | SC-002 | 004 | Medium |
| E2E-036 | New | happy | SC-002 | 003, 002 | Medium |
| E2E-037 | New | happy | SC-003 | 009 | Critical |
| E2E-038 | New | happy | SC-003 | 009 | Critical |
| E2E-039 | New | failure | SC-003 | 010 | Critical |
| E2E-040 | New | failure | SC-003 | 010 | Critical |
| E2E-041 | New | failure | SC-003 | 010 | Critical |
| E2E-042 | New | edge | SC-003 | 009, 011 | Medium |
| E2E-043 | New | edge | SC-003 | 009 | Medium |
| E2E-044 | New | happy | SC-004 | MOD-001 | Critical |
| E2E-045 | New | failure | SC-004 | MOD-001, 001 | Critical |
| E2E-046 | New | edge | SC-004 | MOD-001, 004 | Critical |
| E2E-047 | New | side effect | SC-004 | MOD-001, 012 | High |
| E2E-048 | New | edge | SC-004 | MOD-001, 005 | Medium |
| E2E-049 | New | failure | SC-005 | 008 | Critical |
| E2E-050 | New | happy | SC-005 | 008 | Medium |
| E2E-051 | New | failure | SC-005 | 008, 007 | Critical |
| E2E-052 | New | edge | SC-005 | 008 | Medium |
| E2E-053 | New | happy | SC-001 | 011 | High |
| E2E-054 | New | failure | SC-001 | 011, 006 | Medium |
| E2E-055 | New | edge | SC-001 | 011 | Medium |
| E2E-056 | New | edge | SC-001 | 005, 006 | High |
| E2E-057 | New | edge | SC-001 | 004, 005 | Low |
| E2E-058 | New | happy | SC-001 | 014 | High |
| E2E-059 | New | failure | SC-001 | 014, 001 | High |
| E2E-060 | New | edge | SC-001 | 014, 005 | Medium |
| E2E-061 | New | failure | SC-004 | MOD-001, 003 | High |
| E2E-062 | New | side effect | SC-002 | 007, 006 | High |
| E2E-063 | New | side effect | SC-005 | 008, 006 | Medium |
| E2E-064 | New | failure | SC-005 | 008 | High |
| E2E-065 | New | edge | SC-002 | 003, 007 | High |
| E2E-066 | New | failure | SC-002 | 003, 013 | High |
| E2E-067 | New | edge | SC-002 | 003, 004 | High |
| E2E-068 | New | edge | SC-004 | 012 | High |
| E2E-069 | New | failure | SC-001 | 003, 013 | High |

Coverage: happy 10, failure 29, edge 21, side effect 9, state transition 0 separate (SC-004 covers
the limit lowered), security 0 (not a security lot). Total 69. Happy to failure ratio 1:2.90.

### 9.2 New tests

Each test reads Given / When / Then / And; scenario and requirements are in 9.1.

- **E2E-001** Given the shared setup. When Mia writes a new file `/a.txt` of 40 000 bytes. Then the write is accepted. And Ada reads Atlas's usage as 1 040 000 bytes.
- **E2E-002** Given the shared setup. When Mia writes a new file `/b.txt` of 50 000 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/b.txt` does not exist, and usage is still 1 000 000 bytes.
- **E2E-003** Given the shared setup. When Mia writes a new file of 48 576 bytes. Then the write is accepted. And usage is 1 048 576 bytes.
- **E2E-004** Given usage is 1 048 576 bytes. When Mia writes a new file of 1 byte. Then she is refused with `project storage quota of 1 MB exceeded: 1048576 bytes used, 1 bytes requested`. And usage is still 1 048 576 bytes.
- **E2E-005** Given the shared setup. When Mia writes a 300 000 byte file, over both the project limit and her session budget. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 300000 bytes requested`. And she does not receive `session write quota of 262144 bytes exceeded`.
- **E2E-006** Given Mia was refused a 50 000 byte write as in E2E-002 and Ada then removes Atlas's limit. When Mia writes 262 144 bytes in the same session, then 1 more byte. Then the 262 144 byte write is accepted. And the 1 byte write is refused with `session write quota of 262144 bytes exceeded`.
- **E2E-007** Given Atlas has a limit of 2 MB and uses 0 bytes. When Mia writes 262 145 bytes. Then she is refused with `session write quota of 262144 bytes exceeded`. And the file does not exist.
- **E2E-008** Given `/n.md` is 10 000 bytes. When Mia appends 48 577 bytes to it. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/n.md` is still 10 000 bytes.
- **E2E-009** Given `/n.md` is 10 000 bytes. When Mia appends 48 576 bytes to it. Then the append is accepted. And `/n.md` is 58 576 bytes, and usage is 1 048 576 bytes.
- **E2E-010** Given `/r.md` is 100 000 bytes. When Mia overwrites it with 148 577 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/r.md` keeps its previous content.
- **E2E-011** Given usage is 1 048 576 bytes and `/r.md` is 100 000 bytes. When Mia overwrites `/r.md` with different content of 100 000 bytes. Then the overwrite is accepted. And usage is still 1 048 576 bytes.
- **E2E-012** Given usage is 1 048 576 bytes. When Mia edits `/r.md` (one of the project's files, 100 000 bytes), replacing 5 000 bytes of text with 1 000 bytes. Then the edit is accepted. And usage is 1 044 576 bytes.
- **E2E-013** Given `/r.md` contains the single character `X`. When Mia edits it to replace `X` with 48 578 bytes of text. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 48577 bytes requested`. And `/r.md` still contains `X`.
- **E2E-014** Given the shared setup. When Mia uploads `photo.png` of 60 000 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 60000 bytes requested`. And `photo.png` is not in Atlas.
- **E2E-015** Given `/big.bin` is 50 000 bytes. When Mia copies it to `/big2.bin`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/big2.bin` does not exist.
- **E2E-016** Given `/s.bin` is 40 000 bytes. When Mia copies it to `/s2.bin`. Then the copy is accepted. And usage is 1 040 000 bytes.
- **E2E-017** Given the shared setup. When Mia creates a Word document `/w.docx` whose content makes a document of N bytes, N being the size the same content produces in a project with no limit and N greater than 48 576. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, <N> bytes requested`, N written out. And `/w.docx` does not exist.
- **E2E-018** Given document conversion is on and `/d.pdf` of 30 000 bytes converts to a 20 000 byte Markdown file. When Mia uploads `/d.pdf` requesting conversion. Then the upload is accepted and its documentation outcome reads `project storage quota of 1 MB exceeded: 1030000 bytes used, 20000 bytes requested`. And no Markdown file exists, and usage is 1 030 000 bytes.
- **E2E-019** Given `/e.pdf` of 10 000 bytes converts to a 5 000 byte Markdown file with 4 images and 2 tables totalling 120 000 bytes. When Mia uploads it requesting conversion. Then both are accepted. And usage is 1 015 000 bytes.
- **E2E-020** Given the shared setup. When Mia exports the whole project as a zip of 900 000 bytes. Then the export is accepted. And usage is still 1 000 000 bytes.
- **E2E-021** Given the shared setup. When Mia commits all of Atlas's files to version control history. Then the commit is accepted. And usage is still 1 000 000 bytes.
- **E2E-022** Given `/old.txt` is 100 000 bytes. When Mia deletes it. Then the delete is accepted. And `/old.txt` is in the trash, and usage is still 1 000 000 bytes.
- **E2E-023** Given `/old.txt` of 100 000 bytes is in the trash. When it is permanently purged. Then usage is 900 000 bytes. And Mia can then write a new 148 576 byte file.
- **E2E-024** Given usage is 1 500 000 bytes under a 1 MB limit. When Mia moves `/a.txt` to `/arch/a.txt`. Then the move is accepted. And usage is still 1 500 000 bytes.
- **E2E-025** Given usage is 1 500 000 bytes under a 1 MB limit and `/t.txt` of 30 000 bytes is in the trash. When Mia restores it. Then the restore is accepted. And `/t.txt` is back, and usage is still 1 500 000 bytes.
- **E2E-026** Given usage is 1 500 000 bytes under a 1 MB limit. When Mia pushes new commits to Atlas's repository. Then the push succeeds. And no quota message appears.
- **E2E-027** Given the shared setup. When Mia extracts `pack.zip` with entries `x` (10 000 bytes), `y` (10 000 bytes) and `z` (20 000 bytes). Then the extraction is accepted. And all three files exist, and usage is 1 040 000 bytes.
- **E2E-028** Given the shared setup. When Mia extracts an archive with entries of 20 000, 20 000 and 10 000 bytes. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And none of the three files exists, and usage is still 1 000 000 bytes.
- **E2E-029** Given the shared setup. When Mia extracts an archive whose entries total exactly 48 576 bytes. Then the extraction is accepted. And usage is 1 048 576 bytes.
- **E2E-030** Given folder `/src` holds 3 files totalling 60 000 bytes. When Mia copies `/src` to `/dst`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 60000 bytes requested`. And `/dst` does not exist.
- **E2E-031** Given a remote repository whose files total 60 000 bytes (two files of 30 000). When Mia clones it into Atlas. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 60000 bytes requested`. And no cloned file exists in Atlas.
- **E2E-032** Given Atlas is a clone and the remote has new commits adding 50 000 bytes. When Mia pulls. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And the files stay at their previous commit.
- **E2E-033** Given branch `feature` adds 49 000 bytes over `main`. When Mia merges `feature` into `main`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 49000 bytes requested`. And the files of `main` are unchanged.
- **E2E-034** Given branch `big` holds `/r.md` 50 000 bytes larger than on the current branch. When Mia checks out `big`. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And the current branch and its files stay as they were.
- **E2E-035** Given usage is 1 500 000 bytes under a 1 MB limit and branch `slim` is 200 000 bytes smaller. When Mia checks out `slim`. Then the checkout is accepted. And usage is 1 300 000 bytes.
- **E2E-036** Given a remote repository whose files total 40 000 bytes and whose history is 500 000 bytes. When Mia clones it into Atlas. Then the clone is accepted. And usage is 1 040 000 bytes.
- **E2E-037** Given the shared setup. When Ada reads Atlas's quota. Then she sees project `Atlas`, limit 1 MB and 1 000 000 bytes used.
- **E2E-038** Given the shared setup. When Olivia reads Atlas's quota. Then she sees project `Atlas`, limit 1 MB and 1 000 000 bytes used.
- **E2E-039** Given Mia is a member of Atlas but not its owner. When she reads Atlas's quota. Then she is refused with `'mia@example.com' is not the owner of 'Atlas'`. And no limit or usage is shown.
- **E2E-040** Given Otto belongs to no project. When he reads Atlas's quota. Then he is refused with `'otto@example.com' is not the owner of 'Atlas'`.
- **E2E-041** Given project `Zephyr` does not exist. When Otto reads Zephyr's quota, then Ada does. Then Otto is refused with `'otto@example.com' is not the owner of 'Zephyr'`. And Ada is refused with `project 'Zephyr' not found`.
- **E2E-042** Given Atlas has no limit and uses 5 000 000 bytes. When Ada reads Atlas's quota. Then she sees project `Atlas`, no limit value (not 0, not text), and 5 000 000 bytes used.
- **E2E-043** Given project `Atlas` is empty. When Ada reads its quota, Mia writes a 1 234 byte file, and Ada reads it again. Then the first read shows 0 bytes used. And the second shows 1 234 bytes used.
- **E2E-044** Given Atlas has no limit and uses 1 500 000 bytes. When Ada sets its limit to 1 MB. Then the change is accepted. And every file is still present, and usage is 1 500 000 bytes.
- **E2E-045** Given the state after E2E-044. When Mia writes a new 1 byte file. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 1 bytes requested`.
- **E2E-046** Given the state after E2E-044 and `/r.md` of 100 000 bytes. When Mia overwrites `/r.md` with 40 000 bytes. Then the overwrite is accepted. And usage is 1 440 000 bytes.
- **E2E-047** Given the state after E2E-044. When Mia deletes `/a.txt` (40 000 bytes) and moves `/b.txt` to `/c.txt`. Then both are accepted. And usage is still 1 500 000 bytes.
- **E2E-048** Given Atlas uses 1 048 576 bytes and has no limit. When Ada sets the limit to 1 MB, then Mia writes a 1 byte file. Then the limit change is accepted. And the write is refused with `project storage quota of 1 MB exceeded: 1048576 bytes used, 1 bytes requested`.
- **E2E-049** Given the shared setup. When Mia and Olivia each write a different new 40 000 byte file at the same moment. Then exactly one write is accepted. And the other is refused with `project storage quota of 1 MB exceeded: 1040000 bytes used, 40000 bytes requested`, and usage is 1 040 000 bytes.
- **E2E-050** Given the shared setup. When Mia and Olivia each write a new 24 000 byte file at the same moment. Then both writes are accepted. And usage is 1 048 000 bytes.
- **E2E-051** Given the shared setup. When Mia extracts an archive whose entries total 30 000 bytes while Olivia writes a 30 000 byte file at the same moment. Then exactly one operation is accepted. And usage is 1 030 000 bytes, and when the refused one is the extraction none of its entries exists.
- **E2E-052** Given usage is 1 040 000 bytes. When Mia and Olivia each try 20 times to write a new 8 000 byte file, concurrently. Then exactly one write is accepted. And usage is 1 048 000 bytes and never exceeded 1 048 576 bytes.
- **E2E-053** Given Atlas has no limit and uses 5 000 000 bytes. When Mia writes 262 144 bytes. Then the write is accepted. And usage is 5 262 144 bytes.
- **E2E-054** Given Atlas has no limit. When Mia writes 262 145 bytes. Then she is refused with `session write quota of 262144 bytes exceeded`. And no project quota message appears.
- **E2E-055** Given Mia was refused a 50 000 byte write as in E2E-002. When Ada removes the limit and Mia retries the same write. Then the write is accepted. And usage is 1 050 000 bytes.
- **E2E-056** Given an empty Atlas with a 1 MB limit. When Mia writes a 1 048 577 byte file. Then she is refused with `project storage quota of 1 MB exceeded: 0 bytes used, 1048577 bytes requested`. And she does not receive `session write quota of 262144 bytes exceeded`.
- **E2E-057** Given usage is 1 048 576 bytes. When Mia creates an empty file `/empty.txt`. Then the write is accepted. And usage is still 1 048 576 bytes.
- **E2E-058** Given document conversion is on and `/f.pdf` of 10 000 bytes converts to a 5 000 byte Markdown file. When Mia uploads `/f.pdf` requesting conversion. Then the upload and its Markdown file are accepted. And usage is 1 015 000 bytes.
- **E2E-059** Given document conversion is on. When Mia uploads `/g.pdf` of 50 000 bytes requesting conversion. Then the upload is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And neither `/g.pdf` nor its Markdown file exists.
- **E2E-060** Given document conversion is on and `/h.pdf` of 40 000 bytes converts to an 8 576 byte Markdown file. When Mia uploads `/h.pdf` requesting conversion. Then both are accepted. And usage is exactly 1 048 576 bytes.
- **E2E-061** Given the state after E2E-044 and `/s.bin` of 10 bytes. When Mia copies `/s.bin` to `/s3.bin`. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`. And `/s3.bin` does not exist.
- **E2E-062** Given Mia was refused the archive extraction of E2E-028 and Ada then removes Atlas's limit. When Mia writes 262 144 bytes in the same session. Then the write is accepted. And a further 1 byte write is refused with `session write quota of 262144 bytes exceeded`.
- **E2E-063** Given the race of E2E-049 has finished and Ada then removes Atlas's limit. When the member whose write was refused writes 262 144 bytes in the same session. Then the write is accepted.
- **E2E-064** Given usage is 1 048 000 bytes. When Mia and Olivia each write a new 1 000 byte file at the same moment. Then both are refused with `project storage quota of 1 MB exceeded: 1048000 bytes used, 1000 bytes requested`. And usage is still 1 048 000 bytes.
- **E2E-065** Given the state after E2E-044, and branch `feature` holds a commit adding `/k.txt` of 10 bytes. When Mia cherry picks that commit onto the current branch. Then she is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`. And `/k.txt` does not exist, and usage is still 1 500 000 bytes.
- **E2E-066** Given the shared setup and a stash holding `/st.txt` of 50 000 bytes. When Mia pops the stash. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/st.txt` does not exist, and the stash still exists.
- **E2E-067** Given the state after E2E-044, and branch `swap` differs from the current branch by deleting `/r.md` (100 000 bytes) and adding `/n2.txt` (50 000 bytes). When Mia checks out `swap`. Then the checkout is accepted. And `/r.md` is not in the trash, `/n2.txt` exists, and usage is 1 450 000 bytes.
- **E2E-068** Given Atlas has no limit and uses 1 400 000 bytes, Mia starts a merge that stops on a conflict after removing `/big.bin` (100 000 bytes), and Ada then sets the limit to 1 MB. When Mia aborts the merge. Then the abort is accepted. And `/big.bin` exists again, and usage is 1 400 000 bytes.
- **E2E-069** Given `/r.md` is 100 000 bytes on the current branch and 150 000 bytes in the last commit. When Mia restores `/r.md` from the last commit. Then she is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And `/r.md` is still 100 000 bytes.

### 9.3 to 9.5

No exploit test (not a security lot), no modified test, no removed test.

## 10. Traceability

| Scenario | Requirements (FR-NEW- unless MOD-) | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | 001, 002, 003, 004, 005, 006, 011, 013, 014 | E2E-001, 009, 053, 058 | E2E-002, 004, 005, 007, 008, 010, 013, 014, 015, 017, 018, 054, 059, 069 | E2E-003, 011, 012, 016, 055, 056, 057, 060 | E2E-006, 019 |
| SC-002 | 002, 003, 004, 005, 006, 007, 013 | E2E-027, 036 | E2E-028, 030, 031, 032, 033, 034, 066 | E2E-029, 035, 065, 067 | E2E-062 |
| SC-003 | 002, 009, 010, 011 | E2E-037, 038 | E2E-039, 040, 041 | E2E-042, 043 | E2E-020, 021 |
| SC-004 | 001, 002, 003, 004, 005, 012, MOD-001 | E2E-044 | E2E-045, 061 | E2E-024, 025, 026, 046, 048, 068 | E2E-022, 023, 047 |
| SC-005 | 006, 007, 008 | E2E-050 | E2E-049, 051, 064 | E2E-052 | E2E-063 |

## 11. Open questions

None.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| Limit | the maximum storage size a platform admin sets on a project, in MB; empty means unlimited | Storage |
| Usage | the sum of the declared sizes of a project's files, trashed ones included | Storage |
| Bytes requested | the net change an operation makes to usage | Storage |
| Growing operation | an operation requesting more than zero bytes | Storage |
| Multi file operation | archive extraction, folder copy, and every version control operation listed in FR-NEW-003 | Storage |
| Platform admin | the person who manages projects and sets limits | Administration |
| Owner | the person who owns a project | Administration |
| Session write budget | the bytes a member may write during a session | Session budget |
| Trash, purge | soft deletion with restore; permanent removal | Storage |

## 13. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 1 | NOT-IMPLEMENTABLE, precision only, applied |
| 2 | 3 | 2 | NOT-IMPLEMENTABLE, precision only, applied |
| 3 | 2 | 4 | NOT-IMPLEMENTABLE: the 2 F ask for an error code, a tool name and result field names, which invariant 7 keeps out of a FEAT spec; recorded as design inputs (14.3), A fixes applied; escalated to the owner |

**Closure (owner decision, 2026-10-11):** committed as specified; the error category, operation name
and result labels are chosen by /sdlc-design per Section 14.3, consistent with `admin.set_project_quota`.

**Amendments applied:** round 1: version control operations listed (FR-NEW-003) + E2E-065, 066; read-back value shapes (FR-NEW-009, E2E-042); 14.3 choke point line; consistency fixes (scenario fields, FR-MOD-001 scenario, evidence range, glossary, E2E-012 precondition). Round 2: removals in the net change (FR-NEW-003) + E2E-067; aborts always accepted
(FR-NEW-012) + E2E-068; documented upload refusal order (FR-NEW-014); file restore over the limit E2E-069;
14.3 lines for direct charge paths and REST parity.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| Limit set, cleared, zero refused, lowering accepted | `crates/core/src/mcp/server.rs:2774` tool, `crates/core/src/storage/admin.rs:490-501`, `TOOL_CONTRACT.txt:57` |
| Limit has no effect, no usage computed | `crates/core/src/storage/meta.rs:906-975` (`put_file` takes no limit), `crates/core/src/storage/volume.rs` (no quota lookup); no `SUM` over node sizes in `crates/core/src` |
| Session budget message | `crates/core/src/safety.rs:130-136` |
| Copy and push uncharged | `crates/core/src/core/fs_ops.rs` `copy_path` (metadata copy), `crates/core/src/git/http/` (only `max_pack_size_mb`, `git/http/mod.rs:100`, `:200`) |
| Trash keeps the file stored | `crates/core/src/safety.rs:183` |
| Owner or admin gate checks existence first for non admins | `crates/core/src/state.rs:47-54`, `crates/core/src/storage/admin.rs:403-414` |
| Existing quota tests (8) | `crates/core/src/mcp/server.rs:4535-4740` |
| SPEC-0010 delivered and not delivered rows | `sdlc/specs/SPEC-0010-project-storage-quota/spec.md` Section 6 |

Test command: `make check`.

### 14.2 Class sweep

Not applicable: not a security lot.

### 14.3 Design inputs

- The write choke points (`VolumeClient::write_bytes_atomic`, `copy_file`, `copy_tree`,
  `storage/volume.rs:157`, `:209`, `:222`) take no limit; enforcement needs the project limit and a
  usage figure checked atomically with the node write (FR-NEW-008). Signal: E2E-049, E2E-052.
- Usage per FR-NEW-002 is a sum of `nodes.size` per `volume_id`, trash included; a running counter or
  an aggregate query is a design choice; 100 000 files must not slow writes (7.1).
- Net change (FR-NEW-003) needs the previous size of each target path inside the same check.
- Multi file paths: `tools/archive.rs` pre scan charge (`archive.rs:183-187`), `git/merge.rs:705`
  charge then `:753` apply; folder `copy_tree` (`volume.rs:222`). Signal: E2E-028, E2E-030 to 034.
- Session budget charged after the limit check (FR-NEW-006): today `charge_write` runs first in
  every path (`fs_ops.rs:575`, `:611`). Signal: E2E-006.
- New refusal category: a new `ERR_*` code beside `ERR_WRITE_QUOTA_EXCEEDED` (`errors.rs:9-23`,
  mapping `:111-134`). Signal: E2E-002 asserts a distinct category.
- Version control writes all go through `merge::charge_and_apply` (`git/merge.rs:669`, `charge_quota`
  `:691`; callers `tools/git.rs:2290, 2399, 2590, 2742, 3911, 3967, 4612, 5338, 6618, 6670, 6884, 7089`);
  the project limit check belongs ahead of `charge_quota`. Signal: E2E-065, E2E-066.
- Read tool: no `admin.get_project_quota` exists; the owner or admin gate (`state.rs:47-54`) checks
  existence before ownership, contrary to FR-NEW-010. Signal: E2E-041.
- SPEC-0019 conversion paths charge the session budget once for sibling and artifacts
  (`core/fs_ops.rs` `land_conversion`); only the Markdown sibling counts to the project limit
  (FR-NEW-002). Signal: E2E-018, E2E-019.
- Purge lowers usage only when it permanently removes nodes (`purge.rs`). Signal: E2E-023.
- Not every version control write goes through `charge_and_apply`: `git.checkout_file`
  (`tools/git.rs:1403`, write at `:1404`) and remote clone (`tools/git.rs:3121`) charge directly; both need the check.
  Signal: E2E-031, E2E-069.
- The quota read is exposed on the MCP tool surface and the REST plane with identical payloads.
  Signal: E2E-037 through both drivers.
- The refusal needs a machine readable category distinct from `ERR_WRITE_QUOTA_EXCEEDED` and an HTTP
  status; the quota read needs an operation name and three result labels (the existing sibling
  returns `project_id`, `max_mb`, `mcp/server.rs:2789-2792`). These are contract shapes chosen by
  /sdlc-design, identical on both surfaces (FR-NEW-001, FR-NEW-009, constraint 3.3). Signal: the
  golden tool contract test and a parity test on E2E-002 and E2E-037.
- MCP and REST parity: a new tool needs `TOOL_CONTRACT.txt` and golden updates plus a REST route.
