# mcp-fs-rust — Design Document: Auto-Purge Files and Stale Projects

> Id: SPEC-0017 | Status: implemented | Migrated from legacy SPEC-0014

## 1. Components

- `purge.rs` — shared sweep engine (`sweep_project_files`, `sweep_project`,
  `sweep_grace_period`, `run_cycle`), the single entry point called by both the
  background loop (`app.rs`) and the CLI `purge` verb.
- `storage/rel/schema.rs` — `project.deleted_at` column, `project_purge_config`
  table/columns.
- `storage/admin.rs` — `require_member` gate extended to reject calls against a
  soft-deleted project; new queries backing `set_purge_config`,
  `list_deleted_projects`, `undelete_project`.
- `storage/meta.rs` — atime-update query, invoked from the detached best-effort bump
  helper.
- `core/fs_ops.rs` — wiring of the async atime/mtime bump into read and write
  operations.
- `tools/admin.rs` — three new tool functions: `set_purge_config`,
  `list_deleted_projects`, `undelete_project`.
- `mcp/server.rs` — three new `#[tool]` methods delegating to the above.
- `cli.rs` — new `Purge` verb (`--project`, `--on-demand`).
- `app.rs` — spawns the background sweep loop at boot, stops it on shutdown; mounts the
  new browser screen.
- `deleted_projects_screen.rs` — `/app/deleted-projects` browser screen, modeled on
  `token_screen.rs` (session-cookie + CSRF).
- `config.rs` — two new `SafetyConfig` fields: `purge_interval_secs`,
  `project_purge_grace_days`.

## 2. Flows

1. **Read/write access-time bump:** a content read or write completes successfully →
   `core::fs_ops` fires a detached, best-effort task → `storage/meta.rs` updates
   `atime` (write ops also update `mtime`) → failure is logged, never surfaces to the
   caller.
2. **Internal sweep cycle:** background loop wakes on `purge_interval_secs` →
   `purge::run_cycle` → per qualifying project: file sweep (FR-NEW-007) → project
   soft-delete sweep (FR-NEW-008, computed over the post-file-sweep live set) →
   unconditional grace-period sweep (FR-NEW-012) over every soft-deleted project.
3. **CLI on-demand purge:** `mcp-fs purge --project <id> [--on-demand]` → gate check
   (`autopurge_enabled` or `--on-demand`) → same `purge::run_cycle` logic invoked
   synchronously for that project → summary printed. No `--project` → global grace-period
   sweep only.
4. **Admin configures/soft-deletes/undeletes:** `admin.set_purge_config` /
   `admin.list_deleted_projects` / `admin.undelete_project` → `storage/admin.rs`,
   independent of the sweep loop; configuring never triggers a purge as a side effect.
5. **Blocked access:** any `fs.*`/`git.*` call (MCP or REST) → `state.rs:59
   authorize()` → `storage/admin.rs require_member` → `deleted_at IS NOT NULL` →
   not-found, uniformly on both surfaces since they share this one gate.

## 3. Interfaces

- `admin.set_purge_config(project_id, autopurge_enabled: bool, use_internal_purge:
  bool, file_retention_days: Option<u32>, project_retention_days: Option<u32>)`
- `admin.list_deleted_projects() -> [{project_id, owner, deleted_at,
  days_until_permanent_removal}]`
- `admin.undelete_project(project_id)`
- CLI: `mcp-fs purge [--project <id>] [--on-demand]`
- `/app/deleted-projects` (GET: list + undelete form; session-cookie + CSRF, same
  convention as `/app/tokens`)
- No REST route added for the three new admin tools: no existing `admin.*` tool has a
  REST equivalent, and this spec does not start that convention. The browser screen
  calls the tool functions in-process.

## 4. Data and state

- `project.deleted_at: Option<Timestamp>` — new nullable column, added via the existing
  idempotent `column_migration()` path (same technique as `project.index_mode`).
- `project_purge_config(project_id PK/FK, autopurge_enabled: bool, use_internal_purge:
  bool, file_retention_days: Option<u32>, project_retention_days: Option<u32>)` — new
  table/columns, implementation's choice of shape, declared once in
  `storage/rel/schema.rs` per the dialect checklist.
- `nodes.atime` — existing column, now actually written; no shape change.
- `SafetyConfig.purge_interval_secs: i64` (default `3600`),
  `SafetyConfig.project_purge_grace_days: i64` (default `30`).
- The `volume_id`-in-every-key convention does NOT apply to `project` /
  `project_purge_config`: the admin registry is global to the deployment and carries no
  `volume_id` column at all. Both key off `project_id` alone, like `project_member`.

## 5. Configuration

