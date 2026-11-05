# Extract RAR archives: functional specification

> Id: SPEC-0080
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction), about 20 anticipated requirements, public contract change (a refusal becomes a success, two refusal messages change), no persisted change. Override with --depth.
> Status: designed
> Generated on: 2026-11-04
> Project: existing
> From backlog: BL-0110
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

A project member can extract a `.rar` archive stored in their project, exactly as they can already
extract zip, 7z and tar archives: same destination rules, same all or nothing behavior, same refusals,
same password retry. An encrypted RAR is opened by calling the extraction again with the archive's
password, and the refusal message now tells the user to do exactly that. The same wording is given to the
existing zip and 7z password refusals so the three formats guide the user the same way. RAR support is
shipped as an optional capability of the server: a server delivered without it behaves as today.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member can extract `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz`
  archives in place, from the assistant tool and from the HTTP API. The format is read from the file name
  alone, ignoring letter case. [14.1 row 1]
- A `.rar` archive is refused with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '.rar'`.
  [row 2]
- Before any file is written, the whole archive is read and checked; any problem rejects the whole call
  and nothing is written. The checks refuse: a link or device entry (`ERR_NOT_SUPPORTED`), an entry whose
  resolved path lies outside the destination (`ERR_PATH_OUT_OF_BOUNDS`), an existing file when `overwrite` is false
  (`ERR_NO_CLOBBER`, first collision in archive order), an entry that decodes to more bytes than it
  declares (`ERR_INVALID_ARGUMENT`), and a declared total above a limit: the project's storage
  limit on net growth (`ERR_PROJECT_QUOTA_EXCEEDED`) or the member's session write allowance on the gross total
  (`ERR_WRITE_QUOTA_EXCEEDED`). [row 3]
- An encrypted zip or 7z archive with no password, or a wrong one, is refused with
  `ERR_PASSWORD_REQUIRED`; the user calls again with the `password` parameter. The two messages are
  `password required to extract this archive` and `incorrect password for this archive`. They do not say
  how to retry. [row 4]
- A corrupt archive is refused with `ERR_INVALID_ARGUMENT` and a message starting
  `archive is corrupt or not a valid <format> file:`. [row 5]
- On success the user receives the destination, the number of files written, the number of folders
  created and the number of bytes written, and the operator sees one audit line that never contains the
  password. [row 6]

### 2.2 Specs governing this area

- SPEC-0018 (archive extract in place) defines the behavior above. Its Summary states "`rar` is out of
  scope entirely, per an explicit user decision" and its unsupported extension rule (FR-NEW-006) names
  `.rar` as refused. This increment deviates from both: RAR becomes supported. SPEC-0018 is not modified.
- SPEC-0041 (refuse inconsistent zips) [row 15] FR-NEW-003 and the tests that quote the old missing password message are updated by FR-MOD-002; its rule that the password is asked before the structural refusal is untouched.
- SPEC-0020 (project storage quota) governs the project storage limit the extraction reuses; the session write allowance predates it (SPEC-0018).

### 2.3 What is verified today

- The refusal of `.rar` is pinned by one test (SPEC-0018 E2E-NEW-022). The refusal of a bare `.gz` is
  pinned by another (E2E-NEW-034) and is not touched here.
- The password refusal for zip is verified by wording prefix only ("password required", "incorrect
  password"); for 7z two tests pin the exact messages (missing and wrong password). [row 7]
- No test can build a RAR archive today: none exists in the repository. [row 8]

## 3. Scope

### 3.1 In scope

- Extraction of `.rar` archives (RAR versions 4 and 5, solid or not), file name matched ignoring letter case.
- Encrypted RAR archives, including those whose file names are also encrypted, through the existing
  `password` parameter, on both doors (assistant tool and HTTP API).
- A clearer password refusal message, shared by RAR, zip and 7z.
- RAR support as an optional capability of the delivered server.

### 3.2 Out of scope, and why

- Multi volume RAR archives (`name.part1.rar`, `name.part01.rar`): the tool receives one stored file, not a
  set of files; refused with a clear message. Deferred as BL-0123.
- Other RAR related names (`.cbr`, `.r00`): not recognized, to keep the rule "the last extension decides".
- Creating RAR archives: nothing in the product creates archives of any kind.
- Bare compressed files (`.gz`, `.xz`, `.bz2` without tar): BL-0111 to BL-0113.

### 3.3 Constraints imposed by the context

- The only available RAR reader is covered by a non free licence imposed by its holder (RARLAB): it permits reading RAR archives and forbids creating them or rebuilding the compression. For that reason RAR
  support is optional in the delivered server, and a server delivered without it keeps today's behavior.
  Imposed by: the licence holder.

## 4. Actors

- **Member:** a person with access to a project, extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server; reads the audit trail and the logs; decides whether RAR support is delivered.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | reading a stored archive and materializing its entries in the project | archive, entry, destination, password, refusal |
| Project storage | the files, the storage limit and the session write allowance of a project | storage limit, session write allowance, collision, overwrite |

## 5. Usage scenarios

### SC-001: Extract a plain RAR
**Actor:** Member.
**Preconditions:** the project holds `/uploads/report.rar` containing the folder `data` and the files
`data/a.txt` (5 bytes) and `data/b.txt` (7 bytes). The server has RAR support. Nothing exists at `/uploads/report`.
**Flow:**
1. The member asks to extract `/uploads/report.rar` → the system reads the whole archive, checks it, writes the entries under `/uploads/report`.
2. The system answers the destination `/uploads/report`, 2 files written, 2 folders created (`report` and `report/data`), 12 bytes written.
**Postconditions:** `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` exist with the archive's content; one audit line records the extraction.
**Exceptions:**
- EXC-001a: the archive path does not exist → `ERR_NOT_FOUND`, nothing written.
- EXC-001b: the path is a folder → `ERR_INVALID_ARGUMENT` with the message `'<path>' is a directory, not an archive file`, nothing written.
- EXC-001c: the caller is not a member of the project → `ERR_FORBIDDEN` with the message `'<person>' is not a member of '<project>'`, nothing written.
**Interactions:** SC-003 (refusals), SC-002 (encryption).

### SC-002: Extract an encrypted RAR
**Actor:** Member.
**Preconditions:** `/uploads/secret.rar` is encrypted with the password `s3cret-Ünï`. A second archive `/uploads/hidden.rar` has its file names encrypted too.
**Flow:**
1. The member asks to extract `/uploads/secret.rar` without a password → the system refuses with `ERR_PASSWORD_REQUIRED` and the message `password required to extract this archive; call again with the 'password' parameter`. Nothing is written.
2. The member asks again with the password `s3cret-Ünï` → the system extracts as in SC-001.
**Postconditions:** after step 2 the entries exist; after step 1 nothing changed.
**Exceptions:**
- EXC-002a: wrong password `nope` at step 2 → `ERR_PASSWORD_REQUIRED` with the message `incorrect password for this archive; call again with the correct 'password' parameter`; nothing written.
- EXC-002b: `/uploads/hidden.rar` without a password → same refusal as step 1 (the file names cannot be read without the password); with a wrong password → same as EXC-002a.
- EXC-002c: an archive where `a.txt` is plain and `b.txt` is encrypted, no password → same refusal as step 1 and `a.txt` is not written either; an empty password on an encrypted archive → same as EXC-002a.
**Interactions:** SC-001.

### SC-003: A RAR that must be refused
**Actor:** Member.
**Preconditions:** a RAR archive that holds a problem entry.
**Flow:**
1. The member asks to extract it → the system reads and checks the whole archive first.
2. The system refuses with the same code and message family as for other formats; nothing is written, not even the valid entries that precede the problem entry.
**Postconditions:** the project is unchanged and the remaining storage and the session write allowance are unchanged.
**Order when several problems apply:** FR-NEW-015.
**Exceptions:**
- EXC-003a: a symbolic link, hard link or device entry → `ERR_NOT_SUPPORTED`, the message names the entry.
- EXC-003b: an entry named `../../etc/passwd`, or an absolute name such as `/etc/cron.d/x` → `ERR_PATH_OUT_OF_BOUNDS`.
- EXC-003c: an entry that decodes to more bytes than it declares → `ERR_INVALID_ARGUMENT`, the message names the entry.
- EXC-003d: a file already exists at an entry's destination and `overwrite` is false → `ERR_NO_CLOBBER` naming the first colliding path in archive order.
- EXC-003e: the net growth is above the project's remaining storage → `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`.
- EXC-003f: the declared total is above the member's remaining session write allowance → `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of <N> bytes exceeded`; a destination listed twice counts twice.
**Interactions:** SC-001.

### SC-004: A multi volume RAR
**Actor:** Member. Applies to a server with RAR support; on a server without it SC-006 applies.
**Preconditions:** the project holds `/uploads/big.part1.rar`.
**Flow:**
1. The member asks to extract it → the system refuses with `ERR_NOT_SUPPORTED` and the message `multi volume RAR archives are not supported: extract a single file archive`.
**Postconditions:** nothing written.
**Exceptions:** none. `report.part1x.rar` and `big.part.rar` are not multi volume names and extract as ordinary RAR files (E2E-024).

### SC-005: A corrupt or fake RAR
**Actor:** Member.
**Preconditions:** `/uploads/broken.rar` is a RAR cut in half, `/uploads/fake.rar` is a text file, `/uploads/empty.rar` has 0 bytes.
**Flow:**
1. The member asks to extract one of them → the system refuses with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid rar file:`.
**Postconditions:** nothing written.

