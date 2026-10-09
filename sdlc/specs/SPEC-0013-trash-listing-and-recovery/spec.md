# mcp-fs trash listing and recovery — Specification Document

> Id: SPEC-0013
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0011; new id SPEC-0013 (collision avoidance)

## 1. Summary

`fs.delete` already soft-deletes into a trash directory, and `purge.rs` already auto-purges trashed files and projects on a configurable retention window. This spec adds the ability to **see** what is in the trash and **get it back**: `fs.trash_list`, `fs.trash_restore`, a `/app/trash` browser screen, and retention parameters on `admin.create_project`. A new `trash_entries` table records the original path, deletion time and deleter for every soft-deleted top-level path, replacing lossy on-the-fly reconstruction of the original path with an exact lookup.

## 2. Current State

### 2.1 Functional state prior to this spec

- `fs.delete(mount_id, path, recursive=false, trash=true)` soft-deletes by computing a flattened, timestamped trash destination (`/{trash_dir}/{epoch_ms}__{flattened_path}`) and renaming the node subtree there; `rename` rejects `ERR_NO_CLOBBER` on a destination collision, never silently overwriting.
- `NodeRow` carries no `deleted_at`, no `original_path`, no owner/author column — the only record of a soft-delete is the moved path itself.
- `purge.rs::sweep_project_files` auto-soft-deletes stale files past `file_retention_days` using the identical trash-path + rename mechanics, gated by the per-project `PurgeConfig { autopurge_enabled, use_internal_purge, file_retention_days, project_retention_days }` (default all-off).
- `PurgeConfig` was settable only after project creation, via `admin.set_purge_config`. `admin.create_project(project_id, owner)` took no purge parameters; a fresh project implicitly got `PurgeConfig::default()`.
- `state.authorize(mount_id, person)` is the single membership gate every `fs.*` tool uses; platform admin status does not bypass it. There is no per-file ownership anywhere in the schema.
- `deleted_projects_screen.rs` is the one existing browser screen with a trash-like shape, but for **projects**, not files.

### 2.2 Related specifications

None. No prior `specs/*` document covered trash, purge or soft-delete.

### 2.3 Test coverage prior to this spec

- `core/fs_ops.rs`: 6 tests on soft/hard delete, none asserting anything about a `trash_entries` table (it did not exist).
- `tools/lifecycle.rs`: 4 tests on `fs.delete`'s contract and trash-by-default behavior.
- `purge.rs`: 26 tests on sweep boundary conditions, gating flags, per-file isolation, idempotent re-purge.
- `tools/admin.rs`: 7 tests around `set_purge_config` and the deleted-projects countdown/undelete flow.
- Verified at spec time: no existing test asserted the absence of a `trash_entries` table or depended on the exact return shape of `delete_path`/`sweep_project_files` in a way a new side effect would break — this increment was purely additive.

## 3. Scope

### 3.1 In Scope
- A new `trash_entries` table (per volume) recording `original_path`, `deleted_at`, `deleted_by`, `size`, `kind` for every soft-deleted top-level path.
- `fs.trash_list` and `fs.trash_restore` MCP tools.
- A `/app/trash` browser screen, modeled on `deleted_projects_screen.rs`.
- Optional retention parameters on `admin.create_project`.
- Collision handling on both the write side (two deletes flattening to the same trash path) and the restore side (the original path is occupied again).
- Lazy, idempotent backfill of `trash_entries` for files soft-deleted before this feature shipped.

### 3.2 Out of Scope (Non-Goals)
- Trashed **projects** — already covered by `/app/deleted-projects` and `admin.undelete_project`.
- An on-demand "empty trash now" tool — the existing scheduled purge sweep already permanently removes trashed files past retention.
- File versioning or history beyond the single trash copy a soft-delete already keeps.
- Per-file ownership-based ACL restricting restore to the deleter (DEC-004).
- A dedicated `admin.get_purge_config` MCP tool (DEC-007).

## 4. Actors

- **Project member**: lists and restores trashed files in any project they belong to, via MCP tool or the `/app/trash` GUI.
- **Platform admin**: creates projects (unchanged `require_admin` gate), optionally with retention parameters set at creation time. Has no implicit file access beyond ordinary membership (DEC-005).
- **Background purge sweep** (`purge.rs`, unchanged trigger/gating logic): soft-deletes stale files and must keep `trash_entries` consistent when it does.

## 5. Usage Scenarios

### SC-001: List trashed files
Project member calls `fs.trash_list(mount_id, path_prefix="", limit=200, offset=0)`. The system backfills any untracked legacy trash node, then returns matching entries ordered by `deleted_at` descending, each carrying `trash_path, original_path, size, kind, deleted_at, deleted_by, purge_in_days`.
Exceptions: unknown `mount_id` → `ERR_PROJECT_NOT_FOUND`; non-member → `ERR_FORBIDDEN`; no match → empty `entries`, not an error; negative `limit`/`offset` → `ERR_INVALID_ARGUMENT`.

