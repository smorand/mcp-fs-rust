# Refuse a destination outside the project for every archive format: functional specification

> Id: SPEC-0086
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction, every format), 8 requirements, public behavior change (a destination that climbs above the project root, accepted and redirected to the root today by zip, 7z and the tar family, is refused; an absolute destination that climbs is refused for every format; the destination refusal moves before the archive is read), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-09
> Project: existing
> From backlog: BL-0127
> Depends on: SPEC-0084, SPEC-0085 (both specified, not built; this increment moves the destination refusal inside the orders they define)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a
>
> Note: written non interactively. The user authorized every decision and gave the instruction "spec all of them, design later", which is the explicit approval of the test plan in Section 9. The user had already accepted the strict refusal for every format. Every default applied is listed in Section 14.4.

## 1. Summary

A member who extracts an archive and types a destination that climbs above the project root, such as `../../x` or `/../x`, gets a refusal naming that destination, whatever the archive format, before the archive is read. Today zip, 7z and tar archives quietly redirect such a destination to the project root, while bare `.gz`, `.bz2`, `.xz` files and RAR archives refuse the relative form, so one option behaves two ways and a typing mistake writes files at the root of the project. A destination that climbs and comes back inside, such as `a/../b`, keeps working.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member extracts an archive from the assistant tool or from the HTTP API, and can name a destination folder; without one the archive's own path minus its suffix is used (for a bare compressed file, the archive's folder). Both doors run the same extraction and report the same refusals. [14.1 row 1]
- zip, 7z, plain `.tar` and compressed tar archives (`.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz`, `.txz`): a destination is resolved inside the project in every case. A relative destination is read from the project root; a `..` that goes above the root is dropped. `../../x` and `/../x` both become `/x`, and the entries are written there. [row 2]
- RAR archives and bare `.gz`, `.bz2`, `.xz` files: a relative destination that climbs above the root (`../../x`, `a/../../x`) is refused with `ERR_PATH_OUT_OF_BOUNDS` and the message `path escapes the volume root: <destination as typed>`; an absolute one that climbs (`/../x`, `/a/../../b`) is redirected to the root like the other formats. [row 3]
- A destination whose first folder name begins with two dots, such as `..out` or `/..out/x`, is refused by every format with `ERR_PATH_OUT_OF_BOUNDS` and `path escapes the volume root: <destination as typed>`, although it names a folder inside the project. [row 9]
- A destination holding a NUL character is refused with `ERR_PATH_OUT_OF_BOUNDS` and `path contains a NUL byte`, for every format. [row 4]
- When the destination is checked. zip, 7z, tar family, RAR: after the archive is read, decoded and its entries checked, just before the collision check. A damaged zip with destination `../../x` therefore answers `archive is corrupt or not a valid zip file: ...`. Bare compressed files: before the file is decoded, after membership, path, folder, password and output name checks. [row 5]
- Other tools taking a path redirect a climb to the root in the same way; only the zip export refuses it. [row 6]

### 2.2 Specs governing this area

- SPEC-0018 FR-NEW-016 resolves the destination "normalized" like any other path option, which is the redirection described above. FR-MOD-001 replaces it for a destination that climbs.
- SPEC-0080 FR-NEW-015 item (9) refuses a RAR destination outside the project after decoding; FR-MOD-002 moves it before the archive is read, and FR-MOD-001 extends it to absolute climbs.
- SPEC-0081, SPEC-0082 and SPEC-0083 (bare `.gz`, `.xz`, `.bz2`) refuse a relative destination outside the project at item (6) of their order; FR-MOD-001 extends the refusal to absolute climbs; their order does not change.
- SPEC-0084 FR-NEW-005 item (7) and SPEC-0085 FR-NEW-009 item (9) place "the destination refusal of today" after decoding; FR-MOD-002 moves the refusal of a destination outside the project before the archive is read, ahead of their decoding bound, declared size limits and entry checks. SPEC-0085 E2E-028 ("the limit wins over a refused destination") is reversed (Section 9.4).
- None of these specs is modified by this document. [row 7]

### 2.3 What is verified today

- RAR and the three bare formats have a test refusing `../../x`; the bare formats also show that the refusal wins over a corrupt file. [row 8]
- No test extracts zip, 7z or a tar archive with a destination that climbs, absolute or relative, and no test exercises an absolute climb for any format. The general path tests pin the redirection for every path, which this increment does not change. [row 8]

## 3. Scope

### 3.1 In scope

- The destination option of archive extraction, every format (zip, 7z, RAR, plain and compressed tar, bare `.gz`, `.bz2`, `.xz`), any letter case, from both doors.
- A destination that climbs above the root, relative or absolute, refused with one code and one message.
- The destination refusals (outside the project, NUL character) reported before the archive is read, for zip, 7z, RAR and the tar family.

