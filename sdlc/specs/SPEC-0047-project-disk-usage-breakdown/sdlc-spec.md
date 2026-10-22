# Project disk usage breakdown: functional specification

> Id: SPEC-0047
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 2 areas touched (project administration, storage), 11 anticipated requirements, no persisted change, no new stored concept.
> Status: implemented
> Generated on: 2026-10-21
> Project: existing
> From backlog: BL-0041
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

Since a project's storage limit is enforced (SPEC-0020), a member who is refused a write needs to
know what takes the room. This increment lets a platform admin or the project owner read a project's
files sorted from the largest to the smallest, capped, with a clear sign when the list is cut. It
delivers the intent of SPEC-0010 FR-NEW-007 and FR-NEW-008, never built.

## 2. Current State

### 2.1 What users can do today

- A platform admin or the project owner can read three values for a project: its id, its limit in
  whole megabytes (no value when unlimited) and the bytes it uses now. A member or an outsider is
  refused with `'<person>' is not the owner of '<project>'`, also for a project that does not exist;
  an admin naming a missing project gets `project '<project>' not found`.
- Usage counts every file of the project, trashed files included, each path once even when files
  share content; version control history, zip exports and document artifacts are not counted.
- Nobody can see which files make up that usage. A refused write tells the member the limit, the
  usage and the bytes requested, and nothing about where the bytes are.
- Files deleted by a member stay in the trash, stored and counted, until the purge removes them.
- The same quota read is available to AI assistants and to programs calling the web interface, with
  identical results and refusals.

### 2.2 Specs governing this area

- SPEC-0020 (project quota enforcement, implemented): FR-NEW-002 defines what usage counts and
  FR-NEW-009 and FR-NEW-010 define who reads the quota and the refusal texts. This spec reuses all
  three unchanged. SPEC-0020 sets this capability out of scope (its section 3.2, BL-0041).
- SPEC-0010 (project storage quota, as built): FR-NEW-007 and FR-NEW-008 (breakdown and its refusal)
  were never delivered. This spec delivers their intent. SPEC-0010 is not modified.
- SPEC-0011 (trash listing and recovery): defines how a trashed file is presented with its original
  path. This spec presents trashed files consistently with it and does not change it.

### 2.3 What is verified today

The quota read and the enforcement are covered by 97 tests. No test covers a listing of a project's
files by size, because none exists.

## 3. Scope

### 3.1 In scope

- Reading a project's files sorted by size, largest first, with the limit and the usage.
- A cap on the list, and a sign that it was cut.
- Access limited to a platform admin and the project owner.
- The same result for AI assistants and for the web interface.

### 3.2 Out of scope, and why

| Non goal | Reason | Backlog |
|---|---|---|
| Totals per folder, or a folder tree of sizes | a different view; the file list answers the need first | BL-0070 |
| Reading the breakdown as a plain member | the quota read is owner or admin only; same audience kept | none, decided |
| Deleting from the breakdown | reading only; deleting already exists | none, decided |
| Counting history, zip exports, document artifacts | not project files from the user's view (SPEC-0020) | none, decided |
| Paging through the whole list | a cap is enough to find the largest files | none, decided |

### 3.3 Constraints imposed by the context

- Imposed by the project conventions: AI assistants and web interface consumers get the same
  behavior.
- Decision recorded in this spec: a platform admin sees file paths and sizes through this read. The
  backlog entry and SPEC-0010 both name the admin as an audience, although the project otherwise
  gives admins no implicit access to files.

## 4. Actors

| Actor | Description |
|---|---|
| Platform admin (Ada) | Reads the breakdown of any project. |
| Project owner (Olivia) | Reads the breakdown of her project. |
| Member (Mia) | Belongs to the project; refused. |
| Outsider (Otto) | Does not belong to the project; refused. |
| Other owner (Bram) | Owns another project; refused on this one. |

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Storage | the files a project holds and their sizes | usage, limit, file, trash, purge |
| Administration | who may read a project's quota and breakdown | platform admin, owner, breakdown |

## 5. Usage scenarios

