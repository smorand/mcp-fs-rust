# Extract a bare xz file: functional specification

> Id: SPEC-0082
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction), about 14 anticipated requirements, public contract change (a refusal becomes a success), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-04
> Project: existing
> From backlog: BL-0112
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

A project member can decompress a single file compressed with xz and stored in their project, for example `data.csv.xz` giving `data.csv`, with the extraction tool they already use for zip, 7z, tar and (SPEC-0081) gzip. The file lands next to the compressed file unless the member names another folder. The safety rules are those of the gzip increment: no overwrite unless asked, the project's storage limit and the member's session write allowance are respected, and a decompression bomb is stopped as soon as its output exceeds the lower of the two. Two points are specific to xz: several xz streams stored back to back are all decoded, and zero padding between or after streams is accepted when its length is a multiple of 4 bytes. Compressed tar archives (`.tar.xz`, `.txz`) are unchanged.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member can extract `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz` archives in place, from the assistant tool and from the HTTP API. The format is read from the file name alone, ignoring letter case. [14.1 row 1]
- A bare `.xz` file (not a tar) is refused with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '.xz'`. [row 2]
- The destination of an extraction is a folder; when none is given it is the archive path without its suffix, a new folder. [row 3]
- An archive is read whole, then checked, then written; any problem rejects the call and nothing is written. A file already at a target path is refused with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it` unless `overwrite` is true. [row 4]
- A password given for a tar family archive is refused with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`. [row 5]
- A corrupt archive is refused with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid <format> file:`. [row 6]
- The tar family decompresses its xz stream with no size limit, reads exactly one xz stream; any byte after it, a second stream or padding, makes the decoder fail, so it is refused as corrupt. [row 7]
- Two limits apply to what an extraction writes: the project's storage limit (optional, checked on net growth, refusal `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`) and the member's session write allowance (always set, charged the gross size, refusal `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of <N> bytes exceeded`). Both are checked against sizes the archive declares for its entries, before anything is written. An xz file does not declare its output size in a way this check can use. [row 8]
- On success the answer gives the destination, the files written, the folders created (1 when the destination folder did not exist, else 0) and the bytes written; one audit line is recorded. [row 9]

### 2.2 Specs governing this area

- SPEC-0018 (archive extract in place) defines the behavior above; its extension rule makes a bare `.xz` unsupported. This increment deviates: a bare `.xz` becomes supported. SPEC-0018 is not modified.
- SPEC-0081 (bare gzip file) specifies the same journey for gzip: output naming, the two limits, bomb protection, the destination and collision rules and the order of refusals. This increment restates those rules for xz so it reads alone, and the two specs do not depend on each other. [row 10]
- SPEC-0020 governs the project storage limit; the session write allowance predates it (SPEC-0018). [row 11]

### 2.3 What is verified today

- No test covers a bare `.xz` file; the refusal is not pinned. The xz decoder is exercised only through tar archives. [row 12]
- No test covers several xz streams, zero padding, a decompression bomb or an output name derived from the archive name. [row 12]

## 3. Scope

### 3.1 In scope

- A file whose name ends in `.xz`, in any letter case, and that is not a tar suffix (`.tar.xz`, `.txz`).
- The `destination` and `overwrite` options, the refusals listed in Section 6, and the HTTP API door.

### 3.2 Out of scope, and why

- `.bz2` single files: BL-0113, its own increment.
- A size limit for the tar family: BL-0124.
- The tar family (`.tar.xz`, `.txz`) keeps its current xz decoder, so a BCJ filter or a SHA-256 check inside a tar archive is still refused as corrupt there. Moving it to the new decoder is BL-0132.
- Creating xz files: nothing in the product creates archives.

### 3.3 Constraints imposed by the context

None.

## 4. Actors

- **Member:** a person with access to a project; extracts files through the assistant tool or the HTTP API.
- **Operator:** runs the server and reads the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored compressed file into project files | archive, output file, destination, refusal |
| Project storage | the files, the storage limit and the session write allowance of a project | storage limit, session write allowance, collision, overwrite |

## 5. Usage scenarios

### SC-001: Decompress an xz file
**Actor:** Member.
**Preconditions:** the project holds `/uploads/data.csv.xz`, an xz compression of the 16 byte text `id,name`, newline, `1,Alice`, newline. Nothing exists at `/uploads/data.csv`. The folder `/uploads` exists.
**Flow:**
1. The member asks to extract `/uploads/data.csv.xz` → the system decompresses it and writes `/uploads/data.csv`.
2. The system answers destination `/uploads`, 1 file written, 0 folders created, 16 bytes written.
**Postconditions:** `/uploads/data.csv` holds the 16 bytes; `/uploads/data.csv.xz` is untouched; one audit line records the extraction.
**Exceptions:**
- EXC-001a: the output name cannot be derived (file named `.xz` or `..xz`) → `ERR_INVALID_ARGUMENT`, `cannot derive an output file name from '<name>'`.
- EXC-001b: the member asks for a destination `/new/dir` that does not exist → the folder is created and the answer says 1 folder created.
**Interactions:** SC-002, SC-003.

