# Extract a bare gzip file: functional specification

> Id: SPEC-0081
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction), about 12 anticipated requirements, public contract change (a refusal becomes a success), no persisted change. Override with --depth.
> Status: implemented
> Generated on: 2026-11-04
> Project: existing
> From backlog: BL-0111
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

A project member can decompress a single file compressed with gzip and stored in their project, for example `data.csv.gz` giving `data.csv`, with the same extraction tool they already use for zip, 7z and tar archives. The file lands next to the compressed file unless the member names another folder. The same safety rules apply: no overwrite unless asked, the project's storage limit and the member's session write allowance are respected, and a decompression bomb is stopped as soon as its output exceeds the lower of the two. Compressed tar archives (`.tar.gz`, `.tgz`) are unchanged.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member can extract `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz` archives in place, from the assistant tool and from the HTTP API. The format is read from the file name alone, ignoring letter case. [14.1 row 1]
- A bare `.gz` file (not a tar) is refused with `ERR_NOT_SUPPORTED` and a message containing `.gz`. [row 2]
- The destination of an extraction is a folder; when none is given it is the archive path without its suffix, a new folder. [row 3]
- An archive is read whole, then checked, then written; any problem rejects the call and nothing is written. A file already at a target path is refused with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it` unless `overwrite` is true. [row 4]
- A password given for a tar family archive is refused with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`. [row 5]
- A corrupt archive is refused with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid <format> file:`. [row 6]
- The tar family decompresses its stream with no size limit, and a gzip stream with several members is decoded only up to its first member. [row 7]
- Two limits apply to what an extraction writes: the project's storage limit (optional, checked on net growth, refusal `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`) and the member's session write allowance (always set, charged the gross size, refusal `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of <N> bytes exceeded`). Both are checked against the size the archive declares for each entry, before anything is written. A bare gzip file declares no reliable size. [row 8]
- On success the answer gives the destination, the files written, the folders created (1 when the destination folder did not exist, else 0) and the bytes written; one audit line is recorded. [row 9]

### 2.2 Specs governing this area

- SPEC-0018 (archive extract in place) defines the behavior above, and its extension rule makes a bare `.gz` unsupported (test E2E-NEW-034). This increment deviates: a bare `.gz` becomes supported. SPEC-0018 is not modified.
- SPEC-0080 (RAR archives) adds `.rar` through the same tool; the two specs do not depend on each other. [row 14]
- SPEC-0020 governs the project storage limit; the session write allowance predates it (SPEC-0018). [row 15]

### 2.3 What is verified today

- The refusal of a bare `.gz` is pinned by one test (E2E-NEW-034). Gzip decoding is verified only through tar archives. [row 10]
- No test covers a stream with several gzip members, a decompression bomb, or an output name derived from the archive name.

## 3. Scope

### 3.1 In scope

- A file whose name ends in `.gz`, in any letter case, and that is not a tar suffix (`.tar.gz`, `.tgz`).
- The `destination` and `overwrite` options, the refusals listed in Section 6, and the HTTP API door.

### 3.2 Out of scope, and why

- `.xz` and `.bz2` single files: BL-0112 and BL-0113, each its own increment.
- A size limit for the tar family: BL-0124.
- Reading the file name or comments stored in the gzip header: ignored on purpose (Section 6, FR-NEW-004).
- Creating gzip files: nothing in the product creates archives.

### 3.3 Constraints imposed by the context

None.

## 4. Actors

- **Member:** a person with access to a project; extracts files through the assistant tool or the HTTP API.
- **Operator:** runs the server and reads the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored compressed file into project files | archive, output file, destination, refusal |
| Project storage | files, the storage limit and the session write allowance of a project | storage limit, session write allowance, collision, overwrite |

## 5. Usage scenarios

### SC-001: Decompress a gzip file
**Actor:** Member.
**Preconditions:** the project holds `/uploads/data.csv.gz`, a gzip of the 16 byte text `id,name`, newline, `1,Alice`, newline. Nothing exists at `/uploads/data.csv`. The folder `/uploads` exists.
**Flow:**
1. The member asks to extract `/uploads/data.csv.gz` → the system decompresses it and writes `/uploads/data.csv`.
2. The system answers destination `/uploads`, 1 file written, 0 folders created, 16 bytes written.
**Postconditions:** `/uploads/data.csv` holds the 16 bytes; `/uploads/data.csv.gz` is untouched; one audit line records the extraction.
**Exceptions:**
- EXC-001a: the output name cannot be derived (file named `.gz` or `..gz`) → `ERR_INVALID_ARGUMENT`, `cannot derive an output file name from '<name>'`.
- EXC-001b: the member asks for a destination `/new/dir` that does not exist → the folder is created and the answer says 1 folder created.
**Interactions:** SC-002, SC-003.

### SC-002: Collision and quota
**Actor:** Member.
**Preconditions:** the situation of SC-001, except that a file exists at `/uploads/data.csv`.
**Flow:**
1. A file already exists at `/uploads/data.csv`, `overwrite` false → `ERR_NO_CLOBBER` with `'/uploads/data.csv' already exists; pass overwrite=true to replace it`; nothing is written.
2. The member repeats with `overwrite` true → the file is replaced; the project storage is charged the net growth and the session allowance the full output.
3. The decompressed output exceeds the project's remaining storage → decoding stops there and the member sees `ERR_PROJECT_QUOTA_EXCEEDED`; nothing is written and the remaining storage is unchanged.
4. The decompressed output exceeds the member's remaining session write allowance → decoding stops there and the member sees `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of <N> bytes exceeded`; nothing is written and the allowance is unchanged.
**Postconditions:** after step 1, 3 or 4 the project and both limits are unchanged.
**Exceptions:**
- EXC-002a: a folder bears the output name → `ERR_NO_CLOBBER` with `'<path>' is a folder and cannot be replaced`, even with `overwrite` true.

### SC-003: Bad input
**Actor:** Member.
**Preconditions:** `/uploads/x.gz` is truncated, is a text file, or has non gzip bytes after a valid gzip stream.
**Flow:**
1. The member asks to extract it → `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid gz file:`; nothing is written.
**Order when several refusals apply:** FR-NEW-011.
**Exceptions:**
- EXC-003a: a destination such as `../../x` → `ERR_PATH_OUT_OF_BOUNDS` with `path escapes the volume root: ../../x`.
- EXC-003b: a destination that is a file, or sits under a file → `ERR_INVALID_ARGUMENT` with `'<path>' is a file, not a folder`.

### SC-004: A password or a bad name
**Actor:** Member.
**Flow:**
1. The member gives a password for a `.gz` file → `ERR_INVALID_ARGUMENT` with `password is not applicable to this archive format`; the file is not read.
2. The member extracts a file named `.gz` → EXC-001a.

### SC-005: Tar archives are unchanged
**Actor:** Member.
**Flow:**
1. The member extracts `/uploads/pack.tar.gz` or `/uploads/pack.tgz` → they extract as before, into the folder named like the archive without its suffix.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Decompress a gzip file
> WHEN a member asks to extract a file whose name ends in `.gz`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.gz` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- **Business rules:** the default destination is the folder that holds the archive; the output keeps the letter case of the name; the name `a.b.c.gz` gives `a.b.c` and `README.gz` gives `README`; a gzip of an empty file gives a 0 byte file (1 file written, 0 bytes written).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: Destination option
> WHEN a member gives a destination THE archive extraction SHALL write the output file in that folder, create the folder when it does not exist, and count it in the folders created.
- **Business rules:** a trailing slash makes no difference; the folders created is 1 when the destination did not exist and 0 otherwise.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Unusable output name
> IF the archive name without its `.gz` suffix is empty, `.` or `..` THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `cannot derive an output file name from '<name>'`, and write nothing.
- **User visible text:** `<name>` is the file name of the archive as typed (`.gz`, `..gz`, `.GZ`).
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-004 [EARS-UB]: Names inside the file are ignored
> THE archive extraction SHALL NOT use any name, comment or extra field stored inside the gzip file to name or place the output.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Collision
> IF the output path already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it`, and write nothing. IF the output path holds a folder THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' is a folder and cannot be replaced`, with or without `overwrite`, and write nothing.
- **Business rules:** with `overwrite` true a file is replaced.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: Project storage limit and decompression bombs
> IF the project has a storage limit AND the net growth of the project's stored bytes caused by the output exceeds the remaining storage THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- **Business rules:** net growth is the output size minus the size of the file it replaces when `overwrite` is true; a net growth equal to the remaining storage is accepted; a project without a storage limit never triggers this refusal.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-007 [EARS-E]: Several gzip members
> WHEN a gzip file holds several members back to back THE archive extraction SHALL decode them all and write their concatenation as the one output file.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-008 [EARS-O]: Corrupt file
> IF the file is empty, is truncated, is not gzip, or has bytes that are not gzip after a valid stream THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid gz file:`, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Password given
> IF a password is given, empty or not, for a `.gz` file THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`, before reading the file.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-U]: Tar archives unchanged
> THE archive extraction SHALL extract a file ending in `.tar.gz`, `.tgz` or the same suffixes in any letter case as a tar archive, exactly as before this increment.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-011 [EARS-O]: Order of refusals
> IF several refusals apply to the same call THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`); (2) the path does not exist (`ERR_NOT_FOUND`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`); (4) a password is given (FR-NEW-009); (5) the output name is unusable (FR-NEW-003); (6) the destination lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (7) the destination is a file or sits under a file (FR-NEW-013); (8) collision (FR-NEW-005); (9) corrupt (FR-NEW-008), project storage limit (FR-NEW-006) or session write allowance (FR-NEW-012), whichever decoding meets first, a tie going to the project storage limit.
- **Why this order:** items 1 to 3 are the shared checks of every format (unchanged, SPEC-0018). Items 4 to 8 need no decoding, which keeps a bomb from being decoded when a cheaper refusal applies.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-012 [EARS-O]: Session write allowance
> IF the output exceeds the member's remaining session write allowance THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.
- **Business rules:** on success the allowance is charged the full output size, with no netting for a replaced file; the allowance is always set, so decoding is always bounded; when the output reaches both limits at the same byte, or decoding meets the project storage limit first, the project refusal (FR-NEW-006) is reported; when it meets the session allowance first, this refusal is reported.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-013 [EARS-O]: Destination is a file
> IF the destination is an existing file, or has a file among its ancestors, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a file, not a folder`, naming the file found, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-U]: A bare `.gz` is supported (modifies SPEC-0018 extension rule)
> THE archive extraction SHALL treat a bare `.gz` as a supported extension.
- **Before:** `ERR_NOT_SUPPORTED` with a message containing `.gz`.
- **After:** the statement above.
- **Reason:** BL-0111.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

Performance: no new target. Decoding stops at the lower of the two limits, so the work is bounded by it.

### 7.2 Security and access
- THE archive extraction SHALL require the same project membership as for the other formats.
- IF the output of a gzip file exceeds a limit THEN THE archive extraction SHALL stop decoding at that limit (FR-NEW-006, FR-NEW-012).

### 7.5 Reliability
- IF an extraction is refused THEN THE archive extraction SHALL leave the project files, the remaining storage and the session write allowance unchanged.

### 7.6 Operability
- WHEN an extraction succeeds THE archive extraction SHALL record one audit line stating who extracted what and where.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | gzip of an empty file; 0 byte file named `zero.gz`; valid gzip file named `.gz` | 0 byte output; corrupt refusal; name refusal | E2E-023, E2E-046, E2E-012 |
| Boundary values | output equal to the remaining storage; one byte more; session allowance equal to the output | accepted; refused; accepted | E2E-025, E2E-047, E2E-053 |
| Concurrent actions | not applicable: one synchronous request | none | none |
| Slow or lost connectivity | not applicable: the file is already stored | none | none |
| Access states | not a member | `ERR_FORBIDDEN` | E2E-016 |
| Data variations | accents and spaces; upper case suffix; several dots | kept; detected; last `.gz` removed | E2E-026, E2E-002, E2E-027 |
| Navigation | not applicable | none | none |
| Partial operations | truncated stream; trailing bytes | corrupt refusal | E2E-008, E2E-010 |
| Dependency failures | not applicable: no external service | none | none |
| Ordering and timing | several refusals at once | FR-NEW-011 | E2E-019 to E2E-022, E2E-045 |
| Exhaustion | storage limit reached; session allowance reached; bomb | FR-NEW-006, FR-NEW-012 | E2E-007, E2E-017, E2E-048, E2E-052 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are provided by the design.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-002 | High |
| E2E-004 | New | happy | SC-001 | FR-NEW-002 | High |
| E2E-005 | New | happy | SC-001 | FR-NEW-007 | High |
| E2E-006 | New | failure | SC-002 | FR-NEW-005 | High |
| E2E-007 | New | failure | SC-002 | FR-NEW-006 | Critical |
| E2E-008 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-009 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-010 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-011 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-012 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-013 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-014 | New | failure | SC-003 | FR-NEW-011 | Low |
| E2E-015 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-016 | New | failure | SC-003 | FR-NEW-011 | High |
| E2E-017 | New | failure, security | SC-002 | FR-NEW-006 | Critical |
| E2E-018 | New | edge, security | SC-001 | FR-NEW-004 | Critical |
| E2E-019 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-020 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-021 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-022 | New | failure | SC-003 | FR-NEW-011, FR-NEW-006 | Medium |
| E2E-023 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-024 | New | edge | SC-002 | FR-NEW-005 | Medium |
| E2E-025 | New | edge | SC-002 | FR-NEW-006 | Medium |
| E2E-026 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-027 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-028 | New | happy | SC-005 | FR-NEW-010 | High |
| E2E-029 | New | happy | SC-005 | FR-NEW-010 | Medium |
| E2E-030 | New | side effect | SC-001 | FR-NEW-001 | High |
| E2E-031 | New | side effect | SC-001 | FR-NEW-001 | High |
| E2E-032 | New | side effect | SC-002 | FR-NEW-006 | Medium |
| E2E-033 | New | failure | SC-003 | FR-NEW-008 | Medium |
| E2E-034 | New | failure | SC-002 | FR-NEW-005 | Low |
| E2E-035 | New | failure | SC-003 | FR-NEW-011 | Medium |
| E2E-036 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-037 | New | happy | SC-001 | FR-NEW-007 | Medium |
| E2E-038 | New | edge | SC-001 | FR-NEW-007 | Medium |
| E2E-039 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-040 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-041 | New | failure | SC-004 | FR-NEW-003 | Medium |
| E2E-042 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-043 | New | edge | SC-001 | FR-NEW-004 | Medium |
| E2E-044 | New | happy | SC-005 | FR-NEW-010 | Medium |
| E2E-045 | New | failure | SC-003 | FR-NEW-011, FR-NEW-006 | Medium |
| E2E-046 | New | failure | SC-003 | FR-NEW-008 | Medium |
| E2E-047 | New | edge | SC-002 | FR-NEW-006 | Medium |
| E2E-048 | New | failure, security | SC-002 | FR-NEW-012 | Critical |
| E2E-049 | New | edge | SC-002 | FR-NEW-012 | Medium |
| E2E-050 | New | side effect | SC-002 | FR-NEW-012 | Medium |
| E2E-051 | New | failure | SC-002 | FR-NEW-012, FR-NEW-006 | Medium |
| E2E-052 | New | failure | SC-002 | FR-NEW-012 | Medium |
| E2E-053 | New | edge | SC-002 | FR-NEW-012 | Medium |
| E2E-054 | New | failure | SC-003 | FR-NEW-013, FR-NEW-011 | Medium |
| E2E-055 | New | failure | SC-003 | FR-NEW-013 | Medium |
| E2E-056 | New | failure | SC-003 | FR-NEW-013 | Medium |
| E2E-MOD-001 | Modified | failure | SC-003 | FR-MOD-001, FR-NEW-008 | High |

Coverage (all 57 tests, 56 new and 1 modified): happy 9, failure 31, edge 13, side effect 4, security 3 (E2E-017, E2E-018, E2E-048), state transition 0 (no state machine). Failure to happy ratio 31:9, failure tests outnumber happy tests.

Unless a test says otherwise, the member's session write allowance is large enough not to matter and the project has no storage limit. A test that gives a figure for "remaining storage" sets a project storage limit, reached by filling the project with other files up to that remainder (the limit itself is set in whole megabytes); a figure for "session allowance" sets that allowance.

FR-NEW-011 lists membership, missing path and folder as the shared first checks; they are unchanged behavior, covered here only by one regression test each (E2E-014, E2E-015, E2E-016).

### 9.2 New tests

#### E2E-001: Gzip file decompresses next to the archive
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** a member and `/uploads/data.csv.gz`, the gzip of the 16 byte text `id,name`, newline, `1,Alice`, newline; nothing at `/uploads/data.csv`
- **When** the member extracts `/uploads/data.csv.gz`
- **Then** the answer is destination `/uploads`, 1 file written, 0 folders created, 16 bytes written
- **And** `/uploads/data.csv` reads the 16 bytes; `/uploads/data.csv.gz` is unchanged

#### E2E-002: Upper case suffix
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** the same content stored as `/uploads/DATA.CSV.GZ`
- **When** the member extracts it
- **Then** `/uploads/DATA.CSV` exists with the 16 bytes

#### E2E-003: Explicit existing destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.gz` and the existing empty folder `/out`
- **When** the member extracts it to the destination `/out`
- **Then** the answer is destination `/out`, 1 file written, 0 folders created; `/out/data.csv` exists and `/uploads/data.csv` does not

