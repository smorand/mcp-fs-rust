# Choose the context size of a file comparison: functional specification

> Id: SPEC-0046
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 2 entry points (tool surface, web interface), additive change to a public contract, 10 requirements, no persisted change. No override.
> Status: designed
> Generated on: 2026-10-19
> Project: existing
> From backlog: BL-0069
> Depends on: SPEC-0045
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A member comparing two text files can choose how many unchanged lines the answer shows around each change,
from 0 (changed lines only) up to a whole file. Without a choice the answer stays exactly as today, with 3
lines. The same choice exists for programs using the web interface, with the same answers and refusals.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today
- A member can compare two text files (SPEC-0045). The answer is a unified diff with always exactly 3
  unchanged lines around each change. The member cannot change that number.
- Changes separated by few unchanged lines share one block; farther apart they form separate blocks.
- Writing, editing and multi editing a file also return a diff with 3 unchanged lines. Those are not part of
  this increment.
- The tool description tells callers "3 lines of context".
- On the web interface the comparison takes four values: project and path of each file. A missing or blank
  one is refused, naming the first missing one.
- Other operations that take a number clamp a negative value silently; this increment does not follow that
  habit (see FR-NEW-003).

### 2.2 Specs governing this area
- SPEC-0045 (compare two text files): FR-NEW-001 states "3 unchanged lines of context"; this increment
  modifies it (FR-MOD-001). FR-NEW-015 (order of checks) and FR-NEW-018 (missing web value) stay in force
  and are extended by FR-NEW-007. SPEC-0045 is not modified.
- SPEC-0004 (REST data plane): the web interface gives the same answers as the tool surface. Followed.

### 2.3 What is verified today
A comparison is exercised by 48 tests (Section 14.1). Every one of them uses the fixed 3 lines. No test
chooses another size. Command: Section 14.1.

## 3. Scope

### 3.1 In scope
- An optional context size on the comparison, tool surface and web interface.
- Refusal of invalid sizes.
- The tool description and the published tool contract reflect the new option.

### 3.2 Out of scope, and why
- The diff returned by write, edit and multi edit keeps 3 lines: nobody asked, and it avoids a wider contract
  change.
- A context size as an operator setting: the caller's choice is the need.
- Other diff formats (side by side, word level): not requested.

### 3.3 Constraints imposed by the context
- An answer produced without a size must be byte identical to today's (requester: SPEC-0045 callers).

## 4. Actors
- **Member**: a person allowed to read the projects involved; compares files.
- **Program client**: a program using the web interface on a member's behalf.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| File storage | file comparison and its answer | unified diff, block, context size |
| Access control | who is allowed to read a project | member, project |
| Programmatic access | the web interface | program client |

## 5. Usage scenarios

### SC-001: Compare with a chosen context size
**Actor:** member
**Preconditions:** the member can read the project(s) holding both files.
**Flow:**
1. The member asks to compare two files and gives a context size, or none → the product returns the diff with
   that many unchanged lines around each change (3 when none given).
**Postconditions:** neither file changed.
**Exceptions:**
- EXC-001a: the size is not valid (SC-003).
**Interactions:** SC-003, SC-005.

### SC-002: Changed lines only
**Actor:** member
**Preconditions:** two files differing in one line.
**Flow:**
1. The member gives size 0 → the diff holds only the removed and added lines.
**Postconditions:** none.
**Exceptions:**
- EXC-002a: size `-1` → refused (SC-003).

### SC-003: Invalid size
**Actor:** member or program client
**Preconditions:** any.
**Flow:**
1. The actor gives a size that is negative, not a whole number, or above 10000000 → the product refuses the
   comparison with the message of FR-NEW-003 and examines no project or file.
**Postconditions:** nothing read, nothing touched.
**Exceptions:**
- EXC-003a: the files do not exist, or the actor is not a member → still the size refusal.

### SC-004: Program client
**Actor:** program client
**Preconditions:** the four existing values are given.
**Flow:**
1. The client adds a context size → same answer as the tool surface.
**Postconditions:** none.
**Exceptions:**
- EXC-004a: the size is present but empty → refused.
- EXC-004b: a project or path is missing as well → the missing one is named first.