### SC-001: The owner finds what takes the room
**Actor:** Project owner
**Preconditions:** project `Atlas` has a limit and stores files of different sizes, some in the trash.
**Flow:**
1. The owner asks for the breakdown of `Atlas`, optionally with a cap → the system shows the limit,
   the bytes used, the number of files, and the largest files first, up to the cap, saying whether
   the list was cut.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-001a: the cap is below 1 or above 500 → `limit must be between 1 and 500`.
**Interactions:** SC-004 (files change between two reads).

### SC-002: The admin reads any project
**Actor:** Platform admin
**Preconditions:** a project exists, with or without a limit, possibly over its limit.
**Flow:**
1. The admin asks for the breakdown of the project → same result as SC-001.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-002a: the project does not exist → `project '<project>' not found`.

### SC-003: Someone not allowed asks
**Actor:** Member, outsider, owner of another project
**Preconditions:** project `Atlas` exists.
**Flow:**
1. The person asks for the breakdown of `Atlas` → the system refuses with `'<person>' is not the
   owner of 'Atlas'` and shows nothing about the files.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-003a: the project does not exist → the same refusal, so existence cannot be probed.

### SC-004: The files change around a read
**Actor:** Project owner, a member
**Preconditions:** project `Atlas` stores files.
**Flow:**
1. A member deletes a file, the owner restores it, or the purge removes it → the next breakdown shows
   the file as `In trash`, as `Active` again, or no longer.
**Postconditions:** the breakdown always matches the usage read at the same moment.
**Exceptions:**
- EXC-004a: a write lands while the breakdown is being read → the figures describe the project either
  entirely before or entirely after that write.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Read the breakdown
> WHEN a platform admin or the project owner reads a project's breakdown THE Administration context
> SHALL show the project id, the limit in whole MB (no value when unlimited), the bytes used, the
> number of files, whether the list is cut, and the list of files.
- **Business rules:** the bytes used and the project id follow the quota read of SPEC-0020. The
  number of files is the count of all files of the project, whether or not they are in the list.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-002 [EARS-U]: Largest first
> The Administration context SHALL list the files from the largest to the smallest.
- **Business rules:** files of equal size are ordered by their path, comparing the characters one by
  one in character code order, so `/C.txt` comes before `/a.txt`. Files of 0 bytes come last.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-003 [EARS-U]: What each file shows
> The Administration context SHALL show, for each listed file, its full path, its size in bytes, and
> its state: `Active`, or `In trash`.
- **Business rules:** a file in the trash is shown under the path where it is stored in the trash and
  also shows the path it had before it was deleted. Folders are never listed.
- **User visible text:** states `Active` and `In trash`.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-004 [EARS-O]: The cap
> IF the reader gives a cap between 1 and 500, or none, THEN THE Administration context SHALL list at
> most that many files, 100 when none is given.
- **Business rules:** a cap of 1 and a cap of 500 are accepted. Any other number, whole or not, is refused with
  `limit must be between 1 and 500`, and nothing is shown. A cap is a number of files, not bytes.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-005 [EARS-E]: Say when the list is cut
> WHEN the project holds more files than are listed THE Administration context SHALL report the list
> as cut, and otherwise as complete.
- **Business rules:** a cap equal to the number of files gives a complete list; a cap one below gives
  a cut list. An empty project gives an empty, complete list.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-006 [EARS-U]: The same files as usage
> The Administration context SHALL base the breakdown on the files that usage counts: all files of
> the project, trashed files included, each path once even when files share content.
- **Business rules:** the sizes of all files, listed or not, add up exactly to the bytes used shown in
  the same answer. Two paths with the same content each show the full size. Version control history,
  zip exports and document artifacts are not files of the breakdown. A file removed by the purge no
  longer appears and leaves the bytes used.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-007 [EARS-UB]: Who may read
> The Administration context SHALL NOT show a project's breakdown to anyone who is neither a platform
> admin nor the project owner.
- **User visible text:** `'<person>' is not the owner of '<project>'`, given also when the project
  does not exist; an admin naming a missing project gets `project '<project>' not found`.