#### E2E-004: Missing destination is created
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.gz` and no folder `/new/dir`
- **When** the member extracts it to the destination `/new/dir`
- **Then** the answer is destination `/new/dir`, 1 file written, 1 folder created; `/new/dir/data.csv` exists

#### E2E-005: Two gzip members back to back
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/two.txt.gz` made of two gzip members, the first holding `hello ` and the second `world`
- **When** the member extracts it
- **Then** `/uploads/two.txt` reads `hello world` and the answer reports 11 bytes written

#### E2E-006: Collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/data.csv.gz` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' already exists; pass overwrite=true to replace it`
- **And** `/uploads/data.csv` still reads `old`

#### E2E-007: Output over the remaining quota
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/data.csv` does not exist and the remaining storage is still 10 bytes

#### E2E-008: Truncated file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/cut.gz`, the first half of a valid gzip file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid gz file:`; `/uploads/cut` does not exist

#### E2E-009: Text file named `.gz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/fake.gz` containing the text `not gzip`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/fake` does not exist

#### E2E-010: Bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/tail.gz`, a valid gzip file followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/tail` does not exist

#### E2E-011: Password given
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`; `/uploads/data.csv` does not exist

#### E2E-012: File named `.gz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.gz`, a valid gzip file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `cannot derive an output file name from '.gz'`; nothing is written

#### E2E-013: File named `..gz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/..gz`, a valid gzip file
- **When** the member extracts it
- **Then** the refusal is as E2E-012 with `'..gz'`; nothing is written

#### E2E-014: Missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Low
- **Given** no file at `/uploads/ghost.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.gz' not found`

