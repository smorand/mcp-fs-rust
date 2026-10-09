---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0006, US-0002 (informational)
min_tier: 2
Complexity: M
Files touched: crates/core/src/tools/archive.rs
---

# US-0007 — Symlink/hardlink/device rejection; zip-slip entry-path rejection; void-on-disqualify ordering

## Objective

Reject any entry whose declared type is not a regular file or directory (symlink, hardlink,
device, FIFO, socket), reject any entry whose path escapes the destination (zip-slip, absolute
path, backslash-style traversal), and guarantee the all-or-nothing void-on-disqualify ordering:
no byte written, no directory created, until every entry has passed these checks. This is also
where **DRIFT-002** gets resolved in practice: read the pinned `zip` crate's actual
`external_attributes`/`is_symlink()` surface before writing the check (`[[US-0002]]`).

## Technical Context

### Stack

Rust 2024. Uses the entry list produced by US-0006 (format-specific decode). No new dependency.

### Relevant File Structure

- `crates/core/src/tools/export.rs:116-131` — the private `ensure_no_escape` helper, walking the
  raw path component-by-component treating `..` as a one-level decrease; this spec's `FR-NEW-014`
  reuses the same walking technique, applied to archive entry paths rather than caller-supplied
  `paths`.
- `crates/core/src/tools/archive.rs` (from US-0006) — this story adds the per-entry safety-check
  loop after the entry list is produced and before any write-pass code (US-0009) exists.

### Existing Patterns Referenced

`ensure_no_escape`'s walking technique (`export.rs:122-130`): split on `/`, walk component by
component, `..` is a one-level decrease, any other non-empty, non-`.` component is a one-level
increase, reject if the running total goes below zero at any point.

### Decisions That Govern This Story

- **DEC-008**: 7z symlink/hardlink/device detection is accepted as lower-fidelity than the tar
  and zip branches, because the 7z format has no universally-implemented symlink
  representation. Any 7z entry this branch cannot positively identify as a symlink is instead
  written as an ordinary regular file — a safe default, since `FR-NEW-014`'s independent
  path-safety check still applies.
- **A finding 2** (§18): `FR-NEW-013`'s zip-symlink bit-extraction (`external_attributes >> 16`)
  is stated in the requirement but not independently re-derived against the pinned `zip` crate's
  actual field name/type. Resolve by reading the pinned version's `ZipFile`/`ZipFileData` type
  before writing the check (`[[US-0002]]`, DRIFT-002); prefer a native `is_symlink()` method if
  the pinned version exposes one.

### Applicable NFRs

§7.2 Security: this story is the direct implementation of the zip-slip rejection, the
symlink/hardlink/device rejection, and the all-or-nothing enforcement named in §7.2 as this
spec's security surface.

### Bounded Context

Archive parsing (`tools::archive`) — per-entry safety checks operate purely on the in-memory
decoded entry list produced by US-0006, touching no filesystem state (§4.5).

## Functional Requirements

**FR-NEW-013** [EARS-O] IF an entry's declared type, as reported by the format library, is a
symlink, hardlink, device, FIFO, or socket (for `tar`: any `tar::EntryType` other than `Regular` or
`Directory`; for `zip`: an entry whose Unix mode bits in `external_attributes` mark it `S_IFLNK`,
or an equivalent native symlink indicator if the pinned `zip` crate version exposes one; for `7z`:
an entry whose `sevenz_rust2` attributes mark it as a non-regular, non-directory file) THEN THE
system SHALL fail the WHOLE call with `ERR_NOT_SUPPORTED`, message naming the entry's path and its
type, e.g. `"archive entry 'link' is a symlink, which is not supported"`.

- Inputs: each entry's declared type from the format library.
- Outputs: `Err(ToolError::not_supported(...))` naming the entry path and type.
- Business Rules: applies uniformly across tar/zip/7z, with the 7z branch's accepted
  lower-fidelity per `DEC-008`.

**FR-NEW-014** [EARS-O] IF an entry's path, as recorded in the archive, is absolute (begins with
`/`) OR, when split on `/` and walked component by component treating `..` as a one-level
decrease and any other non-empty, non-`.` component as a one-level increase, the running total
reaches a decrease below zero at any point, THEN THE system SHALL fail the WHOLE call with
`ERR_PATH_OUT_OF_BOUNDS`, message naming the escaping entry's path, using the same walking
technique `tools::export::ensure_no_escape` already uses (`export.rs:122-130`), applied to entry
paths rather than caller-supplied `paths`.

- Inputs: each entry's raw recorded path.
- Outputs: `Err(ToolError::path_out_of_bounds(...))` naming the escaping entry path.
- Business Rules: an absolute path is rejected outright; a relative path is walked component by
  component; backslash-style separators (Windows-style traversal) are covered by the same check,
  mirroring `export.rs`'s own windows-style test case.

**FR-NEW-015** [EARS-UB] The system SHALL NOT write any byte of any entry, nor create any
directory, until every entry of the archive has passed the checks in `FR-NEW-013`, `FR-NEW-014`,
`FR-NEW-017`, and `FR-NEW-018`: a single disqualifying entry anywhere in the archive voids the
WHOLE call, even when most entries are benign.