### SC-002: Restore a trashed file
Member calls `fs.trash_restore(mount_id, trash_path)`. The system resolves (backfilling if needed) `original_path`, recreates missing ancestor directories, renames the node back; on a destination collision it retries at `_restored`, `_restored2`, ... never failing on that account. The `trash_entries` row is deleted on success.
Exceptions: unknown `trash_path`/no node → `ERR_NOT_FOUND`; non-member → `ERR_FORBIDDEN`; empty `trash_path` → `ERR_INVALID_ARGUMENT`; 50 consecutive collisions → `ERR_INTERNAL_ERROR`. Restoring the same `trash_path` twice fails `ERR_NOT_FOUND` the second time (not a no-op): the first restore already deleted the row and moved the node.

### SC-003: Background purge sweep records what it trashes
`sweep_project_files` soft-deletes a stale file as today and additionally writes a `trash_entries` row with `deleted_by: null`. The file becomes listable/restorable via SC-001/SC-002 identically to a user-initiated delete. A same-destination collision within the sweep uses the same `~N` retry as SC-004.

### SC-004: Trash-path write collision
Two deletes flatten to the same destination, or two deletes land in the same millisecond. `rename` returns `ERR_NO_CLOBBER`; the system retries at `{dest}~1`, `{dest}~2`, ... Both files end up trashed at distinct paths. 50 exhausted attempts → `ERR_INTERNAL_ERROR` for that one file, with the original file remaining live and untouched. The retry fires only on `ERR_NO_CLOBBER`; any other error propagates immediately with no retry.

### SC-005: Create a project with retention pre-configured
Project owner or platform admin calls `admin.create_project` with the optional purge parameters; `PurgeConfig` is applied atomically at creation, exactly as if `admin.set_purge_config` had been called immediately after. Zero/negative retention values → `ERR_INVALID_ARGUMENT`. All four params omitted → unchanged existing behavior (`PurgeConfig::default()`).

### SC-006: Browse and restore trash via the GUI
`GET /app/trash?mount_id=<id>` renders the project's trash entries identically to `fs.trash_list`; a restore form posts to `/app/trash/restore` and redirects back on success. `GET /app/trash` with no `mount_id` renders a picker of the viewer's own project memberships. Missing/invalid CSRF on a cookie-authenticated POST → `403 FORBIDDEN`, no side effect. A `mount_id` the viewer is not a member of renders the same `ERR_FORBIDDEN` error page the underlying tool call raises, not an empty list. The screen never reimplements listing/restoring logic.

## 6. Functional Requirements