### 3.2 Out of scope, and why

- Every other path option of every other tool (write, copy, move, the other operations of the HTTP API, the archive path itself): they redirect a climb to the root too, a separate behavior with its own tests; BL-0135.
- Entry names inside the archive that climb above the destination: already refused per format, unchanged.
- A backslash in a destination: a plain character of a folder name today, unchanged; the project tree only separates on `/`.
- An empty destination or `.`: the project root, unchanged.
- The order of refusals for bare compressed files: already destination before decoding.

### 3.3 Constraints imposed by the context

- The user imposed the strict refusal, code `ERR_PATH_OUT_OF_BOUNDS` and message `path escapes the volume root: <destination>`, for every format (the decision BL-0127 asked for).

## 4. Actors

- **Member:** a person with access to a project; extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server and reads the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored archive into project files | destination, climb, entry, refusal, order of refusals, audit line |
| Project storage | the project tree, its storage limit and the session write allowance | project root, folder, remaining storage, session write allowance |

## 5. Usage scenarios

### SC-001: Extract into a destination inside the project
**Actor:** Member.
**Preconditions:** project `atlas`; `/uploads/pack.zip` (or any other supported format) holds `a.txt` with the 5 bytes `hello`.
**Flow:**
1. The member extracts it with destination `out`, `/out`, `a/../b` or `/a/../b` → the system writes `/out/a.txt` (first two) or `/b/a.txt` (last two).
2. The system answers the destination it used (`/out` or `/b`), 1 file, 5 bytes, and records one audit line naming that destination.
**Postconditions:** the files are in the resolved folder inside the project.
**Exceptions:**
- EXC-001a: the destination climbs above the root on the way (`a/../../b`) → SC-002.

### SC-002: A destination outside the project is refused
**Actor:** Member.
**Preconditions:** as SC-001.
**Flow:**
1. The member extracts it with destination `../../x`, `..`, `a/../../x`, `/../x` or `/a/../../b` → the system refuses with `ERR_PATH_OUT_OF_BOUNDS` and `path escapes the volume root: <the destination exactly as typed>`.
**Postconditions:** nothing written, no folder created, storage and session allowance unchanged, no audit line.
**Exceptions:**
- EXC-002a: the destination holds a NUL character → `ERR_PATH_OUT_OF_BOUNDS`, `path contains a NUL byte`, same postconditions.
**Interactions:** the member corrects the destination and extracts again (SC-001).

### SC-003: Order when several refusals apply
**Actor:** Member.
**Flow:**
1. Not a member, missing path, path is a folder, unsupported extension, password given for a tar archive, multi volume RAR name → reported before the destination.
2. A destination outside the project → reported before anything read from the archive: damaged data, link or escaping entry, password needed, size limits, decoding bound, collision.
**Postconditions:** as SC-002.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-U]: What climbing above the root means
> THE archive extraction SHALL treat a destination as outside the project when, reading its segments separated by `/` from left to right and starting at the project root (with or without a leading `/`), a `..` segment is met while at the project root.

- **Business rules:** an empty segment and a `.` segment stay in place; any other segment, including one that merely starts or ends with dots (`..out`, `out..`), enters a folder (a first segment such as `..out` is refused today, Section 2.1, and accepted from now on); `..` leaves it. `a/..` ends at the root and is inside; `a/../../x` and `/a/../../b` are outside; `..` alone is outside.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: A destination inside the project is resolved as today
> WHEN a member extracts an archive with a destination that is not outside the project THE archive extraction SHALL write the entries under the folder that destination resolves to from the project root, and SHALL answer and audit that resolved folder as the destination.

- **Business rules:** `out` and `/out` resolve to `/out`; `a/../b` and `/a/../b` to `/b`; `a/..` to `/`; `./out/./x` and `out//x` to `/out/x`.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-003 [EARS-UB]: A refused destination leaves no trace
> THE archive extraction SHALL NOT read the archive's content, write a file, create a folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused under FR-MOD-001 or FR-NEW-004.

- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: A NUL character in the destination
> IF the destination holds a NUL character THEN THE archive extraction SHALL refuse with `ERR_PATH_OUT_OF_BOUNDS` and the message `path contains a NUL byte`, at the place FR-MOD-002 gives to the destination refusals.

- **Business rules:** code and message are today's; only the moment changes for zip, 7z, RAR and the tar family.
- **Scenario:** SC-002, SC-003
- **Priority:** Should

#### FR-NEW-005 [EARS-U]: Both doors answer alike
> THE archive extraction SHALL report, through the HTTP API, the same refusal code and the same message as through the assistant tool for every destination of this specification.

- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: The rest is unchanged
> THE archive extraction SHALL extract or refuse every call whose destination is absent or inside the project exactly as before this increment, except a destination whose first folder name begins with two dots (FR-NEW-001), and SHALL leave every other path option of every tool unchanged.

- **Business rules:** the default destination, the refusal of entry names climbing above the destination, collisions, limits and the audit line are unchanged.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: Refuse a destination outside the project (modifies SPEC-0018/FR-NEW-016; extends SPEC-0080/FR-NEW-015 item 9 and the destination refusal of SPEC-0081, SPEC-0082 and SPEC-0083 to absolute climbs)
> IF the destination is outside the project THEN THE archive extraction SHALL refuse with `ERR_PATH_OUT_OF_BOUNDS` and the message `path escapes the volume root: <destination>`, `<destination>` being the destination exactly as the member typed it, for every archive format.

- **Before:** zip, 7z and the tar family redirect a climb to the root (`../../x` writes `/x`); RAR and bare compressed files refuse a relative climb and redirect an absolute one; every format refuses `..out` (Section 2.1).
- **After:** the statement above, the same for every format and both forms.
- **Reason:** one option, one behavior; a mistyped destination no longer writes at the project root (BL-0127, user decision).
- **Scenario:** SC-002
- **Priority:** Must

#### FR-MOD-002 [EARS-O]: The destination refusal comes before the archive is read (modifies SPEC-0018 order, SPEC-0080/FR-NEW-015 item 9, SPEC-0084/FR-NEW-005 item 7, SPEC-0085/FR-NEW-009 item 9)
> IF several refusals apply to the extraction of a zip, 7z, RAR, plain tar or compressed tar archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) the extension is not supported, including `.rar` on a server without RAR support (`ERR_NOT_SUPPORTED`, `unsupported archive extension: '<extension as typed>'`); (5) a password is given for a plain or compressed tar archive (`ERR_INVALID_ARGUMENT`, `password is not applicable to this archive format`); (6) a multi volume RAR name (`ERR_NOT_SUPPORTED`, `multi volume RAR archives are not supported: extract a single file archive`); (7) the destination holds a NUL character (FR-NEW-004); (8) the destination is outside the project (FR-MOD-001); (9) every later refusal in the order its own specification gives, the destination refusal of that order removed.

- **Before:** the destination refusals come after the archive is read, decoded and its entries checked (Section 2.1), and after the declared limits of SPEC-0085 and the decoding bound of SPEC-0084.
- **After:** the statement above. Bare `.gz`, `.bz2`, `.xz` files keep their order (destination at item 6, after the output name).
- **Reason:** nothing the archive contains matters once the place to write is refused; the member learns the actual mistake first, and the server reads nothing for a call it will refuse.
- **Scenario:** SC-003
- **Priority:** Must

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.2 Security and access

- The refusal of a destination outside the project never names a folder that exists in the project or outside it: the message repeats the destination as typed (FR-MOD-001).

### 7.5 Reliability

- A refused call leaves the project exactly as it was (FR-NEW-003); repeating it gives the same refusal and still changes nothing (E2E-033).

### 7.6 Operability

