# Compare two text files: functional specification

> Id: SPEC-0045
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 3 areas touched (file operations, access control, programmatic REST access), 18 requirements, no persisted change. No override.
> Status: designed
> Generated on: 2026-10-19
> Project: existing
> From backlog: BL-0010
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A project member can ask the product for the differences between two text files, in one call, without
reading both files and comparing them on their own side. The two files can live in the same project or in two
different projects the member can read. The answer is a standard unified diff, or a clear statement that the
files are identical. Binary files are not supported. The same capability is offered to programs that use the
product's web interface, with the same answer.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today
- A member can read one file at a time, in pages of at most 2000 lines, or several files of one project in
  one call. Nothing compares two files.
- A member sees a diff only as a side effect of changing a file: writing over a file, editing it, or applying
  several edits returns the difference between the old and the new content of that one file, 3 unchanged
  lines around each change, with the file path on both header lines.
- Every file operation works inside exactly one project. No operation takes two projects.
- The product has no notion of a "text file". A read decodes any file as text and silently replaces bytes it
  cannot decode.
- A read never refuses a file because of its size.
- A read counts toward a file's last access date (used by the automatic clean up) and counts as having read
  the file for the "read before edit" rule.
- A line ends with a line feed, a carriage return plus line feed, or a lone carriage return. The existing diff
  keeps these endings, so two lines that differ only by ending are different lines.
- The product publishes a fixed catalogue of 108 operations; tests compare it to a frozen contract.

### 2.2 Specs governing this area
- SPEC-0004 (REST data plane): the web interface offers the same file operations as the tool surface, with
  identical answers. This increment follows it and deviates nowhere. SPEC-0004 is not modified.
- SPEC-0001 (tool annotation hints): a read only tool carries the read only hints. This increment follows it.

### 2.3 What is verified today
The diff engine has 12 unit tests. The write and edit operations are tested through their diff output. No test
exercises a comparison of two files, a refusal of a non text file, or a size limit on reads, because none of
these exist. Command and files: Section 14.1.

## 3. Scope

### 3.1 In scope
- Comparing two text files, each identified by project and path, same project or different projects.
- A unified diff answer with 3 unchanged lines of context, or an "identical" answer.
- Refusal of missing files, directories, non text files, files over the size limit, and files the caller
  cannot read.
- An operator setting for the size limit, default 5 MiB per file.
- The same capability and the same answers through the web interface.

### 3.2 Out of scope, and why
- Binary files: asked for out of scope by the requester.
- Choosing the number of context lines: nobody needs another value yet; deferred as BL-0069.
- Comparing a file with an older version of itself or with a git revision: not requested.
- Comparing directories: not requested.
- Changing how the project is addressed (BL-0067 plans to move the project identifier out of tool arguments):
  this increment follows today's addressing.

### 3.3 Constraints imposed by the context
- The requester imposes that only text files are supported.
- The product's own rules impose that the tool surface and the web interface give the same answer and that
  access to a project is always checked first (Section 14.1).

## 4. Actors
- **Member:** a person who belongs to a project and can read its files. Uses the tool surface through an agent.
- **Program client:** a program calling the web interface on behalf of a member.
- **Operator:** the person who runs the server and sets its limits.
- **Non member:** a person who is not in a project; never sees its content.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| File storage | the files of a project and their text content | file, directory, last access, size |
| Access control | who may read which project | member, project, refusal |
| Programmatic access | the web interface of the same capabilities | same answer, same refusal |
| Safety | rules that protect users from their own edits | read before edit rule |

## 5. Usage scenarios

### SC-001: Compare two files of one project
**Actor:** Member
**Preconditions:** Alice (alice@example.com) is a member of project `alpha`. `alpha` holds `/notes/a.txt`
with the lines `one`, `two`, `three` and `/notes/b.txt` with the lines `one`, `2`, `three`.
**Flow:**
1. Alice asks to compare `alpha:/notes/a.txt` with `alpha:/notes/b.txt` → the product shows a unified diff
   whose first header line names the first file, second header line names the second file, with one change
   (`two` replaced by `2`) and the unchanged lines around it.
**Postconditions:** Nothing changed in `alpha`. Both files now have a fresh last access date.
**Exceptions:**
- EXC-001a: `/notes/b.txt` does not exist → refused with the missing file named (FR-NEW-006).
- EXC-001b: `/notes` is given instead of a file → refused as a directory (FR-NEW-007).
**Interactions:** SC-003, SC-004.

### SC-002: Compare files of two projects
**Actor:** Member
**Preconditions:** Alice is a member of `alpha` and `beta`. `alpha` holds `/cfg/app.txt`, `beta` holds
`/cfg/app.txt` with different content.
**Flow:**
1. Alice asks to compare `alpha:/cfg/app.txt` with `beta:/cfg/app.txt` → the diff headers name each file with
   its own project, so the two sides are never confused.
**Postconditions:** Nothing changed in either project.
**Exceptions:**
- EXC-002a: Alice is not a member of `beta` → refused, nothing about either file is revealed (FR-NEW-005).
- EXC-002b: `beta` was deleted → refused as an unknown project (FR-NEW-014).
**Interactions:** SC-004.

### SC-003: Identical files
**Actor:** Member
**Preconditions:** Two files with the same content, or the same file given twice.
**Flow:**
1. Alice asks for the comparison → the product answers that the files are identical, with an empty diff.
**Postconditions:** Nothing changed.
**Exceptions:** none beyond SC-001.
**Interactions:** SC-001.

