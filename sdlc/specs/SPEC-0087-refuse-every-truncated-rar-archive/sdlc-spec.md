# Refuse every truncated RAR archive: functional specification

> Id: SPEC-0087
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction, RAR only), 10 requirements, public behavior change (a truncated RAR version 4 archive, or a truncated archive whose file names or data are encrypted, extracted today with fewer files or refused with a misleading message, is refused as corrupt; an archive made of the RAR marker alone, or of the marker and the main header, is refused instead of extracting nothing), no persisted change. Override with --depth.
> Status: designed
> Generated on: 2026-11-09
> Project: existing
> From backlog: BL-0131
> Depends on: SPEC-0080 (built); interplay with SPEC-0085 and SPEC-0086 (specified, not built), stated in 2.2
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a
>
> Note: written non interactively. The user authorized every decision and gave the instruction "spec all of them, design later", which is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who extracts a RAR archive that was cut short, by an interrupted upload, a full disk or a copy that stopped, gets a refusal saying the archive is corrupt, whatever the RAR version and whether its data or its file names are encrypted. Today a RAR version 4 archive cut between two files, and an archive with encrypted file names cut anywhere once the right password is given, extract the files before the cut and report success: the member believes they have the whole archive. Other cuts are reported as a file larger than declared or as a wrong password, which sends the member looking for the wrong problem. The password refusals that come first today keep coming first.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member extracts a RAR archive (version 4 or version 5, any letter case of the `.rar` suffix) from the assistant tool or from the HTTP API, on a server delivered with RAR support; both doors run the same extraction and give the same answers. [14.1 row 1]
- RAR version 5, data and file names readable, or data encrypted with readable file names: any cut, whether inside a header, inside a file's data, between two files or just before the end of archive block, is refused with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`. An archive missing its end of archive block is refused the same way. [row 2]
- RAR version 5 with encrypted file names: the completeness check stops at the encryption header. With no password or a wrong one the password refusal comes first, even when the archive is cut (SPEC-0080 E2E-058). With the right password: a cut between two files, or just before the end of archive block, extracts the files before the cut and reports success; a cut inside a file's data is refused with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`; a cut inside a header is refused as corrupt with a varying detail. [row 3]
- RAR version 4, any kind: no completeness check. [row 4] Measured on archives made by RAR 6.24:
  - a cut exactly between two files, or just before the end of archive block, extracts the files before the cut and reports success; a cut inside a recovery record does the same;
  - the RAR marker alone (7 bytes), or the marker and the main header (20 bytes), extracts nothing and reports success with an empty folder;
  - a cut inside a header is refused as corrupt with a varying detail;
  - a cut inside a plain file's data is refused with `ERR_INVALID_ARGUMENT` and `archive entry '<name>' decompressed to more bytes than declared <size>`;
  - a cut inside an encrypted file's data with the right password is refused with the wrong password message; with no password, with the password needed message;
  - with encrypted file names and no password, the password needed message as soon as 14 bytes are present, even when the main header is incomplete.
- A wrong password on a RAR version 4 archive with encrypted file names is refused as corrupt, cut or whole (BL-0136). [row 5]
- An archive preceded by other bytes (a self extracting archive, or 100 zero bytes) is found and extracted; the version 5 completeness check does not run on it. Bytes after the end of archive block are ignored. [row 6]
- A refused extraction writes nothing, creates no folder, charges neither the remaining storage nor the session write allowance, and records no audit line; the password never appears in a message, a log line or an audit line. [row 7]

### 2.2 Specs governing this area

- SPEC-0080 FR-NEW-011 refuses a truncated RAR archive as corrupt, without limiting it to version 5. The behavior of 2.1 for version 4 and for encrypted file names violates it: FR-MOD-001 restates it with the cases spelled out.
- SPEC-0080 FR-NEW-015 orders the RAR refusals: (4) encrypted file names with a missing or wrong password, including when truncated; (5) corrupt; (6) link or escaping entry; (7) encrypted data with a missing or wrong password; (8) decoded larger than declared; (9) destination; (10) collision; (11, 12) limits. This increment keeps that order and places every truncation at item 5 (FR-NEW-007); it adds when item 4 applies to a cut archive (FR-NEW-004, FR-NEW-005).
- SPEC-0080 E2E-014, E2E-048, E2E-058 and E2E-060 stay valid unchanged. SPEC-0080 erratum 2 (a size and checksum mismatch worded as decoded larger than declared) stays for a whole archive; a cut archive no longer reaches it.
- SPEC-0085 FR-NEW-009 puts "SPEC-0080 FR-NEW-015 items 4 to 6" before its declared limits: a truncation therefore stays before the limits once SPEC-0085 is built (E2E-040 holds before and after).
- SPEC-0086 FR-MOD-002 moves the destination refusal before the archive is read: once built, a destination outside the project wins over a truncation. This increment does not move the destination refusal and has no test pairing the two.
- None of these specs is modified by this document. [row 8]

### 2.3 What is verified today

- Version 5: a cut at every length of a plain archive is refused (structural check); a cut archive with readable names and encrypted data is refused as corrupt with and without a wrong password; a cut archive with encrypted names and a wrong password gets the wrong password refusal. [row 9]
- Version 4: whole archives extract (plain, encrypted data with right and wrong password). No test cuts a version 4 archive. No test cuts an archive with encrypted file names and gives the right password. [row 9]

## 3. Scope

### 3.1 In scope

- Every RAR archive extracted on a server with RAR support, version 4 and version 5, plain, with encrypted data, with encrypted file names, solid, with folders, with a recovery record or a comment, from both doors.
- A cut anywhere after a whole RAR marker refused as corrupt with one exact message.
- An archive whose end of archive block is missing refused, version 4 included.
- An archive preceded by other bytes checked the same way from its RAR marker.
- The password refusals that come before the corrupt refusal for encrypted file names, defined for a cut archive.

### 3.2 Out of scope, and why

- A wrong password on a version 4 archive with encrypted file names, reported as corrupt today: a separate defect with its own trade off (version 4 has no password check value); BL-0136. Its answer is pinned as today's by E2E-027.
- Multi volume archives: refused on the name, unchanged; a first volume renamed without the volume suffix is not a cut archive.
- A whole archive with damaged bytes in the middle (not cut): still refused as corrupt with a varying detail, unchanged.
- zip, 7z, tar and bare compressed files: they have their own truncation rules, unchanged.
- The destination refusal's position: owned by SPEC-0086.

### 3.3 Constraints imposed by the context

- The user asked for this to be specified as a feature even though it is a defect against SPEC-0080 FR-NEW-011 (Section 14.4 D-1).
- No archive made by an external tool is produced at test time; test archives are made once with RAR 6.24 (version 4) and RAR 7.23 (version 5), as the existing ones (Section 14.3).

## 4. Actors

- **Member:** a person with access to a project; extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server and reads its logs and the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored archive into project files | RAR archive, version 4, version 5, marker, block, end of archive block, complete archive, truncated archive, encrypted data, encrypted file names, order of refusals |
| Project storage | the project tree, its storage limit and the session write allowance | remaining storage, session write allowance, audit line |