- **Business rules:** the refusal shows no file, size or count of the project. The owner of another
  project is refused on this one.
- **Scenario:** SC-002, SC-003 **Priority:** Must

#### FR-NEW-008 [EARS-U]: The limit as set
> The Administration context SHALL show the limit currently set for the project, no value when it is
> unlimited, whatever the usage.
- **Business rules:** a project over its limit (after the limit was lowered, SPEC-0020 FR-MOD-001) is
  readable and shows its usage above the limit. An unlimited project is readable.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-009 [EARS-UB]: Reading changes nothing
> The Administration context SHALL NOT change any file, the bytes used, the trash, or the reader's
> session write budget when it shows a breakdown.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-010 [EARS-U]: One moment
> The Administration context SHALL take the bytes used, the number of files and the list from the
> same moment.
- **Business rules:** a write landing during the read is seen entirely or not at all.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-011 [EARS-U]: Same for assistants and the web interface
> The Administration context SHALL give AI assistants and web interface consumers the same result and
> the same refusals for the same request.
- **Scenario:** SC-001, SC-003 **Priority:** Must

## 7. Non functional requirements (user observable)

### 7.1 Performance
A breakdown of a project of 100 000 files with the default cap is returned within 5 seconds.

### 7.2 Security and access
FR-NEW-007; the refusal order prevents probing which projects exist, as for the quota read.

### 7.5 Reliability
FR-NEW-009 and FR-NEW-010: a read never alters the project and never shows a half applied write.

### 7.6 Operability
An owner refused a write can name the largest files from one answer and free the room (the answer
carries the paths and sizes needed).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | project without files | 0 files, 0 bytes, empty complete list | E2E-022 |
| Boundary values | cap 1, cap 500, cap equal to the file count, one below | accepted; cut or complete per FR-NEW-005 | E2E-024, E2E-025, E2E-026 |
| Boundary values | cap 0, 501, -5 | refused | E2E-012, E2E-013, E2E-014 |
| Concurrent actions | write during the read | one moment | E2E-032 |
| Slow or lost connectivity | not applicable: the read is local to the product | n/a | n/a |
| Access states | admin, owner, member, outsider, other owner, missing project | per FR-NEW-007 | E2E-002, E2E-007 to E2E-011 |
| Data variations | accents, emoji, equal sizes, shared content, 0 byte files | shown intact; ordered per FR-NEW-002 | E2E-027 to E2E-030 |
| Navigation | not applicable: no navigation | n/a | n/a |
| Partial operations | not applicable: a read has no partial state | n/a | n/a |
| Dependency failures | not applicable: no new dependency | n/a | n/a |
| Ordering and timing | delete then restore, purge | state follows | E2E-005, E2E-019, E2E-020 |
| Exhaustion | project over its limit | readable | E2E-018 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

Shared setup unless stated: project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores five
files, all `Active`:

| Path | Bytes |
|---|---|
| `/video/demo.mp4` | 600 000 |
| `/data/export.csv` | 250 000 |
| `/docs/spec.docx` | 120 000 |
| `/docs/notes.txt` | 4 000 |
| `/readme.md` | 1 500 |