#### E2E-015: Folder named like an archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** a folder `/uploads/dir.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.gz' is a directory, not an archive file`

#### E2E-016: Non member
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/uploads/data.csv.gz` in project `alpha` and `bob@example.com`, who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written

#### E2E-017: Decompression bomb
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project storage limit with 1 000 bytes of remaining storage and `/uploads/bomb.gz`, a gzip file of about 10 KB that decompresses to 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, and the answer arrives without the server holding the 10 000 000 bytes (the server's memory use during the call stays below 1 000 000 bytes above its level before the call)
- **And** `/uploads/bomb` does not exist and the remaining storage is still 1 000 bytes

#### E2E-018: Name stored inside the file is ignored
- **Category:** edge, security; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Critical
- **Given** `/uploads/data.csv.gz` whose gzip header stores the file name `../../etc/passwd`
- **When** the member extracts it
- **Then** `/uploads/data.csv` exists with the content
- **And** no `/etc/passwd` and no `/uploads/etc` exist in the project

#### E2E-019: Membership wins over a missing path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** project `alpha` without `/uploads/ghost.gz` and `bob@example.com`, not a member
- **When** `bob@example.com` extracts `/uploads/ghost.gz`
- **Then** the refusal is `ERR_FORBIDDEN`, not `ERR_NOT_FOUND`

#### E2E-020: Password wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/fake.gz` containing the text `not gzip`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`

#### E2E-021: Collision wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/cut.gz` truncated and an existing `/uploads/cut` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not the corrupt refusal

