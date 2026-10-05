# mcp-fs-rust — Specification Document

> Generated on: 2026-10-04
> Id: SPEC-0014
> Nature: FEAT
> Depth: L
> Depth evidence: new persisted schema required (no last-access tracking exists today: `nodes.atime` at `storage/meta.rs:133` is declared but never written), scope spans storage/rel, core/fs_ops, tools/admin, cli and a new browser screen (4+ modules). Per the depth rule, either signal alone forces L.
> Status: Draft
> Type: Evolution Specification
> From backlog: BL-0001
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Executive Summary

Adds automatic, age-based removal of unused files and stale projects. Activates the
currently-dead `nodes.atime` column as a real last-access signal, adds a background sweep
loop plus a standalone `mcp-fs purge` CLI verb (either or both drivable per project), and
introduces a two-phase project lifecycle — soft-delete (reversible, admin-visible,
inaccessible to normal callers) then, after a global grace period, permanent removal via
the existing project-deletion cascade. Audience: the platform admin configuring retention,
and indirectly every project member whose unused data this reclaims.

## 2. Current State

### 2.1 How it works today

- `nodes.atime` exists in the schema (`storage/meta.rs:117-153`, column declared at
  `meta.rs:133`) but no code path anywhere writes it. It is read back verbatim in
  `core/fs_ops.rs:246` (the `stat` response) but always equals its value at node creation.
  "Last used" has no real data today.
- Project deletion is immediate, manual, and cascading: `admin.delete_project`
  (`tools/admin.rs:228`), specified as FR-027 in `specs/SPEC-0002.../spec.md:494-499`
  ("Project deletion cascades"). There is no soft-delete state for a project — `project`
  (`storage/admin.rs:28-68`) has only `id, owner, created_at, index_mode`.
- File soft-delete ("trash") already exists and is the default delete mode
  (`safety.rs:187-191`, `trash_path`): a metadata-only rename into `/{trash_dir}/...`,
  blob and refcount untouched. It is permanent — nothing ever empties `trash_dir`.
  `allow_hard_delete` (`config.rs:359-377`) gates true hard deletion, default `false`.
- No retention, TTL, purge or scheduling mechanism exists anywhere: `SafetyConfig`
  (`config.rs:359-377`) has exactly `write_quota_bytes, trash_dir, read_guard,
  allow_hard_delete, max_read_lines`; `cli.rs` has `Serve | Keys | Token | Migrate |
  Version`, no scheduling verb. Background work in this codebase is ad hoc
  `tokio::spawn`: the search indexer (`search/indexer.rs:16-23`, fire-and-forget on
  writes/deletes), the OAuth device-flow poller (`tools/git_auth.rs:386`), and per-editor
  live-reload tasks (`tools/editor.rs:264,277`).
  None of these write to the database from a detached task — the "best-effort async DB
  write" this spec needs is a new mechanism, not a reused one (see DRIFT-001).
- The membership gate `state.rs:59 authorize()` calls `storage/admin.rs require_member`,
  and is the single point both the MCP dispatch (`mcp/server.rs`) and the REST plane
  (`api/dataplane.rs:191`, `state.authorize(&mount, &person)`) go through — confirmed by
  reading both call sites, not assumed.
- `admin.list_projects` (`tools/admin.rs:247` → `storage/admin.rs:236
  list_projects_for(person)`) is membership-filtered per caller, not platform-admin-only —
  the convention this spec's new listing tool follows (FR-NEW-009).

### 2.2 Existing specifications governing this area

- `specs/SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md` FR-027 — project
  deletion cascade, reused unchanged by the grace-period sweep (FR-NEW-006). Not modified.
- `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` FR-610 — restates FR-027's git clause;
  likewise reused unchanged, and inherited for free by the `fs.*`/`git.*` access gate
  (FR-NEW-008) since git tools share the same `authorize()` path.
- No existing specification covers storage quota, retention or usage tracking. Backlog
  entries **BL-0002** (storage quota) and **BL-0003** (soft-delete trash retention) are
  adjacent, unspec'd ideas explicitly out of scope here (see 3.2).

### 2.3 Existing test coverage

- `make test` / `cargo test --workspace` is the exact command.
- Project deletion: `tools/admin.rs:854` (round-trip), `:711` (non-member forbidden),
  `:920` (owner self-delete); storage cascade `storage/admin.rs:442`; cross-dialect
  conformance `storage/conformance.rs:367,412`; git-tied teardown `tools/git.rs:14986`.
- File deletion / trash: `core/fs_ops.rs:2639` (soft delete into trash), `:2676`
  (recursive-needed), `:2685` (missing path); refcount/GC in `storage/meta.rs:921-926,
  956-958, 1065, 1092-1096, 1201-1209`; `storage/volume.rs:258,263,301`.
- None of this covers atime, purge, retention or project soft-delete — this increment is
  wholly new surface, unverifiable by anything existing.

## 3. Scope

### 3.1 In Scope
- Real `atime` tracking (read and write both bump it).
- Per-project purge configuration (enable flag, internal-vs-external driver choice, two
  independent age thresholds).
- Background sweep loop and standalone `mcp-fs purge` CLI verb.
- Two-phase project lifecycle: soft-delete (reversible) → grace period → permanent removal.
- `admin.list_deleted_projects`, `admin.undelete_project`, a browser screen for both.

### 3.2 Out of Scope (Non-Goals)
- Storage quotas (BL-0002) — separate backlog entry.
- Any change to file-trash retention/expiry semantics (BL-0003) — purged files land in the
  existing, unmodified trash mechanism; permanent trash cleanup is BL-0003's job later.
- Any UI beyond the one new deleted-projects screen.
- Per-project override of the grace period (it is a single global value).

## 4. User Personas & Actors

- **Platform admin**: configures per-project purge settings, runs on-demand CLI purge,
  views and undeletes soft-deleted projects.
- **Project member/owner**: passively affected — their unused files or stale project may be
  purged; regains access immediately on undelete.
