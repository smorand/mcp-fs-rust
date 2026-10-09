# mcp-fs-rust — Specification Document

> Id: SPEC-0017
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0014; new id SPEC-0017 (collision avoidance)

## 1. Summary

Adds automatic, age-based removal of unused files and stale projects. Activates
`nodes.atime` as a real last-access signal, adds a background sweep loop plus a
standalone `mcp-fs purge` CLI verb (either or both drivable per project), and introduces
a two-phase project lifecycle: soft-delete (reversible, admin-visible, inaccessible to
normal callers), then, after a global grace period, permanent removal via the existing
project-deletion cascade. Audience: the platform admin configuring retention, and
indirectly every project member whose unused data this reclaims.

## 2. Current State

### 2.1 Functional

- `nodes.atime` is a real last-access signal: content-reading tools bump it, content
  writes bump both `atime` and `mtime`, metadata/listing operations never touch it.
- Project deletion remains the existing immediate, cascading operation for a project
  that is not soft-deleted. A new two-phase lifecycle wraps it: a project can be
  soft-deleted (reversible), then permanently removed once a global grace period
  elapses, by invoking that same cascade unchanged.
- File soft-delete ("trash") is unchanged: purge of a file still lands in the existing
  trash mechanism; no new per-file retention clock was introduced.
- A per-project purge configuration (`autopurge_enabled`, `use_internal_purge`,
  `file_retention_days`, `project_retention_days`) gates whether and how a project is
  swept.
- A background sweep loop runs on a fixed interval, performing: file-purge sweep,
  project soft-delete sweep, and grace-period permanent removal — shared logic also
  invoked synchronously by a standalone `mcp-fs purge` CLI verb.
- Calls against a soft-deleted project are blocked at the single shared membership gate,
  for both the MCP and REST surfaces, uniformly, including for a platform admin.
- `admin.list_deleted_projects` and `admin.undelete_project` exist, membership-filtered
  the same way `admin.list_projects` is, plus a browser screen at
  `/app/deleted-projects` for both.

### 2.2 SPEC-0003 (filesystem-engine)

None — this spec is additive and does not modify any SPEC-0003 filesystem-engine
requirement. It reuses the project-deletion cascade (originally specified elsewhere) and
the existing membership gate unchanged.

### 2.3 Test coverage

Covered by the 80-test E2E suite designed for this spec (E2E-NEW-001..080), partitioned
across 14 implementation stories (US-0001..US-0014), all status `done`. Representative
commit: `74ddeb8` (`feat(SPEC-0014): auto-purge unused files and stale projects`). Prior
to this spec, atime, purge, retention and project soft-delete had zero test coverage —
this increment is wholly new, verified surface.

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
- Any change to file-trash retention/expiry semantics (BL-0003) — purged files land in
  the existing, unmodified trash mechanism; permanent trash cleanup is BL-0003's job
  later.
- Any UI beyond the one new deleted-projects screen.
- Per-project override of the grace period (it is a single global value).

## 4. Actors

- **Platform admin**: configures per-project purge settings, runs on-demand CLI purge,
  views and undeletes soft-deleted projects.
- **Project member/owner**: passively affected — their unused files or stale project may
  be purged; regains access immediately on undelete.
- **Internal purge loop**: a background, in-process actor (`tokio::spawn` + interval).
- **External operator / cron**: drives `mcp-fs purge` from outside the server process.

## 5. Usage Scenarios

### SC-001: Admin configures autopurge on a project
**Actor:** platform admin
**Flow:** admin calls `admin.set_purge_config(project_id, autopurge_enabled,
use_internal_purge, file_retention_days?, project_retention_days?)`; system validates
role and argument values, persists the config, returns it.
**Postconditions:** the project's purge config row reflects the call; no purge runs as a
side effect of configuring.
**Exceptions:** non-owner/non-admin caller → forbidden; a present retention value `<= 0`
→ invalid argument; unknown project → not found.