Together 975 500 bytes. Olivia (`olivia@example.com`) is the owner, Mia (`mia@example.com`) a member,
Ada (`ada@example.com`) a platform admin, Otto (`otto@example.com`) belongs to no project. Bram
(`bram@example.com`) owns project `Borealis`. "The standard answer" means: project `Atlas`, limit 2,
bytes used 975 500, 5 files, list complete, entries in the order of the table above, each `Active`.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, 002, 003, 006, 008, 010 | Critical |
| E2E-002 | New | happy | SC-002 | FR-NEW-001, 007 | Critical |
| E2E-003 | New | happy | SC-001 | FR-NEW-003, 006 | High |
| E2E-004 | New | happy | SC-001 | FR-NEW-011 | High |
| E2E-005 | New | happy | SC-004 | FR-NEW-001, 006 | High |
| E2E-006 | New | happy | SC-001 | FR-NEW-006, 010 | High |
| E2E-007 | New | failure | SC-003 | FR-NEW-007 | Critical |
| E2E-008 | New | failure | SC-003 | FR-NEW-007 | Critical |
| E2E-009 | New | failure | SC-003 | FR-NEW-007 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-007 | High |
| E2E-011 | New | failure | SC-003 | FR-NEW-007 | High |
| E2E-012 | New | failure | SC-001 | FR-NEW-004 | High |
| E2E-013 | New | failure | SC-001 | FR-NEW-004 | High |
| E2E-014 | New | failure | SC-001 | FR-NEW-004 | Medium |
| E2E-015 | New | failure | SC-003 | FR-NEW-007, 011 | High |
| E2E-016 | New | failure | SC-003 | FR-NEW-007 | High |
| E2E-017 | New | side effect | SC-001 | FR-NEW-009 | Critical |
| E2E-018 | New | side effect | SC-002 | FR-NEW-008, 009 | High |
| E2E-019 | New | state transition | SC-004 | FR-NEW-003, 006 | High |
| E2E-020 | New | state transition | SC-004 | FR-NEW-003 | High |
| E2E-021 | New | edge | SC-002 | FR-NEW-008 | High |
| E2E-022 | New | edge | SC-001 | FR-NEW-001, 003, 005 | High |
| E2E-023 | New | edge | SC-001 | FR-NEW-004, 005 | High |
| E2E-024 | New | edge | SC-001 | FR-NEW-004, 005 | Medium |
| E2E-025 | New | edge | SC-001 | FR-NEW-004, 005 | Medium |
| E2E-026 | New | edge | SC-001 | FR-NEW-001, 004, 005 | Medium |
| E2E-027 | New | edge | SC-001 | FR-NEW-002 | High |
| E2E-028 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-029 | New | edge | SC-001 | FR-NEW-006 | High |
| E2E-030 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-031 | New | edge | SC-001 | FR-NEW-006 | High |
| E2E-032 | New | edge | SC-004 | FR-NEW-009, 010 | High |
| E2E-033 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-034 | New | edge | SC-001 | FR-NEW-004, 011 | Medium |
| E2E-035 | New | edge | SC-001 | FR-NEW-004, 005, 011 | Medium |

Coverage: happy 6, failure 10, side effect 2, edge 15, state transition 2. Failure tests (10)
outnumber happy tests (6): ratio 1:1.7 in favor of failures.

### 9.2 New tests

#### E2E-001: The owner reads the breakdown
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002, 003, 006, 008, 010
- **Given** the shared setup
- **When** Olivia reads the breakdown of `Atlas` with no cap
- **Then** she gets the standard answer: five entries in the order `/video/demo.mp4` 600 000,
  `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500
- **And** the five sizes add up to 975 500, the bytes used shown

#### E2E-002: The admin reads the same breakdown
- **Category:** happy. **Scenario:** SC-002. **Requirements:** FR-NEW-001, 007
- **Given** the shared setup; Ada belongs to no project
- **When** Ada reads the breakdown of `Atlas`
- **Then** she gets the standard answer

#### E2E-003: A trashed file is listed as in the trash
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003, 006
- **Given** the shared setup, and Mia has deleted `/docs/notes.txt`
- **When** Olivia reads the breakdown
- **Then** the entry of size 4 000 is `In trash`, shows the path where it is stored in the trash and
  the original path `/docs/notes.txt`; the four other entries are `Active`
- **And** 5 files, bytes used 975 500, list complete

#### E2E-004: Assistants and the web interface agree
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-011
- **Given** the shared setup
- **When** Olivia reads the breakdown of `Atlas` once through an AI assistant and once through the web
  interface
- **Then** both answers carry exactly the same values, entries and order

#### E2E-005: A purged file disappears
- **Category:** happy. **Scenario:** SC-004. **Requirements:** FR-NEW-001, 006
- **Given** the shared setup, Mia has deleted `/docs/notes.txt`, and the purge has removed it
- **When** Olivia reads the breakdown
- **Then** 4 files, bytes used 971 500, no entry of size 4 000, list complete

#### E2E-006: The sizes add up to the usage
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-006, 010
- **Given** `Atlas` stores 120 files of 1 000 bytes, and Olivia reads with a cap of 20
- **When** the answer is returned
- **Then** bytes used is 120 000, files 120, 20 entries shown, list cut
- **And** reading again with a cap of 500 gives 120 entries whose sizes add up to 120 000

#### E2E-007: A member is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Mia reads the breakdown of `Atlas`
- **Then** she is refused with `'mia@example.com' is not the owner of 'Atlas'`

#### E2E-008: An outsider is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Otto reads the breakdown of `Atlas`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Atlas'`