### SC-006: A server without RAR support
**Actor:** Member; Operator delivered the server without RAR support.
**Preconditions:** `/uploads/report.rar` exists.
**Flow:**
1. The member asks to extract it → the system refuses with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '.rar'` (the extension is shown as typed: `'.RAR'` for `report.RAR`). The same holds for `big.part1.rar`.
**Postconditions:** nothing written; every other format still extracts.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Extract a RAR archive
> WHEN a member asks to extract a file whose name ends in `.rar`, in any letter case, THE archive extraction SHALL extract it by the rules that apply to every other supported format: default destination is the archive path without its suffix, `destination` and `overwrite` are honoured, the answer carries the destination, files written, folders created and bytes written, and one audit line is recorded.
- **Business rules:** versions 4 and 5 and solid archives are supported; a RAR holding no entry extracts to an empty folder (0 files, 1 folder created).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Password needed
> IF a RAR archive is encrypted and no password is given THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and the message `password required to extract this archive; call again with the 'password' parameter`, and write nothing.
- **User visible text:** the message above, verbatim.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Wrong password
> IF a RAR archive is encrypted and the given password is wrong THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and the message `incorrect password for this archive; call again with the correct 'password' parameter`, and write nothing.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Encrypted file names
> IF a RAR archive has its file names encrypted THEN THE archive extraction SHALL treat the password exactly as for an archive with encrypted data: FR-NEW-002, FR-NEW-003 and FR-NEW-005 apply unchanged.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-E]: Right password
> WHEN a member gives the right password for an encrypted RAR archive THE archive extraction SHALL extract it as FR-NEW-001 describes.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: Link and device entries
> IF a RAR archive holds a symbolic link, hard link or device entry THEN THE archive extraction SHALL refuse the whole archive with `ERR_NOT_SUPPORTED`, name the offending entry in the message, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Entries escaping the destination
> IF a RAR entry name, once resolved, lies outside the destination, whether through `..` or an absolute name, THEN THE archive extraction SHALL refuse the whole archive with `ERR_PATH_OUT_OF_BOUNDS` and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-008 [EARS-O]: Decoded larger than declared
> IF a RAR entry decodes to more bytes than it declares THEN THE archive extraction SHALL refuse the whole archive with `ERR_INVALID_ARGUMENT`, name the offending entry in the message, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Collision
> IF an entry targets a path that already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` naming the first colliding path in archive order, and write nothing.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-010 [EARS-O]: Multi volume names
> IF the server has RAR support AND the file name, ignoring letter case, ends in `.part` followed by one or more digits followed by `.rar` THEN THE archive extraction SHALL refuse with `ERR_NOT_SUPPORTED` and the message `multi volume RAR archives are not supported: extract a single file archive`, and write nothing.
- **Business rules:** `big.part1.rar`, `big.part01.rar`, `big.PART2.RAR` are refused; `big.part.rar`, `big.part1x.rar` are not multi volume names and are read as ordinary RAR files.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-011 [EARS-O]: Corrupt archive
> IF a file named `.rar` is empty, is not a RAR archive or is truncated THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid rar file:`, and write nothing.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-012 [EARS-O]: Server without RAR support
> IF the server is delivered without RAR support AND the file name, ignoring letter case, ends in `.rar` THEN THE archive extraction SHALL refuse with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '<extension as typed>'`, and write nothing; this includes multi volume names.
- **Business rules:** every other format extracts unchanged.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-013 [EARS-UB]: Password never recorded
> THE archive extraction SHALL NOT write the password into any log line or audit line, on success or on failure.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-014 [EARS-O]: Project storage limit
> IF the project has a storage limit AND the net growth of the project's stored bytes caused by the extraction exceeds the remaining storage THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- **Business rules:** net growth is the sum of the declared sizes of the files to write minus the sizes of the files they replace when `overwrite` is true; a net growth equal to the remaining storage is accepted; when an archive lists the same destination more than once only the last occurrence counts toward net growth; the rule is the one zip, 7z and tar already follow; a project without a storage limit never triggers this refusal.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-022 [EARS-O]: Session write allowance
> IF the declared total of the files to write exceeds the member's remaining session write allowance THEN THE archive extraction SHALL refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.
- **Business rules:** on success the allowance is charged the full declared total, with no netting for replaced files; the project storage refusal (FR-NEW-014) is checked first, and the allowance is charged only after it passed; a declared total equal to the remaining allowance is accepted; when an archive lists the same destination more than once, every occurrence counts toward the declared total (gross, unlike FR-NEW-014).
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-015 [EARS-O]: Order of refusals
> IF several refusals apply to the same RAR archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (FR-NEW-018); (2) the path is missing (FR-NEW-019) or is a folder (FR-NEW-021); (3) multi volume name (FR-NEW-010) or, on a server without RAR support, FR-NEW-012; (4) the file names are encrypted and the password is missing or wrong (FR-NEW-002, FR-NEW-003, FR-NEW-020), including when the archive is also truncated; (5) corrupt (FR-NEW-011); (6) the first offending entry in archive order, a link or device entry (FR-NEW-006) or an escaping name (FR-NEW-007); (7) the data is encrypted and the password is missing or wrong (FR-NEW-002, FR-NEW-003, FR-NEW-016, FR-NEW-020); (8) decoded larger than declared (FR-NEW-008); (9) a `destination` that lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (10) collision (FR-NEW-009); (11) project storage limit (FR-NEW-014); (12) session write allowance (FR-NEW-022).
- **Why this order:** membership and the path checks come first for every format. Zip also reports a visible link or escaping name before it asks for the password; 7z asks for the password first and checks entries while decoding. For RAR this order is new, chosen so that a visible problem is reported before any password work, and so that a member who is not allowed learns nothing about the project's files.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-016 [EARS-O]: Partial encryption
> IF at least one entry of a RAR archive is encrypted AND no password is given THEN THE archive extraction SHALL apply FR-NEW-002 to the whole archive and write nothing, not even the plain entries.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-017 [EARS-E]: Password given for a plain archive
> WHEN a member gives a password, empty or not, for a RAR archive in which no entry is encrypted THE archive extraction SHALL ignore it and extract as FR-NEW-001 describes.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-018 [EARS-O]: Membership
> IF the caller is not a member of the project, including a platform administrator without membership, THEN THE archive extraction SHALL refuse with `ERR_FORBIDDEN` and the message `'<person>' is not a member of '<project>'`, and write nothing.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-019 [EARS-O]: Missing archive
> IF the path does not exist THEN THE archive extraction SHALL refuse with `ERR_NOT_FOUND` and the message `'<path>' not found`, and write nothing.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-021 [EARS-O]: Folder instead of archive
> IF the path is a folder THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a directory, not an archive file`, and write nothing.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-020 [EARS-O]: Empty password
> IF an empty password is given for an encrypted RAR archive THEN THE archive extraction SHALL refuse as FR-NEW-003 describes.
- **Scenario:** SC-002
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: `.rar` is no longer refused (modifies SPEC-0018/FR-NEW-006)
> IF the server has RAR support THEN THE archive extraction SHALL treat `.rar` as a supported extension.
- **Before:** every `.rar` is refused with `unsupported archive extension: '.rar'`.
- **After:** the statement above; FR-NEW-012 keeps the old behavior for a server without RAR support.
- **Reason:** BL-0110.

#### FR-MOD-002 [EARS-O]: Password needed wording for zip and 7z (modifies SPEC-0018/FR-NEW-010)
> IF a zip or 7z archive needs a password and none is given THEN THE archive extraction SHALL refuse with the FR-NEW-002 message.
- **Before:** `password required to extract this archive`.
- **After:** `password required to extract this archive; call again with the 'password' parameter`.
- **Reason:** one guidance for all three formats; the user is told how to retry.

#### FR-MOD-003 [EARS-O]: Wrong password wording for zip and 7z (modifies SPEC-0018/FR-NEW-011)
> IF the password given for a zip or 7z archive is wrong THEN THE archive extraction SHALL refuse with the FR-NEW-003 message.
- **Before:** `incorrect password for this archive`.
- **After:** `incorrect password for this archive; call again with the correct 'password' parameter`.
- **Reason:** same as FR-MOD-002.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

Performance: unchanged, same as the other formats; no new target is set.

### 7.2 Security and access
- IF an input is shaped to write outside the destination, to amplify its size or to plant a link THEN THE archive extraction SHALL refuse it before writing anything (FR-NEW-006, FR-NEW-007, FR-NEW-008).
- Membership: FR-NEW-018.

### 7.3 Privacy and compliance
- Password handling: FR-NEW-013.

### 7.4 Usability and accessibility
- THE archive extraction SHALL name the `password` parameter in both password refusals and say whether the password is missing or wrong.

### 7.5 Reliability
- IF an extraction is refused THEN THE archive extraction SHALL leave the project files, the remaining storage and the session write allowance unchanged.

### 7.6 Operability
- WHEN an extraction succeeds THE archive extraction SHALL record one audit line stating who extracted what and where.
- THE refusal for a server without RAR support (FR-NEW-012) SHALL differ from the refusal for a corrupt file (FR-NEW-011), so an operator can tell them apart.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | RAR with no entry; 0 byte file named `.rar` | empty folder created; corrupt refusal | E2E-017, E2E-027 |
| Boundary values | decoded size exceeds declared by 1 byte; remaining storage or session allowance equal to the declared total | refused; succeeds | E2E-036, E2E-037, E2E-038, E2E-079 |
| Concurrent actions | not applicable: one synchronous request; concurrency is covered by the shared project storage rules | none | none |
| Slow or lost connectivity | not applicable: the archive is already stored | none | none |
| Access states | not a member; platform administrator without membership; member of another project | `ERR_FORBIDDEN` | E2E-053, E2E-054, E2E-055 |
| Data variations | accents and spaces in names; accented password; upper case suffix; RAR versions 4 and 5; solid archive | kept; works; detected; extract | E2E-018, E2E-003, E2E-002, E2E-062 to E2E-064 |
| Navigation | not applicable | none | none |
| Partial operations | a link entry after valid entries; one encrypted entry among plain ones | nothing written | E2E-033, E2E-050 |
| Dependency failures | server without RAR support | FR-NEW-012 | E2E-015, E2E-039 |
| Ordering and timing | first collision in archive order; several refusals at once | named; FR-NEW-015 order | E2E-010, E2E-047, E2E-048, E2E-049, E2E-057 to E2E-061, E2E-066 to E2E-068 |
| Exhaustion | storage limit reached; session allowance reached | FR-NEW-014, FR-NEW-022 | E2E-011, E2E-041, E2E-077 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture archives are provided by the design.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | High |
| E2E-003 | New | happy | SC-002 | FR-NEW-005 | Critical |
| E2E-004 | New | happy | SC-002 | FR-NEW-004, FR-NEW-005 | High |
| E2E-005 | New | failure | SC-002 | FR-NEW-002 | Critical |
| E2E-006 | New | failure | SC-002 | FR-NEW-002, FR-NEW-004 | High |
| E2E-007 | New | failure | SC-002 | FR-NEW-003 | Critical |
| E2E-008 | New | failure | SC-003 | FR-NEW-006 | High |
| E2E-009 | New | failure, security | SC-003 | FR-NEW-007 | Critical |
| E2E-010 | New | failure | SC-003 | FR-NEW-009 | High |
| E2E-011 | New | failure | SC-003 | FR-NEW-014 | High |
| E2E-012 | New | failure, security | SC-003 | FR-NEW-008 | Critical |
| E2E-013 | New | failure | SC-004 | FR-NEW-010 | Medium |
| E2E-014 | New | failure | SC-005 | FR-NEW-011 | High |
| E2E-015 | New | failure | SC-006 | FR-NEW-012, FR-MOD-001 | High |
| E2E-016 | New | failure | SC-001 | FR-NEW-019 | Low |
| E2E-017 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-018 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-019 | New | edge | SC-001 | FR-NEW-009 | Medium |
| E2E-020 | New | side effect | SC-001 | FR-NEW-013 | High |
| E2E-021 | New | side effect | SC-002 | FR-NEW-003, FR-NEW-013 | High |
| E2E-022 | New | side effect | SC-002 | FR-NEW-002, FR-NEW-005 | High |
| E2E-023 | New | failure | SC-004 | FR-NEW-010 | Medium |
| E2E-024 | New | edge | SC-004 | FR-NEW-010 | Medium |
| E2E-025 | New | failure | SC-004 | FR-NEW-010 | Medium |
| E2E-026 | New | failure | SC-005 | FR-NEW-011 | High |
| E2E-027 | New | edge | SC-005 | FR-NEW-011 | Medium |
| E2E-028 | New | failure | SC-006 | FR-NEW-012 | Medium |
| E2E-029 | New | happy | SC-006 | FR-NEW-012 | High |
| E2E-030 | New | failure | SC-002 | FR-NEW-003, FR-NEW-004, FR-NEW-013 | High |
| E2E-031 | New | edge | SC-002 | FR-NEW-017 | Medium |
| E2E-032 | New | failure | SC-003 | FR-NEW-006 | High |
| E2E-033 | New | failure | SC-003 | FR-NEW-006 | High |
| E2E-034 | New | failure, security | SC-003 | FR-NEW-007 | Critical |
| E2E-035 | New | failure, security | SC-003 | FR-NEW-007 | High |
| E2E-036 | New | failure | SC-003 | FR-NEW-008 | High |
| E2E-037 | New | edge | SC-003 | FR-NEW-008 | High |
| E2E-038 | New | edge | SC-003 | FR-NEW-014 | Medium |
| E2E-039 | New | failure | SC-006 | FR-NEW-012 | High |
| E2E-040 | New | failure | SC-006 | FR-NEW-012 | Medium |
| E2E-041 | New | side effect | SC-003 | FR-NEW-014 | Medium |
| E2E-042 | New | edge | SC-002 | FR-NEW-017 | Medium |
| E2E-043 | New | side effect | SC-002 | FR-NEW-017 | Medium |
| E2E-044 | New | edge | SC-003 | FR-NEW-009 | Medium |
| E2E-045 | New | failure | SC-002 | FR-MOD-002 | Medium |
| E2E-046 | New | failure | SC-002 | FR-MOD-003 | Medium |
| E2E-047 | New | failure | SC-003 | FR-NEW-015, FR-NEW-006 | High |
| E2E-048 | New | failure | SC-003 | FR-NEW-015, FR-NEW-011 | High |
| E2E-049 | New | failure | SC-003 | FR-NEW-015, FR-NEW-007 | High |
| E2E-050 | New | failure | SC-002 | FR-NEW-016 | High |
| E2E-051 | New | failure | SC-002 | FR-NEW-020 | High |
| E2E-052 | New | happy | SC-002 | FR-NEW-016 | Medium |
| E2E-053 | New | failure | SC-001 | FR-NEW-018 | High |
| E2E-054 | New | failure | SC-001 | FR-NEW-018 | High |
| E2E-055 | New | failure | SC-001 | FR-NEW-018 | High |
| E2E-056 | New | failure | SC-001 | FR-NEW-021 | Medium |
| E2E-057 | New | failure | SC-003 | FR-NEW-015, FR-NEW-018 | High |
| E2E-058 | New | failure | SC-003 | FR-NEW-015, FR-NEW-003 | High |
| E2E-059 | New | failure | SC-003 | FR-NEW-015, FR-NEW-006, FR-NEW-004 | High |
| E2E-060 | New | failure | SC-003 | FR-NEW-015, FR-NEW-011 | High |
| E2E-061 | New | failure | SC-003 | FR-NEW-015 | Medium |
| E2E-062 | New | happy | SC-001 | FR-NEW-001 | High |
| E2E-063 | New | happy | SC-001 | FR-NEW-001 | High |
| E2E-064 | New | happy | SC-001 | FR-NEW-001 | Medium |
| E2E-065 | New | failure | SC-002 | FR-NEW-003 | High |
| E2E-066 | New | failure | SC-003 | FR-NEW-015, FR-NEW-002 | Medium |
| E2E-067 | New | failure | SC-003 | FR-NEW-015, FR-NEW-008 | Medium |
| E2E-068 | New | failure | SC-003 | FR-NEW-015, FR-NEW-009 | Medium |
| E2E-069 | New | failure | SC-002 | FR-NEW-020, FR-NEW-004 | Medium |
| E2E-070 | New | failure | SC-002 | FR-NEW-020 | Medium |
| E2E-071 | New | failure | SC-001 | FR-NEW-019 | Low |
| E2E-072 | New | failure | SC-002 | FR-NEW-016 | Medium |
| E2E-073 | New | edge | SC-003 | FR-NEW-014 | High |
| E2E-077 | New | failure | SC-003 | FR-NEW-022 | High |
| E2E-078 | New | edge | SC-003 | FR-NEW-022 | Medium |
| E2E-079 | New | edge | SC-003 | FR-NEW-022 | Medium |
| E2E-080 | New | side effect | SC-003 | FR-NEW-022 | Medium |
| E2E-081 | New | failure | SC-003 | FR-NEW-015, FR-NEW-022, FR-NEW-014 | Medium |
| E2E-074 | New | failure | SC-001 | FR-NEW-019 | Low |
| E2E-075 | New | failure | SC-001 | FR-NEW-021 | Medium |
| E2E-076 | New | failure | SC-001 | FR-NEW-021 | Medium |
| E2E-MOD-001 | Modified | failure | SC-006 | FR-MOD-001 | High |
| E2E-MOD-002 | Modified | failure | SC-002 | FR-MOD-002 | Medium |
| E2E-MOD-003 | Modified | failure | SC-002 | FR-MOD-002 | Medium |
| E2E-MOD-004 | Modified | failure | SC-002 | FR-MOD-003 | Medium |
| E2E-MOD-005 | Modified | failure | SC-002 | FR-MOD-003 | Medium |

Coverage (all 86 tests, 81 new and 5 modified): happy 9, failure 58, edge 13, side effect 6, security 4 (E2E-009, E2E-012, E2E-034, E2E-035), state transition 0 (no state machine). Failure to happy ratio 58:9, failure tests outnumber happy tests.

FR-NEW-015 to FR-NEW-021 were added after the implementability review (Section 13). Unless a test says otherwise, the session write allowance is large enough not to matter and the project has no storage limit; a figure for "remaining storage" sets a project storage limit, reached by filling the project with other files up to that remainder (the limit itself is set in whole megabytes).

### 9.2 New tests

#### E2E-001: Plain RAR extracts to the default destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** a member of a project holding `/uploads/report.rar` with folder `data`, `data/a.txt` containing `hello` and `data/b.txt` containing `goodbye`, and nothing at `/uploads/report`
- **When** the member extracts `/uploads/report.rar`
- **Then** the answer is destination `/uploads/report`, 2 files written, 2 folders created, 12 bytes written
- **And** `/uploads/report/data/a.txt` reads `hello` and `/uploads/report/data/b.txt` reads `goodbye`; the audit trail has one extraction line for `/uploads/report.rar`
- **Priority:** Critical

#### E2E-002: Upper case suffix is detected
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** the same archive stored as `/uploads/REPORT.RAR`
- **When** the member extracts it
- **Then** the destination is `/uploads/REPORT` and 2 files are written
- **Priority:** High

#### E2E-003: Right password extracts
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-005
- **Given** `/uploads/secret.rar` encrypted with `s3cret-Ünï`, holding `note.txt` containing `top secret`
- **When** the member extracts it with the password `s3cret-Ünï`
- **Then** the answer reports 1 file written and 10 bytes written; `/uploads/secret/note.txt` reads `top secret`
- **Priority:** Critical

#### E2E-004: Right password on encrypted file names
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-005
- **Given** `/uploads/hidden.rar` with encrypted file names and password `s3cret-Ünï`, holding `note.txt`
- **When** the member extracts it with that password
- **Then** `/uploads/hidden/note.txt` exists
- **Priority:** High

#### E2E-005: No password on an encrypted RAR
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `/uploads/secret.rar` as in E2E-003
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message `password required to extract this archive; call again with the 'password' parameter`
- **And** `/uploads/secret` does not exist
- **Priority:** Critical

#### E2E-006: No password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with no password
- **Then** the refusal and message are those of E2E-005; `/uploads/hidden` does not exist
- **Priority:** High

#### E2E-007: Wrong password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message `incorrect password for this archive; call again with the correct 'password' parameter`
- **And** `/uploads/secret` does not exist
- **Priority:** Critical

#### E2E-008: Symbolic link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/links.rar` holding `a.txt` and a symbolic link `l` pointing to `/etc/passwd`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` and the message names the entry `l`
- **And** `/uploads/links` does not exist
- **Priority:** High

#### E2E-009: Entry named `../../etc/passwd`
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/evil.rar` holding the entry `../../etc/passwd` containing `pwned`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`
- **And** read through the project's file listing: no `/etc/passwd` and no `/uploads/etc` exist, and `/uploads/evil` does not exist
- **Priority:** Critical

#### E2E-010: Collision names the first colliding path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001, and `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` already existing
- **When** the member extracts with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER` naming `/uploads/report/data/a.txt`
- **And** both existing files keep their content
- **Priority:** High