### SC-004: Refusals
**Actor:** Member, Non member
**Preconditions:** varies, see the tests.
**Flow:**
1. The caller asks for a comparison that cannot be done → the product refuses with one message that names the
   side (project and path) at fault. No partial diff is ever returned.
**Postconditions:** Nothing changed.
**Exceptions:**
- EXC-004a: a file is not a text file → refused (FR-NEW-008).
- EXC-004b: a file is over the size limit → refused (FR-NEW-009).
- EXC-004c: several things are wrong at once → the first problem in the fixed check order is reported
  (FR-NEW-015).
**Interactions:** SC-001, SC-002.

### SC-005: Program client
**Actor:** Program client
**Preconditions:** as SC-001 or SC-002.
**Flow:**
1. The program asks the web interface for the same comparison → it receives the same diff or the same refusal
   message as the tool surface.
**Postconditions:** Nothing changed.
**Exceptions:** same refusals as SC-004.
**Interactions:** SC-001 to SC-004.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Compare two text files
> WHEN a member asks to compare two text files THE product SHALL return the differences from the first file to
> the second as a unified diff with 3 unchanged lines of context around each change.

- **Business rules:** lines are compared with their line endings (FR-NEW-016). Changes far apart form
  separate blocks. The first file is the old side, the second is the new side.
- **User visible text:** none beyond the diff.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-U]: Diff header names project and path
> The product SHALL label the old side of a diff `--- <project>:<path>` and the new side `+++ <project>:<path>`,
> with the normalized path of each file.

- **Business rules:** the project is always shown, also when both files are in the same project.
- **User visible text:** `--- alpha:/notes/a.txt` and `+++ alpha:/notes/b.txt`.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Identical files
> IF the two files have exactly the same content THEN THE product SHALL answer that they are identical, with an
> empty diff.

- **Business rules:** same content means the same bytes; two empty files are identical; a file compared with
  itself is identical.
- **User visible text:** the answer carries an "identical" indicator set to true and an empty diff; when the
  files differ the indicator is false.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-004 [EARS-U]: Two projects
> The product SHALL accept two files from two different projects when the caller is a member of both.

- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: No access
> IF the caller is not a member of the project of either file THEN THE product SHALL refuse the comparison and
> SHALL NOT reveal the existence or content of either file.

- **Business rules:** both projects are checked before any file is looked up. A refusal is the same whether
  the files exist or not.
- **User visible text:** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'` (the project named is
  the first one the caller cannot read, first file before second).
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: Missing file
> IF a file does not exist THEN THE product SHALL refuse the comparison and name that file.

- **User visible text:** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Directory
> IF a path designates a directory THEN THE product SHALL refuse the comparison and name that path.

- **User visible text:** `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory`
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-008 [EARS-O]: Not a text file
> IF a file contains a NUL byte or bytes that are not valid UTF-8 THEN THE product SHALL refuse the comparison
> and name that file.

- **Business rules:** an empty file is a text file. A file is judged on its whole content.
- **User visible text:** `ERR_INVALID_ARGUMENT: 'alpha:/img/logo.png' is not a text file`
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Size limit
> IF a file is larger than the diff size limit THEN THE product SHALL refuse the comparison and name that
> file, its size and the limit.

- **Business rules:** the limit applies to each file, not to their sum. The default is 5 MiB, that is 5242880
  bytes. A file of exactly the limit is accepted. The operator sets the limit in the server configuration
  under the name pinned in Section 14.4, as a whole number of bytes above zero; a changed value takes effect
  at the next server start; at start up a zero, negative or non numeric value makes the server refuse to
  start with the message pinned in Section 14.4. The limit is checked before the content is judged text or not.
- **User visible text:** `ERR_INVALID_ARGUMENT: 'alpha:/logs/big.log' is 5242881 bytes, over the diff limit of 5242880 bytes`
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-UB]: Read only
> The product SHALL NOT change either file, either project's used storage, or the session write allowance
> when comparing files.

- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-011 [EARS-UB]: Read before edit
> The product SHALL NOT treat a comparison as having read the files for the read before edit rule.

- **Business rules:** a member who only compared a file still has to read it before editing it.
- **User visible text:** `edit '/notes/a.txt' requires reading it first in this session` (with the code the
  product already shows for this refusal, `ERR_EDIT_WITHOUT_PRIOR_READ`).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-012 [EARS-E]: Last access
> WHEN a comparison succeeds THE product SHALL update the last access date of both files within 2 seconds of
> the answer.

- **Business rules:** a refused comparison updates nothing. The date is the one the existing file status
  operation shows.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-013 [EARS-U]: Same answer through the web interface
> The programmatic access context SHALL offer the comparison with the same inputs, the same diff, the same
> identical indicator and the same refusal messages as the tool surface, under the names pinned in
> Section 14.4, a refusal keeping the status the product already attaches to its code.

- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-014 [EARS-O]: Deleted project
> IF the project of either file has been deleted THEN THE product SHALL refuse the comparison with the same
> message as any other read in that project.

- **User visible text:** `ERR_PROJECT_NOT_FOUND: project 'beta' not found`
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-015 [EARS-U]: Order of checks
> The product SHALL run these checks in this order and SHALL report only the first problem found: (1) the path
> of the first file is accepted by the product's path normalization (FR-NEW-017); (2) the same for the second
> path; (3) the first file's project exists and the caller is a member of it; (4) the same for the second
> file's project; (5) the first file exists; (6) the first file is not a directory; (7) the first file is
> within the size limit; (8) the first file is a text file; (9) to (12) the same four checks for the second
> file.