### SC-005: Identical files
**Actor:** member
**Preconditions:** two files with identical content.
**Flow:**
1. The member compares them with any valid size → the product answers that they are identical, with no diff.
**Postconditions:** none.
**Exceptions:**
- EXC-005a: the size is invalid → refused even though the files are identical.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Chosen context size
> WHEN a member asks to compare two text files and gives a context size N THE product SHALL show, around
> each change, at most N unchanged lines before it and at most N after it.

- **Business rules:** fewer lines appear when the file starts or ends sooner. N is a whole number from 0 to
  10000000 (FR-NEW-003).
- **User visible text:** none beyond the diff.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Default size
> IF no context size is given THEN THE product SHALL behave as if 3 were given.

- **Business rules:** the answer is byte identical to the answer before this increment. A value that is given
  but empty on the web interface is not "not given" (FR-NEW-003).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Invalid size
> IF the context size is not a whole number from 0 to 10000000 THEN THE product SHALL refuse the comparison.

- **Business rules:** refused: `-1`, `abc`, `2.5`, `10000001`, an empty or blank value on the web interface.
  Accepted: `0`, `10000000`. The upper bound exceeds the number of lines of the largest comparable file
  (5242880 bytes allow at most 5242880 lines), so an accepted size never cuts a file short.
  The value is read as written by the caller, with no trimming. It is accepted only when it is one or more
  ASCII digits `0` to `9` (leading zeros allowed, so `007` is 7) whose value is at most 10000000, written
  either as a number (`3`) or as text (`"3"`). Everything else is refused with the same message, whatever it
  is: `+3`, ` 3`, `3 `, `3.0`, `1e3`, `0x3`, `-0`, `-1`, `1_0`, `2.5`, any non ASCII digit, a number or text
  above 10000000, an empty text, a boolean, a list, an object. The message does not repeat the given value.
  An omitted size, or a size given as `null`, means "not given" (FR-NEW-002). On the web interface the value
  is the text received after standard URL decoding, and a size repeated in the request uses its first
  occurrence. The quoted text is the refusal message; each surface carries it inside its existing error
  envelope and tests compare the message exactly.
- **User visible text:** `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000` (name pinned in Section 14.4)
- **Scenario:** SC-003, SC-004
- **Priority:** Must

#### FR-NEW-004 [EARS-E]: Size zero
> WHEN the context size is 0 THE product SHALL show only the removed and added lines of each change, with the
> two header lines.

- **Business rules:** the block header names only the changed lines (`@@ -6 +6 @@` for line 6).
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-E]: Blocks merge
> WHEN at most 2 times the context size unchanged lines separate two changes THE product SHALL show them in
> one block.

- **Business rules:** with size 3, 6 unchanged lines in between still merge; 7 do not. With size 0, only
  adjacent changes merge.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-010 [EARS-E]: Blocks stay apart
> WHEN more than 2 times the context size unchanged lines separate two changes THE product SHALL show them
> in two blocks.

- **Business rules:** as FR-NEW-005.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: Identical files
> IF both files have identical content and the context size is valid THEN THE product SHALL answer that they
> are identical, with an empty diff.

- **Business rules:** the answer does not depend on the size.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-007 [EARS-U]: Size checked first
> The product SHALL check the context size before examining any project, path or file, and on the web
> interface right after the check for missing values.

- **Business rules:** an invalid size wins over a missing file, a non member, an unknown project. A missing
  project or path on the web interface wins over an invalid size. A refusal for size changes nothing, not
  even a last access date. On the tool surface, a missing required value (project or path) is refused before
  the size is looked at.
- **Scenario:** SC-003, SC-004
- **Priority:** Must

#### FR-NEW-008 [EARS-UB]: Other diffs unchanged
> The product SHALL NOT change the number of unchanged lines (3) shown in the diff returned by writing,
> editing and multi editing a file.

- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: Same answer through the web interface
> The product SHALL give a program client, for the same files and the same context size, the same answer and
> the same refusals as the tool surface.

