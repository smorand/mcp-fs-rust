# Zip export via signed download URL — Specification Document

> Id: SPEC-0015
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0012; new id SPEC-0015 (collision avoidance)

## 1. Summary

Lets a caller (MCP tool or REST client) select an arbitrary set of files and/or directories from a
project and export them as a single zip archive, delivered through a short-lived, self-authorizing,
single-use signed download link instead of returning the bytes inline. Extends the existing
`download-zip` capability (which only zips one subtree and returns bytes synchronously in the same
request) rather than replacing it.

## 2. Current State

### 2.1 Functional

- A caller authorized for a project can select an arbitrary mixed list of files and/or directories
  (directories walked recursively, relative paths preserved) and invoke an export, either through the
  MCP tool `fs.export_zip` or the REST route `POST /api/fs/{mount_id}/export-zip`. Both perform
  identical logic: authorize, normalize every path, resolve each entry, read bytes, build an in-memory
  zip.
- An empty selection, an unauthorized caller, a path escaping the project root, or any nonexistent
  path all reject the whole request with a specific error code and create nothing (all-or-nothing).
- On success the system mints an unguessable token, stores the zip bytes under a non-content-addressed
  blob key, inserts one record carrying a 5-minute expiry, and returns a URL built from an optional
  configurable public base URL (or a relative path when unset).
- `GET /exports/{token}` requires no authentication or project-membership check at all: the token
  itself is the sole authorization. A successful download atomically deletes the record, reads and
  deletes the blob bytes, and streams back the zip exactly once. A second request on the same token,
  a request for a token that never existed, and a request for an expired-but-never-downloaded token
  are all indistinguishable: `404 Not Found` with a byte-identical body.
- An in-flight download whose expiry elapses mid-transmission still completes, because the response is
  fully buffered before transmission and the expiry check runs exactly once at request start.
- Two or more concurrent requests racing the same token resolve to exactly one winner via the atomic
  delete-and-fetch; every other racer sees the same `404` a nonexistent token would produce.
- A background sweep, folded into the server's existing periodic cleanup cycle, deletes every
  expired, never-downloaded record and its blob per project, isolating a failure on one row from every
  other row and project.
- There is no revocation surface: no tool, route, or CLI verb can invalidate an export before its
  natural expiry or consumption, and no file-count/size limit is enforced (matches the pre-existing
  `download-zip` behavior this feature extends).

### 2.2 Prior specs touched

SPEC-0003 (filesystem-engine), SPEC-0004 (rest-data-plane)

### 2.3 Test coverage

- 59 end-to-end tests (`E2E-NEW-001`..`059`) across creation, signed download, replay, expiry,
  background sweep, concurrency, and structural/security checks. Breakdown: 6 happy path, 14 failure,
  16 edge case, 9 side effect, 6 security, 2 data integrity, 3 structural, 1 state transition, 2
  concurrency.
- Exact test command: `cargo test --workspace`.
- All 5 implementation stories (US-0001..US-0005) are marked `done` in the legacy stories index.

## 3. Scope

### 3.1 In Scope

- `fs.export_zip` MCP tool and `POST /api/fs/{mount_id}/export-zip` REST route: accept a list of file
  and/or directory paths, build a zip in memory (directories walked recursively, relative paths
  preserved), store it, return a signed URL.
- `GET /exports/{token}` unauthenticated, single-use download route with a 5-minute expiry.
- A new `export_links` table and blob storage under key `export:{token}`.
- Background cleanup folded into the existing purge cycle.
- A new `server.public_base_url` config key for building absolute URLs.

### 3.2 Out of Scope (Non-Goals)

- Resumable or multipart downloads.
- Incremental/cached zip generation (always a fresh zip per export call).
- Explicit revocation of an issued link before expiry or consumption.
- Per-call configurable expiry or any cap on file count/total size: both fixed for v1.
- Modifying the existing `download-zip` route in any way.

## 4. Actors

- **Caller**: an MCP tool invoker or REST client authenticated and authorized for the project, who
  wants to hand off a bundle of files.
- **Recipient**: whoever opens the signed link — may have no relationship to the system at all.
  Carries no credential.
- **System (purge cycle)**: the existing background sweep / CLI verb, now also responsible for
  reaping expired, never-downloaded exports.

## 5. Usage Scenarios

### SC-001: Create an export
**Actor:** Caller
**Preconditions:** authenticated + authorized for `mount_id`; read access to every selected path
**Flow:**
1. Caller submits `mount_id` + a list of paths (files and/or directories).
2. System authorizes the caller for `mount_id`, then normalizes every path.
3. System resolves each entry — directories walked recursively — and reads every resulting file's
   bytes.
4. System builds an in-memory zip, each entry named by its full project-relative path.
5. System generates a token, stores the zip bytes, inserts one export record (`created_at`=now,
   `expires_at`=now+5min), and returns a URL containing the token.