#### E2E-011: Over the project storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** a project storage limit with 5 bytes of remaining storage and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`; the remaining storage is still 5 bytes and `/uploads/report` does not exist
- **Priority:** High

#### E2E-012: Entry that decodes larger than declared
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/bomb.rar` whose entry `big.bin` declares 10 bytes and decodes to 1 000 000 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` and the message names `big.bin`
- **And** `/uploads/bomb` does not exist, and the remaining storage and the session write allowance are unchanged
- **Priority:** Critical

#### E2E-013: Multi volume name
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/big.part1.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `multi volume RAR archives are not supported: extract a single file archive`; nothing is written
- **Priority:** Medium

#### E2E-014: Truncated RAR
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/broken.rar`, the first half of a valid RAR
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing is written
- **Priority:** High

#### E2E-015: Server without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012, FR-MOD-001
- **Given** a server delivered without RAR support and `/uploads/report.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.rar'`
- **Priority:** High

#### E2E-016: Missing archive
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019
- **Given** no file at `/uploads/ghost.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`
- **Priority:** Low

#### E2E-017: Empty RAR
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** `/uploads/none.rar` holding no entry
- **When** the member extracts it
- **Then** the answer is 0 files written, 1 folder created, 0 bytes written; `/uploads/none` exists and is empty
- **Priority:** Medium