- **Scenario:** SC-004
- **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-E]: Compare two text files (modifies SPEC-0045/FR-NEW-001)
> WHEN a member asks to compare two text files THE product SHALL return the differences from the first file
> to the second as a unified diff with the requested number of unchanged lines around each change, 3 when
> none is requested.

- **Before:** always exactly 3 unchanged lines (Section 2.1).
- **After:** the statement above.
- **Reason:** BL-0069.
- **User visible text:** the tool description becomes `Compare two text files (same or different project) and return a unified diff. Shows 3 lines of context by default; context_lines (0 to 10000000) chooses another number of unchanged lines around each change.` The parameter description becomes `Optional. Number of unchanged lines shown around each change, a whole number from 0 to 10000000. Default 3.` Both appear in the published tool contract and the web interface description.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
### 7.1 Performance
No new time bound. The per file limit of SPEC-0045 (5242880 bytes) still applies, and a size of 10000000 on
files at that limit completes with the whole file as context (E2E-031).
### 7.2 Security and access
No change: access is checked exactly as before once the size is valid. An invalid size reveals nothing about
any project (FR-NEW-007).
### 7.3 Privacy and compliance
Not applicable.
### 7.4 Usability and accessibility
The refusal names the parameter and the allowed range, and does not repeat the given value (FR-NEW-003).
### 7.5 Reliability
A refused comparison leaves everything untouched (FR-NEW-007).
### 7.6 Operability
Not applicable: no new setting.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | identical files, size 0 | identical answer | E2E-010 |
| Boundary values | 0, 10000000 accepted (file at the size limit: E2E-031); 10000001, -1 refused; merge at 6 and 7 lines apart (size 3) | per FR-NEW-003/005/010 | E2E-003, 006, 007, 008, 011, 014, 043, 044, 046 |
| Concurrent actions | not applicable: read only | | |
| Slow or lost connectivity | not applicable: no new dependency | | |
| Access states | non member with invalid size | size refusal | E2E-017 |
| Data variations | `abc`, `2.5`, `+3`, `3.0`, `1e3`, spaces, `0x3`, boolean, list, empty value, `001`, null | refused / accepted per FR-NEW-003 | E2E-012, 013, 015, 026 to 039, 042, 049 |
| Navigation | not applicable | | |
| Partial operations | not applicable | | |
| Dependency failures | not applicable | | |
| Ordering and timing | change on line 1, size 3 | no lines before, no error | E2E-009 |
| Exhaustion | size larger than the file | whole file around the change | E2E-006 |