- A refused call records no audit line, as every other extraction refusal (FR-NEW-003).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | no destination | default destination, unchanged | E2E-005 |
| Boundary values | climbs back exactly to the root (`a/..`), to the root then down (`a/b/../../c`), one step above (`..`) | root; `/c`; refused | E2E-006, E2E-008, E2E-009 |
| Boundary values | `..` hidden after normalization (`a/../../x`) | refused | E2E-014 |
| Concurrent actions | not applicable: the refusal reads no project state | none | none |
| Slow or lost connectivity | not applicable: no dependency is called | none | none |
| Access states | non member with a climbing destination | membership refusal first | E2E-024 |
| Data variations | names starting or ending with dots (`..out`, `out..`), repeated slashes and `.` segments | folders inside the project | E2E-007, E2E-010 |
| Data variations | NUL character | `path contains a NUL byte`, before reading | E2E-030 |
| Navigation | not applicable: no screen | none | none |
| Partial operations | refusal leaves nothing behind; a corrected call then succeeds | nothing written, then success | E2E-033 |
| Dependency failures | damaged archive with a climbing destination | destination refusal | E2E-019 |
| Ordering and timing | each earlier refusal versus the destination; the destination versus each later one | per FR-MOD-002 | E2E-019 to E2E-029, E2E-MOD-001 |
| Exhaustion | storage limit or session allowance exceeded with a climbing destination | destination refusal | E2E-020, E2E-MOD-001 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Unless a test says otherwise: project `atlas`, member `mia@example.com`; `/uploads/pack.<ext>` is an archive of the named format holding one file `a.txt` with the 5 bytes `hello`; nothing exists at `/x`, `/out`, `/b`, `/c` or `/uploads/pack`; the session allowance is large enough not to matter and the project has no storage limit. "Nothing is written" means `/x`, `/uploads/pack` and every folder named by the test are absent afterwards, the project's stored bytes and the member's used session allowance are unchanged, and the audit trail has no extraction line for the call. "Server with RAR support" names a delivery built with it.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-002, FR-NEW-006 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-002, FR-NEW-006 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-004 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-005 | New | happy | SC-001 | FR-NEW-006 | Medium |
| E2E-006 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-007 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-008 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-009 | New | edge | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-010 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-011 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-012 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-013 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-014 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-015 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001, FR-NEW-003 | Critical |
| E2E-016 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-017 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-018 | New | failure | SC-002 | FR-MOD-001 | Medium |
| E2E-019 | New | failure | SC-003 | FR-MOD-002, FR-NEW-003 | Critical |
| E2E-020 | New | failure | SC-003 | FR-MOD-002, FR-NEW-003 | High |
| E2E-021 | New | failure | SC-003 | FR-MOD-002 | High |
| E2E-022 | New | failure | SC-003 | FR-MOD-002 | High |
| E2E-023 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-024 | New | failure | SC-003 | FR-MOD-002 | High |
| E2E-025 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-026 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-027 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-028 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-029 | New | failure | SC-003 | FR-MOD-002 | Medium |
| E2E-030 | New | failure | SC-002 | FR-NEW-004, FR-MOD-002, FR-NEW-003 | Medium |
| E2E-031 | New | side effect | SC-002 | FR-NEW-005, FR-MOD-001, FR-NEW-003 | Critical |
| E2E-032 | New | side effect | SC-001 | FR-NEW-005, FR-NEW-002 | High |
| E2E-033 | New | side effect | SC-002 | FR-NEW-003, FR-NEW-006 | High |
| E2E-034 | New | side effect | SC-002 | FR-NEW-005, FR-MOD-001, FR-NEW-004 | High |
| E2E-035 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Medium |
| E2E-036 | New | failure | SC-002 | FR-NEW-004, FR-NEW-003 | Medium |
| E2E-MOD-001 | Modified | failure | SC-003 | FR-MOD-002, FR-NEW-003 | High |

Coverage (37 tests: 36 new, 1 modified, 0 removed): happy 5, failure 23, side effect 4, edge 5, state transition 0 (no state machine), security 0 (no security lot). Happy to failure ratio 1:4.6.

### 9.2 New tests

#### E2E-001: Relative destination inside the project, zip
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `out`
- **Then** `/out/a.txt` reads `hello`; the answer gives destination `/out`, 1 file written, 5 bytes written
- **And** the audit trail has one extraction line for `/uploads/pack.zip` naming destination `/out`

#### E2E-002: Absolute destination inside the project, 7z
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `/out`
- **Then** `/out/a.txt` reads `hello`; the answer gives destination `/out`, 1 file written, 5 bytes written

#### E2E-003: A climb that stays inside, compressed tar
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** `/uploads/pack.tar.gz`
- **When** the member extracts it with destination `a/../b`
- **Then** `/b/a.txt` reads `hello`; the answer gives destination `/b`; `/a` does not exist

#### E2E-004: An absolute climb that stays inside, every format
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** on a server with RAR support, `/uploads/pack.zip`, `/uploads/pack.tar`, `/uploads/pack.rar`, and `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts each with destination `/a/../b`, removing `/b` between calls
- **Then** each call succeeds with destination `/b` and `/b/a.txt` reads `hello`

#### E2E-005: No destination keeps the default
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with no destination
- **Then** `/uploads/pack/a.txt` reads `hello`; the answer gives destination `/uploads/pack`

#### E2E-006: Back to the root exactly
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `a/..`
- **Then** `/a.txt` reads `hello`; the answer gives destination `/`

#### E2E-007: Dot segments and repeated slashes
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/pack.tar`
- **When** the member extracts it with destination `./out/./x`, then, after removing `/out`, with `out//x`
- **Then** both calls answer destination `/out/x` and `/out/x/a.txt` reads `hello`

#### E2E-008: To the root, then down
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `a/b/../../c`
- **Then** `/c/a.txt` reads `hello`; the answer gives destination `/c`

#### E2E-009: One step above the root
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `..`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ..`; nothing is written; `/a.txt` does not exist

#### E2E-010: Names made of dots and letters are folders
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz`
- **When** the member extracts it with destination `..out`, then with destination `out..`
- **Then** the answers give destinations `/..out` and `/out..`; `/..out/a.txt` and `/out../a.txt` read `hello`

#### E2E-011: zip, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`
- **And** nothing is written; `/x/a.txt` does not exist