## 5. Usage scenarios

### SC-001: Extract a complete RAR archive
**Actor:** Member.
**Preconditions:** project `atlas`; `/uploads/two4.rar`, a version 4 archive holding `a.txt` (`hello`) and `b.txt` (`goodbye`), complete; nothing at `/uploads/two4`.
**Flow:**
1. The member extracts it (with the password when it is encrypted) → the system writes `/uploads/two4/a.txt` and `/uploads/two4/b.txt`.
2. The system answers 2 files, 12 bytes, and records one audit line.
**Postconditions:** as today for every complete archive, whatever its version, solid or not, with a recovery record, a comment, folders, bytes before the marker or bytes after the end of archive block.
**Exceptions:**
- EXC-001a: the archive is cut → SC-002 or SC-003.

### SC-002: A truncated RAR archive is refused
**Actor:** Member.
**Preconditions:** `/uploads/two4.rar` holds only the first 67 bytes of the archive of SC-001 (the cut falls between `a.txt` and `b.txt`).
**Flow:**
1. The member extracts it → the system refuses with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`.
**Postconditions:** nothing written, no folder created, remaining storage and session write allowance unchanged, no audit line.
**Exceptions:**
- EXC-002a: fewer than the 7 bytes of a version 4 marker (or 8 of a version 5 marker) → refused with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid rar file:`, as today.
**Interactions:** the member uploads the archive again and extracts it (SC-001).

### SC-003: A truncated encrypted RAR archive
**Actor:** Member.
**Preconditions:** `/uploads/hidden.rar`, a version 5 archive with encrypted file names and the password `s3cret-Ünï`, cut before its end.
**Flow:**
1. The member extracts it with no password → `ERR_PASSWORD_REQUIRED`, `password required to extract this archive; call again with the 'password' parameter`.
2. With `nope` → `ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`.
3. With `s3cret-Ünï` → `ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`.
**Postconditions:** as SC-002 at every step.
**Exceptions:**
- EXC-003a: the cut falls inside the archive's leading part (the marker and the encryption header for version 5, the marker and the main header for version 4) → corrupt at every step, the password is not asked for.
- EXC-003b: version 4, encrypted file names, wrong password → corrupt as today (BL-0136); empty password → the wrong password message, as today.
- EXC-003c: readable file names, encrypted data, cut anywhere → corrupt with no password, a wrong one or the right one.

### SC-004: A truncated archive meets another refusal
**Actor:** Member.
**Flow:**
1. Not a member, missing path, folder, multi volume name, server without RAR support, encrypted file names with a missing or wrong password (SC-003) → reported before the truncation.
2. A truncated archive → reported before a link or escaping entry, a missing password for encrypted data, a file larger than declared, the destination, a collision and both limits.
**Postconditions:** as SC-002.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-U]: What a complete RAR archive is
> THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.

- **Business rules:** this holds for version 4 and version 5, for every kind of block (file, folder, link, recovery record, comment, service and encryption blocks), and for blocks whose header is encrypted, read with the right password. Bytes after the end of archive block are ignored, as today. Bytes before the RAR marker are skipped wherever the extraction finds the marker today.
- **Scenario:** SC-001, SC-002, SC-003
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: A missing end of archive block
> IF a RAR archive ends after a whole block that is not the end of archive block THEN THE archive extraction SHALL refuse it as FR-MOD-001 states, even when every file it lists is whole.

- **Business rules:** version 5 already behaves this way; version 4 archives without the block (made by RAR 1.5) are refused from now on.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-003 [EARS-U]: The message of a cut
> THE archive extraction SHALL word the refusal of an archive that holds a whole RAR marker and is not complete exactly `archive is corrupt or not a valid rar file: unexpected end of archive`, with the code `ERR_INVALID_ARGUMENT`.

- **Business rules:** a file shorter than the marker keeps today's refusal: `ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid rar file:`. The version 4 marker is 7 bytes, the version 5 marker 8 bytes.
- **User visible text:** `archive is corrupt or not a valid rar file: unexpected end of archive`
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Missing password first on encrypted file names
> IF a RAR archive declares encrypted file names in its leading part AND that leading part is whole AND no password is given THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `password required to extract this archive; call again with the 'password' parameter`, whether the rest of the archive is complete or not.

- **Business rules:** the leading part is the marker and the encryption header for version 5 (62 bytes in `hidden.rar`), the marker and the main header for version 4 (20 bytes). A cut inside it is refused as FR-NEW-003 states. An empty password counts as given (SPEC-0080 FR-NEW-020).
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Wrong password first on version 5 encrypted file names
> IF a version 5 RAR archive declares encrypted file names in its whole leading part AND the given password is wrong or empty THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete or not.

- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-006 [EARS-UB]: A refused cut leaves no trace
> THE archive extraction SHALL NOT write a file, create a folder, charge the remaining storage or the session write allowance, record an audit line, or show the password in a message, a log line or an audit line, for a call refused under FR-MOD-001.

- **Business rules:** with `overwrite` set, an existing file at a target path keeps its content.
- **Scenario:** SC-002, SC-003, SC-004
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Where the truncation sits in the order of refusals
> IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.

- **Business rules:** this is SPEC-0080 FR-NEW-015 with item 5 covering every cut; messages of the other items are unchanged. With readable file names and encrypted data, a cut is reported before any password question (as SPEC-0080 E2E-048 and E2E-060 already show for version 5).
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-008 [EARS-U]: Both doors answer alike
> THE archive extraction SHALL give, through the HTTP API, the same code and message as through the assistant tool for every refusal of this specification.

- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: The rest is unchanged
> THE archive extraction SHALL extract or refuse every complete RAR archive exactly as before this increment, and SHALL keep, for a version 4 archive with encrypted file names whose leading part is whole, cut or not, today's answers to a password that is not the right one: a wrong password is refused as corrupt (a message starting `archive is corrupt or not a valid rar file:`), an empty password with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.

- **Business rules:** unchanged: plain, solid, encrypted data, encrypted file names, folders, recovery record, comment, bytes before the marker or after the end of archive block, an empty archive, a link entry refused, a whole archive whose file is larger than declared refused with `archive entry '<name>' decompressed to more bytes than declared <size>`, multi volume names refused.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: Refuse a truncated RAR archive (modifies SPEC-0080/FR-NEW-011)
> IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

- **Before:** see 2.1: success with fewer files (version 4 cut between files or before the end block, version 5 with encrypted names and the right password, an archive preceded by other bytes); success with nothing (version 4 marker alone, or with its main header); the larger than declared message (version 4 plain data cut); the wrong password or password needed message (cut inside encrypted data); a varying detail (cut inside a header).
- **After:** the statement above.
- **Reason:** SPEC-0080 FR-NEW-011 says truncated without limiting it; a member must never take a partial extraction for a whole one (BL-0131).
- **Scenario:** SC-002, SC-003
- **Priority:** Must

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance

- The refusal of a cut archive arrives before any file of it is decoded; a complete archive takes no observable longer to extract than today (no second decode of the data).

