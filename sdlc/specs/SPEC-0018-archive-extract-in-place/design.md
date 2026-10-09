# mcp-fs — Archive Extraction In Place — Design Document

> Spec: SPEC-0018 (legacy SPEC-0015)
> Status: implemented
> Representative commit: 27bc282 ("SPEC-0015: fs.extract_archive, extract archives in place (#3)")

## 1. Components

- **`crates/core/src/tools/archive.rs`** (new, 2252 lines): the entire archive-extraction
  engine. `ArchiveFormat`/`TarCompression` enums, `SUFFIXES` lookup table, `detect_format`,
  `extract_archive` (the one typed function both doors call), per-format entry listing/decode
  (zip via `zip` crate `aes-crypto` feature, 7z via `sevenz-rust2`, tar family via `tar` +
  `flate2`/`bzip2`/`lzma-rs`), entry safety checks (symlink/hardlink/device rejection, zip-slip
  path walk), conflict scan, quota charge, write pass, audit/tracing. `#[cfg(test)]`-gated
  `register()` glue for the shared test harness.
- **`crates/core/src/errors.rs`**: new `code::PASSWORD_REQUIRED` constant,
  `ToolError::password_required` constructor, `http_status()` arm returning 428, additions to the
  `client_caused` exhaustiveness-test arrays. ERR_* count moved 14 → 15.
- **`crates/core/src/core/fs_ops.rs`**: `ensure_parents` (line 1199) promoted from private `fn` to
  `pub(crate) fn`, no behavior change, so `tools::archive` reuses it rather than duplicating
  parent-directory creation.
- **`crates/core/src/mcp/server.rs`**: `ExtractArchiveArgs` struct (line 1101), `#[tool]` method
  `fs_extract_archive` (line 2108, registered as `"fs.extract_archive"` at line 2105), calling
  `tools::archive::extract_archive`. Tool-count comments bumped (lines 3569, 3584: fs 38→39,
  total →106 at the time of this commit; final counts are higher after later specs).
- **`crates/core/src/api/dataplane.rs`**: `REST_ROUTES` entry `("POST", "extract-archive")` (line
  89), router registration `.route("/api/fs/{mount_id}/extract-archive", post(extract_archive))`
  (line 152), handler `extract_archive` (line 657) parsing the JSON body and calling the same
  typed function.
- **`crates/core/src/api/openapi.rs`**: `Op` entry (`sub: "extract-archive"`, line 1025) and
  `ExtractArchiveBody` schema entry (line 1282), mirroring `export-zip`/`ExportZipBody`.
- **`Cargo.toml` / `crates/core/Cargo.toml`**: new dependencies `tar` (0.4), `flate2` (1),
  `bzip2` (0.6, `libbz2-rs-sys` pure-Rust backend), `lzma-rs` (0.3), `sevenz-rust2` (0.20,
  `util`+`aes256` features); existing `zip` (2.x) gains the `aes-crypto` feature.
- **`TOOL_CONTRACT.txt` / `tool-contract-golden.json`**: regenerated to include
  `fs.extract_archive`.

## 2. Flows

### Extraction (happy path)

1. `authorize_only`/`state.authorize(mount_id, person)`.
2. `normalize_path(path)` → node lookup; `ERR_NOT_FOUND` if missing, `ERR_INVALID_ARGUMENT` if a
   directory.
3. `detect_format` from the filename extension (compound tar suffixes checked before plain
   `.tar`); `ERR_NOT_SUPPORTED` if unrecognized.
4. Tar-family + password supplied → `ERR_INVALID_ARGUMENT`, checked before any bytes are opened.
5. Read archive bytes; list every entry (zip/7z: without requiring password unless header
   encryption blocks even listing; tar: no encryption concept at all).
6. Decode every regular-file entry fully into memory (fully-buffered model, matches
   `export_zip`). Missing/wrong password at this point → `ERR_PASSWORD_REQUIRED` with one of two
   fixed messages.
