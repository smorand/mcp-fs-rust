# Bounded decoding of the tar family on one xz decoder: functional specification

> Id: SPEC-0084
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction), 13 requirements, public behavior change (a hostile compressed tar archive is refused with a limit error instead of being decoded whole; a `.tar.xz` the extraction refused now extracts), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-09
> Project: existing
> From backlog: BL-0124, BL-0132
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a
>
> Note: written non interactively. The user authorized every decision and gave the instruction "spec all of them, design later", which is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who extracts a compressed tar archive (`.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz`, `.txz`) gets the protection the bare compressed files already have: decoding stops as soon as the decompressed archive grows past the lower of the project's remaining storage and the member's remaining session write allowance, plus a fixed allowance for the tar bookkeeping, and the call is refused with the limit error. Today the whole archive is decompressed in memory before any limit is checked, so a small hostile archive can exhaust the server's memory. In the same increment, a `.tar.xz` archive is decoded exactly like a bare `.xz` file: several xz streams, zero padding, branch filters and a SHA-256 check are accepted, where today they are refused as corrupt. Everything else about tar extraction stays as it is.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member can extract `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz` archives, and bare `.gz`, `.bz2` and `.xz` files, from the assistant tool and from the HTTP API. The format is read from the file name alone, ignoring letter case. `.tbz2` and `.tbz` are not recognized and are refused with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '.tbz2'`. [14.1 row 1]
- A compressed tar archive is decompressed whole, in memory, before anything is checked; only then are its entries read and checked, then the destination, then collisions, then the two limits. No limit stops the decompression. [row 2]
- A plain `.tar` is not decompressed: what is read is the stored file itself, already counted in the project's storage. [row 3]
- The two limits are checked on the sizes the tar entries declare, after decompression: the project's storage limit (optional, net growth, `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`) and the member's session write allowance (always set, 50 MiB by default, gross size, `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of <N> bytes exceeded`, `<N>` being the configured allowance). The project limit is checked first; equality is accepted; a refusal charges nothing and writes nothing. [row 4]
- An entry never yields more bytes than its header declares; a header declaring more bytes than the archive holds makes the call fail as corrupt. Bytes after the tar end marker are ignored. [row 5]
- A `.tar.xz` or `.txz` is decoded as exactly one xz stream: a second stream, zero padding after the stream, a branch conversion filter (such as the x86 filter) or a SHA-256 integrity check make it fail as corrupt, with a message starting `archive is corrupt or not a valid tar.xz file:`. [row 6]
- A `.tar.gz` and a `.tar.bz2` are decoded as one gzip member and one bzip2 stream. [row 7]
- A bare `.gz`, `.bz2` or `.xz` file is already bounded: decoding stops at the lower of the two limits and the refusal names the limit met first, a tie going to the project (SPEC-0081, SPEC-0082, SPEC-0083). The bare `.xz` decoder accepts several streams, zero padding of a multiple of 4 bytes, branch filters and every integrity check type, and refuses an unknown check type and the legacy `.lzma` format as corrupt. [row 8]
- On success the answer gives the destination, files written, folders created and bytes written, and one audit line is recorded; a refused call records no audit line. [row 9]

### 2.2 Specs governing this area

- SPEC-0018 (archive extract in place): FR-NEW-012 decodes every entry fully into memory before the checks; FR-NEW-019 checks the session allowance after the collision check. This increment deviates for compressed tar archives (FR-MOD-001). SPEC-0018 is not modified.
- SPEC-0020 (project storage limit) defines the project refusal; unchanged.
- SPEC-0082 FR-NEW-010 keeps `.tar.xz`/`.txz` "exactly as before"; this increment changes how they decode (FR-MOD-002). SPEC-0082 is not modified.
- SPEC-0081, SPEC-0082 and SPEC-0083 define the bounded decoding of bare files; this increment brings the same rule to the tar family, restated here so it reads alone. [row 10]

### 2.3 What is verified today

- Tar family extraction is tested for success, corrupt gzip, unsafe entry paths, symlink and hardlink entries, passwords and suffix detection; `.tar.xz` and `.txz` are tested on success only. No test pins a `.tar.xz` refusal. [row 11]
- No test covers a decompression bomb, a decompressed size limit, several xz streams, padding, a branch filter or a SHA-256 check in a tar archive. [row 11]

## 3. Scope

### 3.1 In scope

- A decoding bound for `.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz` and `.txz`, any letter case, from both doors.
- `.tar.xz` and `.txz` decoded with the rules of a bare `.xz` file.
- The order of refusals where the bound takes its place.

### 3.2 Out of scope, and why

- Plain `.tar`: nothing is decompressed, so the memory used is that of the stored file, already within the project's storage.
- Admitting declared sizes before decoding for zip, 7z, RAR and tar entries: BL-0125, its own increment.
- A destination such as `../../x` clamped to `/x` by zip, 7z and tar: BL-0127.
- Truncated RAR4 and header encrypted RAR5 archives: BL-0131.
- `.tbz2` and `.tbz` suffixes: not recognized today, adding them is a new capability, BL-0133.
- Several gzip members or bzip2 streams inside a `.tar.gz` or `.tar.bz2`: unchanged, no request for it.
- Creating archives: nothing in the product creates them.

### 3.3 Constraints imposed by the context

None.

## 4. Actors

- **Member:** a person with access to a project; extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server, watches its memory and reads the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored archive into project files | compressed tar archive, decompressed archive, decoding bound, entry, refusal |
| Project storage | the files, the storage limit and the session write allowance of a project | remaining storage, session write allowance, net growth, collision, overwrite |

## 5. Usage scenarios

