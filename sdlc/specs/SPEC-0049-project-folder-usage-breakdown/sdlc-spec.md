# Project folder usage breakdown: functional specification

> Id: SPEC-0049
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 2 areas touched (project administration, storage), 12 requirements, no persisted change, no new stored concept.
> Status: designed
> Generated on: 2026-10-22
> Project: existing
> From backlog: BL-0070
> Depends on: SPEC-0047
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

An admin or the project owner can already list a project's files from the largest to the smallest
(SPEC-0047), but a folder holding thousands of small files never ranks there, so the real cost stays
hidden. This increment lets them read one folder at a time: the folder's direct items, folders and
files mixed, largest first, where a folder's size is everything below it. Starting at the root and
drilling down finds what to delete once the project limit refuses writes.

## 2. Current State

### 2.1 What users can do today

- A platform admin or the project owner can read the project's limit in MB (no value when unlimited)
  and the bytes used; a member or an outsider is refused with `'<person>' is not the owner of
  '<project>'`, also for a project that does not exist; an admin naming a missing project gets
  `project '<project>' not found`.
- The same two callers can list the project's files from the largest to the smallest, trashed files
  included, with a cap from 1 to 500 (100 by default), a sign when the list is cut, and the refusal
  `limit must be between 1 and 500` for any other cap. Equal sizes are ordered by path in character code
  order. Assistants and the web interface give the same answer.
- Nothing shows the size of a folder or its number of files. A folder holding 1 000 files of 100 bytes
  does not rank among the largest files.
- A deleted file stays stored and counted in the trash folder (named `.mcp_trash` by default) until
  the purge removes it.

### 2.2 Specs governing this area

- SPEC-0047 (project disk usage breakdown, implemented): FR-NEW-001 to 012 define the file list, its
  audience, cap and refusals. This spec reuses the audience, the refusal texts, the cap, the cut sign and
  the rule for what is counted. SPEC-0047 is not modified; its file list stays as it is (Section 7.5).
- SPEC-0020 (project quota enforcement, implemented): FR-NEW-002 defines what usage counts (all files,
  trashed ones included, each path once). Reused unchanged.
- BL-0070 (this entry) asked for totals per folder, "rolled up, largest first".

### 2.3 What is verified today

The file list is covered by 36 tests; the usage and limit reads by 97 tests. No test covers a folder read.

## 3. Scope

### 3.1 In scope

- Reading one folder of a project: the folder's own size and file count, and its direct items (folders and files) largest first.
- A folder's size is the total of every file below it, at any depth.
- Drilling down by naming a folder; the root folder by default.
- The cap, the cut sign, the audience and the refusals of the file list, plus three refusals for a bad folder.
- The same answer for assistants and for the web interface.

### 3.2 Out of scope, and why

| Non goal | Reason | Backlog |
|---|---|---|
| A flat list of every folder at every depth | each parent would appear again through its children; drilling down answers the need | none, decided |
| Deleting or moving from this view | reading only; deleting already exists | none, decided |
| Sorting by anything but size | the need is to find what weighs | none, decided |
| A whole tree in one answer | unbounded size; the cap and the folder choice bound it | none, decided |
| Reading as a plain member | same audience as the quota and the file list | none, decided |

### 3.3 Constraints imposed by the context

- Imposed by the project conventions: AI assistants and web interface consumers get the same behavior.
- Decision recorded in this spec: a platform admin sees folder and file names and sizes through this read, as for the file list (SPEC-0047 section 3.3).

## 4. Actors

| Actor | Description |
|---|---|
| Platform admin (Ada) | Reads the folder breakdown of any project. |
| Project owner (Olivia) | Reads her project's. |
| Member (Mia) | Belongs to the project; refused. |
| Outsider (Otto) | Belongs to no project; refused. |
| Other owner (Bram) | Owns another project; refused on this one. |

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Storage | the files and folders a project holds and their sizes | usage, limit, file, folder, trash folder, purge |
| Administration | who may read a project's quota and breakdowns | platform admin, owner, folder breakdown |

## 5. Usage scenarios

