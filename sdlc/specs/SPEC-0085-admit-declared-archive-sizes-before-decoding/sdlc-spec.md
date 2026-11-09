# Admit declared archive sizes before decoding: functional specification

> Id: SPEC-0085
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (archive extraction, three formats), 12 requirements, public behavior change (an archive whose declared sizes exceed a limit is refused before any entry is decoded, which reorders some refusals; an entry declaring an uncountable size is refused instead of bypassing the limits), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-09
> Project: existing
> From backlog: BL-0125
> Depends on: none
> Security: internal finding
> CVSS: not scored
> Affected: every build with archive extraction (SPEC-0018 onward); RAR from SPEC-0080
> Fixed in: unreleased
>
> Note: written non interactively. The user authorized every decision and gave the instruction "spec all of them, design later", which is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who extracts a `.zip`, `.7z` or `.rar` archive is checked against the project's storage limit and their session write allowance on the sizes the archive declares, before a single entry is decoded. Today every entry is decoded into the server's memory first and the limits are checked afterwards, so a small archive declaring a huge content uses that memory before it is refused. A declared size too large to be counted is today turned into a negative figure, which lets an archive pass both limits and even gives the member more allowance; it is refused from now on. While decoding, no entry is held past its declared size plus one fixed chunk, so the server never holds more than the admitted total plus that chunk. Compressed tar archives are bounded by SPEC-0084; nothing else changes.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member extracts `.zip`, `.7z` and, on a server delivered with RAR support, `.rar` archives from the assistant tool and from the HTTP API. [14.1 row 1]
- Every entry of the archive is decoded into the server's memory before the destination, the collisions and the two limits are checked. No limit is checked before or during decoding. [row 2]
- The two limits are checked after the collision check, on the sizes the entries declare: the project's storage limit (optional, on the net growth: the declared size of each file minus the size of the file stored at its target path, the last entry counting when a path repeats; `ERR_PROJECT_QUOTA_EXCEEDED`, message `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested`, `<R>` being the net growth) then the session write allowance (always set, 50 MiB by default, on the gross declared total; `ERR_WRITE_QUOTA_EXCEEDED`, message `session write quota of <N> bytes exceeded`). Equality is accepted; a refusal charges nothing, writes nothing and records no audit line. [row 3]
- A declared size is not counted as such: a size of 9 223 372 036 854 775 808 bytes or more counts as a negative number, so the net growth and the declared total can come out negative. Such an archive passes both limits, its real content is written, and the member's used allowance goes down. Several large declared sizes added together can also wrap around. [row 4]
- An entry never yields more than its declared size: zip and RAR stop decoding at it and refuse with `archive entry '<path>' decompressed to more bytes than declared <D>`; 7z reads exactly the declared size of each entry from its block, so a size lie shows as a damaged block. An entry that holds fewer bytes than it declares is accepted by zip and 7z when no checksum says otherwise (ASSUMED, row 5), and refused by RAR. [row 5]
- Order of refusals today. zip: not a member, missing path, folder, unreadable archive, then per entry in archive order a link, an escaping name or a duplicate name, then a missing password, then a local header disagreeing with the directory, then decoding (wrong password, damaged data, entry past its declared size), then destination, collision, storage limit, session allowance. 7z: same first three, then an encrypted header without the right password or an unreadable header, then a missing password, then entry by entry in decoding order: escaping name or special type, then that entry's decoding, then destination, collision, limits. RAR: SPEC-0080 FR-NEW-015. [row 6]
- A compressed tar archive is decompressed whole before anything is checked; SPEC-0084 bounds it. A plain `.tar` is the stored file itself, never decompressed. Bare `.gz`, `.bz2` and `.xz` files are bounded while decoding (SPEC-0081, SPEC-0082, SPEC-0083). [row 7]
- On success the answer gives destination, files written, folders created and bytes written, and one audit line is recorded. [row 8]

### 2.2 Specs governing this area

- SPEC-0018: FR-NEW-012 decodes every entry fully before the checks; FR-NEW-013 and FR-NEW-014 refuse a link, special or escaping entry; FR-NEW-019 checks the allowance after the collision check; FR-NEW-020 refuses an entry decoding past its declared size. This increment moves the limits before decoding (FR-MOD-001) and the 7z entry checks before decoding (FR-MOD-002). SPEC-0018 is not modified.
- SPEC-0020 FR-NEW-007 defines the net growth admission of an extraction; its figures are unchanged, only checked earlier as well.
- SPEC-0080 FR-NEW-015 orders RAR refusals with the limits last; FR-MOD-001 inserts the declared limits before decoding. Its E2E-068 ("collision wins over the storage limit") changes (Section 9.4).
- SPEC-0080 FR-NEW-014 subtracts replaced files "when `overwrite` is true"; with `overwrite` false a collision was always reported before the limit, so no call reaching the limit saw another figure. FR-NEW-001 subtracts stored files whatever `overwrite` says, which only changes what an over limit archive that also collides reports (FR-MOD-001).
- SPEC-0084 bounds the decompressed tar stream; this increment does not touch the tar family. [row 9]

### 2.3 What is verified today

- Extraction of the three formats is tested for success, passwords, links, escaping names, collisions, size lies, the two limits and their order with a collision (RAR). [row 10]
- No test covers a refusal before decoding, the memory held while decoding zip, 7z or RAR, a zip64 size, a declared size of 2^63 or more, or a declared total that wraps. [row 10]

## 3. Scope

### 3.1 In scope

- `.zip`, `.7z`, `.rar`, any letter case, from both doors: the two limits checked on the declared sizes before any entry is decoded.
- Declared sizes too large to count: refused at the limit, never counted negative.
- The cap on what decoding holds: each entry at its declared size plus one chunk, the whole call at the declared total plus one chunk.
- The 7z entry checks moved before decoding, so the order of refusals is the same shape for the three formats.

### 3.2 Out of scope, and why

- The tar family: its sizes are inside the compressed stream, so they cannot be read before decoding; SPEC-0084 bounds the decompressed stream instead. A plain `.tar` is never decompressed: what is read is the stored file, already within the project's storage.
- A destination such as `../../x` clamped to `/x` by zip, 7z and tar: BL-0127.
- Truncated RAR4 and header encrypted RAR5 archives: BL-0131.
- Office documents (`.docx`, `.xlsx`, `.pptx`) read for their text decode their parts with no size bound: same class of problem, other feature, BL-0134.
- Memory used to read the archive's headers themselves (a huge entry list, a large 7z dictionary): bounded by the archive's stored size and by the 7z reader's own memory limit; no request for more.

### 3.3 Constraints imposed by the context

None.

## 4. Actors

- **Member:** a person with access to a project; extracts archives through the assistant tool or the HTTP API.
- **Operator:** runs the server, watches its memory and reads the audit trail.
- **Attacker:** a member, or someone who got a member to upload a file, who crafts an archive to exhaust memory or to write past the limits.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Archive extraction | turning a stored archive into project files | entry, declared size, declared total, decoding, decoded bytes held, refusal |
| Project storage | the files, the storage limit and the session write allowance of a project | remaining storage, session write allowance, net growth, collision, overwrite |