`safety.purge_interval_secs` (default `3600`) and `safety.project_purge_grace_days`
(default `30`), both new fields on `SafetyConfig`. No feature flag: every new column
defaults to inactive (`autopurge_enabled = false` / `deleted_at = NULL`), so existing
projects are unaffected until an admin opts in.

## 6. Observability

Sweep start/summary logged at INFO, each individual file/project mutation at DEBUG,
failures at ERROR — via `tracing`, consistent with the rest of the codebase. No
OpenTelemetry collector; this spec did not introduce one.

## 7. Decisions

- **DEC-001:** `atime` means true last-access (every content read), and every write also
  bumps it alongside `mtime`. Implements FR-NEW-001, FR-NEW-002.
- **DEC-002:** Dual trigger — internal background loop and standalone `mcp-fs purge` CLI
  verb, selectable per project via `use_internal_purge`. Implements FR-NEW-009,
  FR-NEW-010, FR-NEW-011.
- **DEC-003:** CLI purge on a project not configured for autopurge fails unless
  `--on-demand` is passed. Implements FR-NEW-011.
- **DEC-004:** Project staleness reference = `max(project.created_at, max(atime) over
  live files)`, never an independent project-level activity timestamp. Implements
  FR-NEW-008.
- **DEC-005:** `grep` counts as a content access (bumps `atime`); `stat`/`glob`/`list`/
  `tree` do not. Implements FR-NEW-001, FR-NEW-003.
- **DEC-006:** File-level purge reuses the existing trash mechanism as-is, single phase —
  no new per-file retention clock in this spec (that remains BL-0003's unspec'd scope).
  Implements FR-NEW-007.
- **DEC-007:** Project soft-delete is two-phase (soft-delete, reversible → grace period →
  permanent removal), with its own grace-period clock distinct from BL-0003's file-trash
  retention. Implements FR-NEW-008, FR-NEW-012, FR-NEW-015.
- **DEC-008:** Grace period is one global config value, not per-project. Implements
  FR-NEW-018.
- **DEC-009:** Reuse the existing `ERR_PROJECT_NOT_FOUND` code for calls against a
  soft-deleted project rather than minting a new error code; the error set is frozen.
  Implements FR-NEW-013.
- **DEC-010:** `atime`/`mtime` updates are async, best-effort (fire-and-forget), matching
  the search indexer's existing detached-task idiom, to avoid write-amplifying the most
  frequent operation in the system. Implements FR-NEW-001, FR-NEW-002.
- **DEC-011:** Soft-deleted state is a nullable `project.deleted_at` column added via the
  existing idempotent `column_migration()` path, not a separate table, matching the
  `project.index_mode` precedent. Implements FR-NEW-008.
- **DEC-012:** `admin.list_deleted_projects()` is membership-filtered per caller
  (platform admin sees all), matching the verified convention of `admin.list_projects`,
  not platform-admin-only. Implements FR-NEW-014.
- **DEC-013:** Concurrency between the internal loop and CLI on-demand purge on the same
  project needs no new locking primitive: file soft-delete is naturally idempotent, and
  project soft-delete uses a conditional `UPDATE ... WHERE deleted_at IS NULL`.
  Implements FR-NEW-007, FR-NEW-008.

## 8. Requirement to code map

| Requirement | Story | Status |
|---|---|---|
| (drift) detached best-effort DB-write helper | US-0001 | done |
| FR-NEW-018 | US-0002 | done |
| FR-NEW-001, 002, 003 | US-0003 | done |
| FR-NEW-013 | US-0004, US-0014 | done |
| FR-NEW-007 | US-0005 | done |
| FR-NEW-008 | US-0006 | done |
| FR-NEW-009, 010, 011 | US-0007 | done |
| FR-NEW-012 | US-0008 | done |
| FR-NEW-004, 005, 006 | US-0009, US-0013 | done |
| FR-NEW-014 | US-0010 | done |
| FR-NEW-015, 016 | US-0011 | done |
| FR-NEW-017 | US-0012 | done |

## 9. Legacy mapping

Source: specs/archived/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md (pre-move, renumbered to SPEC-0017)

| Old id | New id | Note |
|---|---|---|
| SPEC-0014 | SPEC-0017 | Collision avoidance renumbering; content unchanged in substance |
| DRIFT-001 (carried as US-0001) | — | resolved via US-0001, see converge gap stories US-0013/US-0014 |
| FR-NEW-001..018 | FR-NEW-001..018 (verbatim ids) | No renumbering of FR ids; all retained as-is |
| E2E-NEW-001..080 | E2E-NEW-001..080 (verbatim ids) | No renumbering; all retained as-is |
| US-0001..US-0014 | US-0001..US-0014 (verbatim ids) | All status `done`, no renumbering |