### SC-001: Extract a compressed tar archive within the limits
**Actor:** Member.
**Preconditions:** the project holds `/uploads/pack.tar.gz` (or `.tar.bz2`, `.tar.xz`, `.tgz`, `.tb2`, `.txz`) holding one entry `a.txt` with the 5 bytes `hello`. Nothing exists at `/uploads/pack`.
**Flow:**
1. The member asks to extract the archive → the system decodes it, checks it and writes `/uploads/pack/a.txt`.
2. The system answers destination `/uploads/pack`, 1 file written, 1 folder created, 5 bytes written, and records one audit line.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: the files' sizes exceed the remaining storage or the session allowance, although the decompressed archive fits the decoding bound → the limit refusal of today.
- EXC-001b: an entry header declares more bytes than the archive holds → the corrupt refusal of today.

### SC-002: A decompression bomb is stopped
**Actor:** Member.
**Preconditions:** the project holds `/uploads/bomb.tar.gz` (or another compressed tar suffix), a file of a few kilobytes whose decompressed archive holds one entry `zeros.bin` of 10 000 000 zero bytes. The decoding bound is far lower.
**Flow:**
1. The member asks to extract it → the system stops decoding as soon as the decompressed archive passes the decoding bound.
2. The member sees the refusal of the limit that sets the bound.
**Postconditions:** nothing is written; the remaining storage and the session allowance are unchanged; no audit line; the server never held more decompressed bytes than the decoding bound.
**Exceptions:**
- EXC-002a: a password is given → the password refusal, before anything is decoded.
- EXC-002b: the compressed data is broken before the bound is reached → the corrupt refusal.

### SC-003: Re extract with overwrite near the limit
**Actor:** Member.
**Preconditions:** `/uploads/pack.tar.gz` holds `big.bin`, 3 000 000 bytes; it was already extracted, so `/uploads/pack/big.bin` exists with the same 3 000 000 bytes; the project has 100 000 bytes of remaining storage.
**Flow:**
1. The member extracts it again with `overwrite` true → the files already stored in the destination count toward the decoding bound, so decoding completes; the net growth is 0, the extraction succeeds.
2. With `overwrite` false → the collision refusal of today, not a limit refusal.
**Postconditions:** the remaining storage is still 100 000 bytes.

### SC-004: A `.tar.xz` decodes like a bare `.xz`
**Actor:** Member.
**Preconditions:** `/uploads/pack.tar.xz` holds `a.txt` (`hello`) compressed as several xz streams, or with zero padding, a branch filter or a SHA-256 check.
**Flow:**
1. The member extracts it → `/uploads/pack/a.txt` is written as for any tar.
**Exceptions:**
- EXC-004a: padding not a multiple of 4 bytes, other bytes after a stream, a truncated stream, a damaged checksum, an unknown check type or a legacy `.lzma` file → `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`.