## 5. Usage scenarios

### SC-001: Extract an archive within the limits
**Actor:** Member.
**Preconditions:** the project holds `/uploads/pack.zip` (or `.7z`, `.rar`) holding `a.txt` with the 5 bytes `hello`; nothing at `/uploads/pack`.
**Flow:**
1. The member asks to extract it → the system checks the declared sizes against the limits, decodes, checks destination and collisions, admits and writes `/uploads/pack/a.txt`.
2. The system answers destination `/uploads/pack`, 1 file, 1 folder, 5 bytes, and records one audit line.
3. A plain `.tar`, a compressed tar archive and a bare compressed file behave as before.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: the declared net growth is one byte over the remaining storage → the project refusal, nothing decoded.

### SC-002: An archive declaring more than the limits is refused before decoding
**Actor:** Member or attacker.
**Preconditions:** `/uploads/bomb.zip` (or `.7z`, `.rar`) holds `zeros.bin`, declared and holding 10 000 000 zero bytes; the remaining storage or the session allowance is far lower.
**Flow:**
1. The member asks to extract it → the system reads the declared sizes, finds the limit exceeded and refuses with that limit's refusal.
**Postconditions:** no entry decoded; nothing written; remaining storage and allowance unchanged; no audit line.
**Exceptions:**
- EXC-002a: both limits are exceeded → the project refusal.
- EXC-002b: entries that each fit but together exceed → refused on the total.

### SC-003: Declared sizes that lie or cannot be counted
**Actor:** Attacker.
**Preconditions:** an archive whose entry declares a size different from its content, or so large it cannot be counted.
**Flow:**
1. A declared size of 9 223 372 036 854 775 808 bytes or more, or a declared total that large → refused at the limit, nothing decoded.
2. An entry holding more than it declares → decoding stops one chunk past the declared size and the call is refused as a size lie.
**Postconditions:** nothing written; the allowance used never goes down.

### SC-004: Order when several refusals apply
**Actor:** Member.
**Flow:**
1. A problem visible in the headers (membership, missing path, unreadable archive, link, escaping name, missing password) is reported before the limits.
2. The declared limits are reported before anything decoding finds (wrong password on data, damaged data, size lie) and before destination and collision.
3. A collision is still reported when the declared net growth fits.