#### E2E-012: 7z, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written

#### E2E-013: plain tar, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.tar`
- **When** the member extracts it with destination `../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../x`; nothing is written

#### E2E-014: Climb revealed only after resolving, compressed tar family
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/pack.tgz`, `/uploads/pack.tar.bz2` and `/uploads/pack.TAR.XZ`
- **When** the member extracts each with destination `a/../../x`
- **Then** each refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: a/../../x`; nothing is written; `/a` does not exist

#### E2E-015: zip, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `/../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written

#### E2E-016: Bare gzip file, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts it with destination `/../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written; `/x/a.txt` does not exist

#### E2E-017: RAR, absolute climb through a folder
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** a server with RAR support and `/uploads/pack.rar`
- **When** the member extracts it with destination `/a/../../b`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /a/../../b`; nothing is written; `/b` does not exist

#### E2E-018: Bare bzip2 and xz files, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** Medium
- **Given** `/uploads/a.txt.bz2` and `/uploads/a.txt.xz`, each holding `hello` compressed
- **When** the member extracts each with destination `/../x`
- **Then** each refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written

#### E2E-019: The destination wins over a damaged archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/broken.zip` holding the 9 bytes `not a zip`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not a message starting `archive is corrupt or not a valid zip file:`; nothing is written

#### E2E-020: The destination wins over the limits
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** High
- **Given** a session allowance of 1 000 bytes and `/uploads/bomb.tar.gz` holding `zeros.bin`, 10 000 000 zero bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_WRITE_QUOTA_EXCEEDED`; nothing is written; the member's used session allowance is still 0

#### E2E-021: The destination wins over a link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/link.zip` holding a symbolic link entry `l` pointing to `/etc/passwd`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_NOT_SUPPORTED`

#### E2E-022: The destination wins over an escaping entry name
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/evil.tar` holding an entry named `../../etc/passwd`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not a message starting `archive entry path escapes the destination:`

#### E2E-023: The destination wins over a missing password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/secret.7z` encrypted with the password `s3cret`
- **When** the member extracts it with no password and destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_PASSWORD_REQUIRED`

#### E2E-024: Membership wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `eve@example.com`, not a member of `atlas`, and `/uploads/pack.zip`
- **When** eve extracts it with destination `../../x`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'eve@example.com' is not a member of 'atlas'`

#### E2E-025: A missing path wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** nothing at `/uploads/none.zip`
- **When** the member extracts `/uploads/none.zip` with destination `../../x`
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/none.zip' not found`

#### E2E-026: A folder path wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** a folder `/uploads/dir.zip`
- **When** the member extracts `/uploads/dir.zip` with destination `../../x`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.zip' is a directory, not an archive file`

#### E2E-027: An unsupported extension wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/pack.lz` holding 5 bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.lz'`

#### E2E-028: A password on a tar archive wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/pack.tar.gz`
- **When** the member extracts it with password `x` and destination `../../x`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`

#### E2E-029: A multi volume RAR name wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** a server with RAR support and `/uploads/pack.part1.rar`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `multi volume RAR archives are not supported: extract a single file archive`

#### E2E-030: A NUL character is refused before the archive is read
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-MOD-002, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/broken.zip` of E2E-019
- **When** the member extracts it with the destination made of `out`, a NUL character, then `x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path contains a NUL byte`, not the corrupt refusal; nothing is written; `/out` does not exist

#### E2E-031: HTTP API, same refusal
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it through the HTTP API with destination `../../x`
- **Then** the answer is a refusal with code `ERR_PATH_OUT_OF_BOUNDS` and message `path escapes the volume root: ../../x`, the same as through the assistant tool
- **And** nothing is written

#### E2E-032: HTTP API, a climb that stays inside
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it through the HTTP API with destination `/a/../b`
- **Then** the answer gives destination `/b`, 1 file written, 5 bytes written; `/b/a.txt` reads `hello`

#### E2E-033: Refused twice, then corrected
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** a storage limit of 1 MB and `/uploads/pack.tar`
- **When** the member extracts it with destination `../../x`, again with `../../x`, then with `x`
- **Then** the first two calls are refused with `path escapes the volume root: ../../x` and leave the stored bytes, the used session allowance and the audit trail unchanged
- **And** the third succeeds with destination `/x`, `/x/a.txt` reads `hello`, the stored bytes grow by 5 and the audit trail gains exactly one extraction line

