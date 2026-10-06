# mcp-fs trash listing and recovery — Specification Document

> Generated on: 2026-10-05
> Id: SPEC-0011
> Nature: FEAT
> Depth: L
> Depth evidence: ≥3 modules touched (tools/, storage/ schema+traits across 3 dialects, a new GUI screen module, admin.rs), new persisted schema (`trash_entries`), >15 anticipated requirements.
> Status: Draft
> Type: Evolution Specification
> From backlog: BL-0003
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Executive Summary

`fs.delete` already soft-deletes into a trash directory, and `purge.rs` already auto-purges trashed
files and projects on a configurable retention window. What is missing is any way to **see** what is
in the trash or **get it back**: no MCP tool and no GUI screen exist for listing or restoring trashed
files, and a project's retention window can only be set after creation, never at creation time. This
increment adds `fs.trash_list`, `fs.trash_restore`, a `/app/trash` browser screen, and retention
parameters on `admin.create_project`, by introducing a new `trash_entries` table that records the
original path, deletion time and deleter for every soft-deleted top-level path, replacing lossy
on-the-fly reconstruction of the original path with an exact lookup.

## 2. Current State

### 2.1 How it works today

- `fs.delete(mount_id, path, recursive=false, trash=true)` (`crates/core/src/tools/lifecycle.rs`,
  schema frozen at `TOOL_CONTRACT.txt:101-105`) calls `core::fs_ops::delete_path`
  (`crates/core/src/core/fs_ops.rs:988-1035`). With `trash=true` (the default) and the path existing,
  it computes `dst = safety.trash_path(norm)` (`crates/core/src/safety.rs:177-182`): `trash_path`
  builds `/{trash_dir}/{epoch_ms}__{flattened_path}`, where `flattened_path` is
  `path.trim_matches('/').replace('/', "__")` and `epoch_ms` is `(now_unix() * 1000.0) as i64`.
  `trash_dir` defaults to `.mcp_trash` (`crates/core/src/config.rs`, confirmed by
  `safety.rs:393-399`'s `trash_path_flattens_and_timestamps` test, with the configured-dir variant
  at `safety.rs:401-405`). It then `makedirs`s the trash parent and calls
  `client.rename(norm, &dst)` (`storage/meta.rs:701`), which moves the whole node subtree (confirmed
  by `fs_ops.rs:2815`, `soft_delete_of_a_directory_moves_the_whole_subtree`) — a genuine `path` column
  rewrite in the `nodes` table, not a flag. `MetaBackend::rename` rejects with `ERR_NO_CLOBBER`
  (`storage/traits.rs:173`, enforced at `storage/meta.rs:716-718`) if the destination path already
  exists; it never silently overwrites.
- `NodeRow` (`storage/traits.rs:13-26`) carries `path, parent, name, kind, size, mode, mtime, ctime,
  atime, sha256` — **no `deleted_at`, no `original_path`, no owner/author column**. The only state a
  soft-delete leaves behind is the moved `path` itself; nothing records who deleted it or what its
  original path was, beyond what can be (lossily) reverse-engineered from the trash path string.
- `purge.rs::sweep_project_files` (`crates/core/src/purge.rs:30-69`) auto-soft-deletes files whose
  `atime` is older than `now - file_retention_days*86400`, using the identical `trash_path` + `rename`
  mechanics, gated by the per-project `PurgeConfig { autopurge_enabled, use_internal_purge,
  file_retention_days: Option<i64>, project_retention_days: Option<i64> }`
  (`storage/traits.rs:119-124`, `Default` is all-off/`None`, per the doc comment at
  `storage/traits.rs:116-118`).
- `PurgeConfig` is settable only **after** project creation, via the MCP tool `admin.set_purge_config`
  (`tools/admin.rs:246-289`, schema at `TOOL_CONTRACT.txt:61-65`): params `project_id:string,
  autopurge_enabled:boolean, use_internal_purge:boolean, file_retention_days:number|null,
  project_retention_days:number|null`, required
  `['project_id','autopurge_enabled','use_internal_purge']`. Its handler (`tools/admin.rs:563-589`)
  rejects `file_retention_days`/`project_retention_days` of `0` or negative with
  `ERR_INVALID_ARGUMENT` (`tools/admin.rs:572-578`). There is **no `admin.get_purge_config` MCP
  tool** — only the internal `AdminBackend` trait method.
- `admin.create_project(project_id:string, owner:string)` (`storage/admin.rs:171`, tool schema
  `tools/admin.rs:62-78`, `TOOL_CONTRACT.txt:6-8`) takes no purge parameters; a fresh project
  implicitly gets `PurgeConfig::default()`.