### SC-005: Re extract with overwrite near the limit
**Actor:** Member.
**Preconditions:** `/uploads/pack.zip` holds `big.bin`, 3 000 000 bytes; `/uploads/pack/big.bin` already holds 3 000 000 bytes; 100 000 bytes of remaining storage.
**Flow:**
1. With `overwrite` true → the net growth is 0, the extraction succeeds.
2. With `overwrite` false → the collision refusal of today, not a limit refusal.
**Exceptions:**
- EXC-005a: the replacement is one byte larger than the remaining storage allows → the project refusal before decoding.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Declared figures computed before decoding
> WHEN a member extracts a `.zip`, `.7z` or `.rar` archive, in any letter case, THE archive extraction SHALL compute, from the archive's headers and before decoding any entry, the declared total and the net growth, and check them against the two limits.
- **Business rules:** the declared total is the sum of the declared sizes of every file entry, folders excluded, an entry counted each time it appears. The net growth is, for each target path, the declared size of the last entry targeting it minus the size of the file stored at that path now (a folder or nothing counts 0), whether `overwrite` is true or false. Target paths are computed from the destination the call will use; when that destination is refused later in the order, no stored file counts. In a 7z archive where several entries share one compressed block, each entry's own declared size counts, never the block's. Both figures are exact: never rounded, never wrapped.
- **Scenario:** SC-001, SC-002, SC-005
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Project refusal on the declared net growth
> IF the project has a storage limit AND the declared net growth exceeds the remaining storage THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and the message `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested`.
- **Business rules:** `<L>` is the limit in whole megabytes, `<U>` the bytes the project stores at the time of the check, `<R>` the declared net growth. Equal is accepted.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Session refusal on the declared total
> IF FR-NEW-002 did not refuse AND the declared total exceeds the member's remaining session write allowance THEN THE archive extraction SHALL refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`.
- **Business rules:** `<N>` is the configured allowance (52 428 800 bytes by default). Equal is accepted. When both limits are exceeded the project refusal is the one reported.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-UB]: A refusal on declared sizes decodes nothing and leaves no trace
> THE archive extraction SHALL NOT decode any entry, write any file, create any folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused under FR-NEW-002, FR-NEW-003 or FR-NEW-005.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Sizes too large to count
> IF one declared size, the declared total or the declared net growth exceeds 9 223 372 036 854 775 807 bytes THEN THE archive extraction SHALL refuse as FR-NEW-002 states when the project has a storage limit, with `<R>` reported as `9223372036854775807`, and as FR-NEW-003 states otherwise.
- **Business rules:** the largest size an archive can declare, 18 446 744 073 709 551 615 bytes, and a total of several sizes past the threshold are refused the same way.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-006 [EARS-UB]: A declared size never lowers a count
> THE archive extraction SHALL NOT count any declared size as a negative amount, and SHALL NOT lower the member's used session write allowance or the project's stored bytes because of an archive's declared sizes.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: An entry decoding past its declared size
> IF an entry of a `.zip`, `.7z` or `.rar` archive decodes to more bytes than it declares THEN THE archive extraction SHALL stop decoding it, refuse with `ERR_INVALID_ARGUMENT` and the message `archive entry '<path>' decompressed to more bytes than declared <D>`, and write nothing.
- **Business rules:** `<path>` is the entry's name in the archive, `<D>` its declared size. An entry declared 0 bytes that holds data is such an entry. The server holds at most `<D>` plus 65 536 bytes of that entry. An entry holding exactly its declared size is accepted.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-008 [EARS-U]: Bound on what decoding holds
> THE archive extraction SHALL hold, during one extraction of a `.zip`, `.7z` or `.rar` archive, at most the declared total plus 65 536 bytes of decoded entry data.
- **Business rules:** since the declared total passed FR-NEW-002 and FR-NEW-003, this is at most the lower of the two limits plus 65 536 bytes. A decoder that ignores a declared size does not lift the bound.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Order of refusals
> IF several refusals apply to the extraction of a `.zip`, `.7z` or `.rar` archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) RAR only: a multi volume name, or a server without RAR support (SPEC-0080 FR-NEW-010, FR-NEW-012); (5) the refusals read from the headers, in this order per format: zip: unreadable archive, then per entry in archive order a link, an escaping name or a duplicate name, then a missing password, then a local header disagreeing with the directory; 7z: an encrypted header without the right password or an unreadable header, then a missing password, then per entry in archive order an escaping name then a link or special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then a missing password for encrypted data; (6) the project storage limit on the declared net growth (FR-NEW-002, FR-NEW-005); (7) the session write allowance on the declared total (FR-NEW-003, FR-NEW-005); (8) what decoding finds, in archive order: a wrong password (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`), damaged data (`ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid <format> file:`), an entry past its declared size (FR-NEW-007); (9) the destination refusal of today; (10) collision (`ERR_NO_CLOBBER`, `'<path>' already exists; pass overwrite=true to replace it`); (11) the project storage limit then the session write allowance at the moment of writing, with the same figures, charged on success.
- **Business rules:** the messages of items 1 to 5 and 8 to 11 are unchanged. `<format>` is `zip`, `7z` or `rar`. Item 11 refuses only when the project's storage changed between item 6 and the write.
- **Scenario:** SC-004, SC-005
- **Priority:** Must

#### FR-NEW-010 [EARS-U]: The rest is unchanged
> THE archive extraction SHALL answer a `.zip`, `.7z` or `.rar` archive within the limits with the same destination, counts and audit line as before, SHALL charge the session write allowance the declared total on success as before, and SHALL extract or refuse plain `.tar`, compressed tar archives and bare `.gz`, `.bz2` and `.xz` files exactly as before this increment.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: Limits on declared sizes before decoding (modifies SPEC-0018/FR-NEW-012, the order of SPEC-0018/FR-NEW-019, SPEC-0080/FR-NEW-015)
> IF the declared net growth or the declared total of a `.zip`, `.7z` or `.rar` archive exceeds its limit THEN THE archive extraction SHALL refuse before decoding any entry, before the destination check and before the collision check.
- **Before:** every entry is decoded first; an over limit archive that also collides is refused with `ERR_NO_CLOBBER`, one that also has a wrong password with `ERR_PASSWORD_REQUIRED`, one with damaged data as corrupt, one with a refused destination with that refusal (Section 2.1).
- **After:** the statement above; those four cases are refused with the limit refusal. SPEC-0080 E2E-068 changes accordingly (E2E-MOD-001).
- **Reason:** BL-0125: a small archive can exhaust the server's memory before it is refused.

#### FR-MOD-002 [EARS-O]: 7z entry checks before decoding (modifies the timing of SPEC-0018/FR-NEW-013 and FR-NEW-014 for 7z)
> IF any entry of a `.7z` archive has an escaping name, a link type or another special type THEN THE archive extraction SHALL refuse it before decoding any entry.
- **Before:** each 7z entry is checked just before it is decoded, so an earlier entry's wrong password or damaged data is reported first.
- **After:** the statement above, as zip and RAR already do; the refusal codes and messages are unchanged.
- **Reason:** the declared limits come after the entry checks (FR-NEW-009 item 5 before item 6); without this move a hostile 7z entry is hidden behind a limit refusal, where zip and RAR report it.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN an extraction is refused on its declared sizes THE archive extraction SHALL have held 0 bytes of decoded entry data.

### 7.2 Security and access
- THE archive extraction SHALL require the same project membership as before.
- THE archive extraction SHALL NOT let an archive's declared sizes write more than the remaining storage or the remaining session write allowance.

### 7.5 Reliability
- IF an extraction is refused THEN THE archive extraction SHALL leave the project files, the remaining storage and the session write allowance unchanged.

### 7.6 Operability
- WHEN an extraction succeeds THE archive extraction SHALL record one audit line, as before; a refused one records none.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | entry declared 0 holding data; folders only | size lie refusal; folders count 0 | E2E-021, E2E-022; existing folder tests |
| Boundary values | net growth equal and one over; total equal to the allowance; tie between limits; entry exactly its declared size | accepted; refused; accepted; project refusal; accepted | E2E-003, E2E-011, E2E-004, E2E-009, E2E-023 |
| Concurrent actions | storage changing between the early check and the write | final admission (item 11) | existing SPEC-0020 E2E-051 |
| Slow or lost connectivity | not applicable: the archive is already stored | none | none |
| Access states | not a member | membership refusal first | E2E-035 |
| Data variations | zip64 sizes; sizes at and past 2^63; largest declarable size; upper case suffixes; duplicate entry names | counted exactly; refused; refused; detected; gross and net | E2E-017, E2E-SEC-001 to 005, E2E-002, E2E-013 |
| Navigation | not applicable | none | none |
| Partial operations | damaged data, wrong password, entry past its size | after the limits; decoding stops at the size plus one chunk | E2E-026, E2E-027, E2E-019, E2E-024 |
| Dependency failures | not applicable: no external service | none | none |
| Ordering and timing | limits versus link, escaping name, password, corruption, destination, collision | FR-NEW-009 | E2E-025 to E2E-036, E2E-MOD-001 |
| Exhaustion | honest bombs in each format; solid 7z block; many fitting entries | refused before decoding | E2E-006 to E2E-008, E2E-041, E2E-012 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are built by the design. Unless a test says otherwise, the project is `atlas`, the member `mia@example.com`, the session allowance is large enough not to matter and the project has no storage limit. "N bytes of remaining storage" sets a storage limit of 1 MB (1 048 576 bytes) and fills the project, the archive included, so that N bytes remain; the project then stores 1 048 576 minus N bytes. When the archive alone exceeds 1 MB the limit is the next whole megabyte and the test asserts the message prefix only. "Session allowance N" configures that allowance with nothing written yet. "Decoded bytes held" is the largest amount of decoded entry data the server holds during the call, read by the operator's measurement hook. A "bomb" is an archive holding `zeros.bin`, 10 000 000 zero bytes, declared as such.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-NEW-010 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-010 | High |
| E2E-003 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-004 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-005 | New | side effect | SC-001 | FR-NEW-010 | Medium |
| E2E-006 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001 | Critical |
| E2E-007 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-004 | Critical |
| E2E-008 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-004, FR-MOD-001 | Critical |
| E2E-009 | New | edge | SC-002 | FR-NEW-002, FR-NEW-003 | High |
| E2E-010 | New | edge | SC-002 | FR-NEW-003 | High |
| E2E-011 | New | failure | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-012 | New | failure | SC-002 | FR-NEW-001, FR-NEW-002 | High |
| E2E-013 | New | edge | SC-002 | FR-NEW-001, FR-NEW-003 | Medium |
| E2E-014 | New | happy | SC-005 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-015 | New | failure | SC-005 | FR-NEW-001, FR-NEW-009 | High |
| E2E-016 | New | failure | SC-005 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-017 | New | failure | SC-003 | FR-NEW-002, FR-NEW-004 | High |
| E2E-018 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-019 | New | failure | SC-003 | FR-NEW-007 | High |
| E2E-020 | New | failure | SC-003 | FR-NEW-007 | High |
| E2E-021 | New | failure | SC-003 | FR-NEW-007 | Medium |
| E2E-022 | New | failure | SC-003 | FR-NEW-007 | Medium |
| E2E-023 | New | edge | SC-003 | FR-NEW-007, FR-NEW-008 | Medium |
| E2E-024 | New | failure | SC-003 | FR-NEW-007, FR-NEW-008 | High |
| E2E-025 | New | failure | SC-004 | FR-NEW-009, FR-MOD-001 | High |
| E2E-026 | New | failure | SC-004 | FR-NEW-009, FR-MOD-001 | High |
| E2E-027 | New | failure | SC-004 | FR-NEW-009, FR-MOD-001 | High |
| E2E-028 | New | failure | SC-004 | FR-NEW-009, FR-MOD-001 | Medium |
| E2E-029 | New | failure | SC-004 | FR-NEW-009, FR-MOD-002 | High |
| E2E-030 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-031 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-032 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-033 | New | failure | SC-004 | FR-NEW-009, FR-MOD-002 | Medium |
| E2E-034 | New | failure | SC-004 | FR-NEW-009, FR-MOD-002 | Medium |
| E2E-035 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-036 | New | edge | SC-004 | FR-NEW-001, FR-NEW-009 | High |
| E2E-037 | New | failure | SC-001 | FR-NEW-010 | Medium |
| E2E-038 | New | happy | SC-001 | FR-NEW-010 | Medium |
| E2E-039 | New | edge | SC-002 | FR-NEW-002, FR-NEW-004 | Medium |
| E2E-040 | New | happy | SC-001 | FR-NEW-001, FR-NEW-008 | High |
| E2E-041 | New | failure | SC-002 | FR-NEW-001, FR-NEW-002 | High |
| E2E-042 | New | edge | SC-005 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-043 | New | happy | SC-005 | FR-NEW-001 | Medium |
| E2E-SEC-001 | New | failure, security | SC-003 | FR-NEW-002, FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-002 | New | failure, security | SC-003 | FR-NEW-003, FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-003 | New | failure, security | SC-003 | FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-004 | New | failure, security | SC-003 | FR-NEW-005 | Critical |
| E2E-SEC-005 | New | failure, security | SC-003 | FR-NEW-005, FR-NEW-006 | High |
| E2E-MOD-001 | Modified | failure | SC-004 | FR-NEW-009, FR-MOD-001 | High |

Coverage (49 tests: 48 new, 1 modified, 0 removed): happy 6, failure 33 (8 of them also security: E2E-006 to E2E-008, E2E-SEC-001 to E2E-SEC-005), side effect 1, edge 9, state transition 0 (no state machine). Happy to failure ratio 1:5.5.

### 9.2 New tests

#### E2E-001: zip within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-010; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/pack.zip` holding `a.txt` (`hello`); nothing at `/uploads/pack`
- **When** the member extracts `/uploads/pack.zip`
- **Then** the answer is destination `/uploads/pack`, 1 file written, 1 folder created, 5 bytes written
- **And** `/uploads/pack/a.txt` reads `hello`; the remaining storage is 995 bytes

#### E2E-002: 7z and RAR within the limits, upper case suffixes
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/u1/PACK.7Z` and, on a server with RAR support, `/u2/pack.RAR`, each holding `a.txt` (`hello`)
- **When** the member extracts each
- **Then** `/u1/PACK/a.txt` and `/u2/pack/a.txt` read `hello`, each answer reporting 1 file and 5 bytes written

#### E2E-003: Net growth equal to the remaining storage
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 5 bytes of remaining storage and `/uploads/pack.zip` of E2E-001
- **When** the member extracts it
- **Then** the extraction succeeds with 5 bytes written; the remaining storage is 0 bytes

#### E2E-004: Declared total equal to the session allowance
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** session allowance 12 and, on a server with RAR support, `/uploads/report.rar` holding folder `data`, `data/a.txt` (`hello`) and `data/b.txt` (`goodbye`)
- **When** the member extracts it
- **Then** the extraction succeeds: 2 files, 12 bytes written
- **And** a following write of a 1 byte file `/uploads/one.bin` is refused with `session write quota of 12 bytes exceeded`

#### E2E-005: HTTP API and audit line unchanged
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/pack.zip`
- **Then** the answer is that of E2E-001
- **And** the audit trail has exactly one extraction line for `/uploads/pack.zip` naming destination `/uploads/pack`, 1 file written and 5 bytes written

#### E2E-006: zip bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.zip`, a bomb of at most 20 000 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 10000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes; the audit trail has no extraction line for `/uploads/bomb.zip`

#### E2E-007: 7z bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** session allowance 1 000 and `/uploads/bomb.7z`, a bomb
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; a following write of a 1 000 byte file `/uploads/small.bin` succeeds

#### E2E-008: RAR bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar`, a bomb
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes

#### E2E-009: Tie between the two limits
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 1 000 bytes of remaining storage, session allowance 1 000, and `/uploads/bomb.zip` of E2E-006
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both are unchanged

#### E2E-010: The session allowance is lower
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** a storage limit of 3 MB with 2 000 000 bytes remaining, session allowance 1 000, and `/uploads/mid.zip` holding `a.bin`, 1 500 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded`
- **And** the decoded bytes held are 0; the remaining storage is still 2 000 000 bytes

#### E2E-011: One byte over the remaining storage
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/over.zip` holding `a.bin`, 1 001 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 1001 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/over` does not exist

#### E2E-012: Entries that each fit but not together
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/three.zip` holding `a.bin`, `b.bin` and `c.bin`, 400 bytes each
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 1200 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/three` does not exist

#### E2E-013: Repeated entry names: net for storage, gross for the allowance
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage, session allowance 1 000, and `/uploads/dup.rar` holding `a.txt` twice, 600 bytes each
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded` (gross 1 200), not the project refusal (net 600)
- **And** the decoded bytes held are 0; `/uploads/dup` does not exist

#### E2E-014: Re extract with overwrite near the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** 100 000 bytes of remaining storage (limit large enough for the files below), `/uploads/pack.zip` holding `big.bin` (3 000 000 bytes of the text `abc` repeated), and `/uploads/pack/big.bin` already holding the same 3 000 000 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds with 1 file and 3 000 000 bytes written
- **And** the remaining storage is still 100 000 bytes

#### E2E-015: Same situation without overwrite
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** High
- **Given** the situation of E2E-014
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/pack/big.bin' already exists; pass overwrite=true to replace it`, not a limit refusal

#### E2E-016: Files stored elsewhere do not count
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-014, the same 3 000 000 bytes stored at `/elsewhere/big.bin`, nothing under `/uploads/pack`, 100 000 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `3000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/pack` does not exist

#### E2E-017: zip64 size against the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/big64.zip` whose only entry `huge.bin` declares 5 000 000 000 bytes in its zip64 record and holds the 5 bytes `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 5000000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/big64` does not exist

#### E2E-018: zip64 size against the default allowance
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the default session allowance and `/uploads/big64.zip` of E2E-017
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`; the decoded bytes held are 0

#### E2E-019: zip entry holding more than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/liar.zip` whose entry `big.bin` declares 10 bytes and holds 1 000 000 bytes of the byte 7
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'big.bin' decompressed to more bytes than declared 10`
- **And** the decoded bytes held never exceed 65 546; `/uploads/liar` does not exist

#### E2E-020: RAR entry holding more than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** a server with RAR support and `/uploads/liar.rar` built as in E2E-019
- **When** the member extracts it
- **Then** the refusal is as in E2E-019; the decoded bytes held never exceed 65 546

#### E2E-021: zip entry declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/zero.zip` whose entry `a.txt` declares 0 bytes and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'a.txt' decompressed to more bytes than declared 0`; `/uploads/zero` does not exist

#### E2E-022: RAR entry declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** a server with RAR support and `/uploads/zero.rar` built as in E2E-021
- **When** the member extracts it
- **Then** the refusal is as in E2E-021

#### E2E-023: Entry holding exactly its declared size
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/exact.zip` whose entry `exact.bin` declares 10 bytes and holds 10 bytes of the byte 7
- **When** the member extracts it
- **Then** the extraction succeeds with 10 bytes written; the decoded bytes held are 10

#### E2E-024: The whole call is bounded by the declared total
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** High
- **Given** `/uploads/mixed.zip` holding `a.bin`, 600 000 bytes declared and held, then `b.bin`, declaring 10 bytes and holding 5 000 000 bytes of the byte 7
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'b.bin' decompressed to more bytes than declared 10`
- **And** the decoded bytes held never exceed 665 546; `/uploads/mixed` does not exist

#### E2E-025: The limit wins over a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.zip` of E2E-006 and an existing `/uploads/bomb/zeros.bin` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9999997 bytes requested`, not `ERR_NO_CLOBBER`
- **And** `/uploads/bomb/zeros.bin` still reads `old`; the decoded bytes held are 0

#### E2E-026: The limit wins over a wrong password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/vault.zip`, a bomb whose entry is encrypted with the password `s3cret`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_PASSWORD_REQUIRED`; the decoded bytes held are 0

#### E2E-027: The limit wins over damaged data
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.zip` of E2E-006 with the first 16 bytes of its compressed data inverted, headers intact
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not a message starting `archive is corrupt or not a valid zip file:`

#### E2E-028: The limit wins over a refused destination
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` of E2E-008
- **When** the member extracts it to the destination `../../x`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_PATH_OUT_OF_BOUNDS`; neither `/x` nor `/uploads/bomb` exists

#### E2E-029: A 7z link entry wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.7z` holding `zeros.bin` of a bomb, then a symbolic link entry `link`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `archive entry 'link' is a symlink, which is not supported`
- **And** the decoded bytes held are 0

#### E2E-030: A RAR escaping name wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` holding `zeros.bin` of a bomb, then an entry named `../../etc/passwd`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with `archive entry path escapes the destination: '../../etc/passwd'`

#### E2E-031: A missing password wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/vault.7z`, a bomb encrypted with `s3cret`, header in clear
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; the decoded bytes held are 0

#### E2E-032: An unreadable archive wins over everything after it
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/cut.zip`, the first 100 bytes of `/uploads/bomb.zip` of E2E-006
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid zip file:`

#### E2E-033: A 7z escaping entry wins over earlier damaged data
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/mix.7z` holding `a.txt` (`hello`) with its compressed data damaged, then an entry named `../evil.txt` (`x`), the two in separate blocks
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with `archive entry path escapes the destination: '../evil.txt'`, not the corrupt refusal

#### E2E-034: A 7z special entry wins over a wrong password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/vault.7z` encrypted with `s3cret`, header in clear, holding `a.txt` (`hello`) then a symbolic link entry `link`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `archive entry 'link' is a symlink, which is not supported`, not `ERR_PASSWORD_REQUIRED`

#### E2E-035: Membership wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.zip` of E2E-006, and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`; the decoded bytes held are 0

#### E2E-036: A collision still wins when the net growth fits
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** High
- **Given** a server with RAR support, 9 bytes of remaining storage, `/uploads/report.rar` of E2E-004 (12 bytes declared) and `/uploads/report/data/a.txt` already holding `old` (3 bytes), so the net growth is 9
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with `'/uploads/report/data/a.txt' already exists; pass overwrite=true to replace it`
- **And** `/uploads/report/data/a.txt` still reads `old`; the remaining storage is still 9 bytes

#### E2E-037: Plain tar over the storage limit unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** 4 bytes of remaining storage and `/uploads/pack.tar` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`; `/uploads/pack` does not exist

#### E2E-038: Tar family and bare files unchanged
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/t1/pack.tar`, `/t2/pack.tar.gz` and `/t3/a.txt.gz`, the first two holding `a.txt` (`hello`), the third being `hello` compressed
- **When** the member extracts each
- **Then** `/t1/pack/a.txt`, `/t2/pack/a.txt` and `/t3/a.txt` read `hello`, each answer reporting 1 file and 5 bytes written

#### E2E-039: A repeated refusal stays a refusal and changes nothing
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** Medium
- **Given** the situation of E2E-006
- **When** the member extracts `/uploads/bomb.zip` twice
- **Then** both refusals carry the message of E2E-006
- **And** the remaining storage is still 1 000 bytes; extracting `/uploads/pack.zip` of E2E-001 right after succeeds and leaves 995 bytes

#### E2E-040: Solid 7z within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-008; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/solid.7z` holding `a.txt` (`hello`), `b.txt` (`world`) and `c.txt` (`again`) in one compressed block
- **When** the member extracts it
- **Then** the extraction succeeds: 3 files, 15 bytes written; the decoded bytes held never exceed 15; the remaining storage is 985 bytes

#### E2E-041: Solid 7z bomb counted by its entries
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/solid.7z` holding `a.bin`, `b.bin` and `c.bin`, 4 000 000 zero bytes each, in one compressed block
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `12000000 bytes requested`; the decoded bytes held are 0

#### E2E-042: Replacement one byte over
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** 100 000 bytes of remaining storage, `/uploads/pack/big.bin` holding 3 000 000 bytes, and `/uploads/pack.zip` holding `big.bin` of 3 100 001 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `100001 bytes requested`; the decoded bytes held are 0; `/uploads/pack/big.bin` is unchanged

#### E2E-043: Explicit destination holding the files
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** the archive of E2E-014, `/out/big.bin` holding the same 3 000 000 bytes, nothing under `/uploads/pack`, 100 000 bytes of remaining storage
- **When** the member extracts it to the destination `/out` with `overwrite` true
- **Then** the extraction succeeds; the remaining storage is still 100 000 bytes

### 9.3 Exploit tests

Each payload is the size an attacker writes into the archive's header. Each test is red before the fix because the size is counted as a negative figure today (Section 14.2 occurrences 1 to 3). Occurrence 4 (memory) is attacked by E2E-006, E2E-007 and E2E-008 in Section 9.2, each with its own red reason.

#### E2E-SEC-001: Size of 2^63 against the storage limit
- **Covers:** class sweep occurrences 1 and 2, zip (Section 14.2)
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: today the call answers 1 file and 10 000 000 bytes written.
- **Given** 1 000 bytes of remaining storage and `/uploads/free.zip` whose entry `free.bin` declares, in its zip64 record, `9223372036854775808` bytes and holds 10 000 000 zero bytes, with a matching checksum
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used, and `/uploads/free/free.bin` does not exist when listed by a second member of `atlas`

#### E2E-SEC-002: Size of 2^63 against the session allowance
- **Covers:** occurrences 1 and 3, zip
- **Red before the fix because:** `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: today the call succeeds, and the later 1 001 byte write succeeds because the used allowance went below zero.
- **Given** a project without a storage limit, session allowance 1 000, and `/uploads/free.zip` of E2E-SEC-001
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded`
- **And** a following write of a 1 001 byte file `/uploads/w.bin` is refused with `session write quota of 1000 bytes exceeded`, and a write of a 1 000 byte file succeeds

#### E2E-SEC-003: The largest declarable size
- **Covers:** occurrence 1, zip
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: the size counts as minus 1 byte today and the call writes `hello`.
- **Given** 1 000 bytes of remaining storage and `/uploads/max.zip` whose entry `a.txt` declares `18446744073709551615` bytes and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used

#### E2E-SEC-004: Two sizes whose sum wraps
- **Covers:** occurrence 2, zip
- **Red before the fix because:** `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: today the sum of the two sizes overflows: the call fails with an internal error or counts a negative total and writes both files.
- **Given** a project without a storage limit and `/uploads/wrap.zip` holding `a.txt` (`a`) and `b.txt` (`b`), each declaring `9223372036854775807` bytes in its zip64 record
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** a following write of a 52 428 800 byte file succeeds, so nothing was charged; `/uploads/wrap` does not exist

#### E2E-SEC-005: 7z size of 2^63
- **Covers:** occurrence 1, 7z
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: today the call writes `free.bin` (Section 14.3 asks the design to confirm the fixture reproduces it).
- **Given** 1 000 bytes of remaining storage and `/uploads/free.7z` whose entry `free.bin` declares `9223372036854775808` bytes with no checksum and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used; `/uploads/free` does not exist

### 9.4 Modified tests

#### E2E-MOD-001: The storage limit wins over a collision (was SPEC-0080 E2E-068, "Collision wins over the storage limit")
- **Validated before:** with 5 bytes remaining, 12 declared and `/uploads/report/data/a.txt` existing, the refusal was `ERR_NO_CLOBBER`.
- **Validates now:** the declared net growth (12 minus the 3 bytes of `old`, so 9) exceeds 5, so the limit is reported first; E2E-036 keeps the collision first when the growth fits.
- **Given** a server with RAR support, 5 bytes of remaining storage, `/uploads/report.rar` of E2E-004 and `/uploads/report/data/a.txt` holding `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9 bytes requested`
- **And** `/uploads/report/data/a.txt` still reads `old`; the remaining storage is still 5 bytes

### 9.5 Removed tests

None. SPEC-0080 E2E-066 (missing password wins over an oversized entry) and E2E-067 (oversized entry wins over a collision) keep their assertions: neither sets a limit their declared 10 bytes exceed.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-008, FR-NEW-010 | E2E-001, E2E-002, E2E-038, E2E-040 | E2E-011, E2E-037 | E2E-003, E2E-004 | E2E-005 |
| SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-MOD-001 | E2E-003 (success branch, SC-001) | E2E-006, E2E-007, E2E-008, E2E-012, E2E-041 | E2E-009, E2E-010, E2E-013, E2E-039 | E2E-007 (later write succeeds) |
| SC-003 | FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-008 | E2E-023 (success branch, categorized edge) | E2E-017 to E2E-022, E2E-024, E2E-SEC-001 to E2E-SEC-005 | E2E-023 | E2E-SEC-002 (later writes) |
| SC-004 | FR-NEW-001, FR-NEW-009, FR-MOD-001, FR-MOD-002 | E2E-001 (success branch, SC-001) | E2E-025 to E2E-035, E2E-MOD-001 | E2E-036 | E2E-025 (file kept) |
| SC-005 | FR-NEW-001, FR-NEW-002, FR-NEW-009 | E2E-014, E2E-043 | E2E-015, E2E-016 | E2E-042 | E2E-014 (storage unchanged) |

SC-002, SC-003 and SC-004 are refusal scenarios; their success branch is named.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-001, E2E-003, E2E-006, E2E-011, E2E-012, E2E-013, E2E-014, E2E-015, E2E-016, E2E-036, E2E-040, E2E-041, E2E-042, E2E-043 | 14 |
| FR-NEW-002 | E2E-003, E2E-006, E2E-008, E2E-009, E2E-011, E2E-012, E2E-014, E2E-016, E2E-017, E2E-039, E2E-041, E2E-042, E2E-SEC-001 | 13 |
| FR-NEW-003 | E2E-004, E2E-007, E2E-009, E2E-010, E2E-013, E2E-018, E2E-SEC-002 | 7 |
| FR-NEW-004 | E2E-006, E2E-007, E2E-008, E2E-017, E2E-039 | 5 |
| FR-NEW-005 | E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-004, E2E-SEC-005 | 5 |
| FR-NEW-006 | E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-005 | 4 |
| FR-NEW-007 | E2E-019, E2E-020, E2E-021, E2E-022, E2E-023, E2E-024 | 6 |
| FR-NEW-008 | E2E-023, E2E-024, E2E-040 | 3 |
| FR-NEW-009 | E2E-015, E2E-025 to E2E-036, E2E-MOD-001 | 14 |
| FR-NEW-010 | E2E-001, E2E-002, E2E-005, E2E-037, E2E-038 | 5 |
| FR-MOD-001 | E2E-006, E2E-008, E2E-025, E2E-026, E2E-027, E2E-028, E2E-MOD-001 | 7 |
| FR-MOD-002 | E2E-029, E2E-033, E2E-034 | 3 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Entry | one file or folder inside an archive | archive extraction |
| Declared size | the size an archive's headers announce for an entry's content, readable before decoding | archive extraction |
| Declared total | the sum of the declared sizes of every file entry, each repeat counted | archive extraction |
| Decoding | turning an entry's stored data back into its content, with decompression and decryption | archive extraction |
| Decoded bytes held | the largest amount of decoded entry data the server holds during one call | archive extraction |
| Chunk | the fixed 65 536 bytes decoding reads at most past a limit before it stops | archive extraction |
| Bomb | a small archive declaring and holding a huge content, here 10 000 000 zero bytes | archive extraction |
| zip64 | the zip extension that lets an entry declare a size above 4 GB | archive extraction |
| Compressed block | a 7z unit compressing several entries together (a solid archive) | archive extraction |
| Size lie | an entry holding more bytes than it declares | archive extraction |
| Storage limit | the optional cap on the bytes a project stores, set in whole megabytes | project storage |
| Remaining storage | the storage limit minus the bytes stored | project storage |
| Session write allowance | the bytes a member can still write in the current session; always set, 52 428 800 bytes by default | project storage |
| Net growth | for each target path, the last declared size minus the stored file it replaces, summed | project storage |
| Destination | the folder that receives the entries; by default the archive path without its suffix | archive extraction |
| Collision | an entry's target path already holds a file | project storage |
| Overwrite | the choice to replace colliding files instead of refusing | project storage |
| Link entry | an entry that is a symbolic link; refused | archive extraction |
| Special entry | an entry that is a link, a device, a FIFO or a socket; refused | archive extraction |
| Escaping name | an entry name that is absolute or climbs above the destination; refused | archive extraction |
| Refusal | an error answer; the project is left unchanged | archive extraction |
| Audit line | the one line an operator reads to see who extracted what and where | archive extraction |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server, watches its memory and reads its audit trail | archive extraction |
| Attacker | a person crafting an archive to exhaust memory or pass the limits | archive extraction |
| HTTP API | the web endpoint a program calls to extract an archive; the second door | archive extraction |
| Assistant tool | the tool an AI assistant calls to extract an archive | archive extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 6 | IMPLEMENTABLE |

**Amendments applied after round 1 (all A, amended in place):** A-1 citations recounted (14.1 rows 1 to 6, 14.2 occurrences 2 and 4, 14.3: `quota.rs:156`, `archive.rs:202-211`, `:223-227`, `:506-527`, `:661-662`, `archive_rar.rs:60-87`, `:69`, `dataplane.rs:176`); A-2 occurrence 4 exploit tests E2E-006 to E2E-008 given their `Covers` and red reasons; A-3 BL-0134 written with `Nature: BUG`, Evidence and Decision; A-4 the short zip64 entry acceptance marked ASSUMED with a design input and its signal; A-5 the SPEC-0080 FR-NEW-014 wording deviation stated in 2.2; A-6 a folder at a target path counts 0 in the net growth. Consistency fixes: E2E-010 rewritten (it contradicted the project first rule), E2E-042 recategorized edge, coverage line recomputed, two forbidden modals removed.
**How the gates ran:** this harness exposes no subagent tool, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks for ids, categories, requirement coverage, traceability counts, modals and implementation terms. Recorded as a concern in the handoff.
**No implementation detail outside Section 14:** PASS. Option names (`destination`, `overwrite`, password), error codes, verbatim messages, file name suffixes and the zip64 format term are WHAT.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | formats, two doors, RAR behind the `rar` feature, membership first | `crates/core/src/tools/archive.rs:44-63`, `:143-151`; `crates/core/src/mcp/server.rs:2373-2379` (`authorize_only` then `extract_archive`); `crates/core/src/api/dataplane.rs:176`, `:876-878` (same function) |
| 2 | read whole, decode every entry in `spawn_blocking`, then destination, collision, admission | `crates/core/src/tools/archive.rs:202-211`, `:223-245`, `:247-262`; `:376-389` (`decode_archive`); `crates/core/src/tools/archive_rar.rs:14-17` (doc comment naming BL-0125) |
| 3 | admission figures, project first, equality, messages, default 50 MiB | `crates/core/src/tools/archive.rs:247-262`; `crates/core/src/quota.rs:96-113` (`check_and_charge`), `:136-159` (`admit_paths`, last occurrence per path, previous size whatever `overwrite`); `crates/core/src/safety.rs:135-147`; `crates/core/src/errors.rs:103-111`; `crates/core/src/config.rs:382`, `:437`, `:1243` (52 428 800) |
| 4 | declared size cast to signed, unchecked sums | `crates/core/src/tools/archive.rs:251` (`e.declared_size as i64`), `:252` (`.sum()`); `crates/core/src/quota.rs:156` (`delta += new_size - previous`), `:108` (`used + delta`); `crates/core/src/safety.rs:139` (`s.bytes_written + num_bytes`). ASSUMED (read, not run): a zip64 entry declaring 2^63 with a matching CRC passes `zip` 9 (no shortfall check, `zip-9.0.5/src/crc32.rs:43-50` only checks the excess) |
| 5 | per entry bound by the libraries; messages; RAR exact size | zip: `zip-9.0.5/src/crc32.rs:43-50`, `crates/core/src/tools/archive.rs:410-424` (`zip_read_error`); 7z: `sevenz-rust2-0.20.2/src/reader.rs:1660-1666` (`BoundedReader` at `file.size`, CRC when present), `crates/core/src/tools/archive.rs:661-666`; RAR: `crates/core/src/tools/archive_rar.rs:60-87`, `:219-223` |
| 6 | order per format | zip `crates/core/src/tools/archive.rs:490-591`; 7z `:593-683` (entry checks inside `for_each_entries`, `:637-647`); RAR `crates/core/src/tools/archive_rar.rs:18-90`; then `archive.rs:225-262` |
| 7 | tar family, bare files | `crates/core/src/tools/archive.rs:720-793`; `crates/core/src/tools/archive_single.rs:204-221`, `:255-276`; SPEC-0084 |
| 8 | answer and audit on success only | `crates/core/src/tools/archive.rs:313-357` |
| 9 | governing specs | `sdlc/specs/SPEC-0018-archive-extract-in-place/spec.md:203-211`; `sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md:246-262`, `:923-927` (E2E-068); `sdlc/specs/SPEC-0084-tar-family-bounded-decoding/sdlc-spec.md` FR-NEW-001 |
| 10 | tests: 104 in `archive.rs`, 78 in `archive_rar.rs`, 160 in `archive_single.rs` (`rg -c '#\[(tokio::)?test'`); limit tests `archive.rs:1783`, `:1806`, `quota_e2e.rs:1227`, `:1244`, `:1269`, `:2094`, `archive_rar.rs:595-680`, `:1334-1356`; no zip64, 2^63 or held bytes assertion on these paths (`rg -n 'zip64\|i64::MAX\|LAST_HIGH_WATER' crates/core/src/tools/archive.rs crates/core/src/tools/archive_rar.rs` finds none) | as cited |

Test command: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar tools::archive_rar`; quota tests `cargo test -p mcp-fs-core --lib tools::quota_e2e`; full gate `cargo test --workspace`.

### 14.2 Class sweep

**Class:** sizes declared inside an uploaded compressed container reaching a signed byte count or an in memory decode with no bound (CWE-190, CWE-409).

**Searches run:**
1. `rg -n 'as i64' crates/core/src | rg -i 'size|len|declared|bytes'`: idiom "unsigned size cast to a signed count". Every hit but one converts the length of a buffer already in memory (at most the largest signed size, so it cannot wrap); the one hit converting a header value is occurrence 1.
2. `rg -n 'by_name|by_index|ZipArchive::new|sevenz_rust2::|unrar::|tar::Archive' crates/core/src --glob '*.rs' | rg -v 'tools/archive(_rar|_single)?\.rs'`: idiom "a container entry decoded outside the extraction". Hits in tests and in readers of our own export zips are safe (inputs built by the server); occurrence 5 is the one reader of uploaded files.
3. `rg -n 'with_capacity\(' crates/core/src | rg -iv 'len\(\)|count|cap\b|CHUNK|paths|entries|archive\.len|names|\d+\)'`: idiom "allocation sized from a parsed header". `git/http/pktline.rs:38` sizes from a buffer already capped (`MAX_PAYLOAD`); the others size from constants or in memory lengths.

Not searched: decoders inside third party libraries (libgit2 pack inflation, PDF stream inflation in the PDF text extractor): their input is the stored file and their sizes are not read by our code; a library level bound is a separate concern.

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | `crates/core/src/tools/archive.rs:251` declared size cast to signed | vulnerable (zip, 7z; tar and RAR refuse a short entry first: `archive.rs:779-781`, `archive_rar.rs:83`) | FR-NEW-005, FR-NEW-006; E2E-SEC-001, 003, 005 |
| 2 | `crates/core/src/tools/archive.rs:252` and `crates/core/src/quota.rs:156`, `:108` unchecked sums | vulnerable through 1 | FR-NEW-005; E2E-SEC-001, 004 |
| 3 | `crates/core/src/safety.rs:139` session sum lowered by a negative charge | vulnerable through 1 | FR-NEW-006; E2E-SEC-002 |
| 4 | `crates/core/src/tools/archive.rs:555-559`, `:571-574` (zip), `:661-662` (7z), `crates/core/src/tools/archive_rar.rs:69` (RAR): every entry decoded before any limit | vulnerable (memory) | FR-NEW-001 to FR-NEW-004, FR-NEW-008; E2E-006 to E2E-008, E2E-041 |
| 5 | `crates/core/src/docs/extract.rs:563-568` (`entry_text`): an Office document part read whole | vulnerable (memory) | BL-0134 |
| 6 | `crates/core/src/tools/archive.rs:727-749` compressed tar read whole | vulnerable (memory) | SPEC-0084 FR-NEW-001 |
| 7 | `crates/core/src/core/fs_ops.rs:578`, `:613`, `crates/core/src/tools/archive_single.rs:278` and the other `.len() as i64` hits | safe: the length of a buffer in memory cannot exceed the largest signed size (`Vec` capacity limit) | none |

### 14.3 Design inputs

- The early check needs the destination and the target paths before decoding. Today the destination is computed after decoding (`archive.rs:223-227`) and its refusal keeps its place after decoding (FR-NEW-009 item 9). Signal: E2E-028, E2E-036.
- The early check computes exactly `admit_paths`' figures without charging and without holding the admission lock across decoding; the final `admit_paths` stays as today (`archive.rs:247-262`, `quota.rs:136-159`). Signal: SPEC-0020 E2E-051 race stays green.
- The sums must be checked and saturate at `i64::MAX` for the message (FR-NEW-005); `admit_paths` and `charge_write` take `i64`. Signal: E2E-SEC-001 to 004.
- The zip and 7z listings already expose declared sizes before decoding (`archive.rs:506-527`; `sevenz_rust2::Archive::files` after `Archive::read`, `archive.rs:597`). RAR exposes `unpacked_size` in the listing pass (`archive_rar.rs:37-41`). The 7z entry checks move to that listing (FR-MOD-002), `archive.rs:637-647`.
- "Decoded bytes held" needs a hook like `LAST_HIGH_WATER` (`archive_single.rs:56-61`) on the three decoders, reading 0 for a call refused before decoding. Signal: every test asserting held bytes.
- The per entry cap and the total cap (FR-NEW-007, FR-NEW-008) can use a capped reader like `read_capped` (`archive_single.rs:92-108`, `CHUNK` 65 536 at `:14`) for zip and 7z; RAR reads through UnRAR's callback into a growing buffer (`unrar-0.5.8/src/open_archive.rs:384-386`, `:480-483`), already stopped at the declared size by UnRAR.
- 7z today can only report a size lie as a damaged block, because the reader takes exactly the declared size (14.1 row 5); the FR-NEW-007 message applies when the design's own cap fires.
- E2E-SEC-001 to E2E-SEC-004 rest on `zip` 9 accepting a zip64 entry that holds fewer bytes than it declares (14.1 row 4, ASSUMED): the design reproduces it first; if `zip` refuses it, that refusal is recorded as the control and the tests stay as guards.
- E2E-SEC-005: the 7z fixture needs a header declaring 2^63 with no CRC; the design confirms it is red today before relying on it, and if the 7z reader already refuses it, records that as the control and keeps the test as a guard.
- The existing RAR test `archive_rar.rs:1334` (`e2e_0080_068_collision_wins_over_the_storage_limi`) is rewritten as E2E-MOD-001; the archive module comment at `archive_rar.rs:14-17` names BL-0125 and needs updating.
- Docs to update: `.agent_docs/tools.md` (archive section, order of refusals), `README.md`, `AGENTS.md` project structure line for `archive.rs`; `TOOL_CONTRACT.txt` only if a description changes (none planned).

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the backlog entry, without the user, under their authorization.

- R1 scope: zip, 7z, RAR admitted on declared sizes before decoding; per entry and total caps; tar out (SPEC-0084); destination clamping out (BL-0127); RAR truncation out (BL-0131).
- R1 why now: BL-0125, deferred from SPEC-0080; SPEC-0084 left these formats to it.
- R2 current state: Section 2 and 14.1, read from the code.
- R3 scenarios: SC-001 to SC-005, failures from today's refusals and the sweep.
- R4 requirements: derived, Section 6. R5 NFR: memory held 0 on early refusal, no trace, audit unchanged.
- D-1 routing: BL-0125 reads `Nature: BUG`; the brief says DEBT; chosen FEAT because observable behavior changes (refusal before decoding, reordered refusals, uncountable sizes refused) and SPEC-0018 FR-NEW-012 mandates the full decode, so no authority is violated by the memory use. Nature: FEAT.
- D-2 depth M: one module, 12 requirements, public behavior change, no persisted change.
- D-3 security `internal finding`: the signed cast lets declared sizes bypass the storage limit and the session allowance (an access control on storage), which is more than resource exhaustion; class sweep and exploit tests included.
- D-4 the early check is a second check, not a move: the final admission keeps charging after the collision check, so concurrent writes stay covered.
- D-5 the net growth subtracts stored files whatever `overwrite` says, as the admission does today, so a collision keeps its refusal whenever the growth fits (E2E-015, E2E-036).
- D-6 a refused destination makes stored files count for nothing in the early check, as SPEC-0084 FR-NEW-002 does.
- D-7 an uncountable figure is refused at the limit, not as corrupt, with `<R>` reported as `9223372036854775807`: the archive is well formed, it is too large.
- D-8 entry checks read from headers come before the limits for all three formats; 7z entry checks move before decoding (FR-MOD-002).
- D-9 chunk 65 536 bytes, the same as SPEC-0081 to SPEC-0084.
- D-10 the 7z size lie message is the zip and RAR one.
- D-11 Office documents deferred to BL-0134 (Nature BUG) under the user's blanket authorization; it stays exploitable until BL-0134 is specified.
- D-12 tests: 48 new, 1 modified (SPEC-0080 E2E-068), 0 removed; test plan approved by the instruction "spec all of them, design later".