### SC-002: Collision and limits
**Actor:** Member.
**Preconditions:** the situation of SC-001, except that a file exists at `/uploads/data.csv`.
**Flow:**
1. `overwrite` false → `ERR_NO_CLOBBER` with `'/uploads/data.csv' already exists; pass overwrite=true to replace it`; nothing is written.
2. The member repeats with `overwrite` true → the file is replaced; the project storage is charged the net growth and the session allowance the full output.
3. With no file at the output path, or `overwrite` true, the decompressed output exceeds the project's remaining storage → decoding stops there and the member sees `ERR_PROJECT_QUOTA_EXCEEDED`; nothing is written and the remaining storage is unchanged.
4. With no file at the output path, or `overwrite` true, the decompressed output exceeds the member's remaining session write allowance → decoding stops there and the member sees `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of <N> bytes exceeded`; nothing is written and the allowance is unchanged.
**Postconditions:** after step 1, 3 or 4 the project and both limits are unchanged.
**Exceptions:**
- EXC-002a: a folder bears the output name → `ERR_NO_CLOBBER` with `'<path>' is a folder and cannot be replaced`, even with `overwrite` true.

### SC-003: Bad input
**Actor:** Member.
**Preconditions:** `/uploads/x.xz` is truncated, is a text file, has a damaged checksum, uses a compression filter the decoder cannot read, or has bytes after a valid stream that are not zero padding.
**Flow:**
1. The member asks to extract it → `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid xz file:`; nothing is written.
**Order when several refusals apply:** FR-NEW-012.
**Exceptions:**
- EXC-003a: a destination such as `../../x` → `ERR_PATH_OUT_OF_BOUNDS` with `path escapes the volume root: ../../x`.
- EXC-003b: a destination that is a file, or sits under a file → `ERR_INVALID_ARGUMENT` with `'<path>' is a file, not a folder`.

### SC-004: A password or a bad name
**Actor:** Member.
**Flow:**
1. The member gives a password for an `.xz` file → `ERR_INVALID_ARGUMENT` with `password is not applicable to this archive format`; the file is not read.
2. The member extracts a file named `.xz` → EXC-001a.

### SC-005: Tar archives and other extensions are unchanged
**Actor:** Member.
**Flow:**
1. The member extracts `/uploads/pack.tar.xz` or `/uploads/pack.txz` → they extract as before, into the folder named like the archive without its suffix.
2. The member extracts `/uploads/pack.lz` → `ERR_NOT_SUPPORTED` with `unsupported archive extension: '.lz'`.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Decompress an xz file
> WHEN a member asks to extract a file whose name ends in `.xz`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.xz` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- **Business rules:** the default destination is the folder that holds the archive; the output keeps the letter case of the name; `a.b.c.xz` gives `a.b.c` and `README.xz` gives `README`; an xz file holding an empty file gives a 0 byte output (1 file written, 0 bytes written); for an archive directly under the project root, such as `/a.xz`, the destination is reported as `/` and the output is `/a`.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: Destination option
> WHEN a member gives a destination THE archive extraction SHALL write the output file in that folder, create the folder when it does not exist, and count it in the folders created.
- **Business rules:** a trailing slash makes no difference; the folders created is 1 when the destination did not exist and 0 otherwise.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Unusable output name
> IF the archive name without its `.xz` suffix is empty, `.` or `..` THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `cannot derive an output file name from '<name>'`, and write nothing.
- **User visible text:** `<name>` is the file name of the archive as typed (`.xz`, `..xz`, `.XZ`).
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Collision
> For a bare `.xz` file, IF the output path already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it`, and write nothing. IF the output path holds a folder THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' is a folder and cannot be replaced`, with or without `overwrite`, and write nothing.
- **Business rules:** with `overwrite` true a file is replaced.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Project storage limit and decompression bombs
> IF the project has a storage limit AND the net growth of the project's stored bytes caused by the output exceeds the remaining storage THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- **Business rules:** net growth is the output size minus the size of the file it replaces when `overwrite` is true; a net growth equal to the remaining storage is accepted; a project without a storage limit never triggers this refusal.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: Session write allowance
> IF the output exceeds the member's remaining session write allowance THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.
- **Business rules:** on success the allowance is charged the full output size, with no netting for a replaced file; the allowance is always set, so decoding is always bounded; when the output reaches both limits at the same byte, or decoding meets the project storage limit first, the project refusal (FR-NEW-005) is reported; when it meets the session allowance first, this refusal is reported.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-007 [EARS-E]: Several xz streams
> WHEN an xz file holds several xz streams back to back THE archive extraction SHALL decode them all and write their concatenation as the one output file.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-008 [EARS-O]: Corrupt file
> IF the file is empty, is truncated, is not xz, has a damaged checksum or index, uses a filter or an integrity check type the xz decoder does not support, or has bytes after a valid stream that FR-NEW-013 does not accept as padding, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid xz file:`, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Password given
> IF a password is given, empty or not, for an `.xz` file THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`, before reading the file.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-U]: Tar archives and other extensions unchanged
> THE archive extraction SHALL extract a file ending in `.tar.xz` or `.txz`, in any letter case, as a tar archive exactly as before this increment, and SHALL keep refusing every other extension that is not supported, such as `.lz`, with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '<extension as typed>'`; the collision, destination and order of refusals rules of zip, 7z and tar archives are unchanged by this increment.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-011 [EARS-O]: Destination is a file
> For a bare `.xz` file, IF the destination is an existing file, or has a file among its ancestors, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a file, not a folder`, naming the file found, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-012 [EARS-O]: Order of refusals
> For a bare `.xz` file, IF several refusals apply to the same call THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, message `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, message `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, message `'<path>' is a directory, not an archive file`); (4) a password is given (FR-NEW-009); (5) the output name is unusable (FR-NEW-003); (6) the destination lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (7) the destination is a file or sits under a file (FR-NEW-011); (8) collision (FR-NEW-004); (9) corrupt (FR-NEW-008), project storage limit (FR-NEW-005) or session write allowance (FR-NEW-006), whichever decoding meets first, a tie going to the project storage limit.
- **Why this order:** items 1 to 3 are the shared checks of every format (unchanged, SPEC-0018). Items 4 to 8 need no decoding, which keeps a bomb from being decoded when a cheaper refusal applies.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-013 [EARS-O]: Zero padding
> IF zero bytes follow an xz stream, between streams or at the end of the file, AND their count is a multiple of 4 THEN THE archive extraction SHALL ignore them; IF their count is not a multiple of 4 THEN THE archive extraction SHALL refuse as FR-NEW-008 states.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-U]: A bare `.xz` is supported (modifies SPEC-0018 extension rule)
> THE archive extraction SHALL treat a bare `.xz` as a supported extension.
- **Before:** `ERR_NOT_SUPPORTED` with `unsupported archive extension: '.xz'`.
- **After:** the statement above.
- **Reason:** BL-0112.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