**Postconditions:** an export record and its blob bytes exist, reachable only via the token, not yet
downloaded.
**Exceptions:**
- EXC-001a: a selected path doesn't exist → whole request rejected, nothing created.
- EXC-001b: caller lacks authorization/read access → whole request rejected, nothing created.
- EXC-001c: empty selection list → rejected, nothing created.

### SC-002: Recipient downloads via the signed URL
**Actor:** Recipient
**Preconditions:** export record exists, not yet downloaded, `now < expires_at` at lookup
**Flow:**
1. `GET /exports/{token}` with no authentication of any kind.
2. System atomically deletes the matching record, reads the blob, deletes the blob bytes, and streams
   the zip back as an `application/zip` attachment named `export.zip`.
**Postconditions:** export gone, file delivered exactly once.
**Cross-scenario notes:** races with a concurrent identical request and with the sweep are both
resolved by whichever operation wins the atomic row-delete.

### SC-003: Replay of an already-downloaded token
**Actor:** Recipient
**Preconditions:** the token was already consumed by a prior SC-002 run
**Flow:**
1. `GET /exports/{token}` on the same token again.
2. System finds no row → `404 Not Found`.
**Postconditions:** no state change.

### SC-004: Expired, never-downloaded token
**Actor:** Recipient
**Preconditions:** export record exists, `now >= expires_at`, never downloaded
**Flow:**
1. `GET /exports/{token}`.
2. System finds the row but it's expired → `404 Not Found`, deleting the row + blob as part of
   handling this request.
**Postconditions:** row and blob gone.

### SC-005: Background purge sweep
**Actor:** System
**Preconditions:** one or more export records with `expires_at` in the past, never downloaded
**Flow:**
1. Sweep runs as a per-project step inside the existing cycle.
2. For each expired row: delete the row, delete the corresponding blob, isolating any per-row failure.
**Postconditions:** no orphaned export artifact survives past its expiry; unexpired rows are
untouched.

**Cross-scenario note on SC-002 (in-flight download crossing the expiry boundary):** when a request's
expiry check has already passed at the moment the request began processing, the already-buffered zip
bytes still finish transmitting normally — a specific timing condition of SC-002, not a separate
scenario.

## 6. Functional Requirements

| ID | Requirement | Evidence |
|---|---|---|
| FR-NEW-001 | [EARS-E] MCP export creation: `fs.export_zip(mount_id, paths)` authorizes, normalizes paths, resolves entries (directories walked recursively) and reads bytes via `VolumeClient::walk`/`read_bytes`, builds an in-memory zip named by full project-relative path | `crates/core/src/tools/export.rs`, `crates/core/src/storage/volume.rs:130,92` |
| FR-NEW-002 | [EARS-E] REST export creation: `POST /api/fs/{mount_id}/export-zip` performs identical logic, no second implementation | `crates/core/src/api/dataplane.rs` |
| FR-NEW-003 | [EARS-O] Empty `paths` rejected with `ERR_INVALID_ARGUMENT`, creates nothing | `crates/core/src/errors.rs:20` |
| FR-NEW-004 | [EARS-O] Unauthorized caller rejected with `ERR_FORBIDDEN` before resolving any path | `crates/core/src/errors.rs:10` |
| FR-NEW-005 | [EARS-O] Path escaping the project root rejected with `ERR_PATH_OUT_OF_BOUNDS`, all-or-nothing | `crates/core/src/errors.rs:13` |
| FR-NEW-006 | [EARS-O] Nonexistent path entry rejected with `ERR_NOT_FOUND`, message names the offending path | `crates/core/src/errors.rs:16` |
| FR-NEW-007 | [EARS-E] Export record and blob creation: `token = uuid::Uuid::new_v4()`, blob stored at `export:{token}`, one `export_links` row inserted (`created_at`=now, `expires_at`=now+300s) | `crates/core/src/storage/meta.rs` |
| FR-NEW-008 | [EARS-U] URL construction: `{server.public_base_url}/exports/{token}` when configured, else relative `/exports/{token}` | `crates/core/src/exports.rs` |
| FR-NEW-009 | [EARS-E] Signed URL download happy path: atomic delete-and-fetch, deletes blob, returns `application/zip` attachment `export.zip` | `crates/core/src/exports.rs` |
| FR-NEW-009b | [EARS-U] Route `GET /exports/{token}` registered unconditionally in `app.rs`, same pattern as `deleted_projects_screen`/`trash_screen`, reachable even when `api.enabled=false` | `crates/core/src/app.rs:136-138`, `crates/core/src/exports.rs` |
| FR-NEW-010 | [EARS-O] Replay or unknown token rejected with `404 Not Found`, identical response across all three causes | `crates/core/src/exports.rs` |
| FR-NEW-011 | [EARS-O] Expired, never-downloaded token rejected with `404 Not Found`; row + blob deleted as part of this request | `crates/core/src/exports.rs` |
| FR-NEW-012 | [EARS-S] In-flight download survives expiry: expiry checked once at request start | `crates/core/src/exports.rs` |
| FR-NEW-013 | [EARS-E] Atomic single-use consumption: concurrent requests race, exactly one served | `crates/core/src/storage/meta.rs` |
| FR-NEW-014 | [EARS-E] Background sweep of expired exports: per-project, per-row isolated | `crates/core/src/purge.rs:174-203` |
| FR-NEW-015 | [EARS-U] Schema: `export_links` table declared alongside `trash_entries` in `meta.rs`, migrated identically across SQLite, PostgreSQL, SQL Server | `crates/core/src/storage/meta.rs` |
| FR-NEW-016 | [EARS-U] Config: `HttpConfig.public_base_url: String` (default `""`), expandable via `${VAR}` | `crates/core/src/config.rs:140-145` |
| FR-NEW-017 | [EARS-UB] No bearer/membership check on `GET /exports/{token}`: the token itself is the sole authorization | `crates/core/src/exports.rs` |
| FR-NEW-018 | [EARS-UB] No revocation surface: no tool, route, or CLI verb invalidates an export before natural expiry/consumption | n/a (structural absence) |