### SC-005: Other formats unchanged
**Actor:** Member.
**Flow:**
1. A plain `.tar`, a `.zip`, a `.7z` and a bare compressed file extract or are refused exactly as before.
2. A `.tbz2` file is still refused with `ERR_NOT_SUPPORTED` and `unsupported archive extension: '.tbz2'`.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: Decoding stops at the bound
> IF the decompressed archive of a `.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz` or `.txz` file, in any letter case, grows past the decoding bound THEN THE archive extraction SHALL stop decoding at that point and refuse the call as FR-NEW-003 states.
- **Business rules:** the decompressed archive is every byte the decompression produces: entry headers, entry contents, padding, the end marker and anything after it. The sizes the entries declare play no part in the bound. A decompressed archive equal to the bound is accepted. The server never holds more decompressed bytes than the bound.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-U]: The decoding bound
> THE archive extraction SHALL set, before decoding a compressed tar archive, a decoding bound equal to the lower of the project side and the session side, plus a framing allowance of 1 048 576 bytes.
- **Business rules:** the project side is the remaining storage plus the total size of the files currently stored in the destination folder and below it, whether `overwrite` is true or false; a project without a storage limit has no project side, and the bound is then the session side plus the allowance. The session side is the member's remaining session write allowance, which is always set. When the destination is refused (FR-NEW-005 item 7), the stored files under it count for nothing. Files stored elsewhere in the project count for nothing. The framing allowance covers the tar bookkeeping (headers, padding, end marker), so an archive whose files fit the limits is not refused by the bound because of it.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Refusal at the bound
> IF decoding stops at the decoding bound THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and the message `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested` when the project side is lower than or equal to the session side, and otherwise with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`.
- **Business rules:** a tie between the two sides goes to the project refusal. `<L>` is the project's storage limit in whole megabytes, `<U>` the bytes the project stores at the time of the refusal, `<R>` the decoding bound plus 1; `<N>` is the configured session write allowance.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-UB]: A refusal at the bound leaves no trace
> THE archive extraction SHALL NOT write any file, create any folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused at the decoding bound.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Order of refusals for a compressed tar archive
> IF several refusals apply to the extraction of a compressed tar archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) a password is given (`ERR_INVALID_ARGUMENT`, `password is not applicable to this archive format`); (5) corrupt compressed data (`ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid <format> file:`) or the decoding bound (FR-NEW-003), whichever decoding meets first; (6) the entry checks of today, in archive order (an unsupported entry type, an entry path leaving the destination, an unreadable or truncated entry); (7) the destination refusal of today; (8) collision (`ERR_NO_CLOBBER`, `'<path>' already exists; pass overwrite=true to replace it`); (9) the project storage limit, then the session write allowance, on the files' sizes.
- **Business rules:** items 1 to 4 and 6 to 9 and their messages are unchanged; only the bound in item 5 is new. `<format>` is `tar.gz` for `.tar.gz` and `.tgz`, `tar.bz2` for `.tar.bz2` and `.tb2`, `tar.xz` for `.tar.xz` and `.txz`.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-006 [EARS-E]: Several xz streams in a `.tar.xz`
> WHEN a `.tar.xz` or `.txz` file holds several xz streams back to back THE archive extraction SHALL decode them all and read their concatenation as the one tar archive.
- **Business rules:** an empty stream contributes nothing.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Zero padding in a `.tar.xz`
> IF zero bytes follow an xz stream of a `.tar.xz` or `.txz` file, between streams or at the end, AND their count is a multiple of 4 THEN THE archive extraction SHALL ignore them.
- **Business rules:** a count that is not a multiple of 4 is refused as FR-NEW-009 states.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-008 [EARS-E]: Filters and integrity checks in a `.tar.xz`
> WHEN a `.tar.xz` or `.txz` file uses the delta filter, a branch conversion filter that xz defines (x86, PowerPC, IA-64, ARM, ARM-Thumb, SPARC, ARM64, RISC-V) or any integrity check type xz defines (none, CRC32, CRC64, SHA-256) THE archive extraction SHALL decode it.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Corrupt `.tar.xz`
> IF a `.tar.xz` or `.txz` file is empty, is truncated, is not xz, is in the legacy `.lzma` format, has a damaged checksum or index, declares an integrity check type xz does not define, uses a filter the decoder does not know, or has bytes after a stream that FR-NEW-007 does not accept as padding, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid tar.xz file:`, and write nothing.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-O]: Declared size larger than the content
> IF an entry header of a compressed tar archive declares more bytes than the decompressed archive holds after it THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid <format> file:`, and write nothing.
- **Business rules:** unchanged behavior, pinned here because the bound counts decompressed bytes and never declared sizes; an entry never yields more bytes than its header declares.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-011 [EARS-U]: The rest is unchanged
> THE archive extraction SHALL extract plain `.tar`, `.zip`, `.7z`, RAR, bare `.gz`, `.bz2` and `.xz` files exactly as before this increment, SHALL answer a compressed tar archive within the limits with the same destination, counts and audit line as before, SHALL check the two limits on the files' sizes as before, and SHALL keep refusing `.tbz2` and every other unsupported extension with `ERR_NOT_SUPPORTED` and `unsupported archive extension: '<extension as typed>'`.
- **Scenario:** SC-001, SC-005
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: A compressed tar archive is decoded under a bound (modifies SPEC-0018/FR-NEW-012 and the order of SPEC-0018/FR-NEW-019)
> IF a compressed tar archive decompresses past the decoding bound THEN THE archive extraction SHALL refuse it at that point, before its entries are checked, before its collisions are checked and before the limits are checked on the files' sizes.
- **Before:** the archive is decompressed whole in memory, then entries, destination, collisions and the limits are checked (Section 2.1). A bomb is held whole; a bomb that also collides is refused with `ERR_NO_CLOBBER`, one that also holds a symlink with `ERR_NOT_SUPPORTED`; an archive with megabytes of bytes after its end marker succeeds.
- **After:** the statement above; those three cases are refused at the bound.
- **Reason:** BL-0124: a small archive can exhaust the server's memory.

#### FR-MOD-002 [EARS-U]: A `.tar.xz` decodes like a bare `.xz` (modifies SPEC-0082/FR-NEW-010)
> THE archive extraction SHALL decode a `.tar.xz` or `.txz` file with the rules of a bare `.xz` file (FR-NEW-006 to FR-NEW-009).
- **Before:** one stream only; padding, a second stream, a branch filter or a SHA-256 check are refused as corrupt (Section 2.1).
- **After:** the statement above.
- **Reason:** BL-0132: one xz rule for the product.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN a compressed tar archive is refused at the decoding bound THE archive extraction SHALL have held at most the decoding bound in decompressed bytes.

### 7.2 Security and access
- THE archive extraction SHALL require the same project membership as before.

### 7.5 Reliability
- IF an extraction is refused THEN THE archive extraction SHALL leave the project files, the remaining storage and the session write allowance unchanged.

### 7.6 Operability
- WHEN an extraction succeeds THE archive extraction SHALL record one audit line, as before; a refused one records none.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | empty xz stream between two; 0 byte `.tar.xz` | ignored; corrupt | E2E-032, E2E-037 |
| Boundary values | decompressed archive equal to the bound, one byte over; files equal to the remaining storage; padding 4 and 3; tie between sides | accepted; refused; accepted; accepted and refused; project refusal | E2E-014, E2E-015, E2E-004, E2E-030, E2E-033, E2E-012 |
| Concurrent actions | not applicable: the bound is read once per call, the admission keeps its own lock | none | none |
| Slow or lost connectivity | not applicable: the archive is already stored | none | none |
| Access states | not a member | unchanged refusal, item 1 of FR-NEW-005 | E2E-018 (order), existing tests |
| Data variations | upper case suffixes; `.tgz`, `.tb2`, `.txz` | detected | E2E-002, E2E-003, E2E-011, E2E-029 |
| Navigation | not applicable | none | none |
| Partial operations | truncated stream; damaged byte; header declaring more than the content | corrupt refusal | E2E-037, E2E-038, E2E-043 to E2E-045 |
| Dependency failures | not applicable: no external service | none | none |
| Ordering and timing | bound versus password, corruption, collision, entry type | FR-NEW-005 | E2E-016 to E2E-020, E2E-023 |
| Exhaustion | bomb against each limit; bytes after the end marker | bound refusal | E2E-008 to E2E-011, E2E-046 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are built by the design. Unless a test says otherwise, the session write allowance is large enough not to matter and the project has no storage limit; a figure for "remaining storage" sets a project storage limit (whole megabytes) and fills the project with other files, outside `/uploads`, up to that remainder; a figure for "session allowance" configures that allowance and nothing has been written in the session yet. "The decompressed bytes held" is the largest amount of decompressed archive the server holds during the call, read by the operator's measurement hook.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-011 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-011 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-011, FR-MOD-002 | High |
| E2E-004 | New | edge | SC-001 | FR-NEW-002, FR-NEW-011 | High |
| E2E-005 | New | failure | SC-001 | FR-NEW-011 | High |
| E2E-006 | New | failure | SC-001 | FR-NEW-011 | High |
| E2E-007 | New | side effect | SC-001 | FR-NEW-011 | Medium |
| E2E-008 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-MOD-001 | Critical |
| E2E-009 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-003, FR-NEW-004 | Critical |
| E2E-010 | New | failure, security | SC-002 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-011 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-003 | Critical |
| E2E-012 | New | edge | SC-002 | FR-NEW-002, FR-NEW-003 | High |
| E2E-013 | New | edge | SC-002 | FR-NEW-002, FR-NEW-003 | High |
| E2E-014 | New | edge | SC-002 | FR-NEW-001, FR-NEW-002 | High |
| E2E-015 | New | failure | SC-002 | FR-NEW-001, FR-NEW-002 | High |
| E2E-016 | New | failure | SC-002 | FR-NEW-005, FR-NEW-002, FR-MOD-001 | High |
| E2E-017 | New | failure | SC-002 | FR-NEW-005, FR-MOD-001 | High |
| E2E-018 | New | failure | SC-002 | FR-NEW-005 | Medium |
| E2E-019 | New | failure | SC-002 | FR-NEW-005 | Medium |
| E2E-020 | New | failure | SC-002 | FR-NEW-005 | Medium |
| E2E-021 | New | side effect | SC-002 | FR-NEW-004 | High |
| E2E-022 | New | happy | SC-003 | FR-NEW-002 | Critical |
| E2E-023 | New | failure | SC-003 | FR-NEW-005, FR-NEW-002 | High |
| E2E-024 | New | edge | SC-003 | FR-NEW-002 | Medium |
| E2E-025 | New | happy | SC-003 | FR-NEW-002 | Medium |
| E2E-026 | New | happy | SC-004 | FR-NEW-006, FR-MOD-002 | High |
| E2E-027 | New | happy | SC-004 | FR-NEW-008, FR-MOD-002 | High |
| E2E-028 | New | happy | SC-004 | FR-NEW-008, FR-MOD-002 | High |
| E2E-029 | New | happy | SC-004 | FR-NEW-006, FR-NEW-008 | Medium |
| E2E-030 | New | edge | SC-004 | FR-NEW-007 | Medium |
| E2E-031 | New | edge | SC-004 | FR-NEW-007 | Medium |
| E2E-032 | New | edge | SC-004 | FR-NEW-006 | Medium |
| E2E-033 | New | failure | SC-004 | FR-NEW-007, FR-NEW-009 | Medium |
| E2E-034 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-035 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-036 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-037 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-038 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-039 | New | happy | SC-005 | FR-NEW-011 | High |
| E2E-040 | New | edge | SC-005 | FR-NEW-011 | Medium |
| E2E-041 | New | failure | SC-005 | FR-NEW-011 | Medium |
| E2E-042 | New | failure | SC-005 | FR-NEW-011 | Medium |
| E2E-043 | New | failure | SC-001 | FR-NEW-010 | High |
| E2E-044 | New | failure | SC-001 | FR-NEW-010 | Medium |
| E2E-045 | New | failure | SC-001 | FR-NEW-010 | Medium |
| E2E-046 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |

Coverage (46 tests, all new, 0 modified, 0 removed): happy 10, failure 25 (4 of them also security: E2E-008 to E2E-011), side effect 2, edge 9, state transition 0 (no state machine). Happy to failure ratio 1:2.5.

### 9.2 New tests

#### E2E-001: `.tar.gz` within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** a member and `/uploads/pack.tar.gz` holding `a.txt` with the 5 bytes `hello`; nothing at `/uploads/pack`
- **When** the member extracts `/uploads/pack.tar.gz`
- **Then** the answer is destination `/uploads/pack`, 1 file written, 1 folder created, 5 bytes written
- **And** `/uploads/pack/a.txt` reads `hello`

#### E2E-002: `.tar.bz2` and `.TB2` within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/uploads/pack.tar.bz2` and `/u2/PACK.TB2`, each holding `a.txt` (`hello`)
- **When** the member extracts each
- **Then** `/uploads/pack/a.txt` and `/u2/PACK/a.txt` read `hello`, each answer reporting 1 file and 5 bytes written