### 7.2 Security and access

- The password is never shown in a refusal, a log line or an audit line (FR-NEW-006, E2E-048).

### 7.5 Reliability

- A refused cut leaves the project exactly as it was; repeating it gives the same refusal (E2E-046).

### 7.6 Operability

- A refused extraction records no audit line, as every other extraction refusal.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | complete version 4 archive with no file; marker alone; marker and main header | empty folder; refused; refused | E2E-005, E2E-013, E2E-012 |
| Boundary values | one byte short; every length from 0 to one byte short; cut exactly on a block boundary; cut inside the marker; cut inside or right after the leading part of an encrypted names archive | refused, exact message after a whole marker, prefix inside it; password questions only once the leading part is whole | E2E-015, E2E-016, E2E-034, E2E-008, E2E-014, E2E-024, E2E-026, E2E-033 |
| Concurrent actions | not applicable: the refusal reads only the archive; no project state is read before it | none | none |
| Slow or lost connectivity | not applicable: the archive is already stored | none | none |
| Access states | not a member with a cut archive | membership refusal first | E2E-042 |
| Data variations | version 4 and 5; solid; folders; recovery record and comment; accented password; bytes before the marker or after the end block | as specified | E2E-001 to E2E-007, E2E-017, E2E-018, E2E-047 |
| Navigation | not applicable: one call | none | none |
| Partial operations | the archive itself is the partial upload; a refused call writes nothing even with `overwrite` | nothing written | E2E-039, E2E-045 |
| Dependency failures | server without RAR support | unsupported extension, unchanged | E2E-043 |
| Ordering and timing | cut plus a link, collision, storage limit, multi volume name, membership, password; destination inside the project | FR-NEW-007 | E2E-019 to E2E-021, E2E-025, E2E-027, E2E-031, E2E-032, E2E-037 to E2E-043, E2E-054, E2E-055 |
| Exhaustion | cut archive declaring more than the remaining storage | corrupt first | E2E-040 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver. An implementing agent uses this as its definition of done.

**Common data.** Project `atlas`, member `alice@example.com`; unless a test says otherwise there is no storage limit, the session write allowance is large, the server has RAR support, the call goes through the assistant tool with no destination and `overwrite` false, and "nothing written" means: the default destination folder (archive path minus `.rar`) does not exist, the remaining storage and the session write allowance are unchanged, and no audit line was recorded. The password of every encrypted archive is `s3cret-Ünï`. "The first N bytes of X" is X cut to its first N bytes and stored under the name the test gives. Archives (how each was made: Section 14.3):

| Archive | Version | Content | Size |
|---|---|---|---|
| `v4.rar` | 4 | `a.txt` = `hello` | 74 bytes |
| `two4.rar` | 4 | `a.txt` = `hello`, `b.txt` = `goodbye` | 123 bytes |
| `big4.rar` | 4 | `r.bin` = 3 000 times `x`, stored without compression, then `a.txt` = `hello` | 3 116 bytes |
| `dir4.rar` | 4 | `a.txt` = `hello`, folders `d/e` and `d` | 152 bytes |
| `solid4.rar` | 4, solid | `a.txt` = `hello`, `b.txt` = `goodbye` | 133 bytes |
| `extras4.rar` | 4 | as `two4.rar`, plus a recovery record, the comment `a comment`, locked | 745 bytes |
| `v4secret.rar` | 4 | `a.txt` = `hello`, data encrypted, names readable | 93 bytes |
| `hp4.rar` | 4 | `a.txt` = `hello`, `b.txt` = `goodbye`, data and names encrypted | 220 bytes |
| `v5.rar` | 5 | `a.txt` = `hello` | 76 bytes |
| `cut.rar` | 5 | the first 103 bytes of an archive holding `note.txt` = `top secret`, data encrypted, names readable | 103 bytes |
| `hidden.rar` | 5 | `note.txt` = `top secret`, data and names encrypted | 254 bytes |
| `cuth.rar` | 5 | the first 169 bytes of `hidden.rar` | 169 bytes |
| `hp5two.rar` | 5 | `a.txt` = `hello`, `b.txt` = `goodbye`, data and names encrypted | 382 bytes |
| `hl2.rar` | 5 | `a.txt` = `hello` and a symbolic link `l`, data and names encrypted | 366 bytes |

Offsets used below, measured on these archives: in `two4.rar` `a.txt` ends at 67 and `b.txt`'s header spans 67 to 108; in `big4.rar` `r.bin`'s data spans 62 to 3 061; in `v4secret.rar` the data spans 70 to 85; in `hp4.rar` `a.txt` ends at 108 and `b.txt`'s data spans 180 to 195; in `hidden.rar` the file ends at 222; in `hp5two.rar` `a.txt` ends at 206; in `hl2.rar` the link's header ends at 334; in `dir4.rar` folder `d/e` ends at 107; in `solid4.rar` `a.txt` ends at 79; in `extras4.rar` the recovery record starts at 224.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-009, FR-NEW-001 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-009, FR-NEW-001 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-009, FR-NEW-001 | Critical |
| E2E-004 | New | happy | SC-001 | FR-NEW-009, FR-NEW-001 | Critical |
| E2E-005 | New | edge | SC-001 | FR-NEW-001, FR-NEW-009 | Medium |
| E2E-006 | New | edge | SC-001 | FR-NEW-001, FR-NEW-009 | Medium |
| E2E-007 | New | edge | SC-001 | FR-NEW-001, FR-NEW-009 | Medium |
| E2E-008 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-009 | New | failure | SC-002 | FR-NEW-002, FR-MOD-001 | Critical |
| E2E-010 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-011 | New | failure | SC-002 | FR-MOD-001, FR-NEW-003 | High |
| E2E-012 | New | failure | SC-002 | FR-NEW-002, FR-MOD-001 | High |
| E2E-013 | New | edge | SC-002 | FR-NEW-003, FR-MOD-001 | High |
| E2E-014 | New | edge | SC-002 | FR-NEW-003 | Medium |
| E2E-015 | New | edge | SC-002 | FR-NEW-002, FR-NEW-003 | High |
| E2E-016 | New | edge | SC-002 | FR-MOD-001, FR-NEW-003, FR-NEW-001 | Critical |
| E2E-017 | New | failure | SC-002 | FR-MOD-001 | High |
| E2E-018 | New | failure | SC-002 | FR-MOD-001 | High |
| E2E-019 | New | failure | SC-003 | FR-MOD-001, FR-NEW-007 | Critical |
| E2E-020 | New | failure | SC-003 | FR-NEW-007, FR-MOD-001 | High |
| E2E-021 | New | failure | SC-003 | FR-NEW-007, FR-MOD-001 | High |
| E2E-022 | New | failure | SC-003 | FR-MOD-001 | Critical |
| E2E-023 | New | failure | SC-003 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-024 | New | edge | SC-003 | FR-MOD-001, FR-NEW-001 | High |
| E2E-025 | New | failure | SC-003 | FR-NEW-004, FR-NEW-007 | High |
| E2E-026 | New | edge | SC-003 | FR-NEW-004, FR-NEW-003 | High |
| E2E-027 | New | failure | SC-003 | FR-NEW-009, FR-NEW-007 | Medium |
| E2E-028 | New | failure | SC-003 | FR-NEW-002, FR-MOD-001 | Critical |
| E2E-029 | New | failure | SC-003 | FR-MOD-001, FR-NEW-003 | Critical |
| E2E-030 | New | failure | SC-003 | FR-MOD-001 | High |
| E2E-031 | New | failure | SC-003 | FR-NEW-005, FR-NEW-007 | High |
| E2E-032 | New | failure | SC-003 | FR-NEW-004, FR-NEW-007 | High |
| E2E-033 | New | edge | SC-003 | FR-NEW-004, FR-NEW-003 | Medium |
| E2E-034 | New | edge | SC-003 | FR-MOD-001, FR-NEW-003, FR-NEW-004 | Critical |
| E2E-035 | New | failure | SC-003 | FR-MOD-001, FR-NEW-009 | Medium |
| E2E-036 | New | failure | SC-002 | FR-NEW-002, FR-NEW-003 | Medium |
| E2E-037 | New | failure | SC-004 | FR-NEW-007 | High |
| E2E-038 | New | failure | SC-004 | FR-NEW-007, FR-NEW-006 | High |
| E2E-039 | New | side effect | SC-004 | FR-NEW-006, FR-NEW-007 | Critical |
| E2E-040 | New | failure | SC-004 | FR-NEW-007 | High |
| E2E-041 | New | failure | SC-004 | FR-NEW-007 | Medium |
| E2E-042 | New | failure | SC-004 | FR-NEW-007 | High |
| E2E-043 | New | failure | SC-004 | FR-NEW-007 | Low |
| E2E-044 | New | failure | SC-002 | FR-NEW-008 | High |
| E2E-045 | New | side effect | SC-002 | FR-NEW-006 | Critical |
| E2E-046 | New | side effect | SC-002 | FR-NEW-006 | Medium |
| E2E-047 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | High |
| E2E-048 | New | side effect | SC-003 | FR-NEW-006 | High |
| E2E-049 | New | failure | SC-001 | FR-NEW-009 | Medium |
| E2E-050 | New | failure | SC-003 | FR-NEW-005 | Medium |
| E2E-051 | New | failure | SC-003 | FR-NEW-005 | High |
| E2E-052 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-053 | New | failure | SC-003 | FR-NEW-008 | Medium |
| E2E-054 | New | edge | SC-004 | FR-NEW-007, FR-NEW-006 | Medium |
| E2E-055 | New | failure | SC-003 | FR-NEW-009, FR-NEW-007, FR-NEW-004 | Medium |

