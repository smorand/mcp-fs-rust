# mcp-fs trash listing and recovery — Design Document

> Spec: SPEC-0013 (migrated from legacy SPEC-0011)
> Status: implemented

## 1. Components

- `crates/core/src/storage/rel/schema.rs`, `storage/rel/{sqlite,postgres,sqlserver}.rs`: the `trash_entries` table, one implementation per dialect, following the project's existing dialect checklist (see `.agent_docs/backends.md`).
- `crates/core/src/storage/meta.rs`: `VolumeClient`'s new `trash` handle, including `record_trash_entry` (insert), the backfill insert, and the delete-on-restore.
- `crates/core/src/core/fs_ops.rs`: `delete_path`'s soft-delete branch, plus the shared `rename_with_collision_retry(client, src, dst) -> Result<String>` helper used by both `delete_path` and `sweep_project_files`.
- `crates/core/src/purge.rs`: `sweep_project_files`, reusing the same collision-retry helper and writing `deleted_by: null` trash entries.
- `crates/core/src/tools/trash.rs` (new): typed `pub(crate)` functions `trash_list` and `trash_restore`, including the lazy backfill reconstruction logic, called by both the MCP tools and the GUI screen.
- `crates/core/src/mcp/server.rs`: two new `#[tool]` methods, `fs.trash_list` and `fs.trash_restore`; `CreateProjectArgs`/`admin_create_project` extended with the four optional purge params.
- `crates/core/src/trash_screen.rs` (new): `GET /app/trash`, `POST /app/trash/restore`, modeled on `deleted_projects_screen.rs`'s session-cookie + CSRF convention.
- `crates/core/src/storage/admin.rs`, `crates/core/src/tools/admin.rs`: `create_project` / `AdminBackend::create_project` signature grows to accept the four optional purge params, applied atomically.
- `crates/core/src/app.rs`: mounts `trash_screen::router` alongside `deleted_projects_screen::router`.
- `TOOL_CONTRACT.txt`, `tool-contract-golden.json`: regenerated last, once every tool/schema change is final.

## 2. Flows

**Soft-delete (user-initiated or sweep):** `delete_path`/`sweep_project_files` compute the flattened trash destination → `rename_with_collision_retry` (retry on `ERR_NO_CLOBBER` only, `~1`..`~50`, else `ERR_INTERNAL_ERROR`) → `record_trash_entry` with `original_path`, `size`, `kind`, `deleted_by` (caller, or `null` for sweep). Hard delete (`trash=false`) skips the trash-entry write entirely.

**List:** `fs.trash_list` → `state.authorize` → backfill pass over untracked nodes directly under `trash_dir` (idempotent insert, reconstructing `original_path` from the flattened name and `deleted_at` from the epoch-ms prefix) → query filtered by `path_prefix`, ordered by `deleted_at` descending, paginated by `limit`/`offset` → `purge_in_days` computed server-side from the project's `PurgeConfig.file_retention_days`.

**Restore:** `fs.trash_restore` → `state.authorize` → backfill if the entry is untracked → resolve `original_path` → `makedirs` any missing ancestor directories → rename back, retrying on destination collision with `_restored`, `_restoredN` suffixes (never failing on this account) → delete the `trash_entries` row → return `{restored_path, trash_path}`.

**GUI:** `GET /app/trash` with no `mount_id` → picker from `AdminBackend::list_projects_for(person)` (empty-state message if zero memberships). With `mount_id` → renders the same `trash_list` result. `POST /app/trash/restore` → CSRF check (only when identity came from the cookie) → calls `trash_restore` → redirect.

## 3. Interfaces

- `fs.trash_list(mount_id: string, path_prefix: string = "", limit: integer = 200, offset: integer = 0) -> {entries: [...], total: int}`. Entry shape: `{trash_path, original_path, size, kind, deleted_at, deleted_by, purge_in_days}`.
- `fs.trash_restore(mount_id: string, trash_path: string) -> {restored_path: string, trash_path: string}`.
- `admin.create_project(project_id: string, owner: string, autopurge_enabled: boolean = false, use_internal_purge: boolean = false, file_retention_days: number|null = null, project_retention_days: number|null = null)` — unchanged return shape.
- `GET /app/trash[?mount_id=<id>]`, `POST /app/trash/restore` (form-encoded, CSRF token on cookie-identified sessions).
- Error codes reused, no new codes added: `ERR_PROJECT_NOT_FOUND`, `ERR_FORBIDDEN`, `ERR_INVALID_ARGUMENT`, `ERR_NOT_FOUND`, `ERR_INTERNAL_ERROR`, `ERR_NO_CLOBBER` (internal retry trigger, never surfaced past the helper).