Performance: no new target. Decoding stops at the lower of the two limits, so the work is bounded by it.

### 7.2 Security and access
- THE archive extraction SHALL require the same project membership as for the other formats.
- IF the output of an xz file exceeds a limit THEN THE archive extraction SHALL stop decoding at that limit (FR-NEW-005, FR-NEW-006).

### 7.5 Reliability
- IF an extraction is refused THEN THE archive extraction SHALL leave the project files, the remaining storage and the session write allowance unchanged.

### 7.6 Operability
- WHEN an extraction succeeds THE archive extraction SHALL record one audit line stating who extracted what and where.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | xz stream of an empty file; 0 byte file named `zero.xz`; valid xz file named `.xz` | 0 byte output; corrupt refusal; name refusal | E2E-026, E2E-027, E2E-017 |
| Boundary values | output equal to the remaining storage; both limits at the same byte; padding of 4 bytes and of 3 bytes | accepted; project refusal; accepted; refused | E2E-029, E2E-058, E2E-032, E2E-033 |
| Concurrent actions | not applicable: one synchronous request | none | none |
| Slow or lost connectivity | not applicable: the file is already stored | none | none |
| Access states | not a member | `ERR_FORBIDDEN` | E2E-020 |
| Data variations | accents and spaces; upper case suffix; several dots | kept; detected; last `.xz` removed | E2E-034, E2E-002, E2E-035 |
| Navigation | not applicable | none | none |
| Partial operations | truncated stream; trailing bytes; damaged checksum | corrupt refusal | E2E-011, E2E-013, E2E-014 |
| Dependency failures | not applicable: no external service | none | none |
| Ordering and timing | several refusals at once | FR-NEW-012 | E2E-022 to E2E-025, E2E-051, E2E-055 |
| Exhaustion | storage limit reached; session allowance reached; bomb | FR-NEW-005, FR-NEW-006 | E2E-008, E2E-009, E2E-010 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are provided by the design.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-002 | High |
| E2E-004 | New | happy | SC-001 | FR-NEW-007 | High |
| E2E-005 | New | happy | SC-005 | FR-NEW-010 | High |
| E2E-006 | New | happy | SC-005 | FR-NEW-010 | Medium |
| E2E-007 | New | failure | SC-002 | FR-NEW-004 | High |
| E2E-008 | New | failure | SC-002 | FR-NEW-005 | Critical |
| E2E-009 | New | failure, security | SC-002 | FR-NEW-005 | Critical |
| E2E-010 | New | failure, security | SC-002 | FR-NEW-006 | Critical |
| E2E-011 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-012 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-013 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-014 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-015 | New | happy | SC-003 | FR-NEW-001 | Medium |
| E2E-016 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-017 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-018 | New | failure | SC-003 | FR-NEW-012 | Low |
| E2E-019 | New | failure | SC-003 | FR-NEW-012 | Medium |
| E2E-020 | New | failure | SC-003 | FR-NEW-012 | High |
| E2E-021 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-022 | New | failure | SC-003 | FR-NEW-012 | Medium |
| E2E-023 | New | failure | SC-003 | FR-NEW-012 | Medium |
| E2E-024 | New | failure | SC-003 | FR-NEW-012 | Medium |
| E2E-025 | New | failure | SC-003 | FR-NEW-012, FR-NEW-005 | Medium |
| E2E-026 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-027 | New | failure | SC-003 | FR-NEW-008 | Medium |
| E2E-028 | New | edge | SC-001 | FR-NEW-013 | Medium |
| E2E-029 | New | edge | SC-002 | FR-NEW-005 | Medium |
| E2E-030 | New | edge | SC-002 | FR-NEW-005 | Medium |
| E2E-031 | New | edge | SC-002 | FR-NEW-006 | Medium |
| E2E-032 | New | edge | SC-001 | FR-NEW-013 | Medium |
| E2E-033 | New | failure | SC-003 | FR-NEW-013, FR-NEW-008 | Medium |
| E2E-034 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-035 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-036 | New | side effect | SC-001 | FR-NEW-001 | High |
| E2E-037 | New | side effect | SC-001 | FR-NEW-001 | High |
| E2E-038 | New | side effect | SC-002 | FR-NEW-006 | Medium |
| E2E-039 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-040 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-041 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-042 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-043 | New | happy | SC-001 | FR-NEW-007 | Medium |
| E2E-044 | New | edge | SC-001 | FR-NEW-007 | Medium |
| E2E-045 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-046 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-047 | New | happy | SC-005 | FR-NEW-010 | Medium |
| E2E-048 | New | failure | SC-005 | FR-MOD-001, FR-NEW-010 | Medium |
| E2E-049 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-050 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-051 | New | failure | SC-003 | FR-NEW-011, FR-NEW-012 | Medium |
| E2E-052 | New | edge | SC-001 | FR-NEW-013 | Medium |
| E2E-053 | New | failure | SC-002 | FR-NEW-004 | Low |
| E2E-054 | New | edge | SC-002 | FR-NEW-004 | Medium |
| E2E-055 | New | failure | SC-003 | FR-NEW-012, FR-NEW-003 | Medium |
| E2E-056 | New | happy | SC-003 | FR-NEW-001 | Medium |
| E2E-057 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-058 | New | edge | SC-002 | FR-NEW-005, FR-NEW-006, FR-NEW-012 | Medium |