Coverage: happy 4, failure 35, side effect 4, edge 12, state transition 0 (no state changes: an extraction either writes or is refused), security 0 (not a security lot; the password exposure check is E2E-048). Happy to failure ratio 1:8.75.

### 9.2 New tests

#### E2E-001: Complete version 4 archive with two files
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/two4.rar`
- **When** the member extracts it
- **Then** the answer is 2 files, 12 bytes; `/uploads/two4/a.txt` holds `hello`, `/uploads/two4/b.txt` holds `goodbye`
- **And** one audit line names `/uploads/two4.rar`

#### E2E-002: Recovery record, comment and lock
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** High
- **Given** `/uploads/extras4.rar`
- **When** the member extracts it
- **Then** the answer is 2 files, 12 bytes, with the content of E2E-001 under `/uploads/extras4`

#### E2E-003: Version 4 with encrypted file names, right password
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the answer is 2 files, 12 bytes; `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`

#### E2E-004: Version 5 with encrypted file names, two files, right password
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp5two.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the answer is 2 files, 12 bytes, with the content of E2E-003 under `/uploads/hp5two`

#### E2E-005: Complete empty version 4 archive
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/empty4.rar`: the first 20 bytes of `v4.rar` followed by its last 7 bytes (marker, main header, end of archive block)
- **When** the member extracts it
- **Then** the answer is 0 files, 0 bytes, 1 folder created; `/uploads/empty4` exists and is empty

#### E2E-006: Bytes after the end of archive block
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/tail4.rar` = `v4.rar` followed by the 13 bytes `trailing junk`, and `/uploads/tail5.rar` = `v5.rar` followed by the same bytes
- **When** the member extracts each
- **Then** each answer is 1 file, 5 bytes; `/uploads/tail4/a.txt` and `/uploads/tail5/a.txt` hold `hello`

#### E2E-007: Bytes before the marker, complete archive
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/sfx4.rar` = 100 zero bytes followed by `v4.rar`, and `/uploads/sfx5.rar` = 100 zero bytes followed by `v5.rar`
- **When** the member extracts each
- **Then** each answer is 1 file, 5 bytes; `/uploads/sfx4/a.txt` and `/uploads/sfx5/a.txt` hold `hello`

#### E2E-008: Version 4 cut between two files
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/half4.rar`, the first 67 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `archive is corrupt or not a valid rar file: unexpected end of archive`; nothing written: `/uploads/half4` does not exist (today `/uploads/half4/a.txt` is written and the call succeeds)

#### E2E-009: Version 4 missing only its end of archive block
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/noend4.rar`, the first 67 bytes of `v4.rar` (every file whole)
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-010: Version 4 cut inside a file's data
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/big4cut.rar`, the first 1 562 bytes of `big4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `archive entry 'r.bin' decompressed to more bytes than declared 3000`; nothing written

#### E2E-011: Version 4 cut inside a header
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hdr4.rar`, the first 90 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is exactly that of E2E-008; nothing written

#### E2E-012: Version 4 marker and main header only
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** High
- **Given** `/uploads/main4.rar`, the first 20 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; `/uploads/main4` does not exist (today an empty folder is created and the call succeeds)

#### E2E-013: Version 4 marker alone
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-MOD-001; **Priority:** High
- **Given** `/uploads/mark4.rar`, the first 7 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; `/uploads/mark4` does not exist

#### E2E-014: Cut inside the marker
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/m6.rar`, the first 6 bytes of `v4.rar`, and `/uploads/m7.rar`, the first 7 bytes of `v5.rar`
- **When** the member extracts each
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing written

#### E2E-015: One byte short
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** `/uploads/short4.rar`, the first 73 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is exactly that of E2E-008; nothing written

#### E2E-016: Every length of the plain version 4 archives
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003, FR-NEW-001; **Priority:** Critical
- **Given** for each of `v4.rar`, `two4.rar`, `dir4.rar`, `solid4.rar` and `extras4.rar`, and each length from 0 to the archive's size minus one, the archive cut to that length stored as `/uploads/p.rar`
- **When** the member extracts each one
- **Then** from 7 bytes on the refusal is exactly that of E2E-008; below 7 bytes it is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; in every case `/uploads/p` does not exist
- **And** each whole archive extracts with every file it holds

#### E2E-017: Version 4 cut after a folder
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/dircut.rar`, the first 107 bytes of `dir4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written (today `a.txt` and `d/e` are written and the call succeeds)

