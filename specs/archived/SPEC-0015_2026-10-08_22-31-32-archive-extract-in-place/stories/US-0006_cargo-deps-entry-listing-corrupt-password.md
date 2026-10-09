---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0005, US-0001 (informational), US-0002 (informational), US-0003
min_tier: 2
Complexity: M
Files touched: crates/core/src/tools/archive.rs, Cargo.toml, crates/core/Cargo.toml
---

# US-0006 — Cargo dependencies; zip/7z entry listing; corrupt-archive detection; password required/incorrect handling

## Objective

Add the five new Cargo dependencies (`tar`, `flate2`, `bzip2`, `lzma-rs`, `sevenz-rust2`) plus the
`aes-crypto` feature on the existing `zip` dependency, open the archive bytes for the format
selected in US-0005, detect corruption, and implement the entry-listing step for `zip`/`sevenz`
archives including password-required and incorrect-password detection. This is also where
**DRIFT-001** gets resolved in practice: read the pinned `zip`/`sevenz-rust2` crate docs for real
metadata-without-password behavior before writing the decode loop (`[[US-0001]]`).

## Technical Context

### Stack

Rust 2024. New workspace dependencies, verified via context7/crates.io before pinning (per the
`rust` skill's Context7 Workflow and this project's own verified-live table in spec §9.5):

| Crate | Version | Feature(s) | Purpose |
|---|---|---|---|
| `zip` (existing) | `2` (unchanged major) | `deflate` (existing) + `aes-crypto` (new) | AES-encrypted zip decrypt |
| `tar` | `0.4` | default | Pure-Rust tar reader |
| `flate2` | `1` | default (pure-Rust `miniz_oxide`) | `.tar.gz`/`.tgz` decompression |
| `bzip2` | `0.6` | default (`libbz2-rs-sys`) | `.tar.bz2`/`.tb2` decompression |
| `lzma-rs` | `0.3` | default | `.tar.xz`/`.txz` decompression |
| `sevenz-rust2` | `0.20` | `util`, `aes256` | `.7z` decompression |

### Relevant File Structure

- `Cargo.toml` (workspace `[workspace.dependencies]`) and `crates/core/Cargo.toml`
  (workspace-true references) per the `rust` skill's workspace-dependency convention.
- `crates/core/src/tools/archive.rs` (from US-0005) — this story adds the open-and-decode step
  after the format-detection and tar-password-gate checks.

### Existing Patterns Referenced

`tools::export::export_zip` fully buffers an archive in memory with no size/count limit
(`export.rs:67-69`, `DEC-010` there) — this spec's `FR-NEW-012` follows the same fully-buffered
model for the decode step this story implements.

### Decisions That Govern This Story

- **DEC-007**: pin `sevenz-rust2`, not `sevenz-rust` (unmaintained advisory, repository deleted).
- **DEC-009**: this spec's own test design was authored without a fresh-context sub-agent audit;
  not directly actionable here but explains why the A findings below exist.
- **A finding 1** (§18): `FR-NEW-009`'s claim that zip entry metadata is readable without a
  password is unverified against the pinned `zip` crate's exact API. Resolve by reading the
  pinned version's docs.rs page before writing the decode loop (`[[US-0001]]`, DRIFT-001). If
  metadata truly requires the password for an AES-encrypted entry, route that case through
  `FR-NEW-010`/`FR-NEW-011` at the "can't even list" point instead of "can list, can't decode" —
  no new requirement needed.

### Applicable NFRs

§7.1 Performance: no new performance target beyond the existing fully-buffered model.
§7.2 Security: the password-never-logged prohibition (`FR-NEW-027`) applies to any error path
this story's password-handling code takes, even though the logging call itself is US-0010's
responsibility — do not format a log line containing the password anywhere in this story's code.

### Bounded Context

Archive parsing (`tools::archive`): entry enumeration, decoding, password handling — owns no
filesystem state (§4.5).

## Functional Requirements