7. Reject the WHOLE call if any entry is symlink/hardlink/device/special (`ERR_NOT_SUPPORTED`),
   or escapes the destination via an absolute or `..`-net-negative path (`ERR_PATH_OUT_OF_
   BOUNDS`), or (when `overwrite=false`) collides with an existing destination path
   (`ERR_NO_CLOBBER`, first colliding path in archive order), or decodes to more bytes than
   declared (`ERR_INVALID_ARGUMENT`).
8. Sum declared uncompressed size across all regular-file entries; `SafetyManager::charge_write`
   once; `ERR_WRITE_QUOTA_EXCEEDED` on failure, with no mutation to the session counter.
9. Write pass: unconditional `makedirs(destination)`, then every directory entry via `makedirs`,
   then every regular-file entry via `ensure_parents` + `write_bytes_atomic` +
   `touch_atime_mtime`.
10. `record_audit`; one `tracing::info!` with `mount_id`, `path`, `destination`,
    `files_written`, `bytes_written` (never `password`).
11. Return `{"destination", "files_written", "dirs_created", "bytes_written"}`.

### Password retry (stateless)

No state is kept server-side between calls. A failed call (missing or wrong password) writes
nothing and charges nothing; the caller simply re-issues the identical call with `password`
filled in, which re-runs the entire pre-scan from scratch.

## 3. Interfaces

- **MCP tool** `fs.extract_archive(mount_id, path, destination?, overwrite?, password?)` →
  `{"destination", "files_written", "dirs_created", "bytes_written"}`.
- **REST** `POST /api/fs/{mount_id}/extract-archive` body `{"path", "destination"?, "overwrite"?,
  "password"?}`, same response shape, same underlying function.
- **Error codes surfaced**: `ERR_NOT_FOUND`, `ERR_INVALID_ARGUMENT`, `ERR_NOT_SUPPORTED`,
  `ERR_PATH_OUT_OF_BOUNDS`, `ERR_NO_CLOBBER`, `ERR_WRITE_QUOTA_EXCEEDED`, `ERR_PASSWORD_REQUIRED`
  (new, HTTP 428).

## 4. Data and state

No new persisted schema, no new table, no new column. Reads existing `nodes`/`blob_refs` rows
through `VolumeClient` and writes new ones through the same primitives every other write-capable
`fs.*` tool uses. No server-side state for the password-retry mechanism: every call is fully
stateless.

## 5. Configuration

No new config section. `fs.extract_archive` is always available whenever the server is, exactly
like `fs.export_zip`. No new deployment requirement.

## 6. Observability

One `tracing::info!` event on success, fields `mount_id`, `path`, `destination`,
`files_written`, `bytes_written`. `password` is never logged, on success or failure, at any
level, verified by dedicated tests (E2E-NEW-028, E2E-NEW-029, E2E-NEW-044).

## 7. Decisions

- **DEC-001**: RAR entirely out of scope, not deferred, no backlog entry. User's exact words:
  "Remove completely rar, this was nice to have, not a requirement."
- **DEC-002**: no stateful paused-operation table; password handling fully stateless, a
  missing/wrong password is `ERR_PASSWORD_REQUIRED` and the caller retries the same call.
- **DEC-003**: `destination` defaults to the archive's path with its recognized multi-part
  extension stripped, for a predictable "extract here" model.
- **DEC-004**: conflict handling defaults to `overwrite=false`, rejecting the whole call on any
  collision; silent partial overwrite is the more dangerous default.
- **DEC-005**: quota is charged once, as the archive's total declared uncompressed size, in the
  pre-scan pass, not per-entry during the write pass — a zip-bomb-style archive must be rejected
  before any byte lands on disk.
- **DEC-006**: the write pass calls `VolumeClient::write_bytes_atomic`/`makedirs` directly, never
  `core::fs_ops::write_bytes`, because that function internally calls `charge_write` and would
  double-charge against the single up-front total (FR-NEW-019), mirroring `export_zip`'s own
  precedent of bypassing `core::fs_ops` for bulk operations with a different quota shape.