#### E2E-022: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-006; **Priority:** Medium
- **Given** the situation of E2E-007 and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`

#### E2E-023: Gzip of an empty file
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/empty.txt.gz`, the gzip of a 0 byte file
- **When** the member extracts it
- **Then** the answer is 1 file written, 0 bytes written; `/uploads/empty.txt` exists with 0 bytes

#### E2E-024: Overwrite replaces
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` (16 bytes decoded), an existing `/uploads/data.csv` of 40 bytes, and a project storage limit with 5 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds, `/uploads/data.csv` holds the 16 bytes and the remaining storage is 29 bytes

#### E2E-025: Output exactly equal to the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the remaining storage is 0 bytes

#### E2E-026: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/rapport été.txt.gz`
- **When** the member extracts it
- **Then** `/uploads/rapport été.txt` exists

#### E2E-027: Several dots and no dot
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/a.b.c.gz` and `/uploads/README.gz`
- **When** the member extracts each
- **Then** `/uploads/a.b.c` and `/uploads/README` exist

#### E2E-028: `.tar.gz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/uploads/pack.tar.gz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack`, 1 file written, 1 folder created; `/uploads/pack/a.txt` exists

#### E2E-029: `.tgz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.tgz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack` and `/uploads/pack/a.txt` exists

#### E2E-030: Success is audited
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** the member extracts it
- **Then** the audit trail has exactly one extraction line for `/uploads/data.csv.gz` with the destination, 1 file written and 16 bytes written

#### E2E-031: The HTTP API gives the same answer
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/data.csv.gz`
- **Then** the answer is that of E2E-001, and a second call with `overwrite` false carries the error of E2E-006