#### E2E-018: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** `/uploads/noms.rar` holding `été 2026/compte rendu.txt` containing `ok`
- **When** the member extracts it
- **Then** `/uploads/noms/été 2026/compte rendu.txt` reads `ok`
- **Priority:** Medium

#### E2E-019: Overwrite true replaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001 and `/uploads/report/data/a.txt` containing `old`
- **When** the member extracts with overwrite true
- **Then** the answer reports 2 files written and `/uploads/report/data/a.txt` reads `hello`
- **Priority:** Medium

#### E2E-020: Success is audited without the password
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-013
- **Given** the encrypted archive of E2E-003
- **When** the member extracts it with the password `s3cret-Ünï`
- **Then** the audit trail has exactly one extraction line for `/uploads/secret.rar` with the destination, 1 file written and 10 bytes written
- **And** neither the audit trail nor the logs contain `s3cret-Ünï`
- **Priority:** High

#### E2E-021: Wrong password is not logged
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-013
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with the password `nope-12345`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` and no log or audit line contains `nope-12345`; there is no extraction audit line
- **Priority:** High

#### E2E-022: The HTTP API gives the same refusal and the same success
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** `/uploads/secret.rar`
- **When** a client asks the HTTP API to extract it with no password, then with `s3cret-Ünï`
- **Then** the first answer carries the error code and message of E2E-005; the second carries the answer of E2E-003
- **Priority:** High

#### E2E-023: `.part02.rar` is multi volume
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/big.part02.rar`
- **When** the member extracts it
- **Then** the refusal and message are those of E2E-013
- **Priority:** Medium