#### E2E-009: An outsider naming a missing project is refused the same way
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup; no project `Ghost` exists
- **When** Otto reads the breakdown of `Ghost`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Ghost'`

#### E2E-010: The admin naming a missing project
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-007
- **Given** the shared setup; no project `Ghost` exists
- **When** Ada reads the breakdown of `Ghost`
- **Then** she is refused with `project 'Ghost' not found`

#### E2E-011: The owner of another project is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup, and Bram owns `Borealis`
- **When** Bram reads the breakdown of `Atlas`
- **Then** he is refused with `'bram@example.com' is not the owner of 'Atlas'`

#### E2E-012: A cap of 0 is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of 0
- **Then** she is refused with `limit must be between 1 and 500` and sees no file

#### E2E-013: A cap of 501 is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of 501
- **Then** she is refused with `limit must be between 1 and 500` and sees no file

#### E2E-014: A negative or fractional cap is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the shared setup
- **When** Olivia reads the breakdown with a cap of -5, then with a cap of 2.5
- **Then** each time she is refused with `limit must be between 1 and 500` and sees no file

#### E2E-015: The web interface refuses like the assistants
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007, 011
- **Given** the shared setup
- **When** Mia reads the breakdown of `Atlas` through the web interface and, separately, through an AI
  assistant
- **Then** both refuse with `'mia@example.com' is not the owner of 'Atlas'`

#### E2E-016: A refusal reveals nothing about the files
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007
- **Given** the shared setup
- **When** Otto reads the breakdown of `Atlas`
- **Then** the refusal contains none of the paths, none of the sizes and no file count of `Atlas`

#### E2E-017: Reading changes nothing
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** the shared setup, and Olivia has written 100 000 bytes this session (budget 262 144)
- **When** Olivia reads the breakdown of `Atlas` three times
- **Then** all three answers are identical
- **And** the quota read still shows 975 500 bytes used, the five files are unchanged, and Olivia can
  still write 162 144 bytes in this session (a write of 162 145 bytes is refused with
  `session write quota of 262144 bytes exceeded`)

#### E2E-018: A project over its limit is readable
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-008, 009
- **Given** `Atlas` stores 3 files of 700 000, 500 000 and 300 000 bytes (1 500 000 in all), and Ada
  has lowered its limit to 1 MB
- **When** Olivia reads the breakdown
- **Then** limit 1, bytes used 1 500 000, 3 files in the order 700 000, 500 000, 300 000, complete
- **And** nothing was removed from the project

#### E2E-019: Deleting moves a file to the trash
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-003, 006
- **Given** the shared setup
- **When** Mia deletes `/data/export.csv`, then Olivia reads the breakdown
- **Then** 5 files, bytes used 975 500; the second entry has size 250 000, is `In trash` and shows the
  original path `/data/export.csv`; its position among the entries is unchanged

#### E2E-020: Restoring brings a file back to Active
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-003
- **Given** the shared setup, and `/data/export.csv` deleted then restored by Mia
- **When** Olivia reads the breakdown
- **Then** the standard answer, every entry `Active`

#### E2E-021: An unlimited project
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-008
- **Given** `Atlas` has no limit and stores the five files of the shared setup
- **When** Ada reads the breakdown
- **Then** the limit is shown as no value (neither 0 nor text), bytes used 975 500, 5 entries in order

#### E2E-022: An empty project
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 003, 005
- **Given** `Atlas` stores no file
- **When** Olivia reads the breakdown
- **Then** 0 files, bytes used 0, an empty list reported as complete