#### E2E-032: A refused extraction leaves the project usable
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** the situation of E2E-007
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the remaining storage is 7 bytes

#### E2E-033: A refused extraction creates nothing
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/cut.gz` truncated and no folder `/fresh`
- **When** the member extracts it to the destination `/fresh`
- **Then** the refusal is as E2E-008 and `/fresh` does not exist

#### E2E-034: A folder bears the output name
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Low
- **Given** `/uploads/data.csv.gz` and a folder `/uploads/data.csv`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' is a folder and cannot be replaced`; the folder is unchanged

#### E2E-035: Destination outside the project
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written

#### E2E-036: Trailing slash on the destination
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and the existing folder `/out`
- **When** the member extracts it to the destination `/out/`
- **Then** the answer is destination `/out` and `/out/data.csv` exists

#### E2E-037: Three gzip members
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/three.txt.gz` made of the members `a`, `b`, `c`
- **When** the member extracts it
- **Then** `/uploads/three.txt` reads `abc` and 3 bytes are written

#### E2E-038: An empty gzip member between two others
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/gap.txt.gz` made of the members `a`, an empty member, `b`
- **When** the member extracts it
- **Then** `/uploads/gap.txt` reads `ab`

#### E2E-039: Empty password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with an empty password
- **Then** the refusal is as E2E-011

