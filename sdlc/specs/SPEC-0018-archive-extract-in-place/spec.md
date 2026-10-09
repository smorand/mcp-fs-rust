# mcp-fs — Archive Extraction In Place — Specification Document

> Id: SPEC-0018
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0015_2026-10-08_22-31-32-archive-extract-in-place/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0015; new id SPEC-0018 (collision avoidance)
> Type: Evolution Specification
> From backlog: BL-0006
> Depends on: none
> Security: n/a

## 1. Summary

Adds `fs.extract_archive`, a tool that extracts an archive already stored in a project's
filesystem directly into that filesystem, with no download/re-upload round trip. It supports
`.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz`. Password-protected
zip and 7z archives are supported through a stateless retry: a missing or wrong password fails
with `ERR_PASSWORD_REQUIRED` and the caller simply re-issues the same call with `password` filled
in. Every entry is validated for path safety (zip-slip) and type (no symlink, hardlink, or device
entry survives), and the whole archive's declared size is charged against the write quota, all in
one pre-scan pass, before a single byte is written. `rar` is out of scope entirely, per an explicit
user decision. The two doors are `fs.extract_archive` (MCP) and
`POST /api/fs/{mount_id}/extract-archive` (REST), both calling one `tools::archive::extract_archive`
function, mirroring the `fs.export_zip` precedent.

## 2. Current State

### 2.1 Functional

The capability now exists, built as `crates/core/src/tools/archive.rs` (new module). The typed
function is `tools::archive::extract_archive`, called by both the MCP `#[tool]` method
`McpServer::fs_extract_archive` and the REST route `POST /api/fs/{mount_id}/extract-archive`, so
the two doors share one implementation, mirroring `tools::export::export_zip`'s precedent.