- **Internal purge loop**: a background, in-process actor (`tokio::spawn` + interval).
- **External operator / cron**: drives `mcp-fs purge` from outside the server process.

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| Access tracking | Real `atime`/`mtime` maintenance on every read/write | `nodes.atime`, `nodes.mtime` |
| Purge configuration | Per-project retention settings | `project_purge_config` (new) |
| Purge execution | The sweep logic shared by the loop and the CLI | file sweep, project-staleness sweep, grace sweep |
| Project lifecycle | Soft-delete / undelete / permanent removal | `project.deleted_at` (new) |
| Admin surface | Tools + browser screen to observe and reverse soft-deletes | `admin.list_deleted_projects`, `admin.undelete_project`, `/app/deleted-projects` |

## 5. Usage Scenarios

### SC-001: Admin configures autopurge on a project
**Actor:** platform admin
**Preconditions:** admin is owner or platform admin of the target project.
**Flow:**
1. Admin calls `admin.set_purge_config(project_id, autopurge_enabled, use_internal_purge,
   file_retention_days?, project_retention_days?)`.
2. System validates role and argument values, persists the config, returns it.
**Postconditions:** the project's purge config row reflects the call; no purge runs as a
side effect of configuring.
**Exceptions:**
- EXC-001a: non-owner/non-admin caller → `ERR_FORBIDDEN`.
- EXC-001b: a retention value present and `<= 0` → `ERR_INVALID_ARGUMENT`.
- EXC-001c: unknown project → `ERR_PROJECT_NOT_FOUND`.
**Cross-scenario notes:** config set here gates SC-002/SC-003/SC-004.

### SC-002: Internal purge loop — file purge
**Actor:** internal purge loop
**Preconditions:** project has `autopurge_enabled && use_internal_purge &&
file_retention_days.is_some()`.
**Flow:**
1. Loop wakes on `safety.purge_interval_secs`.
2. For each qualifying project, finds files where `now - atime > file_retention_days`.
3. Soft-deletes each such file into the existing trash, one commit per file.
**Postconditions:** trashed files are absent from the live tree, present in trash,
refcounts untouched (existing trash semantics, unmodified).
**Exceptions:**
- EXC-002a: DB unavailable mid-sweep → that file is skipped, retried next cycle.
- EXC-002b: a file read lands the instant before the sweep examines it → its `atime` was
  just bumped, so it is correctly not purged (no race, by construction of per-file atime
  checks).
- EXC-002c: a file already trashed by an earlier sweep or a manual `fs.delete` → purging it
  again is a no-op, not an error.
**Cross-scenario notes:** runs before SC-003 in the same cycle, since project staleness in
SC-003 is computed over the *post-file-purge* live file set.

### SC-003: Internal purge loop — project soft-delete
**Actor:** internal purge loop, same cycle as SC-002
**Preconditions:** `autopurge_enabled && use_internal_purge && project_retention_days.is_some()`.
**Flow:**
1. Compute the staleness reference: `max(project.created_at, max(atime) over the
   project's live files)` (falls back to `created_at` alone if the project has no live
   files).
2. If `now - reference > project_retention_days`, set `project.deleted_at = now`, via a
   conditional `UPDATE ... WHERE deleted_at IS NULL`.
**Postconditions:** project inaccessible to normal `fs.*`/`git.*` calls; visible in
`admin.list_deleted_projects`; all underlying data untouched.
**Exceptions:**
- EXC-003a: a project with zero files ever written is eligible from `created_at` alone —
  a never-touched project outlives its retention window exactly like a stale one.
- EXC-003b: the loop runs twice before the admin acts — the second run's conditional
  update is a no-op; `deleted_at` keeps its original value, not bumped to a later "now".
**Cross-scenario notes:** recent activity on any one file resets the whole project's
staleness clock, even for an old project (the `max` term dominates).

### SC-004: CLI on-demand purge
**Actor:** external operator / cron
**Preconditions:** none beyond the target project (if named) existing.
**Flow:**
1. `mcp-fs purge --project <id> [--on-demand]`.
2. If the project's `autopurge_enabled` is false and `--on-demand` is absent → fail, no
   action taken.
3. Otherwise, run the same file-sweep and project-staleness logic as SC-002/SC-003
   synchronously for that project, print a summary.
4. `mcp-fs purge` with no `--project` instead runs the grace-period sweep (SC-007) globally.
**Postconditions:** same as SC-002/SC-003, or SC-007 for the no-`--project` form.
**Exceptions:**
- EXC-004a: named project does not exist → non-zero exit, `ERR_PROJECT_NOT_FOUND`-equivalent
  message.
- EXC-004b: `--on-demand` passed on an already-autopurge-enabled project → accepted,
  redundant, not an error.
**Cross-scenario notes:** the no-`--project` global sweep acts on every soft-deleted
project regardless of `autopurge_enabled`, since a project already soft-deleted is past
the autopurge decision point entirely.

### SC-005: Admin lists soft-deleted projects
**Actor:** platform admin (or any owner/member, filtered to their own, per the
`list_projects_for` convention)
**Flow:**
1. `admin.list_deleted_projects()` (MCP tool) or the `/app/deleted-projects` browser screen.
2. Returns `{project_id, owner, deleted_at, days_until_permanent_removal}` per visible
   soft-deleted project, filtered the same way `admin.list_projects` is.
**Exceptions:** none beyond standard auth; empty result when nothing is soft-deleted.

### SC-006: Admin undeletes a project
**Actor:** platform admin or project owner
**Flow:**
1. `admin.undelete_project(project_id)` (MCP tool), or the browser screen's undelete
   action (CSRF-protected, same convention as `token_screen.rs`).
2. Clears `deleted_at` to `NULL`. Project is immediately accessible again; `atime` values
   are untouched, so an idle project can become stale again quickly.
**Exceptions:**
- EXC-006a: project not currently soft-deleted → `ERR_INVALID_ARGUMENT`.
- EXC-006b: project does not exist at all (already permanently removed, or never existed)
  → `ERR_PROJECT_NOT_FOUND`.
- EXC-006c: non-owner, non-admin caller → `ERR_FORBIDDEN`.

### SC-007: Grace-period sweep → permanent removal
**Actor:** internal purge loop (unconditionally, regardless of `use_internal_purge` — a
soft-deleted project is already committed to removal) and the CLI's no-`--project` form.
**Flow:**
1. For every soft-deleted project where `now - deleted_at > project_purge_grace_days`
   (global config), call the existing `admin.delete_project` cascade unchanged.