Coverage (all 58 tests, 58 new and 0 modified): happy 10, failure 30, edge 15, side effect 3, security 2 (E2E-009, E2E-010), state transition 0 (no state machine). Failure to happy ratio 30:10, failure tests outnumber happy tests.

Unless a test says otherwise, the session write allowance is large enough not to matter and the project has no storage limit; a figure for "remaining storage" sets a project storage limit, reached by filling the project with other files up to that remainder (the limit itself is set in whole megabytes); a figure for "session allowance" sets that allowance.

The plan approved before writing listed 37 new tests and a modified test. Two changes: no test pins the bare `.xz` refusal today, so there is no modified test (FR-MOD-001 is covered by E2E-001, E2E-002 and E2E-048); and 21 tests were added so that every requirement has at least 3 tests and the review gaps are covered (destination, output name, password, padding, integrity check, tie between the limits).

### 9.2 New tests

#### E2E-001: xz file decompresses next to the archive
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** a member and `/uploads/data.csv.xz`, the xz compression of the 16 byte text `id,name`, newline, `1,Alice`, newline; nothing at `/uploads/data.csv`
- **When** the member extracts `/uploads/data.csv.xz`
- **Then** the answer is destination `/uploads`, 1 file written, 0 folders created, 16 bytes written
- **And** `/uploads/data.csv` reads the 16 bytes; `/uploads/data.csv.xz` is unchanged

#### E2E-002: Upper case suffix
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** the same content stored as `/uploads/DATA.CSV.XZ`
- **When** the member extracts it
- **Then** `/uploads/DATA.CSV` exists with the 16 bytes

#### E2E-003: Explicit existing destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.xz` and the existing empty folder `/out`
- **When** the member extracts it to the destination `/out`
- **Then** the answer is destination `/out`, 1 file written, 0 folders created; `/out/data.csv` exists and `/uploads/data.csv` does not

#### E2E-004: Two xz streams back to back
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/two.txt.xz` made of two xz streams, the first holding `hello ` and the second `world`
- **When** the member extracts it
- **Then** `/uploads/two.txt` reads `hello world` and the answer reports 11 bytes written

#### E2E-005: `.tar.xz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/uploads/pack.tar.xz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack`, 1 file written, 1 folder created; `/uploads/pack/a.txt` exists

#### E2E-006: `.txz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.txz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack` and `/uploads/pack/a.txt` exists

#### E2E-007: Collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/uploads/data.csv.xz` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' already exists; pass overwrite=true to replace it`
- **And** `/uploads/data.csv` still reads `old`