### SC-002: Internal purge loop — file purge
**Actor:** internal purge loop
**Preconditions:** project has `autopurge_enabled && use_internal_purge &&
file_retention_days.is_some()`.
**Flow:** on each wake, for each qualifying project, find live files where `now - atime
> file_retention_days`, soft-delete each into the existing trash, one commit per file.
**Exceptions:** a DB failure mid-sweep skips and retries that file next cycle; a read
landing just before the sweep correctly spares the file (no race); re-purging an
already-trashed file is a no-op.
**Cross-scenario notes:** runs before SC-003 in the same cycle; project staleness in
SC-003 is computed over the post-file-purge live file set.

### SC-003: Internal purge loop — project soft-delete
**Actor:** internal purge loop, same cycle as SC-002
**Preconditions:** `autopurge_enabled && use_internal_purge &&
project_retention_days.is_some()`.
**Flow:** compute the staleness reference as `max(project.created_at, max(atime) over
the project's live files)` (falls back to `created_at` alone if no live files); if `now -
reference > project_retention_days`, soft-delete via a conditional update.
**Postconditions:** project inaccessible to normal `fs.*`/`git.*` calls; visible in
`admin.list_deleted_projects`; all underlying data untouched.
**Exceptions:** a never-touched project is eligible from `created_at` alone; a second
sweep run on an already soft-deleted project is a no-op, not re-stamping `deleted_at`.
**Cross-scenario notes:** recent activity on any one file resets the whole project's
staleness clock, even for an old project.

### SC-004: CLI on-demand purge
**Actor:** external operator / cron
**Flow:** `mcp-fs purge --project <id> [--on-demand]`; if the project's
`autopurge_enabled` is false and `--on-demand` is absent, fails with no action taken;
otherwise runs the same file-sweep and project-staleness logic as SC-002/SC-003
synchronously for that project and prints a summary. `mcp-fs purge` with no `--project`
instead runs the grace-period sweep (SC-007) globally.
**Exceptions:** named project not found → non-zero exit; `--on-demand` on an
already-autopurge-enabled project → accepted, redundant, not an error.
**Cross-scenario notes:** the no-`--project` global sweep acts on every soft-deleted
project regardless of `autopurge_enabled`.

### SC-005: Admin lists soft-deleted projects
**Actor:** platform admin (or any owner/member, filtered to their own)
**Flow:** `admin.list_deleted_projects()` or the `/app/deleted-projects` browser screen
returns `{project_id, owner, deleted_at, days_until_permanent_removal}` per visible
soft-deleted project, filtered the same way `admin.list_projects` is.
**Exceptions:** none beyond standard auth; empty result when nothing is soft-deleted.

### SC-006: Admin undeletes a project
**Actor:** platform admin or project owner
**Flow:** `admin.undelete_project(project_id)` or the browser screen's undelete action
(CSRF-protected) clears `deleted_at` to null; project is immediately accessible again;
`atime` values are untouched, so an idle project can become stale again quickly.
**Exceptions:** project not currently soft-deleted → invalid argument; project does not
exist at all → not found; non-owner, non-admin caller → forbidden.

### SC-007: Grace-period sweep → permanent removal
**Actor:** internal purge loop (unconditionally, regardless of `use_internal_purge`) and
the CLI's no-`--project` form.
**Flow:** for every soft-deleted project where `now - deleted_at >
project_purge_grace_days` (global config), call the existing project-deletion cascade
unchanged.
**Postconditions:** project and all its data permanently gone, identical to a manual
delete.
**Exceptions:** cascade failure mid-way surfaces exactly as the existing cascade already
defines.

## 6. Functional Requirements

