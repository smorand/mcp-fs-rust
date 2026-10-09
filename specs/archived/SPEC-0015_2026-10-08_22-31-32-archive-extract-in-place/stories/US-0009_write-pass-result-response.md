---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0008
min_tier: 2
Complexity: L
Files touched: crates/core/src/tools/archive.rs, crates/core/src/core/fs_ops.rs
---

# US-0009 — Write pass; result counts; response shape

## Objective

Implement the write pass itself (directory creation then file writes, calling `VolumeClient`
primitives directly, never `core::fs_ops::write_bytes`, per `DEC-006`), promote
`core::fs_ops::ensure_parents` to `pub(crate)` for reuse, count `files_written`/`dirs_created`/
`bytes_written`, and return the success response shape. **This is the first story where
`fs.extract_archive` is callable end-to-end and most happy-path/edge E2E tests can actually go
green.**

## Technical Context

### Stack

Rust 2024. Uses `VolumeClient::makedirs`, `mkdir`, `write_bytes_atomic`, `touch_atime_mtime`
directly, plus `core::fs_ops::ensure_parents` once promoted. No new dependency.

### Relevant File Structure

- `crates/core/src/storage/volume.rs:62-68,79,130,157,180,184` — `VolumeClient` primitives:
  `exists`, `walk`, `write_bytes_atomic`, `makedirs`, `mkdir`, `touch_atime_mtime`
  (fire-and-forget).
- `crates/core/src/core/fs_ops.rs:589-615` — `core::fs_ops::write_bytes`, wraps
  `write_bytes_atomic` with its own `safety.charge_write` call at line 609; NOT called here, per
  `DEC-006` (double-charge risk and all-or-nothing violation risk).
- `crates/core/src/core/fs_ops.rs:1199-1208` — `ensure_parents`, currently a private `fn`; this
  story promotes it to `pub(crate) fn`, no change to its body or behavior.

### Existing Patterns Referenced

`tools::export::export_zip` calls `client.read_bytes`/`client.blob.put` directly rather than
through `core::fs_ops`, for files that do not fit that layer's per-call quota-charging shape —
this story's write pass follows the same precedent in the opposite direction (write instead of
read).

### Decisions That Govern This Story

- **DEC-006**: the write pass calls `VolumeClient::write_bytes_atomic` and `client.makedirs`
  directly, never `core::fs_ops::write_bytes`, because that wrapper's internal
  `safety.charge_write` call would double-charge against the single up-front total already
  charged in US-0008 (`FR-NEW-019`) and could spuriously fail mid-extraction after some files
  already landed, breaking all-or-nothing.

### Applicable NFRs

§7.4 Reliability: all-or-nothing semantics mean a failed call never leaves partial state; by
this story, every disqualifying check has already run (US-0005 through US-0008), so the write
pass itself is not expected to fail under normal operation.

### Bounded Context

Filesystem write (existing `VolumeClient` + `SafetyManager`, consumed not owned, §4.5): this
story's write-pass calls are the one place this spec's archive-parsing context hands off to that
context's existing primitives.

## Functional Requirements

**FR-NEW-021** [EARS-U] Once every check in `FR-NEW-003` through `FR-NEW-020` has passed, THE
system SHALL perform the write pass as follows, calling `VolumeClient` primitives directly (never
`core::fs_ops::write_bytes`, per `DEC-006`): first, `client.makedirs(destination, true)`
unconditionally (idempotent, also covers a zero-entry archive); then, for every directory entry in
archive order, `client.makedirs(entry_destination_path, true)`; then, for every regular-file entry
in archive order, `core::fs_ops::ensure_parents` (promoted `pub(crate)` by `FR-NEW-022`) followed by
`client.write_bytes_atomic(entry_destination_path, &decoded_bytes)` and
`client.touch_atime_mtime(entry_destination_path)`.

- Inputs: the destination (`FR-NEW-016`), the fully-checked entry list.
- Outputs: files and directories written to the volume.
- Business Rules: directories first (destination root, then every directory entry), then files,
  both in archive order.

**FR-NEW-022** [EARS-U] The system SHALL promote `core::fs_ops::ensure_parents`
(`fs_ops.rs:1199-1208`) from a private `fn` to a `pub(crate) fn`, with no change to its body or
behavior, so `tools::archive` reuses it instead of duplicating its parent-directory-creation
logic.