### SC-001: The owner finds the heavy folder
**Actor:** Project owner
**Preconditions:** project `Atlas` has a limit and stores files in folders, some deleted into the trash.
**Flow:**
1. The owner reads the folder breakdown of `Atlas` with no folder → the system shows the project's limit, bytes used, the root folder's size and file count, and the root's direct items largest first.
2. She names the heaviest folder → the system shows that folder's size and file count and its direct items largest first.
3. She repeats until she reaches the files to delete.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-001a: the cap is below 1 or above 500 → `limit must be between 1 and 500`.
- EXC-001b: the folder does not exist → `folder '<path>' not found`.
- EXC-001c: the name is a file → `'<path>' is not a folder`.
- EXC-001d: the name does not start with `/` → `path must start with /`.
**Interactions:** SC-004 (files change between reads).

### SC-002: The admin reads any project
**Actor:** Platform admin
**Preconditions:** a project exists, with or without a limit, possibly over its limit.
**Flow:**
1. The admin reads the folder breakdown → same answer as SC-001.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-002a: the project does not exist → `project '<project>' not found`.

### SC-003: Someone not allowed asks
**Actor:** Member, outsider, owner of another project
**Preconditions:** project `Atlas` exists.
**Flow:**
1. The person reads the folder breakdown of `Atlas` → refused with `'<person>' is not the owner of 'Atlas'`, with nothing about the files.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-003a: the project does not exist → the same refusal, so existence cannot be probed.

### SC-004: The files change around a read
**Actor:** Project owner, a member
**Preconditions:** project `Atlas` stores files.
**Flow:**
1. A member deletes a file, the owner restores it, the purge removes it, or a member writes a file → the next breakdown shows the bytes in the trash folder, back in their folder, gone, or added.
**Postconditions:** every breakdown matches the usage read at the same moment.
**Exceptions:**
- EXC-004a: a write lands during a read → the figures describe the project entirely before or entirely after that write.

### SC-005: The same read on the web interface
**Actor:** Program calling the web interface
**Preconditions:** as SC-001.
**Flow:**
1. The program asks for a folder breakdown → it receives the answer an assistant would receive.
**Postconditions:** nothing changed.
**Exceptions:**
- EXC-005a: any refusal of SC-001, SC-002 or SC-003 → the same refusal text.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Read a folder
> WHEN a platform admin or the project owner reads a project's folder breakdown THE Administration context
> SHALL show the project id, the limit in whole MB (no value when unlimited), the bytes used, the folder
> read with its size in bytes and its number of files, the number of items listed, whether the list is
> cut, and the items.
- **Business rules:** with no folder given the folder is `/`; the folder's size and file count count every file below it.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-002 [EARS-U]: Direct items, rolled up
> The Storage context SHALL list as items the direct children of the folder, folders and files together,
> and SHALL give a folder item the total size and the number of files of everything below it, at any depth.
- **Business rules:** a folder holding no file is listed with 0 bytes and 0 files; a file item shows no file count.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-003 [EARS-U]: What each item shows
> The Administration context SHALL show, for each item, its full path, its kind (`Folder` or `File`), its
> size in bytes, and for a folder its number of files.
- **User visible text:** kinds `Folder` and `File`.
- **Business rules:** the trash folder (`.mcp_trash` by default) is listed like any other folder: once a first delete has created it, it stays, and an emptied trash folder is listed with 0 bytes and 0 files.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-004 [EARS-U]: Largest first
> The Administration context SHALL list the items from the largest to the smallest.
- **Business rules:** items of equal size are ordered by path, comparing the characters one by one in character code order, so `/t/C.txt` comes before `/t/a.txt`; items of 0 bytes come last, in the same order.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-005 [EARS-O]: The cap
> IF the reader gives a cap between 1 and 500, or none, THEN THE Administration context SHALL list at most
> that many items, 100 when none is given.
- **Business rules:** 1 and 500 are accepted; any other number, whole or not, is refused with `limit must be between 1 and 500` and nothing is shown.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-006 [EARS-E]: Say when the list is cut
> WHEN the folder holds more items than are listed THE Administration context SHALL report the list as cut,
> and otherwise as complete.
- **Business rules:** a cap equal to the number of items gives a complete list; one below gives a cut list; a folder with no item gives an empty, complete list.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-007 [EARS-U]: The same bytes as usage
> The Storage context SHALL count the same files as usage counts: all files of the project, trashed files
> included, each path once even when files share content.
- **Business rules:** the sizes of all items, listed or not, add up to the folder's size; the root folder's size equals the bytes used shown; a folder whose name starts another folder's name (`/doc`, `/docs`) never absorbs the other's files; version control history, zip exports and document artifacts are not counted.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-008 [EARS-UB]: Who may read
> The Administration context SHALL NOT show a project's folder breakdown to anyone who is neither a platform
> admin nor the project owner.
- **User visible text:** `'<person>' is not the owner of '<project>'`, given also when the project does not exist; an admin naming a missing project gets `project '<project>' not found`.
- **Business rules:** a refusal shows no name, size or count of the project; the owner of another project is refused; the cap is checked before the project, the project before the folder.
- **Scenario:** SC-002, SC-003 **Priority:** Must