## 9. Acceptance tests (the contract)
> Fixtures (every line is `l<number>` followed by a line feed unless stated):
> - **P1:** project `alpha`: `/a.txt` = `l1`..`l10`; `/b.txt` = the same with line 6 replaced by `L6`; `/same.txt` = identical to `/a.txt`.
> - **P2:** project `alpha`: `/c.txt` = `l1`..`l14`; `/d.txt` = the same with line 3 replaced by `L3` and line 12 by `L12`.
> - **P3:** project `alpha`: `/e.txt` = `l1`..`l10`; `/f.txt` = same with line 1 replaced by `L1`.
> - **P5:** alpha: `/g.txt` = `l1`..`l15`; `/h.txt` = same with line 3 replaced by `L3` and line 10 by `L10`.
> - **P6:** alpha: `/i.txt` = `l1`..`l16`; `/j.txt` = same with line 3 replaced by `L3` and line 11 by `L11`.
> - **P4:** project `beta`: `/b.txt` = same content as alpha `/b.txt`.
> - Actor: `Alice`, member of `alpha` and `beta`; `Mallory` is not a member of `alpha`.
> A diff is shown below with `|` as line separator; the real answer separates lines with a line feed.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-002, FR-NEW-001, FR-MOD-001 | Critical |
| E2E-003 | New | happy | SC-002 | FR-NEW-004, FR-NEW-001 | High |
| E2E-004 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-005 | New | happy | SC-004 | FR-NEW-009, FR-NEW-001 | High |
| E2E-006 | New | edge | SC-001, SC-003 | FR-NEW-001, FR-NEW-003, FR-NEW-005 | High |
| E2E-007 | New | edge | SC-001 | FR-NEW-010, FR-NEW-001 | High |
| E2E-008 | New | edge | SC-001 | FR-NEW-005, FR-NEW-001 | High |
| E2E-009 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-010 | New | edge | SC-002, SC-005 | FR-NEW-006, FR-NEW-004 | Medium |
| E2E-011 | New | failure | SC-002, SC-003 | FR-NEW-003 | Critical |
| E2E-012 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-013 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-014 | New | failure | SC-003 | FR-NEW-003, FR-NEW-001 | High |
| E2E-015 | New | failure | SC-004 | FR-NEW-003, FR-NEW-009, FR-NEW-002 | High |
| E2E-016 | New | failure | SC-003 | FR-NEW-007, FR-NEW-003 | High |
| E2E-017 | New | failure | SC-003 | FR-NEW-007, FR-NEW-003 | High |
| E2E-018 | New | failure | SC-004 | FR-NEW-009, FR-NEW-003 | High |
| E2E-019 | New | side effect | SC-003 | FR-NEW-007, FR-NEW-003 | High |
| E2E-020 | New | state transition | SC-001 | FR-NEW-008 | Medium |
| E2E-021 | New | failure | SC-005 | FR-NEW-006, FR-NEW-003 | High |
| E2E-022 | New | happy | SC-005 | FR-NEW-006, FR-NEW-002 | Medium |
| E2E-023 | New | failure | SC-004 | FR-NEW-007, FR-NEW-009 | High |
| E2E-024 | New | state transition | SC-001 | FR-NEW-008 | Medium |
| E2E-025 | New | state transition | SC-001 | FR-NEW-008 | Medium |
| E2E-026 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-027 | New | happy | SC-001 | FR-NEW-003, FR-NEW-001 | Medium |
| E2E-028 | New | happy | SC-001 | FR-NEW-002 | Medium |
| E2E-029 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-030 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-031 | New | edge | SC-001 | FR-NEW-001, FR-NEW-003, FR-NEW-005 | Medium |
| E2E-032 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-033 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-034 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-035 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-036 | New | failure | SC-003 | FR-NEW-003 | High |
| E2E-037 | New | happy | SC-001 | FR-NEW-003, FR-NEW-001 | Medium |
| E2E-038 | New | failure | SC-004 | FR-NEW-003, FR-NEW-009 | High |
| E2E-039 | New | failure | SC-004 | FR-NEW-003 | High |
| E2E-040 | New | edge | SC-001, SC-002 | FR-NEW-010, FR-NEW-004 | Medium |
| E2E-041 | New | edge | SC-001 | FR-NEW-010, FR-NEW-001 | Medium |
| E2E-042 | New | failure | SC-003 | FR-NEW-003, FR-NEW-009 | High |
| E2E-043 | New | edge | SC-001 | FR-NEW-005, FR-NEW-001 | Medium |
| E2E-044 | New | edge | SC-001 | FR-NEW-010, FR-NEW-001 | Medium |
| E2E-045 | New | happy | SC-005 | FR-NEW-006 | Medium |
| E2E-046 | New | edge | SC-002 | FR-NEW-004 | Medium |
| E2E-047 | New | edge | SC-004 | FR-NEW-009, FR-NEW-004 | Medium |
| E2E-048 | New | edge | SC-004 | FR-NEW-009, FR-NEW-001 | Medium |
| E2E-049 | New | edge | SC-004 | FR-NEW-009, FR-NEW-003 | Medium |
| E2E-MOD-001 | Modified | requirement | SC-001 | FR-MOD-001, FR-NEW-002 | High |

Coverage: happy 10, failure 21, side effect 1, edge 14, state transition 3, modified 1. Happy to failure ratio 1:2.1.

### 9.2 New tests

#### E2E-001: size 1
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** P1, Alice
- **When** Alice compares alpha `/a.txt` with alpha `/b.txt` with context size 1
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -5,3 +5,3 @@| l5|-l6|+L6| l7`
- **And** the identical indicator is false

#### E2E-002: no size given
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-001, FR-MOD-001
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` without a size
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -3,7 +3,7 @@| l3| l4| l5|-l6|+L6| l7| l8| l9`

#### E2E-003: size 0
- **Category:** happy. **Scenario:** SC-002. **Requirements:** FR-NEW-004, FR-NEW-001
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size 0
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -6 +6 @@|-l6|+L6`