- **Business rules:** for one project, a deleted project is reported as unknown before membership is
  considered (as every other read does). Checks 1 to 4 happen before any file is looked up.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-016 [EARS-U]: Line endings count
> The product SHALL compare lines including their endings (line feed, carriage return plus line feed, lone
> carriage return), and SHALL NOT limit the diff to the first 2000 lines.

- **Business rules:** a file with Windows endings compared with the same text with Unix endings differs on
  every line. A last line without an ending is shown as a line of its own without a "no newline" marker, as
  the existing diff does.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-017 [EARS-U]: Paths are normalized like everywhere else
> The product SHALL normalize the path of each file exactly as every other file operation does, and SHALL show
> the normalized path in refusals and in the diff headers.

- **Business rules:** doubled separators and `.` segments are removed; a `..` segment removes the segment
  before it; a `..` that would climb above the top of the project stops at the top (so `/notes/../../etc/x`
  means `/etc/x` inside the project, and is not a refusal). A path containing a NUL character, or longer than
  the product's path length limit, is refused with the message the other file operations already give.
- **User visible text:** none new.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-018 [EARS-O]: Missing value on the web interface
> IF a program client omits one of the four values (the project and path of each file) or gives it empty THEN
> THE product SHALL refuse the comparison, naming the first missing value in the order first project, first
> path, second project, second path.

- **Business rules:** a value made only of spaces counts as empty. Only the web interface applies this check.
  The tool surface has no missing value check: an empty path designates the project root and is refused as a
  directory (`ERR_INVALID_ARGUMENT: 'alpha:/' is a directory`); an empty or blank project is refused as an
  unknown project (`ERR_PROJECT_NOT_FOUND: project '' not found`), all through the checks of FR-NEW-015.
- **User visible text:** `ERR_INVALID_ARGUMENT: 'mount_id_b' is required` (value names pinned in Section 14.4)
- **Scenario:** SC-005
- **Priority:** Must

### 6.2 Modified
None. The existing write, edit and read operations are unchanged.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN two files of 5 MiB each, differing on 1000 lines, are compared THE product SHALL answer within 10
  seconds. (A measured baseline does not exist: no comparison exists today. Section 11.)

### 7.2 Security and access
- Covered by FR-NEW-005, FR-NEW-014, FR-NEW-015. A comparison never reveals more than a read would.

### 7.3 Privacy and compliance
- None new.

### 7.4 Usability and accessibility
- A refusal always names the project and path at fault, so a caller handling two files knows which to fix.

### 7.5 Reliability
- The product SHALL NOT return a partial diff: either the whole diff or a refusal.

### 7.6 Operability
- An operator can raise or lower the size limit in the server configuration without code change (FR-NEW-009).

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | empty vs non empty; both empty | diff of the added lines; identical | E2E-201, E2E-202 |
| Boundary values | exactly 5242880 bytes; 5242881 bytes | accepted; refused | E2E-204, E2E-205 |
| Concurrent actions | file changed during a comparison | not applicable: each file is read as one complete version, the answer reflects that version | n/a |
| Slow connectivity | slow or lost connection | not applicable: single request, no partial answer (7.5) | n/a |
| Access states | plain member; not a member; deleted project | allowed; refused; refused | E2E-211, E2E-107, E2E-109 |
| Data variations | accents, emoji; CRLF vs LF; no final newline | shown intact; every line differs; handled | E2E-209, E2E-206, E2E-207 |
| Navigation | not applicable: no multi step flow | n/a | n/a |
| Partial operations | not applicable: read only | n/a | n/a |
| Dependency failures | storage unavailable | existing behavior of any read | n/a |
| Ordering and timing | several problems at once | first in the fixed order | E2E-110 |
| Exhaustion | file over 2000 lines; file over limit | whole diff; refused | E2E-210, E2E-106 |