## 7. NFR

### 7.1 Performance
Matches the existing `download-zip` route: fully buffered in memory, no streaming, no new size or
file-count limit.

### 7.2 Security
FR-NEW-017 is a deliberate authorization omission on one route, declared rather than hidden.
Mitigations: UUIDv4 tokens carry 122 bits of random entropy; single-use (FR-NEW-013); 5-minute window
(FR-NEW-009/011); no enumeration endpoint exists. Traces and logs never record the full token value,
only a truncated/hashed form.

### 7.3 Usability
N/A — no UI surface; the signed URL is a plain HTTP link consumable by any HTTP client.

### 7.4 Reliability
Cleanup depends on the existing purge cycle running (background loop or CLI verb). If disabled,
expired exports accumulate — a pre-existing limitation shared by every other purge-dependent cleanup.

### 7.5 Observability
OpenTelemetry, same collector as the rest of the server.
- INFO on export creation: `mount_id`, file count.
- INFO on download outcome: served / expired / replayed (token truncated/hashed, never logged in full).
- DEBUG on sweep: count of rows/blobs deleted per cycle.
- Never traced: the full token value, file contents.

### 7.6 Deployment
No new infrastructure. Reuses the existing blob backend (local/S3) and relational store (SQLite/
PostgreSQL/SQL Server) already configured for the project.

### 7.7 Scalability
Same ceiling as the existing `download-zip` (unlimited, in-memory). Shared future scaling concern, not
a gap introduced by this entry.

## 8. E2E Tests

59 tests total (`E2E-NEW-001`..`059`), covering:
- **Creation (SC-001):** happy path (001, 002), failure (003–007, 041, 042, 044, 045), edge (008–010,
  013, 014, 043, 051, 056), side effect (011, 012), structural (039, 040, 050, 053).
- **Download (SC-002):** happy path (015), side effect (016, 017, 025, 049), security (018, 019, 052,
  054), failure (020, 021), data integrity (022), edge (038, 046, 047, 057, 059), concurrency (037,
  048), structural (058).
- **Replay (SC-003):** happy path setup (023), failure (024), side effect (025), data integrity (026),
  edge (055).
- **Expiry (SC-004):** happy path setup (027), failure (028), side effect (029), edge/boundary (030).
- **Sweep (SC-005):** happy path (031), side effect (032, 033), failure/isolation (034), state
  transition (035), edge/empty state (036).

Coverage: happy 6, failure 14, edge 16, side effect 9, security 6, data integrity 2, structural 3,
state transition 1, concurrency 2. Failure tests outnumber happy-path tests (14 > 6), satisfying test
sufficiency. Exact command: `cargo test --workspace`.

## 9. Glossary

| Term | Definition | Context |
|---|---|---|
| Export | A single zip archive built from a caller-selected set of files/directories, stored transiently, delivered once | Export creation, Signed delivery |
| Signed URL | A URL containing an unguessable token that is itself the sole authorization to download one specific export | Signed delivery |
| Single-use | The first successful `GET` on a token consumes it; any further `GET` on the same token returns `404` | Signed delivery |
| Grace period | Equal to the link's own expiry — no separate window beyond `expires_at` for a never-downloaded export | Lifecycle sweep |
| Sweep | The existing background `purge::run_cycle`, now also responsible for deleting expired, never-downloaded exports | Lifecycle sweep |
| Blob key namespacing | Storing non-filesystem artifacts in the same blob backend under a prefixed key (`export:{token}`), bypassing content-addressed refcounting, as `git:{sha}` already does for git objects | Export creation, Current State |

## 10. Confidence notes

Migrated from a legacy spec that passed its own three-round Implementability Gate (round 3 verdict
amended in place, not escalated) and whose stories index shows all 5 stories `done`. One open scaling
characteristic was captured during implementation (see design.md §9 Legacy mapping / FINDINGS FOR
BACKLOG): SQLite's per-volume-database layout forces an O(P) per-project probe to resolve a token with
no `mount_id` in the download URL. This is a documented cost characteristic, not a functional gap;
PostgreSQL/SQL Server are unaffected (one shared database).