#### E2E-004: two projects, size 1
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** P1, P4, Alice
- **When** Alice compares alpha `/a.txt` with beta `/b.txt` with size 1
- **Then** the diff is `--- alpha:/a.txt|+++ beta:/b.txt|@@ -5,3 +5,3 @@| l5|-l6|+L6| l7`

#### E2E-005: web interface, size 1
- **Category:** happy. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-001
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison of alpha `/a.txt` and alpha `/b.txt` with size `1`
- **Then** the answer equals that of E2E-001, field for field

#### E2E-006: largest accepted size
- **Category:** edge. **Scenario:** SC-001, SC-003. **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-005
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size 10000000
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -1,10 +1,10 @@| l1| l2| l3| l4| l5|-l6|+L6| l7| l8| l9| l10`

#### E2E-007: changes 8 lines apart, size 3
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 3 (8 unchanged lines between the changes, more than 2 times 3)
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -1,6 +1,6 @@| l1| l2|-l3|+L3| l4| l5| l6|@@ -9,6 +9,6 @@| l9| l10| l11|-l12|+L12| l13| l14` (two blocks)

#### E2E-008: same changes, size 4
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 4 (8 equals 2 times 4)
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -1,14 +1,14 @@| l1| l2|-l3|+L3| l4| l5| l6| l7| l8| l9| l10| l11|-l12|+L12| l13| l14` (one block)

#### E2E-009: change on the first line
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** P3, Alice
- **When** Alice compares `/e.txt` with `/f.txt` with size 3
- **Then** the diff is `--- alpha:/e.txt|+++ alpha:/f.txt|@@ -1,4 +1,4 @@|-l1|+L1| l2| l3| l4`

#### E2E-010: identical files, size 0
- **Category:** edge. **Scenario:** SC-002, SC-005. **Requirements:** FR-NEW-006, FR-NEW-004
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size 0
- **Then** the identical indicator is true and the diff is empty

#### E2E-011: negative size
- **Category:** failure. **Scenario:** SC-002, SC-003. **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size `-1`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`
- **And** no diff is returned, in particular not the 3 line one

#### E2E-012: not a number
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `abc`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

#### E2E-013: decimal
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `2.5`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

#### E2E-014: just above the maximum
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1, Alice
- **When** size is `10000001`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

#### E2E-015: web interface, empty size
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-009, FR-NEW-002
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison with all four values and an empty size
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`, and no 3 line diff

#### E2E-016: invalid size and missing file
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/missing.txt` with size `-1`
- **Then** refused with the size message, not a missing file message

#### E2E-017: invalid size by a non member
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, Mallory (not a member of alpha)
- **When** Mallory compares alpha `/a.txt` with alpha `/b.txt` with size `abc`
- **Then** refused with the size message, the same text Alice gets, revealing nothing about alpha

#### E2E-018: web interface, invalid size
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-003
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison with size `abc`
- **Then** the refusal is the same as E2E-012

#### E2E-019: refusal touches nothing
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, last access dates of `/a.txt` and `/b.txt` noted, Alice
- **When** Alice compares them with size `-1`
- **Then** refused as in E2E-011
- **And** both files, used storage of alpha and both last access dates are unchanged; a following comparison with size 1 does count as access (SPEC-0045 behavior)

#### E2E-020: write after comparison
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** P1, Alice has compared `/a.txt` with `/b.txt` with size 0
- **When** Alice reads then overwrites `/a.txt` changing line 6 to `X6`
- **Then** the diff returned by the write shows 3 unchanged lines before and after (`@@ -3,7 +3,7 @@`)

#### E2E-021: identical files, invalid size
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-006, FR-NEW-003
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size `-1`
- **Then** refused with the size message, not the identical answer