#### FR-NEW-009 [EARS-O]: The folder asked for
> IF the folder asked for does not exist, is a file, or does not start with `/` THEN THE Administration
> context SHALL refuse and show nothing.
- **User visible text:** `folder '<path>' not found`; `'<path>' is not a folder`; `path must start with /`, with `<path>` as the reader gave it.
- **Business rules:** a single `/` is the root folder, shown as `/`; for any other text, exactly one trailing `/` is dropped before matching, but only when the text before it does not itself end with `/`, so `/video/` is the folder `/video`, shown as `/video`, while `/video//`, `//` and `///` are refused with `folder '<path>' not found`; an empty folder name is refused with `path must start with /`; a name with a `.` or `..` segment or an empty segment (`//video`) is refused with `folder '<path>' not found`; names are case sensitive (`/Video` is `folder '/Video' not found`); every refusal echoes the path exactly as the reader gave it, trailing `/` included.
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-NEW-010 [EARS-U]: Reading changes nothing, from one moment
> The Administration context SHALL NOT change any file, the bytes used, the trash, the reader's session write
> budget or the answer of the file list of SPEC-0047 when it shows a folder breakdown, and SHALL take every
> figure of one answer from the same moment.
- **Business rules:** a write landing during the read is seen entirely or not at all.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-011 [EARS-U]: The limit as set
> The Administration context SHALL show the limit currently set for the project, no value when it is
> unlimited, whatever the usage.
- **Business rules:** a project over its limit and an unlimited project are readable.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-012 [EARS-U]: Same for assistants and the web interface
> The Administration context SHALL give AI assistants and web interface consumers the same result and the
> same refusals for the same request.
- **Scenario:** SC-005 **Priority:** Must

## 7. Non functional requirements (user observable)

### 7.1 Performance
A folder breakdown of a project of 100 000 files with the default cap is returned within 5 seconds.

### 7.2 Security and access
FR-NEW-008; the refusal order prevents probing which projects exist, as for the quota and the file list.

### 7.5 Reliability
FR-NEW-010: a read never alters the project and never shows half of a write. The file list of SPEC-0047
answers exactly as before (E2E-022). Section 7.1 is asserted by E2E-039.

### 7.6 Operability
An owner refused a write can reach the files to delete by following the heaviest folder from the root, each answer giving the paths and sizes needed.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | empty folder; emptied trash folder | 0 bytes, 0 files, no item, complete; listed | E2E-025, 026 |
| Boundary values | cap 1, equal to the item count, one below; 0, 501, -5, 2.5 | per FR-NEW-005, 006 | E2E-012, 013, 014, 028 |
| Concurrent actions | write during the read | one moment | E2E-037 |
| Slow or lost connectivity | not applicable: the read is local to the product | n/a | n/a |
| Access states | admin, owner, member, outsider, other owner, missing project | per FR-NEW-008 | E2E-002, 007 to 011, 018, 019 |
| Data variations | accents, emoji, shared content, names starting alike, equal sizes, empty items | per FR-NEW-002, 004, 007 | E2E-029 to 034 |
| Navigation | trailing slash, odd names, five levels, missing or wrong folder | per FR-NEW-009 | E2E-015 to 017, 020, 027, 032, 038 |
| Partial operations | not applicable: a read has no partial state | n/a | n/a |
| Dependency failures | not applicable: no new dependency | n/a | n/a |
| Ordering and timing | delete, restore, purge | state follows | E2E-005, 023 to 025 |
| Exhaustion | project over its limit; unlimited project; 100 000 files | readable; within 5 seconds | E2E-035, 036, 039 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