The structural template this spec followed was `fs.export_zip`: one typed `pub(crate) async fn`
called by both doors, direct use of `VolumeClient` primitives (`exists`, `walk`,
`write_bytes_atomic`, `makedirs`, `mkdir`, `touch_atime_mtime`) rather than routing every write
through `core::fs_ops::write_bytes` (which internally charges the write quota per call, which
would double-charge against this spec's single up-front quota charge). `core::fs_ops::
ensure_parents` was promoted from private to `pub(crate)` so this module reuses it.

`SafetyManager::charge_write` is synchronous, adds `num_bytes` to the in-memory per-session
`bytes_written` counter only if the new total stays under `write_quota_bytes`, and returns `Err`
without mutating the counter otherwise. This fail-closed, non-mutating-on-failure property backs
the quota-exceeded scenario.

### 2.2 SPEC-0003 (filesystem-engine)

`SPEC-0003` owns `fs.*` tool conventions this spec follows (`mount_id` required,
`state.authorize` then `normalize_path` then the engine call). Not modified; this spec added one
more `fs.*` tool under the same convention. `SPEC-0004` (REST data plane and OpenAPI) owns the
`/api/fs/{mount_id}/*` route shape this spec extended with one more route and body schema. Not
modified. `SPEC-0005` (multi-backend storage) owns `VolumeClient`/`SafetyManager`/quota; this
spec is a pure consumer of `charge_write` and `write_bytes_atomic`, adding no new backend
behavior. Not modified.

### 2.3 Test coverage

The feature ships with unit/integration tests in `crates/core/src/tools/archive.rs`, REST tests
in `crates/core/src/api/dataplane.rs` (the `extract-archive` block), OpenAPI schema tests in
`crates/core/src/api/openapi.rs`, and MCP contract/tool-count tests in `crates/core/src/mcp/
server.rs`. All 61 originally designed E2E tests (E2E-NEW-001..061) were implemented across the
15 stories tracked in `stories/_index.md`, all marked `done`.

## 3. Scope

### 3.1 In Scope

- One tool, `fs.extract_archive`, extracting an archive already stored in a project's volume
  directly into that volume.
- Formats: `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2`, `.tar.xz`/`.txz`.
- Password-protected `.zip` (ZipCrypto legacy and AES) and `.7z` (AES-256), with a stateless
  retry on `ERR_PASSWORD_REQUIRED`.
- Optional `destination` override; default destination derived by stripping the archive's
  recognized multi-part extension.
- `overwrite` flag (default `false`) governing whether an existing destination entry blocks the
  whole call.
- Per-entry path-safety (zip-slip) and type rejection (symlink/hardlink/device/special), all-or-
  nothing across the whole archive.
- Write-quota charging for the archive's total declared uncompressed size, in one pre-scan pass,
  before any write.
- The REST door `POST /api/fs/{mount_id}/extract-archive`, mirroring `export-zip`.
- The new `ERR_PASSWORD_REQUIRED` error code.

### 3.2 Out of Scope (Non-Goals)

- RAR, in any form. Not deferred, no backlog entry.
- Creating or compressing archives (already covered by the existing `fs.export_zip`).
- Auto-recursing into archives found nested inside the extracted output.
- Multi-volume archives (`.7z.001`, `.zip.001`, split RAR parts, or any other split-archive
  convention): handled like any other recognized-extension file whose bytes do not parse as that
  format: `ERR_INVALID_ARGUMENT`, or `ERR_NOT_SUPPORTED` if the extension itself is unrecognized.
- A stateful paused-operation mechanism: no `continue`/`abort` tool pair, no new relational
  table. Every call is a fresh, fully stateless dry-run-then-write; a password retry is just the
  same tool call issued again with `password` filled in.

## 4. Actors

- **Project member (human or LLM agent acting on a member's behalf)**: the only actor. Calls
  `fs.extract_archive` (or the REST equivalent) against an archive they can already read in a
  project they are a member of. There is no distinct admin or background-job actor for this
  capability.

## 5. Usage Scenarios

### SC-001: Extract a plain archive, default destination

- **Actor**: project member.
- **Preconditions**: an archive exists in the volume and is valid, containing nested files.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path=...)` with no `destination`, no
  `password`, default `overwrite=false`.
- **Postconditions**: every entry exists under the derived default destination with decoded
  bytes; response is `{"destination": ..., "files_written": N, "dirs_created": N,
  "bytes_written": N}`.
- **Exceptions**: none; happy path.

### SC-002: Extract with an explicit destination override

- **Flow**: caller supplies `destination`; every archive entry lands under it, the archive's own
  multi-part extension is never consulted for the destination.
- **Exceptions**: none.

### SC-003: Password-protected archive, correct password on the first call

- **Flow**: caller supplies the correct `password` on the only call; extraction succeeds exactly
  as SC-001, no password echoed in the response.
- **Exceptions**: none.

### SC-004: Password-protected archive, no password given, then retried

- **Flow**: call 1, no `password`, fails `ERR_PASSWORD_REQUIRED` ("password required to extract
  this archive"). Call 2, identical with `password` filled in, succeeds.
- **Postconditions**: nothing exists on disk after call 1; call 2 re-runs the full pre-scan from
  scratch.
- **Exceptions**: `ERR_PASSWORD_REQUIRED` on call 1, by design.

### SC-005: Retry with a wrong password

- **Flow**: call 1 with a wrong password fails `ERR_PASSWORD_REQUIRED` ("incorrect password for
  this archive"); call 2 with the correct password succeeds.
- **Exceptions**: `ERR_PASSWORD_REQUIRED`, same code as SC-004, distinguished only by message.

### SC-006: Destination collision, `overwrite=false` (default)

- **Flow**: an archive entry's destination already exists; caller does not set `overwrite`.
- **Postconditions**: the whole operation is rejected before any write; nothing changes on disk.
- **Exceptions**: `ERR_NO_CLOBBER`, naming the first colliding path in archive entry order.

### SC-007: Same collision, `overwrite=true`

- **Flow**: identical to SC-006 but with `overwrite=true`.
- **Postconditions**: colliding files are overwritten; a pre-existing colliding directory is
  reused, not recreated or rejected.
- **Exceptions**: none.

### SC-008: Zip-slip entry (path-escape attempt)

- **Flow**: an archive contains a benign entry and one whose path climbs outside the
  destination.
- **Postconditions**: the whole extraction is rejected before any write; the benign entry's
  content is never written anywhere.
- **Exceptions**: `ERR_PATH_OUT_OF_BOUNDS`, naming the escaping entry.

### SC-009: Symlink (or hardlink/device/special) entry

- **Flow**: an archive contains a benign entry and a symlink/hardlink/device entry.
- **Postconditions**: the whole extraction is rejected before any write, same as SC-008.
- **Exceptions**: `ERR_NOT_SUPPORTED`, naming the entry and its type.

### SC-010: Quota exceeded

- **Flow**: the archive's total declared uncompressed size exceeds the session's remaining write
  quota.
- **Postconditions**: nothing is written anywhere; the session's `bytes_written` counter is
  unchanged (verified by a follow-up small write still succeeding within the original headroom).
- **Exceptions**: `ERR_WRITE_QUOTA_EXCEEDED`.

### SC-011: Unsupported or corrupt archive

- **Flow**: caller invokes the tool against an unrecognized extension and, separately, against a
  file whose extension is recognized but whose bytes do not parse as that format.
- **Postconditions**: neither call writes anything.
- **Exceptions**: unrecognized extension → `ERR_NOT_SUPPORTED`, naming the extension. Corrupt
  bytes → `ERR_INVALID_ARGUMENT`, stating the archive is corrupt or not a valid file of that
  format.

## 6. Functional Requirements

| ID | Requirement | Evidence |
|---|---|---|
| FR-NEW-001 | The system SHALL expose a tool named `fs.extract_archive` taking `mount_id` (required), `path` (required), `destination` (optional), `overwrite` (optional, default `false`), `password` (optional), served identically by the MCP `#[tool]` method and the REST route, both calling one `pub(crate) async fn extract_archive` in `tools/archive.rs`. | crates/core/src/tools/archive.rs:118; crates/core/src/mcp/server.rs:2108; crates/core/src/api/dataplane.rs:657 |
| FR-NEW-002 | WHEN `fs.extract_archive` is called THE system SHALL call `state.authorize(mount_id, person)` (or the REST plane's `authorize_only`) before any other step. | crates/core/src/tools/archive.rs:95 |
| FR-NEW-003 | WHEN `path` is normalized THE system SHALL use `SafetyManager::normalize_path`, then look up the node; IF the node does not exist THEN fail `ERR_NOT_FOUND`. | crates/core/src/tools/archive.rs:128-132 |
| FR-NEW-004 | IF the node at `path` is a directory THEN fail `ERR_INVALID_ARGUMENT`. | crates/core/src/tools/archive.rs:133 |
| FR-NEW-005 | The system SHALL determine the archive format from the filename extension, matched case-insensitively, compound tar suffixes checked before plain `.tar`. | crates/core/src/tools/archive.rs:37-46,55-67 |
| FR-NEW-006 | IF the extension matches none of the suffixes THEN fail `ERR_NOT_SUPPORTED`, naming the exact extension. | crates/core/src/tools/archive.rs:61-66 |
| FR-NEW-007 | IF the archive's bytes do not parse as the selected format THEN fail `ERR_INVALID_ARGUMENT`. | crates/core/src/tools/archive.rs (decode path) |
| FR-NEW-008 | IF the format is any tar variant AND `password` is supplied THEN fail `ERR_INVALID_ARGUMENT`, checked before bytes are opened. | crates/core/src/tools/archive.rs (tar+password gate) |
| FR-NEW-009 | The system SHALL list every entry of a zip/sevenz archive (path, type, declared size, encrypted flag) without a correct password, except where header encryption makes the listing itself unreadable. | crates/core/src/tools/archive.rs (zip/7z listing); drift 2026-10-09_07-22-58 resolution |
| FR-NEW-010 | IF any entry reports encrypted AND no `password` was supplied THEN fail `ERR_PASSWORD_REQUIRED` ("password required to extract this archive") before decode/write. | crates/core/src/tools/archive.rs |
| FR-NEW-011 | IF a `password` was supplied AND decoding fails due to incorrect password THEN fail `ERR_PASSWORD_REQUIRED` ("incorrect password for this archive"). | crates/core/src/tools/archive.rs |
| FR-NEW-012 | The system SHALL decode every regular-file entry fully into memory during the pre-scan pass, matching `fs.export_zip`'s fully-buffered model. | crates/core/src/tools/archive.rs |
| FR-NEW-013 | IF an entry's declared type is symlink/hardlink/device/FIFO/socket THEN fail the WHOLE call with `ERR_NOT_SUPPORTED`, naming the entry's path and type. | crates/core/src/tools/archive.rs; drift 2026-10-09_13-14-44 (7z non-regular rejection, US-0014) |
| FR-NEW-014 | IF an entry's path is absolute or walks net-negative via `..` components THEN fail the WHOLE call with `ERR_PATH_OUT_OF_BOUNDS`, naming the escaping entry. | crates/core/src/tools/archive.rs; crates/core/src/tools/export.rs:122-130 (walking technique reused) |
| FR-NEW-015 | The system SHALL NOT write any byte nor create any directory until every entry has passed the checks in FR-NEW-013, FR-NEW-014, FR-NEW-017, FR-NEW-018. | crates/core/src/tools/archive.rs |
| FR-NEW-016 | The system SHALL compute the destination as the normalized caller-supplied `destination` when given, otherwise the normalized `path` with its matched extension stripped. | crates/core/src/tools/archive.rs |
| FR-NEW-017 | IF `overwrite` is `false` AND any entry's destination path already exists THEN fail the WHOLE call with `ERR_NO_CLOBBER`, naming the first colliding path in archive entry order. | crates/core/src/tools/archive.rs |
| FR-NEW-018 | IF `overwrite` is `true` THEN permit overwriting an existing file and reuse (not recreate) an existing directory at an entry's destination. | crates/core/src/tools/archive.rs |
| FR-NEW-019 | The system SHALL sum the declared uncompressed size of every regular-file entry and charge that total via `SafetyManager::charge_write(person, mount_id, total)` exactly once, before any write; IF it returns `Err` THEN fail the WHOLE call with `ERR_WRITE_QUOTA_EXCEEDED` with no mutation to `bytes_written`. | crates/core/src/tools/archive.rs; crates/core/src/safety.rs (charge_write fail-closed) |
| FR-NEW-020 | The system SHALL NOT write more decoded bytes than an entry's declared uncompressed size; IF decoding produces more THEN fail the WHOLE call with `ERR_INVALID_ARGUMENT`. | crates/core/src/tools/archive.rs |
| FR-NEW-021 | Once every check passes, THE system SHALL perform the write pass via `VolumeClient` primitives directly (never `core::fs_ops::write_bytes`): unconditional `makedirs(destination)`, then directory entries, then regular-file entries via `ensure_parents` + `write_bytes_atomic` + `touch_atime_mtime`. | crates/core/src/tools/archive.rs; crates/core/src/core/fs_ops.rs:1199 |
| FR-NEW-022 | The system SHALL promote `core::fs_ops::ensure_parents` from private to `pub(crate)`, with no behavior change. | crates/core/src/core/fs_ops.rs:1199 |
| FR-NEW-023 | The system SHALL count `files_written` as regular-file entries written, `dirs_created` as directories created that did not already exist, `bytes_written` as the sum of decoded bytes written. | crates/core/src/tools/archive.rs; drift 2026-10-09_13-14-44 (dirs_created exclusion, US-0015) |
| FR-NEW-024 | On success, THE system SHALL return `{"destination", "files_written", "dirs_created", "bytes_written"}`. | crates/core/src/tools/archive.rs |
| FR-NEW-025 | On success, THE system SHALL call `safety.record_audit(person, mount_id, "extract_archive", <path>, <detail>)` exactly once per call. | crates/core/src/tools/archive.rs |
| FR-NEW-026 | WHEN a call succeeds THE system SHALL emit one `tracing::info!` event carrying `mount_id`, `path`, `destination`, `files_written`, `bytes_written`. | crates/core/src/tools/archive.rs |
| FR-NEW-027 | The system SHALL NOT log the `password` field, in full or in part, at any tracing level, on success or failure. | crates/core/src/tools/archive.rs |
| FR-NEW-028 | The system SHALL add `ERR_PASSWORD_REQUIRED` to `errors.rs`'s `code` module, a `ToolError::password_required` constructor, an `http_status()` arm returning 428, and additions to the exhaustiveness tests, bringing the ERR_* count from 14 to 15. | crates/core/src/errors.rs:23,97-98,130,221,246; drift 2026-10-09_07-22-58 |
| FR-NEW-029 | The system SHALL add the REST route `POST /api/fs/{mount_id}/extract-archive` and its `ExtractArchiveBody`/`Op` OpenAPI entries, backed by a handler calling the same `tools::archive::extract_archive` function. | crates/core/src/api/dataplane.rs:89,152,657; crates/core/src/api/openapi.rs:1025-1029,1282-1283 |
| FR-NEW-030 | The system SHALL regenerate `TOOL_CONTRACT.txt` and `tool-contract-golden.json` to include `fs.extract_archive`. | TOOL_CONTRACT.txt (fs.extract_archive entry present) |

## 7. Non-Functional Requirements

- **Performance**: no new performance target beyond `fs.export_zip`'s existing fully-buffered
  model; one write-quota total charged up front is the only resource-consumption guard for the
  bulk operation.
- **Security**: zip-slip rejection (FR-NEW-014), symlink/hardlink/device rejection (FR-NEW-013),
  all-or-nothing enforcement (FR-NEW-015), size-mismatch (zip-bomb amplification) rejection
  (FR-NEW-020), password-never-logged (FR-NEW-027). Hardening built into a new capability, not a
  fix to an existing vulnerability.
- **Usability**: the stateless password retry (SC-004/SC-005) is the whole usability design — no
  second tool, no session state to track.
- **Reliability**: all-or-nothing semantics (FR-NEW-015) mean a failed call never leaves partial
  state on disk.
- **Observability**: one INFO tracing event on success with five named fields; absolute
  prohibition on logging the password at any level.
- **Deployment**: no change; `fs.extract_archive` is always available whenever the server is.
- **Scalability**: the existing per-session write-quota mechanism is the scaling guard; no new
  one added.

## 8. E2E Tests

61 tests (E2E-NEW-001..061) designed and implemented across the 15 stories in
`stories/_index.md` (all `done`). Distribution: Happy = 15, Failure = 29, Edge = 17. Coverage
includes: format matrix across all six supported extensions (E2E-NEW-002), password
happy/missing/wrong/retry flows (E2E-NEW-004..009,057,058), destination collision with and
without `overwrite` (E2E-NEW-010..013,049,050,053), zip-slip and symlink/hardlink/device
rejection with benign-entry-not-written proof (E2E-NEW-014..019,040,041,054,055), quota exceeded
and counter-unchanged proof (E2E-NEW-020,021), unsupported/corrupt/password-on-wrong-format
matrix (E2E-NEW-022..024,034..039,056), directory-path rejection (E2E-NEW-025,059,060),
size-mismatch rejection (E2E-NEW-026,042,043), `ensure_parents` reuse structural check
(E2E-NEW-027), password-never-logged proof on both outcomes (E2E-NEW-028,029,044),
`ERR_PASSWORD_REQUIRED` HTTP 428 mapping and error-type tests (E2E-NEW-030,045,046), REST route
happy/failure (E2E-NEW-031,032), and contract/OpenAPI/golden regeneration checks
(E2E-NEW-033,047,048,061).

## 9. Glossary

| Term | Definition | Context |
|---|---|---|
| Zip-slip | An archive entry whose path, when extracted naively, climbs outside the intended destination directory, letting an attacker overwrite arbitrary files. | FR-NEW-014, SC-008 |
| ZipCrypto | The legacy, weak password-encryption scheme built into the original zip format, as opposed to the stronger AES extension. | FR-NEW-009, FR-NEW-011 |
| Declared uncompressed size | The size of an entry's decompressed content as recorded in the archive's own metadata, read before decoding. | FR-NEW-009, FR-NEW-019, FR-NEW-020 |
| All-or-nothing | This spec's guarantee that a disqualifying condition anywhere in an archive voids the entire extraction, with no partial write. | FR-NEW-015, SC-006, SC-008, SC-009 |
| Pre-scan pass | The in-memory phase (format detection, entry decode, safety checks, conflict checks, quota charge) that completes entirely before the write pass begins. | §6, FR-NEW-012, FR-NEW-015, FR-NEW-021 |

## 10. Confidence notes

All 30 functional requirements verified present in code (`crates/core/src/tools/archive.rs`,
`errors.rs`, `mcp/server.rs`, `api/dataplane.rs`, `api/openapi.rs`) and in `TOOL_CONTRACT.txt`.
All 15 legacy stories are marked `done` in `stories/_index.md`, including 3 convergence stories
(US-0013, US-0014, US-0015) that closed gaps found after the initial 12-story plan. Two legacy
drift entries (DRIFT-001, DRIFT-002, both A-findings about unverified pinned-crate API surface)
resolved green during implementation, confirmed by the user per the drift register. Three
additional process/implementation drifts were recorded during execution (frozen error-code count
guard, story-ordering leaving contract tests red until the last story, `extract_archive` needing
a `person` parameter) — all accepted and documented, no spec amendment required. No open findings
remain; see design.md §9 Legacy mapping and the Findings for Backlog section below for drift not
already resolved by a story.