#### E2E-008: Output over the project storage limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/data.csv` does not exist and the remaining storage is still 10 bytes

#### E2E-009: Decompression bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 1 000 bytes of remaining storage and `/uploads/bomb.xz`, an xz file of about 1 KB that decompresses to 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, and the answer arrives without the server holding the 10 000 000 bytes (the server's memory use during the call stays below 1 000 000 bytes above its level before the call)
- **And** `/uploads/bomb` does not exist and the remaining storage is still 1 000 bytes

#### E2E-010: Decompression bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project without a storage limit, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.xz` as in E2E-009
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`, reached without the server holding the 10 000 000 bytes (memory as in E2E-009)
- **And** `/uploads/bomb` does not exist and the allowance is still 1 000 bytes

#### E2E-011: Truncated file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/cut.xz`, the first half of a valid xz file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid xz file:`; `/uploads/cut` does not exist

#### E2E-012: Text file named `.xz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/fake.xz` containing the text `not xz`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/fake` does not exist

#### E2E-013: Bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/tail.xz`, a valid xz file followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/tail` does not exist

#### E2E-014: Damaged checksum
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/bad.xz`, a valid xz file of `/uploads/data.csv.xz` with one byte of its compressed data changed
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/bad` does not exist

#### E2E-015: A file with an x86 branch filter decodes
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/bcj.xz`, a valid xz file made with an additional x86 branch filter, holding `hello`
- **When** the member extracts it
- **Then** `/uploads/bcj` reads `hello` and 1 file and 5 bytes are written

#### E2E-016: Password given
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/data.csv.xz`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`; `/uploads/data.csv` does not exist

#### E2E-017: File named `.xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.xz`, a valid xz file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `cannot derive an output file name from '.xz'`; nothing is written

#### E2E-018: Missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Low
- **Given** no file at `/uploads/ghost.xz`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.xz' not found`

#### E2E-019: Folder named like an archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a folder `/uploads/dir.xz`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.xz' is a directory, not an archive file`

#### E2E-020: Non member
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** High
- **Given** `/uploads/data.csv.xz` in project `alpha` and `bob@example.com`, who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written

#### E2E-021: Destination is a file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`; `/uploads/notes.txt` is unchanged

#### E2E-022: Destination outside the project wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/fake.xz` containing the text `not xz`
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not the corrupt refusal; nothing is written

#### E2E-023: Password wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/fake.xz` containing the text `not xz`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`

#### E2E-024: Collision wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/cut.xz` truncated and an existing `/uploads/cut` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not the corrupt refusal

#### E2E-025: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012, FR-NEW-005; **Priority:** Medium
- **Given** the situation of E2E-008 and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`

#### E2E-026: xz of an empty file
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/empty.txt.xz`, the xz compression of a 0 byte file
- **When** the member extracts it
- **Then** the answer is 1 file written, 0 bytes written; `/uploads/empty.txt` exists with 0 bytes

#### E2E-027: A 0 byte file named `zero.xz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** a 0 byte file `/uploads/zero.xz`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/zero` does not exist

#### E2E-028: Zero padding after a stream is accepted
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/pad.txt.xz`, a valid xz stream holding `hello` followed by 4 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pad.txt` reads `hello` and 5 bytes are written

#### E2E-029: Output exactly equal to the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the remaining storage is 0 bytes

#### E2E-030: Overwrite counts net growth
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` (16 bytes decoded), an existing `/uploads/data.csv` of 40 bytes, and a project storage limit with 5 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds, `/uploads/data.csv` holds the 16 bytes and the remaining storage is 29 bytes

#### E2E-031: The allowance is charged the full output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, an existing `/uploads/data.csv` of 40 bytes and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds and the allowance has 4 bytes remaining (16 charged, nothing netted)

#### E2E-032: Padding of 4 bytes between two streams
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/gap.txt.xz` made of a stream holding `a`, 4 zero bytes, then a stream holding `b`
- **When** the member extracts it
- **Then** `/uploads/gap.txt` reads `ab`

#### E2E-033: Padding of 3 bytes is refused
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/pad3.xz`, a valid xz stream followed by 3 zero bytes
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/pad3` does not exist

#### E2E-034: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/rapport été.txt.xz`
- **When** the member extracts it
- **Then** `/uploads/rapport été.txt` exists

#### E2E-035: Several dots and no dot
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/a.b.c.xz` and `/uploads/README.xz`
- **When** the member extracts each
- **Then** `/uploads/a.b.c` and `/uploads/README` exist

#### E2E-036: Success is audited
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** the member extracts it
- **Then** the audit trail has exactly one extraction line for `/uploads/data.csv.xz` with the destination, 1 file written and 16 bytes written

#### E2E-037: The HTTP API gives the same answer
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/data.csv.xz`
- **Then** the answer is that of E2E-001, and a second call with `overwrite` false carries the error of E2E-007