| ID | Requirement | Evidence |
|---|---|---|
| FR-NEW-001 | Content reads (`fs.read`, `fs.read_bytes`, `fs.read_lines`, `fs.read_section`, `fs.head`, `fs.tail`, `fs.extract_text`, `fs.grep`, and REST equivalents) bump `atime` on success, asynchronously and best-effort; a "live file" excludes any node under the project's `trash_dir` subtree. | `storage/meta.rs:133`; US-0003 |
| FR-NEW-002 | Mutating tools/routes (`fs.write`, `fs.edit`, `fs.copy`, `fs.move`, etc.) bump both `atime` and `mtime` on success, via the same best-effort mechanism. | US-0003 |
| FR-NEW-003 | `fs.stat`, `fs.glob`, `fs.list`, `fs.tree`, and `admin.*` listing tools never update `atime`. | US-0003 |
| FR-NEW-004 | `admin.set_purge_config(project_id, autopurge_enabled, use_internal_purge, file_retention_days?, project_retention_days?)` persists and returns the configuration when every present retention value is `> 0`; either retention field may be absent independently. | `tools/admin.rs`; US-0009 |
| FR-NEW-005 | A present `file_retention_days` or `project_retention_days` `<= 0` is rejected, persisting nothing. | US-0009 |
| FR-NEW-006 | A non-owner/non-admin caller to `set_purge_config` is forbidden; a nonexistent project is not found. | US-0004; US-0009 |
| FR-NEW-007 | While a project has `autopurge_enabled && use_internal_purge && file_retention_days.is_some()`, each sweep cycle soft-deletes into the existing trash every live file whose `now - atime > file_retention_days`, committing each file independently (one failure never blocks another). | `purge.rs`; US-0005 |
| FR-NEW-008 | While a project has `autopurge_enabled && use_internal_purge && project_retention_days.is_some()`, each sweep cycle (after the file sweep) soft-deletes the project when `now - max(project.created_at, max(atime) over its live files) > project_retention_days`, via a conditional update that never advances an existing `deleted_at`. | `purge.rs`; US-0006 |
| FR-NEW-009 | The file-purge and project-soft-delete sweeps, plus the grace-period sweep, run on a fixed interval (`safety.purge_interval_secs`) via a detached background task started at server boot and stopped cleanly on shutdown. | `app.rs`; US-0007 |
| FR-NEW-010 | A `purge` CLI verb accepts an optional `--project <id>` and an optional `--on-demand` flag. | `cli.rs`; US-0007 |
| FR-NEW-011 | With `--project` and `autopurge_enabled=false` and no `--on-demand`: exit non-zero, no action. With `--project` and (`autopurge_enabled=true` or `--on-demand`): run the sweep synchronously for that project. Without `--project`: run the grace-period sweep across every soft-deleted project regardless of `autopurge_enabled`. | US-0007; US-0008 |
| FR-NEW-012 | When a soft-deleted project's `now - deleted_at > project_purge_grace_days` (global config), it is permanently removed via the existing project-deletion cascade, unmodified, regardless of `use_internal_purge`. | US-0008 |
| FR-NEW-013 | Any `fs.*`/`git.*` tool or REST equivalent against a project with `deleted_at` non-null returns not-found, enforced in the single shared membership gate used by both the MCP and REST surfaces; applies unconditionally, including to a platform admin. The `deleted_at` filter applies only at that gate, never at project lookup/ownership checks used by the admin tools managing soft-deleted rows. | `storage/admin.rs`; `state.rs:59`; US-0004; US-0014 |
| FR-NEW-014 | `admin.list_deleted_projects()` returns every soft-deleted project visible to the caller — filtered exactly as `admin.list_projects` — as `{project_id, owner, deleted_at, days_until_permanent_removal}`, with the countdown computed as `project_purge_grace_days - floor((now - deleted_at) in days)`, not clamped below zero. | `storage/admin.rs:236`; US-0010 |
| FR-NEW-015 | `admin.undelete_project(project_id)` on a project with `deleted_at` non-null clears it to null and returns success. | US-0011 |
| FR-NEW-016 | `undelete_project` rejects: `deleted_at` already null → invalid argument; project does not exist → not found; caller neither owner nor platform admin → forbidden. | US-0011 |
| FR-NEW-017 | An admin-session-gated `/app/deleted-projects` screen, following the same auth/CSRF conventions as the existing `/app/tokens` screen, lists every soft-deleted project visible to the caller with an undelete action that calls FR-NEW-015 (never a second implementation). | `deleted_projects_screen.rs`; US-0012 |
| FR-NEW-018 | New global configuration `safety.purge_interval_secs` (default `3600`) and `safety.project_purge_grace_days` (default `30`) on `SafetyConfig`. | `config.rs`; US-0002 |