- `state.authorize(mount_id, person)` (`crates/core/src/app.rs` / `state.rs:59-61`) calls
  `admin.require_member(mount_id, person)` — the single membership gate every `fs.*` tool uses.
  Platform admin status does **not** bypass it (`state.rs:57-58`: "managing the platform is not the
  same as reading data"). There is **no per-file ownership** anywhere: no node column records who
  wrote or deleted a specific file; the closest thing is `SafetyManager`'s in-memory,
  non-persisted, per-session audit log (`safety.rs:162-176`, capped at 500 entries,
  `safety.rs:14`).
- `crates/core/src/deleted_projects_screen.rs` is the one existing browser screen with a trash-like
  shape, but for **projects**, not files: `GET /app/deleted-projects` and
  `POST /app/deleted-projects/undelete` (`deleted_projects_screen.rs:41-47`), a screen-scoped
  `CsrfStore`, identity resolved header-then-cookie (`resolve_person`), CSRF enforced only when the
  identity source was the cookie (`deleted_projects_screen.rs:167-178`), raw hand-built HTML with
  `escape_html` on every interpolated value. `list_deleted` (`deleted_projects_screen.rs:130-141`)
  silently filters to projects the viewer is a member of (or all, for a platform admin) — it does not
  401/403 on a project the viewer cannot see, because it lists **across every deleted project**, not
  one named project. `admin.list_deleted_projects` (`tools/admin.rs:403-424`) is the data source: for
  each soft-deleted project it computes `days_until_permanent_removal = grace_days - floor((now -
  deleted_unix)/86400)` (can be negative) against `deleted_at` stored/returned as an RFC3339 string.

### 2.2 Existing specifications governing this area

No `specs/*` document covers trash, purge or soft-delete today — `purge.rs`'s own doc comments
reference "SPEC-0014" and "US-0005"/"US-0011"/"US-0012" but no `specs/SPEC-0014*` directory exists in
this repository (it predates the `specs/` convention currently in use, or was implemented before this
tooling). This document does not modify anything; it only adds.

### 2.3 Existing test coverage

- `crates/core/src/core/fs_ops.rs`: `delete_soft_moves_into_the_trash` (line 2689),
  `hard_delete_is_refused_unless_configured` (2702), `hard_delete_works_when_allowed` (2711),
  `delete_a_directory_needs_recursive` (2726), `delete_a_missing_path_is_not_found` (2735),
  `soft_delete_of_a_directory_moves_the_whole_subtree` (2815) — **6 tests**, none asserting
  anything about a `trash_entries` table (it doesn't exist yet).
- `crates/core/src/tools/lifecycle.rs`: `fs_delete_schema_matches_the_contract` (230),
  `delete_moves_to_trash_by_default` (303), `a_hard_delete_needs_the_server_flag` (314),
  `delete_a_directory_needs_recursive` (334) — 4 tests.
- `crates/core/src/purge.rs`: **26 tests** (recounted directly: `e2e_new_024` through
  `e2e_new_039` and `unconfigured_project_is_a_no_op` at `purge.rs:293-410`,
  `run_cycle_sweeps_every_project_and_aggregates_counts` and
  `run_cycle_on_zero_projects_is_a_zero_count_no_op` at `purge.rs:538-573`, `e2e_new_079` at
  `purge.rs:585`, `e2e_new_053` through `e2e_new_057` at `purge.rs:658-703`, and `e2e_new_080` at
  `purge.rs:857`; the production `run_cycle` function itself at `purge.rs:151` is excluded),
  covering boundary conditions, gating flags, per-file isolation and idempotent re-purge.
- `crates/core/src/tools/admin.rs`: **7 tests** around `set_purge_config`
  (`admin.rs:1411, 1422, 1460, 1485, 1513, 1539` and `zero_retention_values_are_rejected` at
  `admin.rs:1563`), covering schema, happy path owner/platform-admin, forbidden, not-found and
  boundary cases, plus
  the deleted-projects countdown and undelete tests.
- Exact test command: `cargo test --workspace` (per `AGENTS.md`).
- **Verified: no existing test asserts the absence of a `trash_entries` table, an exact row count a
  new insert would change, or the exact return shape of `delete_path`/`sweep_project_files` in a way a
  new side effect (writing a `trash_entries` row) would break.** This increment is purely additive;
  confirmed by independent review — see Phase 4 test design sub-agent findings.

## 3. Scope

### 3.1 In Scope
- A new `trash_entries` table (per volume) recording `original_path`, `deleted_at`, `deleted_by`,
  `size`, `kind` for every soft-deleted top-level path.
- `fs.trash_list` and `fs.trash_restore` MCP tools.
- A `/app/trash` browser screen, modeled on `deleted_projects_screen.rs`.
- Optional retention parameters on `admin.create_project`.
- Collision handling on both the write side (two deletes flattening to the same trash path) and the
  restore side (the original path is occupied again).
- Lazy, idempotent backfill of `trash_entries` for files soft-deleted before this feature shipped.

### 3.2 Out of Scope (Non-Goals)
- Trashed **projects** — already covered by the existing `/app/deleted-projects` screen and
  `admin.undelete_project` tool.
- An on-demand "empty trash now" tool — the existing scheduled purge sweep (`purge.rs`) already
  permanently removes trashed files past retention; no new on-demand trigger is requested.
- File versioning or history beyond the single trash copy a soft-delete already keeps.
- Per-file ownership-based ACL restricting restore to the deleter — rejected by DEC-004 below.
- A dedicated `admin.get_purge_config` MCP tool — not needed by any requirement in this document
  (`fs.trash_list` computes `purge_in_days` server-side from the existing internal config read).

## 4. User Personas & Actors

- **Project member**: lists and restores trashed files in any project they belong to, via MCP tool or
  the `/app/trash` GUI.
- **Platform admin**: creates projects (the existing `admin.create_project` gate,
  `require_admin`, `mcp/server.rs:2084`, is unchanged by this document), optionally with retention
  parameters set at creation time. Has no implicit file access beyond ordinary membership
  (DEC-005).
- **Background purge sweep** (`purge.rs`, unchanged): the system actor that soft-deletes stale files
  and must keep `trash_entries` consistent when it does.

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| Trash tracking | Recording and querying what has been soft-deleted and from where | `trash_entries` row |
| File lifecycle | The existing soft-delete/hard-delete/purge mechanics `fs.delete` and `purge.rs` already own | `NodeRow`, `PurgeConfig` |
| Project provisioning | Creating a project and its initial purge configuration | `Project`, `PurgeConfig` |
| Browser surface | The `/app/trash` screen, session identity and CSRF | `ScreenState`, `CsrfStore` |

## 5. Usage Scenarios

### SC-001: List trashed files
**Actor:** project member.
**Preconditions:** the member is authorized for `mount_id` (`state.authorize`).
**Flow:**
1. The member calls `fs.trash_list(mount_id, path_prefix="", limit=200, offset=0)`.
2. The system backfills (idempotently) any trash-directory node lacking a `trash_entries` row, then
   returns matching entries ordered by `deleted_at` descending, each carrying `trash_path,
   original_path, size, kind, deleted_at, deleted_by, purge_in_days`.
**Postconditions:** no state change beyond the backfill write (which only ever adds rows for
previously-untracked legacy trash nodes, never removes or alters existing ones).
**Exceptions:**
- EXC-001a: `mount_id` does not exist → `ERR_PROJECT_NOT_FOUND`.
- EXC-001b: caller is not a project member → `ERR_FORBIDDEN`.
- EXC-001c: `path_prefix` matches nothing → empty `entries`, not an error.
- EXC-001d: `limit`/`offset` negative → `ERR_INVALID_ARGUMENT`.
**Cross-scenario notes:** entries created by the background purge sweep (SC-003) appear identically,
with `deleted_by: null`.

### SC-002: Restore a trashed file
**Actor:** project member.
**Preconditions:** a `trash_entries` row exists for the given `trash_path` (directly, or backfillable
from an untracked legacy node).
**Flow:**
1. The member calls `fs.trash_restore(mount_id, trash_path)`.
2. The system looks up (backfilling if needed) the entry's `original_path`, recreates any missing
   ancestor directories, and renames the node back.
3. If `original_path` is occupied, the system never fails: it retries at `original_path + "_restored"`,
   then `"_restored2"`, `"_restored3"`, ... until free.
4. The `trash_entries` row is deleted on success.
**Postconditions:** the file (or whole directory subtree) is live again at the resolved path; the
`trash_entries` row is gone.
**Exceptions:**
- EXC-002a: `trash_path` is unknown and no node exists there → `ERR_NOT_FOUND`.
- EXC-002b: caller is not a project member → `ERR_FORBIDDEN`.
- EXC-002c: `trash_path` is empty/whitespace → `ERR_INVALID_ARGUMENT`.
- EXC-002d: 50 consecutive collision retries exhausted → `ERR_INTERNAL_ERROR`.
**Cross-scenario notes:** restoring twice on the same `trash_path` fails EXC-002a the second time
(not a silent no-op): the first restore already deleted the row and moved the node.

### SC-003: Background purge sweep records what it trashes
**Actor:** background purge sweep (`purge.rs::sweep_project_files`, unchanged trigger/gating logic).
**Preconditions:** `PurgeConfig.autopurge_enabled && use_internal_purge && file_retention_days.is_some()`.
**Flow:** 1. The sweep soft-deletes a stale file exactly as today. 2. It additionally writes a
`trash_entries` row with `deleted_by: null`.
**Postconditions:** the file is listable/restorable via SC-001/SC-002 identically to a user-initiated
delete.
**Exceptions:** EXC-003a: the trash-path write collides (two stale files flattening to the same
destination within the sweep) → same `~1`/`~2` retry as SC-004.
**Cross-scenario notes:** none beyond SC-001's.

### SC-004: Trash-path write collision
**Actor:** system (triggered by any `fs.delete` or sweep operation).
**Preconditions:** the computed trash destination already exists (two different original paths
flatten to the same name, or two deletes land in the same millisecond).
**Flow:** 1. `rename` to the trash destination returns `ERR_NO_CLOBBER`. 2. The system retries at
`{dest}~1`, `{dest}~2`, ... 3. On success, the `trash_entries` row records the actual (possibly
suffixed) `trash_path`.
**Postconditions:** both original files end up trashed at distinct paths; neither is lost.
**Exceptions:** EXC-004a: 50 attempts exhausted → `ERR_INTERNAL_ERROR`, the delete/sweep operation
fails for that one file, the original file remains live and untouched.
**Cross-scenario notes:** the retry fires **only** on `ERR_NO_CLOBBER`; any other error from `rename`
propagates immediately with no retry (FR-NEW-006).

### SC-005: Create a project with retention pre-configured
**Actor:** project owner or platform admin.
**Preconditions:** none (greenfield project creation).
**Flow:** 1. The actor calls `admin.create_project` with the optional purge parameters. 2. The project
is created with that `PurgeConfig` applied atomically at creation, instead of the default.
**Postconditions:** `PurgeConfig` for the new project matches what was requested, exactly as if
`admin.set_purge_config` had been called immediately after creation with the same values.
**Exceptions:**
- EXC-005a: `file_retention_days`/`project_retention_days` is `0` or negative → `ERR_INVALID_ARGUMENT`
  (same validation as `admin.set_purge_config`).
- EXC-005b: all four params omitted → unchanged existing behavior (`PurgeConfig::default()`).
**Cross-scenario notes:** none.

### SC-006: Browse and restore trash via the GUI
**Actor:** project member, browser session.
**Preconditions:** authenticated per the existing header/cookie + CSRF convention.
**Flow:** 1. `GET /app/trash?mount_id=<id>` renders the member's own trashable projects as a picker
and, for the selected project, the same entries `fs.trash_list` would return. 2. A restore form per
row posts to `/app/trash/restore`. 3. On success, the page redirects back with the row gone.
**Postconditions:** identical to SC-002, reached through the browser instead of the MCP tool, via the
same underlying function.
**Exceptions:**
- EXC-006a: missing/invalid CSRF token on a cookie-authenticated POST → `403 FORBIDDEN`, no restore
  side effect.
- EXC-006b: `mount_id` the viewer is not a member of → the screen renders the same `ERR_FORBIDDEN`
  the underlying tool call raises (an error page), not an empty list.
**Cross-scenario notes:** never reimplements listing/restoring (FR-NEW-012).

## 6. Functional Requirements

### New Requirements

#### FR-NEW-001 [EARS-U]: `trash_entries` table
> The system SHALL persist one `trash_entries` row per soft-deleted top-level path, with columns
> `volume_id, trash_path (primary key together with volume_id), original_path, size, kind ('file' |
> 'dir'), deleted_at (RFC3339 string), deleted_by (nullable string)`, added to `storage/rel/schema.rs`
> and implemented across `storage/rel/sqlite.rs`, `storage/rel/postgres.rs`, `storage/rel/sqlserver.rs`
> per the dialect checklist in `.agent_docs/backends.md`.

- **Exact names:** table `trash_entries`; columns as above.
- **Priority:** Must-have.

#### FR-NEW-002 [EARS-E]: `fs.delete` writes a trash entry
> WHEN `fs.delete` soft-deletes a path (the `trash=true` branch of `core::fs_ops::delete_path`) THE
> system SHALL insert a `trash_entries` row with `original_path` equal to the pre-delete path,
> `deleted_by` equal to the calling person, `size`/`kind` taken from the moved node, in the same
> transaction as the `rename`.

- **Business Rules:** the hard-delete branch (`trash=false`) SHALL NOT write a `trash_entries` row
  (FR-NEW-013 states this explicitly as a prohibition).
- **Priority:** Must-have.
- **Rationale:** DEC-001.

#### FR-NEW-003 [EARS-E]: `sweep_project_files` writes a trash entry
> WHEN `purge::sweep_project_files` soft-deletes a stale file THE system SHALL insert a
> `trash_entries` row identical in shape to FR-NEW-002's, with `deleted_by` set to `null`.

- **Priority:** Must-have.
- **Rationale:** DEC-001; SC-003.

#### FR-NEW-004 [EARS-E]: trash-path write collision retry
> WHEN a `rename` into a computed trash destination returns `ERR_NO_CLOBBER` THE system SHALL retry
> the same operation at the destination suffixed with `~1`, then `~2`, incrementing up to `~50`, until
> the rename succeeds.

- **Business Rules:** the retry loop SHALL only trigger on `ERR_NO_CLOBBER`; any other error from
  `rename` SHALL propagate immediately without retry (FR-NEW-006).
- **Exact names:** implemented as `pub(crate) async fn rename_with_collision_retry(client: &VolumeClient, src: &str, dst: &str) -> Result<String>` (returning the final, possibly-suffixed destination path) in `crates/core/src/core/fs_ops.rs`, called directly by `delete_path` and by `purge::sweep_project_files` — one implementation, two callers, per this project's own convention of never duplicating an operation between call sites.
- **Priority:** Must-have.
- **Rationale:** DEC-002; SC-004.

#### FR-NEW-005 [EARS-O]: collision retries exhausted
> IF 50 consecutive `~N` retries all still return `ERR_NO_CLOBBER` THEN THE system SHALL return
> `ERR_INTERNAL_ERROR` and leave the original file live and untouched.

- **Priority:** Must-have.
- **Rationale:** DEC-002; SC-004/EXC-004a.

#### FR-NEW-006 [EARS-UB]: no retry on unrelated errors
> The system SHALL NOT retry a trash-destination rename when the underlying error is anything other
> than `ERR_NO_CLOBBER`.

- **Priority:** Must-have.
- **Rationale:** closes test gap identified during Phase 4 design (E2E-NEW-441).

#### FR-NEW-007 [EARS-E]: `fs.trash_list`
> WHEN `fs.trash_list(mount_id:string, path_prefix:string="", limit:integer=200, offset:integer=0)`
> is called THE system SHALL return `{"entries": [...], "total": N}`, where each entry is
> `{trash_path, original_path, size, kind, deleted_at, deleted_by, purge_in_days}`, filtered to
> entries whose `original_path` starts with `path_prefix`, ordered by `deleted_at` descending, and
> paginated by `limit`/`offset`.

- **Inputs:** `mount_id:string` (required), `path_prefix:string="" `, `limit:integer=200`,
  `offset:integer=0`.
- **Outputs:** as above. `purge_in_days` is `file_retention_days - floor((now - deleted_at)/86400)`
  when the project's `PurgeConfig.file_retention_days` is set (can be negative), else `null`.
- **Business Rules:** authorization via `state.authorize(mount_id, person)`, identical to every other
  `fs.*` tool. Before building the response, the system runs the backfill of FR-NEW-011 for every
  untracked node directly under the project's `trash_dir`.
- **Exact names:** tool `fs.trash_list`; response keys `entries`, `total`, and the per-entry keys
  listed above; error codes `ERR_PROJECT_NOT_FOUND`, `ERR_FORBIDDEN`, `ERR_INVALID_ARGUMENT`.
- **Priority:** Must-have.
- **Rationale:** DEC-003; SC-001.

#### FR-NEW-008 [EARS-O]: `fs.trash_list` input validation
> IF `limit` or `offset` is negative THEN THE system SHALL return `ERR_INVALID_ARGUMENT`.

- **Priority:** Must-have.
- **Rationale:** SC-001/EXC-001d.

#### FR-NEW-009 [EARS-E]: `fs.trash_restore`
> WHEN `fs.trash_restore(mount_id:string, trash_path:string)` is called THE system SHALL resolve the
> entry's `original_path` (backfilling per FR-NEW-011 if the entry is untracked but the node exists),
> recreate any missing ancestor directories of the restore destination, rename the node back, delete
> the `trash_entries` row, and return `{"restored_path": "<final path>", "trash_path": "<input>"}`.

- **Inputs:** `mount_id:string`, `trash_path:string` (both required).
- **Outputs:** `{"restored_path": string, "trash_path": string}`.
- **Business Rules:** authorization via `state.authorize(mount_id, person)`. Ancestor directories of
  the restore destination are created via the same `makedirs(parent, true)` pattern
  `delete_path` already uses for the trash side (`fs_ops.rs:1019-1025`).
- **Exact names:** tool `fs.trash_restore`; error codes `ERR_NOT_FOUND`, `ERR_FORBIDDEN`,
  `ERR_INVALID_ARGUMENT`, `ERR_INTERNAL_ERROR`.
- **Priority:** Must-have.
- **Rationale:** DEC-003; SC-002.

#### FR-NEW-010 [EARS-O]: restore destination collision
> IF the restore destination (`original_path`, or a prior suffixed form) already exists THEN THE
> system SHALL retry at `original_path + "_restored"`, then `"_restored2"`, `"_restored3"`, ...,
> appending the suffix after the full filename including its extension, until a free path is found.
> The system SHALL NOT fail `fs.trash_restore` due to this collision.

- **Priority:** Must-have.
- **Rationale:** DEC-002 (user decision); SC-002/EXC steps 3.

#### FR-NEW-011 [EARS-E]: lazy backfill of legacy trash entries
> WHEN `fs.trash_list` or `fs.trash_restore` encounters a node directly under the project's `trash_dir`
> with no corresponding `trash_entries` row THE system SHALL insert one, idempotently, with
> `original_path` reconstructed by stripping the leading `{epoch_ms}__` prefix from the trash path's
> final segment and replacing remaining `__` with `/`, `deleted_at` parsed from that `epoch_ms`
> prefix (falling back to the current time if unparseable), and `deleted_by: null`.

- **Business Rules:** idempotent — calling this twice on the same untracked node SHALL insert exactly
  one row, never a duplicate. This reconstruction is best-effort: a legacy `original_path` that
  itself contained a literal `__` substring is ambiguous with a legacy path whose separator was `/`
  (both flatten identically), and this document does not resolve that ambiguity further — it is a
  known limitation of pre-existing data only, since every delete after this spec ships records
  `original_path` affirmatively (FR-NEW-002/003) and never needs reconstruction.
- **Priority:** Must-have.
- **Rationale:** DEC-003 (user decision: backfill on-demand, best-effort datetime, no startup job);
  SC-001/SC-002.

#### FR-NEW-012 [EARS-U]: `/app/trash` GUI screen
> The system SHALL expose `GET /app/trash?mount_id=<id>` and `POST /app/trash/restore`, modeled on
> `deleted_projects_screen.rs`'s identity resolution and CSRF convention, rendering the result of the
> same `fs.trash_list`/`fs.trash_restore` logic the MCP tools call — never a second implementation.

- **Exact names:** routes `GET /app/trash`, `POST /app/trash/restore`; query param `mount_id`; new
  module `crates/core/src/trash_screen.rs`, mounted in `app.rs` alongside
  `deleted_projects_screen::router`.
- **Business Rules:** CSRF enforced on the restore POST only when the identity source was the cookie
  (mirroring `deleted_projects_screen.rs:167-178` exactly); on authorization failure for the requested
  `mount_id`, the screen renders an error page (the `ERR_FORBIDDEN` the underlying call raises), not
  an empty list.
- **Priority:** Must-have.
- **Rationale:** SC-006.

#### FR-NEW-013 [EARS-UB]: hard delete never writes a trash entry
> The system SHALL NOT write a `trash_entries` row when `fs.delete` is called with `trash=false`
> (the hard-delete branch).

- **Priority:** Must-have.
- **Rationale:** closes gap 9 from Phase 4 design; keeps `hard_delete_works_when_allowed`
  (`fs_ops.rs:2711`) passing unmodified.

#### FR-NEW-014 [EARS-E]: `admin.create_project` retention parameters
> WHEN `admin.create_project` is called with any of `autopurge_enabled:boolean=false,
> use_internal_purge:boolean=false, file_retention_days:number|null=null,
> project_retention_days:number|null=null` THE system SHALL apply the resulting `PurgeConfig` to the
> new project atomically at creation, identical to calling `admin.set_purge_config` with the same
> values immediately afterward.

- **Inputs:** the four new optional params on `admin.create_project`, named and typed exactly as
  `admin.set_purge_config`'s equivalents.
- **Business Rules:** when all four are omitted, behavior is byte-identical to today
  (`PurgeConfig::default()`); every existing caller/test using the 2-arg form continues to compile and
  pass unmodified.
- **Exact names:** tool `admin.create_project`, new params as listed.
- **Priority:** Must-have.
- **Rationale:** DEC-006; SC-005.

#### FR-NEW-015 [EARS-O]: `admin.create_project` retention validation
> IF `file_retention_days` or `project_retention_days` is `0` or negative THEN THE system SHALL
> return `ERR_INVALID_ARGUMENT`, using the identical validation `admin.set_purge_config` already
> applies (`tools/admin.rs:572-578`).

- **Priority:** Must-have.
- **Rationale:** SC-005/EXC-005a.

#### FR-NEW-016 [EARS-E]: ACL on trash tools
> WHEN `fs.trash_list` or `fs.trash_restore` is called THE system SHALL authorize the caller via
> `state.authorize(mount_id, person)`, the same membership gate every other `fs.*` tool uses, with no
> additional per-file ownership check.

- **Business Rules:** `deleted_by` is informational display only; it SHALL NOT gate `fs.trash_restore`.
  Platform admin status SHALL NOT grant implicit access to a project the admin is not a member of.
- **Priority:** Must-have.
- **Rationale:** DEC-004, DEC-005; SC-001/SC-002.

#### FR-NEW-017 [EARS-U]: required build order
> The system SHALL be implemented in this order: (1) FR-NEW-001 (`trash_entries` schema across
> sqlite/postgres/sqlserver); (2) the shared collision-retry helper used by FR-NEW-004/FR-NEW-005/
> FR-NEW-006; (3) FR-NEW-002, FR-NEW-003, FR-NEW-013 (`delete_path` and `sweep_project_files`
> writing or withholding `trash_entries` rows); (4) FR-NEW-007/FR-NEW-008/FR-NEW-011
> (`fs.trash_list`) and FR-NEW-009/FR-NEW-010/FR-NEW-011 (`fs.trash_restore`); (5) FR-NEW-012 and
> FR-NEW-018 (the `/app/trash` screen, which calls only the functions built in step 4); (6)
> FR-NEW-014/FR-NEW-015 (`admin.create_project` params, independent of steps 1-5); (7)
> `TOOL_CONTRACT.txt` and `tool-contract-golden.json` regeneration, last, once every tool/schema
> change from steps 4 and 6 is final.

- **Priority:** Must-have.
- **Rationale:** G5 gap closed at Phase 6; see §14 for the narrative form of this same order.

#### FR-NEW-018 [EARS-O]: `/app/trash` with no `mount_id`
> IF `GET /app/trash` is requested without a `mount_id` query parameter THEN THE system SHALL
> render a picker listing the viewer's own project memberships, obtained via
> `AdminBackend::list_projects_for(person)` (`storage/traits.rs:209`, implemented at
> `storage/admin.rs:286` — the same membership source `admin.list_projects` already uses), each
> linking to `GET /app/trash?mount_id=<id>`, instead of calling `fs.trash_list` or rendering an
> error.

- **Business Rules:** this is a new call site for `list_projects_for` inside `trash_screen.rs`;
  `deleted_projects_screen.rs`'s own membership check (`admin.rs:402-423`) iterates already-deleted
  projects and is not reusable here, so this is not "no new membership-listing code path" — it is a
  new call into an existing trait method. IF `list_projects_for` returns an empty list THEN the
  picker SHALL render with an explicit empty-state message (e.g. "you are not a member of any
  project"), HTTP 200, not an error.
- **Exact names:** route `GET /app/trash` with `mount_id` absent; module `trash_screen.rs`; trait
  method `AdminBackend::list_projects_for`.
- **Priority:** Must-have.
- **Rationale:** closes the G4 forced-choice gap (Phase 6 round 1): SC-006 describes a
  picker-then-list screen but FR-NEW-012 alone never specified the no-`mount_id` case. Amended at
  Phase 6 round 2 to name the correct membership source and the zero-membership case.

## 7. Non-Functional Requirements

### 7.1 Performance
Unchanged from the rest of the `fs.*` surface: internal-tool-scale load, no new throughput target.
`fs.trash_list` is bounded by `limit` (default 200, max unspecified beyond `i32` range — no additional
cap requested).

### 7.2 Security
No new authentication mechanism. Authorization reuses `state.authorize` exactly as every other
`fs.*` tool. The GUI reuses the existing header-then-cookie identity resolution and CSRF convention
verbatim from `deleted_projects_screen.rs`. No secrets are introduced.

### 7.3 Usability
No new accessibility/i18n requirements beyond what `deleted_projects_screen.rs` already establishes
(plain escaped HTML, no JS framework).

### 7.4 Reliability
The `trash_entries` insert/delete happens in the same transaction as the corresponding `rename`
(`run_retrying`, per `storage/meta.rs:701`'s existing transactional pattern) — a failed rename never
leaves an orphaned `trash_entries` row, and a failed insert never leaves an untracked rename (both
roll back together).

### 7.5 Observability
- **Collector:** existing JSONL file collector, unchanged.
- **What to trace:** `fs.trash_list`/`fs.trash_restore` calls at INFO (API calls), the `trash_entries`
  write/delete at DEBUG (file mutations), matching the existing tracing levels for `fs.delete`.
- **Never traced:** no change — no credentials or PII involved.

### 7.6 Deployment
Unchanged — same single deployment target as the rest of the server; no new infrastructure.

### 7.7 Scalability
Unchanged. `trash_entries` is indexed by `(volume_id, trash_path)` as its primary key; a secondary
index on `(volume_id, deleted_at)` supports the ordering `fs.trash_list` requires (implementation
detail, not independently user-observable, left to the dialect-specific schema file).

## 8. Data Model

**New table `trash_entries`** (one row per volume per soft-deleted top-level path):

| Column | Type | Notes |
|---|---|---|
| `volume_id` | string | part of the composite primary key, scopes every row like `nodes`/`blob_refs` |
| `trash_path` | string (bounded per `TextKey` on SQL Server) | part of the composite primary key |
| `original_path` | string | the path to restore to |
| `size` | integer | copied from the node at delete time |
| `kind` | string, `'file'` \| `'dir'` | |
| `deleted_at` | string (RFC3339) | matches `admin.list_deleted_projects`'s convention |
| `deleted_by` | string, nullable | `null` for sweep-initiated deletes |

No changes to `NodeRow` or any existing table.

## 9. Impact Analysis

### 9.1 Affected Components
| File/Module | Impact | Description |
|---|---|---|
| `crates/core/src/core/fs_ops.rs` | Modified | `delete_path` writes a `trash_entries` row on the soft-delete branch; collision-retry helper extracted and shared |
| `crates/core/src/purge.rs` | Modified | `sweep_project_files` writes a `trash_entries` row (`deleted_by: null`), reuses the collision-retry helper |
| `crates/core/src/storage/traits.rs` | Modified | new trait method(s) for `trash_entries` CRUD |
| `crates/core/src/storage/meta.rs`, `storage/rel/{sqlite,postgres,sqlserver}.rs`, `storage/rel/schema.rs` | Modified | new `trash_entries` table/migration per dialect |
| `crates/core/src/mcp/server.rs` | Modified | two new `#[tool]` methods: `fs.trash_list`, `fs.trash_restore` |
| `crates/core/src/tools/trash.rs` | New | the typed functions the above tools and the GUI screen both call |
| `crates/core/src/tools/admin.rs`, `storage/admin.rs` | Modified | `create_project` gains the four optional purge params |
| `crates/core/src/mcp/server.rs` | Modified | `CreateProjectArgs` (`mcp/server.rs:1122`) and `admin_create_project` (`mcp/server.rs:2079`) gain the four optional purge params — this is the production entry point, `tools/admin.rs`'s `create_project` is `#[cfg(test)]`-gated test-dispatch glue only |
| `crates/core/src/storage/traits.rs` (`AdminBackend::create_project`, `storage/traits.rs:204`) and every dialect implementor (sqlite/postgres/sqlserver) | Modified | trait signature grows from `(project_id, owner)` to include the four optional purge params, applied atomically at creation |
| `crates/core/src/trash_screen.rs` | New | the `/app/trash` GUI screen |
| `crates/core/src/app.rs` | Modified | mounts the new screen router |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | Modified | regenerated (`MCPFS_REWRITE_TOOL_CONTRACT=1`) |

### 9.2 Affected Requirements
None — no existing `specs/*` document covers this area (§2.2).

### 9.3 Affected Tests
| Test File | Test | Action | Description |
|---|---|---|---|
| none | — | — | purely additive; no existing test requires modification (confirmed §2.3) |

### 9.4 Affected Documentation
| Document | Section | Action | Description |
|---|---|---|---|
| `AGENTS.md` | module bullet list | Update | add `trash_screen.rs`, `tools/trash.rs`, `trash_entries` bookkeeping note on `purge.rs` |
| `.agent_docs/tools.md` | tool count/reference | Update | 98 → 100 tools |
| `TOOL_CONTRACT.txt` | `fs.*`/`admin.*` sections | Update | new tool entries, updated `admin.create_project` params |

### 9.5 Dependencies & Risks
No new crate dependencies. Risk: `tool-contract-golden.json` is machine-checked; regeneration must be
reviewed, not merely re-run. Rollback: additive schema and tooling only; dropping the
`trash_entries` table and the two new tools fully reverts with no data loss to `nodes`/`blob_refs`.

## 10. Documentation Requirements

- `AGENTS.md`: update the `crates/core/` module bullet list per §9.4.
- `.agent_docs/tools.md`: add `fs.trash_list`/`fs.trash_restore` to the tool reference, update the
  98-tool count.
- `TOOL_CONTRACT.txt`: regenerate.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E (Happy) | E2E (Failure) | E2E (Edge) |
|---|---|---|---|---|
| SC-001 | FR-NEW-007, FR-NEW-008, FR-NEW-016 | E2E-NEW-406 | E2E-NEW-407, E2E-NEW-408, E2E-NEW-438 | E2E-NEW-409, E2E-NEW-410, E2E-NEW-411 |
| SC-002 | FR-NEW-009, FR-NEW-010, FR-NEW-016 | E2E-NEW-417, E2E-NEW-423 | E2E-NEW-418, E2E-NEW-419, E2E-NEW-437 | E2E-NEW-420, E2E-NEW-421, E2E-NEW-422, E2E-NEW-439 |
| SC-003 | FR-NEW-001, FR-NEW-003 | E2E-NEW-403 | E2E-NEW-445 | E2E-NEW-457 |
| SC-004 | FR-NEW-004, FR-NEW-005, FR-NEW-006 | E2E-NEW-443 | E2E-NEW-441, E2E-NEW-447 | E2E-NEW-404, E2E-NEW-405, E2E-NEW-444, E2E-NEW-448, E2E-NEW-449 |
| SC-005 | FR-NEW-014, FR-NEW-015 | E2E-NEW-429 | E2E-NEW-431 | E2E-NEW-430 |
| SC-006 | FR-NEW-012, FR-NEW-018 | E2E-NEW-432, E2E-NEW-462 | E2E-NEW-433, E2E-NEW-434 | E2E-NEW-455, E2E-NEW-463 |
| (cross) | FR-NEW-001, FR-NEW-002, FR-NEW-011, FR-NEW-013 | E2E-NEW-401 | — | E2E-NEW-413, E2E-NEW-424, E2E-NEW-415, E2E-NEW-416 |
| (process) | FR-NEW-017 | n/a — structural: verified by the build succeeding in the stated order (§14) and by `cargo test --workspace` staying green at every step, not by a dedicated E2E test | | |
| (ACL) | FR-NEW-016 | E2E-NEW-427 | E2E-NEW-428 | — |

## 12. End-to-End Test Suite

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-401 | New | Happy | SC-003/cross | FR-NEW-001, FR-NEW-002 | P0 |
| E2E-NEW-402 | New | Side effect | cross | FR-NEW-002 | P1 |
| E2E-NEW-403 | New | Happy | SC-003 | FR-NEW-003 | P0 |
| E2E-NEW-404 | New | Edge | SC-004 | FR-NEW-004 | P0 |
| E2E-NEW-405 | New | Edge | SC-004 | FR-NEW-005 | P2 |
| E2E-NEW-406 | New | Happy | SC-001 | FR-NEW-007 | P0 |
| E2E-NEW-407 | New | Failure | SC-001 | FR-NEW-007 | P0 |
| E2E-NEW-408 | New | Failure | SC-001 | FR-NEW-016 | P0 |
| E2E-NEW-409 | New | Edge | SC-001 | FR-NEW-007 | P1 |
| E2E-NEW-410 | New | Edge | SC-001 | FR-NEW-007 | P1 |
| E2E-NEW-411 | New | Edge | SC-001 | FR-NEW-007 | P1 |
| E2E-NEW-412 | New | Happy | SC-001 | FR-NEW-007 | P1 |
| E2E-NEW-413 | New | Edge | cross | FR-NEW-011 | P0 |
| E2E-NEW-414 | New | Idempotency | cross | FR-NEW-011 | P0 |
| E2E-NEW-415 | New | Edge | cross | FR-NEW-007 | P1 |
| E2E-NEW-416 | New | Edge | cross | FR-NEW-007 | P1 |
| E2E-NEW-417 | New | Happy | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-418 | New | Failure | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-419 | New | Failure | SC-002 | FR-NEW-016 | P0 |
| E2E-NEW-420 | New | Edge | SC-002 | FR-NEW-010 | P0 |
| E2E-NEW-421 | New | Edge | SC-002 | FR-NEW-010 | P1 |
| E2E-NEW-422 | New | Edge | SC-002 | FR-NEW-010 | P1 |
| E2E-NEW-423 | New | Happy | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-424 | New | Edge | cross | FR-NEW-011 | P0 |
| E2E-NEW-425 | New | Idempotency | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-426 | New | Side effect | SC-002 | FR-NEW-009 | P0 |
| E2E-NEW-427 | New | Happy/ACL | ACL | FR-NEW-016 | P0 |
| E2E-NEW-428 | New | Failure/ACL | ACL | FR-NEW-016 | P1 |
| E2E-NEW-429 | New | Happy | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-430 | New | Edge | SC-005 | FR-NEW-014 | P0 |
| E2E-NEW-431 | New | Failure | SC-005 | FR-NEW-015 | P1 |
| E2E-NEW-432 | New | GUI/Happy | SC-006 | FR-NEW-012 | P1 |
| E2E-NEW-433 | New | GUI/Failure | SC-006 | FR-NEW-012 | P0 |
| E2E-NEW-434 | New | GUI/Failure | SC-006 | FR-NEW-012 | P1 |
| E2E-NEW-435 | New | GUI/Side effect | SC-006 | FR-NEW-012 | P2 |
| E2E-NEW-436 | New | State transition | cross | FR-NEW-007, FR-NEW-009 | P0 |
| E2E-NEW-437 | New | Failure | SC-002 | FR-NEW-009 | P2 |
| E2E-NEW-438 | New | Failure | SC-001 | FR-NEW-008 | P2 |
| E2E-NEW-439 | New | Edge | SC-002 | FR-NEW-009 | P1 |
| E2E-NEW-440 | New | Side effect | SC-003 | FR-NEW-003 | P1 |
| E2E-NEW-441 | New | Failure | SC-004 | FR-NEW-006 | P1 |
| E2E-NEW-442 | New | Failure/ACL | ACL | FR-NEW-016 | P1 |
| E2E-NEW-443 | New | Happy | SC-004 | FR-NEW-004 | P1 |
| E2E-NEW-444 | New | Edge | SC-004 | FR-NEW-004 | P2 |
| E2E-NEW-445 | New | Failure | SC-003 | FR-NEW-001, FR-NEW-003 | P1 |
| E2E-NEW-446 | New | Edge | cross | FR-NEW-001 | P2 |
| E2E-NEW-447 | New | Failure | SC-004 | FR-NEW-005 | P2 |
| E2E-NEW-448 | New | Edge | SC-004 | FR-NEW-005 | P2 |
| E2E-NEW-449 | New | Edge | SC-004 | FR-NEW-006 | P2 |
| E2E-NEW-450 | New | Happy | cross | FR-NEW-006 | P2 |
| E2E-NEW-451 | New | Failure | SC-001 | FR-NEW-008 | P2 |
| E2E-NEW-452 | New | Happy | SC-001 | FR-NEW-008 | P2 |
| E2E-NEW-453 | New | Edge | cross | FR-NEW-013 | P0 |
| E2E-NEW-454 | New | Edge | cross | FR-NEW-013 | P1 |
| E2E-NEW-455 | New | GUI/Edge | SC-006 | FR-NEW-012 | P1 |
| E2E-NEW-456 | New | Edge | cross | FR-NEW-013 | P2 |
| E2E-NEW-457 | New | Edge | SC-003 | FR-NEW-003 | P2 |
| E2E-NEW-458 | New | Happy | cross | FR-NEW-002 | P2 |
| E2E-NEW-459 | New | Edge | SC-005 | FR-NEW-014 | P2 |
| E2E-NEW-460 | New | Failure | SC-005 | FR-NEW-015 | P2 |
| E2E-NEW-461 | New | Failure | SC-005 | FR-NEW-015 | P2 |
| E2E-NEW-462 | New | GUI/Happy | SC-006 | FR-NEW-018 | P1 |
| E2E-NEW-463 | New | GUI/Edge | SC-006 | FR-NEW-018 | P2 |

**Coverage statistics (recounted directly from the table above, 63 tests):** happy 14
(401,403,406,412,417,423,427,429,432,443,450,452,458,462), failure 17
(407,408,418,419,428,431,433,434,437,438,441,442,445,447,451,460,461), edge 25
(404,405,409,410,411,413,415,416,420,421,422,424,430,439,444,446,448,449,453,454,455,456,457,459,463),
side effects 4 (402,426,435,440), idempotency 2 (414,425), state transition 1 explicit (436, +2
implicit in 417/423). **Failure (17) outnumbers happy (14), satisfying the sufficiency rule that
failure tests outnumber happy-path tests.**

### 12.2 New Test Specifications

#### E2E-NEW-401: `fs.delete` creates a trash entry
- **Category:** Happy
- **Scenario:** SC-003 (write path shared with SC-002's delete step)
- **Requirements:** FR-NEW-001, FR-NEW-002
- **Driver:** direct function call (`core::fs_ops::delete_path`)
- **Preconditions:** project `proj-trash-1` exists, member `alice`, file `/a.txt` written with content
  `"x"`.
- **Steps:**
  - Given the file above exists.
  - When `alice` calls `fs.delete(mount_id="proj-trash-1", path="/a.txt", recursive=false,
    trash=true)`.
  - Then the response is `{"path": "/a.txt", "trashed": true, "trash_path":
    "/.mcp_trash/{epoch_ms}__a.txt"}`.
  - And a DB query `SELECT * FROM trash_entries WHERE volume_id=? AND trash_path=?` returns exactly
    one row with `original_path="/a.txt", size=1, kind="file", deleted_by="alice"`.
- **Cleanup:** none (test-local volume).
- **Priority:** Critical

#### E2E-NEW-402: `deleted_by` reflects the actual caller
- **Category:** Side Effect
- **Scenario:** cross
- **Requirements:** FR-NEW-002
- **Driver:** direct function call
- **Preconditions:** same as E2E-NEW-401, delete performed by `bob`.
- **Steps:**
  - Given file `/a.txt` exists.
  - When `bob` calls `fs.delete(mount_id="proj-trash-1", path="/a.txt", trash=true)`.
  - Then the `trash_entries` row has `deleted_by="bob"` exactly (DB query).
- **Priority:** High

#### E2E-NEW-403: sweep writes a system-initiated trash entry
- **Category:** Happy
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Driver:** direct function call (`purge::sweep_project_files`)
- **Preconditions:** project `proj-trash-1`, `PurgeConfig{autopurge_enabled: true,
  use_internal_purge: true, file_retention_days: Some(1)}`, file `/old.txt` with `atime` 3 days old.
- **Steps:**
  - Given the stale file above.
  - When `sweep_project_files(client, admin, safety, "proj-trash-1")` runs.
  - Then it returns `Ok(1)`.
  - And a `trash_entries` row exists for the trashed path with `original_path="/old.txt",
    deleted_by=null` (DB query, asserting SQL `NULL`, not empty string).
- **Priority:** Critical

#### E2E-NEW-404: flatten/timestamp collision retries at `~1`
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Driver:** direct function call, with the clock pinned (test seam equivalent to
  `sweep_project_files_at`'s explicit `now`)
- **Preconditions:** project `proj-trash-1`, files `/d/a.txt` and `/d__a.txt` both exist; both deletes
  are forced to compute the identical trash destination `/.mcp_trash/{same_ms}__d__a.txt`.
- **Steps:**
  - Given both files exist and the clock is pinned.
  - When `/d/a.txt` is deleted, then `/d__a.txt` is deleted at the same instant.
  - Then the first succeeds at `/.mcp_trash/{ts}__d__a.txt`; the second, after `ERR_NO_CLOBBER`,
    succeeds at `/.mcp_trash/{ts}__d__a.txt~1`.
  - And both have correct, distinct `trash_entries` rows with correct `original_path` (DB query).
- **Priority:** Critical

#### E2E-NEW-405: collision retries exhausted
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005
- **Driver:** direct function call, fixture pre-seeds 50 colliding nodes
- **Preconditions:** `/.mcp_trash/{ts}__x` through `/.mcp_trash/{ts}__x~50` all pre-exist as dummy
  nodes (seeded directly via the volume client).
- **Steps:**
  - Given the 51 pre-occupied destinations.
  - When a delete that would collide at `/.mcp_trash/{ts}__x` is attempted.
  - Then the call returns `ERR_INTERNAL_ERROR`, completing within 1 second (bounding an infinite
    loop).
  - And the original file `/x` remains live, untouched (filesystem/node check).
- **Priority:** Low

#### E2E-NEW-406: `fs.trash_list` orders by `deleted_at` descending
- **Category:** Happy
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007
- **Driver:** direct function call (MCP tool handler)
- **Preconditions:** project `proj-list-1`, three deletes in sequence: `/a.txt` at `t0`, `/b.txt` at
  `t0+1s`, `/c.txt` at `t0+2s`.
- **Steps:**
  - Given the three deletes above.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="", limit=200, offset=0)` is called.
  - Then `entries` is `[c.txt-entry, b.txt-entry, a.txt-entry]` in that order, `total=3`, each entry
    containing all seven keys.
- **Priority:** Critical

#### E2E-NEW-407: `fs.trash_list` on an unknown project
- **Category:** Failure
- **Scenario:** SC-001 / EXC-001a
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Steps:**
  - Given no project `nonexistent-proj` exists.
  - When `alice` calls `fs.trash_list(mount_id="nonexistent-proj", ...)`.
  - Then the call fails with `ERR_PROJECT_NOT_FOUND`.
- **Priority:** Critical

#### E2E-NEW-408: `fs.trash_list` forbidden for a non-member
- **Category:** Failure
- **Scenario:** SC-001 / EXC-001b
- **Requirements:** FR-NEW-016
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-list-1` with member `alice` only.
  - When `eve` (not a member, not a platform admin) calls `fs.trash_list(mount_id="proj-list-1",
    ...)`.
  - Then the call fails with `ERR_FORBIDDEN`.
- **Priority:** Critical

#### E2E-NEW-409: empty trash
- **Category:** Edge
- **Scenario:** SC-001 / EXC-001c
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-empty` with no deletions ever performed.
  - When `fs.trash_list(mount_id="proj-empty", path_prefix="", limit=200, offset=0)` is called.
  - Then the response is `{"entries": [], "total": 0}`, no error.
- **Priority:** High

#### E2E-NEW-410: pagination boundary
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Preconditions:** project `proj-page` with exactly 5 trashed files `f1.txt`..`f5.txt` (`f5` most
  recent).
- **Steps:**
  - Given the 5 entries.
  - When `fs.trash_list(mount_id="proj-page", path_prefix="", limit=2, offset=4)` is called.
  - Then `entries` has exactly 1 element (`f1.txt`'s entry), `total=5`.
  - And when called again with `limit=2, offset=5`, `entries` is `[]`, `total=5`.
- **Priority:** High

#### E2E-NEW-411: `path_prefix` matches nothing
- **Category:** Edge
- **Scenario:** SC-001 / EXC-001c
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-list-1` with entries under `/data/`.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="/nomatch/", limit=200, offset=0)` is
    called.
  - Then the response is `{"entries": [], "total": 0}`, not an error.
- **Priority:** High

#### E2E-NEW-412: `path_prefix` filters correctly
- **Category:** Happy
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-list-1` with trashed `/data/a.txt` and `/other/b.txt`.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="/data/", limit=200, offset=0)` is
    called.
  - Then `entries` contains exactly the `/data/a.txt` entry.
- **Priority:** High

#### E2E-NEW-413: backfill of a pre-existing untracked trash node
- **Category:** Edge
- **Scenario:** cross (SC-001 + FR-NEW-011)
- **Requirements:** FR-NEW-011
- **Driver:** direct function call, with a node seeded directly via the volume client
- **Preconditions:** project `proj-backfill`, node seeded at
  `/.mcp_trash/1700000000000__legacy__report.txt` with no `trash_entries` row.
- **Steps:**
  - Given the untracked node.
  - When `fs.trash_list(mount_id="proj-backfill", path_prefix="", limit=200, offset=0)` is called.
  - Then the response includes an entry with `original_path="legacy/report.txt"` and `deleted_at`
    derived from epoch `1700000000000` ms.
  - And a `trash_entries` row now exists in the DB for that `trash_path` with `deleted_by=null` (DB
    query).
- **Priority:** Critical

#### E2E-NEW-414: backfill is idempotent
- **Category:** Idempotency
- **Scenario:** cross
- **Requirements:** FR-NEW-011
- **Driver:** direct function call
- **Preconditions:** state immediately after E2E-NEW-413's first call.
- **Steps:**
  - Given one backfilled row already exists.
  - When `fs.trash_list(mount_id="proj-backfill", ...)` is called a second time.
  - Then the response still contains exactly one entry for `legacy/report.txt`.
  - And a DB query confirms exactly one `trash_entries` row for that `trash_path`.
- **Priority:** Critical

#### E2E-NEW-415: `purge_in_days` can go negative
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Preconditions:** project `proj-retention`, `PurgeConfig.file_retention_days = Some(3)`, an entry
  with `deleted_at` backdated 10 days.
- **Steps:**
  - Given the backdated entry.
  - When `fs.trash_list(mount_id="proj-retention", ...)` is called.
  - Then the entry's `purge_in_days` is `-7`.
- **Priority:** High

#### E2E-NEW-416: `purge_in_days` is `null` without retention configured
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-007
- **Driver:** direct function call
- **Preconditions:** project `proj-no-retention`, `PurgeConfig.file_retention_days = None`, a file
  deleted 1 day ago.
- **Steps:**
  - Given the entry.
  - When `fs.trash_list(mount_id="proj-no-retention", ...)` is called.
  - Then the entry's `purge_in_days` is `null`.
- **Priority:** High

#### E2E-NEW-417: restore to the original path
- **Category:** Happy / State Transition
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, file `/doc.txt` with content `"hello"`, deleted
  producing `trash_path="/.mcp_trash/{ts}__doc.txt"`.
- **Steps:**
  - Given the trashed file.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="/.mcp_trash/{ts}__doc.txt")` is
    called.
  - Then the response is `{"restored_path": "/doc.txt", "trash_path":
    "/.mcp_trash/{ts}__doc.txt"}`.
  - And `/doc.txt` exists again with content `"hello"` (filesystem/node check).
  - And the `trash_entries` row for that `trash_path` no longer exists (DB query).
- **Priority:** Critical

#### E2E-NEW-418: restore of an unknown trash path
- **Category:** Failure
- **Scenario:** SC-002 / EXC-002a
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-restore-1`.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="/.mcp_trash/999__nope.txt")` is
    called, no such node or row exists.
  - Then the call fails with `ERR_NOT_FOUND`.
- **Priority:** Critical

#### E2E-NEW-419: restore forbidden for a non-member
- **Category:** Failure
- **Scenario:** SC-002 / FR-NEW-016
- **Requirements:** FR-NEW-016
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-restore-1` with member `alice` only, a trashed entry present.
  - When `eve` (non-member) calls `fs.trash_restore(...)`.
  - Then the call fails with `ERR_FORBIDDEN`.
- **Priority:** Critical

#### E2E-NEW-420: restore collision renames to `_restored`
- **Category:** Edge
- **Scenario:** SC-002 / FR-NEW-010
- **Requirements:** FR-NEW-010
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, `/a.txt` deleted (trashed), then a new `/a.txt` written
  at the original path.
- **Steps:**
  - Given both the trashed entry and the new occupying file.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="<a.txt's trash_path>")` is called.
  - Then the call succeeds with `{"restored_path": "/a.txt_restored", "trash_path": "..."}`.
  - And both `/a.txt` (new) and `/a.txt_restored` (restored) exist distinctly (filesystem/node
    check).
  - And the `trash_entries` row is deleted (DB query).
- **Priority:** Critical

#### E2E-NEW-421: double collision renames to `_restored2`
- **Category:** Edge
- **Scenario:** SC-002 / FR-NEW-010
- **Requirements:** FR-NEW-010
- **Driver:** direct function call
- **Preconditions:** state from E2E-NEW-420, plus a second distinct trashed copy of `/a.txt`.
- **Steps:**
  - Given `/a.txt` and `/a.txt_restored` both occupied, and a second trashed `/a.txt` entry.
  - When that second entry is restored.
  - Then it lands at `/a.txt_restored2`.
- **Priority:** High

#### E2E-NEW-422: collision suffix appended after the extension
- **Category:** Edge
- **Scenario:** SC-002 / FR-NEW-010
- **Requirements:** FR-NEW-010
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, `/report.pdf` deleted, then a new `/report.pdf`
  written to occupy the original path.
- **Steps:**
  - Given the collision.
  - When the trashed `/report.pdf` is restored.
  - Then the restored path is exactly `/report.pdf_restored`, not `/report_restored.pdf`.
- **Priority:** High

#### E2E-NEW-423: restoring a directory restores the whole subtree
- **Category:** Happy
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, directory `/proj/` containing `/proj/x.txt` and
  `/proj/sub/y.txt`, deleted via `fs.delete(path="/proj", recursive=true, trash=true)`.
- **Steps:**
  - Given the trashed directory subtree.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="<proj's trash_path>")` is called.
  - Then `/proj/`, `/proj/x.txt`, `/proj/sub/y.txt` all exist again with original content
    (filesystem/node check on every node in the subtree).
  - And exactly one `trash_entries` row is deleted (DB query).
- **Priority:** Critical

#### E2E-NEW-424: restore backfills an untracked node inline
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-011
- **Driver:** direct function call
- **Preconditions:** project `proj-backfill-2`, node seeded at
  `/.mcp_trash/1700000000000__orphan.txt`, no prior `fs.trash_list` call.
- **Steps:**
  - Given the untracked node.
  - When `fs.trash_restore(mount_id="proj-backfill-2",
    trash_path="/.mcp_trash/1700000000000__orphan.txt")` is called directly.
  - Then the restore succeeds, restoring to `/orphan.txt` (filesystem/node check).
  - And no leftover `trash_entries` row exists afterward (DB query — created then deleted within the
    same call).
- **Priority:** Critical

#### E2E-NEW-425: restoring twice fails the second time
- **Category:** Idempotency
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Preconditions:** state immediately after E2E-NEW-417 succeeds.
- **Steps:**
  - Given the row and node are both gone.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="/.mcp_trash/{ts}__doc.txt")` is
    called again with the identical path.
  - Then the call fails with `ERR_NOT_FOUND`.
- **Priority:** Critical

#### E2E-NEW-426: restore deletes the row, verified independently
- **Category:** Side Effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** direct function call + DB query
- **Preconditions:** project `proj-restore-1`, `/z.txt` deleted, `trash_entries` row confirmed present
  via direct DB query before restore.
- **Steps:**
  - Given the confirmed row.
  - When `fs.trash_restore` is called on it.
  - Then `SELECT COUNT(*) FROM trash_entries WHERE volume_id=? AND trash_path=?` returns `0` (DB
    query, independent of the tool's response).
- **Priority:** Critical

#### E2E-NEW-427: any member can list/restore, not just the deleter
- **Category:** Happy / ACL
- **Scenario:** ACL
- **Requirements:** FR-NEW-016
- **Driver:** direct function call
- **Preconditions:** project `proj-acl`, members `alice` (deleter) and `bob`, `/shared.txt` deleted by
  `alice`.
- **Steps:**
  - Given `/shared.txt` trashed by `alice`.
  - When `bob` calls `fs.trash_list(mount_id="proj-acl", ...)` then
    `fs.trash_restore(mount_id="proj-acl", trash_path="<shared.txt's trash_path>")`.
  - Then both calls succeed for `bob`; the list entry showed `deleted_by="alice"` but did not block
    `bob`'s restore.
- **Priority:** Critical

#### E2E-NEW-428: platform admin without membership is forbidden
- **Category:** Failure / ACL
- **Scenario:** ACL
- **Requirements:** FR-NEW-016
- **Driver:** direct function call
- **Preconditions:** project `proj-acl` with member `alice` only; platform admin `admin-carol` not a
  member.
- **Steps:**
  - Given `admin-carol` is a platform admin but not a member of `proj-acl`.
  - When `admin-carol` calls `fs.trash_list` or `fs.trash_restore` on `proj-acl`.
  - Then the call fails with `ERR_FORBIDDEN`.
- **Priority:** High

#### E2E-NEW-429: `admin.create_project` applies retention at creation
- **Category:** Happy
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-new-purge` exists.
  - When `admin.create_project(project_id="proj-new-purge", owner="alice", autopurge_enabled=true,
    use_internal_purge=true, file_retention_days=7, project_retention_days=30)` is called.
  - Then the project is created.
  - And `admin.get_purge_config("proj-new-purge")` (internal call) returns
    `PurgeConfig{autopurge_enabled: true, use_internal_purge: true, file_retention_days: Some(7),
    project_retention_days: Some(30)}`.
- **Priority:** Critical

#### E2E-NEW-430: omitted params keep today's default
- **Category:** Edge
- **Scenario:** SC-005 / EXC-005b
- **Requirements:** FR-NEW-014
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-old-style` exists.
  - When `admin.create_project(project_id="proj-old-style", owner="alice")` is called with no purge
    params (the existing 2-arg form).
  - Then the project is created with `PurgeConfig::default()` exactly.
- **Priority:** Critical

#### E2E-NEW-431: negative retention at creation is rejected
- **Category:** Failure
- **Scenario:** SC-005 / EXC-005a
- **Requirements:** FR-NEW-015
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-bad-retention` exists.
  - When `admin.create_project(project_id="proj-bad-retention", owner="alice",
    autopurge_enabled=true, use_internal_purge=true, file_retention_days=-1,
    project_retention_days=null)` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** High

#### E2E-NEW-432: `/app/trash` renders entries
- **Category:** GUI / Happy
- **Scenario:** SC-006
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client against the test server
- **Preconditions:** project `proj-gui-1`, viewer-member `alice`, two files trashed.
- **Steps:**
  - Given the two trashed entries and `alice`'s authenticated session.
  - When `alice` performs `GET /app/trash?mount_id=proj-gui-1`.
  - Then the response is `200`, HTML containing both entries' `original_path` values
    (HTML-escaped) and a restore form per row.
- **Priority:** High

#### E2E-NEW-433: restore POST without a valid CSRF token is rejected
- **Category:** GUI / Failure (security)
- **Scenario:** SC-006 / EXC-006a
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client
- **Preconditions:** project `proj-gui-1` with a trashed entry, `alice` authenticated via the cookie
  (ambient credential).
- **Steps:**
  - Given a cookie-authenticated session with no/invalid CSRF token.
  - When `alice`'s session POSTs a restore to `/app/trash/restore` with a missing or wrong CSRF token.
  - Then the response is `403 FORBIDDEN`.
  - And the `trash_entries` row is NOT deleted (DB query).
- **Priority:** Critical

#### E2E-NEW-434: non-member `mount_id` renders an error page
- **Category:** GUI / Failure
- **Scenario:** SC-006 / EXC-006b
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client
- **Preconditions:** project `proj-gui-1` with member `alice` only.
- **Steps:**
  - Given `bob` is authenticated but not a member of `proj-gui-1`.
  - When `bob` performs `GET /app/trash?mount_id=proj-gui-1`.
  - Then the response renders the `ERR_FORBIDDEN` error page (not an empty success list).
- **Priority:** High

#### E2E-NEW-435: GUI restore uses the same underlying function as the MCP tool
- **Category:** GUI / Side Effect
- **Scenario:** SC-006
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client + direct comparison
- **Preconditions:** two identical trashed entries in two otherwise-identical projects, one restored
  via the GUI, one via `fs.trash_restore` directly.
- **Steps:**
  - Given the two identical setups.
  - When one is restored via the GUI POST and the other via the MCP tool.
  - Then the resulting DB/filesystem state (restored path, collision-suffix behavior) is identical
    between the two (direct state comparison), confirming no divergent reimplementation.
- **Priority:** Low

#### E2E-NEW-436: full lifecycle, live → trashed → listed → restored → live
- **Category:** State Transition
- **Scenario:** cross
- **Requirements:** FR-NEW-007, FR-NEW-009
- **Driver:** direct function call, sequential
- **Preconditions:** project `proj-lifecycle`, file `/cycle.txt` with content `"v1"`.
- **Steps:**
  - Given `/cycle.txt` exists.
  - When, in order: `fs.delete(trash=true)` trashes it; `fs.trash_list` is called; `fs.trash_restore`
    restores it; `fs.read("/cycle.txt")` is called.
  - Then after delete, the file is absent from `fs.list("/")` but present in `fs.trash_list`.
  - And after restore, the `trash_entries` row is gone and the file reappears in `fs.list("/")`.
  - And `fs.read` returns `"v1"` unchanged.
- **Priority:** Critical

#### E2E-NEW-437: restore with an empty `trash_path`
- **Category:** Failure
- **Scenario:** SC-002 / EXC-002c
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-restore-1`.
  - When `fs.trash_restore(mount_id="proj-restore-1", trash_path="")` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`, not `ERR_NOT_FOUND`.
- **Priority:** Low

#### E2E-NEW-438: negative `limit`/`offset` rejected
- **Category:** Failure
- **Scenario:** SC-001 / EXC-001d
- **Requirements:** FR-NEW-008
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-list-1`.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="", limit=-1, offset=0)` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

#### E2E-NEW-439: restore recreates a missing ancestor directory
- **Category:** Edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-009
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, `/parent/child/` trashed as a unit, then `/parent`
  itself hard-deleted (test fixture, `allow_hard_delete` on) after trashing.
- **Steps:**
  - Given `/parent` no longer exists live.
  - When `fs.trash_restore` restores `/parent/child/`.
  - Then `/parent/` and `/parent/child/` both exist post-restore (ancestors recreated via
    `makedirs`).
- **Priority:** High

#### E2E-NEW-440: sweep-created entries are listable and restorable like any other
- **Category:** Side Effect
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Driver:** direct function call, sequential
- **Preconditions:** project `proj-sweep`, `PurgeConfig{autopurge_enabled: true,
  use_internal_purge: true, file_retention_days: Some(1)}`, `/stale.txt` with `atime` 5 days old.
- **Steps:**
  - Given the stale file.
  - When `sweep_project_files` trashes it, then `fs.trash_list` is called, then `fs.trash_restore` is
    called on the resulting entry.
  - Then `fs.trash_list` shows `deleted_by: null`.
  - And `fs.trash_restore` restores `/stale.txt` and removes its `trash_entries` row identically to a
    user-deleted entry (filesystem/node check + DB query).
- **Priority:** High

#### E2E-NEW-441: retry fires only on `ERR_NO_CLOBBER`
- **Category:** Failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-006
- **Driver:** direct function call with a mocked/failing volume client
- **Preconditions:** a `rename` call mocked to return a non-collision error (e.g. a simulated volume
  I/O error) on the first attempt at the trash destination.
- **Steps:**
  - Given the mocked non-collision failure.
  - When a delete triggers the trash-path rename.
  - Then the error surfaces immediately with no `~1` retry attempted (mock call-count assertion: the
    rename function is called exactly once).
- **Priority:** High

#### E2E-NEW-442: a removed member loses trash access
- **Category:** Failure / ACL
- **Scenario:** ACL
- **Requirements:** FR-NEW-016
- **Driver:** direct function call
- **Preconditions:** project `proj-acl-2`, member `dave` deletes `/x.txt`, then `admin.remove_member`
  removes `dave` from `proj-acl-2`.
- **Steps:**
  - Given `dave` is no longer a member.
  - When `dave` calls `fs.trash_list(mount_id="proj-acl-2", ...)` or `fs.trash_restore(...)`.
  - Then the call fails with `ERR_FORBIDDEN` — "any current project member" does not mean "whoever
    was ever a member".
- **Priority:** High

#### E2E-NEW-443: the retry counter advances past `~1` when more than one slot is occupied
- **Category:** Happy
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Driver:** direct function call, clock pinned as in E2E-NEW-404
- **Preconditions:** the real file `/f.txt` exists. Both `/.mcp_trash/{ts}__f.txt` (the bare
  destination) and `/.mcp_trash/{ts}__f.txt~1` are pre-seeded as occupied dummy nodes (via the
  volume client directly), at the pinned instant `{ts}`.
- **Steps:**
  - Given `/f.txt` exists and both the bare destination and its `~1` slot are already occupied.
  - When `/f.txt` is deleted with the clock pinned to `{ts}`.
  - Then the delete succeeds at `/.mcp_trash/{ts}__f.txt~2` (tool response `trash_path` field),
    skipping both occupied slots in order.
  - And the `trash_entries` row for `/f.txt` records `trash_path` ending in `~2` (DB query).
- **Priority:** High

#### E2E-NEW-444: collision retry succeeds on the 49th attempt (one before exhaustion)
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-004
- **Driver:** direct function call, fixture pre-seeds 49 colliding nodes
- **Preconditions:** `/.mcp_trash/{ts}__y` through `/.mcp_trash/{ts}__y~48` pre-exist (49 occupied
  destinations, one fewer than E2E-NEW-405's 50).
- **Steps:**
  - Given the 49 pre-occupied destinations.
  - When a delete that would collide at `/.mcp_trash/{ts}__y` is attempted.
  - Then it succeeds at `/.mcp_trash/{ts}__y~49` (tool response check), distinguishing the
    last-successful boundary from E2E-NEW-405's first-exhausted boundary.
- **Priority:** Low

#### E2E-NEW-445: a failed `trash_entries` insert rolls back the sweep's rename
- **Category:** Failure
- **Scenario:** SC-003
- **Requirements:** FR-NEW-001, FR-NEW-003
- **Driver:** direct function call, with the `trash_entries` insert mocked/forced to fail (e.g. a
  simulated constraint violation or I/O error on that one write)
- **Preconditions:** project `proj-sweep-fail`, stale file `/stale2.txt` eligible for sweep, the
  `trash_entries` insert step instrumented to fail once.
- **Steps:**
  - Given the stale file and the forced insert failure.
  - When `sweep_project_files` processes it.
  - Then the per-file failure is reported (matching the existing "each file commits independently"
    semantics, `purge.rs:26`) and `/stale2.txt` remains live, NOT trashed (filesystem/node check) —
    the rename and the `trash_entries` insert commit or roll back together.
- **Priority:** High

#### E2E-NEW-446: trashing a directory writes exactly one row
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-001
- **Driver:** direct function call
- **Preconditions:** project `proj-restore-1`, directory `/big/` containing 10 descendant files.
- **Steps:**
  - Given the 10-file directory.
  - When `fs.delete(path="/big", recursive=true, trash=true)` is called.
  - Then exactly one `trash_entries` row exists for `/big`'s trash path (DB query: `SELECT
    COUNT(*)` = 1), not one per descendant.
- **Priority:** Low

#### E2E-NEW-447: collision exhaustion inside the sweep surfaces the same error
- **Category:** Failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005
- **Driver:** direct function call, fixture pre-seeds 50 colliding nodes
- **Preconditions:** same 50-destination setup as E2E-NEW-405, but triggered via
  `sweep_project_files` instead of `fs.delete`.
- **Steps:**
  - Given the 51 pre-occupied destinations and a stale file that would collide.
  - When `sweep_project_files` processes that file.
  - Then the per-file failure surfaces the same `ERR_INTERNAL_ERROR` condition, the sweep continues
    to the next file (per existing per-file isolation), and the stale file remains live.
- **Priority:** Low

#### E2E-NEW-448: no permanent poison after exhaustion
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-005
- **Driver:** direct function call
- **Preconditions:** state immediately after E2E-NEW-405's exhaustion (50 colliding destinations
  still occupied).
- **Steps:**
  - Given the exhausted state.
  - When one of the 50 occupying dummy nodes is removed, then the same delete is retried.
  - Then it now succeeds at the freed `~N` slot — confirming exhaustion is not a permanent failure
    mode for that path.
- **Priority:** Low

#### E2E-NEW-449: unrelated error on a later retry attempt still aborts without further retry
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-006
- **Driver:** direct function call, mocked volume client
- **Preconditions:** the first `~1` retry attempt is mocked to return a non-collision error (e.g. a
  simulated I/O error), after an initial genuine `ERR_NO_CLOBBER` on the bare destination.
- **Steps:**
  - Given a real collision on attempt 0 and a mocked unrelated failure on attempt 1 (`~1`).
  - When the delete is attempted.
  - Then the unrelated error surfaces immediately after exactly one retry (`~1`), with no `~2`
    attempt made (mock call-count assertion).
- **Priority:** Low

#### E2E-NEW-450: a non-colliding delete never invokes the retry path
- **Category:** Happy
- **Scenario:** cross
- **Requirements:** FR-NEW-006
- **Driver:** direct function call, mocked/instrumented `rename`
- **Preconditions:** a normal delete with no pre-existing trash destination collision.
- **Steps:**
  - Given no collision exists.
  - When the file is deleted.
  - Then `rename` into the trash destination is called exactly once (mock call-count assertion),
    confirming the retry path is dormant on the ordinary path.
- **Priority:** Low

#### E2E-NEW-451: negative `offset` alone is rejected
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Driver:** direct function call
- **Steps:**
  - Given project `proj-list-1`.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="", limit=10, offset=-1)` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

#### E2E-NEW-452: `limit=0` is accepted, not treated as negative
- **Category:** Happy
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Driver:** direct function call
- **Preconditions:** project `proj-list-1` with at least one trashed entry.
- **Steps:**
  - Given at least one entry exists.
  - When `fs.trash_list(mount_id="proj-list-1", path_prefix="", limit=0, offset=0)` is called.
  - Then the call succeeds with `{"entries": [], "total": N}` (N > 0) — `limit=0` returns no rows
    but is not an error, distinguishing it from a negative value.
- **Priority:** Low

#### E2E-NEW-453: hard-deleting a file writes no trash entry
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Driver:** direct function call
- **Preconditions:** server configured with `allow_hard_delete=true`, file `/h.txt` exists.
- **Steps:**
  - Given the file and hard-delete allowed.
  - When `fs.delete(mount_id, path="/h.txt", trash=false)` is called.
  - Then the file is gone (`hard_delete_works_when_allowed` behavior, unchanged).
  - And a DB query for any `trash_entries` row referencing `/h.txt` or its would-be trash path
    returns zero rows.
- **Priority:** Critical

#### E2E-NEW-454: hard-deleting a directory subtree writes no trash entries
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Driver:** direct function call
- **Preconditions:** server configured with `allow_hard_delete=true`, directory `/hd/` with 3
  descendant files.
- **Steps:**
  - Given the directory and hard-delete allowed.
  - When `fs.delete(mount_id, path="/hd", recursive=true, trash=false)` is called.
  - Then the whole subtree is gone.
  - And `SELECT COUNT(*) FROM trash_entries WHERE volume_id=?` is unchanged from before the call (DB
    query) — zero rows added.
- **Priority:** High

#### E2E-NEW-455: GUI renders an empty state for a project with no trashed files
- **Category:** GUI / Edge
- **Scenario:** SC-006
- **Requirements:** FR-NEW-012
- **Driver:** HTTP client
- **Preconditions:** project `proj-gui-empty` with member `alice`, no deletions ever performed.
- **Steps:**
  - Given the empty-trash project.
  - When `alice` performs `GET /app/trash?mount_id=proj-gui-empty`.
  - Then the response is `200` with an explicit empty-state message in the HTML (e.g. "no trashed
    files"), not an error and not a blank/broken table.
- **Priority:** Medium

#### E2E-NEW-456: hard-deleting a file does not disturb an unrelated prior trash entry
- **Category:** Edge
- **Scenario:** cross
- **Requirements:** FR-NEW-013
- **Driver:** direct function call
- **Preconditions:** server configured with `allow_hard_delete=true`. File `/k.txt` soft-deleted
  then restored once (leaving no `trash_entries` row, per E2E-NEW-417's postcondition), then a new
  unrelated file `/m.txt` is created and hard-deleted.
- **Steps:**
  - Given the prior soft-delete/restore cycle on `/k.txt` left zero rows, and `/m.txt` is now
    hard-deleted.
  - When `fs.delete(mount_id, path="/m.txt", trash=false)` is called.
  - Then `SELECT COUNT(*) FROM trash_entries WHERE volume_id=?` is `0` before and after (DB query) —
    the hard delete neither creates a row for `/m.txt` nor resurrects/affects any row related to
    `/k.txt`'s earlier, already-closed lifecycle.
- **Priority:** Low

#### E2E-NEW-457: sweep with no stale files writes no trash entries
- **Category:** Edge
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003
- **Driver:** direct function call
- **Preconditions:** project `proj-sweep-empty`, `PurgeConfig{autopurge_enabled: true,
  use_internal_purge: true, file_retention_days: Some(7)}`, no files with `atime` older than the
  threshold.
- **Steps:**
  - Given no stale files exist.
  - When `sweep_project_files` runs.
  - Then it returns `Ok(0)` (function return check).
  - And `SELECT COUNT(*) FROM trash_entries WHERE volume_id=?` is unchanged (DB query) — the sweep
    being a no-op writes nothing.
- **Priority:** Low

#### E2E-NEW-458: soft-deleting a directory records `kind` and `size` correctly
- **Category:** Happy
- **Scenario:** cross
- **Requirements:** FR-NEW-002
- **Driver:** direct function call
- **Preconditions:** project `proj-trash-1`, directory `/dirx/` with known aggregate node size.
- **Steps:**
  - Given the directory.
  - When `fs.delete(path="/dirx", recursive=true, trash=true)` is called.
  - Then the resulting `trash_entries` row has `kind="dir"` and `size` matching the directory
    node's own recorded size field (DB query) — distinct from E2E-NEW-446, which only checks the
    row count, not these field values.
- **Priority:** Low

#### E2E-NEW-459: partial retention params at creation don't implicitly enable autopurge
- **Category:** Edge
- **Scenario:** SC-005
- **Requirements:** FR-NEW-014
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-partial-purge` exists.
  - When `admin.create_project(project_id="proj-partial-purge", owner="alice",
    file_retention_days=5)` is called with `autopurge_enabled`/`use_internal_purge` left at their
    `false` defaults.
  - Then the resulting `PurgeConfig` has `file_retention_days=Some(5)` but `autopurge_enabled=false`
    (DB query) — confirming the fields apply independently, exactly as `admin.set_purge_config`
    already allows, and the sweep remains gated off per the existing `purge.rs:51` check.
- **Priority:** Low

#### E2E-NEW-460: negative `project_retention_days` at creation is rejected
- **Category:** Failure
- **Scenario:** SC-005
- **Requirements:** FR-NEW-015
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-bad-retention-2` exists.
  - When `admin.create_project(project_id="proj-bad-retention-2", owner="alice",
    autopurge_enabled=true, use_internal_purge=true, file_retention_days=null,
    project_retention_days=-3)` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`.
- **Priority:** Low

#### E2E-NEW-461: exact-zero retention at creation is rejected
- **Category:** Failure
- **Scenario:** SC-005
- **Requirements:** FR-NEW-015
- **Driver:** direct function call
- **Steps:**
  - Given no project `proj-zero-retention` exists.
  - When `admin.create_project(project_id="proj-zero-retention", owner="alice",
    autopurge_enabled=true, use_internal_purge=true, file_retention_days=0,
    project_retention_days=null)` is called.
  - Then the call fails with `ERR_INVALID_ARGUMENT`, mirroring `admin.set_purge_config`'s identical
    rejection of exact zero (`tools/admin.rs:572-578`).
- **Priority:** Low

#### E2E-NEW-462: `/app/trash` with no `mount_id` renders a project picker
- **Category:** GUI / Happy
- **Scenario:** SC-006
- **Requirements:** FR-NEW-018
- **Driver:** HTTP client
- **Preconditions:** `alice` is a member of `proj-gui-1` and `proj-gui-2`.
- **Steps:**
  - Given `alice`'s authenticated session.
  - When `alice` performs `GET /app/trash` (no `mount_id`).
  - Then the response is `200` with HTML listing links to
    `GET /app/trash?mount_id=proj-gui-1` and `GET /app/trash?mount_id=proj-gui-2` (her two
    memberships), and `fs.trash_list` is not invoked (instrumentation/call-count assertion).
- **Priority:** High

## 14. Migration & Implementation Notes

FR-NEW-017 states the required build order; this section is its narrative form, per the template's
requirement that implementation order be documented whenever one requirement breaks another if
applied first:

1. **Schema first.** FR-NEW-001's `trash_entries` table (all 3 dialects) must exist before any code
   compiles against it.
2. **Shared mechanics before their callers.** The collision-retry helper (FR-NEW-004/005/006) must
   exist before `delete_path` and `sweep_project_files` are wired to call it.
3. **Writers before readers.** FR-NEW-002/003/013 (writing or correctly withholding `trash_entries`
   rows) land before FR-NEW-007/009 (`fs.trash_list`/`fs.trash_restore`), which read that table.
4. **Tools before the GUI.** FR-NEW-012/018 (the `/app/trash` screen) call the exact same functions
   FR-NEW-007/009 expose — the screen cannot be implemented, let alone tested, before they exist.
5. **`admin.create_project` (FR-NEW-014/015) is independent** of steps 1-4 and may land in parallel.
6. **Contract regeneration last.** `TOOL_CONTRACT.txt`/`tool-contract-golden.json` regeneration
   (§9.1, §9.4) happens only once every tool/schema change above is final, never mid-sequence, or
   the golden-contract test fails against a partially-updated tool set.

No feature flag is needed: every step above is additive and the existing suite stays green
throughout (confirmed §2.3 — no existing test requires modification).

#### E2E-NEW-463: picker with zero memberships
- **Category:** GUI / Edge
- **Scenario:** SC-006
- **Requirements:** FR-NEW-018
- **Driver:** HTTP client
- **Preconditions:** `dave` is authenticated but a member of no project.
- **Steps:**
  - Given `dave`'s authenticated session, zero memberships.
  - When `dave` performs `GET /app/trash` (no `mount_id`).
  - Then the response is `200` with an explicit "you are not a member of any project" message
    (or equivalent), not an error.
- **Priority:** Low

## 15. Open Questions & TBDs

None outstanding — all gaps identified during Phase 4 test design were resolved and folded into
FR-NEW-005/006/013/015/016 and the Decisions Log below.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| Trash entry | A `trash_entries` row tracking one soft-deleted top-level path's original location, deletion time and deleter | Trash tracking |
| Flatten | Replacing every `/` with `__` when building a trash destination from an original path (`safety.rs:177-182`) | File lifecycle |
| Backfill | The lazy, idempotent insertion of a `trash_entries` row for a trash-directory node that predates this feature | Trash tracking |
| Purge sweep | The existing scheduled process (`purge.rs`) that permanently removes trashed files/projects past retention | File lifecycle |
| `PurgeConfig` | The per-project struct controlling auto-purge gating and retention windows (`storage/traits.rs:119-124`) | Project provisioning |

## 17. Decisions Log

- **DEC-001:** New files/sweep both write a `trash_entries` row rather than reconstructing
  `original_path` from the trash path string going forward. **Rationale:** the existing flatten
  encoding is lossy/ambiguous (`/a/b.txt` and `/a__b.txt` collide); an explicit row removes the
  ambiguity for all future deletes. **Alternatives considered:** reverse the flatten at read time
  (rejected: silently wrong for `__`-containing paths); change the flatten encoding to be reversible
  (rejected: breaks the frozen `trash_path` sample in `TOOL_CONTRACT.txt:600`). **Implemented by:**
  FR-NEW-001, FR-NEW-002, FR-NEW-003. **Round:** 3b. **Code evidence:** `safety.rs:177-182`,
  `fs_ops.rs:1019-1025`.
- **DEC-002:** Trash-write collisions retry with `~N`; restore-destination collisions retry with
  `_restoredN` and never fail. **Rationale:** user decision — restore should never block on a
  collision; the two suffix schemes are kept distinct (one internal/write-side, one
  user-facing/restore-side) rather than unified, since they solve different problems at different
  layers. **Alternatives considered:** silently overwrite on restore collision (rejected by the
  existing no-clobber convention everywhere else in the tool surface); fail restore on collision
  (rejected explicitly by the user). **Implemented by:** FR-NEW-004, FR-NEW-005, FR-NEW-006,
  FR-NEW-010. **Round:** 1, Q5.
- **DEC-003:** `trash_entries` starts empty; legacy trashed files are backfilled lazily and
  idempotently on first `fs.trash_list`/`fs.trash_restore` touch, with best-effort `deleted_at`
  (parsed epoch prefix, else now). **Rationale:** user decision — no separate startup migration job,
  acceptable to use "at worst the current date" for files that predate this feature.
  **Alternatives considered:** a one-time startup backfill job (rejected: unnecessary infra for data
  that's naturally swept up on first access). **Implemented by:** FR-NEW-011. **Round:** 3 (impact
  analysis follow-up).
- **DEC-004:** No per-file ownership-based ACL; any project member can list/restore any trashed file
  in that project. **Rationale:** user decision (Round 1, Q4, option A) — no per-file attribution
  exists anywhere in the codebase today (`NodeRow` has no owner column), so restricting restore to
  the deleter would require new schema work out of proportion to this increment. **Alternatives
  considered:** owner/admin-only restore (option B); new per-file attribution with restricted restore
  (option C) — both rejected by the user. **Implemented by:** FR-NEW-016. **Round:** 1, Q4.
- **DEC-005:** Platform admin status grants no implicit file access; the existing `state.authorize`
  membership gate applies unchanged to the new tools. **Rationale:** matches the existing documented
  convention (`state.rs:57-58`) with no exception carved out for trash. **Implemented by:**
  FR-NEW-016. **Round:** 2 (confirmed from existing code, not re-interviewed).
- **DEC-006:** `admin.create_project` gains the same four optional purge params as
  `admin.set_purge_config`, defaulting to today's `PurgeConfig::default()` when omitted.
  **Rationale:** closes the backlog's explicit ask ("configured per project at creation time");
  mirroring the existing tool's param names/types/validation avoids inventing a second convention.
  **Alternatives considered:** a separate `admin.create_project_with_retention` tool (rejected: an
  unnecessary second entry point for one project-creation operation). **Implemented by:**
  FR-NEW-014, FR-NEW-015. **Round:** 1/3, Q1.
- **DEC-007:** No `admin.get_purge_config` MCP tool is added. **Rationale:** YAGNI — nothing in this
  document's requirements needs it; `fs.trash_list`'s `purge_in_days` is computed server-side from
  the internal config read, and the GUI never needs to display raw `PurgeConfig` values. **Implemented
  by:** n/a — no code impact; deliberately out of scope (§3.2). **Round:** Phase 0 synthesis.
- **DEC-008:** `fs.trash_list`'s collision-exhaustion and unrelated errors reuse `ERR_INTERNAL_ERROR`;
  no new `ERR_*` code is added. **Rationale:** this is an exceedingly rare operational failure (50
  consecutive same-millisecond collisions), not a distinct user-facing error class worth its own
  code, consistent with how the existing error model reserves a dedicated code only for conditions a
  caller is expected to branch on. **Implemented by:** FR-NEW-005. **Round:** Phase 4.6 gap
  resolution.

## 18. Implementability Gate

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 3 (2 BLOCKING: G5 missing implementation order, G4 `/app/trash` with no `mount_id` undecided; 1 MINOR: FR-NEW-007 cross-referenced FR-NEW-010 instead of FR-NEW-011) | 2 (both resolved in place, not registered) | NOT-IMPLEMENTABLE |

**Amendments applied:** FR-NEW-017 (required build order, §6/§14) and FR-NEW-018 (`/app/trash`
with no `mount_id` renders a project picker, §6) added as new numbered requirements, with
E2E-NEW-462 and its traceability-matrix/coverage-statistics updates; the FR-NEW-007 cross-reference
typo corrected from FR-NEW-010 to FR-NEW-011. Both A findings were evident corrections applied
directly: §4's persona wording fixed (platform admin, not project owner, gates
`admin.create_project`, per `mcp/server.rs:2085`), and §9.1 gained the two Affected Components rows
for `mcp/server.rs` and `storage/traits.rs` that FR-NEW-014/015 actually touch.
**Drift registered:** none — both A findings were evident and corrected in place rather than
deferred to a register entry.

| 2 | 1 (BLOCKING: FR-NEW-018 named the wrong membership source and never specified a zero-membership empty state) | 2 (both MINOR, stale line citations introduced by round 1's own amendment) | NOT-IMPLEMENTABLE |
| 3 | 0 | 2 (both MINOR, applied in place: a malformed table row and the unnamed collision-retry helper symbol) | IMPLEMENTABLE |

**Amendments applied (round 2):** FR-NEW-018 corrected to source the picker from
`AdminBackend::list_projects_for` (`storage/traits.rs:209`) instead of the non-reusable
`deleted_projects_screen.rs` membership check, with an explicit zero-membership empty-state clause
added; E2E-NEW-463 added to test it, with matrix/coverage updates. Both A findings (stale
`mcp/server.rs` line citations on `CreateProjectArgs` and `require_admin`, introduced by round 1's
own amendments) corrected in place: `CreateProjectArgs` at `mcp/server.rs:1122`,
`admin_create_project` at `mcp/server.rs:2079`, `require_admin` at `mcp/server.rs:2084`.
**Drift registered (round 2):** none — both A findings were evident and corrected in place.