#### E2E-022: identical files, no size
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-006, FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` without a size
- **Then** the identical indicator is true and the diff is empty

#### E2E-023: missing value wins over invalid size
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-009
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison omitting the second path and giving size `abc`
- **Then** refused naming the second path as required (SPEC-0045 FR-NEW-018 text), not the size message

#### E2E-024: edit after comparison
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** P1, Alice has compared `/a.txt` with `/b.txt` with size 0 and read `/a.txt`
- **When** Alice edits `/a.txt` replacing `l6` by `X6`
- **Then** the diff returned shows 3 unchanged lines before and after

#### E2E-025: multi edit after comparison
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** as E2E-024
- **When** Alice applies a multi edit replacing `l6` by `X6`
- **Then** the diff returned shows 3 unchanged lines before and after

#### E2E-026: leading plus sign
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `+3`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

#### E2E-027: leading zeros
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1 with `/a.txt` and `/b.txt`, Alice
- **When** Alice compares them with size `001`
- **Then** the diff is the one of E2E-001 (size 1)

#### E2E-028: size given as null on the tool surface
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` giving the size as `null`
- **Then** the diff is the one of E2E-002 (3 lines)

#### E2E-029: size is the number `3.0` written without quotes
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the JSON number `3.0`
- **Then** refused with the message of E2E-011

#### E2E-030: size is the number `1e3` written without quotes
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the JSON number `1e3`
- **Then** refused with the message of E2E-011

#### E2E-031: Alice compares two files of 5242880 byte
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-005
- **Given** P1, Alice (or her program client)
- **When** Alice compares two files of exactly 5242880 bytes each, made of 2621440 lines, each line being `x` followed by a line feed, the second file differing only by line 1 being `y`, with size 10000000
- **Then** the diff is returned without failure, in one block `@@ -1,2621440 +1,2621440 @@`

#### E2E-032: size is the text ` 3` (leading space)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text ` 3` (leading space)
- **Then** refused with the message of E2E-011

#### E2E-033: size is the text `3 ` (trailing space)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text `3 ` (trailing space)
- **Then** refused with the message of E2E-011

#### E2E-034: size is the text `0x3`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text `0x3`
- **Then** refused with the message of E2E-011

#### E2E-035: size is the boolean `true`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the boolean `true`
- **Then** refused with the message of E2E-011

#### E2E-036: size is the list `[1]`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the list `[1]`
- **Then** refused with the message of E2E-011

#### E2E-037: Alice compares `/a.txt` with `/b.txt` on
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1, Alice (or her program client)
- **When** Alice compares `/a.txt` with `/b.txt` once with the number `1` and once with the text `"1"`
- **Then** both diffs are the one of E2E-001

#### E2E-038: a program client requests the comparison
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-009
- **Given** P1, Alice (or her program client)
- **When** a program client requests the comparison with the size given twice, first empty then `2`
- **Then** refused with the message of E2E-011 (the first occurrence, empty, counts)

#### E2E-039: a program client requests the comparison
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** a program client requests the comparison with the size ` ` (one space)
- **Then** refused with the message of E2E-011

#### E2E-040: separate changes, size 0
- **Category:** edge. **Scenario:** SC-001, SC-002. **Requirements:** FR-NEW-010, FR-NEW-004
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 0
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -3 +3 @@|-l3|+L3|@@ -12 +12 @@|-l12|+L12`

#### E2E-041: separate changes, size 1
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 1
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -2,3 +2,3 @@| l2|-l3|+L3| l4|@@ -11,3 +11,3 @@| l11|-l12|+L12| l13`

#### E2E-042: negative zero
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003, FR-NEW-009
- **Given** P1, Alice and a program client acting for her
- **When** Alice gives the number `-0` without quotes, and the client gives the text `-0`
- **Then** both are refused with the size message (message of E2E-011)

#### E2E-043: 6 unchanged lines between changes, size 3
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-001
- **Given** P5, Alice
- **When** Alice compares `/g.txt` with `/h.txt` with size 3
- **Then** one block: `--- alpha:/g.txt|+++ alpha:/h.txt|@@ -1,13 +1,13 @@| l1| l2|-l3|+L3| l4| l5| l6| l7| l8| l9|-l10|+L10| l11| l12| l13`

