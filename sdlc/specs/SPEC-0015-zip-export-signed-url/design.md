# Zip export via signed download URL — Design Document

> Id: SPEC-0015 (migrated from legacy SPEC-0012)

## 1. Components

- `crates/core/src/tools/export.rs` — `fs.export_zip` MCP tool, mirrors `tools/trash.rs`'s structure.
- `crates/core/src/api/dataplane.rs` — `POST /api/fs/{mount_id}/export-zip` route; `attachment()`
  helper promoted to `pub(crate)` for reuse by the download route.
- `crates/core/src/exports.rs` — new module: `GET /exports/{token}` handler and its own
  `pub fn router(state: Arc<AppState>) -> Router`, registered unconditionally.
- `crates/core/src/app.rs` — merges `exports::router` alongside `deleted_projects_screen`/
  `trash_screen`, outside the conditionally-mounted `api::router()`.
- `crates/core/src/storage/meta.rs` — `export_links` table + accessor functions (insert, atomic
  delete-and-fetch, sweep-expired), declared in the same schema module as `trash_entries`.
- `crates/core/src/storage/volume.rs` — unchanged; `VolumeClient.blob` is already a public
  `Arc<dyn BlobBackend>` field, reachable directly as `client.blob.{put,get,delete}("export:{token}", ...)`.
- `crates/core/src/purge.rs` — third per-project sweep step inside `run_cycle`; `CycleSummary` gains
  `exports_swept: usize`.
- `crates/core/src/config.rs` — `HttpConfig.public_base_url: String` (default `""`).
- `TOOL_CONTRACT.txt` / `tool-contract-golden.json` — new `fs.export_zip` entry.

## 2. Flows

### Creation (SC-001)
`fs.export_zip` / `POST .../export-zip` → `state.authorize(mount_id, person)` → normalize every path
→ for each path: `VolumeClient::walk` if directory, else treat as file → `VolumeClient::read_bytes`
per resulting file → build in-memory zip (entry name = full project-relative path, leading slash
stripped) → generate `token = uuid::Uuid::new_v4()` → `blob.put("export:{token}", zip_bytes)` →
insert `export_links` row (`created_at`=now, `expires_at`=now+300s) → build URL from
`server.public_base_url` (or relative) → return `{"url": ...}`. All-or-nothing: any rejection
(empty list, unauthorized, out-of-bounds path, missing path) aborts before any row/blob is created.

### Download (SC-002/003/004)
`GET /exports/{token}` (no auth) → atomic delete-and-fetch against `export_links` keyed by `token` →
if no row: `404` (covers never-existed, already-consumed, already-swept, indistinguishably) → if row
found but `expires_at` already passed: delete blob too, `404` → if row found and live: read blob at
`export:{token}`, delete blob, stream back as `application/zip` attachment `export.zip` via
`attachment()`. Expiry is evaluated exactly once, before the blob read; transmission of
already-buffered bytes is never interrupted by expiry elapsing mid-transfer.

### Sweep (SC-005)
`purge::run_cycle`'s per-project loop → existing `sweep_project_files`/`sweep_project` → new third
step: for each `export_links` row with `expires_at` in the past, delete row then delete blob at
`export:{token}`; a failure on one row (e.g. blob already missing) does not abort the row loop or the
project loop.

## 3. Interfaces

| Interface | Shape |
|---|---|
| MCP tool `fs.export_zip` | params `mount_id: String`, `paths: [String]` → `{"url": "<string>"}` or `ToolError` |
| `POST /api/fs/{mount_id}/export-zip` | body `{"paths": ["..."]}` → `200 {"url": "..."}` / `400`/`403`/`404` |
| `GET /exports/{token}` | no auth → `200 application/zip` attachment `export.zip`, or `404` |
| (absent) `DELETE /exports/{token}` | no such route/method; `404`/`405` |

## 4. Data and state

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

## 5. Configuration

`HttpConfig.public_base_url: String`, default `""`, effective config path `server.public_base_url`
(not a new top-level `http:` section, not a rename of the existing `server` field). Supports `${VAR}`
expansion like every other config value.

## 6. Observability

INFO on export creation (`mount_id`, file count); INFO on download outcome (served / expired /
replayed, token truncated/hashed); DEBUG on sweep (rows/blobs deleted per cycle). Full token value and
file contents never traced.

## 7. Decisions

- **DEC-001:** Extend the existing zip capability rather than build a parallel route family. Reuses
  the proven in-memory zip-building code path.
- **DEC-002:** Fixed 5-minute expiry, not configurable per call. Bounds exposure of a self-authorizing
  link; per-call configurable expiry rejected as YAGNI for v1.
- **DEC-003:** Usable from both MCP tools and REST.
- **DEC-004:** The signed URL is self-authorizing — no bearer/membership check on download. A
  shareable link requires the recipient to carry no system credential.