#### E2E-003: `.tar.xz` and `.txz` within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` and `/u3/pack.txz`, each a single xz stream with a CRC64 check holding a tar with `a.txt` (`hello`)
- **When** the member extracts each
- **Then** `/uploads/pack/a.txt` and `/u3/pack/a.txt` read `hello`, each answer reporting destination the archive path without its suffix, 1 file, 1 folder, 5 bytes

#### E2E-004: Files equal to the remaining storage
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-011; **Priority:** High
- **Given** 5 bytes of remaining storage and `/uploads/pack.tar.gz` holding `a.txt` (`hello`), whose decompressed archive is larger than 5 bytes because of the tar bookkeeping
- **When** the member extracts it
- **Then** the extraction succeeds with 5 bytes written
- **And** the remaining storage is 0 bytes

#### E2E-005: Files over the remaining storage, archive within the bound
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** 4 bytes of remaining storage and `/uploads/pack.tar.gz` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/pack` does not exist and the remaining storage is still 4 bytes

#### E2E-006: Files over the session allowance, archive within the bound
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** a session allowance of 4 bytes and `/uploads/pack.tar.xz` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 4 bytes exceeded`
- **And** `/uploads/pack` does not exist

#### E2E-007: Audit line and HTTP API unchanged
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** the situation of E2E-003 for `/uploads/pack.tar.xz`
- **When** a client asks the HTTP API to extract it
- **Then** the answer is that of E2E-003
- **And** the audit trail has exactly one extraction line for `/uploads/pack.tar.xz` naming destination `/uploads/pack`, 1 file written and 5 bytes written