## 9. Acceptance tests (the contract)

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, 002, 016 | Critical |
| E2E-002 | New | journey | SC-002 | FR-NEW-001, 002, 004 | Critical |
| E2E-003 | New | journey | SC-003 | FR-NEW-003 | Critical |
| E2E-004 | New | journey | SC-005 | FR-NEW-013, 001 | High |
| E2E-005 | New | side effect | SC-001 | FR-NEW-010 | High |
| E2E-006 | New | side effect | SC-001 | FR-NEW-012 | Medium |
| E2E-007 | New | side effect | SC-001 | FR-NEW-011 | High |
| E2E-101 | New | failure | SC-001 | FR-NEW-006, 003 | High |
| E2E-102 | New | failure | SC-004 | FR-NEW-006, 015 | High |
| E2E-103 | New | failure | SC-001 | FR-NEW-007 | High |
| E2E-104 | New | failure | SC-004 | FR-NEW-008 | High |
| E2E-105 | New | failure | SC-004 | FR-NEW-008 | High |
| E2E-106 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-107 | New | failure | SC-002 | FR-NEW-005 | Critical |
| E2E-108 | New | failure | SC-004 | FR-NEW-005 | Critical |
| E2E-109 | New | failure | SC-002 | FR-NEW-014 | High |
| E2E-110 | New | failure | SC-004 | FR-NEW-015 | High |
| E2E-111 | New | failure | SC-004 | FR-NEW-015, 017 | Medium |
| E2E-226 | New | failure | SC-004 | FR-NEW-015, 017 | Medium |
| E2E-227 | New | failure | SC-005 | FR-NEW-018 | Medium |
| E2E-228 | New | failure | SC-004 | FR-NEW-015, 017 | Medium |
| E2E-112 | New | failure | SC-005 | FR-NEW-013 | High |
| E2E-212 | New | failure | SC-004 | FR-NEW-007, 015 | High |
| E2E-217 | New | failure | SC-002 | FR-NEW-014 | High |
| E2E-222 | New | failure | SC-004 | FR-NEW-017 | Medium |
| E2E-223 | New | failure | SC-005 | FR-NEW-018 | Medium |
| E2E-224 | New | failure | SC-005 | FR-NEW-018, 013 | Medium |
| E2E-225 | New | edge | SC-004 | FR-NEW-017 | Medium |
| E2E-229 | New | failure | SC-004 | FR-NEW-015, 018 | Medium |
| E2E-230 | New | edge | SC-001 | Section 7.1, FR-NEW-001 | Medium |
| E2E-221 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-213 | New | side effect | SC-002 | FR-NEW-010 | High |
| E2E-215 | New | side effect | SC-002 | FR-NEW-012 | Medium |
| E2E-201 | New | edge | SC-001 | FR-NEW-001, 003 | Medium |
| E2E-202 | New | edge | SC-003 | FR-NEW-003, 008 | Medium |
| E2E-203 | New | edge | SC-003 | FR-NEW-003, 010 | Medium |
| E2E-204 | New | edge | SC-004 | FR-NEW-009 | High |
| E2E-205 | New | edge | SC-004 | FR-NEW-009 | High |
| E2E-206 | New | edge | SC-001 | FR-NEW-016 | High |
| E2E-207 | New | edge | SC-001 | FR-NEW-016, 001 | Medium |
| E2E-208 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-209 | New | edge | SC-001 | FR-NEW-001, 002 | Medium |
| E2E-210 | New | edge | SC-001 | FR-NEW-016 | High |
| E2E-211 | New | edge | SC-002 | FR-NEW-004, 005 | Medium |
| E2E-214 | New | edge | SC-001 | FR-NEW-011 | High |
| E2E-216 | New | edge | SC-003 | FR-NEW-012, 003, 011 | Medium |
| E2E-218 | New | edge | SC-002 | FR-NEW-014, 004 | Medium |
| E2E-219 | New | edge | SC-002 | FR-NEW-006 | Medium |
| E2E-220 | New | edge | SC-005 | FR-NEW-013, 003 | Medium |

Coverage: happy 4, failure 22, side effect 5, edge 18 (total 49), state transition 0 (read only), security 0.
Happy to failure ratio 4:22 (beats 1:1).

### 9.2 New tests
Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

#### E2E-001: Two files of one project differ on one line
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002, 016. **Priority:** Critical
- **Given** Alice and the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/b.txt`
- **Then** she receives `--- alpha:/notes/a.txt`, `+++ alpha:/notes/b.txt`, the block header `@@ -1,3 +1,3 @@`,
  the lines ` one`, `-two`, `+2`, ` three`, and the identical indicator is false
- **And** nothing else is returned

#### E2E-002: Two projects
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-NEW-001, 002, 004. **Priority:** Critical
- **Given** `alpha:/cfg/app.txt` = `port=80\n` and `beta:/cfg/app.txt` = `port=8080\n`, Alice a member of both
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** the headers are `--- alpha:/cfg/app.txt` and `+++ beta:/cfg/app.txt`, with `-port=80` and
  `+port=8080`, identical indicator false

#### E2E-003: Identical files
- **Category:** journey. **Scenario:** SC-003. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** `alpha:/notes/a.txt` and `alpha:/notes/copy.txt` both `one\ntwo\nthree\n`
- **When** Alice compares them
- **Then** the identical indicator is true and the diff is empty (zero characters)

#### E2E-004: Program client gets the same answer
- **Category:** journey. **Scenario:** SC-005. **Requirements:** FR-NEW-013, 001. **Priority:** High
- **Given** the common data
- **When** Alice's program asks the web interface to compare `alpha:/notes/a.txt` with `alpha:/notes/b.txt`
  and, separately, Alice asks the same through the tool surface
- **Then** the diff text and the identical indicator are exactly equal in both answers

#### E2E-005: Comparing changes nothing
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the common data; before: the content of both files, the used storage of `alpha`, and the session
  write allowance Alice has left
- **When** Alice compares `a.txt` with `b.txt` three times
- **Then** both files have the same content, the used storage of `alpha` is the same, and the write allowance
  left is the same as before

#### E2E-006: Last access updated
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012. **Priority:** Medium
- **Given** the common data and last access dates of both files recorded
- **When** Alice compares them, then, separately, compares `a.txt` with `alpha:/notes/missing.txt` (refused)
- **Then** within 2 seconds after the first comparison, as shown by the existing file status operation, both dates are newer; after the refused one `a.txt` keeps the date it
  had after the first

#### E2E-007: Comparing is not reading
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the common data and a new session for Alice
- **When** Alice compares `a.txt` with `b.txt`, then tries to replace `two` with `2` in `a.txt`
- **Then** the edit is refused with `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session`
- **And** after Alice reads `a.txt` the same edit succeeds

#### E2E-101: First file missing
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-006, 003. **Priority:** High
- **Given** the common data, `alpha:/notes/missing.txt` does not exist
- **When** Alice compares `alpha:/notes/missing.txt` with `alpha:/notes/b.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and no diff
- **And** the answer carries no identical indicator and no diff text

#### E2E-102: Second file missing
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-006. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/missing.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and no diff
- **And** the answer carries no "identical" indicator and no diff text, and the second file is the one named