**Postconditions:** project and all its data permanently gone, identical to a manual
FR-027 delete.
**Exceptions:**
- EXC-007a: cascade failure mid-way → surfaces exactly as FR-027 already defines.

## 6. Functional Requirements

### New Requirements

#### FR-NEW-001 [EARS-E]: Content reads bump `atime`
> WHEN a content-reading tool or REST route (`fs.read`, `fs.read_bytes`, `fs.read_lines`,
> `fs.read_section`, `fs.head`, `fs.tail`, `fs.extract_text`, `fs.grep`, and their REST
> GET equivalents) completes successfully THE system SHALL update the node's `atime` to
> the current time, asynchronously and best-effort (a failed bump never surfaces as a
> tool error).

- **Exact names:** `nodes.atime` (existing column, `storage/meta.rs:133`).
- **Business Rules:** "Live file" (used throughout FR-NEW-007/008) excludes any node
  whose path falls under the project's `trash_dir` subtree, matched via the same
  `descendant_pattern`/`Dialect::escape_like_literal` convention as every other subtree
  query. A trashed node is never a purge target and never contributes to the
  `max(atime)` staleness calculation in FR-NEW-008.
- **Priority:** Must-have.

#### FR-NEW-002 [EARS-E]: Writes bump both `atime` and `mtime`
> WHEN a mutating tool or REST route (`fs.write`, `fs.edit`, `fs.copy`, `fs.move`, and
> every other content-mutating `fs.*` tool and its REST equivalent) completes
> successfully THE system SHALL update the node's `atime` and `mtime` to the current
> time, via the same asynchronous best-effort mechanism as FR-NEW-001.

- **Priority:** Must-have.

#### FR-NEW-003 [EARS-UB]: Metadata/listing ops never touch `atime`
> The system SHALL NOT update `atime` as a result of `fs.stat`, `fs.glob`, `fs.list`,
> `fs.tree`, or any `admin.*` listing tool.

- **Priority:** Must-have.

No REST route is added for these three admin tools: no existing `admin.*` tool has a
REST equivalent today (`api/dataplane.rs:103-147` routes only `fs.*` operations;
`admin.create_project`/`delete_project`/`list_projects`/`set_index_mode` are MCP-only,
`mcp/server.rs:2039-2280`). This spec does not break that convention. The browser screen
(FR-NEW-017) calls the tool functions in-process, not over a REST route.

#### FR-NEW-004 [EARS-E]: `admin.set_purge_config`
> WHEN an owner or platform admin calls `admin.set_purge_config(project_id,
> autopurge_enabled: bool, use_internal_purge: bool, file_retention_days: Option<u32>,
> project_retention_days: Option<u32>)` with every present retention value `> 0` THE
> system SHALL persist the configuration and return it.

- **Exact names:** tool name `admin.set_purge_config`; new table/columns
  `project_purge_config(project_id, autopurge_enabled, use_internal_purge,
  file_retention_days, project_retention_days)`.
- **Business Rules:** either retention field SHALL be permitted to be `None`/absent
  (that axis disabled independently of the other).
- **Priority:** Must-have.

#### FR-NEW-005 [EARS-O]: Rejected `set_purge_config` arguments
> IF a present `file_retention_days` or `project_retention_days` is `<= 0` THEN THE
> system SHALL return `ERR_INVALID_ARGUMENT` and persist nothing.

- **Priority:** Must-have.

#### FR-NEW-006 [EARS-O]: `set_purge_config` authorization and existence
> IF the caller is neither the project owner nor a platform admin THEN THE system SHALL
> return `ERR_FORBIDDEN`; IF the project does not exist THEN THE system SHALL return
> `ERR_PROJECT_NOT_FOUND`.

- **Priority:** Must-have.

#### FR-NEW-007 [EARS-S]: Internal file-purge sweep
> WHILE a project has `autopurge_enabled && use_internal_purge &&
> file_retention_days.is_some()` THE system SHALL, on every background sweep cycle,
> soft-delete into the existing trash every live file whose `now - atime >
> file_retention_days`, committing each file independently (a failure on one file SHALL
> NOT block any other file in the same cycle).

- **Business Rules:** re-purging an already-trashed file is a no-op.
- **Priority:** Must-have.

#### FR-NEW-008 [EARS-S]: Internal project soft-delete sweep
> WHILE a project has `autopurge_enabled && use_internal_purge &&
> project_retention_days.is_some()` THE system SHALL, on every background sweep cycle
> after FR-NEW-007 has run for that project, soft-delete the project (set
> `project.deleted_at = now` via `UPDATE ... WHERE deleted_at IS NULL`) when `now -
> max(project.created_at, max(atime) over its live files) > project_retention_days`.

- **Exact names:** new nullable column `project.deleted_at`.
- **Business Rules:** falls back to `created_at` alone when the project has no live
  files; the conditional update makes a repeated trigger a no-op, never advancing
  `deleted_at`; soft-delete never deletes any underlying data.
- **Priority:** Must-have.

#### FR-NEW-009 [EARS-U]: Background sweep loop
> The system SHALL run the FR-NEW-007/FR-NEW-008 sweep, plus the FR-NEW-012
> grace-period sweep, on a fixed interval given by `safety.purge_interval_secs`, via a
> detached background task started at server boot and stopped cleanly on shutdown.

- **Priority:** Must-have.

#### FR-NEW-010 [EARS-U]: CLI `mcp-fs purge`
> The system SHALL provide a `purge` CLI verb accepting an optional `--project <id>` and
> an optional `--on-demand` flag.

- **Priority:** Must-have.

#### FR-NEW-011 [EARS-O]: CLI purge gating and global mode
> IF `--project <id>` is given and that project's `autopurge_enabled` is `false` and
> `--on-demand` is absent THEN THE system SHALL exit non-zero with no action taken. IF
> `--project <id>` is given and (`autopurge_enabled` is `true` OR `--on-demand` is
> present) THEN THE system SHALL run the FR-NEW-007/FR-NEW-008 logic synchronously for
> that project and print a summary. IF `--project` is absent THEN THE system SHALL run
> the FR-NEW-012 grace-period sweep across every soft-deleted project, regardless of
> their `autopurge_enabled`.