#### E2E-044: 7 unchanged lines between changes, size 3
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P6, Alice
- **When** Alice compares `/i.txt` with `/j.txt` with size 3
- **Then** two blocks: `--- alpha:/i.txt|+++ alpha:/j.txt|@@ -1,6 +1,6 @@| l1| l2|-l3|+L3| l4| l5| l6|@@ -8,7 +8,7 @@| l8| l9| l10|-l11|+L11| l12| l13| l14`

#### E2E-045: identical files, size 3
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-006
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size 3
- **Then** the identical indicator is true and the diff is empty

#### E2E-046: adjacent changes, size 0
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-004
- **Given** P7: alpha `/k.txt` = `l1`..`l5`; `/m.txt` = same with lines 2 and 3 replaced by `L2` and `L3`; Alice
- **When** Alice compares `/k.txt` with `/m.txt` with size 0
- **Then** the diff is `--- alpha:/k.txt|+++ alpha:/m.txt|@@ -2,2 +2,2 @@|-l2|-l3|+L2|+L3`

#### E2E-047: web interface, size 0
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-004
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `0`
- **Then** the answer equals that of E2E-003

#### E2E-048: web interface, size 10000000
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-001
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `10000000`
- **Then** the answer equals that of E2E-006

#### E2E-049: web interface, leading zeros
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-003
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `001`
- **Then** the answer equals that of E2E-001

### 9.3 Exploit tests
Not applicable (no `--security`).

### 9.4 Modified tests
#### E2E-MOD-001: SPEC-0045 comparison tests without a size (was the 48 tests of Section 14.1)
- **Validated before:** diff with always 3 lines.
- **Validates now:** the same, because no size is given (FR-NEW-002).
- **Given / When / Then:** as the existing tests; no expected value changes. The existing tests pass unmodified.

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, 002, 005, 008, 010, FR-MOD-001 | E2E-001, 002, 004, 027, 028, 037 | E2E-014 | E2E-006, 007, 008, 009, 031, 040, 041, 043, 044 |
| SC-002 | FR-NEW-004 | E2E-003 | E2E-011 | E2E-010, 046 |
| SC-003 | FR-NEW-003, 007 | E2E-006 | E2E-012, 013, 014, 016, 017, 019, 026, 029 to 036, 042 | E2E-014 |
| SC-004 | FR-NEW-003, 007, 009 | E2E-005 | E2E-015, 018, 023, 038, 039 | E2E-047, 048, 049 |
| SC-005 | FR-NEW-006 | E2E-022, 045 | E2E-021 | E2E-010 |

Requirement coverage (tests): FR-NEW-001: 001, 003, 004, 005, 006, 007, 008, 009, 014, 027, 031, 037, 041. FR-NEW-002: 002, 004, 011, 015, 022, 028, MOD-001. FR-NEW-003: 006, 011 to 019, 021, 026 to 039, 042, 049. FR-NEW-004: 003, 010, 040, 046, 047. FR-NEW-005: 008, 031, 043. FR-NEW-010: 007, 040, 041, 044. FR-NEW-006: 010, 021, 022, 045. FR-NEW-007: 016, 017, 019, 023. FR-NEW-008: 020, 024, 025. FR-NEW-009: 005, 015, 018, 023, 047, 048, 049. FR-MOD-001: 001, 002, MOD-001.

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Context size | number of unchanged lines shown before and after each change in a diff | file storage |
| Block | one group of changes with its context | file storage |
| Unified diff | the standard text listing removed lines with `-`, added with `+`, unchanged with a space | file storage |
| Member | a person allowed in a project | access control |
| Program client | a program using the web interface | programmatic access |
| Web interface | the programmatic access to the same capabilities | programmatic access |
| Identical indicator | the true or false part of the answer telling whether the files have the same content | file storage |
| Last access | the date a file was last read, used by the automatic clean up | file storage |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 | 3 | NOT-IMPLEMENTABLE (accepted syntax and type of the size) |
| 2 | 1 | 7 | NOT-IMPLEMENTABLE (JSON number forms, test coverage of the rules, FR-NEW-005 split) |
| 3 | 1 | 4 | NOT-IMPLEMENTABLE (a number cannot be named as written on the tool surface) |
| 4 | 5 | 5 | NOT-IMPLEMENTABLE (descriptions unpinned, schema type, rendering of odd values, missing boundary tests). Scope decision: the refusal no longer repeats the given value |
| 5 | 1 | 0 | NOT-IMPLEMENTABLE (7.4 contradicted FR-NEW-003; pure precision, fixed; all counts, diffs and citations verified by the auditor) |

