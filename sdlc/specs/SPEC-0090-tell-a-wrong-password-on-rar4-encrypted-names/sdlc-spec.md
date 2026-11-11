# Tell a wrong password on a RAR version 4 archive with encrypted file names: functional specification

> Id: SPEC-0090
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction, RAR version 4 with encrypted file names only), 10 requirements (8 new, 2 modified), public behavior change (a wrong password, today refused as a cut or damaged archive, is refused as a wrong password), no persisted change. Override with --depth.
> Status: designed
> Generated on: 2026-11-11
> Project: existing
> From backlog: BL-0136
> Depends on: SPEC-0080 (built), SPEC-0087 (built); both stated in 2.2, neither modified
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a
>
> Note: written non interactively. The user authorized every decision and gave the instruction "specify BL-0136", which is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who extracts a RAR version 4 archive whose file names are encrypted, and mistypes the password, is told today that the archive is cut short or damaged. They upload it again, or give up on a sound file, when all they had to do was type the password again. This increment tells them the password is wrong, as a RAR version 5 archive already does. Version 4 stores no value to check a password against, so the password is judged on the first protected block of the archive: when that block does not read as a block with the given password, the password is wrong. A cut or damaged archive given the right password keeps its current refusals.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

Measured on the committed test archives (Section 14.1, probes P-1 to P-4). `hp4.rar` is a version 4 archive with encrypted file names holding `a.txt` (`hello`) and `b.txt` (`goodbye`), password `s3cret-Ünï`.

