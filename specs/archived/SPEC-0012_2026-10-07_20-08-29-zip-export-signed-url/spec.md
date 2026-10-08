# Zip export via signed download URL — Specification Document

> Generated on: 2026-10-07
> Id: SPEC-0012
> Nature: FEAT
> Depth: L
> Depth evidence: existing project, new persisted schema (`export_links` table), 4+ modules touched (tools/, api/dataplane.rs, storage/meta.rs, purge.rs, config.rs), 18 anticipated requirements. All three L triggers fire.
> Status: Draft
> Type: Evolution Specification
> From backlog: BL-0005
> Split: not split
> Depends on: none
> Security: internal finding
> CVSS: not scored
> Affected: n/a
> Fixed in: n/a

## 1. Executive Summary

Lets a caller (MCP tool or REST client) select an arbitrary set of files and/or directories from a
project and export them as a single zip archive, delivered through a short-lived, self-authorizing,
single-use signed download link instead of returning the bytes inline. Extends the existing
`download-zip` capability (which only zips one subtree and returns bytes synchronously in the same
request) rather than replacing it.

## 2. Current State

### 2.1 How it works today

- `GET /api/fs/{mount_id}/download-zip?path=<subtree>` (`crates/core/src/api/dataplane.rs:595-627`,
  routed at `dataplane.rs:114-115`) takes **one** directory path, walks it via `r.client.walk`, reads
  every file under it, builds a zip archive fully in memory with the `zip` crate, and returns it
  inline as `application/zip` in the same HTTP response. It requires the normal bearer-token
  authentication and project-membership check (`dataplane.rs`'s `guarded()`, `dataplane.rs:182-202`).
  There is no equivalent MCP tool; `TOOL_CONTRACT.txt` has no zip/export/signed-URL entry.
- Content-addressed blob storage: a sha256 of the bytes is the key; local layout is
  `{dir}/{bucket}/{sha[..2]}/{sha}` (`crates/core/src/storage/blob/local.rs:1-24`), S3/MinIO uses one
  bucket per volume with the sha as the object key directly (`crates/core/src/storage/blob/s3.rs:1-37`).
  Reference counting lives in the metadata store (`crates/core/src/storage/meta.rs:8-10`, refcount
  column `meta.rs:143`); a blob is deleted the instant its last referencing row is removed
  (`meta.rs:856-915`). There is **no TTL/expiry concept anywhere** — GC is purely refcount-triggered
  and immediate, never time-based.
- Non-content-addressed storage already has a precedent: git objects live in the same blob backend
  under key `git:{sha}` (`crates/core/src/git/odb.rs:42-44`), bypassing `meta.rs` refcounting
  entirely; their own lifecycle is tracked by `git/db.rs`'s own small tables.
- Small side tables living alongside the main `nodes`/`blob_refs` tables already exist:
  `trash_entries` is declared directly in `crates/core/src/storage/meta.rs:149` (and indexed at
  `meta.rs:172-173`), with accessor functions in the same file (e.g. `list_trash_entries_page` at
  `meta.rs:273`, insert at `meta.rs:287-318`, delete at `meta.rs:380`,`meta.rs:640`).
- Unguessable tokens are minted elsewhere with `uuid::Uuid::new_v4()` — the CSRF tokens in
  `crates/core/src/token_screen.rs`, `crates/core/src/deleted_projects_screen.rs:66`, and
  `crates/core/src/trash_screen.rs:69` all follow this same pattern.
- There is **no signed-URL or presigned-resource-access mechanism anywhere**. The only signing in the
  codebase is RS256 JWT bearer tokens for caller identity (`crates/core/src/identity.rs:1-10`,
  `crates/core/src/keys.rs:1-21`), which authenticate *who is calling*, not a scoped/expiring grant to
  one resource.
- There is **no config key for this server's own externally-reachable base URL**. The only URL-shaped
  config fields are outbound targets: `GitConfig::gitlab_instance_url` (`config.rs:456`,`:491`) and the
  doc-service `api.url` (`config.rs:796`). `HttpConfig` (`config.rs:138-148`) has `host`/`port`/
  `mcp_path` but nothing for building an absolute link back to this server.
- The existing background purge cycle (`crates/core/src/purge.rs`) has one entry point,
  `run_cycle` (`purge.rs:174-178`), called by both the background loop (`app.rs:210`) and the CLI
  `purge` verb (`cli.rs:214` calls `sweep_grace_period`; `run_cycle` is the per-cycle driver). It
  iterates `admin.list_all_projects()` and runs `sweep_project_files` then `sweep_project` per project,
  isolating failures per project (logged + skipped, cycle continues) (`purge.rs:174-203`).
- The `attachment()` helper (`dataplane.rs:635`, exact signature
  `fn attachment(data: Vec<u8>, mime: &str, name: &str) -> Response`) builds the `Content-Disposition`
  header (ASCII + RFC 5987 `filename*`). It is module-private today and has no dependency on
  `AppState` or auth, so promoting it to `pub(crate)` is safe.

### 2.2 Existing specifications governing this area

- `specs/SPEC-0004_2026-09-18_18-30-00-rest-data-plane/spec.md` owns the REST bytes plane
  (`upload`/`download`/`download-zip`), specifically FR-313 (file download), FR-314 (subtree zip
  download), FR-315 (downloads bypass the engine/no session read recorded), and DEC-304 (deliberate
  asymmetry: writes go through the engine, downloads don't). This specification does not modify that
  one; `download-zip` is unchanged and continues to exist exactly as specified there. This entry adds a
  second, separate capability alongside it.
- `specs/SPEC-0005_2026-09-18_18-55-00-multi-backend-storage/spec.md` owns the relational seam and S3
  blob backend this entry's new table and blob writes build on top of, unmodified.
- `specs/SPEC-0012_...project-storage-quota` (SPEC-0010) is unrelated: export creation is a read path,
  not subject to write quota.

### 2.4 Class Sweep

This entry's own design deliberately removes the normal bearer/membership check on one route
(`GET /exports/{token}`, FR-NEW-017), so the unconditional security-misclassification check
applies even though no vulnerability is being fixed: the sweep here verifies this is the *only* route
missing that check, and that the omission is scoped and mitigated, not a second accidental hole.

**Class:** an HTTP route returning project data without verifying caller identity or project
membership (CWE-306, Missing Authentication for Critical Function / CWE-862, Missing Authorization).

**Searches run:**
1. `grep -n "\.route(" crates/core/src/app.rs crates/core/src/api/dataplane.rs` — every registered
   route, checked against whether its handler is reached through `guarded`/`guarded_json`
   (`dataplane.rs:182-217`, which calls `person_of` then `state.authorize`). Covers every REST route
   idiom in the tree (there is exactly one router file for this plane).
2. `grep -rn "session\|cookie\|login" crates/core/src/token_screen.rs` plus the equivalent read of
   `deleted_projects_screen.rs` and `trash_screen.rs` — covers the second idiom (cookie-session screens
   outside the REST plane, a different auth mechanism, not a different absence of one).

Idiom not searched: the MCP tool surface's own auth. Not re-swept here because every `fs.*`/`git.*`
tool's `state.authorize(mount_id, person)`-first convention is an existing, already-enforced project
rule (AGENTS.md), and this entry's new tool `fs.export_zip` follows it per FR-NEW-004, verified by
E2E-NEW-004/041/044.

| Occurrence | Verdict | Disposition |
|---|---|---|
| `GET /health` (`app.rs:134`, handler documented unauthenticated at `app.rs:260`) | safe: liveness probe returns no project data, by design, pre-existing | out of lot, pre-existing, unchanged |
| every other `/api/fs/*` route (`dataplane.rs:105-147`) | safe: each reaches its handler through `guarded`/`guarded_json` (`dataplane.rs:182-217`), which calls `person_of` then `state.authorize` before any data is touched | out of lot, pre-existing, unchanged |
| `token_screen.rs`/`deleted_projects_screen.rs`/`trash_screen.rs` | safe: gated by the `mcpfs_token` session cookie, verified by the same JWT check as bearer auth (`token_screen.rs:7-25`,`:270`) | out of lot, pre-existing, unchanged |
| `GET /exports/{token}` (this entry, FR-NEW-009/017) | vulnerable-shaped but **in this lot, deliberate**: no identity/membership check at all | in this lot — the entire point of the feature; mitigated by token entropy, single-use (FR-NEW-013), 5-minute expiry (FR-NEW-007/011), and the absence of any enumeration surface (token space is 2^122, no listing endpoint exists) |

The originating occurrence (`GET /exports/{token}`) is fixed by design, not deferred: there is
nothing to backlog, because the lack of auth here is the requirement, not a defect. Security tests
E2E-NEW-018, E2E-NEW-019, E2E-NEW-052, E2E-NEW-054 exercise the boundary this sweep depends on: the
absence of auth is confined to exactly this one route and does not leak into any other handler.

### 2.3 Existing test coverage

- `crates/core/src/storage/blob/local.rs:115` — 9 tests, local blob backend.
- `crates/core/src/storage/blob/s3.rs:170` — 2 tests (1 pure unit, 1 MinIO-gated integration test that
  self-skips when `:9000` is down).
- `crates/core/src/api/dataplane.rs:1329` — 66 `#[test]`/`#[tokio::test]` occurrences, real axum router
  tests driven via `tower::ServiceExt::oneshot`.
- `crates/core/src/purge.rs` — covered by its own test module (not separately counted here; the new
  sweep step is additive to this file).
- Nothing today covers zip-of-arbitrary-selection, signed URLs, or time-based expiry: this area is
  entirely new and unverifiable until this specification's own `E2E-NEW-*` suite exists.
- Exact test command: `cargo test --workspace` (`test.sh:5`, `Makefile:16`).

## 3. Scope

### 3.1 In Scope

- `fs.export_zip` MCP tool and `POST /api/fs/{mount_id}/export-zip` REST route: accept a list of file
  and/or directory paths, build a zip in memory (directories walked recursively, relative paths
  preserved), store it, return a signed URL.
- `GET /exports/{token}` unauthenticated, single-use download route with a 5-minute expiry.
- A new `export_links` table and blob storage under key `export:{token}`.
- Background cleanup folded into the existing `purge::run_cycle`.
- A new `server.public_base_url` config key for building absolute URLs.

### 3.2 Out of Scope (Non-Goals)

- Resumable or multipart downloads.
- Incremental/cached zip generation (always a fresh zip per export call).
- Explicit revocation of an issued link before expiry or consumption (DEC-005, DEC-018).
- Per-call configurable expiry or any cap on file count/total size (DEC-002, DEC-010): both are fixed
  for v1.
- Modifying the existing `download-zip` route in any way.

## 4. User Personas & Actors

- **Caller**: an MCP tool invoker or REST client authenticated and authorized for the project, who
  wants to hand off a bundle of files.
- **Recipient**: whoever opens the signed link — may have no relationship to the system at all (e.g.
  pasted into a browser by a human, or fetched by an unrelated process). Carries no credential.
- **System (purge cycle)**: the existing background sweep / CLI verb, now also responsible for
  reaping expired, never-downloaded exports.

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| Export creation | Reading selected files via the existing filesystem engine and packaging them | `export_links` row (pending state) |
| Signed delivery | Unauthenticated, single-use resource access | `export_links` row (consumed/expired transition), blob at `export:{token}` |
| Lifecycle sweep | Time-based cleanup of artifacts no authentication layer ever gates | `purge::run_cycle` |

## 5. Usage Scenarios

### SC-001: Create an export
**Actor:** Caller
**Preconditions:** authenticated + authorized for `mount_id`; read access to every selected path
**Flow:**
1. Caller submits `mount_id` + a list of paths (files and/or directories).
2. System authorizes the caller for `mount_id`, then normalizes every path.
3. System resolves each entry — a directory is walked recursively via `VolumeClient::walk`
   (`storage/volume.rs:130`) — and reads every resulting file's bytes via `VolumeClient::read_bytes`
   (`storage/volume.rs:92`), the same two calls the existing `download_zip` route already uses
   (`dataplane.rs:607,614`).
4. System builds an in-memory zip, each entry named by its full project-relative path.
5. System generates a token, stores the zip bytes at blob key `export:{token}`, inserts one
   `export_links` row (`created_at`=now, `expires_at`=now+5min), and returns a URL containing the
   token.
**Postconditions:** an export record and its blob bytes exist, reachable only via the token, not yet
downloaded.
**Exceptions:**
- EXC-001a: a selected path doesn't exist → whole request rejected, nothing created (SC-001's own
  failure twin, see FR-NEW-006/007).
- EXC-001b: caller lacks authorization/read access → whole request rejected, nothing created
  (FR-NEW-004/005).
- EXC-001c: empty selection list → rejected, nothing created (FR-NEW-003).
**Cross-scenario notes:** none — creation is independent of delivery and sweep.

### SC-002: Recipient downloads via the signed URL
**Actor:** Recipient
**Preconditions:** export record exists, not yet downloaded, `now < expires_at` at lookup
**Flow:**
1. `GET /exports/{token}` with no authentication of any kind.
2. System atomically deletes the matching `export_links` row, reads the blob at `export:{token}`,
   deletes the blob bytes, and streams the zip back as an `application/zip` attachment named
   `export.zip`.
**Postconditions:** export gone, file delivered exactly once.
**Exceptions:** none — SC-003 and SC-004 are the dedicated failure scenarios.
**Cross-scenario notes:** races with a concurrent identical request (FR-NEW-013) and with the sweep
(SC-005) are both resolved by whichever operation wins the atomic row-delete.

### SC-003: Replay of an already-downloaded token
**Actor:** Recipient
**Preconditions:** the token was already consumed by a prior SC-002 run
**Flow:**
1. `GET /exports/{token}` on the same token again.
2. System finds no row → `404 Not Found`.
**Postconditions:** no state change.
**Exceptions:** none.

### SC-004: Expired, never-downloaded token
**Actor:** Recipient
**Preconditions:** export record exists, `now >= expires_at`, never downloaded
**Flow:**
1. `GET /exports/{token}`.
2. System finds the row but it's expired → `404 Not Found`, and deletes the row + blob as part of
   handling this request (opportunistic cleanup, independent of the sweep).
**Postconditions:** row and blob gone.
**Exceptions:** none.

### SC-005: Background purge sweep
**Actor:** System (`purge::run_cycle`)
**Preconditions:** one or more `export_links` rows with `expires_at` in the past, never downloaded
**Flow:**
1. Sweep runs as a third per-project step inside the existing `run_cycle` loop.
2. For each expired row: delete the row, delete the corresponding blob at `export:{token}`, isolating
   any per-row failure the same way the existing two steps isolate per-project failures.
**Postconditions:** no orphaned export artifact survives past its expiry; unexpired rows are
untouched.
**Exceptions:**
- a blob already missing/corrupt for one row does not abort the sweep for other rows (FR-NEW-014).

**Cross-scenario note on SC-002 (in-flight download crossing the expiry boundary):** when a request's
expiry check (`now < expires_at`) has already passed at the moment the request began processing, the
already-buffered zip bytes still finish transmitting normally (FR-NEW-012) — this is not a separate
scenario, it is SC-002 under a specific timing condition, pinned by E2E-NEW-038/046/047 rather than
given its own `SC-` id, since it shares every precondition and postcondition with SC-002.

## 6. Functional Requirements

### New Requirements

#### FR-NEW-001 [EARS-E]: MCP export creation
> WHEN a caller invokes tool `fs.export_zip(mount_id, paths: [String])` with a non-empty `paths` THE
> system SHALL authorize the caller for `mount_id`, normalize every path, resolve each entry
> (directories walked recursively) via `VolumeClient::walk` (`storage/volume.rs:130`), read every
> resulting file's bytes via `VolumeClient::read_bytes` (`storage/volume.rs:92`) — the same two calls
> the existing `download_zip` route already uses (`dataplane.rs:607,614`) — and build an in-memory zip
> with each entry named by its full project-relative path.

- **Inputs:** `mount_id: String`, `paths: [String]`
- **Outputs:** an in-memory `zip::ZipWriter` buffer (consumed by FR-NEW-007)
- **Business Rules:** a directory entry contributes every file found by recursively walking it; a
  file entry contributes itself; every resulting zip entry name is the full project-relative path
  (leading slash stripped), guaranteeing no collisions
- **Exact names:** tool `fs.export_zip`; params `mount_id: String`, `paths: [String]`
- **Priority:** Must-have

#### FR-NEW-002 [EARS-E]: REST export creation
> WHEN a caller issues `POST /api/fs/{mount_id}/export-zip` body `{"paths": [...]}` THE system SHALL
> perform identical logic to FR-NEW-001 via the same `VolumeClient::walk`/`read_bytes` calls (no
> second implementation of path resolution, reading, or zipping).

- **Inputs:** path param `mount_id`, JSON body `{"paths": ["..."]}`
- **Outputs:** JSON `{"url": "..."}` on success
- **Exact names:** route `POST /api/fs/{mount_id}/export-zip`, body field `paths`, response field `url`
- **Priority:** Must-have
- **Rationale:** keeps MCP and REST in the parity the project already enforces for every other
  `fs.*`/REST pair.

#### FR-NEW-003 [EARS-O]: Empty selection rejected
> IF `paths` is empty THEN THE system SHALL reject the request with error code `ERR_INVALID_ARGUMENT`
> and SHALL NOT create an export record or blob.

- **Business Rules:** applies identically to both FR-NEW-001 and FR-NEW-002
- **Exact names:** error code `ERR_INVALID_ARGUMENT` (`crates/core/src/errors.rs:20`)
- **Priority:** Must-have

#### FR-NEW-004 [EARS-O]: Unauthorized caller rejected
> IF the caller is not authorized for `mount_id` THEN THE system SHALL reject with error code
> `ERR_FORBIDDEN` before resolving any path.

- **Business Rules:** authorization check happens strictly before any path is touched, per the
  project convention (`state.authorize(mount_id, person)` first, per AGENTS.md's handler ordering)
- **Exact names:** error code `ERR_FORBIDDEN` (`errors.rs:10`)
- **Priority:** Must-have

#### FR-NEW-005 [EARS-O]: Path escaping the project root rejected
> IF any path in `paths` normalizes outside the project root THEN THE system SHALL reject the entire
> request with error code `ERR_PATH_OUT_OF_BOUNDS`, creating nothing.

- **Business Rules:** all-or-nothing — one valid path in the list does not save a request containing
  one invalid path
- **Exact names:** error code `ERR_PATH_OUT_OF_BOUNDS` (`errors.rs:13`)
- **Priority:** Must-have

#### FR-NEW-006 [EARS-O]: Nonexistent path rejected
> IF any entry in `paths` does not resolve to an existing file or directory THEN THE system SHALL
> reject the entire request with error code `ERR_NOT_FOUND`, naming the offending path, creating
> nothing.

- **Business Rules:** all-or-nothing, same as FR-NEW-005
- **Exact names:** error code `ERR_NOT_FOUND` (`errors.rs:16`); error message MUST contain the
  offending path's literal string
- **Priority:** Must-have

#### FR-NEW-007 [EARS-E]: Export record and blob creation
> WHEN the zip archive has been built THE system SHALL generate `token = uuid::Uuid::new_v4()`, store
> the zip bytes at blob key `export:{token}` in the volume's configured blob backend, insert one
> `export_links` row (`token`, `volume_id`, `created_at`=now RFC3339, `expires_at`=now+300s RFC3339),
> and return `{"url": ...}`.

- **Business Rules:** token generation follows the codebase's existing convention (`uuid::Uuid::new_v4()`,
  as used in `token_screen.rs`/`deleted_projects_screen.rs:66`/`trash_screen.rs:69`); blob key follows
  the `git:{sha}` precedent (`odb.rs:42-44`) with prefix `export:` instead of `git:`
- **Exact names:** blob key format `export:{token}`; table `export_links`; columns `token`,
  `volume_id`, `created_at`, `expires_at`
- **Priority:** Must-have

#### FR-NEW-008 [EARS-U]: URL construction
> THE system SHALL build `url` as `{server.public_base_url}/exports/{token}` when
> `public_base_url` is non-empty, and as the relative path `/exports/{token}` when it is empty
> (the default).

- **Exact names:** config key `server.public_base_url`
- **Priority:** Must-have

#### FR-NEW-009 [EARS-E]: Signed URL download, happy path
> WHEN `GET /exports/{token}` arrives for a `token` whose `export_links` row exists and whose
> `expires_at` has not passed at the moment of lookup THE system SHALL, in one atomic operation, delete
> the `export_links` row, read the blob bytes at key `export:{token}`, delete those bytes, and return
> them as an `application/zip` attachment named `export.zip` via the `attachment()` helper (promoted
> to `pub(crate)`).

- **Exact names:** route `GET /exports/{token}`; response header `Content-Type: application/zip`;
  `Content-Disposition: attachment; filename="export.zip"`
- **Priority:** Must-have

#### FR-NEW-009b [EARS-U]: Route registration independent of the REST data plane
> THE system SHALL register `GET /exports/{token}` unconditionally in `crates/core/src/app.rs`,
> merged the same way as `crate::deleted_projects_screen::router`/`crate::trash_screen::router`
> (`app.rs:136-138`) rather than inside `crate::api::router()` (`api/dataplane.rs`), so the route
> stays reachable even when the REST data plane is disabled (`api.enabled: false`, `app.rs:141-144`),
> since `fs.export_zip` (FR-NEW-001) has no such dependency and a link it mints must not become
> permanently unreachable.

- **Exact names:** new module `crates/core/src/exports.rs`, exposing `pub fn router(state: Arc<AppState>) -> Router`, merged in `app.rs` alongside `deleted_projects_screen`/`trash_screen` (`app.rs:136-138`)
- **Priority:** Must-have
- **Rationale:** Phase 6 round 3 finding F4 — the handler logic (atomic delete-and-serve) is unchanged,
  only its registration point moves out of the conditionally-merged `api::router()`.

#### FR-NEW-010 [EARS-O]: Replay or unknown token rejected
> IF a `GET /exports/{token}` request arrives for a `token` with no existing `export_links` row
> (never existed, already consumed, or already swept) THEN THE system SHALL respond `404 Not Found`,
> with no response distinguishing which of those three cases applied.

- **Business Rules:** the three causes (never existed / consumed / swept) MUST produce byte-identical
  response bodies and the same status code, so a recipient cannot learn anything about a token that
  isn't theirs
- **Priority:** Must-have

#### FR-NEW-011 [EARS-O]: Expired, never-downloaded token rejected
> IF the row exists but `expires_at` has already passed at lookup THEN THE system SHALL respond
> `404 Not Found` (same response shape as FR-NEW-010) and SHALL delete the row and its blob bytes as
> part of handling this request, independent of the background sweep.

- **Priority:** Must-have

#### FR-NEW-012 [EARS-S]: In-flight download survives expiry
> WHILE a request's expiry check (FR-NEW-009) has already passed at the moment the request began
> processing THE system SHALL still complete transmitting the already-buffered zip bytes to the
> client, since the response is fully buffered before transmission and `expires_at` is evaluated
> exactly once at request start.

- **Rationale:** this is the reason the window can stay short (5 minutes): a slow transfer is never
  interrupted, only a slow *arrival* is rejected
- **Priority:** Must-have

#### FR-NEW-013 [EARS-E]: Atomic single-use consumption
> WHEN two concurrent `GET /exports/{token}` requests race for the same `token` THE system SHALL
> serve the zip to exactly one of them (whichever wins the atomic row-delete) and respond
> `404 Not Found` to the other, per FR-NEW-010.

- **Priority:** Must-have

#### FR-NEW-014 [EARS-E]: Background sweep of expired exports
> WHEN `purge::run_cycle`'s per-project loop runs THE system SHALL, for each project, delete every
> `export_links` row whose `expires_at` has passed as of the sweep's start time, and delete the
> corresponding blob bytes at `export:{token}` for each deleted row, isolating failures per row the
> same way `sweep_project_files`/`sweep_project` isolate failures per project.

- **Business Rules:** a failure deleting one row's blob (e.g. already missing) does not abort the
  sweep for other rows or other projects
- **Exact names:** extends `crates/core/src/purge.rs`'s `run_cycle` (`purge.rs:174-203`) with a third
  step; `CycleSummary` (`purge.rs:163-167`) gains a new counter field, e.g. `exports_swept: usize`
- **Priority:** Must-have

#### FR-NEW-015 [EARS-U]: Schema
> THE system SHALL declare the `export_links` table in the same schema module as `trash_entries`
> (`crates/core/src/storage/meta.rs`), with columns `token` (`TextKey(36)`, primary key), `volume_id`
> (`TextKey(VOLUME_ID_LEN)`), `created_at` (`Text`), `expires_at` (`Text`), migrated identically across
> SQLite, PostgreSQL and SQL Server via the store's existing migration path.

- **Exact names:** table `export_links`; column types `ColumnType::TextKey(36)`,
  `ColumnType::TextKey(VOLUME_ID_LEN)`, `ColumnType::Text` (x2)
- **Priority:** Must-have

#### FR-NEW-016 [EARS-U]: Config
> THE system SHALL add `public_base_url: String` (default `""`) to the existing `HttpConfig` struct
> (`crates/core/src/config.rs:140-145`), reachable as `ServerConfig.server.public_base_url`
> (`ServerConfig.server: HttpConfig` at `config.rs:856`) — not a new top-level `http:` YAML section,
> and not a rename of the existing `server` field. Expandable via
> `${VAR}` like every other config value.

- **Exact names:** `HttpConfig::public_base_url: String`, default `""`; effective config path
  `server.public_base_url`
- **Priority:** Must-have

#### FR-NEW-017 [EARS-UB]: No bearer/membership check on download
> THE system SHALL NOT require an `Authorization`/forwarded-identity header or project-membership
> check on `GET /exports/{token}`; the token itself is the sole authorization.

- **Rationale:** DEC-004 — the self-authorizing design is deliberate, mitigated by UUIDv4 entropy,
  single-use, and the 5-minute window; see §7.2.
- **Priority:** Must-have

#### FR-NEW-018 [EARS-UB]: No revocation surface
> THE system SHALL NOT expose any tool, route, or CLI verb to invalidate an `export_links` row before
> its natural expiry or consumption.

- **Rationale:** DEC-005 — confirms a deliberate omission, not a gap
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
Matches the existing `download-zip` route: fully buffered in memory, no streaming, no new size or
file-count limit (DEC-010, same as today's unlimited `download-zip`).

### 7.2 Security
Front block reads `Security: internal finding` (not `n/a`): FR-NEW-017 is a deliberate authorization
omission on one route, and the unconditional misclassification rule treats that as security-relevant
regardless of intent, so it is declared rather than hidden behind `n/a`. The full class sweep is in
§2.4. Mitigations for the one in-lot occurrence: `uuid::Uuid::new_v4()` tokens carry 122 bits of random
entropy; single-use (FR-NEW-013); 5-minute window (FR-NEW-009/011); no enumeration endpoint exists.
Traces and logs MUST NOT record the full token value — only a truncated/hashed form — per the existing
"never log a token or a key" rule (AGENTS.md).

### 7.3 Usability
N/A — no UI surface; the signed URL is a plain HTTP link consumable by any HTTP client.

### 7.4 Reliability
Cleanup depends on the existing purge cycle running (background loop or CLI verb). If disabled,
expired exports accumulate — the same existing limitation shared by every other purge-dependent
cleanup in this codebase, not a new risk introduced here.

### 7.5 Observability
OpenTelemetry, same collector as the rest of the server (JSONL file by default).
- INFO on export creation: `mount_id`, file count.
- INFO on download outcome: served / expired / replayed (token truncated/hashed, never logged in full).
- DEBUG on sweep: count of rows/blobs deleted per cycle.
- Never traced: the full token value, file contents.

### 7.6 Deployment
No new infrastructure. Reuses the existing blob backend (local/S3) and relational store (SQLite/
PostgreSQL/SQL Server) already configured for the project. Scale-to-zero posture unaffected — no new
always-on component.

### 7.7 Scalability
Same ceiling as the existing `download-zip` (unlimited, in-memory). Flagged as a shared future
scaling concern, not a gap introduced by this entry.

## 8. Data Model

New table `export_links`:

| Column | Type | Notes |
|---|---|---|
| `token` | `ColumnType::TextKey(36)` | primary key, UUIDv4 string |
| `volume_id` | `ColumnType::TextKey(VOLUME_ID_LEN)` | scopes which project/blob-backend instance owns this export |
| `created_at` | `ColumnType::Text` | RFC3339 |
| `expires_at` | `ColumnType::Text` | RFC3339, `created_at` + 300s |

Blob store: one object per export at key `export:{token}`, same backend (local dir / S3 bucket) the
owning `volume_id` already uses — not part of the content-addressed `nodes`/`blob_refs` graph, exactly
like git objects under `git:{sha}`.

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact | Description |
|---|---|---|
| `crates/core/src/tools/export.rs` | New | `fs.export_zip` tool, mirrors `tools/trash.rs`'s structure |
| `crates/core/src/api/dataplane.rs` | Modified | new `POST /api/fs/{mount_id}/export-zip`; `attachment()` promoted to `pub(crate)` |
| `crates/core/src/exports.rs` (new) | New | `GET /exports/{token}` handler and its unconditional `router()`, merged in `app.rs` (FR-NEW-009b) |
| `crates/core/src/app.rs` | Modified | merges `exports::router` alongside `deleted_projects_screen`/`trash_screen` (`app.rs:136-138`) |
| `crates/core/src/storage/meta.rs` | Modified | `export_links` table + accessor fns (insert, atomic delete-and-fetch, sweep-expired) |
| `crates/core/src/storage/volume.rs` | None | `VolumeClient.blob` is already a public `Arc<dyn BlobBackend>` field (`volume.rs:15`), reachable directly as `client.blob.{put,get,delete}("export:{token}", ...)` exactly as `git/odb.rs` already does (`odb.rs:42-44`) with no `volume.rs` wrapper; no change needed here |
| `crates/core/src/purge.rs` | Modified | third per-project sweep step in `run_cycle`; `CycleSummary` gains `exports_swept` |
| `crates/core/src/config.rs` | Modified | `HttpConfig.public_base_url: String` (default `""`) |
| `TOOL_CONTRACT.txt` / `tool-contract-golden.json` | Modified | new `fs.export_zip` entry (regenerate via `MCPFS_REWRITE_TOOL_CONTRACT=1`) |

### 9.2 Affected Requirements

| Spec | Requirement ID | Impact | Description |
|---|---|---|---|
| SPEC-0004 | FR-313, FR-314, FR-315, DEC-304 | None | `download-zip` unchanged; this entry is additive, referenced not modified |

### 9.3 Affected Tests

| Test File | Test | Action | Description |
|---|---|---|---|
| `crates/core/src/api/dataplane.rs` | (new tests in existing `mod tests`) | New | E2E-NEW-001..030, 037-043 |
| `crates/core/src/purge.rs` | (new tests in existing `mod tests`) | New | E2E-NEW-031..036 |
| `crates/core/src/storage/meta.rs` | conformance suite | New | E2E-NEW-040 (structural, three-dialect run) |

No existing test is modified or removed.

### 9.4 Affected Documentation

| Document | Section | Action | Description |
|---|---|---|---|
| `TOOL_CONTRACT.txt` | tool list | Update | add `fs.export_zip` |
| `.agent_docs/api.md` | REST routes | Update | add the two new routes |
| `.agent_docs/tools.md` | tool reference | Update | add `fs.export_zip` |
| `AGENTS.md` | index | Update | none required beyond existing `.agent_docs` pointers |

### 9.5 Dependencies & Risks
No new crate dependencies (`uuid` and `zip` are already in use). No breaking change, no migration
needed for existing data, no rollback concern beyond the usual schema-migration rollback already
exercised by `trash_entries`'s precedent.

## 10. Documentation Requirements
`TOOL_CONTRACT.txt` (regenerate golden), `.agent_docs/api.md` and `.agent_docs/tools.md` gain the new
route pair and tool. No README change required (feature is internal-API-level, not a user-facing CLI
change).

## 11. Traceability Matrix

| Scenario | Functional Req | E2E (Happy) | E2E (Failure) | E2E (Edge) |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-018 | E2E-NEW-001, E2E-NEW-002 | E2E-NEW-003, E2E-NEW-004, E2E-NEW-005, E2E-NEW-006, E2E-NEW-007, E2E-NEW-041, E2E-NEW-042, E2E-NEW-044, E2E-NEW-045 | E2E-NEW-008, E2E-NEW-009, E2E-NEW-010, E2E-NEW-013, E2E-NEW-014, E2E-NEW-043, E2E-NEW-051, E2E-NEW-056 |
| SC-002 | FR-NEW-009, FR-NEW-009b, FR-NEW-012, FR-NEW-013, FR-NEW-017, FR-NEW-018 | E2E-NEW-015 | E2E-NEW-020, E2E-NEW-021 | E2E-NEW-038, E2E-NEW-046, E2E-NEW-047, E2E-NEW-057, E2E-NEW-059 |
| SC-003 | FR-NEW-010 | E2E-NEW-023 | E2E-NEW-024 | E2E-NEW-055 |
| SC-004 | FR-NEW-011 | E2E-NEW-027 | E2E-NEW-028 | E2E-NEW-030 |
| SC-005 | FR-NEW-014 | E2E-NEW-031 | E2E-NEW-034 | E2E-NEW-036 |

Only tests whose §12.1 Category is literally Happy/Failure/Edge are placed in those three matrix
columns, so side-effect, security, structural, concurrency, data-integrity and state-transition tests
(e.g. E2E-NEW-016/017/022/025/026/029/032/033/035/037/048/049/052/053/054) are not duplicated into this
matrix; they remain fully listed and traced in §12.1's full table instead. FR-NEW-015, FR-NEW-016,
FR-NEW-018 are structural/configuration/negative requirements, cross-cutting rather than tied to one
scenario, assigned to SC-001 (where the export record and its config-derived URL are created) for
traceability: covered by E2E-NEW-040/050, E2E-NEW-013/014/051/056, and E2E-NEW-039/053/054 (the last
two also tagged SC-002 where the check concerns the download route itself) respectively.

## 12. End-to-End Test Suite

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-001 | New | Happy path | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-008 | P0 |
| E2E-NEW-002 | New | Happy path | SC-001 | FR-NEW-002 | P0 |
| E2E-NEW-003 | New | Failure | SC-001 | FR-NEW-003 | P0 |
| E2E-NEW-004 | New | Failure | SC-001 | FR-NEW-004 | P0 |
| E2E-NEW-005 | New | Failure | SC-001 | FR-NEW-005 | P0 |
| E2E-NEW-006 | New | Failure | SC-001 | FR-NEW-006 | P0 |
| E2E-NEW-007 | New | Failure | SC-001 | FR-NEW-006 | P1 |
| E2E-NEW-008 | New | Edge (data variation) | SC-001 | FR-NEW-001 | P1 |
| E2E-NEW-009 | New | Edge (path traversal) | SC-001 | FR-NEW-005 | P0 |
| E2E-NEW-010 | New | Edge (boundary) | SC-001 | FR-NEW-001 | P1 |
| E2E-NEW-011 | New | Side effect | SC-001 | FR-NEW-007, FR-NEW-015 | P0 |
| E2E-NEW-012 | New | Side effect | SC-001 | FR-NEW-007 | P0 |
| E2E-NEW-013 | New | Edge (config boundary) | SC-001 | FR-NEW-008, FR-NEW-016 | P1 |
| E2E-NEW-014 | New | Edge (config boundary) | SC-001 | FR-NEW-008 | P1 |
| E2E-NEW-015 | New | Happy path | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-016 | New | Side effect | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-017 | New | Side effect | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-018 | New | Security | SC-002 | FR-NEW-017 | P0 |
| E2E-NEW-019 | New | Security | SC-002 | FR-NEW-017 | P1 |
| E2E-NEW-020 | New | Failure | SC-002 | FR-NEW-010 | P0 |
| E2E-NEW-021 | New | Failure | SC-002 | FR-NEW-010 | P1 |
| E2E-NEW-022 | New | Data integrity | SC-002 | FR-NEW-001, FR-NEW-009 | P0 |
| E2E-NEW-023 | New | Happy path (setup) | SC-003 | FR-NEW-010 | P0 |
| E2E-NEW-024 | New | Failure | SC-003 | FR-NEW-010 | P0 |
| E2E-NEW-025 | New | Side effect | SC-003 | FR-NEW-009, FR-NEW-010 | P1 |
| E2E-NEW-026 | New | Data integrity | SC-003, SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-027 | New | Happy path | SC-004 | FR-NEW-011 | P0 |
| E2E-NEW-028 | New | Failure | SC-004 | FR-NEW-011 | P0 |
| E2E-NEW-029 | New | Side effect | SC-004 | FR-NEW-011 | P0 |
| E2E-NEW-030 | New | Edge (boundary) | SC-004 | FR-NEW-011 | P1 |
| E2E-NEW-031 | New | Happy path | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-032 | New | Side effect | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-033 | New | Side effect | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-034 | New | Failure/isolation | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-035 | New | State transition | SC-005 | FR-NEW-014 | P1 |
| E2E-NEW-036 | New | Edge (empty state) | SC-005 | FR-NEW-014 | P1 |
| E2E-NEW-037 | New | Concurrency | SC-002 | FR-NEW-013 | P0 |
| E2E-NEW-038 | New | Edge (concurrency/timing) | SC-002 | FR-NEW-012 | P0 |
| E2E-NEW-039 | New | Security | SC-001 | FR-NEW-018 | P1 |
| E2E-NEW-040 | New | Structural | SC-001 | FR-NEW-015 | P1 |
| E2E-NEW-041 | New | Failure | SC-001 | FR-NEW-004 | P1 |
| E2E-NEW-042 | New | Failure | SC-001 | FR-NEW-003 | P2 |
| E2E-NEW-043 | New | Edge (empty string) | SC-001 | FR-NEW-003, FR-NEW-006 | P2 |
| E2E-NEW-044 | New | Failure | SC-001 | FR-NEW-002, FR-NEW-004 | P1 |
| E2E-NEW-045 | New | Failure | SC-001 | FR-NEW-002, FR-NEW-005 | P1 |
| E2E-NEW-046 | New | Edge | SC-002 | FR-NEW-012 | P1 |
| E2E-NEW-047 | New | Edge | SC-002 | FR-NEW-012 | P1 |
| E2E-NEW-048 | New | Concurrency | SC-002 | FR-NEW-013 | P1 |
| E2E-NEW-049 | New | Side effect | SC-002 | FR-NEW-013 | P1 |
| E2E-NEW-050 | New | Structural | SC-001 | FR-NEW-015 | P2 |
| E2E-NEW-051 | New | Edge | SC-001 | FR-NEW-016 | P2 |
| E2E-NEW-052 | New | Security | SC-002 | FR-NEW-017 | P1 |
| E2E-NEW-053 | New | Security | SC-001 | FR-NEW-018 | P2 |
| E2E-NEW-054 | New | Security | SC-002 | FR-NEW-018 | P2 |
| E2E-NEW-055 | New | Edge | SC-003 | FR-NEW-010 | P2 |
| E2E-NEW-056 | New | Edge | SC-001 | FR-NEW-016 | P2 |
| E2E-NEW-057 | New | Edge | SC-002 | FR-NEW-009b | P0 |
| E2E-NEW-058 | New | Structural | SC-002 | FR-NEW-009b | P1 |
| E2E-NEW-059 | New | Edge | SC-002 | FR-NEW-009b | P0 |

**Coverage statistics (recounted directly from the Category column above):** happy 6 (001, 002, 015,
023, 027, 031), failure 14 (003–007, 020, 021, 024, 028, 034, 041, 042, 044, 045), edge 16 (008–010,
013, 014, 030, 036, 038, 043, 046, 047, 051, 055, 056, 057, 059), side effects 9 (011, 012, 016, 017,
025, 029, 032, 033, 049), security 6 (018, 019, 039, 052, 053, 054), data integrity 2 (022, 026),
structural 3 (040, 050, 058), state transitions 1 (035), concurrency 2 (037, 048). Total 59, matching
the 59 rows in the updated table (E2E-NEW-057/058/059 added for FR-NEW-009b). **Happy = 6, failure =
14**: failure tests outnumber happy tests (14 > 6, i.e. roughly 2.3 failure tests per happy test),
satisfying the sufficiency rule that failure tests must outnumber happy-path tests rather than the
reverse. Performance: 0, justified N/A in §4.3 (matches the existing unlimited `download-zip`
behavior, no new limit introduced).

### 12.2 New Test Specifications

#### E2E-NEW-001: fs.export_zip happy path, mixed file+directory selection, MCP tool
- **Category:** Core Journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-008
- **Driver:** direct MCP tool call (test harness)
- **Preconditions:** volume `proj` owned by `owner@test.com`; files `/src/main.rs` (`b"fn main() {}"`),
  `/docs/readme.md` (`b"# Hello"`), `/docs/notes/todo.txt` (`b"buy milk"`) written via `core::fs_ops`.
- **Steps:**
  - Given the fixture above
  - When `owner@test.com` calls `fs.export_zip` with `{"mount_id": "proj", "paths": ["/src/main.rs", "/docs"]}`
  - Then the call returns `Ok` with JSON `{"url": "<matches ^/exports/[0-9a-f-]{36}$>"}`
- **Cleanup:** none (export remains unconsumed, out of scope for this test)
- **Priority:** Critical

#### E2E-NEW-002: POST /api/fs/{mount_id}/export-zip happy path, identical logic to tool
- **Category:** Core Journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Driver:** HTTP client (`tower::ServiceExt::oneshot`)
- **Preconditions:** same fixture as E2E-NEW-001
- **Steps:**
  - Given the fixture
  - When `POST /api/fs/proj/export-zip` body `{"paths": ["/src/main.rs", "/docs"]}`, header
    `Authorization: Bearer <owner_token>`
  - Then status is `200 OK`, body is `{"url": "<same regex>"}`
- **Priority:** Critical

#### E2E-NEW-003: empty paths rejected with ERR_INVALID_ARGUMENT, creates nothing
- **Category:** Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Driver:** direct MCP tool call
- **Preconditions:** zero `export_links` rows, zero `export:*` blob keys
- **Steps:**
  - Given the fixture
  - When `fs.export_zip` is called with `{"mount_id": "proj", "paths": []}`
  - Then the error code is exactly `ERR_INVALID_ARGUMENT`
  - And `SELECT COUNT(*) FROM export_links WHERE volume_id='proj'` is `0`
  - And no blob key matching `export:*` exists
- **Priority:** Critical

#### E2E-NEW-004: unauthorized caller rejected with ERR_FORBIDDEN before path resolution
- **Category:** Security / Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Driver:** direct MCP tool call
- **Preconditions:** `stranger@test.com` not a member of `proj`
- **Steps:**
  - Given `/src/main.rs` exists
  - When `stranger@test.com` calls `fs.export_zip` with `{"mount_id": "proj", "paths": ["/src/main.rs"]}`
  - Then error code is exactly `ERR_FORBIDDEN`
  - And no `export_links` row or blob was created (proves authorization precedes path resolution)
- **Priority:** Critical

#### E2E-NEW-005: path escaping project root rejected with ERR_PATH_OUT_OF_BOUNDS, all-or-nothing
- **Category:** Security / Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `/src/main.rs` exists
  - When `fs.export_zip` is called with `{"mount_id": "proj", "paths": ["/src/main.rs", "/../../../etc/passwd"]}`
  - Then error code is exactly `ERR_PATH_OUT_OF_BOUNDS`
  - And no row/blob created, even though `/src/main.rs` alone was valid
- **Priority:** Critical

#### E2E-NEW-006: one missing file entry rejects the whole request with ERR_NOT_FOUND
- **Category:** Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `/src/main.rs` exists, `/src/missing.rs` does not
  - When `fs.export_zip` is called with `{"mount_id": "proj", "paths": ["/src/main.rs", "/src/missing.rs"]}`
  - Then error code is exactly `ERR_NOT_FOUND`, message contains `/src/missing.rs`
  - And no row/blob created
- **Priority:** Critical

#### E2E-NEW-007: missing directory entry also rejects with ERR_NOT_FOUND
- **Category:** Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `/docs` exists, `/nonexistent-dir` does not
  - When `fs.export_zip` is called with `{"mount_id": "proj", "paths": ["/docs", "/nonexistent-dir"]}`
  - Then error code is exactly `ERR_NOT_FOUND`, message contains `/nonexistent-dir`
  - And no row/blob created
- **Priority:** High

#### E2E-NEW-008: data variation, unicode and special characters
- **Category:** Edge Case
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Driver:** direct MCP tool call + HTTP download
- **Steps:**
  - Given a file at `/docs/日本語 résumé (final)!.txt` with bytes `b"content"`
  - When `fs.export_zip({"paths": ["/docs/日本語 résumé (final)!.txt"]})` then the returned URL is
    downloaded
  - Then the zip has exactly one entry named exactly `docs/日本語 résumé (final)!.txt` with bytes
    exactly `b"content"`
- **Priority:** High

#### E2E-NEW-009: edge, path traversal attempts rejected
- **Category:** Security / Edge Case
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Driver:** direct MCP tool call
- **Steps:**
  - Given the fixture
  - When `fs.export_zip` is called with `{"paths": ["/src/../../outside.txt"]}` and separately with
    `{"paths": ["..\\..\\windows\\system32"]}`
  - Then both return error code exactly `ERR_PATH_OUT_OF_BOUNDS`
  - And neither creates a row/blob
- **Priority:** Critical

#### E2E-NEW-010: boundary, single file, empty directory, mixed selection
- **Category:** Edge Case
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Driver:** direct MCP tool call + HTTP download
- **Steps:**
  - Given `/a.txt` (`b"A"`), empty directory `/empty_dir`, `/b/c.txt` (`b"C"`)
  - When three calls: (a) `{"paths": ["/a.txt"]}`, (b) `{"paths": ["/empty_dir"]}`,
    (c) `{"paths": ["/a.txt", "/b"]}`
  - Then (a) zip has 1 entry `a.txt`=`b"A"`; (b) zip has 0 entries (succeeds, not `ERR_NOT_FOUND`);
    (c) zip has 2 entries `a.txt`=`b"A"`, `b/c.txt`=`b"C"`
- **Priority:** High

#### E2E-NEW-011: side effect, export_links row created with correct columns
- **Category:** Side Effect
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007, FR-NEW-015
- **Driver:** direct MCP tool call + DB query
- **Steps:**
  - Given `t0 = Utc::now()` read just before the call
  - When `fs.export_zip({"paths": ["/src/main.rs"]})` returns `url`
  - Then a row exists with `token` = UUID from `url`, `volume_id = "proj"`, `created_at` in
    `[t0, t0+2s]`, `expires_at = created_at + 300s` (±1s)
- **Priority:** Critical

#### E2E-NEW-012: side effect, zip bytes stored at blob key export:{token}
- **Category:** Side Effect
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007
- **Driver:** direct MCP tool call + blob store read
- **Steps:**
  - Given `/src/main.rs` = `b"fn main() {}"`
  - When `fs.export_zip({"paths": ["/src/main.rs"]})` returns token `T`
  - Then `blob_store.read("export:T")` returns bytes that, parsed as a zip, contain exactly one entry
    `src/main.rs` = `b"fn main() {}"`
- **Priority:** Critical

#### E2E-NEW-013: URL uses public_base_url when configured
- **Category:** Edge Case (config boundary)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008, FR-NEW-016
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `ServerConfig.server.public_base_url = "https://files.example.com"`
  - When `fs.export_zip({"paths": ["/src/main.rs"]})`
  - Then `url` is exactly `"https://files.example.com/exports/{token}"`
- **Priority:** High

#### E2E-NEW-014: URL is relative when public_base_url is empty
- **Category:** Edge Case (config boundary)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `ServerConfig.server.public_base_url = ""` (default)
  - When `fs.export_zip({"paths": ["/src/main.rs"]})`
  - Then `url` is exactly `"/exports/{token}"` and does not start with `http`
- **Priority:** High

#### E2E-NEW-015: SC-002 happy path, correct content-type and filename
- **Category:** Core Journey
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** HTTP client
- **Preconditions:** export created for `["/src/main.rs", "/docs/readme.md"]`, token `T`
- **Steps:**
  - Given export `T` exists, unconsumed, unexpired
  - When `GET /exports/T` with no `Authorization` header
  - Then status is `200 OK`, `content-type: application/zip`,
    `content-disposition: attachment; filename="export.zip"`
  - And the body, parsed as a zip, has exactly 2 entries `src/main.rs` and `docs/readme.md` with
    byte-exact content
- **Priority:** Critical

#### E2E-NEW-016: side effect, export_links row deleted atomically on download
- **Category:** Side Effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** HTTP client + DB query
- **Steps:**
  - Given export `T` row confirmed present
  - When `GET /exports/T` returns `200`
  - Then `SELECT COUNT(*) FROM export_links WHERE token='T'` is `0` immediately after
- **Priority:** Critical

#### E2E-NEW-017: side effect, blob bytes deleted on successful download
- **Category:** Side Effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** HTTP client + blob store read
- **Steps:**
  - Given blob `export:T` confirmed present
  - When `GET /exports/T` returns `200`
  - Then a direct read of `export:T` after the request returns the not-found variant
- **Priority:** Critical

#### E2E-NEW-018: security, download succeeds with zero auth header and with a garbage header
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-NEW-017
- **Driver:** HTTP client
- **Steps:**
  - Given two independently-created tokens `T_a`, `T_b` (single-use forces two)
  - When `GET /exports/T_a` with no `Authorization` header, and `GET /exports/T_b` with
    `Authorization: Bearer garbage-token`
  - Then both return `200 OK` with correct zip content; neither ever returns `401`/`403`
- **Priority:** Critical

#### E2E-NEW-019: security, a non-member of the mount can still consume a valid token
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-NEW-017
- **Driver:** HTTP client
- **Steps:**
  - Given `owner@test.com` (member) created token `T`; `stranger@test.com` confirmed non-member via
    `admin.list_members`
  - When the download `GET` carries `stranger@test.com`'s bearer token
  - Then response is `200 OK` with correct zip content
- **Priority:** High

#### E2E-NEW-020: failure, token that never existed returns 404
- **Category:** Error Handling
- **Scenario:** SC-002
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given no export ever created for this token
  - When `GET /exports/00000000-0000-0000-0000-000000000000`
  - Then status is exactly `404`, body contains none of `"export_links"`, `"expired"`
- **Priority:** Critical

#### E2E-NEW-021: failure, malformed token returns 404 not 400/500
- **Category:** Error Handling
- **Scenario:** SC-002
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given no setup needed
  - When `GET /exports/not-a-valid-uuid-at-all`
  - Then status is exactly `404` (not `400`, not `500`)
- **Priority:** High

#### E2E-NEW-022: zip-content integrity end to end, byte-for-byte round trip
- **Category:** Data Integrity
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001, FR-NEW-009
- **Driver:** direct MCP tool call + HTTP download
- **Steps:**
  - Given `/data/report.bin` written with 10,000 bytes from `StdRng::seed_from_u64(42)`, captured as
    `expected_bytes`
  - When `fs.export_zip({"paths": ["/data/report.bin"]})` then `GET` the URL
  - Then the single entry `data/report.bin` equals `expected_bytes` exactly (full comparison)
- **Priority:** Critical

#### E2E-NEW-023: SC-003 setup, first download succeeds
- **Category:** Core Journey (setup step, asserted explicitly)
- **Scenario:** SC-003
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given token `T` created
  - When `GET /exports/T` once
  - Then status is `200 OK`
- **Priority:** Critical

#### E2E-NEW-024: SC-003 replay, second GET on same token returns 404
- **Category:** Error Handling
- **Scenario:** SC-003
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given `T` already consumed (E2E-NEW-023)
  - When `GET /exports/T` a second time
  - Then status is exactly `404`, body byte-identical to E2E-NEW-020's captured body
- **Priority:** Critical

#### E2E-NEW-025: side effect, replay does not re-delete or error on absent row
- **Category:** Side Effect
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-010
- **Driver:** HTTP client + DB query
- **Steps:**
  - Given `T` already consumed, row/blob already gone
  - When `GET /exports/T` a second time
  - Then no panic, no `500`, exactly `404`
  - And `export_links` row count for `T` remains `0`
- **Priority:** High

#### E2E-NEW-026: data integrity, 404 body identical across never-existed/expired/replayed
- **Category:** Data Integrity
- **Scenario:** SC-003, SC-004
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given `T1` (never created), `T2` (created, `expires_at` set to the past via direct DB update),
    `T3` (created, then downloaded once)
  - When `GET` is issued for each
  - Then all three return `404` with byte-identical bodies (same content-length, same bytes)
- **Priority:** High

#### E2E-NEW-027: SC-004 setup, unexpired never-downloaded token succeeds
- **Category:** Core Journey (setup)
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** HTTP client
- **Steps:**
  - Given token `T` created, `expires_at` unmodified (5 min future)
  - When `GET /exports/T` immediately
  - Then status is `200 OK` with correct zip content
- **Priority:** Critical

#### E2E-NEW-028: SC-004 failure, expired never-downloaded token returns 404
- **Category:** Error Handling
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** HTTP client + direct DB update
- **Steps:**
  - Given token `T`, `expires_at` directly updated to `now - 1s`
  - When `GET /exports/T`
  - Then status is exactly `404`, body shape identical to E2E-NEW-020
- **Priority:** Critical

#### E2E-NEW-029: side effect, expired-token GET deletes row and blob immediately
- **Category:** Side Effect
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** HTTP client + DB query + blob store read
- **Steps:**
  - Given same setup as E2E-NEW-028, row/blob confirmed present before the GET
  - When `GET /exports/T` returns `404`
  - Then row for `T` is gone (count 0) and blob at `export:T` is gone, both immediately after the
    response (not deferred to the sweep)
- **Priority:** Critical

#### E2E-NEW-030: boundary, GET exactly at and exactly after the expiry instant
- **Category:** Edge Case (boundary)
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** HTTP client + direct DB update + `tokio::time::sleep`
- **Steps:**
  - Given token `T` with `expires_at` set to `now + 100ms`; token `T2` created identically
  - When (a) `GET /exports/T` immediately; (b) sleep 150ms then `GET /exports/T2`
  - Then (a) returns `200`; (b) returns `404`
- **Priority:** High

#### E2E-NEW-031: SC-005 happy path, sweep deletes expired never-downloaded export
- **Category:** Core Journey (background)
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call (`purge::run_cycle`)
- **Steps:**
  - Given token `T` created, `expires_at` directly set to `now - 1h`
  - When `purge::run_cycle` runs for `proj`
  - Then it returns success
- **Priority:** Critical

#### E2E-NEW-032: side effect, export_links row removed by sweep
- **Category:** Side Effect
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call + DB query
- **Steps:**
  - Given state from E2E-NEW-031, row confirmed present before
  - When `purge::run_cycle` runs
  - Then row count for `T` is `0` after
- **Priority:** Critical

#### E2E-NEW-033: side effect, export blob removed by sweep
- **Category:** Side Effect
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call + blob store read
- **Steps:**
  - Given state from E2E-NEW-031, blob confirmed present before
  - When `purge::run_cycle` runs
  - Then `export:T` read after returns the not-found variant
- **Priority:** Critical

#### E2E-NEW-034: failure isolation, one bad row doesn't abort the sweep for others
- **Category:** Error Handling (isolation)
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call + DB/blob checks
- **Steps:**
  - Given expired tokens `T1`, `T2`; `T1`'s blob removed out-of-band before the sweep (simulating
    corruption), `T2`'s blob intact
  - When `purge::run_cycle` runs for `proj`
  - Then both rows are removed regardless of `T1`'s blob-delete outcome; `T2`'s blob confirmed
    deleted; the sweep still returns success
- **Priority:** Critical

#### E2E-NEW-035: state transition, sweep leaves unexpired rows untouched
- **Category:** State Transition
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call + DB/blob checks
- **Steps:**
  - Given `T_expired` (`expires_at = now-1h`) and `T_live` (`expires_at = now+4min`)
  - When `purge::run_cycle` runs
  - Then `T_expired` row+blob deleted; `T_live` row still present with unchanged `expires_at`, blob
    still readable with original bytes
- **Priority:** High

#### E2E-NEW-036: edge, sweep with zero export_links rows is a no-op
- **Category:** Edge Case (empty state)
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call + DB query
- **Steps:**
  - Given zero `export_links` rows for `proj`
  - When `purge::run_cycle` runs
  - Then it returns success, no panic, row count remains `0`
- **Priority:** High

#### E2E-NEW-037: concurrency, two concurrent GETs race on one token, exactly one wins
- **Category:** Concurrency
- **Scenario:** SC-002
- **Requirements:** FR-NEW-013
- **Driver:** `tokio::join!` on two `oneshot` calls against the same `Arc<AppState>`
- **Steps:**
  - Given token `T`, row/blob present
  - When two `GET /exports/T` requests are issued concurrently
  - Then exactly one response is `200` with correct zip content, the other is exactly `404`
  - And after both complete, row count for `T` is `0` and blob is gone
- **Priority:** Critical

#### E2E-NEW-038: SC-002 edge, in-flight download crossing the expiry boundary
- **Category:** Concurrency/Timing
- **Scenario:** SC-002
- **Requirements:** FR-NEW-012
- **Note:** this is the in-flight-crossing-expiry condition described in §5's cross-scenario note
  under SC-002, not a separate scenario.
- **Driver:** HTTP client + direct DB update + injected delay
- **Steps:**
  - Given token `T`, `expires_at` set to `now + 50ms`; the blob-read step is delayed ~150ms (test
    seam, see implementation note below) so `expires_at` elapses while the body is being produced
  - When `GET /exports/T` is issued before expiry, and the atomic delete commits before expiry
  - Then the response still completes with `200` and the complete, correct zip bytes, and the test
    asserts its own injected delay genuinely crossed `expires_at`
- **Priority:** Critical
- **Implementation note:** if a reliable test seam for injecting delay between the atomic delete and
  byte transmission proves awkward, prefer proving the ordering via code inspection (the delete must
  happen before any byte is read, per FR-NEW-009's "atomically... then read") plus a smaller
  deterministic unit test on the handler's internal sequencing, over a flaky wall-clock race.

#### E2E-NEW-044: failure, unauthorized caller rejected via the REST route too
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-004
- **Driver:** HTTP client
- **Steps:**
  - Given `stranger@test.com` not a member of `proj`, `/src/main.rs` exists
  - When `POST /api/fs/proj/export-zip` body `{"paths": ["/src/main.rs"]}` with `stranger@test.com`'s
    bearer token
  - Then status is exactly the REST equivalent of `ERR_FORBIDDEN` (`403`), no row/blob created
- **Priority:** High

#### E2E-NEW-045: failure, path escaping project root rejected via the REST route too
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Driver:** HTTP client
- **Steps:**
  - Given `/src/main.rs` exists, owner's bearer token
  - When `POST /api/fs/proj/export-zip` body `{"paths": ["/src/main.rs", "/../../../etc/passwd"]}`
  - Then status is the REST equivalent of `ERR_PATH_OUT_OF_BOUNDS` (`400`), no row/blob created
- **Priority:** High

#### E2E-NEW-046: edge, expiry is checked exactly once at request start, never re-checked mid-transmission
- **Category:** Edge Case
- **Scenario:** SC-002
- **Requirements:** FR-NEW-012
- **Driver:** code-level/unit test on the handler's internal sequencing (per the implementation note
  on E2E-NEW-038)
- **Steps:**
  - Given a token whose atomic delete-and-read has already committed
  - When the handler proceeds to transmit the bytes, with `expires_at` artificially rewound to the
    past on a stale in-memory clone (simulating "expiry elapsed after the check")
  - Then transmission still completes with `200` and full correct bytes, proving no second expiry
    check exists anywhere after the atomic step
- **Priority:** High

#### E2E-NEW-047: edge, an expired token's request is rejected even while an unrelated in-flight download for a different token is still transmitting
- **Category:** Edge Case
- **Scenario:** SC-002
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client, two tokens
- **Steps:**
  - Given token `T_live` mid-transmission (artificially delayed) and token `T_expired` whose
    `expires_at` has already passed, never downloaded
  - When `GET /exports/T_expired` is issued while `T_live`'s response is still being written
  - Then `T_expired`'s request returns `404` immediately, unaffected by `T_live`'s in-flight state
    (confirms the expiry check is per-request, not a global gate)
- **Priority:** Medium

#### E2E-NEW-048: concurrency, three concurrent GETs race on one token, exactly one wins
- **Category:** Concurrency
- **Scenario:** SC-002
- **Requirements:** FR-NEW-013
- **Driver:** `tokio::join!` on three `oneshot` calls against the same `Arc<AppState>`
- **Steps:**
  - Given token `T`, row/blob present
  - When three `GET /exports/T` requests are issued concurrently
  - Then exactly one is `200` with correct content, the other two are exactly `404`
- **Priority:** Medium

#### E2E-NEW-049: side effect, the race's losing requests see a truly absent row, not a logical rejection
- **Category:** Side Effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-013
- **Driver:** `tokio::join!` + DB query
- **Steps:**
  - Given the race from E2E-NEW-037/048
  - When both/all losing requests return `404`
  - Then a direct DB query immediately after confirms zero rows for `T` (the loser's `404` reflects
    the row already being gone, not a separate advisory check)
- **Priority:** Medium

#### E2E-NEW-050: structural, export_links migration runs cleanly on a fresh database, all three dialects
- **Category:** Structural
- **Scenario:** SC-001
- **Requirements:** FR-NEW-015
- **Driver:** `storage::conformance` suite
- **Steps:**
  - Given a brand-new SQLite file, a fresh PostgreSQL schema, and a fresh SQL Server database (no
    prior `export_links` data)
  - When the store's migration path runs
  - Then `export_links` exists with the exact declared columns on all three, and a subsequent
    insert/read/delete cycle succeeds identically on each
- **Priority:** Medium

#### E2E-NEW-051: edge, public_base_url supports ${VAR} expansion like every other config value
- **Category:** Edge Case (config)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-016
- **Driver:** config loader unit test
- **Steps:**
  - Given environment variable `MCPFS_PUBLIC_URL=https://files.example.com` and config
    `server.public_base_url: "${MCPFS_PUBLIC_URL}"`
  - When the config is loaded
  - Then `HttpConfig.public_base_url` resolves to exactly `"https://files.example.com"`
- **Priority:** Medium

#### E2E-NEW-052: security, download succeeds even with a forwarded-identity header present and invalid
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-NEW-017
- **Driver:** HTTP client
- **Steps:**
  - Given a valid token `T`
  - When `GET /exports/T` with header `X-Forwarded-Authorization: Bearer garbage`
  - Then status is exactly `200` with correct zip content, proving the route never inspects that
    header either
- **Priority:** Medium

#### E2E-NEW-053: security (structural), no admin.* tool exposes export invalidation
- **Category:** Security (structural)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-018
- **Driver:** structural check over `TOOL_CONTRACT.txt`'s `admin.*` family
- **Steps:**
  - Given the 14 `admin.*` tool entries in `TOOL_CONTRACT.txt:1-66`
  - When searching for any entry naming `export`
  - Then none exists (distinct from E2E-NEW-039's check over the full contract and CLI verbs)
- **Priority:** Medium

#### E2E-NEW-056: structural, public_base_url defaults to empty string when unset
- **Category:** Edge Case (config)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-016
- **Driver:** config loader unit test
- **Steps:**
  - Given a config file with no `server.public_base_url` key at all
  - When the config is loaded
  - Then `HttpConfig.public_base_url` resolves to exactly `""` (the declared default)
- **Priority:** Medium

#### E2E-NEW-057: edge, download route reachable when api.enabled=false
- **Category:** Edge Case (config)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009b
- **Driver:** HTTP client against an `AppState` built with `ServerConfig.api.enabled = false`
- **Steps:**
  - Given a server instance with `api.enabled: false` (MCP-only), token `T` created via `fs.export_zip`
  - When `GET /exports/T`
  - Then status is `200 OK` with correct zip content (not `404`, which is what it would be if the
    route lived inside `api::router()`)
- **Priority:** Critical

#### E2E-NEW-058: structural, export download router is merged unconditionally, not inside api::router()
- **Category:** Structural
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009b
- **Driver:** source-level check
- **Steps:**
  - Given `crates/core/src/app.rs`'s router assembly
  - When checking where `exports::router` is merged
  - Then it is merged alongside `deleted_projects_screen::router`/`trash_screen::router`
    (unconditional block), never inside the `if state.config.api.enabled { ... }` block that merges
    `api::router()`
- **Priority:** High

#### E2E-NEW-059: edge, full create-then-download round trip succeeds on an MCP-only deployment
- **Category:** Edge Case (end-to-end, config)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009b
- **Driver:** direct MCP tool call + HTTP client, `api.enabled = false`
- **Steps:**
  - Given a server instance with `api.enabled: false`
  - When `fs.export_zip({"mount_id": "proj", "paths": ["/src/main.rs"]})` returns a URL, then that URL
    is downloaded
  - Then the download succeeds with `200` and correct content, proving the whole feature works end to
    end on an MCP-only server, not just the two halves in isolation
- **Priority:** Critical

#### E2E-NEW-054: security (structural), no DELETE/invalidate route exists for an export
- **Category:** Security (structural)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-018
- **Driver:** HTTP client
- **Steps:**
  - Given a valid token `T`
  - When `DELETE /exports/T` is issued
  - Then the response is `404` or `405` (no such route/method exists), and a subsequent
    `GET /exports/T` still succeeds with `200` (the attempted delete had no effect)
- **Priority:** Medium

#### E2E-NEW-055: edge, a replay attempt with a case-differing token also returns 404
- **Category:** Edge Case
- **Scenario:** SC-003
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client
- **Steps:**
  - Given token `T` already consumed (lowercase UUID string)
  - When `GET /exports/{T.to_uppercase()}` is issued
  - Then status is exactly `404` (no case-insensitive fallback lookup exists)
- **Priority:** Low

#### E2E-NEW-039: security, no invalidation surface exists
- **Category:** Security (structural)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-018
- **Note:** cross-cutting (confirms the absence declared by FR-NEW-018), assigned to SC-001 per §11's
  convention for structural/negative requirements.
- **Driver:** structural check over `TOOL_CONTRACT.txt` (620 lines, `TOOL_CONTRACT.txt:1-620`) and the
  CLI verb list
- **Steps:**
  - Given the full tool contract and the `mcp-fs` CLI verbs (`serve | keys | token | migrate |
    purge | version`, `crates/core/src/cli.rs`)
  - When searching for any export-invalidation entry
  - Then no tool name other than `fs.export_zip` contains `export`; no CLI verb other than `purge`
    touches `export_links`
- **Priority:** High

#### E2E-NEW-040: structural, export_links schema declared alongside trash_entries, migrated identically
- **Category:** Data Integrity (structural)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-015
- **Note:** cross-cutting (confirms FR-NEW-015), assigned to SC-001 per §11's convention for
  structural/negative requirements.
- **Driver:** source-level check + `storage::conformance` suite run
- **Steps:**
  - Given `crates/core/src/storage/meta.rs`'s schema module
  - When checking the module `export_links` is declared in
  - Then it's the same module/list as `trash_entries`, and the conformance suite's create/read/delete
    cycle passes identically across SQLite, PostgreSQL, SQL Server
- **Priority:** High

#### E2E-NEW-041: failure, unauthenticated MCP tool call rejected consistently with other tools
- **Category:** Security / Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Driver:** direct MCP tool call
- **Steps:**
  - Given no resolvable identity attached to the call context
  - When `fs.export_zip` is invoked with `{"mount_id": "proj", "paths": ["/src/main.rs"]}`
  - Then the error code matches exactly what an existing tool (e.g. `fs.read_text`) returns under the
    identical missing-identity condition (cross-checked, not hardcoded)
  - And no row/blob created
- **Priority:** High

#### E2E-NEW-042: failure, malformed JSON body on REST route returns 400
- **Category:** Error Handling
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Driver:** HTTP client
- **Steps:**
  - Given a valid bearer token
  - When `POST /api/fs/proj/export-zip` body `{"paths": "not-an-array"}`
  - Then status is exactly `400`, no row/blob created
- **Priority:** Medium

#### E2E-NEW-043: edge, empty-string path entry rejected, never silently skipped
- **Category:** Edge Case
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003, FR-NEW-006
- **Driver:** direct MCP tool call
- **Steps:**
  - Given `/src/main.rs` exists
  - When `fs.export_zip` is called with `{"mount_id": "proj", "paths": ["/src/main.rs", ""]}`
  - Then the call returns an error (exact code recorded: `ERR_NOT_FOUND` or `ERR_INVALID_ARGUMENT`,
    whichever the implementation actually produces — never a silent `200` with the entry dropped)
  - And no row/blob created
- **Priority:** Medium

## 15. Open Questions & TBDs

None outstanding — every Round 1-5 question was answered or explicitly decided during the interview.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| Export | A single zip archive built from a caller-selected set of files/directories, stored transiently, delivered once | Export creation, Signed delivery |
| Signed URL | A URL containing an unguessable token that is itself the sole authorization to download one specific export | Signed delivery |
| Single-use | The property that the first successful `GET` on a token consumes it; any further `GET` on the same token returns `404` | Signed delivery |
| Grace period | Equal to the link's own expiry (DEC-013) — there is no separate window beyond `expires_at` for a never-downloaded export | Lifecycle sweep |
| Sweep | The existing background `purge::run_cycle`, now also responsible for deleting expired, never-downloaded exports | Lifecycle sweep |
| Blob key namespacing | Storing non-filesystem artifacts in the same `BlobBackend` under a prefixed key (`export:{token}`), bypassing content-addressed refcounting, as `git:{sha}` already does for git objects | Export creation, Current State |

## 17. Decisions Log

- **DEC-001:** Extend the existing zip capability (arbitrary file list + signed-URL mode) rather than
  build a parallel route family. **Rationale:** user's explicit confirmation; reuses the proven
  in-memory zip-building code path. **Alternatives considered:** a wholly separate feature — rejected
  as redundant. **Implemented by:** FR-NEW-001, FR-NEW-002. **Round:** 1.
- **DEC-002:** Fixed 5-minute expiry, not configurable per call. **Rationale:** short window chosen
  deliberately to bound exposure of a self-authorizing link; justified by DEC-006's "in-flight survives
  expiry" property. **Alternatives considered:** per-call configurable expiry with a max cap —
  rejected, YAGNI for v1. **Implemented by:** FR-NEW-007. **Round:** 1.
- **DEC-003:** Usable from both MCP tools and REST. **Implemented by:** FR-NEW-001, FR-NEW-002.
  **Round:** 1.
- **DEC-004:** The signed URL is self-authorizing — no bearer/membership check on the download route.
  **Rationale:** the point of a hand-off link is that the recipient has no system credential at all.
  **Alternatives considered:** requiring the original caller's bearer token on download — rejected,
  defeats the purpose of a shareable link. **Implemented by:** FR-NEW-017. **Round:** 1.
- **DEC-005:** No revocation capability in v1. **Rationale:** short TTL makes revocation low-value;
  explicit user decision. **Alternatives considered:** an explicit invalidate tool — deferred, not
  built. **Implemented by:** FR-NEW-018. **Round:** 1.
- **DEC-006:** Expiry checked once at request start; an in-flight buffered download completes even
  past the 5-minute mark. **Rationale:** the response is always fully buffered before transmission in
  this codebase's existing pattern (`download`/`download-zip`), so this falls out naturally rather than
  requiring new machinery. **Implemented by:** FR-NEW-012. **Round:** 1.
- **DEC-007:** Relative paths preserved in the zip — each entry named by its full project-relative
  path. **Rationale:** guarantees no name collisions across an arbitrary multi-path selection without
  needing a "common root" computation. **Alternatives considered:** flattening to basenames — rejected,
  collides and loses structure. **Implemented by:** FR-NEW-001. **Round:** 3.
- **DEC-008:** Opaque random token (`uuid::Uuid::new_v4()`) + a server-side `export_links` record,
  not a stateless signed (HMAC/JWT) token. **Rationale:** a record is needed regardless for cleanup
  bookkeeping, so statelessness buys nothing. **Alternatives considered:** stateless HMAC-signed token
  — rejected for that reason. **Implemented by:** FR-NEW-007, FR-NEW-015. **Round:** Phase 2.
- **DEC-009:** Cleanup folds into the existing `purge::run_cycle`, no new sweeper process.
  **Rationale:** one cleanup entry point already exists and already has the right error-isolation
  shape. **Implemented by:** FR-NEW-014. **Round:** 1/Phase 2.
- **DEC-010:** Unlimited file count/total size, matching today's `download-zip`. **Rationale:** no new
  limit was requested; consistent with the capability being extended. **Implemented by:** §7.1 (NFR,
  no FR needed — absence of a limit is not a testable positive requirement). **Round:** 1.
- **DEC-011:** Download route is `GET /exports/{token}`, a wholly new top-level path sibling to
  `/health`/`/api`/`/git`/`/app` (`app.rs`'s router), not nested under `/api/fs/*`. **Rationale:**
  avoids leaking which project a token belongs to via the URL shape (the token alone must resolve it);
  revised in round 2 of Phase 6 from an earlier `/api/fs/export/{token}` shape after the auditor found
  that shape would collide with `/api/fs/{mount_id}/{action}` for any project literally named
  `export` (axum's router prefers a static segment over a parameter at the same tree position, so
  `export` as a `mount_id` would be shadowed) — moving the route to a separate top-level path removes
  the collision entirely rather than reserving a word. **Implemented by:** FR-NEW-009. **Round:** 1,
  revised Phase 6 round 2.
- **DEC-012:** Single-use semantics — first `GET` serves and deletes; replay returns `404`.
  **Rationale:** simplest implementation consistent with the existing fully-buffered response pattern;
  avoids needing retry/multi-use bookkeeping. **Alternatives considered:** repeatable-until-expiry —
  rejected, user confirmed single-use is acceptable. **Implemented by:** FR-NEW-010, FR-NEW-013.
  **Round:** 1.
- **DEC-013:** Grace period for a never-downloaded export equals the link's own expiry — no separate
  grace window. **Rationale:** user's explicit simplification; the sweep's criterion is simply
  `expires_at < now`. **Implemented by:** FR-NEW-014. **Round:** 1.
- **DEC-014 (Phase 2, Approach A chosen):** store export bytes directly in the existing `BlobBackend`
  under key `export:{token}`, using the same **blob key namespacing** technique the `git:{sha}`
  precedent already established, bypassing content-addressing entirely; rejected Approach B (writing
  the zip into the visible filesystem tree and reusing `fs_ops` write/delete) because it would pollute
  `fs.list`/`fs.grep`/search/quota/trash with ephemeral system-internal artifacts. **Implemented by:**
  FR-NEW-007, FR-NEW-015. **Round:** Phase 2.
- **DEC-015:** New config key `server.public_base_url` (default `""`), since no existing config key
  builds an absolute link back to this server. **Implemented by:** FR-NEW-008, FR-NEW-016.
  **Round:** Phase 2 research.
- **DEC-016:** `export_links` declared in the same schema module as `trash_entries`
  (`storage/meta.rs`), not a separate sub-store like `git/db.rs`. **Rationale:** `export_links` is
  core-engine state, not git-specific; `trash_entries` is the closer and more consistent precedent for
  a small side table living alongside `nodes`/`blob_refs`. **Implemented by:** FR-NEW-015.
  **Round:** Phase 3 (corrected during impact analysis after checking the precedent directly).
- **DEC-017:** Zip filename returned to the recipient is fixed `export.zip` (no caller-supplied name).
  **Rationale:** YAGNI — not requested, keeps the response shape simple. **Implemented by:**
  FR-NEW-009. **Round:** Phase 2.
- **DEC-018:** No separate out-of-scope list item needed for "revocation" beyond DEC-005 — confirmed
  as one and the same decision, not two.

## 18. Implementability Gate

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 1 (F1: config key named `http.public_base_url` throughout, but `ServerConfig.server: HttpConfig` is the real struct path per `config.rs:140-145,856` — every deployment config uses the `server:` YAML key, never `http:`) | 1 (DRIFT-001: §9.1 claimed `storage/volume.rs` needed modification; `VolumeClient.blob` is already a public field, no change needed) | NOT-IMPLEMENTABLE |
| 2 | 2 (F2: FR-NEW-001/002 cited `core::fs_ops` for directory walk and byte reads, but the real `download_zip` precedent calls `VolumeClient::walk`/`VolumeClient::read_bytes` directly, never `core::fs_ops` — `fs_ops.rs` has no `walk` fn and no plain-bytes `read_bytes` fn, only `read_bytes_b64`; F3: route `GET /api/fs/export/{token}` would collide with `/api/fs/{mount_id}/{action}` for a project literally named `export`, since axum's router prefers a static segment over a parameter at the same tree position) | 2 (DRIFT-002: §2.1/FR-NEW-007's `odb.rs` citation for `blob_key` pointed at the wrong range during a round-2 self-correction that was itself wrong; the real fn is at `odb.rs:42-44` — confirmed correct again in round 3; DRIFT-003: FR-NEW-014's `purge.rs:166-170` citation for `CycleSummary` was off by a few lines, the real struct is at `purge.rs:163-167`) | NOT-IMPLEMENTABLE |
| 3 | 1 (F4: §9.1 placed the `GET /exports/{token}` handler inside `api/dataplane.rs`'s `router()`, which only merges into the app when `state.config.api.enabled` is true (`app.rs:141-144`); an MCP-only deployment with `api.enabled: false` would mint tokens via `fs.export_zip` whose only consumption route is never registered — permanent 404. DEC-011 already required the route to behave like `/health` (always mounted), §9.1's placement just didn't follow through) | 2 (DRIFT-004/005: two more citation off-by-a-few-lines errors, `dataplane.rs:600,610` for the `walk`/`read_bytes` call sites — the real lines are `607,614` — surfaced while verifying F4's surrounding code) | NOT-IMPLEMENTABLE |

**Three-round cap reached.** Per the gate's own rule, a round 3 `NOT-IMPLEMENTABLE` stops here rather
than spawning a round 4 auditor. F4 is a pure precision/registration-mechanics fix with no product
ambiguity (DEC-011 already settled the *intent* — "always reachable, like `/health`" — §9.1 simply
hadn't implemented that intent), so it was corrected directly rather than escalated for arbitration:
the handler moves to a new module `crates/core/src/exports.rs` with its own `router()`, merged
unconditionally in `app.rs` alongside `deleted_projects_screen`/`trash_screen` (new FR-NEW-009b).
DRIFT-004/005 were likewise evident citation corrections, applied in place. This fix was verified
manually against the cited files (`app.rs:136-144`) rather than via a fourth auditor sub-agent, and is
flagged to the user for explicit sign-off before commit, per the escalation rule for a round that
finds a genuinely new F each time.

**Amendments applied:** Round 1 — F1: every occurrence of `http.public_base_url` replaced with
`server.public_base_url` (FR-NEW-008, FR-NEW-016, E2E-NEW-013, E2E-NEW-014, E2E-NEW-051,
E2E-NEW-056, §9.1, DEC-015); FR-NEW-016 now cites the exact struct/field path. DRIFT-001 — §9.1's
`storage/volume.rs` row corrected to `None`/no change, with the real access path cited. Round 2 — F2:
FR-NEW-001/002 and the SC-001 flow now cite `VolumeClient::walk`/`VolumeClient::read_bytes`
(`storage/volume.rs:130`,`:92`), matching `dataplane.rs:607,614`'s real precedent, not `core::fs_ops`.
F3: download route moved from `/api/fs/export/{token}` to a new top-level `/exports/{token}` (DEC-011
revised), eliminating the mount-id collision entirely rather than reserving the word `export`; every
occurrence in the document updated. DRIFT-002/003 — citation line numbers corrected and re-verified
(`odb.rs:42-44`, `purge.rs:163-167`). Round 3 — F4: new FR-NEW-009b requires `GET /exports/{token}`
to be registered in `app.rs` unconditionally (its own `exports.rs` module, mirroring
`deleted_projects_screen`/`trash_screen`), not inside `api::router()`; §9.1's affected-components table
updated accordingly (dropped the route from `api/dataplane.rs`'s row, added `exports.rs` and `app.rs`
rows). DRIFT-004/005 — `dataplane.rs:600,610` corrected to the real call-site lines `607,614`
(FR-NEW-001, §5 SC-001 step 3, and §18's own round-2 narrative).
**Drift registered:** none outstanding — all five A findings across three rounds were evident
corrections, amended in place rather than deferred to the register.