| ID | Requirement | Evidence |
|---|---|---|
| FR-NEW-001 | `trash_entries` table: one row per soft-deleted top-level path, columns `volume_id, trash_path (composite PK with volume_id), original_path, size, kind ('file'\|'dir'), deleted_at (RFC3339), deleted_by (nullable)`, implemented across sqlite/postgres/sqlserver dialects. | `crates/core/src/storage/rel/schema.rs`, `storage/rel/{sqlite,postgres,sqlserver}.rs` |
| FR-NEW-002 | WHEN `fs.delete` soft-deletes a path THE system SHALL insert a `trash_entries` row with `original_path`, `deleted_by` (caller), `size`/`kind` from the moved node. Hard-delete (`trash=false`) SHALL NOT write a row (FR-NEW-013). | `crates/core/src/core/fs_ops.rs` (`delete_path`) |
| FR-NEW-003 | WHEN `purge::sweep_project_files` soft-deletes a stale file THE system SHALL insert an identical row with `deleted_by: null`. | `crates/core/src/purge.rs` (`sweep_project_files`) |
| FR-NEW-004 | WHEN a rename into a computed trash destination returns `ERR_NO_CLOBBER` THE system SHALL retry at `~1`, `~2`, ... up to `~50`, via a single shared `rename_with_collision_retry` helper called by both `delete_path` and `sweep_project_files`. | `crates/core/src/core/fs_ops.rs` (`rename_with_collision_retry`) |
| FR-NEW-005 | IF 50 consecutive `~N` retries all return `ERR_NO_CLOBBER` THEN THE system SHALL return `ERR_INTERNAL_ERROR` and leave the original file live and untouched. | `crates/core/src/core/fs_ops.rs` |
| FR-NEW-006 | The system SHALL NOT retry a trash-destination rename on any error other than `ERR_NO_CLOBBER`. | `crates/core/src/core/fs_ops.rs` |
| FR-NEW-007 | WHEN `fs.trash_list(mount_id, path_prefix="", limit=200, offset=0)` is called THE system SHALL return `{"entries":[...], "total": N}`, each entry `{trash_path, original_path, size, kind, deleted_at, deleted_by, purge_in_days}`, filtered by `path_prefix`, ordered by `deleted_at` descending, paginated. Backfill (FR-NEW-011) runs first. | `crates/core/src/tools/trash.rs`, `crates/core/src/mcp/server.rs` |
| FR-NEW-008 | IF `limit` or `offset` is negative THEN THE system SHALL return `ERR_INVALID_ARGUMENT`. | `crates/core/src/tools/trash.rs` |
| FR-NEW-009 | WHEN `fs.trash_restore(mount_id, trash_path)` is called THE system SHALL resolve `original_path` (backfilling if needed), recreate missing ancestor directories, rename the node back, delete the `trash_entries` row, and return `{"restored_path": "...", "trash_path": "..."}`. | `crates/core/src/tools/trash.rs` |
| FR-NEW-010 | IF the restore destination already exists THEN THE system SHALL retry at `original_path + "_restored"`, `"_restored2"`, ..., appended after the full filename including extension, never failing `fs.trash_restore` due to this collision. | `crates/core/src/tools/trash.rs` |
| FR-NEW-011 | WHEN `fs.trash_list`/`fs.trash_restore` encounters a trash-directory node with no `trash_entries` row THE system SHALL insert one idempotently, reconstructing `original_path` by stripping the `{epoch_ms}__` prefix and replacing `__` with `/`, `deleted_at` from the epoch prefix (else now), `deleted_by: null`. Known limitation: a legacy `original_path` containing a literal `__` is ambiguous with one using `/`; this does not affect any delete performed after this spec (which always records `original_path` affirmatively). | `crates/core/src/tools/trash.rs` |
| FR-NEW-012 | The system SHALL expose `GET /app/trash?mount_id=<id>` and `POST /app/trash/restore`, modeled on `deleted_projects_screen.rs`'s identity/CSRF convention, delegating to the same `fs.trash_list`/`fs.trash_restore` logic — never a second implementation. CSRF enforced on the restore POST only when identity came from the cookie; an authorization failure renders an error page, not an empty list. | `crates/core/src/trash_screen.rs` |
| FR-NEW-013 | The system SHALL NOT write a `trash_entries` row when `fs.delete` is called with `trash=false`. | `crates/core/src/core/fs_ops.rs` |
| FR-NEW-014 | WHEN `admin.create_project` is called with any of `autopurge_enabled=false, use_internal_purge=false, file_retention_days=null, project_retention_days=null` THE system SHALL apply the resulting `PurgeConfig` atomically at creation, identical to an immediate `admin.set_purge_config` call. Omitting all four is byte-identical to prior behavior. | `crates/core/src/storage/admin.rs`, `crates/core/src/tools/admin.rs`, `crates/core/src/mcp/server.rs` |
| FR-NEW-015 | IF `file_retention_days`/`project_retention_days` is `0` or negative THEN THE system SHALL return `ERR_INVALID_ARGUMENT`, using the identical validation `admin.set_purge_config` already applies. | `crates/core/src/tools/admin.rs` |
| FR-NEW-016 | WHEN `fs.trash_list`/`fs.trash_restore` is called THE system SHALL authorize via `state.authorize(mount_id, person)` with no per-file ownership check. `deleted_by` is informational only and SHALL NOT gate restore. Platform admin status SHALL NOT grant implicit project access. | `crates/core/src/tools/trash.rs` |
| FR-NEW-017 | The system SHALL be implemented in the order: (1) schema; (2) shared collision-retry helper; (3) writers (`delete_path`, `sweep_project_files`); (4) readers (`fs.trash_list`, `fs.trash_restore`); (5) `/app/trash` screen; (6) `admin.create_project` params (independent, parallel-safe); (7) contract regeneration last. | process requirement, verified by build order and `cargo test --workspace` staying green at every step |
| FR-NEW-018 | IF `GET /app/trash` is requested with no `mount_id` THEN THE system SHALL render a picker of the viewer's own project memberships via `AdminBackend::list_projects_for(person)`, each linking to `GET /app/trash?mount_id=<id>`, instead of calling `fs.trash_list` or erroring. An empty membership list SHALL render an explicit empty-state message, HTTP 200. | `crates/core/src/trash_screen.rs`, `crates/core/src/storage/traits.rs` (`list_projects_for`) |

## 7. Non-Functional Requirements