**FR-NEW-007** [EARS-O] IF the archive's bytes, once read, do not parse as the format selected by
`FR-NEW-005` (corrupt, truncated, or wrong magic bytes) THEN THE system SHALL fail with
`ERR_INVALID_ARGUMENT`, message stating the archive is corrupt or not a valid file of that
format, distinct from the `ERR_NOT_SUPPORTED` of `FR-NEW-006`.

- Inputs: the raw archive bytes read from the volume.
- Outputs: `Err(ToolError::invalid_argument(...))` naming the corruption.
- Business Rules: distinct error code path from `FR-NEW-006`'s unsupported-extension case —
  `ERR_INVALID_ARGUMENT` here, `ERR_NOT_SUPPORTED` there.

**FR-NEW-009** [EARS-U] The system SHALL list every entry of a `zip` or `sevenz` archive —
relative path, declared type (regular file / directory / other), declared uncompressed size for a
regular file, and whether the entry is password-encrypted — without requiring a correct password,
except when the format's own header encryption (an entirely header-encrypted 7z archive) makes
the entry list itself unreadable without the password, in which case `FR-NEW-019`/`FR-NEW-020`
apply to that failure instead of this listing step.

- Inputs: the parsed `zip`/`sevenz` archive handle.
- Outputs: an in-memory list of `(relative_path, kind, declared_size, encrypted)` tuples.
- Business Rules: see DRIFT-001 resolution above — verify against the pinned crate version
  before assuming metadata is readable without a password for every case.