#### E2E-018: Solid version 4 cut between files
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/solidcut.rar`, the first 79 bytes of `solid4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-019: Version 4, encrypted data, cut, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-007; **Priority:** Critical
- **Given** `/uploads/v4cut.rar`, the first 78 bytes of `v4secret.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not `incorrect password for this archive; call again with the correct 'password' parameter`; nothing written

#### E2E-020: Version 4, encrypted data, cut, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-MOD-001; **Priority:** High
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-008, not `ERR_PASSWORD_REQUIRED`; nothing written

#### E2E-021: Version 4, encrypted data, cut, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-MOD-001; **Priority:** High
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it with `nope`
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-022: Version 4, encrypted names, cut between files, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4half.rar`, the first 108 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written (today `/uploads/hp4half/a.txt` is written and the call succeeds)

#### E2E-023: Version 4, encrypted names, cut inside data, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/hp4data.rar`, the first 190 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not the wrong password message; nothing written

#### E2E-024: Version 4, encrypted names, leading part only, right password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4lead.rar`, the first 20 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; `/uploads/hp4lead` does not exist (today an empty folder is created)

#### E2E-025: Version 4, encrypted names, cut, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-007; **Priority:** High
- **Given** `/uploads/hp4half.rar` of E2E-022
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; nothing written

#### E2E-026: Version 4, encrypted names, cut inside the leading part, no password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4short.rar`, the first 16 bytes of `hp4.rar`
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-008, not the password needed message (today it is the password needed message); nothing written

#### E2E-027: Version 4, encrypted names, cut, wrong password (unchanged, BL-0136)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009, FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/hp4half.rar` of E2E-022, and `/uploads/hp4.rar` whole
- **When** the member extracts each with `nope`
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing written

#### E2E-028: Version 5, encrypted names, end block missing, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hid222.rar`, the first 222 bytes of `hidden.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written (today `/uploads/hid222/note.txt` is written and the call succeeds)

#### E2E-029: Version 5, encrypted names, cut inside data, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not the wrong password message; nothing written

#### E2E-030: Version 5, encrypted names, cut between files, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/hp5half.rar`, the first 206 bytes of `hp5two.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-031: Version 5, encrypted names, cut, wrong password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** High
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `incorrect password for this archive; call again with the correct 'password' parameter`; nothing written

#### E2E-032: Version 5, encrypted names, cut, no password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-007; **Priority:** High
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-025; nothing written

#### E2E-033: Version 5, cut inside the encryption header, no password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hid40.rar`, the first 40 bytes of `hidden.rar`
- **When** the member extracts it with no password, then with `nope`
- **Then** both refusals are that of E2E-008; nothing written

#### E2E-034: Every length of the encrypted names archives, right password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Given** for each of `hidden.rar`, `hp4.rar` and `hp5two.rar`, and each length from 0 to the archive's size minus one, the archive cut to that length stored as `/uploads/p.rar`
- **When** the member extracts each one with `s3cret-Ünï`
- **Then** from the whole marker on the refusal is exactly that of E2E-008, below it a message starting `archive is corrupt or not a valid rar file:`; `/uploads/p` never exists
- **And** extracted with no password, every length from the whole leading part on (20 bytes for `hp4.rar`, 62 for the two others) gives the refusal of E2E-025, and every shorter length from the whole marker on gives that of E2E-008

#### E2E-035: Version 5, readable names, encrypted data, cut, right password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/cut.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-036: Version 5 missing only its end of archive block (unchanged)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/noend5.rar`, the first 68 bytes of `v5.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written

#### E2E-037: A cut wins over a link entry
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/hl2cut.rar`, the first 334 bytes of `hl2.rar` (the link `l` is listed, the end block is missing)
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not `ERR_NOT_SUPPORTED` naming `l`; nothing written

#### E2E-038: A cut wins over a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-006; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008 and `/uploads/half4/a.txt` holding `old`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `ERR_NO_CLOBBER`; `/uploads/half4/a.txt` still holds `old`

#### E2E-039: A cut with `overwrite` replaces nothing
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-NEW-007; **Priority:** Critical
- **Given** `/uploads/half4.rar` of E2E-008 and `/uploads/half4/a.txt` holding `old`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is that of E2E-008; `/uploads/half4/a.txt` still holds `old` (today it is replaced by `hello` and the call succeeds); the remaining storage and session write allowance are unchanged; no audit line

#### E2E-040: A cut wins over the storage limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/big4cut.rar` of E2E-010 (it declares 3 005 bytes)
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `ERR_PROJECT_QUOTA_EXCEEDED`; the remaining storage is still 1 000 bytes

#### E2E-041: A multi volume name wins over a cut
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/big.part1.rar`, the first 67 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `multi volume RAR archives are not supported: extract a single file archive`; nothing written

#### E2E-042: Membership wins over a cut
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008 in project `atlas`, and `bob@example.com`, not a member of `atlas`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with `'bob@example.com' is not a member of 'atlas'`; nothing written

#### E2E-043: Server without RAR support
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Low
- **Given** a server delivered without RAR support and `/uploads/half4.rar` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `unsupported archive extension: '.rar'`; nothing written

#### E2E-044: The HTTP API refuses a version 4 cut alike
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it through the HTTP API
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, as E2E-008; nothing written

#### E2E-045: A refused cut leaves no trace
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a storage limit with 10 000 bytes remaining, a session write allowance of 10 000 bytes, and `/uploads/half4.rar` of E2E-008
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; the remaining storage is 10 000 bytes, the session allowance still accepts a 10 000 byte write; `/uploads/half4` does not exist; the audit trail holds no extraction line

#### E2E-046: Repeating a refused cut
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it twice
- **Then** both refusals are that of E2E-008; nothing written after either

#### E2E-047: Bytes before the marker, cut archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/sfxcut4.rar` = 100 zero bytes followed by the first 67 bytes of `two4.rar`, and `/uploads/sfxcut5.rar` = 100 zero bytes followed by the first 68 bytes of `v5.rar`
- **When** the member extracts each
- **Then** each refusal is that of E2E-008; nothing written (today both succeed)

#### E2E-048: The password stays out of a cut refusal
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4data.rar` of E2E-023
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; the text `s3cret-Ünï` appears in no message, log line or audit line written during the call

#### E2E-049: A whole archive whose file is larger than declared (unchanged)
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/bomb.rar`, a complete version 5 archive whose file `big.bin` declares 10 bytes and holds 1 000 000 bytes (SPEC-0080 E2E-012)
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'big.bin' decompressed to more bytes than declared 10`; nothing written

#### E2E-050: Version 5, encrypted names, cut, empty password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with the empty password
- **Then** the refusal is that of E2E-031; nothing written

#### E2E-051: Version 5, encrypted names, cut between files, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp5half.rar` of E2E-030
- **When** the member extracts it with `nope`
- **Then** the refusal is that of E2E-031; nothing written

#### E2E-052: The HTTP API refuses a version 5 encrypted names cut alike
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/hid222.rar` of E2E-028
- **When** the member extracts it through the HTTP API with `s3cret-Ünï`
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`; nothing written