- Inputs: none (visibility change only).
- Outputs: `ensure_parents` callable from `tools::archive`.
- Business Rules: zero behavior change to the existing function.

**FR-NEW-023** [EARS-U] The system SHALL count `files_written` as the number of regular-file
entries written in `FR-NEW-021`, `dirs_created` as the number of directories created in
`FR-NEW-021` (the unconditional destination directory plus every directory entry) that did **not**
already exist immediately before this call, and `bytes_written` as the sum of decoded bytes
actually written across all regular-file entries.

- Inputs: the write pass's own actions.
- Outputs: three integer counts.
- Business Rules: `dirs_created` only counts directories that did not already exist; intermediate
  parent directories created transitively by `makedirs`'s own recursive behavior are not counted
  individually against this total (only the archive's own directory-shaped entries plus the
  destination root are, per `E2E-NEW-051`).

**FR-NEW-024** [EARS-U] On success, THE system SHALL return `{"destination": <string, the
destination from FR-NEW-016>, "files_written": <integer>, "dirs_created": <integer>,
"bytes_written": <integer>}`.

- Inputs: the counts from `FR-NEW-023`, the destination from `FR-NEW-016`.
- Outputs: the JSON response object.
- Business Rules: the response key is `destination`, never `extracted_to` or any other alias
  (spec §13 Consistency Notes).

## Acceptance Tests

### Test Data

| Archive | destination | expected response |
|---|---|---|
| `.tar.gz`: `a.txt=b"hello"`, `sub/b.txt=b"world"` | default `/uploads/report` | `{destination:"/uploads/report", files_written:2, dirs_created:1, bytes_written:10}` |
| six format variants, each one entry `only.txt=b"x"` | default | `Ok`, file exists with `b"x"` for all six |

#### E2E-NEW-001 — Extract `.tar.gz` with default destination

- Category: Happy. Scenario: SC-001. Requirements: FR-NEW-001,002,003,005,009,012,016,019,021,
  022,023,024,025,026.
- Driver: `tools::archive::extract_archive`, called directly (unit-level) and once more through
  `McpServer::fs_extract_archive` (integration-level) in the same test module, mirroring
  `export.rs`'s own dual-level pattern.
- Preconditions: fixture volume `proj` owned by `owner@test.com`; a `.tar.gz` archive built in
  the test (via the `tar`+`flate2` crates themselves) containing `a.txt = b"hello"` and
  `sub/b.txt = b"world"`, written to `/uploads/report.tar.gz`.
- Steps: Given the fixture, When `extract_archive(&state, "proj", "/uploads/report.tar.gz",
  None, false, None)` is awaited, Then `Ok` with `destination == "/uploads/report"`,
  `files_written == 2`, `dirs_created == 1`, `bytes_written == 10` (5+5), And
  `client.read_bytes("/uploads/report/a.txt").await.unwrap() == b"hello"`, And
  `client.read_bytes("/uploads/report/sub/b.txt").await.unwrap() == b"world"`.
- Cleanup: fixture dropped at end of test. Priority: P0.

#### E2E-NEW-002 — Extract `.tgz`, `.tb2`, `.txz`, `.tar`, `.zip`, `.7z` each, default destination

- Category: Edge. Scenario: SC-001. Requirements: FR-NEW-005.
- Preconditions: one fixture per format, each containing one entry `only.txt = b"x"`, built
  with the matching crate at `/u/f.<ext>` for each of `.zip`, `.7z`, `.tar`, `.tgz`, `.tb2`,
  `.txz`.
- Steps: Given each fixture in turn, When `extract_archive` is called with no destination, Then
  `Ok` And `/u/f/only.txt` exists with content `b"x"` for every one of the six.
- Cleanup: as above. Priority: P0.

#### E2E-NEW-003 — Extract `.zip` with explicit `destination` override

- Category: Happy. Scenario: SC-002. Requirements: FR-NEW-016.
- Preconditions: a `.zip` at `/uploads/report.zip` with one entry `x.txt = b"y"`; `/extracted`
  does not exist.
- Steps: Given the fixture, When `extract_archive(.., path="/uploads/report.zip",
  destination=Some("/extracted"), ..)`, Then `destination == "/extracted"` And
  `/extracted/x.txt` exists with content `b"y"` And `/uploads/report` (the would-be default)
  does not exist.