#### E2E-008: `.tar.gz` bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz`, a file of at most 20 000 bytes whose decompressed archive holds `zeros.bin`, 10 000 000 zero bytes, declared as such
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes

#### E2E-009: `.tar.bz2` bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Given** a project without a storage limit, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.bz2` holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; a following write of a 1 000 byte file `/uploads/small.bin` succeeds

#### E2E-010: `.tar.xz` bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.xz`, a single xz stream holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes

#### E2E-011: `.TXZ` bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** Critical
- **Given** no storage limit, a session allowance of 1 000 bytes and `/uploads/BOMB.TXZ` holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/BOMB` does not exist

#### E2E-012: Tie between the two sides
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 1 000 bytes of remaining storage, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged

#### E2E-013: The session side is lower
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 2 000 000 bytes of remaining storage, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`; the remaining storage is still 2 000 000 bytes

#### E2E-014: Decompressed archive exactly at the bound
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage, nothing stored under `/uploads/fit`, and `/uploads/fit.tar.gz` whose decompressed archive is exactly 1 049 576 bytes: `a.txt` (`hello`), the end marker, then zero bytes
- **When** the member extracts it
- **Then** the extraction succeeds: 1 file written, 5 bytes written; the remaining storage is 995 bytes

#### E2E-015: One byte over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** the situation of E2E-014 with `/uploads/fit.tar.gz` replaced by one whose decompressed archive is 1 049 577 bytes (one more zero byte)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`; `/uploads/fit` does not exist; the remaining storage is still 1 000 bytes

#### E2E-016: The bound wins over a collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-002, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.tar.gz` of E2E-008, and an existing `/uploads/bomb/zeros.bin` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_NO_CLOBBER`
- **And** `/uploads/bomb/zeros.bin` still reads `old`

#### E2E-017: The bound wins over an unsupported entry
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` whose decompressed archive holds a symbolic link `link` pointing to `/etc/passwd`, then `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_NOT_SUPPORTED`; `/uploads/bomb` does not exist

#### E2E-018: Password wins over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`
- **And** the decompressed bytes held are 0

#### E2E-019: Corruption met first wins over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008 with its first 2 bytes (the gzip signature) set to zero
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.gz file:`

#### E2E-020: The bound met first wins over a later corruption
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008 with its last 8 bytes (the gzip trailer) inverted
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not the corrupt refusal

#### E2E-021: A refusal at the bound is not audited
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the situation of E2E-008
- **When** the member extracts `/uploads/bomb.tar.gz` and is refused
- **Then** the audit trail has no extraction line for `/uploads/bomb.tar.gz`
- **And** extracting `/uploads/pack.tar.gz` of E2E-001 right after succeeds and leaves 995 bytes of remaining storage

#### E2E-022: Re extract with overwrite near the limit
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Critical
- **Given** 100 000 bytes of remaining storage, `/uploads/pack.tar.gz` holding `big.bin` (3 000 000 bytes of the text `abc` repeated), and `/uploads/pack/big.bin` already holding the same 3 000 000 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds with 1 file and 3 000 000 bytes written
- **And** the remaining storage is still 100 000 bytes

#### E2E-023: Same situation without overwrite
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-002; **Priority:** High
- **Given** the situation of E2E-022
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/pack/big.bin' already exists; pass overwrite=true to replace it`, not a limit refusal

#### E2E-024: Files stored elsewhere do not count
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-022, the same 3 000 000 bytes stored at `/elsewhere/big.bin`, nothing under `/uploads/pack`, and 100 000 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`; `/uploads/pack` does not exist

#### E2E-025: Explicit destination holding the files
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-022, `/out/big.bin` holding the same 3 000 000 bytes, nothing under `/uploads/pack`, and 100 000 bytes of remaining storage
- **When** the member extracts it to the destination `/out` with `overwrite` true
- **Then** the extraction succeeds; `/out/big.bin` holds the 3 000 000 bytes; the remaining storage is still 100 000 bytes

#### E2E-026: Two xz streams
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` made of two xz streams, the first holding the first 512 bytes of a tar with `a.txt` (`hello`), the second holding the rest
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello` and the answer reports 1 file, 5 bytes

#### E2E-027: x86 branch filter
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` compressed with the x86 branch filter before the main compression, holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-028: SHA-256 integrity check
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` made with a SHA-256 integrity check, holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-029: `.txz` of two streams with a CRC32 check
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/pack.txz` made of two streams with a CRC32 check, together holding a tar with `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-030: 4 zero bytes after the stream
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by 4 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-031: 4 zero bytes between two streams
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-026 with 4 zero bytes between its two streams
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-032: Empty stream between two streams
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-026 with an empty xz stream inserted between its two streams
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`

#### E2E-033: 3 zero bytes after the stream
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by 3 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`; `/uploads/pack` does not exist

#### E2E-034: Other bytes after the stream
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-033

#### E2E-035: Legacy `.lzma` file named `.tar.xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` holding the tar of E2E-003 compressed in the legacy `.lzma` format
- **When** the member extracts it
- **Then** the refusal is as E2E-033

#### E2E-036: Integrity check type xz does not define
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 whose stream header and footer declare the reserved check type 3, with valid header and footer checksums
- **When** the member extracts it
- **Then** the refusal is as E2E-033

#### E2E-037: Truncated and empty files
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/cut.tar.xz`, the first half of `/uploads/pack.tar.xz` of E2E-003, and `/uploads/zero.tar.xz`, a 0 byte file
- **When** the member extracts each
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`; neither `/uploads/cut` nor `/uploads/zero` exists

#### E2E-038: Damaged compressed byte
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/pack.tar.xz` of E2E-003 with one byte of its compressed data changed
- **When** the member extracts it
- **Then** the refusal is as E2E-033