- With the right password, a whole archive extracts. A cut anywhere after its whole leading part (the RAR marker and the main header, 20 bytes) is refused with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive` (SPEC-0087). [14.1 row 1]
- With no password: `ERR_PASSWORD_REQUIRED`, `password required to extract this archive; call again with the 'password' parameter`. With the empty password: `ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`. [row 2]
- With a wrong password the member never sees the wrong password message. On the whole archive:
  - `nope`, `s3cret-Üni` (one letter off) and every one of 300 generated passwords `wrong0` to `wrong299`: `ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`, as if the file were cut;
  - `w785`: `ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: File header damaged`.
  The same holds for the archive cut at 108 bytes, and for `hp4long.rar` (another version 4 archive with encrypted names) given `hp4.rar`'s password. [row 3]
- Damage inside the first protected block (one byte changed at offset 50) with the right password: `ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: File header damaged`. Damage inside the second file's block (offset 140) with the right password: the same message. [row 4]
- A version 4 archive with readable file names and encrypted data, given a wrong password, already gets the wrong password message; so does every version 5 archive with encrypted file names. [row 5]
- A refused extraction writes nothing, creates no folder, charges neither the remaining storage nor the session write allowance, records no audit line; the password never appears in a message, a log line or an audit line. [row 6]

### 2.2 Specs governing this area

- SPEC-0080 FR-NEW-003 (a wrong password gets the wrong password message) and FR-NEW-004 (encrypted file names are treated like encrypted data) are violated by 2.1 for version 4 with encrypted names. FR-NEW-015 item 4 puts a wrong password on encrypted names before the corrupt refusal (item 5). This increment satisfies them; it does not modify SPEC-0080.
- SPEC-0087 FR-NEW-009 pins today's corrupt answer to a wrong password on version 4 encrypted names, and FR-NEW-007 item 4 lists only a missing password, a version 5 wrong password and a version 4 empty password. FR-MOD-001 and FR-MOD-002 change those two clauses. SPEC-0087 FR-MOD-001 (every cut refused as `unexpected end of archive`) stays true for the right password, and for a password the archive's bytes cannot judge (FR-NEW-003, FR-NEW-004).
- SPEC-0087 E2E-027 pins today's answer and becomes E2E-MOD-001. SPEC-0087 E2E-022, E2E-025, E2E-026, E2E-034 and E2E-055 stay valid unchanged. [row 7]

### 2.3 What is verified today

- Version 4 encrypted names: right password whole and cut, no password, empty password, a cut inside the leading part; a wrong password pinned as corrupt (`nope` and `w785`, whole and cut at 108). One internal check pins that a first protected block failing its check with the right password is left to the reading library. [row 8]
- Not verified: a password one letter off, a password for another archive, damage inside the first protected block, the order of a wrong version 4 password against a collision or a storage limit, the HTTP API door for a wrong version 4 password. [row 8]

## 3. Scope

### 3.1 In scope

- RAR version 4 archives with encrypted file names, extracted with a password that is not empty, on a server with RAR support, from both doors.
- How the password is judged, and the answer for each case: wrong, right, archive cut before it can be judged, archive damaged.
- The place of that wrong password in the order of refusals.

### 3.2 Out of scope, and why

- Telling a damaged first protected block from a wrong password: the bytes cannot (version 4 stores no password check value). Such an archive given the right password is told the password is wrong; this is the lesser evil, chosen in BL-0136's notes, and is stated by FR-NEW-002.
- A wrong password whose reading of the first protected block happens to look like a possible block that runs past the end of the archive: told `unexpected end of archive`, as today. Measured at most 9 in 300 generated passwords on `hp4.rar` (Section 14.1 P-3); FR-NEW-004 and E2E-019 pin it.
- Damaged encrypted data with the right password (version 4, either kind): told the password is wrong today (P-4, SPEC-0080 decision on the data password signal); unchanged here, not reported by BL-0136.
- No password, the empty password, encrypted data with readable names, version 5: unchanged (FR-NEW-008).
- Multi volume archives, other formats: unchanged.

### 3.3 Constraints imposed by the context

- The user asked for this to be specified as a feature although the backlog entry says BUG (Section 14.4 D-1).
- No archive is made at test time; tests use the committed archives, cut or with one byte changed at run time (Section 14.3).

## 4. Actors

- **Member:** a person with access to a project; extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server, reads its logs and the audit trail.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored archive into project files | RAR version 4, encrypted file names, leading part, first protected block, its start, possible block start, wrong password, truncated archive, damaged archive, order of refusals |
| Project storage | the project tree, its storage limit and the session write allowance | remaining storage, session write allowance, audit line |

## 5. Usage scenarios

### SC-001: Extract with the right password
**Actor:** Member.
**Preconditions:** project `atlas`; `/uploads/hp4.rar` whole; nothing at `/uploads/hp4`.
**Flow:**
1. The member extracts it with `s3cret-Ünï` → the system writes `/uploads/hp4/a.txt` (`hello`) and `/uploads/hp4/b.txt` (`goodbye`).
2. The system answers 2 files, 12 bytes, and records one audit line.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: the archive is cut after its first protected block → `unexpected end of archive` (SPEC-0087, unchanged).
- EXC-001b: a later block is damaged → a refusal starting `archive is corrupt or not a valid rar file:` (unchanged).

### SC-002: The member mistypes the password
**Actor:** Member.
**Preconditions:** as SC-001.
**Flow:**
1. The member extracts it with `s3cret-Üni` → `ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`.
2. The member calls again with `s3cret-Ünï` → SC-001.
**Postconditions:** step 1 writes nothing, charges nothing, records no audit line.
**Exceptions:**
- EXC-002a: the archive is also cut, after the start of its first protected block → still the wrong password message.
- EXC-002b: the archive ends before the start of its first protected block is whole → `unexpected end of archive` (the password cannot be judged).
- EXC-002c: the wrong password happens to read the start as a possible block running past the end → `unexpected end of archive` (accepted limit).

### SC-003: A damaged or cut archive with the right password
**Actor:** Member.
**Preconditions:** `/uploads/hp4.rar` cut or with one byte changed.
**Flow:**
1. The member extracts it with `s3cret-Ünï` → cut anywhere: `unexpected end of archive`; damaged in the first protected block: the wrong password message; damaged in a later block: a refusal starting `archive is corrupt or not a valid rar file:`.
**Postconditions:** nothing written.

### SC-004: A wrong password meets another refusal
**Actor:** Member.
**Flow:**
1. Not a member, a missing path, a folder, a multi volume name, a server without RAR support → reported first, as today.
2. Otherwise the wrong password is reported before a cut, damage, a link or escaping entry, a destination, a collision and both limits.

## 6. Functional requirements

Definitions used below (glossary in Section 12). The **leading part** of a version 4 archive is its marker and main header: 20 bytes. The **first protected block** follows it: 8 bytes of its own that vary per archive, then the block itself, protected by the password. Its **start** is its first 24 bytes (those 8 bytes and the first 16 protected bytes): in `hp4.rar`, bytes 20 to 44. A **possible block start** is a start that, read with the given password, names one of the eight kinds of block version 4 allows after the main header (file, comment, authenticity, old service, old recovery, signature, service, end of archive) and declares a size of at least 32 bytes for a file or service block, at least 7 for the six others.

### 6.1 New

#### FR-NEW-001 [EARS-O]: A start that is not a block is a wrong password
> IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is whole AND it is not a possible block start THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete, cut or damaged.

- **Business rules:** the archive is not read any further; no file is decoded.
- **User visible text:** `incorrect password for this archive; call again with the correct 'password' parameter`
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: A whole first block that fails its check is a wrong password
> IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is a possible block start AND the whole block is present AND it fails its integrity check THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.

- **Business rules:** this includes damage inside that block with the right password: the bytes cannot tell it from a wrong password, and a member told to retype a password loses less than one told to discard a sound file.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Cut before the password can be judged
> IF a version 4 RAR archive declares encrypted file names in its whole leading part AND a password that is not empty is given AND the archive ends before the start of the first protected block is whole THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, whatever the password.

- **Business rules:** in `hp4.rar`, a length from 21 to 43 bytes. A length of 20 bytes exactly (the leading part alone) is also refused this way (SPEC-0087, unchanged).
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: A possible start cut short
> IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is a possible block start AND the archive ends before that block, at the size its start declares, is whole THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`.

- **Business rules:** with the right password this is a cut archive (SPEC-0087). With a wrong password whose start happens to be possible, the answer is the same: the accepted limit of 3.2.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: A first block that reads is the right password
> IF the first protected block of a version 4 RAR archive with encrypted file names is whole and passes its integrity check with the given password THEN THE archive extraction SHALL extract or refuse the archive exactly as before this increment.

- **Business rules:** a cut after that block: `unexpected end of archive`; a later damaged block: a refusal starting `archive is corrupt or not a valid rar file:`; damaged encrypted data: the wrong password message (unchanged, 3.2); a complete sound archive: extracted.
- **Scenario:** SC-001, SC-003
- **Priority:** Must

#### FR-NEW-006 [EARS-UB]: A refused call leaves no trace
> THE archive extraction SHALL NOT write a file, create a folder, charge the remaining storage or the session write allowance, record an audit line, or show the password in a message, a log line or an audit line, for a call refused under FR-NEW-001 to FR-NEW-004.

- **Business rules:** with `overwrite` set, an existing file at a target path keeps its content; a `destination` folder that did not exist is not created.
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-007 [EARS-U]: Both doors answer alike
> THE archive extraction SHALL give, through the HTTP API, the same code and message as through the assistant tool for every answer of this specification.

- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-008 [EARS-U]: The other password answers are unchanged
> THE archive extraction SHALL keep today's answers for no password and for the empty password on a version 4 archive with encrypted file names, for every password on a version 4 archive with readable file names, and for every version 5 archive.

- **Business rules:** no password, leading part whole: `password required to extract this archive; call again with the 'password' parameter`; empty password: `incorrect password for this archive; call again with the correct 'password' parameter`; `v4secret.rar` with `nope`: the wrong password message; `hidden.rar` (version 5) with `nope`: the wrong password message.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: Wrong password on version 4 encrypted names (modifies SPEC-0087/FR-NEW-009)
> IF a version 4 RAR archive with encrypted file names whose leading part is whole is given a password that is not empty and not the right one THEN THE archive extraction SHALL answer as FR-NEW-001 to FR-NEW-004 decide, cut or not.

- **Before:** SPEC-0087 FR-NEW-009 keeps a corrupt refusal (a message starting `archive is corrupt or not a valid rar file:`); measured: `unexpected end of archive` or `File header damaged` (2.1).
- **After:** the statement above: the wrong password message whenever the start of the first protected block is whole and the password is judged wrong.
- **Reason:** SPEC-0080 FR-NEW-003 and FR-NEW-004; BL-0136. The empty password clause of SPEC-0087 FR-NEW-009 is unchanged (FR-NEW-008).
- **Scenario:** SC-002
- **Priority:** Must