- Cleanup: none. Priority: P0.

#### E2E-NEW-004 — Extract AES-encrypted `.7z` with correct password

- Category: Happy. Scenario: SC-003. Requirements: FR-NEW-009,010,011,012.
- Preconditions: a `.7z` built with `sevenz_rust2`'s AES-256 password encryption, password
  `"swordfish"`, one entry `s.txt = b"secret"`, at `/uploads/secret.7z`.
- Steps: Given the fixture, When `extract_archive(.., password=Some("swordfish"), ..)`, Then
  `Ok` And `/uploads/secret/s.txt == b"secret"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-005 — Extract AES-encrypted and legacy ZipCrypto `.zip` with correct password

- Category: Happy. Scenario: SC-003. Requirements: FR-NEW-009,012.
- Preconditions: two zips: one AES-encrypted (via `zip`'s `aes-crypto` feature), one legacy
  ZipCrypto-encrypted, both password `"p"`, one entry `z.txt = b"z"` each.
- Steps: for each, Given the fixture, When `extract_archive(.., password=Some("p"), ..)`, Then
  `Ok` And the entry's content matches.
- Cleanup: none. Priority: P0.

#### E2E-NEW-007 — Retry after E2E-NEW-006 with correct password succeeds

- Category: Happy. Scenario: SC-004. Requirements: FR-NEW-010,019.
- Preconditions: continuation of E2E-NEW-006 in the same test, same fixture state (no write
  happened).
- Steps: Given the state right after E2E-NEW-006's failure, When `extract_archive(..,
  password=Some("correct"), ..)`, Then `Ok` And the entry exists with its expected content.
- Cleanup: none. Priority: P1.

#### E2E-NEW-009 — Retry after E2E-NEW-008 with correct password succeeds

- Category: Happy. Scenario: SC-005. Requirements: FR-NEW-011.
- Preconditions: continuation of E2E-NEW-008.
- Steps: Given the state right after E2E-NEW-008's failure, When `extract_archive(..,
  password=Some("correct"), ..)`, Then `Ok` And the entry exists with its expected content.
- Cleanup: none. Priority: P1.

#### E2E-NEW-012 — Same collision, `overwrite=true`

- Category: Happy. Scenario: SC-007. Requirements: FR-NEW-018.
- Preconditions: identical to E2E-NEW-010 (US-0008's fixture).
- Steps: Given the fixture, When `extract_archive(.., overwrite=true, ..)`, Then `Ok` And
  `/uploads/report/a.txt == b"new"` And `/uploads/report/sub/b.txt == b"new2"`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-013 — `overwrite=true` against a pre-existing directory entry

- Category: Edge. Scenario: SC-007. Requirements: FR-NEW-018,022.
- Preconditions: a `.zip` with a directory entry `sub/` and a file `sub/c.txt = b"c"`;
  `/uploads/report/sub` already exists as a directory (created independently, not by this
  archive) with an unrelated file `/uploads/report/sub/other.txt` already inside it.
- Steps: Given the fixture, When `extract_archive(.., overwrite=true, ..)`, Then `Ok` And
  `/uploads/report/sub/other.txt` still exists (the directory was reused, not
  recreated/emptied) And `/uploads/report/sub/c.txt == b"c"`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-027 — Structural: `tools::archive` calls `core::fs_ops::ensure_parents`, no duplicate loop

- Category: Edge (structural). Requirements: FR-NEW-022.
- Driver: a `#[test]` (non-async, no fixture) reading `crates/core/src/tools/archive.rs`'s
  source text.
- Steps: Given the file at `crates/core/src/tools/archive.rs`, When read as a string, Then it
  contains the literal substring `ensure_parents` (proving reuse) And it does not define a
  function whose body re-implements a parent-directory-walk loop (grepped for the literal
  substring `rfind('/')` combined with a `makedirs` call outside of a call to `ensure_parents`
  — absence asserted).
- Cleanup: none. Priority: P2.

#### E2E-NEW-050 — `overwrite=true` with zero actual collisions is a no-op success

- Category: Edge. Scenario: SC-007. Requirements: FR-NEW-018.
- Preconditions: a `.zip` with entries `a.txt = b"a"` and `b.txt = b"b"`; destination
  `/uploads/report` does not exist at all (nothing to collide with).