#### E2E-024: `part1x` is not a multi volume name
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** `/uploads/report.part1x.rar` and `/uploads/big.part.rar`, two valid RARs each holding `a.txt` (5 bytes)
- **When** the member extracts each
- **Then** the destinations are `/uploads/report.part1x` and `/uploads/big.part` and 1 file is written each time
- **Priority:** Medium

#### E2E-025: Upper case multi volume name
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/BIG.PART3.RAR`
- **When** the member extracts it
- **Then** the refusal and message are those of E2E-013
- **Priority:** Medium

#### E2E-026: A text file named `.rar`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/fake.rar` containing the text `not an archive`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing is written
- **Priority:** High

#### E2E-027: A 0 byte `.rar`
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/empty.rar` with 0 bytes
- **When** the member extracts it
- **Then** the refusal is as E2E-026; `/uploads/empty` does not exist
- **Priority:** Medium

#### E2E-028: Extension shown as typed
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/report.RAR`
- **When** the member extracts it
- **Then** the message is `unsupported archive extension: '.RAR'`
- **Priority:** Medium

#### E2E-029: Other formats unchanged without RAR support
- **Category:** happy; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/pack.zip` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** 1 file is written at `/uploads/pack/a.txt`
- **Priority:** High

#### E2E-030: Wrong password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-013
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with the password `nope-67890`
- **Then** the refusal and message are those of E2E-007; `/uploads/hidden` does not exist; no log or audit line contains `nope-67890`
- **Priority:** High

#### E2E-031: Password on a plain RAR is ignored
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** the member extracts it with the password `whatever`
- **Then** the answer is that of E2E-001
- **Priority:** Medium

#### E2E-032: Hard link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/hl.rar` holding `a.txt` and a hard link `b.txt` to it
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` and the message names `b.txt`; `/uploads/hl` does not exist
- **Priority:** High

#### E2E-033: Link after valid entries writes nothing
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/mix.rar` holding `ok1.txt`, `ok2.txt`, then a symbolic link `l`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED`; `/uploads/mix` does not exist, so neither `ok1.txt` nor `ok2.txt` was written
- **Priority:** High

#### E2E-034: Absolute entry name
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/abs.rar` holding the entry `/etc/cron.d/x` containing `* * * * * root id`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`
- **And** `/etc/cron.d/x` does not exist in the project and `/uploads/abs` does not exist
- **Priority:** Critical

#### E2E-035: Traversal in the middle of a name
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/mid.rar` holding `a/../../b.txt` containing `x`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`; `/uploads/b.txt` and `/b.txt` do not exist
- **Priority:** High

#### E2E-036: One oversized entry among honest ones
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/one.rar` with `ok.txt` honest and `liar.bin` declaring 10 bytes and decoding to 11
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` naming `liar.bin`; `ok.txt` was not written
- **Priority:** High

#### E2E-037: Declared 10 bytes, decoded 10 bytes
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/exact.rar` with `exact.bin` declaring 10 bytes and decoding to 10 bytes
- **When** the member extracts it
- **Then** 1 file and 10 bytes are written
- **Priority:** High

#### E2E-038: Storage exactly equal to the declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** a project storage limit with 12 bytes of remaining storage and `/uploads/report.rar` of E2E-001
- **When** the member extracts it
- **Then** 2 files are written and the remaining storage is 0 bytes
- **Priority:** Medium

#### E2E-039: Multi volume name without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/big.part1.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.rar'`; nothing is written
- **Priority:** High

#### E2E-040: Upper case multi volume name without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/big.PART2.RAR`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.RAR'`
- **Priority:** Medium

#### E2E-041: A refused storage limit leaves the project usable
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** the situation of E2E-011
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the remaining storage is 2 bytes
- **Priority:** Medium

#### E2E-042: Empty password on a plain RAR is ignored
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** the member extracts it with an empty password
- **Then** the answer is that of E2E-001
- **Priority:** Medium

#### E2E-043: Ignored password through the HTTP API
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** a client asks the HTTP API to extract it with the password `whatever`
- **Then** the client receives the answer of E2E-001
- **Priority:** Medium

#### E2E-044: Collision under an explicit destination
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001 and `/out/data/b.txt` already existing
- **When** the member extracts it to the destination `/out` with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER` naming `/out/data/b.txt`; `/out/data/a.txt` does not exist
- **Priority:** Medium

#### E2E-045: Zip missing password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002
- **Given** an encrypted `/uploads/pack.zip`
- **When** a client asks the HTTP API to extract it with no password
- **Then** the error code is `ERR_PASSWORD_REQUIRED` and the message is `password required to extract this archive; call again with the 'password' parameter`
- **Priority:** Medium

#### E2E-046: 7z wrong password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003
- **Given** an encrypted `/uploads/pack.7z`
- **When** a client asks the HTTP API to extract it with the password `nope`
- **Then** the error code is `ERR_PASSWORD_REQUIRED` and the message is `incorrect password for this archive; call again with the correct 'password' parameter`
- **Priority:** Medium

#### E2E-047: Link entry wins over a missing password when names are readable
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-006
- **Given** `/uploads/both.rar`, encrypted with readable file names, holding `a.txt` and a symbolic link `l`
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_NOT_SUPPORTED` naming `l`, not `ERR_PASSWORD_REQUIRED`
- **Priority:** High

#### E2E-048: Corrupt wins over a wrong password when file names are readable
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-011
- **Given** `/uploads/cut.rar`, a truncated RAR with encrypted data and readable file names
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`
- **Priority:** High

#### E2E-049: Escaping name wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-007
- **Given** `/uploads/two.rar` holding `a.txt` and `../x.txt`, and `/uploads/two/a.txt` already existing
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`, not `ERR_NO_CLOBBER`
- **Priority:** High