#### E2E-023: The default cap is 100
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads the breakdown with no cap
- **Then** 100 entries are shown, 120 files are counted, bytes used 120 000, the list is reported as cut

#### E2E-024: A cap equal to the number of files
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads with a cap of 120
- **Then** 120 entries, the list is reported as complete

#### E2E-025: A cap one below the number of files
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads with a cap of 119
- **Then** 119 entries, 120 files counted, the list is reported as cut

#### E2E-026: The smallest and the largest accepted caps
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 004, 005
- **Given** the shared setup
- **When** Olivia reads with a cap of 1, then with a cap of 500
- **Then** the cap of 1 shows only `/video/demo.mp4` (600 000), 5 files counted, the list cut
- **And** the cap of 500 shows the five entries, the list complete

#### E2E-027: Equal sizes are ordered by path
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** `Atlas` stores `/b.txt`, `/a.txt` and `/C.txt`, each 10 bytes
- **When** Olivia reads the breakdown
- **Then** the order is `/C.txt`, `/a.txt`, `/b.txt`

#### E2E-028: Empty files come last
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** `Atlas` stores `/empty.txt` (0 bytes), `/a.txt` (10 bytes) and `/b.txt` (20 bytes)
- **When** Olivia reads the breakdown
- **Then** the order is `/b.txt`, `/a.txt`, `/empty.txt`, and `/empty.txt` shows size 0

#### E2E-029: Files sharing content each show their full size
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** `Atlas` stores `/a/copy1.bin` and `/a/copy2.bin` with identical content of 100 000 bytes
- **When** Olivia reads the breakdown
- **Then** both entries show 100 000 and bytes used is 200 000

#### E2E-030: Accents and emoji stay intact
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** `Atlas` stores `/rapports/été 📊.xlsx` of 5 000 bytes
- **When** Olivia reads the breakdown
- **Then** the entry shows the path `/rapports/été 📊.xlsx` exactly, size 5 000

#### E2E-031: History, exports and artifacts are not counted
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** `Atlas` stores one file `/a.txt` of 10 000 bytes, has version control history of earlier
  versions, one zip export, and two images kept from a converted document
- **When** Olivia reads the breakdown
- **Then** 1 file, bytes used 10 000, one entry `/a.txt` 10 000

#### E2E-032: A write during the read is seen entirely or not at all
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, 010
- **Given** the shared setup
- **When** Mia writes `/docs/new.txt` of 30 000 bytes while Olivia reads the breakdown
- **Then** the answer is either the standard answer (5 files, 975 500) or the answer with 6 files and
  bytes used 1 005 500 including `/docs/new.txt`; in both cases the entries add up to the bytes used
  and the file count matches
- **And** a second read afterwards shows 6 files, bytes used 1 005 500

#### E2E-033: Folders are not listed
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the shared setup, which holds the folders `/video`, `/data` and `/docs`
- **When** Olivia reads the breakdown
- **Then** no entry is a folder; the file count is 5

#### E2E-034: The cap through the web interface
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, 011
- **Given** the shared setup
- **When** Olivia reads the breakdown through the web interface with a cap of 2
- **Then** two entries (`/video/demo.mp4`, `/data/export.csv`), 5 files counted, the list cut, and
  with a cap of 0 the refusal `limit must be between 1 and 500`

#### E2E-035: The default cap through the web interface
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005, 011
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads the breakdown through the web interface with no cap
- **Then** 100 entries shown, 120 files counted, the list cut, identical to the assistant answer

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001 to 006, 008, 009, 011 | E2E-001, 003, 004, 006 | E2E-012, 013, 014 | E2E-022 to 031, 033 to 035 |
| SC-002 | FR-NEW-001, 007, 008 | E2E-002 | E2E-010 | E2E-021 |
| SC-003 | FR-NEW-007, 011 | E2E-002 | E2E-007, 008, 009, 011, 015, 016 | E2E-009 |
| SC-004 | FR-NEW-003, 006, 009, 010 | E2E-005 | E2E-032 (concurrent write) | E2E-019, 020, 032 |