- **Priority:** Must-have.

#### FR-NEW-012 [EARS-E]: Grace-period permanent removal
> WHEN a soft-deleted project's `now - deleted_at > project_purge_grace_days` (global
> config) THE system SHALL permanently remove it via the existing `admin.delete_project`
> cascade (FR-027, `specs/SPEC-0002.../spec.md:494-499`), unmodified, regardless of that
> project's `use_internal_purge` value.

- **Priority:** Must-have.

#### FR-NEW-013 [EARS-E]: Blocked access to a soft-deleted project
> WHEN any `fs.*` or `git.*` tool, or its REST equivalent, is called against a project
> with `deleted_at` non-null THE system SHALL return `ERR_PROJECT_NOT_FOUND`, enforced in
> `storage/admin.rs require_member` (called from `state.rs:59 authorize()`), so both the
> MCP and REST surfaces inherit it from the one shared gate.

- **Business Rules:** applies unconditionally, including to a platform admin (no implicit
  file access, per existing convention). The `deleted_at` filter SHALL be applied only
  inside `require_member` (the `fs.*`/`git.*` access gate) and nowhere else:
  `admin.get_project`, `admin.require_owner`, and `state.rs:47 require_owner_or_admin`
  SHALL continue to operate on a soft-deleted project's row unchanged, so that
  FR-NEW-004/006 (`set_purge_config`), FR-NEW-014 (`list_deleted_projects`) and
  FR-NEW-015/016 (`undelete_project`) keep working on exactly the soft-deleted rows they
  exist to manage.
- **Priority:** Must-have.

#### FR-NEW-014 [EARS-E]: `admin.list_deleted_projects`
> WHEN a caller invokes `admin.list_deleted_projects()` THE system SHALL return every
> soft-deleted project visible to that caller — filtered exactly as `admin.list_projects`
> filters by membership, with a platform admin seeing all — as
> `{project_id, owner, deleted_at, days_until_permanent_removal}`.

- **Business Rules:** `days_until_permanent_removal = project_purge_grace_days -
  floor((now - deleted_at) in days)`, not clamped below zero.
- **Priority:** Must-have.

#### FR-NEW-015 [EARS-E]: `admin.undelete_project`
> WHEN an owner or platform admin calls `admin.undelete_project(project_id)` on a project
> with `deleted_at` non-null THE system SHALL set `deleted_at` to `NULL` and return
> success.

- **Priority:** Must-have.

#### FR-NEW-016 [EARS-O]: `undelete_project` rejections
> IF the project's `deleted_at` is already `NULL` THEN THE system SHALL return
> `ERR_INVALID_ARGUMENT`; IF the project does not exist THEN THE system SHALL return
> `ERR_PROJECT_NOT_FOUND`; IF the caller is neither owner nor platform admin THEN THE
> system SHALL return `ERR_FORBIDDEN`.

- **Priority:** Must-have.

#### FR-NEW-017 [EARS-U]: Browser screen `/app/deleted-projects`
> The system SHALL serve an admin-session-gated `/app/deleted-projects` screen, following
> the same auth and CSRF conventions as the existing `/app/tokens` screen
> (`token_screen.rs`), listing every soft-deleted project visible to the caller with an
> undelete action that calls FR-NEW-015 (never a second implementation).

- **Priority:** Must-have.

#### FR-NEW-018 [EARS-U]: New global configuration
> The system SHALL add `safety.purge_interval_secs` (default `3600`) and
> `safety.project_purge_grace_days` (default `30`) to `SafetyConfig`.

- **Priority:** Must-have.

## 7. Non-Functional Requirements

### 7.1 Performance
No specific load target (personal-project scale). The sweep iterates all qualifying
projects/files per cycle; acceptable at current scale, no pagination needed now.

### 7.2 Security
No new authentication mechanism. `admin.set_purge_config`, `admin.list_deleted_projects`,
`admin.undelete_project` reuse the existing bearer-JWT + `require_owner_or_admin`/
membership-filter gates. The browser screen reuses `token_screen.rs`'s session-cookie +
CSRF convention exactly.

### 7.3 Usability
The new browser screen is the only UI surface; it mirrors `/app/tokens`'s look and
interaction pattern for consistency.

### 7.4 Reliability
No retry beyond "the next sweep cycle picks up anything missed" (per-file/per-project
idempotent commits already make this safe). No rollback mechanism needed since every
operation is independently idempotent.

### 7.5 Observability
This codebase has no OpenTelemetry collector anywhere — only `tracing` to stderr
(`logging.rs:34`, `RUST_LOG` env filter). This spec follows that existing convention
rather than introducing a collector: sweep start/summary at INFO, each file/project
mutation at DEBUG, failures at ERROR — consistent with how other write-mutations log
today.

### 7.6 Deployment
No infrastructure change. The sweep loop runs in-process via `tokio::spawn`, matching the
search indexer's existing pattern (`search/indexer.rs:16-23`). The CLI verb ships in the
existing `mcp-fs` binary. No new crate dependency.

### 7.7 Scalability
Full-set sweep per cycle; fine at current scale. Revisit if project/file counts grow by
orders of magnitude (not anticipated now — explicitly out of scope).

## 8. Data Model

- `project.deleted_at: Option<Timestamp>` — new nullable column, added via the existing
  idempotent `column_migration()` path (same technique as `project.index_mode`,
  `storage/admin.rs:63-70`).
- `project_purge_config` — new table (or columns on `project`, implementation's choice,
  not prescribed): `project_id` (PK/FK → `project.id`), `autopurge_enabled: bool`,
  `use_internal_purge: bool`, `file_retention_days: Option<u32>`,
  `project_retention_days: Option<u32>`.
- `nodes.atime` — existing column, now actually written (no shape change).
- `SafetyConfig.purge_interval_secs: i64` (default `3600`),
  `SafetyConfig.project_purge_grace_days: i64` (default `30`) — new fields.