#### E2E-053: The HTTP API refuses a version 4 encrypted data cut alike
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it through the HTTP API with `s3cret-Ünï`
- **Then** the refusal is that of E2E-052; nothing written

#### E2E-054: A cut with a destination inside the project
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it with destination `out`
- **Then** the refusal is that of E2E-008; `/out` does not exist

#### E2E-055: Version 4, encrypted names, cut, empty password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009, FR-NEW-007, FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/hp4half.rar` of E2E-022
- **When** the member extracts it with the empty password
- **Then** the refusal is that of E2E-031; nothing written

### 9.4 Modified tests

None. SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060 and E2E-065 keep their Given, When and Then; Section 14.3 names the internal checks that change.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-009 | E2E-001 to E2E-004 | E2E-049 | E2E-005, E2E-006, E2E-007 |
| SC-002 | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006, FR-NEW-008 | E2E-008 (main flow) | E2E-009 to E2E-012, E2E-017, E2E-018, E2E-036, E2E-044, E2E-047 | E2E-013 to E2E-016 |
| SC-003 | FR-MOD-001, FR-NEW-002 to FR-NEW-009 | E2E-028 (main flow, step 3), E2E-032 (step 1), E2E-031 (step 2) | E2E-019 to E2E-023, E2E-025, E2E-027, E2E-029, E2E-030, E2E-035, E2E-050 to E2E-053, E2E-055 | E2E-024, E2E-026, E2E-033, E2E-034 |
| SC-004 | FR-NEW-006, FR-NEW-007 | E2E-037 (main flow) | E2E-038, E2E-040 to E2E-043 | E2E-054 |

Side effects: SC-001 E2E-001 (its audit line); SC-002 E2E-045, E2E-046; SC-003 E2E-048; SC-004 E2E-039.

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001 to E2E-007, E2E-016, E2E-024, E2E-047 |
| FR-NEW-002 | E2E-009, E2E-012, E2E-015, E2E-028, E2E-036 |
| FR-NEW-003 | E2E-008, E2E-010, E2E-011, E2E-013 to E2E-016, E2E-023, E2E-026, E2E-029, E2E-033, E2E-034, E2E-036 |
| FR-NEW-004 | E2E-025, E2E-026, E2E-032, E2E-033, E2E-034, E2E-055 |
| FR-NEW-005 | E2E-031, E2E-050, E2E-051 |
| FR-NEW-006 | E2E-038, E2E-039, E2E-045, E2E-046, E2E-048, E2E-054 |
| FR-NEW-007 | E2E-019 to E2E-021, E2E-025, E2E-027, E2E-031, E2E-032, E2E-037 to E2E-043, E2E-054, E2E-055 |
| FR-NEW-008 | E2E-044, E2E-052, E2E-053 |
| FR-NEW-009 | E2E-001 to E2E-007, E2E-027, E2E-035, E2E-049, E2E-055 |
| FR-MOD-001 | E2E-008 to E2E-013, E2E-016 to E2E-024, E2E-028 to E2E-030, E2E-034, E2E-035, E2E-047 |

## 11. Open questions

- The test designer, the self consistency reviewer and the auditor were the author (no subagent tool in this run, Section 13): does not change observable behavior.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| RAR archive | a file in the RAR format, ending in `.rar` in any letter case | archive extraction |
| Version 4, version 5 | the two RAR formats the extraction reads; version 4 is written by RAR up to 6, version 5 by RAR 5 and later | archive extraction |
| Marker | the fixed bytes that start a RAR archive: 7 for version 4, 8 for version 5 | archive extraction |
| Block | one unit after the marker: a header, possibly followed by the data it declares | archive extraction |
| End of archive block | the block that closes a complete archive | archive extraction |
| Complete archive | an archive where every block, up to and including the end of archive block, is wholly present (FR-NEW-001) | archive extraction |
| Truncated archive, cut | an archive missing its last bytes, from any position after the start | archive extraction |
| Leading part | for an archive with encrypted file names, the part that declares the encryption: marker and encryption header (version 5), marker and main header (version 4) | archive extraction |
| Encrypted data | file contents encrypted, file names readable | archive extraction |
| Encrypted file names | file names and contents encrypted; listing the archive needs the password | archive extraction |
| Solid archive | an archive whose files are compressed as one stream | archive extraction |
| Recovery record | a block that lets RAR repair damage; holds no file | archive extraction |
| Bytes before the marker | a prefix such as a self extracting program, skipped to find the archive | archive extraction |
| Order of refusals | which refusal a member sees when several apply (FR-NEW-007) | archive extraction |
| Member | a person with access to the project | archive extraction |
| Operator | the person who runs the server and reads its logs and audit trail | archive extraction |
| Remaining storage | what the project's storage limit still allows | project storage |
| Session write allowance | what the member can still write in their session | project storage |
| Audit line | the record of one successful extraction | project storage |
| HTTP API, assistant tool | the two doors through which a member extracts | archive extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 | 3 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Round 1 findings and amendments:** F-1 (G1 forced choice) an empty password on a version 4 archive with encrypted names, cut or whole, had no stated answer (FR-NEW-009 named only a wrong one, FR-NEW-004 treats empty as given, FR-NEW-005 is version 5 only); probe P-3 shows today's answer is the wrong password message: FR-NEW-009 and FR-NEW-007 item 4 now state it, E2E-055 added, coverage recomputed. A-1 E2E-049 rewritten on SPEC-0080 E2E-012's real archive (`big.bin`, 10 declared, `archive_rar.rs:402-418`). A-2 the run time line claimed a dependency optimization profile that does not exist (`Cargo.toml` has none), corrected in 14.3. A-3 14.1 row 7 citation narrowed to `archive_rar.rs:922-936`, `:987-1027`. Consistency fixes: wording naming the decoding component in Section 2 and 3.2 rewritten in product terms; E2E-016's last clause made exact.
**How the gates ran:** this harness exposes no subagent tool, so the test design (8.1), the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks (every test specified once and in the summary with the same category, scenario and requirements; requirement coverage counts; traceability lists; forbidden modals in EARS lines; implementation terms in Sections 1 to 13), every message reopened in the code (`archive.rs:439-443`, `:704-714`, `archive_rar.rs:103`, `:219-223`) and every offset and outcome re measured on the probes of 14.1. Round 2 re ran the same checks on the amended file: clean.
**No implementation detail outside Section 14:** PASS. Error codes, verbatim messages, option names (`overwrite`, `destination`, password), file names, byte counts and offsets of the test archives are WHAT: they are what the member stores and sees.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | two doors, one function; RAR behind the `rar` feature | `crates/core/src/tools/archive.rs:143` (`extract_archive`), `:386` (`ArchiveFormat::Rar => decode_rar`); `crates/core/src/api/dataplane.rs:176`; `crates/core/Cargo.toml:27` |
| 2 | RAR5 structural check, exact message | `crates/core/src/tools/archive_rar.rs:21` (called first), `:110-135` (`ensure_rar5_complete`), `:103` (`TRUNCATED`); `crates/core/src/tools/archive.rs:439-443` (`corrupt`: `archive is corrupt or not a valid {format} file: {detail}`) |
| 3 | RAR5 walk stops at the encryption header; listing password mapping; encrypted read error mapped to the password message | `crates/core/src/tools/archive_rar.rs:129` (`kind == HEAD_END \|\| kind == HEAD_CRYPT`), `:105-109` (doc: "neither is a cut after an archive encryption header"), `:36-40` (listing), `:162-169` (`listing_error`), `:69-81` (read error: plain is `oversized`, encrypted `BadPassword`/`BadData`/`MissingPassword` is `password_required_message`); probe P-2 below |
| 4 | RAR4 not checked; outcomes per cut | `crates/core/src/tools/archive_rar.rs:111-113` (`strip_prefix(RAR5_SIGNATURE)` else `Ok`), `:219-223` (`oversized`); `.agent_docs/tools.md:208-210`; probe P-1 below |
| 5 | wrong password on RAR4 encrypted names reads as corrupt | probe P-3; `crates/core/src/tools/archive_rar.rs:162-169`; BL-0136 |
| 6 | prefix found by the reader, walk skipped; trailing bytes ignored | probe P-4; `unrar_sys-0.5.8/vendor/unrar/rardefs.hpp:24` (`MAXSFXSIZE 0x200000`), `archive.cpp:150`; `crates/core/src/tools/archive_rar.rs:111` |
| 7 | refusal leaves no trace; password never logged | `crates/core/src/tools/archive_rar.rs:922-936` (`assert_never_recorded`), `:987-1027` (SPEC-0080 E2E-020, E2E-021, E2E-030); SPEC-0080 FR-NEW-013 |
| 8 | governing specs | `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md:230-233` (FR-NEW-011), `:258-262` (FR-NEW-015), `:554` (E2E-014), `:794` (E2E-048), `:863` (E2E-058), `:875` (E2E-060), `:905` (E2E-065), `:1177-1179` (erratum 2); `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-design.md:197` (DEC-004); `sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/sdlc-spec.md:181` (FR-NEW-009 item 5); `sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/sdlc-spec.md:164-165` (FR-MOD-002) |
| 9 | tests today | `crates/core/src/tools/archive_rar.rs:713-739` (E2E-014 twice, RAR5 builder), `:741-752` (`every_strict_prefix_of_a_rar5_archive_is_corrupt`, unit, builder only), `:755-759` (`non_rar5_input_is_left_to_the_library`), `:1030-1038` (E2E-062, `v4.rar` whole), `:1123` (E2E-065), `:1195-1201` (E2E-048 `cut.rar`), `:1255-1260` (E2E-058 `cuth.rar` with `nope`), `:1273-1280` (E2E-060); no test cuts a RAR4 archive or passes the right password on a cut encrypted names archive (`rg -n 'V4\[|V4_SECRET\[|HIDDEN\[|CUTH, Some\(PASSWORD' crates/core/src/tools/archive_rar.rs` finds nothing); 78 tests in the file (`rg -c '#\[(tokio::)?test'`) |