#### E2E-034: HTTP API, absolute climb and NUL on a bare file
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001, FR-NEW-004; **Priority:** High
- **Given** `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts it through the HTTP API with destination `/../x`, then with the destination made of `out`, a NUL character, then `x`
- **Then** the refusals are `ERR_PATH_OUT_OF_BOUNDS` with `path escapes the volume root: /../x`, then `ERR_PATH_OUT_OF_BOUNDS` with `path contains a NUL byte`; nothing is written

#### E2E-035: overwrite does not lift the refusal
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/pack.zip` and `/x/a.txt` holding `old`
- **When** the member extracts it with destination `../../x` and `overwrite` true
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; `/x/a.txt` still reads `old`

#### E2E-036: A NUL character in an otherwise valid call
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with the destination made of `out`, a NUL character, then `x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path contains a NUL byte`; nothing is written; `/out` does not exist

### 9.4 Modified tests

#### E2E-MOD-001: The destination wins over the declared limit, RAR (was SPEC-0085 E2E-028)
- **Validated before:** with 1 000 bytes of remaining storage and the RAR bomb, destination `../../x` answered `ERR_PROJECT_QUOTA_EXCEEDED`.
- **Validates now:** the destination refusal comes first (FR-MOD-002 item 8).
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** High
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` holding `zeros.bin`, 10 000 000 zero bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_PROJECT_QUOTA_EXCEEDED`; neither `/x` nor `/uploads/bomb` exists

### 9.5 Removed tests

None. SPEC-0080 E2E-061 and the `../../x` tests of SPEC-0081 (E2E-035), SPEC-0082 (E2E-022) and SPEC-0083 (E2E-032) keep their assertions.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-005, FR-NEW-006 | E2E-001 to E2E-005 | E2E-009 (SC-002 branch of EXC-001a) | E2E-006, E2E-007, E2E-008, E2E-010 | E2E-032 |
| SC-002 | FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-MOD-001 | E2E-033 (third call) | E2E-011 to E2E-018, E2E-030, E2E-035, E2E-036 | E2E-009 | E2E-031, E2E-033, E2E-034 |
| SC-003 | FR-MOD-002, FR-NEW-003 | E2E-001 (no refusal applies) | E2E-019 to E2E-029, E2E-MOD-001 | E2E-024 (access state) | E2E-020 (allowance unchanged) |

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-003, E2E-004, E2E-006, E2E-008, E2E-009, E2E-010, E2E-014, E2E-015, E2E-016, E2E-017 | 10 |
| FR-NEW-002 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-006, E2E-007, E2E-008, E2E-010, E2E-032 | 9 |
| FR-NEW-003 | E2E-011, E2E-012, E2E-013, E2E-015, E2E-019, E2E-020, E2E-030, E2E-031, E2E-033, E2E-035, E2E-036, E2E-MOD-001 | 12 |
| FR-NEW-004 | E2E-030, E2E-034, E2E-036 | 3 |
| FR-NEW-005 | E2E-031, E2E-032, E2E-034 | 3 |
| FR-NEW-006 | E2E-001, E2E-002, E2E-005, E2E-033 | 4 |
| FR-MOD-001 | E2E-009, E2E-011 to E2E-018, E2E-031, E2E-034, E2E-035 | 12 |
| FR-MOD-002 | E2E-019 to E2E-029, E2E-030, E2E-MOD-001 | 13 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Destination | the folder that receives what an archive holds; typed by the member or, when absent, derived from the archive's path | archive extraction |
| Project root | the top folder of the project, `/` | project storage |
| Climb | a `..` segment, which leaves the current folder for its parent | archive extraction |
| Outside the project | a destination whose segments, read from the project root, meet a `..` while at the root (FR-NEW-001) | archive extraction |
| Resolved folder | the folder inside the project a destination designates once its `.` and `..` segments and repeated slashes are applied | archive extraction |
| Entry | one file or folder inside an archive | archive extraction |
| Escaping entry name | an entry name that is absolute or climbs above the destination; refused | archive extraction |
| Link entry | an entry that is a symbolic link; refused | archive extraction |
| Bare compressed file | a `.gz`, `.bz2` or `.xz` file holding one compressed file, not an archive of several | archive extraction |
| Compressed tar archive | a tar archive compressed as `.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz` or `.txz` | archive extraction |
| Multi volume RAR | a RAR archive split across numbered files; refused | archive extraction |
| Order of refusals | which refusal a call reports when several apply | archive extraction |
| Refusal | an error answer; the project is left unchanged | archive extraction |
| Audit line | the one line an operator reads to see who extracted what and where | archive extraction |
| Storage limit | the optional cap on the bytes a project stores, in whole megabytes | project storage |
| Remaining storage | the storage limit minus the bytes stored | project storage |
| Session write allowance | the bytes a member can still write in the current session | project storage |
| Overwrite | the choice to replace colliding files instead of refusing | project storage |
| Collision | an entry's target path already holds a file | project storage |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server and reads its audit trail | archive extraction |
| HTTP API | the web endpoint a program calls to extract an archive; the second door | archive extraction |
| Assistant tool | the tool an AI assistant calls to extract an archive | archive extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 4 | IMPLEMENTABLE |