#### E2E-103: A directory
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes` with `alpha:/notes/b.txt`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory`

#### E2E-104: A file with a NUL byte
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** `alpha:/img/logo.png` holding the 8 bytes `89 50 4E 47 0D 0A 1A 00` (it contains a NUL)
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/img/logo.png`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/img/logo.png' is not a text file`, and no partial diff

#### E2E-105: A file with invalid UTF-8
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** `alpha:/data/latin1.txt` holding the 4 bytes `63 61 66 E9` (`caf` then a lone byte E9, no NUL)
- **When** Alice compares `alpha:/data/latin1.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/data/latin1.txt' is not a text file`

#### E2E-106: A file over the limit
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/big.log` of 5242881 bytes of the letter `a`
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/logs/big.log`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/logs/big.log' is 5242881 bytes, over the diff limit of 5242880 bytes`
- **And** after the operator sets the limit to 6291456 bytes and restarts the server, the same request succeeds

#### E2E-107: No access to the second project
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-005. **Priority:** Critical
- **Given** `beta:/cfg/app.txt` exists with the line `secret=1`; Bob is a member of `alpha` only
- **When** Bob compares `alpha:/notes/a.txt` with `beta:/cfg/app.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`
- **And** the answer contains neither `secret` nor any line of either file, and is word for word the same
  when Bob names `beta:/cfg/nothing.txt` which does not exist

#### E2E-108: No access to the first project
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-005. **Priority:** Critical
- **Given** as E2E-107
- **When** Bob compares `beta:/cfg/app.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`, no content

#### E2E-109: Deleted project
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-014. **Priority:** High
- **Given** `beta` has been deleted; Alice was a member
- **When** Alice compares `alpha:/notes/a.txt` with `beta:/cfg/app.txt`
- **Then** `ERR_PROJECT_NOT_FOUND: project 'beta' not found`

#### E2E-110: Several problems, fixed order
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015. **Priority:** High
- **Given** Bob a member of `alpha` only; `alpha:/notes` a directory; `beta:/cfg/nothing.txt` does not exist
- **When** Bob compares `alpha:/notes` (a directory) with `beta:/cfg/nothing.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'` (access outranks the directory problem)
- **And** when Alice (member of both) makes the same request, the answer is
  `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory` (first file outranks the missing second file)

#### E2E-111: Climbing above the top stops at the top
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** the common data; `alpha` has no `/etc/passwd`
- **When** Alice compares `alpha:/notes/../../etc/passwd` with `alpha:/notes/b.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/etc/passwd' not found`

#### E2E-112: Program client gets the same refusal
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-013. **Priority:** High
- **Given** as E2E-107
- **When** Bob's program asks the web interface for the same comparison as E2E-107
- **Then** the refusal message equals the one from E2E-107 word for word

#### E2E-201: Empty against non empty
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 003. **Priority:** Medium
- **Given** `alpha:/e.txt` empty (0 bytes) and `alpha:/notes/b.txt`
- **When** Alice compares `/e.txt` with `/notes/b.txt`
- **Then** the diff has the block header `@@ -0,0 +1,3 @@` and the three lines `+one`, `+2`, `+three`;
  indicator false

#### E2E-202: Two empty files
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-003, 008. **Priority:** Medium
- **Given** `alpha:/e1.txt` and `alpha:/e2.txt`, both 0 bytes
- **When** Alice compares them
- **Then** identical true, empty diff (an empty file is a text file)

#### E2E-203: The same file on both sides
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-003, 010. **Priority:** Medium
- **Given** the common data and the content and used storage of `alpha` recorded
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/a.txt`
- **Then** identical true, empty diff
- **And** the file and the used storage of `alpha` are unchanged

#### E2E-204: Exactly at the limit
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/edge.log` of exactly 5242880 bytes of the letter `a` (no line ending)
- **When** Alice compares it with `alpha:/notes/a.txt`
- **Then** a diff is returned, no refusal

#### E2E-205: One byte over the limit
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/over.log` of 5242881 bytes
- **When** Alice compares `alpha:/notes/a.txt` with it
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/logs/over.log' is 5242881 bytes, over the diff limit of 5242880 bytes`

#### E2E-206: Windows against Unix line endings
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016. **Priority:** High
- **Given** `alpha:/w.txt` = `one\r\ntwo\r\n` and `alpha:/u.txt` = `one\ntwo\n`
- **When** Alice compares `/w.txt` with `/u.txt`
- **Then** identical false; both lines appear as removed and re added (2 `-` lines, 2 `+` lines)

#### E2E-207: No final line ending
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016, 001. **Priority:** Medium
- **Given** `alpha:/n1.txt` = `a` (no ending) and `alpha:/n2.txt` = `b` (no ending)
- **When** Alice compares `/n1.txt` with `/n2.txt`
- **Then** the diff is `--- alpha:/n1.txt`, `+++ alpha:/n2.txt`, `@@ -1 +1 @@`, `-a`, `+b`, each on its own
  line, with no "no newline" marker

#### E2E-208: Two distant changes
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001. **Priority:** Medium
- **Given** `alpha:/l1.txt` with lines `line1` to `line20`, and `alpha:/l2.txt` the same except `LINE1` and
  `LINE20`
- **When** Alice compares them
- **Then** the diff has exactly 2 blocks, the first changing `line1`, the second changing `line20`, each with
  at most 3 unchanged lines around the change

