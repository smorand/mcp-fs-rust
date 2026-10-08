---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0006, US-0007
min_tier: 2
Complexity: L
Files touched: crates/core/src/tools/archive.rs
---

# US-0008 — Destination computation; no-clobber/overwrite; quota charge; decoded-size-vs-declared check

## Objective

Compute the extraction destination, implement the no-clobber conflict check (and its
`overwrite=true` bypass), charge the write quota once for the archive's total declared
uncompressed size, and reject any entry whose decoded bytes exceed its declared size. Five FRs
in one story, over the nominal T2 budget of 2-4 — justified explicitly here as a decision-density
override: every FR in this story is fully pinned by the spec, zero design choice left, and all
five operate in the same pre-scan stage over the same in-memory entry list, immediately before
the write pass (US-0009) begins.

## Technical Context

### Stack

Rust 2024. Uses the entry list from US-0006/US-0007 and existing `SafetyManager::normalize_path`
/ `SafetyManager::charge_write` / `VolumeClient::exists`. No new dependency.

### Relevant File Structure

- `crates/core/src/safety.rs:131-142` — `SafetyManager::charge_write(&self, person: &str,
  project: &str, num_bytes: i64) -> Result<()>`, synchronous, adds `num_bytes` to the in-memory
  per-session `bytes_written` counter only if the new total stays under `write_quota_bytes`,
  returning `Err` **without mutating the counter** otherwise (`safety.rs:134-138`).
- `crates/core/src/safety.rs:149-154` — `refund_write`, unused by this spec (see `DEC-006`).
- `crates/core/src/tools/archive.rs` (from US-0007) — this story adds destination computation,
  the conflict-check pass, the quota charge, and the size-mismatch guard, all before any write.

### Existing Patterns Referenced

`SafetyManager::normalize_path` for the `destination` parameter, exactly like any other `fs.*`
destination parameter (SC-002 cross-scenario note), never through the stricter entry-escape
check reserved for archive entries (`FR-NEW-014`, US-0007).

### Decisions That Govern This Story

- **DEC-003**: `destination` defaults to the archive's path with its recognized multi-part
  extension stripped. Rationale: predictable "extract here" mental model. Implemented by
  `FR-NEW-016`.
- **DEC-004**: conflict handling defaults to `overwrite=false`, rejecting the WHOLE call on any
  collision; an explicit `overwrite=true` is the deliberate, informed choice. Implemented by
  `FR-NEW-017`, `FR-NEW-018`.
- **DEC-005**: quota is charged once, as the archive's total declared uncompressed size, in the
  pre-scan pass, rather than per-entry during the write pass — a zip-bomb-style archive must be
  rejected before any byte lands on disk. Implemented by `FR-NEW-019`.
- **DEC-010**: the quota charge uses each entry's **declared** uncompressed size, not the actual
  decoded length, with `FR-NEW-020` as a separate guard against an entry lying about its
  declared size then decoding to something huge. Implemented by `FR-NEW-019`, `FR-NEW-020`.

### Applicable NFRs

§7.1 Performance: this story's quota charge is the project's only existing resource-consumption
guard for a bulk operation (no new bespoke limit). §7.4 Reliability: all-or-nothing semantics —
a failed conflict/quota/size check never leaves partial state on disk.

### Bounded Context

Straddles archive parsing (destination computation, size-mismatch check operate on the in-memory
entry list) and filesystem write (the conflict check's `exists` probes and the quota charge are
pure consumption of existing `VolumeClient`/`SafetyManager` primitives, §4.5) — this story is the
seam between the two contexts, adding no new primitive to either.

## Functional Requirements

**FR-NEW-016** [EARS-U] The system SHALL compute the extraction destination as: the normalized
form of the caller-supplied `destination` (via `SafetyManager::normalize_path`) when given;
otherwise the normalized `path` with its matched extension from `FR-NEW-005` stripped (e.g.
`/uploads/report.tar.gz` → `/uploads/report`; `/uploads/data.zip` → `/uploads/data`), preserving
the stem's original case.

- Inputs: optional caller `destination`, the normalized `path`, the matched extension.
- Outputs: the computed destination string.
- Business Rules: `destination` is normalized exactly like any other `fs.*` destination
  parameter, never through the entry-escape check.