#### E2E-039: Plain `.tar` unchanged
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/uploads/pack.tar` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the answer is destination `/uploads/pack`, 1 file, 1 folder, 5 bytes

#### E2E-040: Plain `.tar` files equal to the remaining storage
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** 5 bytes of remaining storage and `/uploads/pack.tar` of E2E-039
- **When** the member extracts it
- **Then** the extraction succeeds and the remaining storage is 0 bytes

#### E2E-041: `.zip` over the remaining storage unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** 4 bytes of remaining storage and `/uploads/pack.zip` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`; `/uploads/pack` does not exist

#### E2E-042: `.tbz2` still unsupported
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/pack.tbz2`, a bzip2 compressed tar holding `a.txt`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.tbz2'`

#### E2E-043: `.tar.gz` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/lie.tar.gz` whose only entry `a.txt` declares 1 000 000 000 bytes and is followed by the 5 bytes `hello`, then the end of the archive
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.gz file:`, not a limit refusal
- **And** `/uploads/lie` does not exist; the remaining storage is still 1 000 bytes

#### E2E-044: `.tar.bz2` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/lie.tar.bz2` built as in E2E-043
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.bz2 file:`

#### E2E-045: `.tar.xz` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/lie.tar.xz` built as in E2E-043
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`

#### E2E-046: Bytes after the end marker count
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/tail.tar.gz` whose decompressed archive holds `a.txt` (`hello`), the end marker, then 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/tail` does not exist

### 9.3 Exploit tests

Not applicable (`--security` not set, Section 14.4); E2E-008 to E2E-011 and E2E-046 are the bomb tests.

### 9.4 Modified tests

None. No existing test pins a `.tar.xz` refusal; the existing tar tests (success, corrupt gzip, unsafe paths, links, passwords, suffixes) keep their assertions and stay green (Section 14.1 row 11).

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-002, FR-NEW-010, FR-NEW-011, FR-MOD-002 | E2E-001, E2E-002, E2E-003 | E2E-005, E2E-006, E2E-043, E2E-044, E2E-045 | E2E-004 | E2E-007 |
| SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-MOD-001 | E2E-014 (success branch, categorized edge) | E2E-008, E2E-009, E2E-010, E2E-011, E2E-015, E2E-016, E2E-017, E2E-018, E2E-019, E2E-020, E2E-046 | E2E-012, E2E-013, E2E-014 | E2E-021 |
| SC-003 | FR-NEW-002, FR-NEW-005 | E2E-022, E2E-025 | E2E-023 | E2E-024 | none |
| SC-004 | FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-MOD-002 | E2E-026, E2E-027, E2E-028, E2E-029 | E2E-033, E2E-034, E2E-035, E2E-036, E2E-037, E2E-038 | E2E-030, E2E-031, E2E-032 | none |
| SC-005 | FR-NEW-011 | E2E-039 | E2E-041, E2E-042 | E2E-040 | none |

SC-002 is a refusal scenario; its success branch is E2E-014.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-008, E2E-009, E2E-010, E2E-011, E2E-014, E2E-015, E2E-046 | 7 |
| FR-NEW-002 | E2E-004, E2E-012, E2E-013, E2E-014, E2E-015, E2E-016, E2E-022, E2E-023, E2E-024, E2E-025 | 10 |
| FR-NEW-003 | E2E-008, E2E-009, E2E-011, E2E-012, E2E-013 | 5 |
| FR-NEW-004 | E2E-008, E2E-009, E2E-021 | 3 |
| FR-NEW-005 | E2E-016, E2E-017, E2E-018, E2E-019, E2E-020, E2E-023 | 6 |
| FR-NEW-006 | E2E-026, E2E-029, E2E-032 | 3 |
| FR-NEW-007 | E2E-030, E2E-031, E2E-033 | 3 |
| FR-NEW-008 | E2E-027, E2E-028, E2E-029 | 3 |
| FR-NEW-009 | E2E-033, E2E-034, E2E-035, E2E-036, E2E-037, E2E-038 | 6 |
| FR-NEW-010 | E2E-043, E2E-044, E2E-045 | 3 |
| FR-NEW-011 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-006, E2E-007, E2E-039, E2E-040, E2E-041, E2E-042 | 11 |
| FR-MOD-001 | E2E-008, E2E-010, E2E-016, E2E-017, E2E-046 | 5 |
| FR-MOD-002 | E2E-003, E2E-026, E2E-027, E2E-028 | 4 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Compressed tar archive | a file named `.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz` or `.txz`: a tar archive inside one compression | archive extraction |
| Decompressed archive | every byte the decompression of a compressed tar archive produces, bookkeeping included | archive extraction |
| Tar bookkeeping | the entry headers, the padding after each entry and the end marker of a tar archive | archive extraction |
| End marker | the zero blocks that close a tar archive; bytes after it are not entries | archive extraction |
| Decoding bound | the most decompressed bytes an extraction is allowed to produce: the lower of the project side and the session side, plus the framing allowance | archive extraction |
| Project side | the remaining storage plus the files stored in the destination folder and below it; absent without a storage limit | project storage |
| Session side | the member's remaining session write allowance | project storage |
| Framing allowance | the fixed 1 048 576 bytes added to the bound for the tar bookkeeping | archive extraction |
| Decompressed bytes held | the largest amount of decompressed archive the server holds during one call | archive extraction |
| Decompression bomb | a small compressed file that expands to a huge output | archive extraction |
| Entry | one file or folder inside an archive | archive extraction |
| Declared size | the size an entry header announces for its content | archive extraction |
| xz stream | one complete compressed unit in an xz file; a file can hold several | archive extraction |
| Zero padding | zero bytes between or after xz streams, accepted when their count is a multiple of 4 | archive extraction |
| Branch conversion filter | an xz filter that improves the compression of executable code (x86, ARM and others) | archive extraction |
| Integrity check | the checksum type an xz stream declares: none, CRC32, CRC64 or SHA-256 | archive extraction |
| Legacy `.lzma` format | the format that preceded xz; not xz | archive extraction |
| Storage limit | the optional cap on the bytes a project stores, set in whole megabytes | project storage |
| Remaining storage | the storage limit minus the bytes stored | project storage |
| Session write allowance | the bytes a member can still write in the current session; always set, 50 MiB by default | project storage |
| Net growth | the files' sizes minus the sizes of the files they replace | project storage |
| Destination | the folder that receives the entries; by default the archive path without its suffix | archive extraction |
| Collision | an entry's target path already holds a file | project storage |
| Overwrite | the choice to replace colliding files instead of refusing | project storage |
| Refusal | an error answer; the project is left unchanged | archive extraction |
| Audit line | the one line an operator reads to see who extracted what and where | archive extraction |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server, watches its memory and reads its audit trail | archive extraction |
| HTTP API | the web endpoint a program calls to extract an archive; the second door | archive extraction |
| Assistant tool | the tool an AI assistant calls to extract an archive | archive extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 | 3 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied after round 1:** F-1 (G1) the project refusal's figures were unspecified at the bound: FR-NEW-003 now spells the whole message and `<R>`; A-1 and A-2 test and call site citations recounted (14.1 rows 9 and 11); A-3 the filter list leaned on an unverified decoder version: citation added (14.1 row 8) and the delta filter named in FR-NEW-008. Consistency fixes: refusal list renumbered without an empty item, one forbidden modal removed from the glossary.
**How the gates ran:** this harness exposes no subagent tool, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3) were run inline by the author against the file on disk, with scripted checks for ids, traceability counts, modals and implementation terms. Recorded as a concern in the handoff.
**No implementation detail outside Section 14:** option names (`destination`, `overwrite`, password), error codes, verbatim messages and file name suffixes are WHAT.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | suffix table, case insensitive, `.tbz2`/`.tbz` absent, unsupported message; two doors | `crates/core/src/tools/archive.rs:42-63`, `:69-81`; `crates/core/src/api/dataplane.rs` extract route calls `tools::archive::extract_archive` |
| 2 | compressed tar decompressed whole with no cap, then parsed; then destination, collision, admission | `crates/core/src/tools/archive.rs:203-211` (read whole, decode in `spawn_blocking`), `:727-749` (`read_to_end` per codec), `:751-791` (parse), `:228-232` (destination), `:236-245` (collision), `:248-262` (admission) |
| 3 | plain `.tar` is the stored bytes | `crates/core/src/tools/archive.rs:728` (`bytes.to_vec()`), `:206` |
| 4 | the two limits, declared sizes, project first, equality accepted, nothing charged on refusal; default allowance 50 MiB; messages | `crates/core/src/tools/archive.rs:248-262`, `crates/core/src/quota.rs:136-159`, `crates/core/src/safety.rs:136-147`, `crates/core/src/config.rs:360-362`, `crates/core/src/errors.rs:103-111` |
| 5 | an entry yields at most its declared size; a short entry fails at the next header (`unexpected EOF during skip`) mapped to corrupt; parsing stops at the first zero block, so bytes after the end marker are ignored (`ignore_zeros` never set) | `crates/core/src/tools/archive.rs:779-781`, `tar-0.4.46/src/archive.rs:293-311`, `:555-567` (`Cargo.lock:5849-5850`); `rg -n ignore_zeros crates/core/src` finds nothing |
| 6 | `.tar.xz` through lzma-rs 0.3: one stream, trailing data refused, unknown filter id refused (only LZMA2), SHA-256 refused | `crates/core/src/tools/archive.rs:743-748`, `lzma-rs-0.3.0/src/decode/xz.rs:86-90`, `:178-182`, `:326-329` (`Cargo.toml:87`) |
| 7 | `.tar.gz` and `.tar.bz2` decoded by single member/stream decoders | `crates/core/src/tools/archive.rs:731`, `:738`; `flate2-1.1.10/src/gz/read.rs:106` (`GzDecoder` points to `MultiGzDecoder` for several members) |
| 8 | bare files bounded; caps computed before decoding, tie to the project; project refusal figures (`requested` = cap plus 1); xz decoder with `CONCATENATED`, `TELL_UNSUPPORTED_CHECK`, explicit stream decoder (no legacy `.lzma`); an unsupported check is an error; bundled xz 5.8.4 knows ARM64 and RISC-V | `crates/core/src/tools/archive_single.rs:204-221`, `:180-199`, `:92-108`, `:67-86`, `:240-270`; `liblzma-0.4.9/src/stream.rs:1320`; `liblzma-sys-0.4.9/xz/src/liblzma/api/lzma/version.h:19-25`, `liblzma-sys-0.4.9/src/manual.rs:89-91` (`Cargo.lock:3584-3585`) |
| 9 | answer and audit only on success | `crates/core/src/tools/archive.rs:313-352`, called at `:297-306` only |
| 10 | SPEC-0018 FR-NEW-012 and FR-NEW-019; SPEC-0082 FR-NEW-010 | `sdlc/specs/SPEC-0018-archive-extract-in-place/spec.md:203`, `:210`; `sdlc/specs/SPEC-0082-extract-xz-single-file/sdlc-spec.md` FR-NEW-010 |
| 11 | existing tar tests; `.tar.xz` only on success; no tar bomb, multi stream, padding, filter or check test | `crates/core/src/tools/archive.rs:1001-1041` (passwords), `:1045-1070` (suffixes), `:1204` (corrupt gzip), `:1382-1507` (paths and links), `:1949`, `:1968-1996` (tar.gz and all formats succeed, `.txz` via lzma-rs), `crates/core/src/tools/archive_single.rs:2106-2145` (`.tar.xz`, `.txz`, `.TAR.XZ` succeed, E2E-005/006/047 of SPEC-0082); `rg -n 'tar\.xz\|txz' crates/ --glob '*.rs'` recounted, no refusal assertion |

Test command: `cargo test -p mcp-fs-core --lib tools::archive` (includes `tools::archive_single`); full gate `cargo test --workspace`.

### 14.2 Class sweep

Not applicable (`--security` not set, Section 14.4 default D-3). Sister occurrences of the same class (zip, 7z, RAR decoded fully before admission) are tracked by BL-0125.

### 14.3 Design inputs

- The tar decoder reads each codec to the end into one buffer (`archive.rs:727-749`). The bound needs a capped read of the decompressed stream before parsing; `archive_single.rs:92-108` (`read_capped`) and `:204-221` (`caps`) already implement the shape for one output file. Signal: E2E-008 to E2E-011, E2E-046.
- The project side adds the bytes stored under the destination subtree (FR-NEW-002); `quota.rs:165-178` (`project_remaining_for`) takes one replaced size, and `quota.rs:222` (`folder_usage`) or a subtree sum gives the rest. The destination is normalized today after decoding (`archive.rs:228-232`); computing the bound needs it before decoding while its refusal keeps its place in the order (FR-NEW-005 item 8). Signal: E2E-016, E2E-022, E2E-024, E2E-025.
- The bound refusal can reuse `Caps::refusal` (`archive_single.rs:180-199`), which already reports `requested` as the cap plus 1; FR-NEW-003 fixes `<R>` as the decoding bound plus 1. Signal: E2E-008 asserts the prefix; a unit check on the figures is the design's choice.
- "Decompressed bytes held" needs a measurable hook like `LAST_HIGH_WATER` (`archive_single.rs:56-61`), which today exists only on the single file path. Signal: E2E-008 to E2E-011, E2E-018, E2E-046.
- After decoding, the parse keeps a second copy of each entry (`archive.rs:779-788`), so peak memory is up to twice the bound; FR-NEW-001 and 7.1 bound the decompressed archive only. Streaming the parse is a design choice, not required.
- The `.tar.xz` decoder becomes the bare `.xz` decoder (`archive_single.rs:78-84`); `lzma-rs` then remains only as a test compressor (`archive.rs:1994`, `archive_single.rs:2120`). Dropping the dependency (`Cargo.toml:87`, `crates/core/Cargo.toml:72`) means rewriting those two fixtures with `liblzma`; their assertions do not change. Decision for `/sdlc-design`.
- The `.tar.gz`/`.tar.bz2` decoders stay single member (Section 3.2); the bound applies to them unchanged otherwise.
- Docs to update: `.agent_docs/tools.md` (archive section), `README.md`, `AGENTS.md` project structure line for `archive.rs`; `TOOL_CONTRACT.txt` only if a parameter description changes (none planned).

### 14.4 Interview answers and defaults applied

Round 1 to Round 5 were answered from the code and the two backlog entries, without the user, under their authorization.

- R1 scope: the tar family under a decoding bound, `.tar.xz` on the bare xz rules; plain `.tar` excluded because it is not decompressed (`archive.rs:728`).
- R1 why now: BL-0124 (memory exhaustion) and BL-0132 (two xz decoders, refused BCJ and SHA-256), deferred from SPEC-0081 and SPEC-0082.
- R2 current state: Section 2 and 14.1.
- R3 scenarios: SC-001 to SC-005; failures from the SPEC-0082 xz refusals and the order of today.
- R4 requirements: derived, Section 6.
- R5 NFR: memory bound, no trace on refusal, audit unchanged.
- D-1 routing: BL-0124 reads `Nature: BUG` and BL-0132 `Nature: DEBT`; the user directed FEAT because observable behavior changes (refusal instead of memory exhaustion, new accepted `.tar.xz` files) and no spec defines a tar decoding bound (SPEC-0018 FR-NEW-012 mandates the full decode). Nature: FEAT.
- D-2 depth M: one module, 13 requirements, public behavior change, no persisted change.
- D-3 security field `n/a`: resource exhaustion is not in the security classes of the spec process (bypass, injection, exposure, deserialization, traversal, advisory upgrade); SPEC-0081 and SPEC-0082 handle the same bomb class as ordinary acceptance tests. No class sweep; BL-0125 carries the sister occurrences.
- D-4 the bound counts every decompressed byte (headers, padding, end marker, bytes after it), never declared sizes, so it is the memory actually used. Rejected alternative: counting only entry contents, which lets an archive of millions of empty entries exhaust memory.
- D-5 framing allowance 1 048 576 bytes added to the bound, so an archive whose files fit is not refused because of its bookkeeping; the files' sizes are still admitted exactly as today. Rejected: no allowance (an archive whose files fit exactly the remaining storage would be refused, changing E2E-004 behavior).
- D-6 the project side adds the files stored under the destination, for `overwrite` true and false, so a re extraction near the limit keeps succeeding and a collision keeps its `ERR_NO_CLOBBER` (E2E-022, E2E-023). Rejected: adding them only with `overwrite` true, which turns today's collision refusal into a limit refusal.
- D-7 the bound refusal precedes entry checks, destination, collision and the limits on files' sizes, as the decompression does today; the bound and corruption are ordered by which decoding meets first.
- D-8 tie between the sides goes to the project refusal, as the bare formats and the admission do.
- D-9 `.tar.gz` and `.tar.bz2` keep single member decoding; `.tbz2` and `.tbz` stay unsupported and become BL-0133.
- D-10 tests: 46 new, 0 modified (no existing test pins a `.tar.xz` refusal); test plan approved by the instruction "spec all of them, design later".
- D-11 lzma-rs removal is a design input (14.3), not a requirement.
- D-12 memory observable: decompressed bytes held at most the bound (1 049 576 bytes for a 1 000 byte side), as SPEC-0082 E2E-009 did for the bare file.