## 4. Data and state

**`trash_entries`** (one row per volume per soft-deleted top-level path):

| Column | Type | Notes |
|---|---|---|
| `volume_id` | string | composite PK with `trash_path`, scopes every row like `nodes`/`blob_refs` |
| `trash_path` | string (bounded, `TextKey` on SQL Server) | composite PK |
| `original_path` | string | restore target |
| `size` | integer | copied from the node at delete time |
| `kind` | string, `'file'` \| `'dir'` | |
| `deleted_at` | string (RFC3339) | matches `admin.list_deleted_projects`'s convention |
| `deleted_by` | string, nullable | `null` for sweep-initiated deletes |

Secondary index on `(volume_id, deleted_at)` supports `fs.trash_list`'s required ordering. No changes to `NodeRow` or any existing table. `PurgeConfig` itself is unchanged in shape; only its write path (now also reachable from `create_project`) changes.

## 5. Configuration

No new configuration keys. `admin.create_project`'s four new optional parameters mirror `admin.set_purge_config`'s existing names/types/validation (`0` or negative retention → `ERR_INVALID_ARGUMENT`); omitting all four preserves today's `PurgeConfig::default()` exactly.

## 6. Observability

- `fs.trash_list` / `fs.trash_restore` traced at INFO (API-call level, matching every other `fs.*` tool).
- The `trash_entries` insert/delete traced at DEBUG (file-mutation level, matching `fs.delete`'s existing tracing).
- No new credentials, tokens, or PII enter any trace.

## 7. Decisions

- **DEC-001:** New deletes/sweeps write an explicit `trash_entries` row rather than reconstructing `original_path` from the trash path string going forward, because the flatten encoding is lossy (`/a/b.txt` and `/a__b.txt` collide) and changing the encoding itself would break the frozen `trash_path` sample already in `TOOL_CONTRACT.txt`.
- **DEC-002:** Trash-write collisions retry with `~N` (bounded, fails loud at 50); restore-destination collisions retry with `_restoredN` and never fail. Two distinct suffix schemes are kept because they solve different problems at different layers (internal write-side vs. user-facing restore-side), and silently overwriting or hard-failing on restore collision were both rejected.
- **DEC-003:** `trash_entries` starts empty; legacy trashed files are backfilled lazily and idempotently on first `fs.trash_list`/`fs.trash_restore` touch, with best-effort `deleted_at` (parsed epoch prefix, else now) — no separate startup migration job.
- **DEC-004:** No per-file ownership-based ACL; any project member can list/restore any trashed file in that project, since no per-file attribution exists anywhere in the codebase (`NodeRow` has no owner column) and adding one was judged out of proportion to this increment.
- **DEC-005:** Platform admin status grants no implicit file access; the existing `state.authorize` membership gate applies unchanged to the new tools, matching the project's documented convention that managing the platform is not the same as reading data.
- **DEC-006:** `admin.create_project` gains the same four optional purge params as `admin.set_purge_config` rather than a separate `create_project_with_retention` tool, to avoid a second entry point for one operation.
- **DEC-007:** No `admin.get_purge_config` MCP tool is added — nothing in this spec's requirements needs it; `fs.trash_list`'s `purge_in_days` is computed server-side from the internal config read.
- **DEC-008:** Collision-exhaustion and unrelated rename errors on the trash-write path reuse `ERR_INTERNAL_ERROR`; no new `ERR_*` code, since this is an exceedingly rare operational failure, not a distinct class a caller is expected to branch on.
- **DEC-009 (accepted drift, 2026-10-06):** the rename and the `trash_entries` insert/delete do **not** share one transaction, despite the spec's original NFR 7.4 and FR-NEW-002 wording. `VolumeClient::meta` is a trait object (`MetaBackend`) whose generic `rename` implementation commits its own transaction internally; `record_trash_entry` runs as a separate, best-effort-sequential `run_retrying` transaction immediately after. **Rationale (user decision at Phase 4.6 CONVERGE):** a true joint-transaction fix requires widening `MetaBackend` with a new dialect-agnostic rename+insert method, judged out of proportion to this increment; the existing lazy backfill (FR-NEW-011) already self-heals the narrow crash-window case (a renamed-but-untracked trash node gets picked up and backfilled, with a best-effort `deleted_at` rather than the exact original delete time), so the practical blast radius is bounded and the file is never lost. **Deferred to:** `backlog/BL-0011_atomic-trash-entry-write.md`. **Not implemented in this spec's scope.**

## 8. Requirement to code map

| FR | Component |
|---|---|
| FR-NEW-001 | `storage/rel/schema.rs`, `storage/rel/{sqlite,postgres,sqlserver}.rs` |
| FR-NEW-002, FR-NEW-013 | `core/fs_ops.rs::delete_path` |
| FR-NEW-003 | `purge.rs::sweep_project_files` |
| FR-NEW-004, FR-NEW-005, FR-NEW-006 | `core/fs_ops.rs::rename_with_collision_retry` |
| FR-NEW-007, FR-NEW-008, FR-NEW-011 | `tools/trash.rs::trash_list`, `mcp/server.rs` (`fs.trash_list`) |
| FR-NEW-009, FR-NEW-010, FR-NEW-011 | `tools/trash.rs::trash_restore`, `mcp/server.rs` (`fs.trash_restore`) |
| FR-NEW-012, FR-NEW-018 | `trash_screen.rs`, `app.rs` (mount) |
| FR-NEW-014, FR-NEW-015 | `storage/admin.rs`, `tools/admin.rs`, `mcp/server.rs` (`CreateProjectArgs`/`admin_create_project`) |
| FR-NEW-016 | `tools/trash.rs` (authorize call), both tool handlers |
| FR-NEW-017 | process requirement, no single code location |

## 9. Legacy mapping

Source: specs/archived/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md (pre-move, renumbered to SPEC-0013); plus its drift/2026-10-06_11-29-34.md

| Old id | New id | Note |
|---|---|---|
| SPEC-0011 (trash listing and recovery) | SPEC-0013 | implemented; legacy numbering collided with another legacy SPEC-0011 ("full-git-dev-process"), renumbered on migration |
| US-0001 — `trash_entries` persistence layer | SPEC-0013 FR-NEW-001 | done, implemented, not re-tracked in v2 queue |
| US-0002 — `fs.delete` writes the trash entry, collision-safe | SPEC-0013 FR-NEW-002, 004, 005, 006, 013 | done, implemented, not re-tracked in v2 queue (sha 0bfb359) |
| US-0003 — Sweep writes the trash entry | SPEC-0013 FR-NEW-003 | done, implemented, not re-tracked in v2 queue |
| US-0004 — `fs.trash_list` tool | SPEC-0013 FR-NEW-007, 008, 011, 016 | done, implemented, not re-tracked in v2 queue |
| US-0005 — `fs.trash_restore` tool | SPEC-0013 FR-NEW-009, 010 | done, implemented, not re-tracked in v2 queue |
| US-0006 — `/app/trash` GUI screen | SPEC-0013 FR-NEW-012, 018 | done, implemented, not re-tracked in v2 queue |
| US-0007 — `admin.create_project` retention parameters | SPEC-0013 FR-NEW-014, 015 | done, implemented, not re-tracked in v2 queue |
| US-0008 — Contract regeneration & docs | SPEC-0013 FR-NEW-017 | done, implemented, not re-tracked in v2 queue |
| Drift (US-0002, discovered 2026-10-06): rename and `trash_entries` insert not in one transaction, bypassing the `MetaBackend` trait-object abstraction | SPEC-0013 §7 DEC-009 | resolved by explicit user acceptance (not code fix); the gap is documented as accepted behavior, with the real fix deferred — see FINDINGS FOR BACKLOG |