#### E2E-209: Accents and emoji
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002. **Priority:** Medium
- **Given** `alpha:/é/a.txt` = `café ☕\n` and `alpha:/é/b.txt` = `café 🍵\n`
- **When** Alice compares them
- **Then** the headers are `--- alpha:/é/a.txt` and `+++ alpha:/é/b.txt` and the lines `-café ☕` and
  `+café 🍵` appear intact

#### E2E-210: More than 2000 lines
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016. **Priority:** High
- **Given** `alpha:/big1.txt` with 3000 lines `row1` to `row3000`, and `alpha:/big2.txt` identical except
  line 2999 is `CHANGED`
- **When** Alice compares them
- **Then** the diff contains `-row2999` and `+CHANGED`, which sit beyond line 2000, and the diff carries
  no truncation notice

#### E2E-211: A plain member
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-004, 005. **Priority:** Medium
- **Given** Carol (carol@example.com), a plain member (not owner) of `alpha` and `beta`
- **When** Carol compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** she receives the same diff as in E2E-002

#### E2E-212: Second path is a directory
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, 015. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory`, and no diff

#### E2E-213: Cross project comparison changes nothing
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** E2E-002 data; before: content of both files and used storage of `alpha` and `beta`
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`, and also makes a refused comparison
  against `beta:/cfg/missing.txt`
- **Then** both files, `alpha` used storage and `beta` used storage are exactly as before

#### E2E-215: Last access in each project
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-012. **Priority:** Medium
- **Given** E2E-002 data and the last access dates of both files recorded
- **When** Alice compares them
- **Then** within 2 seconds, as shown by the existing file status operation, the last access date of `alpha:/cfg/app.txt` and of `beta:/cfg/app.txt` are both newer

#### E2E-217: First file in a deleted project
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-014. **Priority:** High
- **Given** `beta` deleted
- **When** Alice compares `beta:/cfg/app.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_PROJECT_NOT_FOUND: project 'beta' not found`

#### E2E-214: Edit of the second file after a comparison
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the common data and a new session for Alice
- **When** Alice compares `a.txt` with `b.txt`, then tries to replace `2` with `two` in `b.txt`
- **Then** `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/b.txt' requires reading it first in this session`

#### E2E-216: Identical files still count as accessed
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-012, 003, 011. **Priority:** Medium
- **Given** E2E-003 data, a new session for Alice and the last access dates recorded
- **When** Alice compares the identical files, then tries to replace `two` with `2` in `a.txt`
- **Then** identical true, and within 2 seconds both last access dates are newer
- **And** the edit is refused with `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session`

#### E2E-218: Deleted project brought back
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-014, 004. **Priority:** Medium
- **Given** `beta` deleted, then restored by an administrator
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** she receives the same diff as in E2E-002

#### E2E-219: Missing second file in another project
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-006. **Priority:** Medium
- **Given** E2E-002 data, `beta:/cfg/missing.txt` does not exist
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/missing.txt`
- **Then** `ERR_NOT_FOUND: 'beta:/cfg/missing.txt' not found`

#### E2E-220: Identical files through the web interface
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-013, 003. **Priority:** Medium
- **Given** E2E-003 data
- **When** Alice's program asks the web interface to compare the two identical files
- **Then** identical true and an empty diff, exactly as E2E-003

#### E2E-221: Invalid limit at start up
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** Medium
- **Given** an operator who sets the diff size limit to `0`, then to `-5`, then to `five megabytes`
- **When** the operator starts the server with each value
- **Then** the server refuses to start each time, with the startup message pinned in Section 14.4 (the same
  text for the three values)

#### E2E-222: Climbing above the top, second file, other project
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-017. **Priority:** Medium
- **Given** the common data; `beta` has no `/secret.txt`
- **When** Alice compares `alpha:/notes/a.txt` with `beta:/../secret.txt`
- **Then** `ERR_NOT_FOUND: 'beta:/secret.txt' not found`

#### E2E-223: Web interface, second project missing
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with the first project and path and the second path, but no
  second project
- **Then** `ERR_INVALID_ARGUMENT: 'mount_id_b' is required`

#### E2E-224: Web interface, several values missing
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018, 013. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with only the first project given and the first path empty
- **Then** `ERR_INVALID_ARGUMENT: 'path_a' is required` (the first missing value in the fixed order)

#### E2E-225: Path with doubled separators
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-017. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes//a.txt` with `alpha:/notes/missing.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and a successful comparison of
  `alpha:/notes//a.txt` with `alpha:/notes/b.txt` shows the header `--- alpha:/notes/a.txt`

#### E2E-226: Path normalization does not outrank access
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** Bob (member of `alpha` only)
- **When** Bob compares `alpha:/notes/a.txt` with `beta:/../x.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`

#### E2E-227: Value of spaces on the web interface
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with all values given but the second path equal to three
  spaces
- **Then** `ERR_INVALID_ARGUMENT: 'path_b' is required`

#### E2E-228: Path with a NUL character
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with a path made of `/notes/b.txt` followed by a NUL character
- **Then** `ERR_PATH_OUT_OF_BOUNDS: path contains a NUL byte`, and no file is looked up

#### E2E-229: Empty path on the tool surface
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 018. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with project `alpha` and an empty path, through the tool surface
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/' is a directory`

#### E2E-230: Large comparison within the time bound
- **Category:** edge. **Scenario:** SC-001. **Requirements:** Section 7.1, FR-NEW-001. **Priority:** Medium
- **Given** `alpha:/p1.txt` and `alpha:/p2.txt`, each 5242880 bytes of 80 byte lines, differing on 1000 spread
  lines