- Inputs: the full entry list, including this story's `FR-NEW-013`/`FR-NEW-014` checks (the
  `FR-NEW-017`/`FR-NEW-018` conflict checks are implemented in US-0008; this story's own
  acceptance only exercises void-on-disqualify for its own two checks).
- Outputs: no partial write state after a failure.
- Business Rules: the check loop must run to completion over every entry before any write-pass
  code runs, even if an earlier entry in archive order already disqualifies the call — this
  story's loop collects/short-circuits on the first disqualifying entry it finds (archive order),
  since the spec's own SC-006 cross-scenario note treats "first colliding/escaping path in
  archive entry order" as the naming contract, which implies sequential evaluation is
  acceptable as long as nothing is written before the loop finishes.

## Acceptance Tests

### Test Data

| Fixture | expected |
|---|---|
| `.tar`: `good.txt` + `../../etc/passwd` | `ERR_PATH_OUT_OF_BOUNDS`, names `../../etc/passwd` |
| `.zip`: `good.txt` + absolute `/etc/passwd` | `ERR_PATH_OUT_OF_BOUNDS`, names `/etc/passwd` |
| `.zip`: entry named `..\\..\\windows\\system32\\evil.dll` | `ERR_PATH_OUT_OF_BOUNDS` |
| `.tar`: `good.txt` + symlink entry `link` → `/etc/passwd` | `ERR_NOT_SUPPORTED`, names `link`, `symlink` |
| `.tar`: `good.txt` + hardlink entry (`EntryType::Link`) | `ERR_NOT_SUPPORTED`, names entry, hardlink type |
| `.zip`: entry with `external_attributes` encoding `S_IFLNK` named `link` + `good.txt` | `ERR_NOT_SUPPORTED`, names `link` |

#### E2E-NEW-014 — Archive with one zip-slip entry and one benign entry

- Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014,015.
- Preconditions: a `.tar` containing `good.txt = b"benign"` and an entry literally named
  `../../etc/passwd` with content `b"pwned"`, at `/uploads/evil.tar`.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_PATH_OUT_OF_BOUNDS"` And `message` contains `"../../etc/passwd"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-015 — Same archive: benign entry's content never appears anywhere on disk

- Category: Edge. Scenario: SC-008. Requirements: FR-NEW-015.
- Preconditions: same as E2E-NEW-014.
- Steps: Given the fixture, When `extract_archive(..)` fails, Then
  `client.exists("/uploads/evil/good.txt").await.unwrap() == false` And a direct walk of the
  volume's blob store confirms the literal content `"pwned"` was never written anywhere
  (verification channel: blob-store walk, not the tool's own response).
- Cleanup: none. Priority: P0.

#### E2E-NEW-040 — Zip entry with an absolute path `/etc/passwd`

- Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014,015.
- Preconditions: a `.zip` with entries `good.txt = b"benign"` and an entry whose stored name is
  literally `/etc/passwd` (absolute).
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_PATH_OUT_OF_BOUNDS"` And `message` contains `"/etc/passwd"` And `good.txt`'s content is
  never written anywhere.
- Cleanup: none. Priority: P1.

#### E2E-NEW-041 — Backslash-style traversal entry

- Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014.
- Preconditions: a `.zip` with one entry whose stored name is
  `..\\..\\windows\\system32\\evil.dll`, mirroring `tools::export::ensure_no_escape`'s own
  windows-style test case.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_PATH_OUT_OF_BOUNDS"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-016 — Archive with a tar symlink entry and one benign entry

- Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013,015.
- Preconditions: a `.tar` built with the `tar` crate's symlink-entry API, containing `good.txt =
  b"benign"` and a symlink entry `link` → `/etc/passwd`, at `/uploads/evil.tar`.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_NOT_SUPPORTED"` And `message` contains `"link"` and `"symlink"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-017 — Archive with a tar hardlink entry

- Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013.
- Preconditions: same shape as E2E-NEW-016 but the special entry is a tar hardlink
  (`EntryType::Link`) instead.
- Steps: as E2E-NEW-016, Then `message` names the entry and a type naming it as a hardlink (not
  the word "symlink").
- Cleanup: none. Priority: P1.

#### E2E-NEW-018 — Zip entry with Unix mode `S_IFLNK`

- Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013.
- Preconditions: a `.zip` with one entry whose `external_attributes` encode Unix mode `0o120777`
  (`S_IFLNK`), content being a link-target string, named `link`, plus a benign `good.txt`.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_NOT_SUPPORTED"` And `message` contains `"link"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-019 — Symlink-carrying archive: benign entry's content never appears anywhere on disk

- Category: Edge. Scenario: SC-009. Requirements: FR-NEW-015.
- Preconditions: same as E2E-NEW-016.
- Steps: as E2E-NEW-015's verification, applied to this fixture: the benign entry's content
  never appears anywhere on disk after the failure.
- Cleanup: none. Priority: P0.

## Constraints

- Files Not to Touch: destination computation, conflict checks, quota charge (US-0008), write
  pass (US-0009).
- Dependencies Not to Add: none.
- Patterns to Avoid: do not write any byte or create any directory before this story's full
  check loop completes over every entry.
- Scope Boundary: entry-type rejection and entry-path-escape rejection only.

## Non Regression

No existing tool behavior changes. `tools::export::export_zip`'s own `ensure_no_escape` is
reused conceptually (same walking technique) but not modified.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