- Steps: Given the fixture, When `extract_archive(.., overwrite=true, ..)`, Then `Ok` And both
  files exist with their expected content, proving `overwrite=true` changes nothing about the
  happy path when there is genuinely no collision.
- Cleanup: none. Priority: P2.

#### E2E-NEW-051 — Destination override pointing at a nested, not-yet-existing path two levels deep

- Category: Edge. Scenario: SC-002. Requirements: FR-NEW-016,022.
- Preconditions: a `.zip` with one entry `x.txt = b"y"`; `destination = "/a/b/c"`, none of
  `/a`, `/a/b`, `/a/b/c` exist yet.
- Steps: Given the fixture, When `extract_archive(.., destination=Some("/a/b/c"), ..)`, Then
  `Ok` And `/a/b/c/x.txt == b"y"` And `dirs_created >= 1` for the destination itself
  (intermediate parents `/a` and `/a/b` are created by `client.makedirs`'s own recursive
  behavior, not counted individually against `dirs_created`, which counts only the archive's
  own directory-shaped entries plus the destination root per `FR-NEW-023`).
- Cleanup: none. Priority: P1.

#### E2E-NEW-052 — AES zip with a zero-byte entry, password correct

- Category: Edge. Scenario: SC-003. Requirements: FR-NEW-009,012.
- Preconditions: an AES-encrypted `.zip`, password `"p"`, with one entry `empty.txt` of zero
  bytes.
- Steps: Given the fixture, When `extract_archive(.., password=Some("p"), ..)`, Then `Ok` And
  `client.read_bytes(".../empty.txt").await.unwrap() == b""` And `bytes_written == 0` And
  `files_written == 1`.
- Cleanup: none. Priority: P2.

#### E2E-NEW-053 — Same archive shape as E2E-NEW-010 but with no pre-existing collision succeeds

- Category: Happy. Scenario: SC-006. Requirements: FR-NEW-017.
- Preconditions: the same `.zip` shape as E2E-NEW-010 (`a.txt`, `sub/b.txt`) but
  `/uploads/report` does not exist at all beforehand (no collision).
- Steps: Given the fixture, When `extract_archive(.., overwrite=false, ..)` (default), Then
  `Ok`, proving the conflict-detection machinery itself does not misfire when nothing actually
  collides.
- Cleanup: none. Priority: P1.

#### E2E-NEW-054 — Same archive shape as E2E-NEW-014 but with no escaping entries succeeds

- Category: Happy. Scenario: SC-008. Requirements: FR-NEW-014,015.
- Preconditions: the same `.tar` shape as E2E-NEW-014 (`good.txt`) but with no
  `../../etc/passwd`-style entry at all.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Ok` And `good.txt`'s content is
  written normally, proving the path-safety check itself does not misfire on an entirely benign
  archive.
- Cleanup: none. Priority: P1.

#### E2E-NEW-055 — Same archive shape as E2E-NEW-016 but with no special entries succeeds

- Category: Happy. Scenario: SC-009. Requirements: FR-NEW-013,015.
- Preconditions: the same `.tar` shape as E2E-NEW-016 (`good.txt`) but with no symlink entry at
  all.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Ok`, proving the entry-type check
  itself does not misfire on an archive containing only regular files.
- Cleanup: none. Priority: P1.

#### E2E-NEW-056 — Extension matched case-insensitively (`.ZIP` uppercase) succeeds

- Category: Edge. Scenario: SC-011. Requirements: FR-NEW-005.
- Preconditions: a valid zip archive at `/uploads/REPORT.ZIP` (uppercase extension), one entry
  `x.txt = b"y"`.
- Steps: Given the fixture, When `extract_archive(.., path="/uploads/REPORT.ZIP", ..)`, Then
  `Ok` And the format is correctly resolved as `zip` despite the uppercase extension, per
  `FR-NEW-005`'s case-insensitive matching clause.
- Cleanup: none. Priority: P1.

## Constraints

- Files Not to Touch: audit/tracing (US-0010), REST route/OpenAPI (US-0011), tool contract
  (US-0012).
- Dependencies Not to Add: none.
- Patterns to Avoid: never call `core::fs_ops::write_bytes` from the write pass (`DEC-006`).
- Scope Boundary: write pass, counts, response shape only.

## Non Regression

`core::fs_ops::ensure_parents`'s existing callers (if any beyond this new one) keep working
identically after the visibility change.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