- **When** Alice compares them
- **Then** a diff with 1000 changed lines is returned within 10 seconds, identical false

### 9.3 Exploit tests
Not applicable: not a security entry.

### 9.4 Modified tests
None for behavior. Existing tests that count the tools or compare the frozen contract change because a tool is
added: see Section 14.3.

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, 002, 006, 007, 010, 011, 012, 016 | E2E-001, 005, 006, 007 | E2E-101, 103 | E2E-201, 206, 207, 208, 209, 210, 214 |
| SC-002 | FR-NEW-004, 005, 014 | E2E-002, 213, 215 | E2E-107, 109, 217 | E2E-211, 218, 219 |
| SC-003 | FR-NEW-003 | E2E-003 | E2E-101 (a refusal never claims identical) | E2E-202, 203, 216 |
| SC-004 | FR-NEW-005, 006, 008, 009, 015 | E2E-001 | E2E-102, 104, 105, 106, 108, 110, 111, 212, 221, 222, 226, 228 | E2E-204, 205 |
| SC-005 | FR-NEW-013, 018 | E2E-004 | E2E-112, 223, 224, 227 | E2E-220 |

Requirement coverage (tests naming each): FR-NEW-001: 001, 002, 004, 201, 207, 208, 209. 002: 001, 002, 209.
003: 003, 101, 201, 202, 203, 216, 220. 004: 002, 211, 218. 005: 107, 108, 211. 006: 101, 102, 219. 007: 103, 110, 212.
008: 104, 105, 202. 009: 106, 204, 205, 221. 010: 005, 213, 203. 011: 007, 214, 216. 012: 006, 215, 216.
013: 004, 112, 220. 017: 111, 222, 225, 226, 228. 018: 223, 224, 227, 229. 014: 109, 217, 218. 015: 102, 110, 111, 212. 016: 001, 206, 207, 210.

## 11. Open questions
- Is 10 seconds for two 5 MiB files (7.1) right? It does not change any behavior, only the measure the design
  must meet; no baseline exists. Does not change observable behavior.
- The test plan was written by the requirements author, not by a separate test designer. Does not change
  observable behavior.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Project | a named space holding files; members can read it | access control |