The `volume_id`-in-every-key convention (AGENTS.md) applies only to the per-volume
metadata tree (`nodes`, `blob_refs`, `git_*`) — `nodes.atime` is already scoped that way
and needs no change. It does NOT apply to `project` or `project_purge_config`: the
admin registry is global to the deployment and carries no `volume_id` column at all
(`storage/admin.rs:1-6`, `:30-36` — `project(id, owner, created_at)`, no such column).
`project.deleted_at` and `project_purge_config` key off `project_id` alone, exactly like
the existing `project_member` table. New columns go through the dialect checklist in
`.agent_docs/backends.md` (declared once in `storage/rel/schema.rs`, never hand-built
per-dialect DDL).

## 9. Impact Analysis

### 9.1 Affected Components
| File/Module | Impact | Description |
|---|---|---|
| `storage/rel/schema.rs` | Modified | New `project.deleted_at` column, new `project_purge_config` table/columns |
| `storage/admin.rs` | Modified | `require_member` gate checks `deleted_at`; new queries for config/list/undelete |
| `storage/meta.rs` | Modified | Atime-update query |
| `core/fs_ops.rs` | Modified | Wire the async atime/mtime bump into read and write ops |
| `tools/admin.rs` | Modified | Three new tool functions |
| `mcp/server.rs` | Modified | Three new `#[tool]` methods |
| `purge.rs` (new) | New | Shared sweep logic used by both the loop and the CLI |
| `cli.rs` | Modified | New `Purge` verb |
| `app.rs` | Modified | Spawn the background loop; mount the new browser screen |
| `deleted_projects_screen.rs` (new) | New | Browser screen, modeled on `token_screen.rs` |
| `config.rs` | Modified | Two new `SafetyConfig` fields |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | Modified | Regenerate (ASSUMED: 94 existing tools per `AGENTS.md`'s header count → 97 after adding the 3 new `admin.*` tools in FR-NEW-004/014/015) |

### 9.2 Affected Requirements
| Spec | Requirement ID | Impact | Description |
|---|---|---|---|
| SPEC-0002 | FR-027 | Referenced, unmodified | Reused verbatim by FR-NEW-012's grace-period removal |
| SPEC-0007 | FR-610 | Referenced, unmodified | Inherited for free via the shared `authorize()` gate |

### 9.3 Affected Tests
| Test File | Test | Action | Description |
|---|---|---|---|
| — | — | None modified/removed | Pure addition; see Section 12 for the new suite |

### 9.4 Affected Documentation
| Document | Section | Action | Description |
|---|---|---|---|
| `AGENTS.md` | tool count, module list, "Behaviour worth knowing" | Update | ASSUMED: 94→97 tools (per `AGENTS.md`'s own header count), atime goes live, new modules |
| `.agent_docs/tools.md` | tool reference | Update | Document the 3 new `admin.*` tools (ASSUMED: 94 → 97 total, per `AGENTS.md`'s header count) |
| `.agent_docs/config.md` | config schema | Update | Document the 2 new `SafetyConfig` keys |
| `.agent_docs/architecture.md` | request lifecycle | Update | New background loop, new browser screen |

### 9.5 Dependencies & Risks
No new crate dependencies. No breaking changes — new columns default to inactive/null, no
migration of existing data required. Risk: atime write-amplification on every read,
mitigated by the async best-effort design (DEC in Section 17). No rollback plan needed
beyond reverting the migration (new columns, additive only).

## 10. Documentation Requirements

`AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/config.md`, `.agent_docs/architecture.md`
as listed in 9.4. `TOOL_CONTRACT.txt` and `tool-contract-golden.json` regenerated via
`MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`,
diff reviewed before commit.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E (Happy) | E2E (Failure) | E2E (Edge) |
|---|---|---|---|---|
| SC-001 | FR-NEW-004, FR-NEW-005, FR-NEW-006 | E2E-NEW-015, E2E-NEW-016, E2E-NEW-017 | E2E-NEW-018, E2E-NEW-019, E2E-NEW-020 | E2E-NEW-021, E2E-NEW-022, E2E-NEW-023 |
| SC-002 | FR-NEW-001, FR-NEW-003, FR-NEW-007 | E2E-NEW-001, E2E-NEW-002, E2E-NEW-024 | E2E-NEW-005, E2E-NEW-013, E2E-NEW-027 | E2E-NEW-025, E2E-NEW-026, E2E-NEW-028, E2E-NEW-029, E2E-NEW-030, E2E-NEW-031 |
| SC-003 | FR-NEW-008 | E2E-NEW-032 | E2E-NEW-037 | E2E-NEW-033, E2E-NEW-034, E2E-NEW-035, E2E-NEW-036, E2E-NEW-038, E2E-NEW-039 |
| SC-004 | FR-NEW-010, FR-NEW-011 | E2E-NEW-046, E2E-NEW-048 | E2E-NEW-047, E2E-NEW-050 | E2E-NEW-051, E2E-NEW-052 |
| SC-005 | FR-NEW-014 | E2E-NEW-058, E2E-NEW-059 | E2E-NEW-060 | E2E-NEW-061, E2E-NEW-062 |
| SC-006 | FR-NEW-015, FR-NEW-016 | E2E-NEW-063, E2E-NEW-064 | E2E-NEW-065, E2E-NEW-066, E2E-NEW-067 | E2E-NEW-068 |
| SC-007 | FR-NEW-012 | E2E-NEW-049, E2E-NEW-053 | E2E-NEW-080 | E2E-NEW-054, E2E-NEW-055, E2E-NEW-056, E2E-NEW-057 |
| (cross-cutting) | FR-NEW-002, FR-NEW-009, FR-NEW-013, FR-NEW-017, FR-NEW-018 | E2E-NEW-010..012, E2E-NEW-043, E2E-NEW-069, E2E-NEW-074..075, E2E-NEW-077 | E2E-NEW-014, E2E-NEW-040..042, E2E-NEW-070 | E2E-NEW-006..009, E2E-NEW-044..045, E2E-NEW-071..073, E2E-NEW-076, E2E-NEW-078..079 |

## 12. End-to-End Test Suite

> This is the contract. 80 tests, E2E-NEW-001..080, full Given/When/Then detail designed
> by an independent test-designer sub-agent against this document's requirement and
> scenario set, reviewed and corrected against two verified facts (G2, G5 below) before
> inclusion. The full per-test table (driver, exact values, side-effect verification
> method) lives in the design transcript; summarized here by area with every requirement
> and scenario traced in Section 11.

### 12.1 Test Summary

| Area | Test IDs | Count | Categories covered |
|---|---|---|---|
| Content-read bumps `atime` only | E2E-NEW-001..009 | 9 | happy, failure, side-effect, concurrency, best-effort semantics, state (metadata exempt) |
| Write bumps both | E2E-NEW-010..014 | 5 | happy, failure, partial-op |
| `admin.set_purge_config` | E2E-NEW-015..023 | 9 | happy (incl. a 2nd happy-path variant, since no REST route exists per the Section 9.1 correction), forbidden, not-found, boundary, data-variation, idempotency |
| Internal file-purge sweep | E2E-NEW-024..031 | 8 | happy, boundary, gating, partial-commit, idempotency, empty-state |
| Internal project soft-delete sweep | E2E-NEW-032..039 | 8 | happy, boundary, `max()` semantics, gating, idempotency, side-effect (no data loss) |
| `fs.*`/`git.*` access gate | E2E-NEW-040..045 | 6 | failure (per-tool), REST parity, control (live project), admin-no-bypass, re-check-on-undelete |
| CLI `mcp-fs purge` | E2E-NEW-046..052 | 7 | happy, gating failure, override, global sweep, not-found, empty-state |
| Grace-period sweep | E2E-NEW-053..057, 080 | 6 | happy, boundary, unconditional-on-`use_internal_purge`, cascade-parity, failure |
| `admin.list_deleted_projects` | E2E-NEW-058..062 | 5 | happy (incl. a 2nd happy-path variant, since no REST route exists per the Section 9.1 correction), filter-scoping, empty-state, boundary (countdown) |
| `admin.undelete_project` | E2E-NEW-063..068 | 6 | happy, invalid-argument, not-found, forbidden, re-access transition |
| Browser screen | E2E-NEW-069..073 | 5 | happy, auth-failure, action-parity, CSRF, empty-state |
| Config | E2E-NEW-074..076 | 3 | defaults, override, boundary |
| Loop lifecycle | E2E-NEW-077..079 | 3 | end-to-end (real interval), graceful shutdown, scale |

**Coverage statistics:** happy 27, non-happy (failure + edge + side-effect + idempotency
combined) 53. **Ratio ≈ 1.96:1**, beats 1:1. Every requirement (FR-NEW-001..018) has ≥3
tests; every scenario (SC-001..007) has ≥5, each with at least one happy, one failure
and one edge test.

### 12.2 New Test Specifications (representative; full Given/When/Then per test id carried
in the design transcript, referenced here so `/implement` can request the complete table)

#### E2E-NEW-001: Content read bumps atime
- **Category:** Core Journey
- **Scenario:** SC-002 (precondition-setting)
- **Requirements:** FR-NEW-001
- **Driver:** direct MCP tool call via test harness
- **Preconditions:** file `a.txt` seeded with `atime = T0` via direct DB write
- **Steps:**
  - Given file `a.txt` with `atime = T0`
  - When `fs.read(mount_id, "a.txt")` succeeds
  - Then, polled within 2000ms, `node.atime > T0`
- **Priority:** Critical

#### E2E-NEW-007: Best-effort bump never surfaces as a tool error
- **Category:** Edge Case (best-effort semantics)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001
- **Driver:** direct function call with an injected failure on the detached bump task
- **Steps:**
  - Given the atime-bump background task is forced to fail (test double)
  - When `fs.read` is called
  - Then the read call itself still returns success to the caller
- **Priority:** Critical — this is the literal content of the "best-effort" wording.

#### E2E-NEW-028: Per-file commit, not all-or-nothing
- **Category:** Side Effect / Error Recovery
- **Scenario:** SC-002
- **Requirements:** FR-NEW-007
- **Driver:** direct call, sweep over 3 seeded stale files with the 2nd forced to fail
- **Steps:**
  - Given 3 stale files in one project, file 2's commit forced to fail (locked row)
  - When the sweep runs
  - Then file 1 is purged, file 2 is not, file 3 is still purged
- **Priority:** Critical

#### E2E-NEW-036: Project `max()` staleness semantics
- **Category:** Edge Case / State Transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-008
- **Driver:** direct call
- **Steps:**
  - Given `created_at = now - 200 days`, one live file `atime = now - 5 days`,
    `project_retention_days = 30`
  - When the sweep runs
  - Then the project is NOT soft-deleted (reference = `max(created_at, max atime) = now -
    5 days`, age 5 < 30)
- **Priority:** Critical — the crux of FR-NEW-008.

#### E2E-NEW-038: Repeated trigger does not advance `deleted_at`
- **Category:** Idempotency
- **Scenario:** SC-003
- **Requirements:** FR-NEW-008
- **Driver:** direct call, two sweep runs
- **Steps:**
  - Given the project already has `deleted_at` set from a prior sweep
  - When the sweep runs again on the same stale project
  - Then `deleted_at`'s value is unchanged (not bumped to a new "now")
- **Priority:** Critical

#### E2E-NEW-042: REST inherits the access gate automatically
- **Category:** Security / Cross-Scenario
- **Scenario:** cross-cutting
- **Requirements:** FR-NEW-013
- **Driver:** HTTP client against `/api/fs`
- **Steps:**
  - Given a soft-deleted project
  - When `GET /api/fs/{mount}/read?path=any.txt` is called
  - Then it fails with `ERR_PROJECT_NOT_FOUND`'s mapped HTTP status, proving the gate
    lives in the one shared `require_member`, not a duplicated REST-only check
- **Priority:** Critical — verified true in Section 2.1 (`api/dataplane.rs:191`), this
  test pins the guarantee.

#### E2E-NEW-047: CLI refuses an unconfigured project without `--on-demand`
- **Category:** Failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** CLI (`assert_cmd`)
- **Steps:**
  - Given project `proj1` with `autopurge_enabled = false`
  - When `mcp-fs purge --project proj1` runs (no `--on-demand`)
  - Then exit code is non-zero, a message names the project and the reason, and the DB
    is unchanged
- **Priority:** Critical

#### E2E-NEW-080: Grace-sweep cascade failure surfaces like any FR-027 failure
- **Category:** Error Handling
- **Scenario:** SC-007
- **Requirements:** FR-NEW-012
- **Driver:** direct call, with the underlying `admin.delete_project` cascade forced to
  fail partway (e.g. a locked child row)
- **Steps:**
  - Given a soft-deleted project past its grace period, and the delete cascade forced to
    fail on one of its steps
  - When the grace sweep invokes the cascade for that project
  - Then the failure surfaces exactly as an unmodified `admin.delete_project` call would
    fail today (same error shape, no silent swallow), and the project is NOT left
    half-deleted outside of what FR-027's own cascade already guarantees
- **Priority:** High

#### E2E-NEW-052: Global sweep ignores `autopurge_enabled`
- **Category:** Cross-Scenario
- **Scenario:** SC-004, SC-007
- **Requirements:** FR-NEW-011, FR-NEW-012
- **Driver:** CLI
- **Steps:**
  - Given a soft-deleted project with `autopurge_enabled = false`, past grace period
  - When `mcp-fs purge` runs with no `--project`
  - Then the project is permanently removed anyway
- **Priority:** Critical — distinguishes the per-project gate (FR-NEW-007/008) from the
  grace sweep (FR-NEW-012), a likely implementation confusion point.

## 13. Consistency Notes

None — no conflicts found with existing specifications. FR-027/FR-610 are referenced,
never altered.

## 14. Migration & Implementation Notes

Order of operations, since several requirements depend on earlier ones existing first:
1. Schema: `project.deleted_at`, `project_purge_config` (FR-NEW-004, 008, 013 depend on
   these existing).
2. Atime wiring (FR-NEW-001/002/003) — independent of the rest, can land first or in
   parallel.
3. `require_member` gate update (FR-NEW-013) — must land before the sweep logic can be
   tested end-to-end, since E2E-NEW-040..045 assume it.
4. Sweep logic module (`purge.rs`) implementing FR-NEW-007/008/012, shared by:
5. The background loop (FR-NEW-009) and the CLI verb (FR-NEW-010/011) — both thin callers
   of step 4, built last so they can't diverge.
6. Admin tools (FR-NEW-004..006, 014..016) and the browser screen (FR-NEW-017) — can land
   any time after schema, since they only read/write the new columns directly.

No feature flag needed: every new column defaults to inactive (`autopurge_enabled =
false` / `deleted_at = NULL`), so existing projects are unaffected until an admin opts in.
No data migration beyond the additive schema change. Rollback = revert the migration;
additive-only means no destructive rollback step exists to get wrong.

## 15. Open Questions & TBDs

None outstanding — all ambiguities raised during interview and test design were resolved
with the user or verified against the code (see Section 17 and the Drift Register).

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| `atime` | Last-access timestamp on a file node; activated by this spec | Access tracking |
| Autopurge | The per-project opt-in to automatic age-based purging | Purge configuration |
| `use_internal_purge` | Per-project flag choosing the in-process loop vs. external CLI/cron as the driver | Purge configuration |
| On-demand purge | A CLI-triggered purge of a project not configured for autopurge, via `--on-demand` | Purge execution |
| Staleness reference | `max(project.created_at, max(atime) over live files)`, the value a project's age is measured against | Project lifecycle |
| Soft-deleted project | A project with `deleted_at` set: inaccessible to normal callers, reversible, data intact | Project lifecycle |
| Grace period | The global window (`project_purge_grace_days`) a soft-deleted project survives before permanent removal | Project lifecycle |
| Permanent removal | The existing FR-027 cascade, applied once the grace period elapses | Project lifecycle |

## 17. Decisions Log

- **DEC-001:** `atime` means true last-access, updated on every content read, and every
  write also bumps it (alongside `mtime`). **Rationale:** the existing column is dead and
  this feature is the natural place to activate it; a write-only signal would
  under-represent "used". **Alternatives considered:** last-write-only (insufficient).
  **Implemented by:** FR-NEW-001, FR-NEW-002. **Round:** 1.
- **DEC-002:** Dual trigger — an internal background loop and a standalone `mcp-fs purge`
  CLI verb, selectable per project via `use_internal_purge`. **Rationale:** matches the
  codebase's existing ad hoc background-task pattern while still allowing external
  cron control. **Alternatives considered:** internal-only (rejected: no external
  control); CLI-only (rejected: no zero-config automatic behavior). **Implemented by:**
  FR-NEW-009, FR-NEW-010, FR-NEW-011. **Round:** 1.
- **DEC-003:** CLI purge on a project not configured for autopurge fails unless
  `--on-demand` is passed. **Rationale:** prevents accidental purge of projects that
  never opted in. **Implemented by:** FR-NEW-011. **Round:** 1.
- **DEC-004:** Project staleness reference = `max(project.created_at, max(atime) over
  live files)`, never an independent project-level activity timestamp. **Rationale:**
  metadata-only operations (listing, stat) must not keep a project artificially "alive".
  **Implemented by:** FR-NEW-008. **Round:** 1.
- **DEC-005:** `grep` counts as a content access (bumps `atime`); `stat`/`glob`/`list`/
  `tree` do not. **Rationale:** grep reads file content to search it, unlike pure
  metadata ops. **Implemented by:** FR-NEW-001, FR-NEW-003. **Round:** 1.
- **DEC-006:** File-level purge reuses the existing trash mechanism as-is, single phase —
  no new per-file retention clock in this spec. **Rationale:** permanent trash cleanup is
  BL-0003's unspec'd scope; building a second, competing retention clock here would
  conflict with it later. **Implemented by:** FR-NEW-007. **Round:** 1.
- **DEC-007:** Project soft-delete is two-phase (soft-delete, reversible → grace period →
  permanent removal), with its own grace-period clock distinct from BL-0003's file-trash
  retention. **Rationale:** user's explicit design: an admin needs to undo an accidental
  stale-project classification, which a single-phase delete cannot offer.
  **Implemented by:** FR-NEW-008, FR-NEW-012, FR-NEW-015. **Round:** 1.
- **DEC-008:** Grace period is one global config value, not per-project.
  **Rationale:** explicit user decision; per-project override adds complexity with no
  stated need. **Implemented by:** FR-NEW-018. **Round:** 1.
- **DEC-009:** Reuse `ERR_PROJECT_NOT_FOUND` for calls against a soft-deleted project,
  rather than minting a 15th error code. **Rationale:** `errors.rs:9-22` declares exactly
  14 `ERR_*` constants, and AGENTS.md documents that set as frozen ("no new code"); from
  a normal caller's perspective a soft-deleted project is indistinguishable from an
  absent one.
  **Alternatives considered:** a new `ERR_PROJECT_DELETED` code (rejected: unnecessary
  distinction for any caller except the admin tools, which bypass the check entirely).
  **Implemented by:** FR-NEW-013. **Round:** 1.
- **DEC-010 (Fork A):** `atime`/`mtime` updates are async, best-effort (fire-and-forget),
  matching `search/indexer.rs:16-23`'s existing detached-task idiom. **Rationale:** a
  synchronous update would write-amplify the single most frequent operation in the
  system (reads). **Alternatives considered:** synchronous same-transaction update
  (rejected: latency cost on every read). **Implemented by:** FR-NEW-001, FR-NEW-002.
  **Round:** 2 (Phase 2 approach exploration).
- **DEC-011 (Fork B):** Soft-deleted state is a nullable `project.deleted_at` column
  added via the existing `column_migration()` idempotent path, not a separate table.
  **Rationale:** minimal, one-WHERE-clause change everywhere `project` is read; matches
  the precedent at `storage/admin.rs:63-70` (`project.index_mode`). **Alternatives
  considered:** a separate `deleted_projects` table (rejected: a move-based lifecycle adds
  complexity with no benefit at this scale). **Implemented by:** FR-NEW-008.
  **Round:** 2.
- **DEC-012:** `admin.list_deleted_projects()` is membership-filtered per caller (platform
  admin sees all), matching the verified convention of `admin.list_projects`
  (`storage/admin.rs:236 list_projects_for`), not platform-admin-only.
  **Rationale:** consistency with the existing tool's exact filtering convention,
  verified by reading its implementation rather than assumed. **Implemented by:**
  FR-NEW-014. **Round:** 6 (implementability audit correction, see Drift Register note
  below — corrected before commit, so it is not a registered drift, just a documented
  course-correction during Phase 4).
- **DEC-013:** Concurrency between the internal loop and CLI on-demand purge on the same
  project is resolved without a new locking primitive: file soft-delete is naturally
  idempotent, and project soft-delete uses a conditional `UPDATE ... WHERE deleted_at IS
  NULL`. **Rationale:** matches the codebase's existing style — no general per-project
  fs-lock exists today, only git has one (`git/repo.rs:44`); adding a new lock primitive
  for this alone would be disproportionate. **Implemented by:** FR-NEW-007, FR-NEW-008.
  **Round:** 3c.

## 18. Implementability Gate

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 2 | 1 | NOT-IMPLEMENTABLE (pre-amendment) |
| 2 | 2 (newly found) | 1 | NOT-IMPLEMENTABLE (pre-amendment) |
| 3 | 0 | 1 | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:**
- Round 1 → 2: FR-NEW-001 gained a "live file excludes the `trash_dir` subtree"
  business rule (closing a divergence between the file-purge and project-staleness
  queries on whether trashed nodes count); FR-NEW-013 gained an explicit "the
  `deleted_at` filter lives only in `require_member`" business rule (otherwise
  `admin.undelete_project`/`admin.set_purge_config` risk becoming unreachable on the
  very rows they manage); Section 2.1 dropped an inaccurate `tools/git.rs:9258`
  citation (a test helper, not a production task).
- Round 2 → 3: Section 8 corrected — the `volume_id` convention does not apply to
  `project`/`project_purge_config` (the admin registry is global, no `volume_id` column
  exists there at all); dropped the invented, precedent-free REST surface for the three
  new admin tools (no existing `admin.*` tool has a REST route; this spec does not start
  that convention) from Section 9.1 and the two "(MCP/REST)" mentions in SC-005/SC-006;
  fixed FR-NEW-009's citation of FR-NEW-011 to the correct FR-NEW-012.

**Drift registered:** DRIFT-001 (see below).

## 19. Implementation Drift Register

#### DRIFT-001: No existing fire-and-forget database write pattern to model FR-NEW-001/002 on
- **Spec says:** FR-NEW-001/FR-NEW-002, "asynchronously and best-effort, a failed bump
  never surfaces as a tool error" — implying a reusable detached-write idiom.
- **Code does:** `tokio::spawn` fire-and-forget exists for a server task
  (`tools/git_auth.rs:386`), a clone progress task (`tools/git.rs:9258`), and per-editor
  watchers (`tools/editor.rs:264,277`) — none of these write to the database from the
  detached task. The search indexer (`search/indexer.rs:16-23`) is the closest precedent
  (detached work triggered by a write, logging failures rather than propagating them) but
  it calls an external embedding service, not a DB write.
- **Nature:** missing capability (no detached-DB-write helper exists yet to reuse).
- **Resolution during implementation:** build the detached atime/mtime bump as a small
  helper in `purge.rs` or `core/fs_ops.rs`, following `search/indexer.rs`'s
  log-on-failure convention, and expose a test-only completion hook (e.g. a
  `tokio::sync::Notify` fired after the spawned task finishes) so tests can await
  completion deterministically instead of polling with a fixed timeout.
- **Detected by:** E2E-NEW-007 (asserts the read call succeeds even when the detached
  bump is forced to fail) is the parity test that makes this unmissable — it fails if the
  helper is built synchronously or if a bump failure is allowed to propagate.
- **Blocks which requirement:** none, informational — implementation proceeds with the
  resolution above; FR-NEW-001/002 are otherwise fully specified.
- **Status:** resolved (E2E-NEW-007 /
  `detached_write_never_surfaces_as_caller_error`, commit 712df72, SPEC-0014_US-0001)