Requirement to test (at least 3 each):

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, 002, 005, 022, 026 |
| FR-NEW-002 | E2E-001, 027, 028 |
| FR-NEW-003 | E2E-001, 003, 019, 020, 022, 030, 033 |
| FR-NEW-004 | E2E-012, 013, 014, 023, 024, 025, 026, 034, 035 |
| FR-NEW-005 | E2E-022, 023, 024, 025, 026, 035 |
| FR-NEW-006 | E2E-001, 003, 005, 006, 019, 029, 031 |
| FR-NEW-007 | E2E-002, 007, 008, 009, 010, 011, 015, 016 |
| FR-NEW-008 | E2E-001, 018, 021 |
| FR-NEW-009 | E2E-017, 018, 032 |
| FR-NEW-010 | E2E-001, 006, 032 |
| FR-NEW-011 | E2E-004, 015, 034, 035 |

## 11. Open questions

None.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| Breakdown | a project's files listed from the largest to the smallest, with its limit and usage | Administration |
| Usage (bytes used) | the sum of the declared sizes of all files of a project, trashed included | Storage |
| Cap | the largest number of files the breakdown lists | Administration |
| Cut list | a list showing fewer files than the project holds | Administration |
| `In trash` | a deleted file still stored and counted, until the purge | Storage |
| `Active` | a file that is not in the trash | Storage |
| Purge | the removal of trashed files after their retention | Storage |
| Platform admin | a person allowed to manage projects | Administration |
| Owner | the person who owns a project | Administration |

## 13. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 (inline checklist; the independent auditor could not run, provider weekly limit) | 0 | 4 | IMPLEMENTABLE |

**Amendments applied:** A1 Makefile citation corrected to `Makefile:51`; A2 "no listing" evidence
reworded; A3 FR-NEW-004 and E2E-014 now fix the refusal of a non whole cap; A4 E2E-017 now observes
the session budget through the refusal text it produces instead of an unobservable counter.
**No implementation detail outside Section 14:** PASS (scanned Sections 1 to 13)
**Concern:** the independent auditor of the contract did not run. Re run it before design if wanted.

## 14. Evidence appendix

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| Quota read shows id, limit, used bytes; owner or admin; refusal texts | `crates/core/src/tools/admin.rs:647-676` |
| Same read for assistants and the web interface | `crates/core/src/mcp/server.rs:2918-2928`, `crates/core/src/api/dataplane.rs:398-412` |
| Usage is one counter, read under the admission lock | `crates/core/src/quota.rs:164-167`, `crates/core/src/storage/meta.rs:1751` |
| Trashed files are stored under a flattened stamped path | `crates/core/src/safety.rs:225` |
| Trash rows hold the original path | `crates/core/src/storage/meta.rs:216-231` |
| No listing by size exists | `rg -n 'disk_usage|get_project_disk_usage' crates/core/src` returns no match (zero files) |
| 97 quota tests | `rg -c '#\[tokio::test' crates/core/src/tools/quota_e2e.rs` gives 97 |

Test command: `make check` (`Makefile:51`); `./test.sh` runs `cargo test --workspace`.

### 14.3 Design inputs

- Ordering by path in character code order (FR-NEW-002) must hold on every database engine; engines
  differ in default text collation (`crates/core/src/storage/rel/dialect.rs`), so the design must not
  rely on the engine's order for ties. Signal: a test with `/C.txt`, `/a.txt` (E2E-027) on PostgreSQL.
- The used bytes come from a counter (`quota.rs:164`) while the list comes from the files: FR-NEW-006
  and FR-NEW-010 need both read under the same admission lock or transaction. Signal: E2E-006 and
  E2E-032.
- Trashed entries need their original path from the trash rows; files in the trash without a row exist
  (lazy backfill, `crates/core/src/tools/trash.rs`). Signal: E2E-003 on a legacy trashed file.
- The frozen tool contract (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`) and the tool count in
  `AGENTS.md` change with a new tool. Signal: the golden contract tests.
- Sorting 100 000 files within 5 seconds (section 7.1): needs an index or a bounded top N selection.
  Signal: a performance test at that size.