**FR-NEW-017** [EARS-O] IF `overwrite` is `false` (the default) AND any entry's destination path
(the destination from `FR-NEW-016` joined with the entry's relative path, normalized) already
exists as either a file or a directory THEN THE system SHALL fail the WHOLE call with
`ERR_NO_CLOBBER`, message naming the first such colliding destination path in archive entry
order, before any write.

- Inputs: each entry's computed destination path, probed via `client.exists`.
- Outputs: `Err(ToolError::no_clobber(...))` naming the first colliding path in archive order.
- Business Rules: "first" means archive entry order, not lexicographic order.

**FR-NEW-018** [EARS-O] IF `overwrite` is `true` THEN THE system SHALL permit writing over an
existing file at an entry's destination path and SHALL reuse (not reject, not recreate) an
existing directory at an entry's destination path.

- Inputs: the `overwrite` flag.
- Outputs: the conflict check in `FR-NEW-017` is bypassed entirely.
- Business Rules: reuse, never recreate/empty, a pre-existing directory.

**FR-NEW-019** [EARS-U] The system SHALL sum the declared uncompressed size (`FR-NEW-009`) of
every regular-file entry across the whole archive and SHALL charge that single total via
`SafetyManager::charge_write(person, mount_id, total)` exactly once, before any write; IF that call
returns `Err` THEN THE system SHALL fail the WHOLE call with `ERR_WRITE_QUOTA_EXCEEDED` and SHALL
NOT have mutated the session's `bytes_written` counter, relying on `charge_write`'s own
fail-closed behavior (`safety.rs:134-138`) rather than a bespoke check.

- Inputs: every regular-file entry's declared uncompressed size.
- Outputs: `Err(ToolError::write_quota_exceeded(...))` on quota failure, counter unmutated.
- Business Rules: charged exactly once, after the conflict check, before any write.

**FR-NEW-020** [EARS-UB] The system SHALL NOT write, for any regular-file entry, more decoded
bytes than that entry's declared uncompressed size (`FR-NEW-009`); IF decoding an entry produces
more bytes than declared THEN THE system SHALL fail the WHOLE call with `ERR_INVALID_ARGUMENT`,
message naming the entry and stating it decompressed to more bytes than declared.

- Inputs: each entry's declared size vs. its actually-decoded byte length.
- Outputs: `Err(ToolError::invalid_argument(...))` naming the entry and the mismatch.
- Business Rules: this guard exists independently of the quota charge, closing the
  size-mismatch amplification vector `DEC-010` describes.

## Acceptance Tests

### Test Data

| Fixture | overwrite | expected |
|---|---|---|
| `.zip`: `a.txt`+`sub/b.txt`; `/uploads/report/a.txt` pre-exists (different content) | false | `ERR_NO_CLOBBER` names `a.txt` path |
| same, entry order `sub/b.txt` first, collision on that one | false | `ERR_NO_CLOBBER` names `sub/b.txt` path |
| `.zip`: `d/` + `d/x.txt`; `/uploads/report/d` pre-exists as a **file** | false | `ERR_NO_CLOBBER` names `/uploads/report/d` |
| `write_quota_bytes=100`; `.zip` entry declares 10,000 bytes | n/a | `ERR_WRITE_QUOTA_EXCEEDED` |
| crafted zip entry: declared size 1, decodes to 10 bytes | n/a | `ERR_INVALID_ARGUMENT`, size mismatch |
| tar entry: header size 1, actual content block larger | n/a | `ERR_INVALID_ARGUMENT`, size mismatch |
| 7z entry: folder-header declares smaller size than actual decompressed output | n/a | `ERR_INVALID_ARGUMENT`, size mismatch |

#### E2E-NEW-010 — Destination collision, `overwrite` omitted (defaults false)

- Category: Failure. Scenario: SC-006. Requirements: FR-NEW-017.
- Preconditions: a `.zip` at `/uploads/report.zip` with entries `a.txt = b"new"` and `sub/b.txt
  = b"new2"`; prior state has `/uploads/report/a.txt = b"old"` already written (different
  content), `/uploads/report/sub` does not yet exist.
- Steps: Given the fixture, When `extract_archive(.., overwrite=false, ..)` (or omitted), Then
  `Err` with `code == "ERR_NO_CLOBBER"` And `message` contains `"/uploads/report/a.txt"` And
  `client.read_bytes("/uploads/report/a.txt").await.unwrap() == b"old"` (unchanged) And
  `client.exists("/uploads/report/sub/b.txt").await.unwrap() == false`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-011 — Collision naming: message names first colliding path in archive order

- Category: Edge. Scenario: SC-006. Requirements: FR-NEW-017.
- Preconditions: same as E2E-NEW-010 but the archive's entry order is `sub/b.txt` first, `a.txt`
  second, and `/uploads/report/sub/b.txt` is the pre-existing collision instead.
- Steps: as above. Then the error message names `/uploads/report/sub/b.txt`, not
  `/uploads/report/a.txt`, proving "first colliding path in archive entry order" rather than
  lexicographic or any other order.
- Cleanup: none. Priority: P1.

#### E2E-NEW-049 — No-clobber conflict against a pre-existing directory (not a file)

- Category: Failure. Scenario: SC-006. Requirements: FR-NEW-017.
- Preconditions: a `.zip` with one entry `d/` (a directory entry) and `d/x.txt = b"x"`;
  `/uploads/report/d` already exists on disk as a **file** (not a directory), from an unrelated
  prior write.
- Steps: Given the fixture, When `extract_archive(.., overwrite=false, ..)`, Then `Err` with
  `code == "ERR_NO_CLOBBER"` And `message` contains `/uploads/report/d` And nothing under
  `/uploads/report` changes.
- Cleanup: none. Priority: P1.

#### E2E-NEW-020 — Archive total declared size exceeds remaining quota

- Category: Failure. Scenario: SC-010. Requirements: FR-NEW-019.
- Preconditions: `ServerConfig.safety.write_quota_bytes` set to `100` in the fixture's config; a
  `.zip` at `/uploads/big.zip` with one entry declaring (and actually containing) `10_000` bytes.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_WRITE_QUOTA_EXCEEDED"` And `client.exists("/uploads/big/<entry>").await.unwrap() ==
  false`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-021 — After E2E-NEW-020's rejection, a small unrelated write within original headroom still succeeds

- Category: Edge. Scenario: SC-010. Requirements: FR-NEW-019.
- Preconditions: continuation of E2E-NEW-020, same session.
- Steps: Given the state right after E2E-NEW-020's failure, When a small, unrelated
  `fs.write(bytes=50)` call is made in the same session, Then it succeeds (`50 <= 100`
  headroom), proving the failed extraction charged nothing.
- Cleanup: none. Priority: P0.

#### E2E-NEW-026 — Crafted zip entry whose decoded bytes exceed its declared size

- Category: Failure. Requirements: FR-NEW-020.
- Preconditions: a hand-crafted zip (built via the `zip` crate's low-level writer, patched
  post-write) whose central-directory entry declares `uncompressed_size = 1` for an entry that
  actually decompresses to a larger buffer, OR a unit-level test that calls the internal
  decode-and-verify step with a mocked declared size of `1` against real decoded bytes of
  length `10`.
- Steps: Given the mismatch, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_INVALID_ARGUMENT"` And `message` names the entry and states it decompressed to more
  bytes than declared.
- Cleanup: none. Priority: P1.

#### E2E-NEW-042 — Tar entry whose header size is smaller than its actual content

- Category: Failure. Requirements: FR-NEW-020.
- Preconditions: a `.tar` entry whose header declares `size = 1` but whose actual tar data
  block (padded to the 512-byte boundary, as the tar format requires) is hand-crafted to make
  the reader yield more than 1 byte for that entry.
- Steps: Given the fixture, When `extract_archive(..)`, Then `Err` with `code ==
  "ERR_INVALID_ARGUMENT"` And `message` names the entry and states a size mismatch.
- Cleanup: none. Priority: P2.

#### E2E-NEW-043 — 7z entry size-mismatch

- Category: Failure. Requirements: FR-NEW-020.
- Preconditions: a `.7z` entry whose folder-header declares a smaller uncompressed size than
  its actual decompressed output (hand-crafted via `sevenz_rust2`'s low-level writer, patched
  post-write).
- Steps: as E2E-NEW-042.
- Cleanup: none. Priority: P2.

## Constraints

- Files Not to Touch: the write pass itself (`VolumeClient::write_bytes_atomic`/`makedirs`
  calls), response shape, audit, tracing (US-0009/US-0010).
- Dependencies Not to Add: none.
- Patterns to Avoid: do not charge the quota per-entry during a write pass — one charge for the
  whole archive total, before any write (`DEC-005`). Do not route this charge through
  `core::fs_ops::write_bytes` (`DEC-006`, enforced at the write-pass story, US-0009, but the
  quota-charge call site itself must call `SafetyManager::charge_write` directly, not through
  that wrapper).
- Scope Boundary: destination computation, conflict check, quota charge, size-mismatch guard
  only. No actual byte written to the volume in this story.

## Non Regression

`SafetyManager::charge_write`'s existing fail-closed behavior is consumed, not modified.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