- **DEC-005:** No revocation capability in v1. Short TTL makes revocation low-value.
- **DEC-006:** Expiry checked once at request start; an in-flight buffered download completes even
  past the 5-minute mark, since the response is always fully buffered before transmission in this
  codebase's existing pattern.
- **DEC-007:** Relative paths preserved in the zip — guarantees no name collisions across an arbitrary
  multi-path selection without a "common root" computation. Flattening to basenames rejected (collides,
  loses structure).
- **DEC-008:** Opaque random token (`uuid::Uuid::new_v4()`) + a server-side `export_links` record, not
  a stateless signed (HMAC/JWT) token. A record is needed regardless for cleanup bookkeeping, so
  statelessness buys nothing.
- **DEC-009:** Cleanup folds into the existing `purge::run_cycle`, no new sweeper process.
- **DEC-010:** Unlimited file count/total size, matching today's `download-zip`. No new limit was
  requested.
- **DEC-011:** Download route is `GET /exports/{token}`, a wholly new top-level path sibling to
  `/health`/`/api`/`/git`/`/app`, not nested under `/api/fs/*`. Avoids leaking which project a token
  belongs to via the URL shape, and avoids a router collision with `/api/fs/{mount_id}/{action}` for
  any project literally named `export` (axum prefers a static segment over a parameter at the same
  tree position).
- **DEC-012:** Single-use semantics — first `GET` serves and deletes; replay returns `404`.
  Repeatable-until-expiry rejected; user confirmed single-use is acceptable.
- **DEC-013:** Grace period for a never-downloaded export equals the link's own expiry — no separate
  grace window; the sweep's criterion is simply `expires_at < now`.
- **DEC-014:** Store export bytes directly in the existing `BlobBackend` under key `export:{token}`
  (the `git:{sha}` blob-key-namespacing technique), bypassing content-addressing entirely. Writing the
  zip into the visible filesystem tree and reusing `fs_ops` write/delete was rejected: it would pollute
  `fs.list`/`fs.grep`/search/quota/trash with ephemeral system-internal artifacts.
- **DEC-015:** New config key `server.public_base_url` (default `""`), since no existing config key
  builds an absolute link back to this server.
- **DEC-016:** `export_links` declared in the same schema module as `trash_entries`
  (`storage/meta.rs`), not a separate sub-store like `git/db.rs`, since it is core-engine state, not
  git-specific.
- **DEC-017:** Zip filename returned to the recipient is fixed `export.zip` (no caller-supplied name).
  YAGNI — not requested, keeps the response shape simple.
- **DEC-018:** No separate out-of-scope item for "revocation" beyond DEC-005 — one and the same
  decision, not two.
- **DEC-019 (was FR-NEW-009b's fix, Phase 6 round 3):** `GET /exports/{token}` is registered
  unconditionally in `app.rs`, in its own `exports.rs` module mirroring
  `deleted_projects_screen`/`trash_screen`, never inside the conditionally-mounted `api::router()`,
  so the route stays reachable even when `api.enabled: false` (MCP-only deployment).

## 8. Requirement to code map

| Requirement | Code |
|---|---|
| FR-NEW-001, FR-NEW-002 | `crates/core/src/tools/export.rs`, `crates/core/src/api/dataplane.rs` |
| FR-NEW-003..006 | validation in the shared creation path (both tool and REST) |
| FR-NEW-007, FR-NEW-015 | `crates/core/src/storage/meta.rs` (`export_links` table + accessors) |
| FR-NEW-008, FR-NEW-016 | `crates/core/src/config.rs` (`HttpConfig.public_base_url`), `crates/core/src/exports.rs` (URL build) |
| FR-NEW-009, FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013, FR-NEW-017 | `crates/core/src/exports.rs` |
| FR-NEW-009b | `crates/core/src/app.rs`, `crates/core/src/exports.rs` |
| FR-NEW-014 | `crates/core/src/purge.rs` (`run_cycle`, `CycleSummary.exports_swept`) |
| FR-NEW-018 | structural absence; no code reference |

## 9. Legacy mapping

Source: specs/archived/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md (pre-move, renumbered to SPEC-0015); plus drift/2026-10-08_15-03-55.md

| Old id | New id | Note |
|---|---|---|
| SPEC-0012 (zip-export-signed-url) | SPEC-0015 | renumbered on migration; legacy number collided with another legacy-numbered SPEC-0012 (rust-skill-compliance-debt) |
| US-0001 (foundation: schema + storage primitives) | FR-NEW-015 | done, sha 6fd01b8 |
| US-0002 (export creation: tool + REST route + URL config) | FR-NEW-001..008, 016 | done, sha 6fd01b8 |
| US-0003 (signed URL download: route, atomic consume, concurrency, unconditional mount) | FR-NEW-009, 009b, 010..013, 017, 018 | done, sha 6fd01b8 |
| US-0004 (background purge sweep) | FR-NEW-014 | done, sha 6fd01b8 |
| US-0005 (converge gap: export_links multi-dialect conformance case) | FR-NEW-015 | done, sha 6fd01b8 |