Test command: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` (as `test.sh:7`); full gate `cargo test --workspace` plus that line.

**Probes (2026-11-09, throwaway, `/tmp` only, nothing committed).** A release build of `unrar` 0.5.8 (the locked version, `Cargo.lock:6419-6420`) running the same two passes as `decode_rar` without the RAR5 walk, over every strict prefix of each archive; archives made by `/tmp/rarbin/v6/rar/rar` (RAR 6.24 trial) and `/tmp/rarbin/rar/rar` (RAR 7.23 trial).

- **P-1 RAR4 plain** (`v4.rar` 74 bytes): 0 to 6 `BadArchive`; 7 `Ok []`; 8 to 19 `BadData` (listing); 20 `Ok []`; 21 to 61 `BadData`; 62 to 66 `BadData` at read (today `oversized`); 67 `Ok [a.txt]`; 68 to 73 `BadData`. `two4.rar` (123): 67 `Ok [a.txt]`, 109 to 115 read `BadData` on `b.txt`, 116 `Ok [a.txt, b.txt]`. `big4.rar` (3 116): 62 to 3 061 read `BadData`, 3 062 `Ok [r.bin]`, 3 109 `Ok [r.bin, a.txt]`. `dir4.rar` (152): 67, 107, 145 `Ok` with fewer entries. `solid4.rar` (133): 79 `Ok [a.txt]`, 124 `Ok` both. `extras4.rar` (745): 224 to 738 `Ok` both (recovery record cut). `v4secret.rar` right password: 70 to 85 read `BadData` (today the wrong password message), 86 `Ok`; no password: 70 to 86 `PWNEEDED` (today the password needed message).
- **P-2 encrypted names, right password**: `hp4.rar` (220): 20 `Ok []`, 92 and 180 read `BadData`, 108 `Ok [a.txt]`, 196 `Ok` both. `hidden.rar` (254): 222 and 253 `Ok [note.txt]`, 190 read `BadData`. `hp5two.rar` (382): 206 and 236 `Ok [a.txt]`, 318 read `BadData`, 350 `Ok` both. `hl2.rar` (366): 334 and 361 `Ok [a.txt, l]` (today the link refusal).
- **P-3 encrypted names, no or wrong password**: `hp4.rar` none: 8 to 13 `BadData`, from 14 `MissingPassword`; empty password, whole or first 108 bytes: `MissingPassword`, mapped today to the wrong password message; `nope`: `BadData` (whole too); 20 `Ok []`. `hidden.rar`/`hp5two.rar` none: from 62 `MissingPassword`, `nope`: from 62 `BadPassword`; with the walk, 8 to 61 are refused first.
- **P-4 framing**: 100 zero bytes before `v4.rar` or `v5.rar`: `Ok [a.txt]`; `trailing junk` after either: `Ok [a.txt]`; `empty4.rar` (first 20 bytes of `v4.rar` plus its last 7): `Ok []`.
- **P-5 what RAR 6.24 writes at the end of a RAR4 archive**: every archive above, plain, solid, encrypted data, with folders, recovery record, comment and lock, ends with the 7 byte end of archive block `c4 3d 7b 00 40 07 00` (CRC `0x3dc4`, type `0x7B`, flags `0x4000`, size 7); with encrypted names the end block is encrypted like every header after the main header. `rar a -ma4 -en` (the old "no end block" switch) is accepted silently (exit 0) and still writes the block; `-ed` means "do not add empty directories" (`rar -?` line 50), not "no end block". RAR 7.23 cannot write version 4. UnRAR's own comment: only "older RAR 1.5" archives lack the block (`unrar_sys-0.5.8/vendor/unrar/arcread.cpp:100-105`), and at the end of a file it reports the end of the archive whether or not the block was seen (`dll.cpp:222-231`: `ERAR_END_ARCHIVE` unless `BrokenHeader`).

### 14.3 Design inputs

- **No end of archive signal through the safe API.** `unrar` 0.5.8 maps `ERAR_END_ARCHIVE` to `Ok(None)` (`unrar-0.5.8/src/open_archive.rs:452`) and UnRAR returns it at end of file even when the end block was never read (`dll.cpp:222-231`, `arcread.cpp:96-111` only sets a warning). `FileHeader` carries no packed size (`open_archive.rs:582-590`). Completeness of an archive with encrypted file names (FR-MOD-001 with the right password) therefore cannot be read from the library; the header walk needs the decrypted headers: version 4 AES 128 CBC with the RAR 3 key derivation and an 8 byte salt per header; version 5 AES 256 CBC with PBKDF2 HMAC SHA 256 and the encryption header's salt and check value. `unsafe` is forbidden in the workspace (SPEC-0080 DEC-006 rejected raw `unrar_sys` for that reason). **Signal:** E2E-022, E2E-024, E2E-028, E2E-030, E2E-034, E2E-037 fail.
- **RAR4 block walk.** Generic: CRC16, type, flags, header size, and when flag `0x8000` (LONG_BLOCK) is set a 4 byte add size (file blocks: packed size, plus high 4 bytes when `0x100` LHD_LARGE); end block type `0x7B`; main header flag `0x0080` (MHD_PASSWORD) means encrypted headers. **Signal:** E2E-016 over `extras4.rar` (recovery record and comment blocks), `dir4.rar`.
- **Marker search.** UnRAR finds the marker up to `MAXSFXSIZE` (2 MiB) into the file (`rardefs.hpp:24`, `archive.cpp:150`); the walk must start at the same marker the reader uses, not at offset 0 (`archive_rar.rs:111` uses `strip_prefix`). **Signal:** E2E-047.
- **Leading part threshold for the password question.** UnRAR answers `MissingPassword` from 14 bytes of a RAR4 encrypted names archive (P-3), before the main header is whole; FR-NEW-004 asks for a whole leading part (20 bytes). **Signal:** E2E-026, E2E-034.
- **Messages.** Every cut after a whole marker uses the detail `unexpected end of archive` (`archive_rar.rs:103`), never the reader's own text, and never `oversized` or the password messages. **Signal:** E2E-010, E2E-011, E2E-019, E2E-023, E2E-029.
- **Internal tests that pin the old limitation.** `non_rar5_input_is_left_to_the_library` (`archive_rar.rs:755-759`) asserts that a bare RAR4 marker passes the check; it changes or goes with the design. `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:741-752`) is the model for E2E-016 and E2E-034. `.agent_docs/tools.md:208-210` states the RAR4 limitation and must be rewritten.
- **Test archives.** New archives to commit under `crates/core/tests/fixtures/rar/` with a README line each (made once, never at test time): `two4.rar` = `rar a -ma4 -ep two4.rar a.txt b.txt`; `big4.rar` = `rar a -ma4 -ep -m0 big4.rar r.bin a.txt` where `r.bin` is 3 000 bytes of `x`; `dir4.rar` = `rar a -ma4 dir4.rar d a.txt` with empty folder `d/e`; `solid4.rar` = `rar a -ma4 -ep -s solid4.rar a.txt b.txt`; `extras4.rar` = `rar a -ma4 -ep -rr5% -zc.txt -k extras4.rar a.txt b.txt` with `c.txt` = `a comment`; `hp4.rar` = `rar a -ma4 -ep -hps3cret-Ünï hp4.rar a.txt b.txt` (all RAR 6.24); `hp5two.rar` = `rar a -ma5 -ep -hps3cret-Ünï hp5two.rar a.txt b.txt` (RAR 7.23). Sizes in 9.0 are those measured on the probes; RAR stamps file times, so a remade archive keeps its sizes but not its bytes, and the offsets of 9.0 must be re measured if an archive is remade. Cut archives are NOT committed: tests cut the committed archives at run time (E2E-016 and E2E-034 loop over every length, as `every_strict_prefix_of_a_rar5_archive_is_corrupt` does), so one committed archive covers every cut and no binary per offset is kept.
- **Run time.** Every length with a password runs the key derivation each time: the probe took 2.9 s (`hp4.rar`), 7.1 s (`hidden.rar`), 10.4 s (`hp5two.rar`) in release; the workspace has no dependency optimization override for the dev profile (`rg -n 'profile.dev.package' Cargo.toml` finds nothing), so the debug test build compiles the C++ reader without optimization and runs slower than these figures. The design may run the password loop of E2E-034 over the boundary offsets of 9.0 plus every length of the leading part when the full loop is too slow, and must say so.
- **Order inside decoding.** E2E-037 needs the truncation known before the entry rules refuse the link: today the listing pass applies the entry rules header by header (`archive_rar.rs:36-40`).

### 14.4 Defaults applied (the user authorized every decision; one line each)

- D-1 routing: the backlog says `Nature: DEBT`; the code violates an authority (SPEC-0080 FR-NEW-011 says truncated without limit, `sdlc-spec.md:230-233`; `archive_rar.rs:105-109` admits the gap), so Phase 1 Q2 routes it as BUG. The prompt hands a BUG to `/sdlc-analysis` with no document; the user asked for a spec ("spec all of them, design later"), as for BL-0124 in SPEC-0084, so this is written as a FEAT spec: the observable behavior changes (success with fewer files becomes a refusal, two misleading messages become the corrupt one). Nature: FEAT.
- D-2 complete means the end of archive block is reached, version 4 included; archives without it (RAR 1.5 era) are refused. Evidence P-5: RAR 6.24 always writes it, even with `-en`. Alternative rejected: accept a chain ending exactly at the end of the file, which leaves every cut on a block boundary undetected.
- D-3 one exact message for every cut after a whole marker (`unexpected end of archive`, today's RAR5 text); a file shorter than the marker keeps the prefix only, since it cannot be told from a non RAR file.
- D-4 password questions on encrypted names come first only when the leading part is whole; a cut inside it is corrupt (today's RAR5 behavior, extended to RAR4).
- D-5 a wrong password on RAR4 encrypted names stays corrupt here; BL-0136 (Nature BUG) written, pinned by E2E-027.
- D-6 RAR5 without end block stays refused (unchanged, E2E-036).
- D-7 archives preceded by bytes are in scope (same defect, E2E-047), bytes after the end block stay ignored.
- D-8 a cut sits at SPEC-0080 FR-NEW-015 item 5 for every kind, so it is reported before a link entry (E2E-037), the data password, the size check, collision and limits; the destination's position is left to SPEC-0086.
- D-9 tests cut committed archives at run time; new real archives are made once with RAR 6.24 and RAR 7.23 (14.3).
- D-10 tests: 55 new, 0 modified, 0 removed; no existing test changes its Given, When or Then. Test plan approved by the instruction "spec all of them, design later".
- D-11 depth M: one module, 10 requirements, no persisted change.
- D-12 interview rounds answered by the author, one line each: R1 scope: every truncated RAR archive refused as corrupt, password first order kept; out: BL-0136, multi volume, damaged but whole archives, other formats. R1 why now: BL-0131 found during US-0173; partial extractions reported as success. R2: Section 2 and probes P-1 to P-5. R3: SC-001 to SC-004 with their exceptions. R4: Section 6. R5: no extra decode of a complete archive, password never shown, nothing written on refusal.