- **DEC-007**: pin `sevenz-rust2`, not `sevenz-rust` (unmaintained, repository deleted, carries an
  advisory).
- **DEC-008**: 7z symlink/hardlink/device detection is accepted as lower-fidelity than the
  tar/zip branches, because 7z has no universally-implemented symlink representation; an entry
  this branch cannot positively identify is written as an ordinary regular file, a safe default
  still covered by the independent zip-slip path-safety check.
- **DEC-009**: the spec's own test design and implementability-gate audit were performed by the
  same agent/context that wrote the requirements, because no sub-agent-spawning tool was
  available in that execution environment; mitigated by a mechanical, counted sufficiency check
  rather than an eyeballed one.
- **DEC-010**: the quota charge uses each entry's **declared** uncompressed size, with a separate
  guard (FR-NEW-020) rejecting any entry whose actual decoded length exceeds its declared size,
  closing a size-mismatch zip-bomb-amplification vector the charge alone would not close.
- **DEC-011**: for this spec's 11 narrow, single-mechanism usage scenarios, the generic "5 tests
  per scenario" guidance is satisfied by presence of a happy/failure/edge test wherever a
  scenario's own Exceptions field names a real failure mode, rather than by inventing
  scenario-specific failure modes that do not exist for scenarios whose Exceptions field reads
  "none" (SC-001, SC-002, SC-003, SC-007).
- **DEC-012** (process drift, resolved): the frozen error-code-count guard test
  (`e2e_new_184_no_new_error_constant_is_introduced` in `tools/git.rs`, an earlier spec's guard)
  was updated from 14 to 15 with a comment naming this spec's FR-NEW-028; its intent (no
  unrelated new error code) is preserved, only the frozen baseline moved.