## 7. Non-Functional Requirements

- **Performance:** no specific load target (personal-project scale); the sweep iterates
  all qualifying projects/files per cycle, acceptable at current scale, no pagination.
- **Security:** no new authentication mechanism; the three new admin tools reuse the
  existing bearer-JWT + owner/admin or membership-filter gates; the browser screen reuses
  the existing session-cookie + CSRF convention exactly.
- **Usability:** the new browser screen mirrors the existing token screen's look and
  interaction pattern.
- **Reliability:** no retry beyond "the next sweep cycle picks up anything missed";
  per-file/per-project idempotent commits make this safe; no rollback mechanism needed
  since every operation is independently idempotent.
- **Observability:** sweep start/summary at INFO, each file/project mutation at DEBUG,
  failures at ERROR, consistent with existing logging conventions; no OpenTelemetry
  collector introduced.
- **Deployment:** no infrastructure change; the sweep loop runs in-process via
  `tokio::spawn`; the CLI verb ships in the existing binary; no new crate dependency.
- **Scalability:** full-set sweep per cycle, fine at current scale; revisit only if
  project/file counts grow by orders of magnitude (not anticipated).

## 8. E2E Tests

80 tests (E2E-NEW-001..080), partitioned across 14 stories (US-0001..US-0014), all
status `done`. Area breakdown:

| Area | Test IDs | Count |
|---|---|---|
| Content-read bumps `atime` only | E2E-NEW-001..009 | 9 |
| Write bumps both | E2E-NEW-010..014 | 5 |
| `admin.set_purge_config` | E2E-NEW-015..023 | 9 |
| Internal file-purge sweep | E2E-NEW-024..031 | 8 |
| Internal project soft-delete sweep | E2E-NEW-032..039 | 8 |
| `fs.*`/`git.*` access gate | E2E-NEW-040..045 | 6 |
| CLI `mcp-fs purge` | E2E-NEW-046..052 | 7 |
| Grace-period sweep | E2E-NEW-053..057, 080 | 6 |
| `admin.list_deleted_projects` | E2E-NEW-058..062 | 5 |
| `admin.undelete_project` | E2E-NEW-063..068 | 6 |
| Browser screen | E2E-NEW-069..073 | 5 |
| Config | E2E-NEW-074..076 | 3 |
| Loop lifecycle | E2E-NEW-077..079 | 3 |

Coverage statistics: happy 27, non-happy (failure + edge + side-effect + idempotency) 53,
ratio ≈ 1.96:1. Every FR has ≥3 tests; every scenario has ≥5, each with at least one
happy, one failure and one edge test.

## 9. Glossary

| Term | Definition |
|---|---|
| `atime` | Last-access timestamp on a file node; activated by this spec |
| Autopurge | The per-project opt-in to automatic age-based purging |
| `use_internal_purge` | Per-project flag choosing the in-process loop vs. external CLI/cron as the driver |
| On-demand purge | A CLI-triggered purge of a project not configured for autopurge, via `--on-demand` |
| Staleness reference | `max(project.created_at, max(atime) over live files)`, the value a project's age is measured against |
| Soft-deleted project | A project with `deleted_at` set: inaccessible to normal callers, reversible, data intact |
| Grace period | The global window (`project_purge_grace_days`) a soft-deleted project survives before permanent removal |
| Permanent removal | The existing project-deletion cascade, applied once the grace period elapses |

## 10. Confidence notes

All 18 FRs trace to a completed story (US-0001..US-0012 implement them; US-0013/US-0014
are converge-gap stories closing test coverage on the production `set_purge_config`
handler and the access gate end-to-end). The legacy spec's single drift item (DRIFT-001:
no existing fire-and-forget DB-write pattern to model the atime/mtime bump on) was
carried forward and resolved as US-0001 before any dependent story started, so no open
drift remains. No open questions were left unresolved in the legacy document.