- **Performance:** unchanged scale (internal-tool-scale load). `fs.trash_list` bounded by `limit` (default 200, no additional cap).
- **Security:** no new auth mechanism; reuses `state.authorize` and the existing header-then-cookie + CSRF convention verbatim. No secrets introduced.
- **Usability:** no new accessibility/i18n requirements beyond `deleted_projects_screen.rs`'s existing baseline (plain escaped HTML, no JS framework).
- **Reliability:** intended design was a single transaction spanning the `rename` and the `trash_entries` insert/delete (so neither commits without the other) — see §10 Migration notes in design.md for the accepted gap between this intent and the shipped implementation.
- **Observability:** `fs.trash_list`/`fs.trash_restore` traced at INFO; the `trash_entries` write/delete at DEBUG, matching existing `fs.delete` tracing levels. No credentials or PII involved.
- **Deployment/Scalability:** unchanged deployment target; `trash_entries` indexed by `(volume_id, trash_path)` primary key, with a secondary `(volume_id, deleted_at)` index supporting the required ordering.

## 8. E2E Tests

63 tests (`E2E-NEW-401` through `E2E-NEW-463`), all implemented, all passing at spec completion. Coverage by category: happy 14, failure 17, edge 25, side effects 4, idempotency 2, state transition 1 explicit (+2 implicit). Failure tests (17) outnumber happy-path tests (14), satisfying the project's sufficiency rule.

| Scenario | FRs | Representative tests |
|---|---|---|
| SC-001 (list) | FR-NEW-007, 008, 016 | 406 (happy, ordering), 407/408 (unknown project/forbidden), 409/411 (empty/no-match), 410 (pagination boundary), 412 (prefix filter), 451/452 (limit/offset validation) |
| SC-002 (restore) | FR-NEW-009, 010, 016 | 417/423 (happy), 418/419 (not found/forbidden), 420-422 (collision retry suffixes), 425 (idempotent backfill), 436/437/439 (state transition, double-restore) |
| SC-003 (sweep records) | FR-NEW-001, 003 | 403 (happy, `deleted_by=null`), 440 (side effect), 445 (failed insert rolls back rename), 457 (no-op sweep writes nothing) |
| SC-004 (write collision) | FR-NEW-004, 005, 006 | 404 (retry at `~1`), 405 (50 exhausted), 441/449 (unrelated error aborts without retry), 443/444 (happy/49th-attempt boundary), 447/448 (sweep-side exhaustion, non-permanent) |
| SC-005 (create with retention) | FR-NEW-014, 015 | 429 (happy), 430/459 (partial params don't implicitly enable autopurge), 431/460/461 (zero/negative rejected) |
| SC-006 (GUI) | FR-NEW-012, 018 | 432/462 (happy render/picker), 433/434 (GUI failure/ACL), 455 (empty state), 463 (zero-membership picker) |
| cross | FR-NEW-001, 002, 011, 013 | 401 (delete writes row), 402/458 (deleted_by/kind+size correctness), 413/414 (backfill + idempotency), 424 (restore needs backfill), 446 (one row per directory delete), 453/454/456 (hard delete writes nothing, including adjacency to unrelated prior entries) |

Exact test command: `cargo test --workspace`.

## 9. Glossary

| Term | Definition |
|---|---|
| Trash entry | A `trash_entries` row tracking one soft-deleted top-level path's original location, deletion time and deleter |
| Flatten | Replacing every `/` with `__` when building a trash destination from an original path |
| Backfill | The lazy, idempotent insertion of a `trash_entries` row for a trash-directory node that predates this feature |
| Purge sweep | The existing scheduled process (`purge.rs`) that permanently removes trashed files/projects past retention |
| `PurgeConfig` | The per-project struct controlling auto-purge gating and retention windows |

## 10. Confidence notes

- This is a migration of an already-implemented, already-archived legacy spec (legacy id SPEC-0011, pre-existing collision with another legacy-numbered SPEC-0011 for "full-git-dev-process"). The functional content (summary, scope, scenarios, FRs, decisions) is carried over verbatim from the archived spec.md and its stories/_index.md, which confirm all 8 implementing stories (US-0001..US-0008) as `Status: done`.
- Code-location citations in the Evidence column point to the modules the legacy spec and its stories named as the implementation sites; this migration did not re-verify every line number against current `HEAD`, since the archived document and its drift record (dated 2026-10-06, close to this migration) are the authoritative as-built record for that work, not a design still in progress.
- One known gap between stated intent and shipped behavior exists: §7 Reliability describes the originally-specified single-transaction guarantee for the rename + `trash_entries` write, which a drift finding during US-0002 implementation (2026-10-06) found was not achievable through the existing `MetaBackend` trait-object boundary without widening that trait. The user explicitly accepted the resulting best-effort-sequential write as shipped behavior rather than requiring a fix in this spec's scope; see design.md §7 Decisions (DEC-009) and §9 Legacy mapping for the full trace, and `backlog/BL-0011_atomic-trash-entry-write.md` for the deferred follow-up.