#### E2E-040: Password through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** a client asks the HTTP API to extract it with the password `s3cret`
- **Then** the error code and message are those of E2E-011

#### E2E-041: Upper case `.GZ` alone
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.GZ`, a valid gzip file
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.GZ'`

#### E2E-042: A different stored name
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` whose gzip header stores the file name `other.txt`
- **When** the member extracts it
- **Then** `/uploads/data.csv` exists and `/uploads/other.txt` does not

#### E2E-043: Comment and extra fields in the header
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` whose header holds a comment `from alice` and an extra field
- **When** the member extracts it
- **Then** `/uploads/data.csv` holds the content and nothing else is created in `/uploads`

#### E2E-044: `.TAR.GZ` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/PACK.TAR.GZ` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/PACK` and `/uploads/PACK/a.txt` exists

#### E2E-045: Storage limit met before the corruption
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/big.gz`, a gzip stream decompressing to 1 000 bytes and then truncated before its end
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, because the limit is met first

#### E2E-046: Empty file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** a 0 byte file `/uploads/zero.gz`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/zero` does not exist

#### E2E-047: One byte over the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and a gzip file decompressing to 17 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` and the remaining storage is still 16 bytes

#### E2E-048: Bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Critical
- **Given** a project without a storage limit, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.gz` as in E2E-017
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`, reached without the server holding the 10 000 000 bytes (memory as in E2E-017)
- **And** `/uploads/bomb` does not exist and the allowance is still 1 000 bytes

#### E2E-049: The allowance is charged the full output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, an existing `/uploads/data.csv` of 40 bytes and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds and the allowance has 4 bytes remaining (16 charged, nothing netted)

#### E2E-050: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 10 bytes remaining and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 7 bytes remaining

#### E2E-051: Both limits at the same byte
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-012, FR-NEW-006; **Priority:** Medium
- **Given** 10 bytes remaining in the project storage and 10 bytes remaining in the session allowance, and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`

#### E2E-052: The lower limit is met first
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** 1 000 bytes remaining in the project storage, 100 bytes remaining in the session allowance, and `/uploads/bomb.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED`

#### E2E-053: Allowance equal to the output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 16 bytes remaining and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the allowance has 0 bytes remaining

#### E2E-054: Destination is a file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013, FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`; `/uploads/notes.txt` is unchanged

#### E2E-055: A file among the ancestors of the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt/sub`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`

#### E2E-056: Destination is a file, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** the situation of E2E-054
- **When** a client asks the HTTP API to extract it to the destination `/uploads/notes.txt`
- **Then** the error code and message are those of E2E-054

### 9.3 Exploit tests

Not applicable (`--security` not set); E2E-017 and E2E-018 are ordinary acceptance tests.

### 9.4 Modified tests