Shared setup unless stated: project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files,
all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000,
`/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes
(1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada
(`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram
(`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-011 | Critical |
| E2E-002 | New | happy | SC-002 | FR-NEW-001, FR-NEW-008 | Critical |
| E2E-003 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003 | Critical |
| E2E-004 | New | happy | SC-001 | FR-NEW-002, FR-NEW-004, FR-NEW-006 | Critical |
| E2E-005 | New | happy | SC-004 | FR-NEW-003, FR-NEW-007 | High |
| E2E-006 | New | happy | SC-005 | FR-NEW-012 | High |
| E2E-007 | New | failure | SC-003 | FR-NEW-008 | Critical |
| E2E-008 | New | failure | SC-003 | FR-NEW-008 | Critical |
| E2E-009 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-008 | High |
| E2E-011 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-012 | New | failure | SC-001 | FR-NEW-005 | High |
| E2E-013 | New | failure | SC-001 | FR-NEW-005 | High |
| E2E-014 | New | failure | SC-001 | FR-NEW-005 | Medium |
| E2E-015 | New | failure | SC-001 | FR-NEW-009 | High |
| E2E-016 | New | failure | SC-001 | FR-NEW-009 | High |
| E2E-017 | New | failure | SC-001 | FR-NEW-009 | High |
| E2E-018 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-019 | New | failure | SC-005 | FR-NEW-008, FR-NEW-012 | High |
| E2E-020 | New | failure | SC-005 | FR-NEW-009, FR-NEW-012 | High |
| E2E-021 | New | side effect | SC-001 | FR-NEW-010 | Critical |
| E2E-022 | New | side effect | SC-001 | FR-NEW-010 | High |
| E2E-023 | New | state transition | SC-004 | FR-NEW-007, FR-NEW-003 | High |
| E2E-024 | New | state transition | SC-004 | FR-NEW-007 | High |
| E2E-025 | New | state transition | SC-004 | FR-NEW-007 | High |
| E2E-026 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-006 | High |
| E2E-027 | New | edge | SC-001 | FR-NEW-009 | Medium |
| E2E-028 | New | edge | SC-001 | FR-NEW-005, FR-NEW-006 | High |
| E2E-029 | New | edge | SC-001 | FR-NEW-004 | High |
| E2E-030 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-031 | New | edge | SC-001 | FR-NEW-003 | Medium |
| E2E-032 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-033 | New | edge | SC-001 | FR-NEW-002, FR-NEW-007 | High |
| E2E-034 | New | edge | SC-001 | FR-NEW-007 | High |
| E2E-035 | New | edge | SC-002 | FR-NEW-011 | High |
| E2E-036 | New | edge | SC-002 | FR-NEW-011 | High |
| E2E-037 | New | edge | SC-004 | FR-NEW-010 | High |
| E2E-038 | New | failure | SC-001 | FR-NEW-009 | High |
| E2E-039 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | High |
 
Coverage: happy 6, failure 15, side effect 2, state transition 3, edge 13, security 0.
Happy to failure ratio 1:2.5 (must beat 1:1).

### 9.2 New tests

#### E2E-001: The owner reads the root folder
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-011. **Priority:** Critical
- **Given** the shared setup
- **When** Olivia reads the folder breakdown of `Atlas` with no folder and no cap
- **Then** she sees project `Atlas`, limit 2 MB, bytes used 1 225 500, folder `/` of 1 225 500 bytes holding 7 files, 4 items, list complete: `/video` Folder 800 000 bytes (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/readme.md` File 1 500
- **And** the four item sizes add up to 1 225 500, the folder total

#### E2E-002: The admin reads the same root
- **Category:** happy. **Scenario:** SC-002. **Requirements:** FR-NEW-001, FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Ada reads the folder breakdown of `Atlas` with no folder
- **Then** she gets exactly the answer of E2E-001

#### E2E-003: Drilling into a folder
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003. **Priority:** Critical
- **Given** the shared setup
- **When** Olivia reads the folder `/video` of `Atlas`
- **Then** she sees folder `/video` of 800 000 bytes holding 2 files, 2 items, list complete: `/video/demo.mp4` File 600 000 bytes, `/video/clips` Folder 200 000 bytes (1 file)

#### E2E-004: A folder of many small files outranks one bigger file
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-004, FR-NEW-006. **Priority:** Critical
- **Given** `Atlas` stores `/logs/l0001.txt` to `/logs/l1000.txt` of 100 bytes each and `/big.bin` of 90 000 bytes, nothing else
- **When** Olivia reads the folder breakdown of `Atlas` with no folder
- **Then** bytes used is 190 000 and the items are, in order: `/logs` Folder 100 000 bytes (1 000 files), `/big.bin` File 90 000 bytes

#### E2E-005: The trash folder shows what the trash weighs
- **Category:** happy. **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-007. **Priority:** High
- **Given** the shared setup, and Mia has deleted `/docs/notes.txt`
- **When** Olivia reads the folder breakdown of `Atlas` with no folder
- **Then** bytes used is still 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 120 000 (1 file), `/.mcp_trash` Folder 4 000 (1 file), `/readme.md` File 1 500
- **And** the item sizes add up to 1 225 500

#### E2E-006: Assistants and the web interface agree
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown of `Atlas` for no folder, then for `/video/`, each once through an AI assistant and once through the web interface
- **Then** for each of the two folders both interfaces carry exactly the same values, items and order

#### E2E-007: A member is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Mia reads the folder breakdown of `Atlas`
- **Then** she is refused with `'mia@example.com' is not the owner of 'Atlas'`

#### E2E-008: An outsider is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** Critical
- **Given** the shared setup
- **When** Otto reads the folder breakdown of `Atlas`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Atlas'`

#### E2E-009: An outsider naming a missing project is refused the same way
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup; no project `Ghost` exists
- **When** Otto reads the folder breakdown of `Ghost`
- **Then** he is refused with `'otto@example.com' is not the owner of 'Ghost'`

#### E2E-010: The admin naming a missing project
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup; no project `Ghost` exists
- **When** Ada reads the folder breakdown of `Ghost`
- **Then** she is refused with `project 'Ghost' not found`

#### E2E-011: The owner of another project is refused
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup, and Bram owns `Borealis`
- **When** Bram reads the folder breakdown of `Atlas`
- **Then** he is refused with `'bram@example.com' is not the owner of 'Atlas'`

#### E2E-012: A cap of 0 is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of 0
- **Then** she is refused with `limit must be between 1 and 500` and sees no item

#### E2E-013: A cap of 501 is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of 501
- **Then** she is refused with `limit must be between 1 and 500` and sees no item

#### E2E-014: A negative or fractional cap is refused
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-005. **Priority:** Medium
- **Given** the shared setup
- **When** Olivia reads the folder breakdown with a cap of -5, then with a cap of 2.5
- **Then** each time she is refused with `limit must be between 1 and 500` and sees no item

#### E2E-015: A folder that does not exist
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/nothing` of `Atlas`
- **Then** she is refused with `folder '/nothing' not found`

#### E2E-016: A file given as the folder
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/readme.md` of `Atlas`
- **Then** she is refused with `'/readme.md' is not a folder`

#### E2E-017: A folder path without a leading slash
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `video` of `Atlas`
- **Then** she is refused with `path must start with /`

#### E2E-018: A refusal reveals nothing about the files
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** the shared setup
- **When** Otto reads the folder breakdown of `Atlas`
- **Then** the refusal contains none of the names `video`, `readme`, none of the sizes `800000`, `1225500`, and no file count of `Atlas`

#### E2E-019: The web interface refuses like the assistants
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-008, FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Mia reads the folder breakdown of `Atlas` through the web interface and, separately, through an AI assistant
- **Then** both refuse with `'mia@example.com' is not the owner of 'Atlas'`

#### E2E-020: The web interface reports a missing folder like the assistants
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-009, FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/nothing` through the web interface and, separately, through an AI assistant
- **Then** both refuse with `folder '/nothing' not found`

#### E2E-021: Reading changes nothing
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** Critical
- **Given** the shared setup, and Olivia has written `/session.bin` of 100 000 bytes this session (budget 262 144)
- **When** Olivia reads the folder breakdown of `Atlas` three times with no folder and once for `/video`
- **Then** the three answers for no folder are identical: bytes used 1 325 500, items in order `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/session.bin` File 100 000, `/readme.md` File 1 500
- **And** the usage read still shows 1 325 500 bytes used, the files are unchanged, and Olivia can still write 162 144 bytes in this session (a write of 162 145 bytes is refused with `session write quota of 262144 bytes exceeded`)

#### E2E-022: The file list of the previous breakdown is unchanged
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the file list (largest files first) before and after reading the folder breakdown twice
- **Then** both file lists are identical: bytes used 1 225 500, 7 files, in order `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/video/clips/a.mp4` 200 000, `/docs/spec.docx` 120 000, `/data/old/x.csv` 50 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500, list complete

#### E2E-023: Deleting moves bytes into the trash folder
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-003. **Priority:** High
- **Given** the shared setup
- **When** Mia deletes `/data/export.csv`, then Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/.mcp_trash` Folder 250 000 (1 file), `/docs` Folder 124 000 (2 files), `/data` Folder 50 000 (1 file), `/readme.md` File 1 500

#### E2E-024: Restoring brings the bytes back
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the shared setup, `/data/export.csv` deleted then restored by Mia
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 225 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 124 000 (2 files), `/readme.md` File 1 500, followed by `/.mcp_trash` Folder 0 bytes (0 files); 5 items, list complete
- **And** the item sizes add up to 1 225 500

#### E2E-025: A purge removes the bytes
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the shared setup, `/docs/notes.txt` deleted by Mia and then removed for good by the purge
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 1 221 500 and the items are, in order: `/video` Folder 800 000 (2 files), `/data` Folder 300 000 (2 files), `/docs` Folder 120 000 (1 file), `/readme.md` File 1 500, followed by `/.mcp_trash` Folder 0 bytes (0 files); 5 items, list complete
- **And** the item sizes add up to 1 221 500

#### E2E-026: An empty folder
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-006. **Priority:** High
- **Given** the shared setup, and Olivia has created the empty folder `/empty`
- **When** Olivia reads the folder `/empty`, then the folder breakdown with no folder
- **Then** `/empty` shows 0 bytes, 0 files, 0 items, list complete
- **And** with no folder, `/empty` is listed as a Folder of 0 bytes (0 files) after the four non empty items

#### E2E-027: A trailing slash means the same folder
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** Medium
- **Given** the shared setup
- **When** Olivia reads the folder `/video/` and the folder `/video`
- **Then** the two answers are identical, including the folder shown as `/video`

#### E2E-028: Cap boundaries
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-006. **Priority:** High
- **Given** `Atlas` stores the folder `/many` holding `/many/f000.bin` to `/many/f119.bin` of 1 000 bytes each (120 files)
- **When** Olivia reads the folder `/many` with no cap, with a cap of 120, with a cap of 119 and with a cap of 1
- **Then** no cap shows 100 items and the list is cut; a cap of 120 shows 120 items and the list is complete; a cap of 119 shows 119 items and the list is cut; a cap of 1 shows only `/many/f000.bin` and the list is cut
- **And** each answer reports the folder as 120 000 bytes holding 120 files

#### E2E-029: Equal sizes are ordered by name
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004. **Priority:** High
- **Given** `Atlas` stores `/t/b.txt`, `/t/a.txt` and `/t/C.txt` of 10 bytes each and the folder `/t/sub` holding one file of 10 bytes
- **When** Olivia reads the folder `/t`
- **Then** the order is `/t/C.txt`, `/t/a.txt`, `/t/b.txt`, `/t/sub` (names compared character by character in character code order)

#### E2E-030: Empty items come last
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004. **Priority:** Medium
- **Given** `Atlas` stores `/z/a.txt` of 10 bytes, `/z/empty.txt` of 0 bytes and the empty folder `/z/e`
- **When** Olivia reads the folder `/z`
- **Then** the order is `/z/a.txt` File 10 bytes, `/z/e` Folder 0 bytes (0 files), `/z/empty.txt` File 0 bytes

#### E2E-031: Accents and emoji in a folder name
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003. **Priority:** Medium
- **Given** `Atlas` stores `/rapports été 📊/a.xlsx` of 5 000 bytes
- **When** Olivia reads the folder breakdown with no folder
- **Then** the item shows the path `/rapports été 📊` exactly, Folder 5 000 bytes (1 file)

#### E2E-032: Five levels deep
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002. **Priority:** Medium
- **Given** `Atlas` stores `/l1/l2/l3/l4/l5/deep.bin` of 7 000 bytes
- **When** Olivia reads the folder breakdown with no folder, then the folder `/l1/l2/l3`
- **Then** the first answer lists `/l1` Folder 7 000 bytes (1 file); the second lists `/l1/l2/l3/l4` Folder 7 000 bytes (1 file) and shows the folder `/l1/l2/l3` as 7 000 bytes holding 1 file

#### E2E-033: A folder name that starts another folder's name
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-007. **Priority:** High
- **Given** `Atlas` stores `/doc/a.bin` of 1 000 bytes and `/docs/b.bin` of 2 000 bytes
- **When** Olivia reads the folder breakdown with no folder, then the folder `/doc`
- **Then** the first answer lists `/docs` Folder 2 000 bytes (1 file) then `/doc` Folder 1 000 bytes (1 file); the second shows folder `/doc` as 1 000 bytes holding 1 file with the single item `/doc/a.bin` File 1 000 bytes

#### E2E-034: Files sharing content count once per path
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** `Atlas` stores `/a/copy1.bin` and `/a/copy2.bin` with identical content of 100 000 bytes
- **When** Olivia reads the folder breakdown with no folder
- **Then** bytes used is 200 000 and `/a` shows as Folder 200 000 bytes (2 files)

#### E2E-035: An unlimited project
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the shared setup with the project limit removed
- **When** Ada reads the folder breakdown with no folder
- **Then** the limit is shown as no value (neither 0 nor text) and the items are those of E2E-001

#### E2E-036: A project over its limit is readable
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** `Atlas` stores `/big/a.bin` 700 000, `/big/b.bin` 500 000 and `/big/c.bin` 300 000 bytes, and Ada has lowered its limit to 1 MB
- **When** Olivia reads the folder breakdown with no folder
- **Then** limit 1 MB, bytes used 1 500 000, and one item `/big` Folder 1 500 000 bytes (3 files)
- **And** nothing was removed from the project

#### E2E-037: A write during the read is seen entirely or not at all
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the shared setup
- **When** Mia writes `/docs/new.txt` of 30 000 bytes while Olivia reads the folder breakdown with no folder
- **Then** the answer is either the answer of E2E-001 or the same answer with bytes used 1 255 500 and `/docs` Folder 154 000 bytes (3 files); in both cases the item sizes add up to the folder total
- **And** a second read afterwards shows the second form

#### E2E-038: Odd folder names are refused as typed
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads, one after the other, the folder named by the empty text, `/video/../docs`, `//video`, `/video//`, `//`, `/Video` and `/`
- **Then** the empty name is refused with `path must start with /`; `/video/../docs` with `folder '/video/../docs' not found`; `//video` with `folder '//video' not found`; `/video//` with `folder '/video//' not found`; `//` with `folder '//' not found`; `/Video` with `folder '/Video' not found`; and `/` shows the root folder exactly as with no folder (the answer of E2E-001)

#### E2E-039: The default read of a very large project is fast
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002. **Priority:** High
- **Given** `Atlas` stores 100 000 files of 100 bytes spread over 1 000 folders `/d0000` to `/d0999`, 100 files each, and its limit is 20 MB
- **When** Olivia reads the folder breakdown with no folder and no cap
- **Then** the answer arrives within 5 seconds: bytes used 10 000 000, folder `/` of 10 000 000 bytes holding 100 000 files, 100 items shown and the list cut, the first item `/d0000` Folder 10 000 bytes (100 files)

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001 to 007, 009, 010, 011 | E2E-001, 003, 004 | E2E-012 to 017, 038 | E2E-026 to 034, 039 |
| SC-002 | FR-NEW-001, 008, 011 | E2E-002 | E2E-010 | E2E-035, 036 |
| SC-003 | FR-NEW-008 | none (a refusal scenario) | E2E-007, 008, 011, 018 | E2E-009 |
| SC-004 | FR-NEW-003, 007, 010 | E2E-005 | none (a read cannot fail here beyond EXC-004a) | E2E-023 to 025, 037 |
| SC-005 | FR-NEW-009, 012 | E2E-006 | E2E-019, 020 | none (E2E-006 also reads a folder with a trailing slash) |

Every requirement has at least 3 tests:

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, 002, 003, 026, 039 |
| FR-NEW-002 | E2E-001, 003, 004, 026, 032, 033, 039 |
| FR-NEW-003 | E2E-001, 003, 005, 023, 031 |
| FR-NEW-004 | E2E-001, 004, 029, 030 |
| FR-NEW-005 | E2E-012, 013, 014, 028 |
| FR-NEW-006 | E2E-001, 004, 026, 028 |
| FR-NEW-007 | E2E-005, 023, 024, 025, 033, 034 |
| FR-NEW-008 | E2E-002, 007, 008, 009, 010, 011, 018, 019 |
| FR-NEW-009 | E2E-015, 016, 017, 020, 027, 038 |
| FR-NEW-010 | E2E-021, 022, 037 |
| FR-NEW-011 | E2E-001, 035, 036 |
| FR-NEW-012 | E2E-006, 019, 020 |

## 11. Open questions

None.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| Folder breakdown | one folder's own size and file count and its direct items, largest first | Administration |
| Item | a direct child of the folder read: a `Folder` or a `File` | Administration |
| Rolled up | counted with everything below, at any depth | Storage |
| Trash folder | the folder where deleted files wait for the purge (`.mcp_trash` by default) | Storage |
| Cap | the largest number of items listed | Administration |
| Cut list | a list showing fewer items than the folder holds | Administration |
| Usage (bytes used) | the sum of the declared sizes of all files of a project, trashed included | Storage |
| Purge | the removal of trashed files after their retention | Storage |
| Platform admin | a person allowed to manage projects | Administration |
| Owner | the person who owns a project | Administration |

## 13. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 5 | NOT-IMPLEMENTABLE |
| 2 | 2 | 0 | NOT-IMPLEMENTABLE |
| 3 | 2 | 1 | NOT-IMPLEMENTABLE |
| 4 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** trash folder across delete, restore and purge (FR-NEW-003, E2E-024, E2E-025); folder path forms (FR-NEW-009, E2E-038); fixtures starting from "stores ..." hold only the named files; traceability corrected; performance test E2E-039 added; round 3 precision fix for `//` and `///`; citation range corrected. One round 3 finding (spell the tool, route and field names) was rejected: names belong to /sdlc-design (design input in 14.3); the round 4 auditor, told so, confirmed it is not a finding.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| Quota read, owner or admin gate and refusal texts | `crates/core/src/tools/admin.rs:667-700` |
| File list, cap, cut sign, refusal text | `crates/core/src/tools/admin.rs:700-755` |
| Ties by path in character code order | `crates/core/src/storage/meta.rs:660` |
| Usage is a counter read under the admission lock | `crates/core/src/quota.rs:164-195` |
| File list on the web interface | `crates/core/src/api/dataplane.rs:424-440` |
| Trash folder name default | `crates/core/src/config.rs:362-364` |
| Deleted files go under the trash folder | `crates/core/src/safety.rs:221-226` |
| No folder size read exists | `rg -n 'folder' crates/core/src/tools/admin.rs` shows no folder read |
| 36 tests for the file list | `rg -c '^async fn spec_0047' crates/core/src/tools/disk_usage_e2e.rs` gives 36 |
| 97 tests for the usage read | `rg -c '#\[tokio::test' crates/core/src/tools/quota_e2e.rs` gives 97 |

Test command: `make check`.

### 14.3 Design inputs

- Whether the folder read is a new tool or an extra optional parameter of the file list: either way the frozen tool contract changes (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`) and E2E-022 pins the file list.
- A folder's size and count need a rollup of the files by path prefix on a whole component (`/doc` versus `/docs`). Signal: E2E-033.
- Ties by name must follow character code order on every database engine; the file list already resolves ties in code (`crates/core/src/storage/meta.rs:660`). Signal: E2E-029 on PostgreSQL.
- Empty folders are folder rows with no file below them and must be listed; the trash folder can remain after a purge. Signals: E2E-026, 030, 025.
- Direct children of a folder come from the stored parent of each row, files and folders both. Signal: E2E-003, 032.
- The names a consumer sees (the tool or parameter carrying the folder, the answer fields) are chosen by /sdlc-design; they should mirror the file list of SPEC-0047 (`project_id`, `max_mb`, `used_bytes`, `truncated`, a list field, `limit`) so both reads stay uniform (`crates/core/src/tools/admin.rs:735-749`).
- The usage counter and the rollup are read at one moment (admission lock and one transaction, as for the file list). Signals: E2E-001 (root equals bytes used), E2E-037.
- Performance at 100 000 files with the default cap: one pass aggregation. Signal: E2E-039.
- The emptied trash folder must remain a listed folder after the purge removed its last file (FR-NEW-003); nothing in the purge removes it today (`crates/core/src/purge.rs`, no directory removal outside test wrappers). Signal: E2E-025.
- A first delete must leave the trash folder as a stored folder that stays after a restore and a purge. Signals: E2E-024, E2E-025.
- Empty folders must exist as stored folder rows after creation; if a creation path writes none, the design adds them. Signal: E2E-026.
- The root total must come from the same moment as the usage counter (admission lock, one transaction), because the counter and the rollup are separate sources. Signals: E2E-001, E2E-037.