**Amendments applied:** round 5 fix to 7.4, tool and parameter descriptions pinned in FR-MOD-001, FR-NEW-003 simplified (no echoed value), E2E-043 to E2E-049, FR-NEW-003 rules, FR-NEW-005 split into FR-NEW-005 and FR-NEW-010, Section 7.1, E2E-026 to E2E-042, Section 14.1 citations, 14.3, 14.4.
**No implementation detail outside Section 14:** PASS (the parameter name appears only inside the pinned user visible message)

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Comparison always uses 3 lines | `crates/core/src/tools/diff_files.rs:6` import, `:63` passes `DEFAULT_CONTEXT`; `crates/core/src/core/diff.rs:11` (= 3) |
| Engine already takes a size | `crates/core/src/core/diff.rs:56-77` (`unified_with_context`, `unified_labeled(.., context)`) |
| Write/edit diffs use the default | `crates/core/src/core/diff.rs:48-51` (`unified`) |
| Tool description says 3 lines | `crates/core/src/mcp/server.rs:2046`, `crates/core/src/tools/diff_files.rs:77` (test only `register()` glue, `:70-100`), `TOOL_CONTRACT.txt:182`, `tool-contract-golden.json`, `README.md:276` |
| Four web values, first missing named | `crates/core/src/api/dataplane.rs:358-385` |
| Tool arguments struct | `crates/core/src/mcp/server.rs:625-634` |
| Other numbers clamp negatives silently | `crates/core/src/api/dataplane.rs:1154`, `:1436` |
| 48 comparison tests | `rg -c "#\[tokio::test\]\|#\[test\]" crates/core/src/tools/diff_files_e2e.rs` returned 48 |
Test command: `cargo test --workspace`

### 14.2 Class sweep
Not applicable.

### 14.3 Design inputs
- The parameter must be optional in the generated tool schema; the frozen contract changes (description and
  new parameter). Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`
  and update `TOOL_CONTRACT.txt`. Signal: the three contract golden tests fail.
- `Q::int_or` (`dataplane.rs:291`) parses `i64` with a different message and would turn an empty value into a
  default or another message; the web path needs its own parse to produce the FR-NEW-003 text. Signal: E2E-015, 018.
- The size must be validated at the shared entry `diff_files` (`tools/diff_files.rs:26`) so both doors agree,
  before `normalize_path`. Signal: E2E-016, 017.
- Tool surface: missing required values fail in argument deserialization, before the size check
  (`mcp/server.rs:625-634`). No signal needed.
- The web path must read the first `context_lines` pair raw from the query vector: `Q::opt` (`dataplane.rs:272-274`)
  skips empty values and would pick a later occurrence. Signal: E2E-015, E2E-038.
- The upper bound must be checked for overflow of the platform line count type when handed to the diff engine
  (`group_diff_ops` in the `similar` crate with 10000000). Signal: E2E-006.
- The OpenAPI document (`crates/core/src/api/openapi.rs:545`) lists the web parameters. Signal: its tests.

### 14.4 Pinned names
- Parameter (tool surface and web query): `context_lines`. Optional. Its published schema must not restrict
  its JSON type (no `type` keyword, or all JSON types); the handler receives the raw value and refuses every
  non null value that is not an accepted whole number or digit text with the FR-NEW-003 message, instead of a
  protocol level deserialization error (`DiffFilesArgs`, `mcp/server.rs:625-634`). Signal: E2E-011, 012, 013,
  029 to 036 through the tool surface. The test registry glue (`tools/diff_files.rs:70-100`) must declare it
  identically or the three contract golden tests diverge.
