---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0004
min_tier: 2
Complexity: S
Files touched: crates/core/src/tools/archive.rs
---

# US-0005 — Format detection, unsupported extension, tar+password gate

## Objective

Determine the archive format from the normalized path's filename extension, reject unsupported
extensions with `ERR_NOT_SUPPORTED`, and reject a `password` supplied against any tar-family
format before the archive's bytes are even opened. This runs entirely on the filename string, no
archive-decode dependency yet.

## Technical Context

### Stack

Rust 2024, no new dependency: extension matching is plain string logic on the already-normalized
path from US-0004. The actual decode crates (`tar`, `flate2`, `bzip2`, `lzma-rs`, `zip`,
`sevenz-rust2`) are added in US-0006, not here.

### Relevant File Structure

- `crates/core/src/tools/archive.rs` (from US-0004) — this story adds the format-detection
  function and the two rejection checks after the directory check.

### Existing Patterns Referenced

None beyond the general `fs.*` convention of failing fast with a typed `ToolError` before doing
any expensive work.

### Decisions That Govern This Story

None of §17's 11 decisions specifically govern format detection; `FR-NEW-005`, `FR-NEW-006`,
`FR-NEW-008` are the governing requirements.

### Applicable NFRs

None beyond the general performance note (§7.1): no new resource guard, this is cheap string
matching.

### Bounded Context

Archive parsing (`tools::archive`) — format detection is this context's first responsibility,
owning no filesystem state (§4.5).

## Functional Requirements

**FR-NEW-005** [EARS-U] The system SHALL determine the archive format from the normalized `path`'s
filename extension, matched case-insensitively, using exactly this mapping, compound suffixes
checked before the plain `.tar` suffix: `.zip` → zip; `.7z` → sevenz; `.tar.gz` or `.tgz` →
tar+gzip; `.tar.bz2` or `.tb2` → tar+bzip2; `.tar.xz` or `.txz` → tar+xz; `.tar` → tar
(uncompressed).

- Inputs: normalized `path`'s filename.
- Outputs: an internal `ArchiveFormat` enum value (zip / sevenz / tar{plain,gzip,bzip2,xz}).
- Business Rules: matching is case-insensitive; compound suffixes (`.tar.gz`, `.tgz`, etc.) are
  checked before the plain `.tar` suffix so `.tar.gz` never matches as plain `.tar`.

**FR-NEW-006** [EARS-O] IF the normalized `path`'s extension matches none of the suffixes in
`FR-NEW-005` THEN THE system SHALL fail with `ERR_NOT_SUPPORTED`, message naming the exact
extension found, e.g. `"unsupported archive extension: '.rar'"`.

- Inputs: the unmatched extension string.
- Outputs: `Err(ToolError::not_supported(...))` naming the exact extension.
- Business Rules: this includes `.rar` (non-goal, §3.2), bare `.gz` (not `.tar.gz`), and any
  split-archive-style name like `.zip.001` — all are simply unmatched extensions under this
  mapping, no special-casing needed.

**FR-NEW-008** [EARS-O] IF the format selected by `FR-NEW-005` is `tar` in any of its four
variants (plain, gzip, bzip2, xz) AND `password` is supplied THEN THE system SHALL fail with
`ERR_INVALID_ARGUMENT`, message `"password is not applicable to this archive format"`, checked
before the archive's bytes are opened at all.

- Inputs: the selected format, the `password` parameter.
- Outputs: `Err(ToolError::invalid_argument(...))` when both conditions hold.
- Business Rules: this check runs immediately after format detection and before any byte of the
  archive is read, for all four tar variants uniformly.

## Acceptance Tests

### Test Data

| path | password | expected |
|---|---|---|
| `/uploads/data.rar` | none | `ERR_NOT_SUPPORTED`, message contains `.rar` |
| `/uploads/data.gz` | none | `ERR_NOT_SUPPORTED`, message contains `.gz` |
| `/uploads/parts.zip.001` | none | `ERR_NOT_SUPPORTED`, message contains `.001` |
| `/uploads/plain.tar.gz` | `"anything"` | `ERR_INVALID_ARGUMENT`, "password is not applicable..." |
| `/uploads/plain.tar` | `"x"` | `ERR_INVALID_ARGUMENT`, "password is not applicable..." |
| `/uploads/plain.tar.bz2` | `"x"` | `ERR_INVALID_ARGUMENT`, "password is not applicable..." |

#### E2E-NEW-022 — `.rar` extension

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006.
- Preconditions: a file at `/uploads/data.rar` (any bytes; extension alone drives this check).
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_NOT_SUPPORTED"` And `message` contains `".rar"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-034 — Bare `.gz` (not `.tar.gz`) is an unsupported extension

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006.
- Preconditions: a file at `/uploads/data.gz` (plain gzip, not a tar).
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_NOT_SUPPORTED"` And `message` contains `".gz"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-035 — Multi-volume-style `.zip.001` is an unsupported extension (Non-Goal)

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006.
- Preconditions: a file at `/uploads/parts.zip.001` (a split-archive-style name).
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_NOT_SUPPORTED"` And `message` contains `".001"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-024 — Password supplied against a `.tar.gz` archive

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008.
- Preconditions: a valid, unencrypted `.tar.gz` at `/uploads/plain.tar.gz`.
- Steps: Given the fixture, When `extract_archive(.., password=Some("anything"), ..)`, Then
  `Err` with `code == "ERR_INVALID_ARGUMENT"` And `message == "password is not applicable to
  this archive format"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-038 — Password supplied against a plain `.tar`

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008.
- Preconditions: a valid, unencrypted plain `.tar` at `/uploads/plain.tar`.
- Steps: Given the fixture, When `extract_archive(.., password=Some("x"), ..)`, Then `Err` with
  `code == "ERR_INVALID_ARGUMENT"` And `message == "password is not applicable to this archive
  format"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-039 — Password supplied against a `.tar.bz2`

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008.
- Preconditions: a valid, unencrypted `.tar.bz2` at `/uploads/plain.tar.bz2`.
- Steps: as E2E-NEW-038, against `.tar.bz2`.
- Cleanup: none. Priority: P1.

## Constraints

- Files Not to Touch: no decode crates added yet (US-0006). No entry safety checks yet
  (US-0007).
- Dependencies Not to Add: `tar`, `flate2`, `bzip2`, `lzma-rs`, `zip`, `sevenz-rust2` — none of
  these land until US-0006.
- Patterns to Avoid: do not open or read any archive bytes in this story; format detection and
  the password gate operate on the filename string and the `password` parameter only.
- Scope Boundary: extension matching and the tar-password gate only.

## Non Regression

No existing tool behavior changes.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