#### E2E-038: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 10 bytes remaining and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 7 bytes remaining

#### E2E-039: Missing destination is created
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and no folder `/new/dir`
- **When** the member extracts it to the destination `/new/dir`
- **Then** the answer is destination `/new/dir`, 1 file written, 1 folder created; `/new/dir/data.csv` exists

#### E2E-040: Trailing slash on the destination
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and the existing folder `/out`
- **When** the member extracts it to the destination `/out/`
- **Then** the answer is destination `/out` and `/out/data.csv` exists

#### E2E-041: File named `..xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/..xz`, a valid xz file
- **When** the member extracts it
- **Then** the refusal is as E2E-017 with `'..xz'`; nothing is written

#### E2E-042: Upper case `.XZ` alone
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.XZ`, a valid xz file
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.XZ'`

#### E2E-043: Three xz streams
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/three.txt.xz` made of the streams `a`, `b`, `c`
- **When** the member extracts it
- **Then** `/uploads/three.txt` reads `abc` and 3 bytes are written

#### E2E-044: An empty xz stream between two others
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/hole.txt.xz` made of the streams `a`, an empty stream, `b`
- **When** the member extracts it
- **Then** `/uploads/hole.txt` reads `ab`

#### E2E-045: Empty password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.xz`
- **When** the member extracts it with an empty password
- **Then** the refusal is as E2E-016

#### E2E-046: Password through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.xz`
- **When** a client asks the HTTP API to extract it with the password `s3cret`
- **Then** the error code and message are those of E2E-016

#### E2E-047: `.TAR.XZ` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/PACK.TAR.XZ` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/PACK` and `/uploads/PACK/a.txt` exists

#### E2E-048: Another extension is still refused
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-MOD-001, FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.lz` holding `anything`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.lz'`

#### E2E-049: Destination is a file, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** the situation of E2E-021
- **When** a client asks the HTTP API to extract it to the destination `/uploads/notes.txt`
- **Then** the error code and message are those of E2E-021

#### E2E-050: A file among the ancestors of the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt/sub`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`

#### E2E-051: Destination is a file, wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/data.csv.xz`, a file `/uploads/notes.txt` and an existing `/uploads/data.csv`
- **When** the member extracts it to the destination `/uploads/notes.txt` with `overwrite` false
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`, not `ERR_NO_CLOBBER`

#### E2E-052: Padding of 8 bytes after a stream
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/pad8.txt.xz`, a valid xz stream holding `hello` followed by 8 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pad8.txt` reads `hello`

#### E2E-053: A folder bears the output name
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Low
- **Given** `/uploads/data.csv.xz` and a folder `/uploads/data.csv`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' is a folder and cannot be replaced`; the folder is unchanged

#### E2E-054: Overwrite replaces a file
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` true
- **Then** `/uploads/data.csv` reads the 16 bytes of the content

#### E2E-055: Unusable name wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.xz` containing the text `not xz`
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.xz'`, not the corrupt refusal

#### E2E-056: A file with a SHA-256 integrity check decodes
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/sha.xz`, a valid xz file made with a SHA-256 integrity check, holding `hello`
- **When** the member extracts it
- **Then** `/uploads/sha` reads `hello` and 1 file and 5 bytes are written

#### E2E-057: Archive directly under the project root
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/a.xz`, the xz compression of `hello`
- **When** the member extracts it
- **Then** the answer is destination `/`, 1 file written, 0 folders created, 5 bytes written; `/a` reads `hello`