#### E2E-050: One encrypted entry among plain ones, no password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-016
- **Given** `/uploads/part.rar` with `a.txt` plain and `b.txt` encrypted with `s3cret-Ünï`
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-005; `/uploads/part` does not exist, so `a.txt` was not written
- **Priority:** High

#### E2E-051: Empty password on an encrypted RAR
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with an empty password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007
- **Priority:** High

#### E2E-052: Partly encrypted RAR with the right password
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-016
- **Given** `/uploads/part.rar` as in E2E-050
- **When** the member extracts it with `s3cret-Ünï`
- **Then** 2 files are written and `/uploads/part/a.txt` and `/uploads/part/b.txt` exist
- **Priority:** Medium

#### E2E-053: Non member
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** `/uploads/report.rar` in project `alpha`, and `bob@example.com` who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written
- **Priority:** High

#### E2E-054: Platform administrator without membership
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** the same archive and `root@example.com`, a platform administrator who is not a member of `alpha`
- **When** `root@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'root@example.com' is not a member of 'alpha'`
- **Priority:** High

#### E2E-055: Member of another project
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** `carol@example.com`, a member of project `beta` only, and the archive in `alpha`
- **When** `carol@example.com` extracts it asking for project `alpha`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'carol@example.com' is not a member of 'alpha'`
- **Priority:** High

#### E2E-056: Path is a folder
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021
- **Given** a folder `/uploads/dir.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.rar' is a directory, not an archive file`
- **Priority:** Medium

#### E2E-057: Non member asking for a missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-018; **Priority:** High
- **Given** project `alpha` without `/uploads/ghost.rar`, and `bob@example.com` who is not a member of `alpha`
- **When** `bob@example.com` extracts `/uploads/ghost.rar`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`, not `ERR_NOT_FOUND`

#### E2E-058: Truncated archive with encrypted file names, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-003; **Priority:** High
- **Given** `/uploads/cuth.rar`, a truncated RAR with encrypted file names, password `s3cret-Ünï`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007; nothing is written

#### E2E-059: Encrypted file names, right password, link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-006, FR-NEW-004; **Priority:** High
- **Given** `/uploads/hl2.rar` with encrypted file names, password `s3cret-Ünï`, holding `a.txt` and a symbolic link `l`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is `ERR_NOT_SUPPORTED` naming `l`; `/uploads/hl2` does not exist

#### E2E-060: Truncated archive with readable names, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-011; **Priority:** High
- **Given** `/uploads/cut2.rar`, a truncated RAR with encrypted data and readable file names
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`

#### E2E-061: Destination outside the project
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015; **Priority:** Medium
- **Given** `/uploads/report.rar` as in E2E-001
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written

#### E2E-062: RAR version 4 extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/v4.rar`, created as RAR version 4, holding `a.txt` containing `hello`
- **When** the member extracts it
- **Then** 1 file and 5 bytes are written; `/uploads/v4/a.txt` reads `hello`

#### E2E-063: RAR version 5 extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/v5.rar`, created as RAR version 5, holding `a.txt` containing `hello`
- **When** the member extracts it
- **Then** 1 file and 5 bytes are written; `/uploads/v5/a.txt` reads `hello`

#### E2E-064: Solid archive extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/solid.rar`, a solid archive holding `a.txt` containing `hello` and `b.txt` containing `goodbye`
- **When** the member extracts it
- **Then** 2 files and 12 bytes are written

#### E2E-065: Wrong password on a RAR version 4
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/uploads/v4secret.rar`, an encrypted RAR version 4, password `s3cret-Ünï`
- **When** the member extracts it with `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007, not a corrupt refusal

#### E2E-066: Missing password wins over an oversized entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/encbomb.rar`, encrypted data with readable names, entry `big.bin` declaring 10 bytes and decoding to 1 000 000
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-005

#### E2E-067: Oversized entry wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/bomb.rar` as in E2E-012 and an existing file `/uploads/bomb/big.bin`
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_INVALID_ARGUMENT` naming `big.bin`, not `ERR_NO_CLOBBER`

#### E2E-068: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-009; **Priority:** Medium
- **Given** the situation of E2E-011 (5 bytes remaining, 12 declared) and `/uploads/report/data/a.txt` already existing
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`

#### E2E-069: Empty password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020, FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with an empty password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007

#### E2E-070: Empty password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020; **Priority:** Medium
- **Given** `/uploads/secret.rar`
- **When** a client asks the HTTP API to extract it with an empty password
- **Then** the error code and message are those of E2E-007

#### E2E-071: Missing archive through the HTTP API
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019; **Priority:** Low
- **Given** no file at `/uploads/ghost.rar`
- **When** a client asks the HTTP API to extract it
- **Then** the error code is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`

#### E2E-072: Partly encrypted RAR, no password, through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-016; **Priority:** Medium
- **Given** `/uploads/part.rar` as in E2E-050
- **When** a client asks the HTTP API to extract it with no password
- **Then** the error code and message are those of E2E-005; `/uploads/part` does not exist

#### E2E-073: Overwrite counts net growth against the storage limit
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a project storage limit with 5 bytes of remaining storage, `/uploads/report.rar` as in E2E-001 (12 declared bytes), and existing `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` totalling 20 bytes
- **When** the member extracts it with overwrite true
- **Then** 2 files are written and the remaining storage is 13 bytes

#### E2E-074: Missing archive with an explicit destination
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019; **Priority:** Low
- **Given** no file at `/uploads/ghost.rar`
- **When** the member extracts it to the destination `/out`
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`; `/out` does not exist

#### E2E-075: Folder named with an upper case suffix
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021; **Priority:** Medium
- **Given** a folder `/uploads/DIR.RAR`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/DIR.RAR' is a directory, not an archive file`

#### E2E-076: Folder through the HTTP API
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021; **Priority:** Medium
- **Given** a folder `/uploads/dir.rar`
- **When** a client asks the HTTP API to extract it
- **Then** the error code and message are those of E2E-056