| Member | a person allowed in a project | access control |
| Unified diff | the standard text format listing removed lines with `-`, added lines with `+`, and context | file storage |
| Block | one group of changes with its context in a diff | file storage |
| Text file | a file with no NUL byte and valid UTF-8 content, an empty file included | file storage |
| Diff size limit | the largest file the comparison accepts, 5242880 bytes by default | file storage |
| Last access | the date a file was last read, used by the automatic clean up | file storage |
| Read before edit rule | an edit is refused unless the member read the file in this session | safety |
| Identical indicator | the true or false part of the answer telling whether the files have the same content; its name is pinned in Section 14.4 | file storage |
| Web interface | the programmatic access to the same capabilities | programmatic access |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 3 | 0 | NOT-IMPLEMENTABLE (check order contradiction, unnamed setting, unnamed tool) |
| 2 | 2 | 3 | NOT-IMPLEMENTABLE (names pinned in 14.4, setting and startup text, citations) |
| 3 | 2 | 4 | NOT-IMPLEMENTABLE (path refusal text, missing web value) |
| 4 | 2 | 4 | NOT-IMPLEMENTABLE (the path refusal cannot exist in the code) |
| 5 | 2 (minor) | 2 | NOT-IMPLEMENTABLE (blank values on the tool surface, untested 10 second bound; amended with the auditor's own text, not re-audited) |

**Amendments applied:** FR-NEW-015 rewritten, FR-NEW-017 (rewritten in round 4 to reuse the existing normalization), FR-NEW-018, E2E-213 to E2E-228, Section 14.4, citations corrected.
**No implementation detail outside Section 14:** the pinned names appear in requirement and test text by reference to 14.4 only where a user visible message needs them (FR-NEW-017, FR-NEW-018, E2E-223, E2E-224).
**Status:** specified on the requester's instruction after round 5; the round 5 amendments were not re-audited.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Diff only as side effect of write, edit, multi_edit | `crates/core/src/core/fs_ops.rs:952`, `:1037`, `:1068`, `:1102` |
| Diff engine labels both sides with one path; 3 context lines | `crates/core/src/core/diff.rs:50-56`, `:10` |
| Diff engine keeps line endings (`\r\n`, `\n`, lone `\r`) | `crates/core/src/core/diff.rs:17-45` |
| No "no newline" marker; missing final newline test | `crates/core/src/core/diff.rs:165-169` |
| Diff engine has 12 unit tests | `rg -c "#\[test\]\|#\[tokio::test\]" crates/core/src/core/diff.rs` returned 12 |
| Every fs tool takes a single mount id, no two project tool | `TOOL_CONTRACT.txt:166-245`; `rg "mount_id_[ab]\|other_mount\|source_mount" TOOL_CONTRACT.txt crates/core/src` returned no match |
| Read decodes lossy; no text check | `crates/core/src/storage/volume.rs:119-122` |
| Read pages 2000 lines, no size refusal | `crates/core/src/core/fs_ops.rs:68-70`, `crates/core/src/config.rs:365-397` (`max_read_lines` 2000) |
| Read records the read for the guard and touches last access | `crates/core/src/core/fs_ops.rs:65-66`; guard message `crates/core/src/safety.rs:166-168` |
| Existing messages: forbidden, not found | built at `crates/core/src/storage/admin.rs:393-395` (deleted project reported before membership at `:392-394`); `crates/core/src/storage/volume.rs:80`; asserted in tests at `crates/core/src/errors.rs:198` |
| Deleted project message | `crates/core/src/errors.rs:62-63`, `:299` |
| 108 tools | `AGENTS.md` Overview; `TOOL_CONTRACT.txt` |
| Web interface parity rule | `sdlc/specs/SPEC-0004-rest-data-plane/spec.md:216-218` |
| Web read route registered next to the others | `crates/core/src/api/dataplane.rs:129-151` |

Test command: `cargo test --workspace` (also `make check`); diff unit tests: `cargo test -p mcp-fs-core --lib core::diff`.

### 14.2 Class sweep
Not applicable.

### 14.3 Design inputs
- Tool name, parameter names, answer fields, web route and operator setting are pinned in Section 14.4. They
  follow the contract conventions (`TOOL_CONTRACT.txt`, snake_case, descriptions frozen once written). BL-0067
  may later remove the project argument: do not block it. The convention that every `fs.*` tool takes one
  `mount_id` (AGENTS.md Conventions) does not fit a two project tool; Section 14.4 states the exception.
- `unified_with_context` takes one label for both header lines (`core/diff.rs:56`, header written from line 69 on); the design needs two
  labels. Signal: compiler error and existing `diff.rs` unit tests (callers of `diff::unified` at `fs_ops.rs:952,1037,1068,1102`
  must keep byte identical output).
- No file size or text check exists; `read_text` is lossy (`storage/volume.rs:119`). The check must run on the
  raw bytes, and the size must be known before the blob is loaded (node metadata holds it: `NodeRow.size` at `storage/traits.rs:19`, returned by
  `VolumeClient::stat` at `storage/volume.rs:75-82`). Signal: E2E-104, 105, 106, 204, 205.
- The limit needs a configuration key and the config snapshot tests (`config.rs:1633-1634`) will change.
  Signal: the two snapshot tests fail.
- Adding the 109th tool changes the count "108" in `AGENTS.md`, `README.md`, `TOOL_CONTRACT.txt`,
  `tool-contract-golden.json` (regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1`), `.agent_docs/tools.md` and
  the count asserted in code: `rg -l "108" --glob '!target' --glob '!sdlc/**'` lists `crates/agent/src/mcp.rs`,
  `crates/core/src/tools/artifacts.rs`, `crates/core/src/tools/contract_golden.rs`,
  `crates/core/src/mcp/server.rs`, `crates/core/src/token_screen.rs`, `crates/core/src/tools/git.rs`,
  `crates/core/src/storage/rel/schema.rs`. Signal: contract and count tests fail.
- Authorization today is one project per call (`mcp/server.rs:178-181`); two projects need both checked
  before any lookup (FR-NEW-005, 015).
- Do not call the read recording used by reads (`fs_ops.rs:65`) (FR-NEW-011); do call the last access touch
  (`fs_ops.rs:66`) only on success (FR-NEW-012).
- The tool must carry the read only hints (SPEC-0001) and a REST parity route next to `dataplane.rs:129-151`
  (SPEC-0004 FR-009).
- The REST caller reaches the same engine function; no second implementation (AGENTS.md Conventions).

### 14.4 Pinned public names (normative for /sdlc-design, decided with the requester's "defaults" answer)
- **Tool:** `fs.diff_files`, required string arguments `mount_id_a`, `path_a` (old side), `mount_id_b`,
  `path_b` (new side). Read only hints per SPEC-0001.
- **Answer:** an object with exactly two fields: `identical` (boolean) and `diff` (string, empty when
  identical).
- **Web route:** `GET /api/fs/diff` with the same four values as query parameters, the same two answer fields
  on success; a refusal carries the same code and message as the tool and the status the product already maps
  to that code (`crates/core/src/errors.rs:124-128`: 403 for forbidden, 404 for not found and unknown project,
  400 for invalid argument and path out of bounds).
- **Operator setting:** `safety.max_diff_file_bytes`, whole number of bytes, default `5242880`.
- **Startup refusal text (exact):** `invalid safety.max_diff_file_bytes: must be a whole number of bytes above zero`
- **Exception to "mount_id on every fs tool":** this tool carries `mount_id_a` and `mount_id_b` instead of
  `mount_id`. Verified: the only `mount_id` schema assertion in the golden tests covers the git family
  (`crates/core/src/tools/contract_golden.rs:254-271`, loop over `NEW_GIT_TOOLS`), so it does not apply.

Additions from the audit (design inputs):
- The existing not found message has no project prefix (`storage/volume.rs:80`); FR-NEW-006 pins a prefixed one, so the design builds it.
- Deserialize the diff size setting leniently and validate it at boot with the pinned text (14.4); a plain numeric field fails inside the config reader with its own message (`config.rs:377-397`). Signal: E2E-221 fails with a reader message.
- The diff body writer appends a line feed to any line lacking one (`crates/core/src/core/diff.rs:121-127`), so a line ending in a lone carriage return prints with an added line feed. Keep it for byte identity with write and edit; it is existing behavior, not a new one.
- Path normalization (`crates/core/src/safety.rs:131-146`) drops a `..` that would climb above the root
  (`crates/core/src/util/posix.rs:48-55`) and refuses only a NUL character or an over long path. FR-NEW-017
  reuses it unchanged; the `..` refusals of an earlier draft were removed because the code cannot produce them.
  Signal: E2E-111, 222 and 226 pass only if the existing normalization is reused.
- The registry size is asserted as 108 at `contract_golden.rs:252`; it becomes 109. Signal: that test fails.