**Amendments applied after round 1 (all A, amended in place):** A-1 `..out` is refused today by every format (`safety.rs:103-104`): added to 2.1, 14.1 row 9, FR-NEW-001, FR-NEW-006, FR-MOD-001 and a 14.3 line with its signal E2E-010; A-2 compressed tar suffixes `.tb2` and `.txz` added (`archive.rs:45-62`); A-3 14.1 row 2 corrected (the prefix check fires on `..out`, never on a climb); A-4 BL-0135 Evidence corrected the same way. Consistency fixes: FR-NEW-004 given a third test (E2E-036), coverage recomputed; two modals in Section 2 prose rewritten; "HTTP routes" rewritten in product terms.
**How the gates ran:** this harness exposes no subagent tool, so the test design (8.1), the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks for ids, categories, requirement coverage, traceability counts, modals and implementation terms, and every cited message and suffix reopened in the code (`archive.rs:69-91`, `:182-184`, `:459`, `:509-512`, `:704-712`).
**No implementation detail outside Section 14:** PASS. Option names (`destination`, `overwrite`, password), error codes, verbatim messages, file name suffixes and example paths typed by the member are WHAT.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | two doors, one function, membership first; default destinations | `crates/core/src/mcp/server.rs:2370-2380` (`authorize_only` then `tools::archive::extract_archive`); `crates/core/src/api/dataplane.rs:176`, `:878-902` (same function behind `guarded_json`); default: `crates/core/src/tools/archive.rs:226`, `crates/core/src/tools/archive_single.rs:243-247` |
| 2 | zip, 7z, tar family clamp a climb | `crates/core/src/tools/archive.rs:223-227` (`Some(d) => state.safety.normalize_path(d)?`); `crates/core/src/safety.rs:99-111` (relative prefixed with `/` at `:103`, the `"/.."` check at `:104` never fires on a climb after `normpath`); `crates/core/src/util/posix.rs:48-55` (a root `..` dropped for an absolute path) |
| 3 | RAR and bare files refuse a relative climb, clamp an absolute one | `crates/core/src/tools/archive.rs:224` (RAR through `normalize_destination`); `crates/core/src/tools/archive_single.rs:123-130` (`climbs` only when `!dest.starts_with('/')`, then `normalize_path`), `:244` |
| 4 | NUL refused for every format | `crates/core/src/safety.rs:100-102` |
| 5 | when the destination is checked | multi format: read `archive.rs:205`, decode `:206-211`, destination `:223-227`, collision after; bare: `archive_single.rs:241` (output name), `:243-248` (destination then folder check), decode later; earlier checks `archive.rs:152-185` (path, folder, `detect_format` `:169`, password `:170-174`, multi volume `:178-185`) |
| 6 | other tools clamp; export refuses | `crates/core/src/tools/mod.rs:147`, `:153`; `crates/core/src/core/fs_ops.rs:167`, `:1858`, `:1890`; `crates/core/src/api/dataplane.rs:198`; pinning tests `crates/core/src/safety.rs:263-265`, `crates/core/src/util/posix.rs:115-118`; export `crates/core/src/tools/export.rs:73`, `:119-126` |
| 7 | governing specs | `sdlc/specs/SPEC-0018-archive-extract-in-place/spec.md:207` (FR-NEW-016); `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md:259` (FR-NEW-015 item 9); `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-spec.md:183`; `sdlc/specs/SPEC-0082-extract-xz-single-file/sdlc-spec.md:190`; `sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-spec.md:189`; `sdlc/specs/SPEC-0084-tar-family-bounded-decoding/sdlc-spec.md:157` (item 7); `sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/sdlc-spec.md:181` (item 9), `:486-490` (E2E-028) |
| 8 | tests today | `../../x` refusals: `crates/core/src/tools/archive_rar.rs:1283-1301` (`e2e_0080_061_destination_outside_the_project`), `crates/core/src/tools/archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272` (corrupt file plus `../../x`); `rg -n '\.\./' crates/core/src/tools/archive.rs` finds only entry name tests (`:1386-1390`), no destination; no `"/../` destination in any archive test file. Counts: 104 tests in `archive.rs`, 160 in `archive_single.rs`, 78 in `archive_rar.rs` (`rg -c '#\[(tokio::)?test'`) |
| 9 | `..out` refused | `crates/core/src/safety.rs:103-104`: `..out` becomes `/..out`, which `starts_with("/..")`; suffixes `crates/core/src/tools/archive.rs:45-62` |