**FR-NEW-012** [EARS-U] The system SHALL decode every regular-file entry of the archive fully into
memory during the pre-scan pass (the same fully-buffered model `fs.export_zip` uses, per
`export.rs:67-69`'s own `DEC-010`), so that password correctness, entry safety, and the full
write pass all operate on already-decoded bytes with no second decode.

- Inputs: every regular-file entry across every supported format.
- Outputs: an in-memory map of entry path to fully-decoded bytes.
- Business Rules: decoding happens once, during the pre-scan pass, before any safety/conflict/
  quota check that depends on decoded bytes (`FR-NEW-020`) and before the write pass (US-0009).

**FR-NEW-010** [EARS-O] IF any entry in a `zip` or `sevenz` archive reports itself encrypted AND
no `password` was supplied THEN THE system SHALL fail with `ERR_PASSWORD_REQUIRED`, message
`"password required to extract this archive"`, before any entry is decoded or written.

- Inputs: the entry list's `encrypted` flags, the `password` parameter.
- Outputs: `Err(ToolError::password_required("password required to extract this archive"))`.
- Business Rules: checked before any entry decode begins.

**FR-NEW-011** [EARS-O] IF any entry in a `zip` or `sevenz` archive reports itself encrypted AND
a `password` was supplied AND decoding any such entry with that password fails specifically due to
an incorrect password (as distinguished from a general corruption error by the decoding library)
THEN THE system SHALL fail with `ERR_PASSWORD_REQUIRED`, message `"incorrect password for this
archive"`.

- Inputs: the supplied `password`, the decode attempt's result.
- Outputs: `Err(ToolError::password_required("incorrect password for this archive"))`.
- Business Rules: an explicitly-supplied empty string password is treated as a supplied-but-wrong
  password (this message), never the missing-password message of `FR-NEW-010`.

## Acceptance Tests

### Test Data

| Fixture | password | expected |
|---|---|---|
| `/uploads/fake.zip` = `b"not a zip file at all"` | none | `ERR_INVALID_ARGUMENT`, corrupt |
| truncated `.7z` (last 200 bytes cut) | none | `ERR_INVALID_ARGUMENT`, corrupt |
| `.tar.gz` with gzip magic zeroed | none | `ERR_INVALID_ARGUMENT`, corrupt |
| AES zip, correct pw `"correct"` | none | `ERR_PASSWORD_REQUIRED`, "password required..." |
| AES zip, correct pw `"correct"` | `"wrong"` | `ERR_PASSWORD_REQUIRED`, "incorrect password..." |
| AES zip, correct pw `"correct"` | `""` (empty string) | `ERR_PASSWORD_REQUIRED`, "incorrect password..." |

#### E2E-NEW-023 — `.zip`-named file with non-zip bytes

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007.
- Preconditions: a file at `/uploads/fake.zip` containing the plain-text bytes `b"not a zip
  file at all"`.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_INVALID_ARGUMENT"` And `message` states the archive is corrupt or not a valid zip.
- Cleanup: none. Priority: P0.

#### E2E-NEW-036 — Truncated `.7z` is corrupt

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007.
- Preconditions: a valid `.7z` whose last 200 bytes are truncated before being written to
  `/uploads/broken.7z`.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_INVALID_ARGUMENT"` And `message` states the archive is corrupt.
- Cleanup: none. Priority: P1.

#### E2E-NEW-037 — Corrupt gzip header inside a `.tar.gz` is corrupt

- Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007.
- Preconditions: a valid `.tar.gz` whose first four bytes (the gzip magic) are overwritten with
  `b"\x00\x00\x00\x00"` before being written to `/uploads/broken.tar.gz`.
- Steps: as E2E-NEW-036.
- Cleanup: none. Priority: P1.

#### E2E-NEW-006 — Encrypted archive, no password

- Category: Failure. Scenario: SC-004. Requirements: FR-NEW-010.
- Preconditions: an AES-encrypted `.zip` at `/uploads/secret.zip`, password `"correct"`, one
  entry.
- Steps: Given the fixture, When `extract_archive(.., password=None, ..)`, Then `Err` with
  `code == "ERR_PASSWORD_REQUIRED"` And `message == "password required to extract this
  archive"` And nothing exists under `/uploads/secret` (verified via `client.exists`).
- Cleanup: none. Priority: P0.

#### E2E-NEW-057 — Second wrong-password retry after E2E-NEW-006 still fails identically

- Category: Edge. Scenario: SC-004. Requirements: FR-NEW-010.
- Preconditions: continuation of E2E-NEW-006 (no password given, failed).
- Steps: Given the state right after E2E-NEW-006's failure, When `extract_archive(..,
  password=None, ..)` is called a second time, identically, Then it fails identically
  (`ERR_PASSWORD_REQUIRED`, same message), proving the failure is stateless and repeatable
  rather than drifting on retry.
- Cleanup: none. Priority: P2.

#### E2E-NEW-008 — Encrypted archive, wrong password

- Category: Failure. Scenario: SC-005. Requirements: FR-NEW-011.
- Preconditions: same fixture shape as E2E-NEW-006, password `"correct"`.
- Steps: Given the fixture, When `extract_archive(.., password=Some("wrong"), ..)`, Then `Err`
  with `code == "ERR_PASSWORD_REQUIRED"` And `message == "incorrect password for this archive"`
  And nothing exists under the destination.
- Cleanup: none. Priority: P0.

#### E2E-NEW-058 — Empty-string password is treated as wrong, not as absent

- Category: Edge. Scenario: SC-005. Requirements: FR-NEW-011.
- Preconditions: the E2E-NEW-008 fixture (password `"correct"`).
- Steps: Given the fixture, When `extract_archive(.., password=Some(""), ..)` (an explicit empty
  string, not `None`), Then `Err` with `code == "ERR_PASSWORD_REQUIRED"` And `message ==
  "incorrect password for this archive"` (the wrong-password message, not the missing-password
  one), because an explicitly supplied empty string is a supplied password that happens to be
  wrong, distinct from `None`.
- Cleanup: none. Priority: P1.

## Constraints

- Files Not to Touch: entry safety checks (symlink/hardlink/zip-slip, US-0007), destination
  computation and write pass (US-0008/US-0009) are not implemented here.
- Dependencies Not to Add: nothing beyond the six listed above — do not add an unlisted
  compression/archive crate.
- Patterns to Avoid: do not log the password in any error path (`FR-NEW-027`, enforced at code
  review even though the logging call itself is US-0010's).
- Scope Boundary: open/parse/corrupt-detect, entry listing, password required/incorrect only. No
  write pass.

## Non Regression

No existing tool behavior changes. `cargo audit`/`cargo deny check` must stay clean with the new
dependencies (`make security`).

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