#### FR-MOD-002 [EARS-O]: Order of refusals (modifies SPEC-0087/FR-NEW-007)
> IF a refusal under FR-NEW-001 or FR-NEW-002 meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password, a wrong one (version 5, or version 4 under FR-NEW-001 or FR-NEW-002) or an empty one; (5) the truncation; (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.

- **Before:** item 4 did not name a wrong version 4 password, which fell under item 5 or a damaged archive.
- **After:** the list above; messages of the other items unchanged.
- **Reason:** SPEC-0080 FR-NEW-015 item 4 already says "the password is missing or wrong" without limiting the version.
- **Scenario:** SC-004
- **Priority:** Must

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance

- A wrong password is answered before any file of the archive is decoded; a right password takes no observable longer than today (the password is already worked out once per call today).

### 7.2 Security and access

- The password never appears in a refusal, a log line or an audit line (FR-NEW-006, E2E-030).

### 7.5 Reliability

- Repeating a refused call gives the same answer and leaves the project as it was (E2E-029).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | leading part alone (20 bytes), wrong password | `unexpected end of archive` | E2E-011 |
| Boundary values | 43 and 44 bytes (one short of the start, start exactly whole); 50 and 91 bytes (inside the first block); 108 bytes | FR-NEW-003, FR-NEW-001, FR-NEW-004 | E2E-008 to E2E-013, E2E-007 |
| Concurrent actions | not applicable: the answer reads only the archive bytes and the password | none | none |
| Slow or lost connectivity | not applicable: the archive is already stored | none | none |
| Access states | not a member with a wrong password | membership refusal first | E2E-025 |
| Data variations | accented password one letter off; 40 character password; another archive's password; 300 generated passwords | wrong password message (at least 291 of 300) | E2E-005, E2E-002, E2E-006, E2E-020 |
| Navigation | retry with the right password after a refusal | extracts | E2E-036 |
| Partial operations | refused call with `overwrite` or a new `destination` | nothing written, nothing created | E2E-028, E2E-031 |
| Dependency failures | server without RAR support | unsupported extension, unchanged (SPEC-0080 FR-NEW-012) | none new: no wrong password is read on such a server |
| Ordering and timing | wrong password plus a cut, a collision, a storage limit, a new destination | wrong password first | E2E-007, E2E-026 to E2E-028 |
| Exhaustion | storage limit below the archive's 12 bytes | wrong password first | E2E-027 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver. An implementing agent uses this as its definition of done.

**Common data.** Project `atlas`, member `alice@example.com`; unless a test says otherwise there is no storage limit, the session write allowance is large, the server has RAR support, the call goes through the assistant tool with no destination and `overwrite` false. "Nothing written" means: the default destination folder (archive path minus `.rar`) does not exist, the remaining storage and the session write allowance are unchanged, and no audit line was recorded. "The first N bytes of X" is X cut to its first N bytes. "X with byte K changed" is X with the bit 0 of its byte at offset K (counted from 0) flipped. **WRONG** is `ERR_PASSWORD_REQUIRED` with `incorrect password for this archive; call again with the correct 'password' parameter`. **CUT** is `ERR_INVALID_ARGUMENT` with `archive is corrupt or not a valid rar file: unexpected end of archive`. **CORRUPT** is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:` that is not CUT.

| Archive | Version | Content | Password | Size |
|---|---|---|---|---|
| `hp4.rar` | 4 | `a.txt` = `hello`, `b.txt` = `goodbye`, data and names encrypted | `s3cret-Ünï` | 220 bytes |
| `hp4long.rar` | 4 | `a.txt` = `hello`, data and names encrypted | `CorrectHorseBatteryStapleÜnï0123456789ab` | 132 bytes |
| `v4secret.rar` | 4 | `a.txt` = `hello`, data encrypted, names readable | `s3cret-Ünï` | 93 bytes |
| `hidden.rar` | 5 | `note.txt` = `top secret`, data and names encrypted | `s3cret-Ünï` | 254 bytes |

Offsets in `hp4.rar`: leading part 0 to 20; first protected block 20 to 92 (its own 8 bytes 20 to 28), its start 20 to 44; `a.txt`'s data 92 to 108; `b.txt`'s block 108 to 180; `b.txt`'s data 180 to 196; end of archive block 196 to 220. In `hp4long.rar` the first protected block spans 20 to 92 too. Passwords used and how `hp4.rar`'s start reads with each (P-2): `nope`, `w785`, `s3cret-Üni`: not a possible block start; `wrong135`: a file block declaring 28 935 bytes, a possible start running past the end.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-005, FR-NEW-008 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-005 | High |
| E2E-003 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-004 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-005 | New | failure | SC-002 | FR-NEW-001 | Critical |
| E2E-006 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-007 | New | failure | SC-002 | FR-NEW-001, FR-MOD-001, FR-MOD-002 | Critical |
| E2E-008 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-009 | New | edge | SC-002 | FR-NEW-001, FR-NEW-003 | High |
| E2E-010 | New | edge | SC-002 | FR-NEW-003 | High |
| E2E-011 | New | failure | SC-003 | FR-NEW-003 | Medium |
| E2E-012 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-013 | New | failure | SC-003 | FR-NEW-004 | High |
| E2E-014 | New | failure | SC-003 | FR-NEW-002, FR-MOD-001 | Critical |
| E2E-015 | New | failure | SC-002 | FR-NEW-001 | Medium |
| E2E-016 | New | failure | SC-003 | FR-NEW-005 | Critical |
| E2E-017 | New | failure | SC-003 | FR-NEW-005 | High |
| E2E-018 | New | failure | SC-003 | FR-NEW-005 | Medium |
| E2E-019 | New | edge | SC-002 | FR-NEW-004 | High |
| E2E-020 | New | edge | SC-002 | FR-NEW-001, FR-NEW-004 | High |
| E2E-021 | New | failure | SC-002 | FR-NEW-008 | Medium |
| E2E-022 | New | failure | SC-002 | FR-NEW-008 | Medium |
| E2E-023 | New | failure | SC-002 | FR-NEW-008 | Medium |
| E2E-024 | New | failure | SC-002 | FR-NEW-008 | Low |
| E2E-025 | New | failure | SC-004 | FR-MOD-002 | High |
| E2E-026 | New | failure | SC-004 | FR-MOD-002, FR-NEW-006 | High |
| E2E-027 | New | edge | SC-004 | FR-MOD-002 | High |
| E2E-028 | New | side effect | SC-004 | FR-MOD-002, FR-NEW-006 | Medium |
| E2E-029 | New | side effect | SC-002 | FR-NEW-006 | Critical |
| E2E-030 | New | side effect | SC-002 | FR-NEW-006 | High |
| E2E-031 | New | side effect | SC-004 | FR-NEW-006 | High |
| E2E-032 | New | failure | SC-002 | FR-NEW-007 | High |
| E2E-033 | New | failure | SC-003 | FR-NEW-007, FR-NEW-002 | High |
| E2E-034 | New | failure | SC-002 | FR-NEW-007, FR-NEW-003 | Medium |
| E2E-035 | New | failure | SC-003 | FR-NEW-002 | High |
| E2E-036 | New | happy | SC-002 | FR-NEW-005, FR-NEW-001 | High |
| E2E-MOD-001 | Modified | failure | SC-002 | FR-MOD-001, FR-NEW-001 | Critical |
| E2E-MOD-002 | Modified | failure | SC-003 | FR-NEW-002, FR-NEW-008 | High |

Coverage: happy 3, failure 26, side effect 4, edge 5, state transition 0 (no state: an extraction writes or is refused), security 0 (not a security lot; the password exposure check is E2E-030). Happy to failure ratio 1:8.67.

### 9.2 New tests

#### E2E-001: Right password extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-008; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`; the answer reports 2 files and 12 bytes; one audit line is recorded

#### E2E-002: A 40 character password extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp4long.rar`
- **When** the member extracts it with `CorrectHorseBatteryStapleÜnï0123456789ab`
- **Then** `/uploads/hp4long/a.txt` holds `hello`; 1 file, 5 bytes

#### E2E-003: Wrong password on the whole archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG (today CUT); nothing written

#### E2E-004: Wrong password that today reads as a damaged header
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `w785`
- **Then** the refusal is WRONG (today `archive is corrupt or not a valid rar file: File header damaged`); nothing written

#### E2E-005: Password one letter off
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Üni`
- **Then** the refusal is WRONG; nothing written

#### E2E-006: Another archive's password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4long.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is WRONG; nothing written

#### E2E-007: Wrong password on a cut archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001, FR-MOD-002; **Priority:** Critical
- **Given** `/uploads/hp4half.rar`, the first 108 bytes of `hp4.rar`
- **When** the member extracts it with `nope`, then with `w785`
- **Then** each refusal is WRONG, not CUT; nothing written

#### E2E-008: Wrong password, archive cut inside the first protected block
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4in.rar`, the first 50 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written

#### E2E-009: Start exactly whole
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4s.rar`, the first 44 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written

#### E2E-010: One byte short of the start
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4t.rar`, the first 43 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is CUT; nothing written

#### E2E-011: Leading part alone, or one byte more
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hp4l.rar`, the first 20 bytes of `hp4.rar`, then the first 21 bytes
- **When** the member extracts each with `nope`, then with `s3cret-Ünï`
- **Then** all four refusals are CUT; nothing written

#### E2E-012: Right password, cut before the start is whole
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the first 30 bytes, then the first 43 bytes of `hp4.rar`, each stored as `/uploads/hp4r.rar`
- **When** the member extracts each with `s3cret-Ünï`
- **Then** each refusal is CUT; nothing written

#### E2E-013: Right password, cut inside the first protected block
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the first 44, 50, then 91 bytes of `hp4.rar`, each stored as `/uploads/hp4r.rar`
- **When** the member extracts each with `s3cret-Ünï`
- **Then** each refusal is CUT, not WRONG; nothing written

#### E2E-014: Right password, first protected block damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4d.rar`, `hp4.rar` with byte 50 changed
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is WRONG (today `archive is corrupt or not a valid rar file: File header damaged`); nothing written

#### E2E-015: Wrong password on the damaged archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/hp4d.rar` of E2E-014
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written

#### E2E-016: Right password, a later block damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** `/uploads/hp4b.rar`, `hp4.rar` with byte 140 changed (inside `b.txt`'s block)
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CORRUPT, not WRONG; nothing written, `/uploads/hp4b/a.txt` does not exist

#### E2E-017: Right password, cut after the first file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CUT; nothing written

#### E2E-018: Right password, cut inside the second file's data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/hp4data.rar`, the first 190 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CUT; nothing written

#### E2E-019: A wrong password that reads as a possible block (accepted limit)
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `wrong135`
- **Then** the refusal is CUT; nothing written

#### E2E-020: Three hundred wrong passwords
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-004; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with each of `wrong0` to `wrong299`
- **Then** every refusal is WRONG or CUT, none is a success or another message; at least 291 are WRONG; `wrong135` is CUT; nothing written after any of them

#### E2E-021: No password, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/hp4.rar`, and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each with no password
- **Then** each refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; nothing written

#### E2E-022: Empty password, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/hp4.rar`, and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each with the empty password
- **Then** each refusal is WRONG; nothing written

#### E2E-023: Readable names, encrypted data, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/v4secret.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written

#### E2E-024: Version 5, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Low
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written

#### E2E-025: Not a member comes first
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/hp4.rar` in `atlas`; `bob@example.com` is not a member of `atlas`
- **When** `bob@example.com` extracts it with `nope`
- **Then** the refusal is `ERR_FORBIDDEN` with `'bob@example.com' is not a member of 'atlas'`; nothing written

#### E2E-026: Wrong password before a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-MOD-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar` and an existing `/uploads/hp4/a.txt` holding `old`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG, not the collision; `/uploads/hp4/a.txt` still holds `old`; `/uploads/hp4/b.txt` does not exist

#### E2E-027: Wrong password before the storage limit
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/hp4.rar` and a remaining storage of 5 bytes (below the 12 declared)
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG, not the storage limit; remaining storage still 5 bytes; nothing written

#### E2E-028: Wrong password with a new destination
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-MOD-002, FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/hp4.rar`; `/out` does not exist
- **When** the member extracts it with `nope` and the destination `/out`
- **Then** the refusal is WRONG; `/out` does not exist

#### E2E-029: A refused wrong password leaves nothing, twice
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** `/uploads/hp4.rar`; the remaining storage and session write allowance are noted
- **When** the member extracts it with `nope`, twice
- **Then** both refusals are WRONG; `/uploads/hp4` does not exist; the remaining storage and session write allowance are unchanged; no audit line names `/uploads/hp4.rar`

#### E2E-030: The wrong password is never recorded
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Üni`
- **Then** the refusal is WRONG; `s3cret-Üni` appears in no message, no log line and no audit line

#### E2E-031: Overwrite set, refused, nothing replaced
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar` and `/uploads/hp4/a.txt` holding `old`
- **When** the member extracts it with `nope` and `overwrite` set
- **Then** the refusal is WRONG; `/uploads/hp4/a.txt` still holds `old`

#### E2E-032: HTTP API, wrong password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it through the HTTP API with `nope`
- **Then** the refusal is WRONG, the same code and message as E2E-003; nothing written

#### E2E-033: HTTP API, damaged first block and a cut, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-002; **Priority:** High
- **Given** `/uploads/hp4d.rar` of E2E-014 and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each through the HTTP API with `s3cret-Ünï`
- **Then** the first refusal is WRONG, the second CUT, as through the assistant tool; nothing written

#### E2E-034: HTTP API, cut before the start
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-007, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hp4t.rar` of E2E-010
- **When** the member extracts it through the HTTP API with `nope`
- **Then** the refusal is CUT; nothing written

#### E2E-035: Right password, first block of another archive damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/hp4longd.rar`, `hp4long.rar` with byte 50 changed
- **When** the member extracts it with `CorrectHorseBatteryStapleÜnï0123456789ab`
- **Then** the refusal is WRONG; nothing written

#### E2E-036: Retry with the right password after a refusal
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4.rar`, just refused with `s3cret-Üni` (WRONG)
- **When** the member extracts it again with `s3cret-Ünï`
- **Then** `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`; exactly one audit line names `/uploads/hp4.rar`

### 9.4 Modified tests

#### E2E-MOD-001: Version 4, encrypted names, wrong password (was SPEC-0087 E2E-027)
- **Validated before:** with `nope` and `w785`, on the first 108 bytes of `hp4.rar` and on the whole archive, a refusal starting `archive is corrupt or not a valid rar file:`.
- **Validates now:** the same four calls answer WRONG.
- **Given** `/uploads/hp4half.rar` (first 108 bytes) and `/uploads/hp4.rar`
- **When** the member extracts each with `nope`, then with `w785`
- **Then** each of the four refusals is WRONG; nothing written

#### E2E-MOD-002: A damaged first block with the right password (was the internal check that leaves it to the reading library, 14.1 row 8)
- **Validated before:** the first 108 bytes of `hp4.rar` with byte 50 changed, right password, pass the completeness check and are left to the reading library; the same cut with no password or the empty password pass it too.
- **Validates now:** that damaged cut, right password: WRONG. With no password and with the empty password: unchanged (needed, and WRONG as today).
- **Given** the first 108 bytes of `hp4.rar` with byte 50 changed, as `/uploads/hp4dc.rar`
- **When** the member extracts it with `s3cret-Ünï`, then with no password, then with the empty password
- **Then** WRONG; then `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; then WRONG; nothing written

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-005, FR-NEW-008 | E2E-001, E2E-002 | E2E-016, E2E-017, E2E-018 | E2E-013 |
| SC-002 | FR-NEW-001 to FR-NEW-004, FR-NEW-006 to FR-NEW-008, FR-MOD-001 | E2E-036 | E2E-003 to E2E-008, E2E-015, E2E-021 to E2E-024, E2E-032, E2E-034, E2E-MOD-001 | E2E-009, E2E-010, E2E-019, E2E-020 |
| SC-003 | FR-NEW-002 to FR-NEW-005, FR-NEW-007 | E2E-001 | E2E-011 to E2E-014, E2E-016 to E2E-018, E2E-033, E2E-035, E2E-MOD-002 | E2E-013 |
| SC-004 | FR-MOD-002, FR-NEW-006 | E2E-001 | E2E-025, E2E-026 | E2E-027 (5 bytes against 12) |

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-003 to E2E-009, E2E-015, E2E-020, E2E-036, E2E-MOD-001 |
| FR-NEW-002 | E2E-014, E2E-033, E2E-035, E2E-MOD-002 |
| FR-NEW-003 | E2E-009 to E2E-012, E2E-034 |
| FR-NEW-004 | E2E-013, E2E-019, E2E-020 |
| FR-NEW-005 | E2E-001, E2E-002, E2E-016 to E2E-018, E2E-036 |
| FR-NEW-006 | E2E-026, E2E-028 to E2E-031 |
| FR-NEW-007 | E2E-032 to E2E-034 |
| FR-NEW-008 | E2E-001, E2E-021 to E2E-024, E2E-MOD-002 |
| FR-MOD-001 | E2E-003, E2E-004, E2E-007, E2E-014, E2E-MOD-001 |
| FR-MOD-002 | E2E-007, E2E-025 to E2E-028 |

## 11. Open questions

- The test designer, the self consistency reviewer and the auditor were the author (subagents not allowed in this run, Section 13): does not change observable behavior.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| RAR version 4, version 5 | the two RAR formats the extraction reads; version 4 stores no value to check a password against | archive extraction |
| Encrypted file names | file names and contents protected by the password; listing the archive needs it | archive extraction |
| Encrypted data | contents protected, names readable | archive extraction |
| Leading part | the marker and main header of a version 4 archive, 20 bytes; it says whether the names are encrypted | archive extraction |
| First protected block | the first block after the leading part, preceded by 8 bytes of its own and readable only with the password | archive extraction |
| Start (of the first protected block) | its first 24 bytes: its own 8 bytes and the first 16 protected bytes | archive extraction |
| Possible block start | a start that, read with the given password, names one of the eight block kinds version 4 allows after the main header with at least that kind's minimum size (Section 6) | archive extraction |
| Integrity check | the check every version 4 block carries over its own bytes | archive extraction |
| Wrong password | a password that is not empty and is not the one the archive was made with | archive extraction |
| Truncated archive, cut | an archive missing its last bytes (SPEC-0087) | archive extraction |
| Damaged archive | an archive whose length is whole but some byte is changed | archive extraction |
| Order of refusals | which refusal a member sees when several apply (FR-MOD-002) | archive extraction |
| Member | a person with access to the project | archive extraction |
| Operator | runs the server, reads its logs and audit trail | archive extraction |
| Remaining storage | what the project's storage limit still allows | project storage |
| Session write allowance | what the member can still write in their session | project storage |
| Audit line | the record of one successful extraction | project storage |
| HTTP API, assistant tool | the two doors through which a member extracts | archive extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 2 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Round 1 findings and amendments:** F-1 (G1) E2E-030 used `nope-24680`, a password whose reading of the start was never measured, so WRONG or CUT were both possible: now `s3cret-Üni` (P-2: not a possible start). F-2 (G1) E2E-022 gave the empty password on the first 44 bytes, never measured: now the first 108 bytes (SPEC-0087 P-3, E2E-055). A-1 nine citations in 14.1 and 14.3 off by a few lines (SPEC-0087 FR-NEW-007 and FR-NEW-009, `archive_rar.rs` size read, RAR5 check value, `RAR4_SALT`, `TRUNCATED`, key pins, E2E-065, the test helpers): corrected after reopening each. A-2 SC-004 had no edge category test: E2E-027 (5 bytes against 12, a boundary) recategorized as edge, counts recomputed. The word salt removed from Sections 1 to 13.
**How the gates ran:** subagents were not allowed in this run, so the test design (8.1), the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks listed in 14.4 D-11.
**No implementation detail outside Section 14:** PASS. Error codes, verbatim messages, option names (`overwrite`, `destination`, password), file names, byte counts, offsets and block kinds of the test archives are WHAT: what the member stores, types and sees, and what the RAR format itself defines.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim | Evidence |
|---|---|---|
| 1 | right password whole extracts; every cut after the leading part is CUT | probe P-1; `crates/core/src/tools/archive_rar.rs:32` (walk first), `:287-335` (`walk_rar4_encrypted`); `crates/core/src/tools/archive_rar_cut_e2e.rs:445` (E2E-022), E2E-034 at `:577-624` |
| 2 | no password: needed; empty: incorrect | probe P-1; `archive_rar.rs:289-291` (empty or none: walk stops, the library answers), `:474-481` (`listing_error`: `MissingPassword`/`BadPassword` to `password_required_message`); `crates/core/src/tools/archive.rs:1005-1015` (needed when `None`, incorrect otherwise) |
| 3 | wrong password: CUT or `File header damaged` | probes P-1, P-3; `archive_rar.rs:311-314` (the size read from the first decrypted block, then `rest.get(..padded)` refused as truncated when it runs past the end), `:324-326` (CRC16 fails: `Ok`, left to UnRAR, which answers `BadData`, mapped by `listing_error` to corrupt with the crate's text `File header damaged`, `unrar-0.5.8/src/error.rs:90`) |
| 4 | damage with the right password | probe P-4 |
| 5 | readable names or version 5: wrong password message | probe P-1; `archive_rar.rs:91-111` (data read error with an encrypted entry), `:399-401` (RAR5 check value) |
| 6 | no trace on refusal | SPEC-0087 FR-NEW-006 and its tests; `crates/core/src/tools/archive_rar_cut_e2e.rs:35-54` (`assert_refused_and_nothing_written`) |
| 7 | governing specs | `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md:188-196` (FR-NEW-003, FR-NEW-004), `:258-261` (FR-NEW-015), `:289-290` (FR-NEW-020); `sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/sdlc-spec.md:178-182` (FR-NEW-007), `:191-196` (FR-NEW-009), `:494-498` (E2E-027) |
| 8 | tests today | `archive_rar_cut_e2e.rs:540-549` (`e2e_0087_027_version_4_encrypted_names_cut_wrong`: `nope`, `w785`, `corrupt_any`), `:528-530` (E2E-025), `:535` (E2E-026), `:667-669` (E2E-055); `archive_rar.rs:1750-1767` (`rar4_encrypted_headers_left_to_the_library`, `garbled[50] ^= 1` at `:1764` passes the walk), `:1442-1463` (SPEC-0080 E2E-065, `v4secret.rar`), `:1744-1748` and `:1774-1777` (key derivation pins on `hp4.rar`, `hp4long.rar`); 52 tests in `archive_rar_cut_e2e.rs`, 83 in `archive_rar.rs` (`rg -c '#\[(tokio::)?test'`); `.agent_docs/tools.md:231-232` ("still reads as corrupt (BL-0136)") |

Test command: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` (`test.sh:7`); full gate `./test.sh`.

**Probes (2026-11-11, throwaway test module in the worktree, deleted before the commit, `cargo test -p mcp-fs-core --lib --features rar -- probe_0090 --nocapture`).**

- **P-1 outcomes through `extract_archive`:** `hp4.rar` none: `ERR_PASSWORD_REQUIRED` needed; empty: `ERR_PASSWORD_REQUIRED` incorrect; `nope`: `ERR_INVALID_ARGUMENT` `... unexpected end of archive`; `w785`: `ERR_INVALID_ARGUMENT` `... File header damaged`; `s3cret-Üni`: `... unexpected end of archive`; right: OK, 2 files, 12 bytes. First 108 bytes: `nope` CUT, `w785` `File header damaged`, right CUT. First 50, 28, 21, 20, 92 bytes with `nope`: CUT; first 50 right: CUT. `hp4long.rar` none: needed; empty: incorrect; `nope` and `s3cret-Ünï`: CUT; right: OK 1 file 5 bytes. `v4secret.rar` none: needed; empty and `nope`: incorrect; right: OK. `hidden.rar` `nope`: incorrect.
- **P-2 library and first block:** `unrar` 0.5.8 listing (`Cargo.lock` locked version): `hp4.rar` and `hp4long.rar` with none or empty: `MissingPassword`; with `nope`, `w785`, or `hp4long.rar` with `hp4.rar`'s password: `BadData`; right: the names. `v4secret.rar` lists `a.txt` with any password. UnRAR itself says, for an encrypted header failing its checksum, "Checksum error in the encrypted file %s. Corrupt file or wrong password." (`unrar_sys-0.5.8/vendor/unrar/loclang.hpp:221` `MEncrBadCRC`, raised at `arcread.cpp:516-541`): the library cannot tell either. First 16 protected bytes of `hp4.rar` decrypted: right: type `0x74`, flags `0x9424`, size 50; `nope`: type `0x63`, size 59 100; `w785`: type `0x07`, size 42; `s3cret-Üni`: type `0xda`, size 19 256; empty: type `0x7d`, size 48 620. Right key, whole first block (64 bytes): bytes 50 to 63 are zero; the `b.txt` block and the end block (`c4 3d 7b 00 40 07 00`) also end in zero padding.
- **P-3 300 generated passwords `wrong0` to `wrong299` on whole `hp4.rar`:** today all 300 CUT (the walk); decrypted type in `0x72` to `0x7b`: 9 of 300; a size that fits in the 192 bytes after the salt: 0 of 300. Over `wrong0` to `wrong2999`, a file kind (`0x74`) with at least 32 bytes: 13 of 3 000, among them `wrong135` (28 935 bytes). Hence the floor of 291 in E2E-020 (300 minus the 9 with a type in the range, a superset of the possible starts).
- **P-4 damage, right password:** byte 50 changed: `File header damaged`; byte 140: `File header damaged`; byte 100 (inside `a.txt`'s encrypted data): `ERR_PASSWORD_REQUIRED` incorrect. With `nope`, all three: CUT.

### 14.3 Design inputs

- **Where today's answer comes from.** `walk_rar4_encrypted` (`archive_rar.rs:287-335`) reads the size from the first decrypted AES block and refuses as truncated when it runs past the end (`:311-314`), so most wrong passwords read CUT; a failing CRC16 returns `Ok` (`:324-326`) and UnRAR answers `BadData`, corrupt (`listing_error`, `:474-481`). Both branches change for the first protected block only: later blocks keep today's handling (FR-NEW-005). **Signal:** E2E-003, E2E-004, E2E-014, E2E-016 fail.
- **Possible block start.** Kinds `0x74` to `0x7b` (`unrar_sys-0.5.8/vendor/unrar/headers.hpp:81-83`); minimum size 32 for file `0x74` and service `0x7a` (`SIZEOF_FILEHEAD3`, `headers.hpp:8`), 7 for the others (`SIZEOF_SHORTBLOCKHEAD`, `:9`). The rule is normative (Section 6): a stricter check (flags, zero padding) would change E2E-019's answer and is a spec change, not a design choice. UnRAR does not check the CRC of `0x76` and `0x79` (`arcread.cpp:516-518`); RAR 6.24 writes a file or service block first, so this is not reached by the fixtures. **Signal:** E2E-019, E2E-020.
- **The start is 24 bytes.** 8 byte salt (`RAR4_SALT`, `archive_rar.rs:276`) plus one AES block (`AES_BLOCK`); fewer present: CUT whatever the password (FR-NEW-003). **Signal:** E2E-009, E2E-010.
- **Messages.** WRONG is `password_required_message(Some(_))` (`archive.rs:1005-1015`); CUT is `corrupt("rar", TRUNCATED)` (`archive_rar.rs:140`, `archive.rs:626-630`). The empty password keeps reaching UnRAR (`:289-291`, FR-NEW-008).
- **Tests that change.** `e2e_0087_027_version_4_encrypted_names_cut_wrong` (`archive_rar_cut_e2e.rs:540-549`) becomes E2E-MOD-001 (`corrupt_any` to `password_incorrect`), and its comment about `w785` "the branch that must leave the answer to UnRAR (BL-0136)" goes. `rar4_encrypted_headers_left_to_the_library` (`archive_rar.rs:1750-1767`): its `garbled` assertion (`:1764-1765`) becomes a refusal with the wrong password message (E2E-MOD-002); the `None` and `Some("")` assertions stay; rename it. The doc comment of `walk_rar4_encrypted` (`:278-286`, "left to UnRAR, so its answer stays today's (BL-0136)") and `.agent_docs/tools.md:231-232` must be rewritten. SPEC-0087 E2E-055 (`:667-669`) stays as is.
- **HTTP door.** SPEC-0087's REST tests (`crates/core/src/api/dataplane.rs:3664-3712`, `assert_cut_refused`) are the model for E2E-032 to E2E-034. **Signal:** those tests.
- **Run time.** E2E-020 pays 300 RAR 3 key derivations (0x40000 SHA-1 rounds each) in a debug build; probe P-3 ran 300 walks plus other probes in 144 s. The design may drive E2E-020 through the completeness check rather than full extractions, provided `wrong135`, `nope` and two others also go through the assistant tool, and must say so.
- **Fixtures.** No new archive: `hp4.rar`, `hp4long.rar`, `v4secret.rar`, `hidden.rar` are committed (`crates/core/tests/fixtures/rar/README.md:25-26` and earlier rows); damaged and cut variants are made at run time.

### 14.4 Defaults applied (the user authorized every decision; one line each)

- D-1 routing: the entry says `Nature: BUG` and an authority is violated (SPEC-0080 FR-NEW-003, FR-NEW-004, FR-NEW-015 item 4). The prompt hands a BUG to `/sdlc-analysis`; the user asked for a spec, as for SPEC-0084, SPEC-0087, SPEC-0088 and SPEC-0089, so it is written as a FEAT: the observable answer changes. Nature: FEAT.
- D-2 the password is judged on the first protected block only; once it reads, later damage is corrupt (no new password signal past it).
- D-3 a whole first block failing its check is a wrong password, damage included (BL-0136 notes, the lesser evil).
- D-4 a start that is not a possible block start is a wrong password even when the archive is cut later or inside that block: the member fixes the password first, then sees the cut (SPEC-0080 FR-NEW-015 item 4 before 5).
- D-5 fewer than 24 bytes after the leading part: CUT whatever the password (nothing to judge).
- D-6 a possible start running past the end: CUT, accepted limit, pinned by E2E-019 with `wrong135`; the possible start rule is normative so the answer is the same in every implementation.
- D-7 damaged encrypted data with the right password stays the wrong password message (out of scope, side finding).
- D-8 no password, empty password, readable names and version 5 unchanged.
- D-9 slug `tell-a-wrong-password-on-rar4-encrypted-names`, from the entry's title; id SPEC-0090 pre assigned by the user (parallel runs on SPEC-0091 to SPEC-0093).
- D-10 tests: 36 new, 2 modified (SPEC-0087 E2E-027, the internal check `rar4_encrypted_headers_left_to_the_library`), 0 removed. Test plan approved by the instruction "specify BL-0136".
- D-11 gates inline (no subagents allowed): scripted checks on the file: every E2E id in 9.1 has one specification in 9.2 or 9.4 with the same category, scenario and requirements; each requirement has at least 3 tests; category counts recomputed; no `should`, `may`, `could`, `might`, `would` in an EARS line; Sections 1 to 13 scanned for paths, crate, function and field names.
- D-12 interview rounds answered by the author, one line each. R1 scope: a wrong password on version 4 encrypted names gets the wrong password message; out: damaged data, version 5, no or empty password, other formats. R1 why now: BL-0136, a mistyped password reads as a cut or damaged file. R2: Section 2, probes P-1 to P-4. R3: SC-001 to SC-004. R4: Section 6. R5: no extra decode, password never shown, nothing written on refusal.
- D-13 depth M: one module, 10 requirements, two requirements of SPEC-0087 modified, no persisted change.