Test command: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar tools::archive_rar`; REST tests `cargo test -p mcp-fs-core --lib api::dataplane`; full gate `cargo test --workspace`.

### 14.3 Design inputs

- One check for every format: `archive_single::normalize_destination` (`archive_single.rs:123-130`) covers relative climbs only; FR-NEW-001 also refuses absolute ones. `export.rs:119-126` (`ensure_no_escape`) already walks segments with a depth counter and refuses both forms with the same message; reusing one walk avoids a third rule. Signal: E2E-015 to E2E-018.
- `..out` (FR-NEW-001) is refused today by the `starts_with("/..")` test of `normalize_path` (`safety.rs:104`), which tests a string prefix, not a segment; the destination check must not inherit it. Signal: E2E-010.
- `SafetyManager::normalize_path` and its pinning tests (`safety.rs:263-265`, `posix.rs:115-118`) stay as they are (FR-NEW-006, BL-0135); the refusal is added in front of it for the destination only.
- The destination is resolved today after decoding (`archive.rs:223-227`); FR-MOD-002 needs it right after the multi volume check (`archive.rs:178-185`) and before `read_bytes` (`:205`). The bare file branch (`:187-199`) keeps its own order. Signal: E2E-019 to E2E-023, E2E-030, E2E-MOD-001.
- SPEC-0084 (14.3, line 710) and SPEC-0085 (14.3, first line) both need the destination before decoding for their bounds while keeping its refusal after decoding; with this increment the refusal moves too. Whichever spec is built second adapts the shared order. Signal: E2E-020, E2E-MOD-001 and SPEC-0084/SPEC-0085 order tests.
- E2E-MOD-001 rewrites SPEC-0085 E2E-028, which is not built yet: if SPEC-0085 lands first, its test changes; if this lands first, SPEC-0085's design writes it in the new form.
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json`: no description changes (the `destination` description stays `Destination directory for the extracted entries.`, `archive.rs:107`).
- Docs to update: `.agent_docs/tools.md` (archive section: destination rule, order), `README.md`, the `archive.rs` comment at `:213-218` that says "The other formats keep the clamping they have always had", and the `AGENTS.md` line on `archive_single.rs`.
- The membership refusal is emitted before `extract_archive` is called (`server.rs:2378`, `dataplane.rs:884`); E2E-024 holds by construction.

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the backlog entry, without the user, under their authorization.

- R1 scope: the archive destination option, every format, both doors; other tools out (BL-0135). Why now: BL-0127, found by SPEC-0081, the formats disagree since SPEC-0080 and SPEC-0081.
- R2 current state: Section 2 and 14.1, read from the code.
- R3 scenarios: SC-001 to SC-003; failures from today's refusal list.
- R4 requirements: derived, Section 6. R5 NFR: no trace, same doors, no audit line on refusal.
- D-1 routing: BL-0127 reads `Nature: FEAT`; Q1 yes (`archive.rs:225` writes `../../x` into `/x`); Q2 no authority violated: SPEC-0018 FR-NEW-016 asks for the destination normalized like any path, which is the clamp. Nature: FEAT.
- D-2 depth M: one module, 8 requirements, public behavior change, no persisted change.
- D-3 security `n/a`: the clamp keeps every write inside the project (`util/posix.rs:48-55`); no file outside the project is reachable today, so no traversal flaw exists; this is a consistency change.
- D-4 strict refusal for every format, code and message as in SPEC-0081 (user decision, recorded in 3.3).
- D-5 absolute climbs (`/../x`, `/a/../../b`) are refused too, for every format including RAR and bare files, because they are as much outside the project as the relative form and the zip export already refuses them (`export.rs:119-126`); one rule for both forms.
- D-6 a climb is judged while walking, not on the final path: `a/../../x` and `/a/../../b` are refused even though a clamp would land inside (FR-NEW-001).
- D-7 the message repeats the destination exactly as typed, as SPEC-0081 does.
- D-8 position in the order: after every check made on the archive's name (membership, path, folder, extension, tar password, multi volume RAR), before the archive is read; bare files keep theirs.
- D-9 the NUL refusal moves with the destination check (FR-NEW-004), keeping today's message.
- D-10 `a/..`, `.` and an empty destination resolve to the project root as today; names such as `..out` are folders.
- D-11 backslashes stay plain characters (3.2).
- D-12 other path tools: they all clamp (14.1 row 6); a real separate behavior with its own pinning tests, so BL-0135 (`Nature: FEAT`) instead of widening this increment.
- D-13 the NUL refusal keeps its code and message for the bare files, where it already came before decoding.
- D-14 tests: 36 new, 1 modified (SPEC-0085 E2E-028, not built yet), 0 removed; no existing test in the code changes. Test plan approved by the instruction "spec all of them, design later".