- **DEC-013** (process drift, resolved): story ordering registered the MCP tool in US-0004 but
  deferred contract regeneration to the last story (US-0012), leaving 5 contract-count tests
  pinned red for the intervening stories (`fs_tool_count_is_thirty_six`,
  `total_tool_count_is_one_hundred_and_three`, `tool_router_lists_exactly_the_100_contract_names`,
  the ninety-five-tool-schema structural test, and the agent's own 100-tool-listing test). Real
  frozen values at that point were 36 fs / 100 contract / 103 total, not the spec's placeholder
  149→150 language; US-0012 used the real values. Recommendation for `/plan-spec`: order contract
  regeneration immediately after the story that registers a new tool.
- **DEC-014** (process drift, resolved): `extract_archive`'s signature needed a `person: &str`
  parameter, omitted from FR-NEW-001's pinned signature, because FR-NEW-019's quota charge and
  FR-NEW-025's audit call both require the caller's identity. `McpServer::fs_extract_archive`
  passes the authenticated person through in one added line. Also noted: E2E-NEW-042/043 (tar/7z
  decoded-size-exceeds-declared) run as unit tests against the shared `ensure_decoded_size_
  matches` guard rather than as full archive-level scenarios, because `tar` 0.4.46 bounds reads
  to the header size at the library level, making the archive-level scenario unforgeable.

## 8. Requirement to code map

| FR | Code |
|---|---|
| FR-NEW-001 | crates/core/src/tools/archive.rs:118; mcp/server.rs:2108; api/dataplane.rs:657 |
| FR-NEW-002..004 | crates/core/src/tools/archive.rs:95,128-133 |
| FR-NEW-005,006 | crates/core/src/tools/archive.rs:37-67 |
| FR-NEW-007,008 | crates/core/src/tools/archive.rs (decode / tar+password gate) |
| FR-NEW-009..012 | crates/core/src/tools/archive.rs (entry listing/decode) |
| FR-NEW-013,014,015 | crates/core/src/tools/archive.rs (safety checks); export.rs:122-130 (walk technique) |
| FR-NEW-016..020 | crates/core/src/tools/archive.rs (destination/conflict/quota/size-mismatch) |
| FR-NEW-021,022 | crates/core/src/tools/archive.rs (write pass); core/fs_ops.rs:1199 |
| FR-NEW-023,024 | crates/core/src/tools/archive.rs (counts/response) |
| FR-NEW-025,026,027 | crates/core/src/tools/archive.rs (audit/tracing) |
| FR-NEW-028 | crates/core/src/errors.rs:23,97-98,130,221,246 |
| FR-NEW-029 | crates/core/src/api/dataplane.rs:89,152,657; api/openapi.rs:1025-1029,1282-1283 |
| FR-NEW-030 | TOOL_CONTRACT.txt (fs.extract_archive present) |

## 9. Legacy mapping

Source: specs/archived/SPEC-0015_2026-10-08_22-31-32-archive-extract-in-place/spec.md (pre-move, renumbered to SPEC-0018); plus its drift/ directory (3 files)

| Old id | New id | Note |
|---|---|---|
| SPEC-0015 | SPEC-0018 | Renumbered on migration, collision avoidance (SPEC-0018 was the next free global number). |
| US-0001 | (resolved) | [drift] DRIFT-001 tracker; resolved in practice by US-0006 (zip metadata readable without password confirmed, zip 2.4.2 `ZipArchive::by_index_raw`, read.rs:1097). |
| US-0002 | (resolved) | [drift] DRIFT-002 tracker; resolved in practice by US-0007 (`ZipFile::is_symlink()` found at zip-2.4.2/src/read.rs:1746-1749, used instead of manual bit-mask). |
| US-0003 | §6 FR-NEW-028 | New error code `ERR_PASSWORD_REQUIRED`. Done. |
| US-0004 | §6 FR-NEW-001..004 | Tool skeleton: registration, authorize, path resolve, directory rejection. Done. |
| US-0005 | §6 FR-NEW-005,006,008 | Format detection, unsupported extension, tar+password gate. Done. |
| US-0006 | §6 FR-NEW-007,009,010,011 | Cargo deps; zip/7z listing; corrupt detection; password handling. Done. |
| US-0007 | §6 FR-NEW-013,014,015 | Symlink/hardlink/device rejection; zip-slip rejection; void-on-disqualify ordering. Done. |
| US-0008 | §6 FR-NEW-016..020 | Destination computation; no-clobber/overwrite; quota charge; size check. Done. |
| US-0009 | §6 FR-NEW-021..024 | Write pass; result counts; response shape. Done. |
| US-0010 | §6 FR-NEW-025,026,027 | Audit entry; tracing; password-never-logged. Done. |
| US-0011 | §6 FR-NEW-029 | REST route + OpenAPI schema. Done. |
| US-0012 | §6 FR-NEW-030 | `TOOL_CONTRACT.txt` + golden regeneration. Done. |
| US-0013 | §6 FR-NEW-010,011 | [converge] 7z missing/wrong password tested; closed previously-unwritten E2E-NEW-004,005,007,009,058. Done. |
| US-0014 | §6 FR-NEW-013,014 | [converge] 7z non-regular entry rejection (within DEC-008's accepted scope); 7z zip-slip error code. Done. |
| US-0015 | §6 FR-NEW-023 | [converge] `dirs_created` excludes pre-existing directories. Done. |
| drift 2026-10-09_07-22-58 | DEC-012 | Frozen error-code count guard test updated 14→15 for FR-NEW-028; resolved, documented. |
| drift 2026-10-09_07-43-24 | DEC-013 | Story order leaves 5 contract tests red from US-0004 to US-0012; real frozen counts (36/100/103) used, not spec placeholder language; resolved, documented; recommendation carried to Findings for Backlog. |
| drift 2026-10-09_13-14-44 | DEC-014 | `extract_archive` needs `person` parameter, omitted from FR-NEW-001's pinned signature; resolved, documented. |