#### E2E-077: Over the session write allowance
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** High
- **Given** a project without a storage limit, a session write allowance with 5 bytes remaining, and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`; `/uploads/report` does not exist and the allowance is still 5 bytes

#### E2E-078: The allowance is charged the full declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, existing `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` totalling 20 bytes, and `/uploads/report.rar` as in E2E-001 (12 declared bytes)
- **When** the member extracts it with overwrite true
- **Then** 2 files are written and the allowance has 8 bytes remaining (12 charged, nothing netted)

#### E2E-079: Allowance equal to the declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** a session write allowance with 12 bytes remaining and `/uploads/report.rar` as in E2E-001
- **When** the member extracts it
- **Then** 2 files are written and the allowance has 0 bytes remaining

#### E2E-080: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** the situation of E2E-077
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 2 bytes remaining

#### E2E-081: The project refusal comes first
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-022, FR-NEW-014; **Priority:** Medium
- **Given** a project storage limit with 5 bytes of remaining storage, a session write allowance with 5 bytes remaining, and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged

### 9.3 Exploit tests

Not applicable (`--security` not set); the security shaped tests above are ordinary acceptance tests.

### 9.4 Modified tests

#### E2E-MOD-001: `.rar` support (was SPEC-0018 E2E-NEW-022)
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-MOD-001; **Priority:** High
- **Validated before:** a `.rar` is refused with `ERR_NOT_SUPPORTED` naming `.rar`.
- **Validates now:** with RAR support, `/uploads/data.rar` holding a valid archive extracts; without RAR support the refusal of before still holds.
- **Given** `/uploads/data.rar`; **When** extracted on each kind of server; **Then** extraction succeeds with RAR support and `ERR_NOT_SUPPORTED` containing `.rar` without.

#### E2E-MOD-002: 7z missing password message (was the 7z test pinning `password required to extract this archive`)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Validated before:** exact message `password required to extract this archive`.
- **Validates now:** exact message of FR-NEW-002.
- **Given** an encrypted 7z; **When** extracted with no password; **Then** `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`.

#### E2E-MOD-003: zip missing password message (was SPEC-0018 E2E-NEW-006)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Validated before:** message contains `password required`.
- **Validates now:** exact message of FR-NEW-002.
- **Given** an encrypted zip; **When** extracted with no password; **Then** the exact message of FR-NEW-002.

#### E2E-MOD-004: zip wrong password message (was SPEC-0018 E2E-NEW-057)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003; **Priority:** Medium
- **Validated before:** message contains `incorrect password`.
- **Validates now:** exact message of FR-NEW-003.
- **Given** an encrypted zip; **When** extracted with `nope`; **Then** the exact message of FR-NEW-003.

#### E2E-MOD-005: 7z wrong password message
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003; **Priority:** Medium
- **Validated before:** exact message `incorrect password for this archive`.
- **Validates now:** exact message of FR-NEW-003.
- **Given** an encrypted 7z; **When** extracted with `nope`; **Then** `ERR_PASSWORD_REQUIRED` with `incorrect password for this archive; call again with the correct 'password' parameter`.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-MOD-001, FR-NEW-001, FR-NEW-009, FR-NEW-013, FR-NEW-018, FR-NEW-019, FR-NEW-021 | E2E-001, E2E-002, E2E-062, E2E-063, E2E-064 | E2E-016, E2E-053, E2E-054, E2E-055, E2E-056, E2E-071, E2E-074, E2E-075, E2E-076 | E2E-017, E2E-018, E2E-019 | E2E-020 |
| SC-002 | FR-MOD-002, FR-MOD-003, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-013, FR-NEW-016, FR-NEW-017, FR-NEW-020 | E2E-003, E2E-004, E2E-052 | E2E-005, E2E-006, E2E-007, E2E-030, E2E-045, E2E-046, E2E-050, E2E-051, E2E-065, E2E-069, E2E-070, E2E-072, E2E-MOD-002, E2E-MOD-003, E2E-MOD-004, E2E-MOD-005 | E2E-031, E2E-042 | E2E-021, E2E-022, E2E-043 |
| SC-003 | FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-011, FR-NEW-014, FR-NEW-015, FR-NEW-018, FR-NEW-022 | none | E2E-008, E2E-009, E2E-010, E2E-011, E2E-012, E2E-032, E2E-033, E2E-034, E2E-035, E2E-036, E2E-047, E2E-048, E2E-049, E2E-057, E2E-058, E2E-059, E2E-060, E2E-061, E2E-066, E2E-067, E2E-068, E2E-077, E2E-081 | E2E-037, E2E-038, E2E-044, E2E-073, E2E-078, E2E-079 | E2E-041, E2E-080 |
| SC-004 | FR-NEW-010 | none | E2E-013, E2E-023, E2E-025 | E2E-024 | none |
| SC-005 | FR-NEW-011 | none | E2E-014, E2E-026 | E2E-027 | none |
| SC-006 | FR-MOD-001, FR-NEW-012 | E2E-029 | E2E-015, E2E-028, E2E-039, E2E-040, E2E-MOD-001 | none | none |

SC-004 and SC-005 are refusal scenarios; their happy path is the ordinary extraction of SC-001 (E2E-024 reads names that merely look multi volume as ordinary RAR files).

| Requirement | Tests | Count |
|---|---|---|
| FR-MOD-001 | E2E-001, E2E-002, E2E-015, E2E-MOD-001 | 4 |
| FR-MOD-002 | E2E-045, E2E-MOD-002, E2E-MOD-003 | 3 |
| FR-MOD-003 | E2E-046, E2E-MOD-004, E2E-MOD-005 | 3 |
| FR-NEW-001 | E2E-001, E2E-002, E2E-017, E2E-018, E2E-062, E2E-063, E2E-064 | 7 |
| FR-NEW-002 | E2E-005, E2E-006, E2E-022, E2E-066 | 4 |
| FR-NEW-003 | E2E-007, E2E-021, E2E-030, E2E-058, E2E-065 | 5 |
| FR-NEW-004 | E2E-004, E2E-006, E2E-030, E2E-059, E2E-069 | 5 |
| FR-NEW-005 | E2E-003, E2E-004, E2E-022 | 3 |
| FR-NEW-006 | E2E-008, E2E-032, E2E-033, E2E-047, E2E-059 | 5 |
| FR-NEW-007 | E2E-009, E2E-034, E2E-035, E2E-049 | 4 |
| FR-NEW-008 | E2E-012, E2E-036, E2E-037, E2E-067 | 4 |
| FR-NEW-009 | E2E-010, E2E-019, E2E-044, E2E-068 | 4 |
| FR-NEW-010 | E2E-013, E2E-023, E2E-024, E2E-025 | 4 |
| FR-NEW-011 | E2E-014, E2E-026, E2E-027, E2E-048, E2E-060 | 5 |
| FR-NEW-012 | E2E-015, E2E-028, E2E-029, E2E-039, E2E-040 | 5 |
| FR-NEW-013 | E2E-020, E2E-021, E2E-030 | 3 |
| FR-NEW-014 | E2E-011, E2E-038, E2E-041, E2E-073, E2E-081 | 5 |
| FR-NEW-015 | E2E-047, E2E-048, E2E-049, E2E-057, E2E-058, E2E-059, E2E-060, E2E-061, E2E-066, E2E-067, E2E-068, E2E-081 | 12 |
| FR-NEW-016 | E2E-050, E2E-052, E2E-072 | 3 |
| FR-NEW-017 | E2E-031, E2E-042, E2E-043 | 3 |
| FR-NEW-018 | E2E-053, E2E-054, E2E-055, E2E-057 | 4 |
| FR-NEW-019 | E2E-016, E2E-071, E2E-074 | 3 |
| FR-NEW-020 | E2E-051, E2E-069, E2E-070 | 3 |
| FR-NEW-021 | E2E-056, E2E-075, E2E-076 | 3 |
| FR-NEW-022 | E2E-077, E2E-078, E2E-079, E2E-080, E2E-081 | 5 |

## 11. Open questions

None.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Archive | a stored file that holds other files and folders | extraction |
| Entry | one file, folder or link inside an archive | extraction |
| Destination | the folder where entries are written | extraction |
| Refusal | an error answer; the project is left unchanged | extraction |
| Collision | an entry whose target path already holds a file | extraction |
| Overwrite | the choice to replace colliding files instead of refusing | extraction |
| Password | the secret the member supplies to open an encrypted archive | extraction |
| Encrypted file names | a RAR whose list of entries cannot be read without the password | extraction |
| Solid archive | a RAR whose entries are compressed together | extraction |
| Multi volume archive | an archive split over several files, named `.partN.rar` | extraction |
| RAR support | the optional capability of a server to read RAR archives | extraction |
| Audit line | the one line an operator reads to see who extracted what and where | extraction |
| Storage limit | the optional cap on the total bytes a project stores, set in whole megabytes | project storage |
| Remaining storage | the project's storage limit minus the bytes already stored | project storage |
| Session write allowance | the bytes a member can still write in the current session; always set | project storage |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server and reads its audit trail | extraction |
| Platform administrator | an operator account that manages projects and members and has no implicit access to files | project storage |
| Declared total | the sum of the sizes the entries of an archive announce for themselves | extraction |
| Assistant tool | the tool an AI assistant calls to extract an archive | extraction |
| HTTP API | the web endpoint a program calls to extract an archive; the second door to the same behavior | extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 5 | 6 | NOT-IMPLEMENTABLE |
| 2 | 4 | 4 | NOT-IMPLEMENTABLE |
| 3 | 1 | 2 | NOT-IMPLEMENTABLE (F: quota rule ambiguous, amended as FR-NEW-014 and E2E-073) |
| 4 | 0 | 2 | IMPLEMENTABLE |

**Erratum 2026-11-04 (found while writing SPEC-0081):** the single "write quota" was two limits; FR-NEW-014 now names the project storage limit, FR-NEW-022 the session write allowance, with E2E-077 to E2E-081 and the order item (12). Reviewed by a fresh read only agent on the changed parts: 3 findings (one leftover wording, duplicate destinations in FR-NEW-022, line citations), all applied; verdict IMPLEMENTABLE.
**Amendments applied after round 3:** F-1 quota rule (FR-NEW-014 net growth, E2E-073); A-1 7z exact message citations; A-2 `password_required_message` reuse; FR-NEW-019 split with FR-NEW-021 (E2E-056, E2E-074 to E2E-076); E2E-069 to E2E-072 added in round 2 fixes.
**Amendments applied after round 4:** A-1 duplicate destinations and session allowance in FR-NEW-014; A-2 wording of 3.3; Section 13 and 9.1 note corrected.
**Amendments applied after round 2:** F-1 order (membership first, FR-NEW-015 rewritten), F-2 encrypted names ordering (FR-NEW-015 items 4 and 5, E2E-058 to E2E-060), F-3 destination in the order (E2E-061), F-4 RAR versions and solid (E2E-062 to E2E-065), A-1 to A-4, and the consistency fixes (categories, EARS shapes, FR-NEW-019, FR-NEW-020, glossary, order pair tests E2E-066 to E2E-068).
**Amendments applied after round 1:** F-1 (FR-NEW-012 covers multi volume names, E2E-039, E2E-040, the 14.3 decision removed), F-2 (FR-NEW-015 and E2E-047 to E2E-049), F-3 (FR-NEW-016, FR-NEW-017, E2E-050 to E2E-052), F-4 (the 7.1 target removed: performance is unchanged), F-5 (FR-NEW-018, E2E-053 to E2E-055; the RAR reader presence is a licence constraint in 3.3 and a design input in 14.3), A-1 to A-6, plus the self consistency fixes (coverage, traceability, EARS splits, glossary).
**No implementation detail outside Section 14:** user visible option names (`password`, `destination`, `overwrite`), error codes and verbatim messages are WHAT; every other hit rewritten.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | six families and case insensitive suffix detection | `crates/core/src/tools/archive.rs:40-69` |
| 2 | `.rar` refused with the quoted message | `crates/core/src/tools/archive.rs:62-66`, test `:796-804` |
| 3 | pre scan then all or nothing; refusals and codes | `crates/core/src/tools/archive.rs:131-215`, `:284-300`, symlink `:420-424`, escape `:359-385`, size `:339-347`, collision `:166-176`, quota `:178-198` |
| 4 | zip and 7z password messages | `crates/core/src/tools/archive.rs:441`, `:489`, `:531`, `:615-620` |
| 5 | corrupt message | `crates/core/src/tools/archive.rs:350-354` |
| 6 | answer shape, folder counting, audit line, logs | `crates/core/src/tools/archive.rs:214-275` |
| 7 | zip tests prefix only, 7z exact for both messages | `crates/core/src/tools/archive.rs:1034`, `:1047`, `:2158`, `:2174` |
| 8 | no RAR fixture in the tree | `fd -e rar` over the repository returns 0 files (recounted) |
| 9 | SPEC-0018 states rar out of scope and FR-NEW-006 names the refusal | `sdlc/specs/SPEC-0018-archive-extract-in-place/spec.md` (Summary) and `archive.rs:56-58` |
| 10 | two limits: project storage on net growth (`ERR_PROJECT_QUOTA_EXCEEDED`), session allowance on the gross declared total (`ERR_WRITE_QUOTA_EXCEEDED`); project checked first | `crates/core/src/tools/archive.rs:178-198`, `crates/core/src/quota.rs:96-113`, `:136-158`, `crates/core/src/safety.rs:136-147`, `crates/core/src/errors.rs:101-112` |
| 11 | bare `.gz` refusal pinned by E2E-NEW-034 | `crates/core/src/tools/archive.rs:806-815` |
| 12 | membership refusal text and code | `crates/core/src/storage/admin.rs:393` (text), `crates/core/src/errors.rs:60` (code) |
| 13 | folder path refusal text | `crates/core/src/tools/archive.rs:136-140` |
| 15 | SPEC-0041 FR-NEW-003 quotes the old missing password message and asks the password before the structural refusal | `sdlc/specs/SPEC-0041-refuse-inconsistent-zips/sdlc-spec.md:100-101` |
| 14 | existing refusal order for zip (pre scan, password, header check, decode) | `.agent_docs/tools.md:196-204` |

Test command: `cargo test -p mcp-fs-core tools::archive`

### 14.2 Class sweep

Not applicable (`--security` not set).

### 14.3 Design inputs

- The RAR reader is the `unrar` crate, a wrapper of RARLAB's C library; to confirm with context7 before design: it opens a file path, not bytes, and takes the password at open; listing entry headers needs no password unless the file names are encrypted. The entry pre scan therefore needs a per call temporary file. Signal: a compile or test failure on an in memory open.
- Its licence is not in `deny.toml` `[licenses] allow` (`deny.toml:31`); `cargo deny check` fails until an exception is added (`deny.toml:21-27` shows how the advisory ignores are documented). Signal: the `make check` security step.
- Feature name `rar`, off by default, forwarded from `crates/mcp-fs` like `postgres` (`crates/core/Cargo.toml:15-23`). `make check` uses `--all-features`, so CI compiles it. A build without the feature must not link the reader: check with `cargo tree -p mcp-fs-core` without the feature (licence constraint of 3.3).
- RAR fixtures cannot be created from Rust; commit small binary fixtures and document how they were built (needs the RARLAB command line tool, to confirm). Cases: plain, encrypted data, encrypted names, partly encrypted, symlink, hardlink, traversal names, oversized, truncated.
- The quota rule is `quota::admit_paths` (net change per destination, `crates/core/src/quota.rs:148-158`); RAR reuses it unchanged; the session allowance is charged by the same call (`quota.rs:96-113`). Signal: E2E-073, E2E-078.
- The reader may report a hard link as a file (FR-NEW-006); verify with the E2E-032 fixture.
- The password messages are literals at `archive.rs:441`, `:489`, `:531`, `:617`, `:619`; extend the existing `password_required_message` (`archive.rs:615`) to carry the new suffixes, route the three remaining literals (`:441`, `:489`, `:531`) through it, and use it for RAR (FR-NEW-002, 003, MOD-002, MOD-003).
- "folders created" in the answer is the key `dirs_created`; "files written" is `files_written`; "bytes written" is `bytes_written` (`archive.rs:265-268`).
- The refusal order of FR-NEW-015: membership runs before path normalization (`crates/core/src/tools/mod.rs:10-16`); zip reports its pre scan before the password (`archive.rs:418-440`, `.agent_docs/tools.md:196-204`); 7z asks the password first and checks entries while decoding (`archive.rs:508-577`); for RAR the order is new. The destination is normalized after decode and before the collision scan (`archive.rs:162-165`).
- The reader must distinguish a wrong or missing password from corruption for RAR versions 4 and 5, with data encryption and with name encryption. Signal: the fixtures of E2E-007, E2E-030, E2E-051, E2E-058, E2E-065. If the reader reports only a generic failure, FR-NEW-003 cannot be met for that case: the design must raise it before building.
- Update the exact assertions at `archive.rs:2158` and `:2174` and the US-0048 tests that quote the old missing password text (`sdlc/stories/US-0048-*`).
- Update `.agent_docs/tools.md:180-183`, `TOOL_CONTRACT.txt` (the golden changes only if a parameter description changes; it need not) and `README.md`; BL-0123 records the multi volume deferral.