#### E2E-058: Both limits at the same byte
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006, FR-NEW-012; **Priority:** Medium
- **Given** a project storage limit with 1 000 bytes of remaining storage, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.xz` as in E2E-009
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged

### 9.3 Exploit tests

Not applicable (`--security` not set); E2E-009 and E2E-010 are ordinary acceptance tests.

### 9.4 Modified tests

None: no existing test pins the bare `.xz` refusal. The nearest pinned refusals (`.rar`, bare `.gz`, `.zip.001`) belong to other increments.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-013 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-043 | none | E2E-026, E2E-028, E2E-032, E2E-034, E2E-035, E2E-039, E2E-040, E2E-044, E2E-052, E2E-057 | E2E-036, E2E-037 |
| SC-002 | FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-012 | none | E2E-007, E2E-008, E2E-009, E2E-010, E2E-053 | E2E-029, E2E-030, E2E-031, E2E-054, E2E-058 | E2E-038 |
| SC-003 | FR-NEW-003, FR-NEW-005, FR-NEW-008, FR-NEW-011, FR-NEW-012, FR-NEW-013 | none | E2E-011, E2E-012, E2E-013, E2E-014, E2E-015, E2E-018, E2E-019, E2E-020, E2E-021, E2E-022, E2E-023, E2E-024, E2E-025, E2E-027, E2E-033, E2E-049, E2E-050, E2E-051, E2E-055, E2E-056 | none | none |
| SC-004 | FR-NEW-003, FR-NEW-009 | none | E2E-016, E2E-017, E2E-041, E2E-042, E2E-045, E2E-046 | none | none |
| SC-005 | FR-MOD-001, FR-NEW-010 | E2E-005, E2E-006, E2E-047 | E2E-048 | none | none |

SC-004 is a refusal scenario; its happy path is SC-001. SC-003 is a refusal scenario; its success branches are the padding cases (E2E-028, E2E-032, E2E-052). SC-002 is a refusal scenario whose success branches are overwrite (E2E-030, E2E-031, E2E-054) and the exact fit (E2E-029). SC-005 is a regression scenario.

| Requirement | Tests | Count |
|---|---|---|
| FR-MOD-001 | E2E-001, E2E-002, E2E-048 | 3 |
| FR-NEW-001 | E2E-001, E2E-002, E2E-026, E2E-034, E2E-035, E2E-036, E2E-037, E2E-057 | 8 |
| FR-NEW-002 | E2E-003, E2E-039, E2E-040 | 3 |
| FR-NEW-003 | E2E-017, E2E-041, E2E-042, E2E-055 | 4 |
| FR-NEW-004 | E2E-007, E2E-053, E2E-054 | 3 |
| FR-NEW-005 | E2E-008, E2E-009, E2E-025, E2E-029, E2E-030, E2E-058 | 6 |
| FR-NEW-006 | E2E-010, E2E-031, E2E-038, E2E-058 | 4 |
| FR-NEW-007 | E2E-004, E2E-043, E2E-044 | 3 |
| FR-NEW-008 | E2E-011, E2E-012, E2E-013, E2E-014, E2E-027, E2E-033 | 6 |
| FR-NEW-009 | E2E-016, E2E-045, E2E-046 | 3 |
| FR-NEW-010 | E2E-005, E2E-006, E2E-047, E2E-048 | 4 |
| FR-NEW-011 | E2E-021, E2E-049, E2E-050, E2E-051 | 4 |
| FR-NEW-012 | E2E-018, E2E-019, E2E-020, E2E-022, E2E-023, E2E-024, E2E-025, E2E-051, E2E-055, E2E-058 | 10 |
| FR-NEW-013 | E2E-028, E2E-032, E2E-033, E2E-052 | 4 |

## 11. Open questions

None.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Archive | a stored file that holds other files, or one compressed file | extraction |
| Output file | the single file produced by decompressing an xz file | extraction |
| Destination | the folder that receives the output | extraction |
| Refusal | an error answer; the project is left unchanged | extraction |
| xz stream | one complete compressed unit inside an xz file; a file can hold several | extraction |
| Zero padding | zero bytes between or after xz streams, accepted when their count is a multiple of 4 | extraction |
| Decompression bomb | a small compressed file that expands to a huge output | extraction |
| Collision | the output path already holds a file or a folder | extraction |
| Overwrite | the choice to replace a colliding file instead of refusing | extraction |
| Password | the secret option, not applicable to xz | extraction |
| Storage limit | the optional cap on the total bytes a project stores, set in whole megabytes | project storage |
| Remaining storage | the project's storage limit minus the bytes already stored; no figure when the project has no limit | project storage |
| Session write allowance | the bytes a member can still write in the current session; always set | project storage |
| Net growth | the output size minus the size of the file it replaces | project storage |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server and reads its audit trail | extraction |
| Audit line | the one line an operator reads to see who extracted what and where | extraction |
| HTTP API | the web endpoint a program calls to extract a file; the second door | extraction |
| Assistant tool | the tool an AI assistant calls to extract a file | extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 3 | 4 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied after round 1:** F-1 scope limited to a bare `.xz` file for FR-NEW-004, FR-NEW-011, FR-NEW-012 and tar unchanged in FR-NEW-010; F-2 integrity check type (FR-NEW-008, E2E-056); F-3 root level archive (FR-NEW-001, E2E-057); A-1 the decoder rejects data after a stream (2.1, 14.1 row 7, 14.3); A-2 and A-3 the decoder holds a whole block in memory, caps computed before decoding (14.3); A-4 citation; consistency fixes (E2E-022, E2E-049, E2E-051 retags, FR-NEW-013 shape, [row 12], SC-002 steps 3 and 4, tie test E2E-058).
**No implementation detail outside Section 14:** option names (`destination`, `overwrite`, password), error codes and verbatim messages are WHAT.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | tar family, zip, 7z, case insensitive suffix detection; two doors | `crates/core/src/tools/archive.rs:40-69`, `crates/core/src/api/dataplane.rs:94-95` |
| 2 | bare `.xz` refused with the quoted message (no `.xz` entry in the table) | `crates/core/src/tools/archive.rs:40-50`, `:62-66` |
| 3 | destination is a folder, default is the path minus the suffix | `crates/core/src/tools/archive.rs:166-169` |
| 4 | read whole, check, write; collision message | `crates/core/src/tools/archive.rs:131-215`, collision `:172-181` |
| 5 | password against tar family refused | `crates/core/src/tools/archive.rs:146-150` |
| 6 | corrupt message | `crates/core/src/tools/archive.rs:350-354` |
| 7 | xz decoded with no cap, one stream; data after the stream fails with `Unexpected data after last XZ block` | `crates/core/src/tools/archive.rs:650-655`, `lzma-rs-0.3.0/src/decode/xz.rs:18-91` (the crate version locked by `Cargo.toml:87`) |
| 8 | two limits: project storage on net growth (`ERR_PROJECT_QUOTA_EXCEEDED`), session allowance on the gross declared total (`ERR_WRITE_QUOTA_EXCEEDED`); project checked first, session charged only after | `crates/core/src/tools/archive.rs:183-205`, `crates/core/src/quota.rs:96-113`, `crates/core/src/safety.rs:136-147`, `crates/core/src/errors.rs:101-112`, `:19`, `:24` |
| 9 | answer shape, folder counting, audit line | `crates/core/src/tools/archive.rs:214-275` |
| 10 | SPEC-0081 specifies the same journey for gzip | `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-spec.md` (Summary) |
| 11 | SPEC-0020 defines the project storage refusal; the session allowance exists since SPEC-0018 | `sdlc/specs/SPEC-0020-project-quota-enforcement/sdlc-spec.md:150`, `crates/core/src/safety.rs:136-147` |
| 12 | no test of a bare `.xz` | `rg -n '\.xz"' crates/core/src/tools/archive.rs` lists only tar names (`:863-864`, `:882-883`), recounted; the same search finds no multi stream, padding, bomb or output name test |

Test command: `cargo test -p mcp-fs-core tools::archive`

### 14.2 Class sweep

Not applicable (`--security` not set).

### 14.3 Design inputs

- The decoder takes a writer: a capped writer that fails when the output exceeds the lower of two caps (the project's remaining storage plus the size of the file replaced under `overwrite`, only when the project has a limit, `crates/core/src/safety.rs:72-81`; the remaining session write allowance, `crates/core/src/safety.rs:136-147`) gives the bound. The refusal code follows the cap met first, a tie going to the project (FR-NEW-005, FR-NEW-006). With lzma-rs the writer cap sees bytes only after a whole block is decoded into memory (`xz.rs:226-285`), so a one block bomb (the usual output of the xz command line tool) is held whole: E2E-009 and E2E-010 need a decoder that stops mid block, or a bound on the block's decoded size. `liblzma` already appears in `Cargo.lock`. The memory observable needs a measurable hook. Signal: E2E-009, E2E-010 (red with lzma-rs). The caps are computed before decoding; the declared size admission (`archive.rs:183-205`) is then called with the real decoded size after a successful decode.
- `decode_stream` rejects any byte after the stream (`lzma-rs-0.3.0/src/decode/xz.rs:86-90`). The design must split the input into streams itself, walking each footer and index, skip zero padding of a multiple of 4 bytes (FR-NEW-013), call the decoder per stream (FR-NEW-007), or use a decoder that supports multiple streams; any other trailing byte is refused (FR-NEW-008). Signal: E2E-004, E2E-013, E2E-028, E2E-033.
- The decoder is `liblzma` 0.4.9 as a direct dependency (a streaming multi stream reader, MIT OR Apache-2.0, already in `Cargo.lock`): it stops mid block, so a bound on the writer holds the bomb tests, and it decodes the BCJ filters and every integrity check it knows. A file whose filter or check it does not support, or whose stream ends early, takes the corrupt refusal (FR-NEW-008). The tar family keeps `lzma-rs` (BL-0132). Signal: E2E-015, E2E-056, E2E-009, E2E-010.
- The collision and destination checks run before decoding (FR-NEW-012), a reorder of the current flow (`archive.rs:154-181` decodes first). Signal: E2E-024, E2E-022.
- A folder bearing the output name: today's `exists` probe covers files and folders alike (`archive.rs:172-181`) and `overwrite` true bypasses it. Signal: E2E-053.
- The answer keys (`destination`, `files_written`, `dirs_created`, `bytes_written`) and the audit line are shared (`archive.rs:242-275`).
- The same decoding flow serves SPEC-0081 (gzip) and BL-0113 (bzip2): one bounded decode with per format readers. Update `.agent_docs/tools.md:180-183` and `README.md`; `TOOL_CONTRACT.txt` changes only if a parameter description changes. BL-0124 records the tar family cap.

## Erratum 1 (design issue of 2026-11-05)

The only xz decoder in the tree, `lzma-rs` 0.3, decodes a whole block into memory and rejects any stream after the first, so E2E-009, E2E-010 and FR-NEW-007 could not hold. Decision: use `liblzma` as a direct dependency (the user's default, option (a) of `design-issues.md`). Consequences: E2E-015 (x86 branch filter) and E2E-056 (SHA-256 check) become happy tests; FR-NEW-008 and Section 3.2 lose the filter and check refusals; the tar family keeps its decoder (BL-0132). The total is 58 tests, happy 10, failure 30.