#### E2E-MOD-001: bare `.gz` (was SPEC-0018 E2E-NEW-034)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-008; **Priority:** High
- **Validated before:** the file `/uploads/data.gz` holding the 8 bytes `anything` is refused with `ERR_NOT_SUPPORTED` naming `.gz`.
- **Validates now:** the same file is refused with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid gz file:`, because it is not gzip.
- **Given** `/uploads/data.gz` holding `anything`; **When** the member extracts it; **Then** the corrupt refusal above.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-007 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-037 | none | E2E-018, E2E-023, E2E-026, E2E-027, E2E-036, E2E-038, E2E-042, E2E-043 | E2E-030, E2E-031 |
| SC-002 | FR-NEW-005, FR-NEW-006, FR-NEW-012 | none | E2E-006, E2E-007, E2E-017, E2E-034, E2E-048, E2E-051, E2E-052 | E2E-024, E2E-025, E2E-047, E2E-049, E2E-053 | E2E-032, E2E-050 |
| SC-003 | FR-MOD-001, FR-NEW-006, FR-NEW-008, FR-NEW-011, FR-NEW-013 | none | E2E-008, E2E-009, E2E-010, E2E-014, E2E-015, E2E-016, E2E-019, E2E-020, E2E-021, E2E-022, E2E-033, E2E-035, E2E-045, E2E-046, E2E-054, E2E-055, E2E-056, E2E-MOD-001 | none | none |
| SC-004 | FR-NEW-003, FR-NEW-009 | none | E2E-011, E2E-012, E2E-013, E2E-039, E2E-040, E2E-041 | none | none |
| SC-005 | FR-NEW-010 | E2E-028, E2E-029, E2E-044 | none | none | none |

SC-003, SC-004 and SC-005 are refusal or regression scenarios; the happy path of SC-003 and SC-004 is SC-001. SC-002 is a refusal scenario whose success branch is overwrite (E2E-024, E2E-049) and the exact fits (E2E-025, E2E-053).

| Requirement | Tests | Count |
|---|---|---|
| FR-MOD-001 | E2E-001, E2E-002, E2E-MOD-001 | 3 |
| FR-NEW-001 | E2E-001, E2E-002, E2E-023, E2E-026, E2E-027, E2E-030, E2E-031 | 7 |
| FR-NEW-002 | E2E-003, E2E-004, E2E-036 | 3 |
| FR-NEW-003 | E2E-012, E2E-013, E2E-041 | 3 |
| FR-NEW-004 | E2E-018, E2E-042, E2E-043 | 3 |
| FR-NEW-005 | E2E-006, E2E-024, E2E-034 | 3 |
| FR-NEW-006 | E2E-007, E2E-017, E2E-022, E2E-025, E2E-032, E2E-045, E2E-047, E2E-051 | 8 |
| FR-NEW-007 | E2E-005, E2E-037, E2E-038 | 3 |
| FR-NEW-008 | E2E-008, E2E-009, E2E-010, E2E-033, E2E-046, E2E-MOD-001 | 6 |
| FR-NEW-009 | E2E-011, E2E-039, E2E-040 | 3 |
| FR-NEW-010 | E2E-028, E2E-029, E2E-044 | 3 |
| FR-NEW-011 | E2E-014, E2E-015, E2E-016, E2E-019, E2E-020, E2E-021, E2E-022, E2E-035, E2E-045, E2E-054 | 10 |
| FR-NEW-012 | E2E-048, E2E-049, E2E-050, E2E-051, E2E-052, E2E-053 | 6 |
| FR-NEW-013 | E2E-054, E2E-055, E2E-056 | 3 |

## 11. Open questions

None.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Archive | a stored file that holds other files, or one compressed file | extraction |
| Output file | the single file produced by decompressing a gzip file | extraction |
| Destination | the folder that receives the output | extraction |
| Refusal | an error answer; the project is left unchanged | extraction |
| Gzip member | one compressed stream inside a gzip file; a file can hold several | extraction |
| Decompression bomb | a small compressed file that expands to a huge output | extraction |
| Collision | the output path already holds a file or a folder | extraction |
| Overwrite | the choice to replace a colliding file instead of refusing | extraction |
| Password | the secret option, not applicable to gzip | extraction |
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
| 1 | 2 | 2 | NOT-IMPLEMENTABLE |
| 2 | 1 | 3 | NOT-IMPLEMENTABLE |
| 3 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied after round 1:** F-1 the two limits (FR-NEW-006 project storage, FR-NEW-012 session allowance, tie rule, E2E-007, E2E-017, E2E-047 to E2E-053), F-2 empty file (E2E-046), consistency fixes (modals, Section 8 cells).
**Amendments applied after round 2:** F-1 destination is a file (FR-NEW-013, E2E-054 to E2E-056, order item 7), folder refusal moved into FR-NEW-005, SC-002 charging wording, glossary, evidence rows 11 to 13.
**Amendments applied after round 3 (consistency review):** E2E-022 header, evidence rows 14 and 15.
**No implementation detail outside Section 14:** option names (`destination`, `overwrite`, password), error codes and verbatim messages are WHAT.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | tar family, zip, 7z, case insensitive suffix detection; two doors | `crates/core/src/tools/archive.rs:40-69`, `crates/core/src/api/dataplane.rs:94-95` |
| 2 | bare `.gz` refused, message contains `.gz` | `crates/core/src/tools/archive.rs:62-66`, test `:806-814` |
| 3 | destination is a folder, default is the path minus the suffix | `crates/core/src/tools/archive.rs:162-170` |
| 4 | read whole, check, write; collision message | `crates/core/src/tools/archive.rs:131-215`, collision `:172-181` |
| 5 | password against tar family refused | `crates/core/src/tools/archive.rs:146-150` |
| 6 | corrupt message | `crates/core/src/tools/archive.rs:350-354` |
| 7 | gzip decoded with no cap, first member only (`GzDecoder`, not the multi member decoder) | `crates/core/src/tools/archive.rs:636-642` |
| 8 | two limits: project storage on net growth (`ERR_PROJECT_QUOTA_EXCEEDED`), session allowance on the gross declared total (`ERR_WRITE_QUOTA_EXCEEDED`); project checked first, session charged only after | `crates/core/src/tools/archive.rs:183-205`, `crates/core/src/quota.rs:31-45`, `:100-118`, `crates/core/src/safety.rs:136-147`, `crates/core/src/errors.rs:101-112`, `:19`, `:24` |
| 9 | answer shape, folder counting, audit line | `crates/core/src/tools/archive.rs:214-275` |
| 10 | gzip verified only through tar | `crates/core/src/tools/archive.rs:1754-1772` |
| 14 | SPEC-0080 adds `.rar` through the same tool; SPEC-0020 FR-NEW-001 defines the project storage refusal | `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md` (Summary), `sdlc/specs/SPEC-0020-project-quota-enforcement/sdlc-spec.md:150` |
| 15 | the session write allowance exists since SPEC-0018 and is charged by `charge_write`; the HTTP door is the `extract-archive` route | `crates/core/src/safety.rs:136-147`, `crates/core/src/api/dataplane.rs:94-95` |
| 11 | E2E-NEW-034 is the only `.gz` test | `crates/core/src/tools/archive.rs:806-814` |
| 12 | no test of several members, a bomb or a derived output name | `rg -n 'MultiGz|bomb' crates/core/src/tools/archive.rs` returns 0 lines (recounted) |
| 13 | gzip size is not read, the decoder reads to the end | `crates/core/src/tools/archive.rs:636-642` |

Test command: `cargo test -p mcp-fs-core tools::archive`

### 14.2 Class sweep

Not applicable (`--security` not set).

### 14.3 Design inputs

- Decoding must be a bounded stream: read in chunks and stop when the output exceeds the lower of two caps: the project's remaining storage plus the size of the file replaced under `overwrite` (only when the project has a limit, `crates/core/src/safety.rs:72-81`), and the remaining session write allowance (`crates/core/src/safety.rs:136-147`). The refusal code follows the cap met first, a tie going to the project (FR-NEW-006, FR-NEW-012). The existing admission (`quota::admit_paths`) takes declared sizes; here it is called with the real decoded size after a capped decode. `GzDecoder` reads one member; a multi member decoder is needed (FR-NEW-007). Signal: E2E-005, E2E-017.
- Trailing non gzip bytes (FR-NEW-008) are not detected by the multi member decoder by default; the design must check them. Signal: E2E-010.
- The header name, comment and extra fields must be skipped, not used (FR-NEW-004). Signal: E2E-018, E2E-042, E2E-043.
- The collision and destination checks run before decoding (FR-NEW-011), a reorder of the current flow (`archive.rs:154-181` decodes first). Signal: E2E-021, E2E-035.
- A folder bearing the output name: today's `exists` probe covers files and folders alike (`archive.rs:172-181`); `overwrite` true bypasses it. Signal: E2E-034.
- The audit line and the answer keys (`destination`, `files_written`, `dirs_created`, `bytes_written`) are shared (`archive.rs:242-275`).
- The memory observable of E2E-017 needs a measurable hook (peak allocation or a capped reader counter); decide at design.
- Update `.agent_docs/tools.md:180-183`, `README.md`, the E2E-NEW-034 test (`archive.rs:806-814`); `TOOL_CONTRACT.txt` changes only if a parameter description changes. BL-0124 records the tar family size cap; BL-0112 and BL-0113 reuse this flow for `.xz` and `.bz2`.

## Erratum (BL-0128)

In the order of refusals, "the destination is a file or sits under a file" (item 7) and "collision" (item 8) never apply to the same call: when the destination is a file, or lies below one, the output path is below that file and cannot exist, so no collision is possible. Their relative order is therefore not observable and is not required; the two items are one step for ordering purposes. its destination-is-a-file test pins the outcome (the destination refusal, never `ERR_NO_CLOBBER`), not a sequence. Nothing else in the list changes.
