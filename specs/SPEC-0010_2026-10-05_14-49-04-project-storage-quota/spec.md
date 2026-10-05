# mcp-fs — Per-Project Storage Quota — Specification Document

> Generated on: 2026-10-05
> Id: SPEC-0010
> Nature: FEAT
> Depth: L
> Depth evidence: new persisted schema (no `quota_bytes` field exists anywhere on `Project`/`PurgeConfig`, `traits.rs:105-122`) and 7+ modules touched (`storage/traits.rs`, `storage/admin.rs`, `storage/meta.rs`, `storage/rel/schema.rs`, `errors.rs`, `mcp/server.rs`, `core/fs_ops.rs`, `tools/doc.rs`, `tools/git.rs`, `git/merge.rs`) — the L rule fires on either signal alone.
> Status: Draft
> Type: Evolution Specification
> From backlog: BL-0002
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Executive Summary

A platform admin can set a maximum storage size (in MB) on a project. Once a write would push that
project's declared (logical) byte usage past the cap, the write is rejected with a clear, literal
error — `"Quota exhausted"` — until the admin raises the cap or usage drops. Admins also gain a
`du`-like breakdown of a project's storage, sorted by file size descending, to find what to clean up.
This closes the FinOps gap named in BL-0002: "one project can consume unbounded storage and there is
no way to isolate tenants from each other's growth," and nobody currently cleans up on their own.

## 2. Current State

### 2.1 How it works today

- `Project` (`crates/core/src/storage/traits.rs:105-111`): fields `id`, `owner`, `created_at`,
  `index_mode`. No size, byte or quota field of any kind.
- `PurgeConfig` (`traits.rs:117-122`): `autopurge_enabled`, `use_internal_purge`,
  `file_retention_days`, `project_retention_days`. Unrelated to storage size; a time-based retention
  mechanism, not a capacity cap.
- The only existing "quota" is `write_quota_bytes` (`config.rs:369`, default `52_428_800` = 50 MB,
  asserted at `config.rs:1157`): a **per-session**, **in-memory**, **rate-limiter**, keyed by
  `(person, project)` in `SessionState` (`safety.rs:25-30`, `sessions: Mutex<HashMap<...>>` at
  `safety.rs:81`), charged by `charge_write` (`safety.rs:131-140`) and refunded by `refund_write`
  (`safety.rs:149-154`). It resets whenever the session object is recreated and carries no persisted,
  per-project cap. There is no existing "total bytes used per project" query, column or maintained
  aggregate anywhere in `meta.rs`, `admin.rs` or `storage/rel/schema.rs`.
- Blobs are content-addressed and refcounted in `blob_refs` (schema `meta.rs:139-146`,
  `tx_incref`/`tx_decref` at `meta.rs:276-320`), keyed by `sha256`. Logical file size is tracked
  independently on `nodes.size` (`BigInt`, default `0`, `schema.rs:348`), set by the single write
  choke point `MetaBackend::put_file` (trait `storage/traits.rs:143`, impl `storage/meta.rs:485-553`).
  Every byte-producing operation funnels through `put_file`: `fs.write`/`fs.append`
  (`mcp/server.rs:1413,1441` → `fs_ops.rs:573,609`), `fs.create_empty` (`server.rs:1466` →
  `fs_ops.rs:797`), `fs.write_bytes` (`server.rs:1488` → `fs_ops.rs:826`), the edit family
  (`server.rs:1526-1644` → `fs_ops.rs:1098`), `fs.copy` (`server.rs:1898` → `fs_ops.rs:1466`),
  `fs.write_docx` (`server.rs:2007` → `tools/doc.rs:268`), git single-blob writes
  (`tools/git.rs:1402`), git bulk import (`tools/git.rs:3121`) and git merge apply, atomic
  (`git/merge.rs:705`).
- Trash is **not** a separate deleted-flag: a soft delete moves the node to
  `/{trash_dir}/{epoch_ms}__{flattened path}` (`safety.rs:179-183`) and it stays an ordinary row in
  `nodes` with its `size` intact (`fs_ops.rs:985-1034`) until the purge sweep (`purge.rs`) actually
  removes it. There is therefore no special-casing needed to "include trash": a trashed file already
  counts in any `SUM(nodes.size)` query, by construction.
- `volume_id` is 1:1 with `project_id` (`storage/mod.rs:295`, confirmed by the comment at
  `storage/admin.rs:3`: the `project`/`project_member` registry carries no `volume_id` column at all,
  it is global and keyed by `project_id` directly, while `nodes`/`blob_refs` are scoped by
  `volume_id = project_id`), so a per-project quota maps directly onto a `SUM(...) WHERE
  volume_id = ?` query against `nodes`.
- Authorization: `state.require_admin` (`state.rs:37`, backed by `config.admins: Vec<String>` at
  `config.rs:193`, checked caselessly by `is_admin` at `config.rs:957`/`state.rs:32`) is platform-admin
  only, used today by `admin.create_project` (`mcp/server.rs:2069-2076`). `state.require_owner_or_admin`
  (`state.rs:47`) allows the project owner **or** a platform admin, used today by
  `admin.set_purge_config` (`server.rs:2385-2394`). These are the two existing authorization tiers this
  increment reuses; no new tier is introduced.
- Schema additions to a live, deployed table use `ColumnMigration` (`storage/rel/schema.rs:120-127`,
  declared via `SchemaSet::column_migration` at `schema.rs:163-166`), rendered per dialect
  (SQLite/PostgreSQL/SQL Server) specifically because `CREATE TABLE IF NOT EXISTS` cannot widen an
  existing table. This is the existing, exercised mechanism for adding `quota_bytes` to `project`.
- REST: `admin.*` has no REST equivalent today (`api/dataplane.rs` exposes no `admin.`-prefixed route;
  the only `state.admin.*` call in that file is `list_projects_for` at `dataplane.rs:324`, used to
  render the project list, not to administer one). The REST `/upload` endpoint, however, calls the
  same `core::fs_ops` engine functions the MCP tools call, so it inherits any enforcement added inside
  `put_file` for free — no parallel REST check is required.

### 2.2 Existing specifications governing this area

`specs/SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md` owns the `Project` data model
(§8, `FR-025..027`) and the per-session write quota (`FR-035`, `spec.md:560-565`). This increment
**extends** that data model with a new, independent `quota_bytes` field and a new, independent
enforcement mechanism; it does not modify SPEC-0002, and the two quotas (session rate-limit vs.
project storage cap) are deliberately orthogonal and both remain in force (DEC-008 below).

### 2.3 Existing test coverage

No existing test covers a per-project storage cap: zero hits for `quota_bytes`, `project.*quota`,
`disk_usage` or `used_bytes` across `crates/core/src`. Two existing tests exercise the unrelated
session quota and MUST remain green, unmodified, after this change: `write_bytes_charges_the_quota_and_audits`
(`core/fs_ops.rs:3008`) and `write_bytes_refuses_to_exceed_the_quota` (`fs_ops.rs:3022`) — both call
`write_bytes` with no project quota configured, so `quota_bytes = None` and behavior is identical to
today. Exact test command: `./test.sh` (= `cargo test --workspace`; quality gate additionally runs
`cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`, per
`AGENTS.md`).

## 3. Scope

### 3.1 In Scope
- A nullable, persisted `quota_bytes` on each project (`NULL` = unlimited, no default cap).
- Three new `admin.*` MCP tools: set the quota, read the quota + live usage, read a sorted
  disk-usage breakdown.
- Enforcement at the single write choke point `MetaBackend::put_file`, counting declared (logical)
  file size, including trashed files, race-safe against concurrent writers.

### 3.2 Out of Scope (Non-Goals)
- No aggregate platform-wide quota across projects.
- No alerting/webhook when a project nears its cap — hard rejection at the boundary only.
- No change to the existing per-session `write_quota_bytes` rate limiter; the two coexist
  independently (DEC-008).
- No forced eviction or grandfathering when an admin lowers a quota below current usage: existing
  files are never touched; only writes that would further increase usage beyond the new cap are
  rejected (FR-NEW-013).
- No pre-flight "would the whole git import/merge exceed quota" prediction pass (DEC-007): each
  caller of `put_file` is checked individually, as it already is for every other write path.

## 4. User Personas & Actors

| Actor | Role |
|---|---|
| **Platform admin** | Listed in `config.admins` (`config.rs:193`). Sole authority to set/read a project's `quota_bytes`. Also authorized to read the disk-usage breakdown. |
| **Project owner** | The person named `owner` at project creation (`traits.rs:106`). Authorized to read the disk-usage breakdown (to know what to clean up) but **cannot** set or clear the quota. |
| **Project member / any write caller** | Triggers the enforcement path transparently through normal `fs.*`/`git.*` operations; no new capability granted. |

No background/scheduled actor: enforcement is synchronous, inline in the existing write transaction.

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Platform** (per SPEC-0002 §4.5) | Projects, membership, platform administration — this increment adds one field here. | Project (+`quota_bytes`) |
| **Storage accounting** (new, scoped to this increment) | Live usage computation and the write-time cap check, both against `nodes`/`blob_refs` for one `volume_id`. | `nodes.size`, `blob_refs`, `used_bytes` |

## 5. Usage Scenarios

### SC-001: Admin sets a project's quota
**Actor:** Platform admin.
**Preconditions:** project exists.
**Flow:**
1. Admin calls `admin.set_project_quota(project_id, max_mb: Option<u32>)`.
2. The system validates the caller is a platform admin and `max_mb` is `None` or `>= 1`.
3. The system stores `quota_bytes = max_mb.map(|m| m as i64 * 1_048_576)` on the project and returns
   `{project_id, max_mb}`.
**Postconditions:** future `put_file` calls for this project are checked against the new cap (or
unchecked, if `max_mb = None`).
**Exceptions:**
- EXC-001a: project not found → `ERR_PROJECT_NOT_FOUND`.
- EXC-001b: caller is not a platform admin → `ERR_FORBIDDEN`, no mutation (FR-NEW-003).
- EXC-001c: `max_mb = Some(0)` → `ERR_INVALID_ARGUMENT`, `"max_mb must be > 0"` (FR-NEW-004).
**Cross-scenario notes:** lowering the quota below current usage is accepted (SC-004's exception,
not this scenario's); it does not evict or modify any existing node (FR-NEW-013).

### SC-002: Admin reads a project's quota and live usage
**Actor:** Platform admin.
**Preconditions:** project exists.
**Flow:**
1. Admin calls `admin.get_project_quota(project_id)`.
2. The system computes `used_bytes = SUM(nodes.size) WHERE volume_id = project_id AND kind = 'file'`,
   live, no cache.
3. Returns `{project_id, max_mb, used_bytes}`.
**Postconditions:** none (read-only).
**Exceptions:**
- EXC-002a: project not found → `ERR_PROJECT_NOT_FOUND`.
- EXC-002b: caller is not a platform admin → `ERR_FORBIDDEN` (FR-NEW-006).

### SC-003: A write stays under quota (no observable change from today)
**Actor:** any caller of a write path (`fs.write`, `fs.append`, `fs.copy`, `fs.write_bytes`,
`fs.write_docx`, edit family, git push/import/merge, REST `/upload`).
**Preconditions:** `quota_bytes` is `None`, or `Some(cap)` with headroom.
**Flow:**
1. The caller performs the write exactly as today.
2. `put_file` computes `new_total` and finds it `<= cap` (or skips the check entirely when
   `quota_bytes = None`).
3. The write proceeds unchanged.
**Postconditions:** byte-for-byte identical behavior to pre-increment code when under quota or
unlimited.
**Exceptions:** none specific to this scenario; see SC-004 for the boundary.

### SC-004: A write would exceed the project quota
**Actor:** any caller of a write path.
**Preconditions:** `quota_bytes = Some(cap)`, and the attempted write's `new_total > cap`.
**Flow:**
1. The caller performs the write.
2. `put_file`, inside the existing transaction, computes `new_total = usage_in_tx -
   old_size_at_path + size`.
3. `new_total > cap` → the call is rejected with `ERR_PROJECT_QUOTA_EXCEEDED`, message exactly
   `"Quota exhausted"`, before any row mutation, before any blob refcount change, and before
   `SafetyManager::charge_write` is invoked (FR-NEW-012).
**Postconditions:** no node created/modified, no blob refcount change, `used_bytes` unchanged,
per-session `write_quota_bytes` budget unchanged.
**Exceptions:**
- EXC-004a: the entry point is `fs.copy`, and source and destination would share one deduplicated
  blob — the destination's **declared** size still counts fully; dedup does not grant a quota
  discount (FR-NEW-011).
- EXC-004b: the entry point is git bulk import or git merge apply — each blob is checked
  individually as it is written; an import may partially land before the blob that breaches the cap
  aborts the rest (DEC-007), while the already-atomic merge-apply path keeps its existing
  all-or-nothing guarantee (so the first blob to breach the cap aborts that merge's whole apply).
- EXC-004c: two writers race for the same headroom — the transactional check guarantees exactly one
  wins, never both, never neither incorrectly (FR-NEW-009).
**Cross-scenario notes:** a quota already lowered below usage (SC-001's exception) stays in this
scenario's territory for every subsequent write that would grow usage further; a write that shrinks
usage net is still accepted even while already over cap (FR-NEW-013).

### SC-005: Admin or owner reads the disk-usage breakdown
**Actor:** platform admin or project owner.
**Preconditions:** project exists.
**Flow:**
1. Caller calls `admin.get_project_disk_usage(project_id, limit: Option<u32>)` (default limit 1000).
2. The system reads every live file node for the project's `volume_id`, sorted by `size` descending,
   capped at `limit` rows.
3. Returns `{project_id, entries: [{path, size}], truncated: bool}`.
**Postconditions:** none (read-only).
**Exceptions:**
- EXC-005a: project not found → `ERR_PROJECT_NOT_FOUND`.
- EXC-005b: caller is neither the project owner nor a platform admin → `ERR_FORBIDDEN`
  (FR-NEW-008).
- EXC-005c: the project has more live file nodes than `limit` → `truncated: true`, exactly `limit`
  rows returned (the largest ones).
**Cross-scenario notes:** directories are not listed (per-file only, v1 scope; per-directory subtree
totals are deferred, YAGNI).

## 6. Functional Requirements

### New Requirements

#### FR-NEW-001 [EARS-U]: Project quota field
> The system SHALL persist an optional `quota_bytes: Option<i64>` on each project record, defaulting
> to `NULL` (no quota) for both newly created and pre-existing projects.

- **Inputs:** none (structural).
- **Outputs:** a new, nullable `quota_bytes` column.
- **Business Rules:** `NULL` means unlimited; there is no platform-wide default cap.
- **Exact names:** new column `quota_bytes` (`BigInt`, nullable) on table `project`, added via a
  `ColumnMigration` entry (`storage/rel/schema.rs:120`) for every dialect, and via the dialect's
  `CREATE TABLE` for fresh installs. `storage::traits::Project` (`traits.rs:105-111`) gains field
  `quota_bytes: Option<i64>`.
- **Priority:** Must-have.

#### FR-NEW-002 [EARS-E]: Admin sets the quota
> WHEN a platform admin calls `admin.set_project_quota` with `project_id` and `max_mb: Option<u32>`,
> THE system SHALL store `quota_bytes = max_mb.map(|m| m as i64 * 1_048_576)` on that project and
> SHALL return `{project_id, max_mb}`.

- **Inputs:** `project_id: String`, `max_mb: Option<u32>`.
- **Outputs:** `{"project_id": String, "max_mb": Option<u32>}`.
- **Business Rules:** `max_mb = None` clears the quota (sets `quota_bytes = NULL`, unlimited).
  `max_mb = Some(0)` is invalid (FR-NEW-004).
- **Exact names:** new MCP tool `admin.set_project_quota`; new trait method
  `AdminBackend::set_quota(&self, project_id: &str, quota_bytes: Option<i64>) -> Result<()>`
  (new on `storage/traits.rs`, implemented in `storage/admin.rs`).
- **Priority:** Must-have.

#### FR-NEW-003 [EARS-O]: Authorization on quota mutation
> IF the caller of `admin.set_project_quota` is not a platform admin (per `state.require_admin`,
> `state.rs:37`) THEN THE system SHALL reject the call with `ERR_FORBIDDEN` and SHALL NOT modify
> `quota_bytes`.

- **Business Rules:** deliberately stricter than `admin.set_purge_config`'s owner-or-admin pattern
  (`state.rs:47`) — the project owner has no authority over the quota (DEC-003).
- **Priority:** Must-have.

#### FR-NEW-004 [EARS-O]: Invalid quota value
> IF `admin.set_project_quota` is called with `max_mb = Some(0)` THEN THE system SHALL reject the
> call with `ERR_INVALID_ARGUMENT` and the message `"max_mb must be > 0"`.

- **Priority:** Must-have.

#### FR-NEW-005 [EARS-E]: Admin reads quota and usage
> WHEN a platform admin calls `admin.get_project_quota` with `project_id`, THE system SHALL return
> `{project_id, max_mb: Option<u32>, used_bytes: i64}`, where `used_bytes` is computed live as
> `SELECT COALESCE(SUM(size),0) FROM nodes WHERE volume_id = project_id AND kind='file'` and
> `max_mb` is `quota_bytes.map(|b| (b / 1_048_576) as u32)`.

- **Exact names:** new MCP tool `admin.get_project_quota`; new trait method
  `MetaBackend::usage_bytes(&self) -> Result<i64>` (new on `storage/traits.rs`, implemented in
  `storage/meta.rs`).
- **Priority:** Must-have.

#### FR-NEW-006 [EARS-O]: Authorization on quota read
> IF the caller of `admin.get_project_quota` is not a platform admin THEN THE system SHALL reject
> the call with `ERR_FORBIDDEN`.

- **Priority:** Must-have.

#### FR-NEW-007 [EARS-E]: Disk-usage breakdown
> WHEN an authorized caller (platform admin or the project's owner, per `state.require_owner_or_admin`,
> `state.rs:47`) calls `admin.get_project_disk_usage` with `project_id` and optional
> `limit: Option<u32>` (default `1000`), THE system SHALL return `{project_id, entries:
> [{path: String, size: i64}], truncated: bool}`, where `entries` lists every live file node for
> that project's `volume_id`, sorted by `size` descending, capped at `limit` rows, and `truncated`
> is `true` if and only if more rows exist beyond `limit`.

- **Exact names:** new MCP tool `admin.get_project_disk_usage`; new trait method
  `MetaBackend::disk_usage(&self, limit: usize) -> Result<Vec<(String, i64)>>`, implemented to query
  `limit + 1` rows so truncation is detected without a second query.
- **Priority:** Should-have (FinOps visibility, not enforcement-critical).

#### FR-NEW-008 [EARS-O]: Authorization on disk-usage read
> IF the caller of `admin.get_project_disk_usage` is neither the project owner nor a platform admin
> THEN THE system SHALL reject the call with `ERR_FORBIDDEN`.

- **Priority:** Must-have.

#### FR-NEW-009 [EARS-E]: Quota enforcement at write time
> WHEN any operation calls `MetaBackend::put_file` for a project whose `quota_bytes` is `Some(cap)`,
> THE system SHALL compute, inside the same transaction as the existing node lookup
> (`meta.rs:498-515`), `new_total = usage_bytes_in_tx - old_size_at_path + size`, and SHALL reject the
> call with error code `ERR_PROJECT_QUOTA_EXCEEDED`, message exactly `"Quota exhausted"`, mapped to
> HTTP status `507`, when `new_total > cap`, before any row is inserted or updated and before any
> blob refcount is incremented or decremented.

- **Business Rules:** when `quota_bytes` is `None`, the check is skipped and behavior is unchanged.
  The comparison is strict `>`: `new_total == cap` is accepted.
- **Exact names:** `MetaBackend::put_file` (`storage/traits.rs:143`, impl `storage/meta.rs:485`)
  gains a parameter `quota_bytes: Option<i64>`. New error code `errors::codes::PROJECT_QUOTA_EXCEEDED
  = "ERR_PROJECT_QUOTA_EXCEEDED"` (`errors.rs`), constructor `ToolError::project_quota_exceeded`,
  HTTP mapping `507`.
- **Priority:** Must-have.

#### FR-NEW-010 [EARS-U]: Write choke point gains quota awareness
> The system SHALL add a field `admin: Arc<dyn AdminBackend>` to `VolumeClient`
> (`storage/volume.rs:10-14`), populated at construction in `StoreManager::client`
> (`storage/mod.rs:426-436`) from the same `AdminBackend` instance already held by `AppState`.
> `VolumeClient::write_bytes_atomic` (`volume.rs:147`), `copy_file` (`volume.rs:196`) and
> `copy_tree` (`volume.rs:218`) SHALL each call `self.admin.get_quota(&self.project_id)` and pass
> the result to `MetaBackend::put_file`'s new `quota_bytes` parameter before every
> `self.meta.put_file(...)` call in those three methods.

- **Business Rules:** no other call site in `core/fs_ops.rs`, `tools/doc.rs`, `tools/git.rs` or
  `git/merge.rs` requires modification: every write path already routes through one of these three
  `VolumeClient` methods — confirmed, the only production call sites of `MetaBackend::put_file` are
  `volume.rs:147,196,218` (e.g. `fs_ops.rs:609` calls `client.write_bytes_atomic`, not `put_file`
  directly).
- **Exact names:** new trait method `AdminBackend::get_quota(&self, project_id: &str) ->
  Result<Option<i64>>` (new on `storage/traits.rs`, impl `storage/admin.rs`). New field
  `VolumeClient::admin: Arc<dyn AdminBackend>`; modified methods `VolumeClient::write_bytes_atomic`,
  `copy_file`, `copy_tree` (`storage/volume.rs:147,196,218`); `StoreManager::client`
  (`storage/mod.rs:426`) gains an `admin: Arc<dyn AdminBackend>` parameter or field to pass through.
- **Priority:** Must-have.

#### FR-NEW-011 [EARS-UB]: No quota bypass via deduplication
> The system SHALL NOT allow `fs.copy`, or any other operation producing a node whose content is
> already stored under another path, to escape FR-NEW-009's `new_total` computation: the destination
> node's declared (logical) size SHALL count in full, exactly as for any other `put_file` call,
> regardless of whether the underlying blob is deduplicated via refcount.

- **Priority:** Must-have.

#### FR-NEW-012 [EARS-UB]: Session quota independence on rejection
> The system SHALL NOT consume the per-session `write_quota_bytes` budget (`safety.rs:131-140`) when
> a write is rejected with `ERR_PROJECT_QUOTA_EXCEEDED`, and SHALL perform the project quota check
> before calling `SafetyManager::charge_write`.

- **Priority:** Must-have.

#### FR-NEW-014 [EARS-O]: Check ordering for quota authorization vs. existence
> WHEN `admin.set_project_quota` or `admin.get_project_quota` is called, THE system SHALL check
> platform-admin authorization (`state.require_admin`, `state.rs:37`) BEFORE checking project
> existence, so that a non-admin caller naming a nonexistent project receives `ERR_FORBIDDEN`, not
> `ERR_PROJECT_NOT_FOUND`.

- **Business Rules:** this is the opposite order from `require_owner_or_admin`'s existence-before-
  ownership precedent (`state.rs:47-53`, `storage/admin.rs:349-356`), deliberately: quota
  authorization has no owner-tier fallback to make existence meaningful to a non-admin caller
  (DEC-003's asymmetric, admin-only authority model).
- **Priority:** Must-have.

#### FR-NEW-013 [EARS-O]: No eviction on quota lowering
> IF a platform admin sets a project's quota to a value lower than that project's current
> `used_bytes` THEN THE system SHALL accept the configuration change, SHALL NOT delete or modify any
> existing node, and SHALL continue rejecting only subsequent writes whose `new_total` (per
> FR-NEW-009) exceeds the new cap.

- **Business Rules:** a write that nets a *decrease* in usage (e.g. shrinking an existing file) is
  accepted even while the project remains over its (now lower) cap, because `new_total` for that
  write is `<= cap` by construction in that case.
- **Priority:** Must-have.

## 7. Non-Functional Requirements

### 7.1 Performance
No explicit throughput target (internal/personal-scale tool, per SPEC-0002 §7.6). The quota check
adds one `SUM`/aggregate read, by `volume_id`, inside the transaction `put_file` already opens; this
must not weaken the existing transactional guarantees of `put_file` (atomicity of the incref/decref
pair). The disk-usage query is bounded by `limit` (default 1000) to avoid returning unbounded result
sets.

### 7.2 Security
No new authentication mechanism. Reuses the existing bearer-token identity (SPEC-0002) and the two
existing authorization tiers (`require_admin`, `require_owner_or_admin`). Not a `--security` entry:
no injection, no auth bypass, no exposure — a new, correctly-gated capability.

### 7.3 Usability
Error message is deliberately short and literal — `"Quota exhausted"` — per explicit product
decision (DEC-001), not a templated string with numbers; the numbers are available via
`admin.get_project_quota` for anyone who needs to investigate.

### 7.4 Reliability
The quota check and the write it guards are atomic with respect to each other (same transaction);
a rejected write leaves no partial state (FR-NEW-009).

### 7.5 Observability
A rejected write SHOULD be traced at the same level as other `ERR_*` rejections already are (no new
tracing infrastructure introduced; this project's existing logging in `fs_ops.rs`/`logging.rs`
already covers tool-level errors). No prompts, tokens or credentials are involved. (Deferred to
implementation: match the existing log fields used for other `ERR_*` rejections; no new field names
are mandated by this spec since none are mandated for the existing `ERR_WRITE_QUOTA_EXCEEDED` either.)

### 7.6 Deployment
No infrastructure change. Schema change applies identically across SQLite, PostgreSQL and SQL Server
via the existing `ColumnMigration` mechanism (`schema.rs:120-127`), already exercised for other
columns.

### 7.7 Scalability
No change in growth assumptions; this feature is itself the cap on a tenant's unbounded growth
(the FinOps motivation stated in BL-0002, confirmed: not tied to a specific incident).

## 8. Data Model

| Entity | Key | Fields (new/changed) | Storage |
|---|---|---|---|
| **Project** | `id` | `+quota_bytes: Option<i64>` (NULL = unlimited) | `project` table (`storage/admin.rs`), `ColumnMigration` (`schema.rs:120`), type `traits.rs:105-111` |
| **Node usage (derived, not stored)** | `volume_id` | `used_bytes = SUM(nodes.size) WHERE volume_id=? AND kind='file'` | computed live against existing `nodes` table, no new column |

## 9. Impact Analysis

### 9.1 Affected Components
| File/Module | Impact | Description |
|---|---|---|
| `crates/core/src/storage/traits.rs` | Modified | `Project.quota_bytes`; new `AdminBackend::get_quota`/`set_quota`; new `MetaBackend::usage_bytes`/`disk_usage`; `put_file` signature gains `quota_bytes: Option<i64>` |
| `crates/core/src/storage/admin.rs` | Modified | implement `get_quota`/`set_quota` against the new column |
| `crates/core/src/storage/meta.rs` | Modified | implement `usage_bytes`/`disk_usage`; in-transaction cap check inside `put_file` (`485-553`) |
| `crates/core/src/storage/rel/schema.rs` | Modified | `ColumnMigration` for `project.quota_bytes` |
| `crates/core/src/errors.rs` | Modified | new `ERR_PROJECT_QUOTA_EXCEEDED`, constructor, HTTP 507 mapping |
| `crates/core/src/mcp/server.rs` | Modified | 3 new `admin.*` tools: `set_project_quota`, `get_project_quota`, `get_project_disk_usage` |
| `crates/core/src/core/fs_ops.rs` | Modified | thread `quota_bytes` through every `put_file` call site (`573,609,797,826,1098,1466,1504,1535,1550`) |
| `crates/core/src/tools/doc.rs` | Modified | `write_docx` call site (`268`) |
| `crates/core/src/tools/git.rs` | Modified | single-blob write (`1402`), bulk import (`3121`) |
| `crates/core/src/git/merge.rs` | Modified | merge-apply call site (`705`) |
| `TOOL_CONTRACT.txt` / `tool-contract-golden.json` | Modified | 3 new tool schemas; regenerate golden via `MCPFS_REWRITE_TOOL_CONTRACT=1` |
| `.agent_docs/tools.md` | Modified | document the 3 new tools, families, parameters, authorization |
| `AGENTS.md` | Modified | add one line to "Behaviour worth knowing" |

### 9.2 Affected Requirements
| Spec | Requirement ID | Impact | Description |
|---|---|---|---|
| SPEC-0002 | §8 Data Model, `Project` row | Extended, not modified | `Project` gains `quota_bytes`; SPEC-0002's own text is unchanged, this spec documents the extension here per Invariant 9 |

### 9.3 Affected Tests
| Test File | Test | Action | Description |
|---|---|---|---|
| `core/fs_ops.rs:3008` | `write_bytes_charges_the_quota_and_audits` | None (verified unmodified) | session quota only, `quota_bytes=None` path, must stay green |
| `core/fs_ops.rs:3022` | `write_bytes_refuses_to_exceed_the_quota` | None (verified unmodified) | same, session quota only |
| (new, 47 tests) | `E2E-NEW-001..047` | New | see §12 |

### 9.4 Affected Documentation
| Document | Section | Action | Description |
|---|---|---|---|
| `AGENTS.md` | Behaviour worth knowing | Update | note the new per-project storage cap alongside the other documented behaviors |
| `.agent_docs/tools.md` | tool families | Update | document 3 new `admin.*` tools |
| `TOOL_CONTRACT.txt` | tool list | Update | 3 new tool entries, human-readable |

### 9.5 Dependencies & Risks
No new crate dependencies. Risk: the `ColumnMigration` must be exercised against all three dialects
via the existing `conformance.rs` suite before merge. Rollback: `ColumnMigration` is additive-only
(no destructive down-migration exists in this framework); rollback means simply stopping use of the
new column and tools — acceptable, matches how every other `ColumnMigration` in this codebase is
already treated.

## 10. Documentation Requirements

`AGENTS.md` "Behaviour worth knowing" gains one line about the per-project storage cap and the
`"Quota exhausted"` error. `.agent_docs/tools.md` documents the 3 new `admin.*` tools (parameters,
authorization tier, exact error codes). `TOOL_CONTRACT.txt` and `tool-contract-golden.json` gain the
3 new tool schemas (the golden file is regenerated via `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p
mcp-fs --lib tool_contract_golden_is_current`, diff reviewed).

## 11. Traceability Matrix

| Scenario | Functional Req | E2E (Happy) | E2E (Failure) | E2E (Edge) |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-013, FR-NEW-014 | E2E-NEW-001, E2E-NEW-004, E2E-NEW-041 | E2E-NEW-002, E2E-NEW-003, E2E-NEW-037, E2E-NEW-051 | E2E-NEW-029, E2E-NEW-040, E2E-NEW-044, E2E-NEW-045, E2E-NEW-049 |
| SC-002 | FR-NEW-005, FR-NEW-006, FR-NEW-014 | E2E-NEW-005 | E2E-NEW-007, E2E-NEW-052 | E2E-NEW-006, E2E-NEW-034, E2E-NEW-036, E2E-NEW-038 |
| SC-003 | FR-NEW-009, FR-NEW-010, FR-NEW-012, FR-NEW-013 | E2E-NEW-014, E2E-NEW-046, E2E-NEW-047, E2E-NEW-050 | E2E-NEW-048 | E2E-NEW-031, E2E-NEW-032 |
| SC-004 | FR-NEW-009, FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013 | E2E-NEW-020 | E2E-NEW-015, E2E-NEW-018, E2E-NEW-019, E2E-NEW-021, E2E-NEW-022, E2E-NEW-023, E2E-NEW-024, E2E-NEW-025, E2E-NEW-026, E2E-NEW-027, E2E-NEW-030, E2E-NEW-042 | E2E-NEW-016, E2E-NEW-017, E2E-NEW-028, E2E-NEW-033, E2E-NEW-039 |
| SC-005 | FR-NEW-007, FR-NEW-008 | E2E-NEW-008, E2E-NEW-009 | E2E-NEW-010 | E2E-NEW-011, E2E-NEW-012, E2E-NEW-013, E2E-NEW-035, E2E-NEW-043 |

## 12. End-to-End Test Suite

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-001 | New | happy | SC-001 | FR-NEW-002, FR-NEW-003, FR-NEW-004 | P0 |
| E2E-NEW-002 | New | failure | SC-001 | FR-NEW-004 | P0 |
| E2E-NEW-003 | New | security | SC-001 | FR-NEW-003, FR-NEW-014 | P0 |
| E2E-NEW-004 | New | state-transition | SC-001 | FR-NEW-002, FR-NEW-003 | P1 |
| E2E-NEW-005 | New | happy | SC-002 | FR-NEW-005, FR-NEW-006 | P0 |
| E2E-NEW-006 | New | edge | SC-002 | FR-NEW-005 | P1 |
| E2E-NEW-007 | New | security | SC-002 | FR-NEW-006, FR-NEW-014 | P0 |
| E2E-NEW-008 | New | happy | SC-005 | FR-NEW-007, FR-NEW-008 | P0 |
| E2E-NEW-009 | New | happy | SC-005 | FR-NEW-007, FR-NEW-008 | P0 |
| E2E-NEW-010 | New | security | SC-005 | FR-NEW-008 | P0 |
| E2E-NEW-011 | New | data-integrity | SC-005 | FR-NEW-007 | P1 |
| E2E-NEW-012 | New | edge | SC-005 | FR-NEW-007 | P1 |
| E2E-NEW-013 | New | edge | SC-005 | FR-NEW-007 | P2 |
| E2E-NEW-014 | New | happy | SC-003 | FR-NEW-009, FR-NEW-010, FR-NEW-012 | P0 |
| E2E-NEW-015 | New | failure | SC-004 | FR-NEW-009 | P0 |
| E2E-NEW-016 | New | side-effect | SC-004 | FR-NEW-009 | P0 |
| E2E-NEW-017 | New | side-effect | SC-004 | FR-NEW-012 | P0 |
| E2E-NEW-018 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-019 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-020 | New | edge (boundary, allow) | SC-004 | FR-NEW-010 | P2 |
| E2E-NEW-021 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-022 | New | failure | SC-004 | FR-NEW-010, FR-NEW-011 | P1 |
| E2E-NEW-023 | New | failure | SC-004 | FR-NEW-010 | P2 |
| E2E-NEW-024 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-025 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-026 | New | failure | SC-004 | FR-NEW-010 | P1 |
| E2E-NEW-027 | New | failure | SC-004 | FR-NEW-010 | P0 |
| E2E-NEW-028 | New | data-integrity | SC-004 | FR-NEW-011 | P0 |
| E2E-NEW-029 | New | state-transition | SC-001 | FR-NEW-013 | P0 |
| E2E-NEW-030 | New | failure | SC-004 | FR-NEW-013 | P1 |
| E2E-NEW-031 | New | edge/state-transition | SC-003 | FR-NEW-013 | P1 |
| E2E-NEW-032 | New | edge (boundary, allow) | SC-003 | FR-NEW-009 | P1 |
| E2E-NEW-033 | New | edge (boundary, reject) | SC-004 | FR-NEW-009 | P1 |
| E2E-NEW-034 | New | edge (empty) | SC-002 | FR-NEW-005 | P2 |
| E2E-NEW-035 | New | edge (empty) | SC-005 | FR-NEW-007 | P2 |
| E2E-NEW-036 | New | data-integrity | SC-002 | FR-NEW-005, FR-NEW-006 | P0 |
| E2E-NEW-037 | New | failure | SC-001 | FR-NEW-002, FR-NEW-014 | P2 |
| E2E-NEW-038 | New | edge/data-integrity | SC-002 | FR-NEW-005, FR-NEW-009 | P1 |
| E2E-NEW-039 | New | state-transition (concurrency) | SC-004 | FR-NEW-009 | P0 |
| E2E-NEW-040 | New | state-transition (schema) | SC-001 | FR-NEW-001 | P0 |
| E2E-NEW-041 | New | happy (schema) | SC-001 | FR-NEW-001 | P1 |
| E2E-NEW-042 | New | failure | SC-004 | FR-NEW-009 | P0 |
| E2E-NEW-043 | New | performance | SC-005 | FR-NEW-007 | P2 |
| E2E-NEW-044 | New | state-transition | SC-001 | FR-NEW-002, FR-NEW-013 | P2 |
| E2E-NEW-045 | New | data-integrity (schema) | SC-001 | FR-NEW-001 | P1 |
| E2E-NEW-046 | New | happy | SC-003 | FR-NEW-009, FR-NEW-011 | P1 |
| E2E-NEW-047 | New | happy | SC-003 | FR-NEW-009, FR-NEW-010 | P1 |
| E2E-NEW-048 | New | failure | SC-003 | FR-NEW-009, FR-NEW-010 | P2 |
| E2E-NEW-049 | New | edge (boundary, allow) | SC-001 | FR-NEW-002, FR-NEW-004 | P2 |
| E2E-NEW-050 | New | happy | SC-003 | FR-NEW-009, FR-NEW-012 | P1 |
| E2E-NEW-051 | New | security | SC-001 | FR-NEW-014 | P0 |
| E2E-NEW-052 | New | security | SC-002 | FR-NEW-014 | P0 |

**Coverage statistics:** happy 9, failure 15, side effects 2, edge 11, state transitions 5,
security 5, data integrity 4, performance 1. **Happy:failure ratio 9:15 ≈ 1:1.67**, beats 1:1.

### 12.2 New Test Specifications

Common fixtures unless stated otherwise: `mount_id = "quota-mount"`, project `proj-quota-e2e`
(`volume_id = "vol-quota-e2e"`), platform admin `admin@test.local`, project owner `owner@test.local`
(owner of `proj-quota-e2e`), unrelated identity `intruder@test.local` (no membership on
`proj-quota-e2e`). `1 MB = 1_048_576` bytes.

#### E2E-NEW-001: Admin sets a quota
- **Category:** Core Journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-003 (contrast: the admin caller passes the authorization gate), FR-NEW-004 (contrast: a valid non-zero `max_mb` is accepted)
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` exists, `quota_bytes = NULL`.
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = NULL`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=10)`
  - Then the response is exactly `{"project_id":"proj-quota-e2e","max_mb":10}`
  - And a direct SQL read (`SELECT quota_bytes FROM project WHERE id='proj-quota-e2e'`) returns
    `10485760`
- **Priority:** Critical

#### E2E-NEW-002: Zero max_mb is rejected
- **Category:** Error
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` exists with any prior `quota_bytes` value `V`.
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = V`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=0)`
  - Then the call fails with `ERR_INVALID_ARGUMENT`, message exactly `"max_mb must be > 0"`
  - And a direct SQL read confirms `quota_bytes` is still `V` (unchanged)
- **Priority:** Critical

#### E2E-NEW-003: Non-admin cannot set the quota
- **Category:** Security
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Driver:** MCP tool call
- **Preconditions:** `owner@test.local` is the owner of `proj-quota-e2e` but not a platform admin.
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = NULL`
  - When `owner@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=5)`
  - Then the call fails with `ERR_FORBIDDEN`
  - And a direct SQL read confirms `quota_bytes` is still `NULL` (no mutation, read through a
    channel other than the rejected call)
- **Priority:** Critical

#### E2E-NEW-004: Admin clears a previously set quota
- **Category:** State Transition
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-003 (contrast: the admin caller passes the authorization gate)
- **Driver:** MCP tool call + `fs.write_bytes`
- **Preconditions:** `proj-quota-e2e.quota_bytes = 10485760` (from E2E-NEW-001).
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = 10485760`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=None)`
  - Then the response is `{"project_id":"proj-quota-e2e","max_mb":null}`
  - And a direct SQL read confirms `quota_bytes IS NULL`
  - And a subsequent `fs.write_bytes(mount_id="quota-mount", path="/big.bin", content=<52,000,000 bytes>)` succeeds
- **Priority:** High

#### E2E-NEW-005: Admin reads quota and usage together
- **Category:** Feature
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005, FR-NEW-006 (contrast: the admin caller passes the authorization gate)
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = 10485760`; two file nodes exist, sizes 1000 and
  2000 bytes.
- **Steps:**
  - Given `proj-quota-e2e` has file nodes totaling 3000 bytes and `quota_bytes = 10485760`
  - When `admin@test.local` calls `admin.get_project_quota(project_id="proj-quota-e2e")`
  - Then the response is exactly `{"project_id":"proj-quota-e2e","max_mb":10,"used_bytes":3000}`
- **Priority:** Critical

#### E2E-NEW-006: Unlimited quota reads back as null
- **Category:** Edge Case
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = NULL`; same 3000-byte fixture.
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = NULL` and `used_bytes = 3000` (by construction of the fixture)
  - When `admin@test.local` calls `admin.get_project_quota(project_id="proj-quota-e2e")`
  - Then the response is exactly `{"project_id":"proj-quota-e2e","max_mb":null,"used_bytes":3000}`
- **Priority:** High

#### E2E-NEW-007: Non-admin cannot read the quota
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-NEW-006
- **Driver:** MCP tool call
- **Preconditions:** `intruder@test.local` has no membership on `proj-quota-e2e` and is not a
  platform admin.
- **Steps:**
  - Given `proj-quota-e2e` exists
  - When `intruder@test.local` calls `admin.get_project_quota(project_id="proj-quota-e2e")`
  - Then the call fails with `ERR_FORBIDDEN`
- **Priority:** Critical

#### E2E-NEW-008: Owner reads the disk-usage breakdown
- **Category:** Feature
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007, FR-NEW-008 (contrast: the owner caller passes the authorization gate)
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` has file nodes `/a.txt` (500), `/b/c.txt` (2000), `/d.bin` (100).
- **Steps:**
  - Given the three file nodes above exist under `vol-quota-e2e`
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e", limit=None)`
  - Then the response is exactly `{"project_id":"proj-quota-e2e","entries":[{"path":"/b/c.txt","size":2000},{"path":"/a.txt","size":500},{"path":"/d.bin","size":100}],"truncated":false}`
- **Priority:** Critical

#### E2E-NEW-009: Admin reads the same breakdown independent of membership
- **Category:** Feature
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007, FR-NEW-008 (contrast: the admin caller passes the authorization gate independent of membership)
- **Driver:** MCP tool call
- **Preconditions:** same fixture as E2E-NEW-008; `admin@test.local` is a platform admin and not a
  member of `proj-quota-e2e`.
- **Steps:**
  - Given the fixture from E2E-NEW-008
  - When `admin@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e")`
  - Then the response payload is identical to E2E-NEW-008's
- **Priority:** Critical

#### E2E-NEW-010: Non-owner, non-admin cannot read disk usage
- **Category:** Security
- **Scenario:** SC-005
- **Requirements:** FR-NEW-008
- **Driver:** MCP tool call
- **Preconditions:** `intruder@test.local` is neither owner nor platform admin.
- **Steps:**
  - Given `proj-quota-e2e` exists
  - When `intruder@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e")`
  - Then the call fails with `ERR_FORBIDDEN`
- **Priority:** Critical

#### E2E-NEW-011: Entries are sorted strictly by size descending
- **Category:** Data Integrity
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007
- **Driver:** MCP tool call
- **Preconditions:** fixture from E2E-NEW-008 (sizes 500, 2000, 100).
- **Steps:**
  - Given the fixture from E2E-NEW-008
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e", limit=1000)`
  - Then `entries` is `[2000, 500, 100]` in that order
  - And `truncated` is `false` (3 <= 1000)
- **Priority:** High

#### E2E-NEW-012: `limit` truncates to the N largest
- **Category:** Edge Case (boundary)
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` has exactly 5 file nodes of 5 distinct sizes.
- **Steps:**
  - Given 5 file nodes of distinct sizes exist
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e", limit=3)`
  - Then exactly 3 entries are returned, the 3 largest, and `truncated` is `true`
- **Priority:** High

#### E2E-NEW-013: Default limit of 1000 applies when omitted
- **Category:** Edge Case
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` has 1200 file nodes, each 1 byte.
- **Steps:**
  - Given 1200 one-byte file nodes exist
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e")` (no `limit`)
  - Then exactly 1000 entries are returned and `truncated` is `true`
- **Priority:** Medium

#### E2E-NEW-014: A write under quota behaves exactly as before
- **Category:** Core Journey
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-010, FR-NEW-012 (contrast: the session write-quota is still charged exactly as before, since the project quota check passed and never short-circuits `charge_write` for an accepted write)
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = 10485760`, `used_bytes = 3000`; session `bytes_written = B` before the call.
- **Steps:**
  - Given `used_bytes = 3000`, `quota_bytes = 10485760`, session `bytes_written = B`
  - When `fs.write_bytes(mount_id="quota-mount", path="/new.bin", content=<2000 bytes>)` is called
  - Then the call succeeds with the same response shape as before this feature existed
  - And a direct SQL read confirms `used_bytes` is now `5000`
  - And the session's `bytes_written` is now exactly `B + 2000` (charged normally, FR-NEW-012 only changes behavior on rejection)
- **Priority:** Critical

#### E2E-NEW-015: A write that would exceed quota is rejected
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-009
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = 10485760`, `used_bytes = 10480000`.
- **Steps:**
  - Given `used_bytes = 10480000`, `quota_bytes = 10485760`
  - When `fs.write_bytes(mount_id="quota-mount", path="/overflow.bin", content=<10000 bytes>)` is called (`new_total = 10490000`)
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`, message exactly `"Quota exhausted"`, HTTP mapping `507`
- **Priority:** Critical

#### E2E-NEW-016: A rejected write leaves no partial node or blob state
- **Category:** Side Effect
- **Scenario:** SC-004
- **Requirements:** FR-NEW-009
- **Driver:** direct SQL, read through a channel other than the rejected call
- **Preconditions:** same as E2E-NEW-015; record the content's `sha256` and its pre-call refcount.
- **Steps:**
  - Given the state and rejected call from E2E-NEW-015
  - When the rejected write executes
  - Then a direct SQL read shows no row in `nodes` for `/overflow.bin`
  - And the blob's refcount for that `sha256` is unchanged from its pre-call value
  - And `used_bytes` is unchanged at `10480000`
- **Priority:** Critical

#### E2E-NEW-017: A rejected write does not consume the session write-quota budget
- **Category:** Side Effect
- **Scenario:** SC-004
- **Requirements:** FR-NEW-012
- **Driver:** direct inspection of `SessionState.bytes_written` (`safety.rs:81`), a channel other than
  the rejected MCP call
- **Preconditions:** same as E2E-NEW-015; session's `bytes_written` is at a known value `B` before
  the call.
- **Steps:**
  - Given the session's `bytes_written = B`
  - When the rejected write from E2E-NEW-015 executes
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And the session's `bytes_written` is still exactly `B` (unchanged — `charge_write` was never
    invoked)
- **Priority:** Critical

#### E2E-NEW-018: `fs.write` is enforced
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** project with `quota_bytes = 1048576`, `used_bytes = 1048000`.
- **Steps:**
  - Given `used_bytes = 1048000`, `quota_bytes = 1048576`
  - When `fs.write(mount_id="quota-mount", path="/w.txt", content=<1000 bytes>)` is called
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node exists for `/w.txt`
- **Priority:** High

#### E2E-NEW-019: `fs.append` is enforced
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `/a.txt` exists at 1048000 bytes under a project with `quota_bytes = 1048576`.
- **Steps:**
  - Given `/a.txt` is 1048000 bytes
  - When `fs.append(mount_id="quota-mount", path="/a.txt", content=<1000 bytes>)` is called
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms `/a.txt` is still 1048000 bytes
- **Priority:** High

#### E2E-NEW-020: A zero-byte write exactly at the cap is allowed
- **Category:** Edge Case (boundary, allow)
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `used_bytes == quota_bytes` exactly.
- **Steps:**
  - Given `used_bytes = quota_bytes = 1000000`
  - When `fs.create_empty(mount_id="quota-mount", path="/empty.txt")` is called (declared size 0,
    `new_total = 1000000`)
  - Then the call succeeds (comparison is strict `>`, `1000000 > 1000000` is false)
  - And a direct SQL read confirms `/empty.txt` exists with `size = 0`
- **Priority:** Medium

#### E2E-NEW-021: An edit that would grow a file past the cap is fully rolled back
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `/doc.md` exists at 1048000 bytes under `quota_bytes = 1048576`.
- **Steps:**
  - Given `/doc.md` is 1048000 bytes
  - When an edit tool call (`fs.edit`) that grows `/doc.md` by 2000 bytes is issued
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms `/doc.md`'s content/size is unchanged at 1048000 bytes (not
    partially applied)
- **Priority:** High

#### E2E-NEW-022: `fs.copy` is enforced on the destination's declared size
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010, FR-NEW-011
- **Driver:** MCP tool call
- **Preconditions:** `/src.bin` exists (500000 bytes), `quota_bytes = 600000`, `used_bytes = 500000`.
- **Steps:**
  - Given `/src.bin` is 500000 bytes, `used_bytes = 500000`, `quota_bytes = 600000`
  - When `fs.copy(mount_id="quota-mount", source="/src.bin", dest="/dst.bin")` is called
    (`new_total = 1000000`)
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node exists for `/dst.bin`
- **Priority:** High

#### E2E-NEW-023: `fs.write_docx` is enforced
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `quota_bytes = 1048576`, `used_bytes = 1040000`; the requested docx renders to
  20000 bytes.
- **Steps:**
  - Given `used_bytes = 1040000`, `quota_bytes = 1048576`
  - When `fs.write_docx(mount_id="quota-mount", path="/report.docx", ...)` renders a 20000-byte file
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node exists for `/report.docx`
- **Priority:** Medium

#### E2E-NEW-024: `git push` (single-blob write) is enforced
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** `git push` over the smart HTTP endpoint
- **Preconditions:** git-enabled project `proj-quota-git`, `quota_bytes = 1048576`,
  `used_bytes = 1040000`; local clone has a new 20000-byte file committed.
- **Steps:**
  - Given `used_bytes = 1040000`, `quota_bytes = 1048576` on `proj-quota-git`
  - When `git push` to `/git/proj-quota-git/` is performed with the 20000-byte addition
  - Then the push is rejected and the error attributable to the push surfaces
    `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node was created for the new file and the ref did not advance
- **Priority:** High

#### E2E-NEW-025: git bulk import is rejected without partial landing
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** MCP tool call (`git.*` import tool)
- **Preconditions:** same quota setup as E2E-NEW-024; import tree's declared total size exceeds
  remaining headroom.
- **Steps:**
  - Given the same quota setup as E2E-NEW-024
  - When the git bulk import tool is invoked with an oversized tree
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms zero nodes were created from the import batch
- **Priority:** High

#### E2E-NEW-026: git merge-apply stays atomic under the cap
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** `git.*` combine/merge tool call
- **Preconditions:** same quota setup as E2E-NEW-024; the merge's resulting applied tree exceeds the
  cap.
- **Steps:**
  - Given the same quota setup as E2E-NEW-024
  - When a three-way merge whose applied result exceeds the cap is invoked
  - Then the response carries a rejection attributable to `ERR_PROJECT_QUOTA_EXCEEDED` (not a
    `status: "conflict"` response)
  - And a direct SQL read confirms no new nodes were created and the operation is not marked merged
- **Priority:** High

#### E2E-NEW-027: REST `/upload` inherits enforcement
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-010
- **Driver:** HTTP client against `/api/fs/upload`
- **Preconditions:** `quota_bytes = 1048576`, `used_bytes = 1040000`.
- **Steps:**
  - Given `used_bytes = 1040000`, `quota_bytes = 1048576`
  - When `POST /api/fs/upload?mount_id=quota-mount&path=/u.bin` is called with a 20000-byte body
  - Then the HTTP response status is `507` and the body's error code is `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node exists for `/u.bin`
- **Priority:** Critical

#### E2E-NEW-028: Deduplicated blobs still count fully for quota
- **Category:** Data Integrity
- **Scenario:** SC-004
- **Requirements:** FR-NEW-011
- **Driver:** MCP tool call + direct SQL
- **Preconditions:** `quota_bytes = 1500000`, `used_bytes = 0`; blob `B` is 1000000 bytes.
- **Steps:**
  - Given `used_bytes = 0`, `quota_bytes = 1500000`
  - When `fs.write_bytes(path="/p1.bin", content=B)` succeeds (`used_bytes` becomes `1000000`)
  - And then `fs.copy(source="/p1.bin", dest="/p2.bin")` is called (`new_total = 2000000`)
  - Then the copy fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms the blob's refcount for `B`'s `sha256` stays at `1` and no node
    exists for `/p2.bin`
- **Priority:** Critical

#### E2E-NEW-029: Lowering the quota below current usage does not evict anything
- **Category:** State Transition
- **Scenario:** SC-001
- **Requirements:** FR-NEW-013
- **Driver:** MCP tool call + direct SQL
- **Preconditions:** `quota_bytes = 10485760`, `used_bytes = 5000000` across 2 file nodes.
- **Steps:**
  - Given `used_bytes = 5000000`, `quota_bytes = 10485760`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=1)`
  - Then the call succeeds with `{"project_id":"proj-quota-e2e","max_mb":1}`
  - And a direct SQL read confirms both pre-existing nodes are unchanged (same count, sizes, `sha256`)
  - And a subsequent `admin.get_project_quota` call still reports `used_bytes = 5000000`
- **Priority:** Critical

#### E2E-NEW-030: After lowering, growth-only writes still rejected
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-013
- **Driver:** MCP tool call
- **Preconditions:** state from E2E-NEW-029 (`quota_bytes = 1048576`, `used_bytes = 5000000`, already
  over cap).
- **Steps:**
  - Given `quota_bytes = 1048576`, `used_bytes = 5000000`
  - When `fs.write_bytes(path="/more.bin", content=<10 bytes>)` is called (`new_total = 5000010`)
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node for `/more.bin` and `used_bytes` unchanged at `5000000`
- **Priority:** High

#### E2E-NEW-031: A net-shrinking write is allowed even while over cap
- **Category:** Edge Case / State Transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-013
- **Driver:** MCP tool call
- **Preconditions:** state from E2E-NEW-029; `/big.bin` is one of the 2 nodes, at 4000000 bytes.
- **Steps:**
  - Given `/big.bin` is 4000000 bytes, `used_bytes = 5000000`, `quota_bytes = 1048576`
  - When `fs.write_bytes(path="/big.bin", content=<40000 bytes>)` overwrites it (`new_total =
    5000000 - 4000000 + 40000 = 1040000`)
  - Then the call succeeds (`1040000 <= 1048576`)
  - And a direct SQL read confirms `/big.bin` is now `40000` bytes and `used_bytes = 1040000`
- **Priority:** High

#### E2E-NEW-032: A write landing exactly at the cap is allowed
- **Category:** Edge Case (boundary, allow)
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009
- **Driver:** MCP tool call
- **Preconditions:** `quota_bytes = 1000000`, `used_bytes = 999000`.
- **Steps:**
  - Given `used_bytes = 999000`, `quota_bytes = 1000000`
  - When `fs.write_bytes(path="/exact.bin", content=<1000 bytes>)` is called (`new_total = 1000000`)
  - Then the call succeeds
  - And a direct SQL read confirms `used_bytes = 1000000`
- **Priority:** High

#### E2E-NEW-033: A write landing one byte over the cap is rejected
- **Category:** Edge Case (boundary, reject)
- **Scenario:** SC-004
- **Requirements:** FR-NEW-009
- **Driver:** MCP tool call
- **Preconditions:** `quota_bytes = 1000000`, `used_bytes = 999000`.
- **Steps:**
  - Given `used_bytes = 999000`, `quota_bytes = 1000000`
  - When `fs.write_bytes(path="/overby1.bin", content=<1001 bytes>)` is called (`new_total = 1000001`)
  - Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms no node exists for `/overby1.bin` and `used_bytes` unchanged at
    `999000`
- **Priority:** High

#### E2E-NEW-034: Quota and usage on an empty project
- **Category:** Edge Case (empty)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005
- **Driver:** MCP tool call
- **Preconditions:** new project `proj-quota-empty`, `quota_bytes = 5242880`, zero file nodes.
- **Steps:**
  - Given `proj-quota-empty` has zero file nodes and `quota_bytes = 5242880`
  - When `admin@test.local` calls `admin.get_project_quota(project_id="proj-quota-empty")`
  - Then the response is exactly `{"project_id":"proj-quota-empty","max_mb":5,"used_bytes":0}`
- **Priority:** Medium

#### E2E-NEW-035: Disk usage on an empty project
- **Category:** Edge Case (empty)
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-empty` has zero file nodes.
- **Steps:**
  - Given `proj-quota-empty` has zero file nodes
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-empty")`
  - Then the response is exactly `{"project_id":"proj-quota-empty","entries":[],"truncated":false}`
- **Priority:** Medium

#### E2E-NEW-036: `used_bytes` matches an independent SQL sum
- **Category:** Data Integrity
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005, FR-NEW-006 (contrast: the admin caller passes the authorization gate)
- **Driver:** MCP tool call cross-checked against direct SQL
- **Preconditions:** `proj-quota-e2e`'s file nodes sum to `7500000` bytes, confirmed independently
  via `SELECT SUM(size) FROM nodes WHERE volume_id='vol-quota-e2e' AND kind='file'`.
- **Steps:**
  - Given the independent SQL sum is `7500000`
  - When `admin@test.local` calls `admin.get_project_quota(project_id="proj-quota-e2e")`
  - Then the response's `used_bytes` is exactly `7500000`
- **Priority:** Critical

#### E2E-NEW-037: Setting a quota on a nonexistent project fails
- **Category:** Error
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Driver:** MCP tool call
- **Preconditions:** no project with id `proj-does-not-exist`.
- **Steps:**
  - Given no project `proj-does-not-exist` exists
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-does-not-exist", max_mb=10)`
  - Then the call fails with `ERR_PROJECT_NOT_FOUND`
- **Priority:** Medium

#### E2E-NEW-038: Trashed files still count toward usage
- **Category:** Edge Case / Data Integrity
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005, FR-NEW-009
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = 1000000`; `/temp.txt` (200000 bytes) is soft-deleted
  (moved under `/.mcp_trash/...`, per `safety.rs:179-183`), not purged.
- **Steps:**
  - Given `/temp.txt` (200000 bytes) has been soft-deleted into trash
  - When `admin@test.local` calls `admin.get_project_quota(project_id="proj-quota-e2e")` immediately after
  - Then `used_bytes` still includes the 200000 trashed bytes
  - And a subsequent `fs.write_bytes` of content that would push total usage past `1000000` is
    rejected with `ERR_PROJECT_QUOTA_EXCEEDED`
- **Priority:** High

#### E2E-NEW-039: Concurrent writers never jointly overshoot the cap
- **Category:** Side Effect / State Transition (concurrency)
- **Scenario:** SC-004
- **Requirements:** FR-NEW-009
- **Driver:** two concurrent MCP tool calls
- **Preconditions:** `quota_bytes = 1000000`, `used_bytes = 900000` (100000 bytes of headroom).
- **Steps:**
  - Given `used_bytes = 900000`, `quota_bytes = 1000000`
  - When two concurrent `fs.write_bytes` calls are issued simultaneously, each writing a new
    80000-byte file (`/race1.bin`, `/race2.bin`)
  - Then exactly one call succeeds and the other fails with `ERR_PROJECT_QUOTA_EXCEEDED` (never both
    succeed, never both fail)
  - And a direct SQL read confirms `used_bytes` is exactly `980000` (one successful 80000-byte write,
    not two)
- **Priority:** Critical

#### E2E-NEW-040: Migration backfills NULL for pre-existing projects
- **Category:** State Transition (schema)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Driver:** direct SQL after server startup against a pre-migration snapshot
- **Preconditions:** a database snapshot with projects created before this feature, no `quota_bytes`
  column populated.
- **Steps:**
  - Given a pre-migration project row exists
  - When the server starts and the `ColumnMigration` for `project.quota_bytes` runs
  - Then a direct SQL read confirms the column exists and every pre-existing row has
    `quota_bytes IS NULL`
- **Priority:** Critical

#### E2E-NEW-041: New projects default to unlimited
- **Category:** Feature
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Driver:** MCP tool call (`admin.create_project`) + direct SQL
- **Preconditions:** migration from E2E-NEW-040 already applied.
- **Steps:**
  - Given the migration is applied
  - When `admin@test.local` creates a new project `proj-quota-new` via `admin.create_project`
  - Then a direct SQL read confirms `quota_bytes IS NULL` for `proj-quota-new`
- **Priority:** High

#### E2E-NEW-042: Exact error payload across both transports
- **Category:** Error
- **Scenario:** SC-004
- **Requirements:** FR-NEW-009
- **Driver:** MCP tool call AND HTTP client (same scenario, both transports in one test)
- **Preconditions:** `quota_bytes = 1048576`, `used_bytes = 1048566`.
- **Steps:**
  - Given `used_bytes = 1048566`, `quota_bytes = 1048576`
  - When `fs.write_bytes(path="/exactmsg.bin", content=<20 bytes>)` is called over MCP
  - Then the MCP error payload has `error_code == "ERR_PROJECT_QUOTA_EXCEEDED"` and
    `message == "Quota exhausted"` exactly
  - And when the REST-equivalent `POST /api/fs/upload` is called with the same oversized content
  - Then the HTTP response status is `507` and the body's error code/message pair match the MCP
    payload exactly (no drift between transports)
- **Priority:** Critical

#### E2E-NEW-043: Disk-usage query completes within budget at scale
- **Category:** Performance
- **Scenario:** SC-005
- **Requirements:** FR-NEW-007
- **Driver:** MCP tool call, wall-clock measured
- **Preconditions:** `proj-quota-e2e` has 50000 file nodes of varying sizes.
- **Steps:**
  - Given 50000 file nodes exist
  - When `owner@test.local` calls `admin.get_project_disk_usage(project_id="proj-quota-e2e", limit=1000)`
  - Then the response returns the top 1000 by size, `truncated=true`, within 2 seconds
- **Priority:** Low

#### E2E-NEW-044: Raising the quota lifts a previously-rejecting cap
- **Category:** State Transition
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-013
- **Driver:** MCP tool call
- **Preconditions:** `quota_bytes = 1048576`, `used_bytes = 500000`.
- **Steps:**
  - Given `quota_bytes = 1048576`, `used_bytes = 500000`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=2)`
  - Then the call succeeds with `{"project_id":"proj-quota-e2e","max_mb":2}`
  - And a direct SQL read confirms `quota_bytes = 2097152` and `used_bytes` unchanged at `500000`
  - And a write of 600000 new bytes (`new_total = 1100000`, which would have failed under the old
    1 MB cap) now succeeds under the new 2 MB cap
- **Priority:** Medium

#### E2E-NEW-045: The migrated column is nullable and typed correctly
- **Category:** Data Integrity (schema)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Driver:** schema introspection (`PRAGMA table_info` on SQLite; `information_schema.columns` on
  PostgreSQL/SQL Server)
- **Preconditions:** migration applied, per dialect under test.
- **Steps:**
  - Given the migration from E2E-NEW-040 has run on the dialect under test
  - When the `project` table's schema is introspected for the `quota_bytes` column
  - Then it is nullable and of a big-integer type (`INTEGER`/`BIGINT` per dialect, matching
    `nodes.size`'s own type mapping at `schema.rs:361,370`)
- **Priority:** High

#### E2E-NEW-046: `fs.copy` under quota succeeds unchanged
- **Category:** Core Journey
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-011
- **Driver:** MCP tool call
- **Preconditions:** `/src2.bin` exists (1000 bytes), `quota_bytes = 1000000`, `used_bytes = 1000`.
- **Steps:**
  - Given `/src2.bin` is 1000 bytes, `used_bytes = 1000`, `quota_bytes = 1000000`
  - When `fs.copy(source="/src2.bin", dest="/dst2.bin")` is called (`new_total = 2000`)
  - Then the call succeeds, identical to pre-increment behavior
  - And a direct SQL read confirms `used_bytes = 2000`
- **Priority:** High

#### E2E-NEW-047: `fs.append` under quota succeeds unchanged
- **Category:** Core Journey
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `/log.txt` exists (1000 bytes), `quota_bytes = 1000000`, `used_bytes = 1000`.
- **Steps:**
  - Given `/log.txt` is 1000 bytes, `used_bytes = 1000`, `quota_bytes = 1000000`
  - When `fs.append(path="/log.txt", content=<500 bytes>)` is called (`new_total = 1500`)
  - Then the call succeeds, identical to pre-increment behavior
  - And a direct SQL read confirms `/log.txt` is now `1500` bytes
- **Priority:** High

#### E2E-NEW-048: Quota enforcement does not mask or reorder an unrelated failure
- **Category:** Error
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-010
- **Driver:** MCP tool call
- **Preconditions:** `/existing.bin` exists (1000 bytes) under `quota_bytes = 1000000`,
  `used_bytes = 1000` (ample headroom); `fs.write` is called with `no_clobber=true` against the same
  path.
- **Steps:**
  - Given `/existing.bin` exists and ample quota headroom remains
  - When `fs.write(mount_id="quota-mount", path="/existing.bin", content=<10 bytes>, no_clobber=true)`
    is called
  - Then the call fails with `ERR_NO_CLOBBER` (the pre-existing no-clobber contract), not
    `ERR_PROJECT_QUOTA_EXCEEDED`
  - And a direct SQL read confirms `/existing.bin` is unchanged and `used_bytes` is unchanged at
    `1000` — proving the quota check neither suppresses nor is suppressed by an unrelated, already
    established failure mode when there is ample headroom
- **Priority:** Medium

#### E2E-NEW-049: The minimum valid `max_mb` is accepted
- **Category:** Edge Case (boundary, allow)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-004
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e` exists, `quota_bytes = NULL`.
- **Steps:**
  - Given `proj-quota-e2e.quota_bytes = NULL`
  - When `admin@test.local` calls `admin.set_project_quota(project_id="proj-quota-e2e", max_mb=1)`
  - Then the response is exactly `{"project_id":"proj-quota-e2e","max_mb":1}`
  - And a direct SQL read confirms `quota_bytes = 1048576` — the smallest valid non-zero value is
    accepted, contrasting with E2E-NEW-002's rejection of `0`
- **Priority:** Low

#### E2E-NEW-051: Non-admin naming a nonexistent project still gets forbidden, not not-found
- **Category:** Security
- **Scenario:** SC-001
- **Requirements:** FR-NEW-014
- **Driver:** MCP tool call
- **Preconditions:** `owner@test.local` is not a platform admin; no project `proj-ghost` exists.
- **Steps:**
  - Given no project `proj-ghost` exists and `owner@test.local` is not a platform admin
  - When `owner@test.local` calls `admin.set_project_quota(project_id="proj-ghost", max_mb=10)`
  - Then the call fails with `ERR_FORBIDDEN`, not `ERR_PROJECT_NOT_FOUND` — proving the authorization
    check runs before the existence check (FR-NEW-014)
- **Priority:** Critical

#### E2E-NEW-052: Non-admin naming a nonexistent project cannot distinguish it via the quota read
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-NEW-014
- **Driver:** MCP tool call
- **Preconditions:** `intruder@test.local` is not a platform admin; no project `proj-ghost` exists.
- **Steps:**
  - Given no project `proj-ghost` exists and `intruder@test.local` is not a platform admin
  - When `intruder@test.local` calls `admin.get_project_quota(project_id="proj-ghost")`
  - Then the call fails with `ERR_FORBIDDEN`, not `ERR_PROJECT_NOT_FOUND`
- **Priority:** Critical

#### E2E-NEW-050: An unconfigured quota never blocks the session quota path
- **Category:** Core Journey
- **Scenario:** SC-003
- **Requirements:** FR-NEW-009, FR-NEW-012
- **Driver:** MCP tool call
- **Preconditions:** `proj-quota-e2e.quota_bytes = NULL` (no project quota configured at all);
  session `bytes_written = B` before the call.
- **Steps:**
  - Given `quota_bytes = NULL`, session `bytes_written = B`
  - When `fs.write_bytes(path="/noquota.bin", content=<5000 bytes>)` is called
  - Then the call succeeds exactly as it would have before this feature existed (project quota
    check is skipped entirely per FR-NEW-009)
  - And the session's `bytes_written` is now exactly `B + 5000` (charged normally, proving
    FR-NEW-012's "before `charge_write`" ordering has no effect at all when there is no project
    quota to check)
- **Priority:** Medium

## 13. Consistency Notes

No inconsistency found with `SPEC-0002`: this increment is a pure additive extension of its `Project`
data model (new nullable field) and introduces no change to the existing session quota, which
SPEC-0002 §6 `FR-035` continues to govern unchanged.

## 14. Migration & Implementation Notes

Order matters: (1) the schema `ColumnMigration` (FR-NEW-001) must land and be exercised against all
three dialects before (2) `put_file`'s signature changes (FR-NEW-009/FR-NEW-010), because every
caller needs `quota_bytes: Option<i64>` to compile against — doing it in the other order leaves the
codebase in a non-compiling intermediate state. (3) The three new `admin.*` tools (FR-NEW-002,
FR-NEW-005, FR-NEW-007) can land in parallel with (2) since they only need `AdminBackend::get_quota/
set_quota` and `MetaBackend::usage_bytes/disk_usage`, which do not depend on `put_file`'s signature.
No feature flag: this is additive and `quota_bytes = NULL` reproduces today's behavior exactly, so
there is nothing to roll back behaviorally if deployment is paused mid-rollout (the schema migration
alone, with the enforcement code not yet deployed, is a safe intermediate state).

## 15. Open Questions & TBDs

None outstanding. All ambiguities raised during the interview (counting semantics, authorization
tiers, git-import atomicity, error message format) were resolved and logged in §17.

## 16. Glossary

| Term | Definition | Context (if multiple) |
|---|---|---|
| **declared size / logical size** | The `size` column of a `nodes` row (`schema.rs:348`): what the file claims to be, independent of blob deduplication. | — |
| **used_bytes** | `SUM(nodes.size)` for a project's `volume_id`, computed live at read time, no cached/maintained aggregate. | — |
| **quota_bytes** | The persisted cap on `used_bytes` for a project; `NULL` means unlimited. | — |
| **platform admin** | A person listed in `config.admins` (`config.rs:193`), checked via `state.is_admin`/`state.require_admin` (`state.rs:32,37`); distinct from a project owner or member. | — |
| **project owner** | The person named `owner` at project creation (`traits.rs:106`); authorized for disk-usage reads but not quota mutation in this increment. | — |
| **trash** | A soft-delete destination (`safety.rs:179-183`); an ordinary `nodes` row under `/{trash_dir}/...`, not a separate deleted flag — already counted by any `SUM(nodes.size)` query. | — |

## 17. Decisions Log

- **DEC-001:** quota-exceeded error message is the literal string `"Quota exhausted"`, with no
  interpolated numbers. **Rationale:** explicit user instruction; numbers are available separately
  via `admin.get_project_quota` for anyone investigating. **Alternatives considered:** a templated
  message embedding `cap`/`used` — rejected per explicit instruction for a short, clear literal.
  **Implemented by:** FR-NEW-009. **Round:** 3. **Code evidence:** n/a (new behavior).

- **DEC-002:** quota counts declared/logical file size (`nodes.size`), including trashed files
  (trash is already an ordinary node, no special-casing needed), not deduplicated blob bytes.
  **Rationale:** explicit user instruction, anticipating future copy-on-write where a file could be
  "edited" (logically distinct) while sharing disk bytes with another file; counting blob bytes would
  let logically-distinct files escape the cap. **Alternatives considered:** unique blob bytes
  (`blob_refs.size` sum) — rejected, would let `fs.copy` dodge the cap; blob bytes excluding trash —
  rejected, trash has no separate representation to exclude. **Implemented by:** FR-NEW-009,
  FR-NEW-011. **Round:** 1. **Code evidence:** `storage/meta.rs:139-146` (`blob_refs` schema),
  `schema.rs:348` (`nodes.size`), `safety.rs:179-183` (trash is an ordinary path).

- **DEC-003:** quota mutation is restricted to platform admin only (`require_admin`), never the
  project owner — a deliberate asymmetry from `admin.set_purge_config`'s owner-or-admin pattern.
  **Rationale:** explicit user instruction ("not by the project owner"). **Alternatives considered:**
  mirror `set_purge_config`'s owner-or-admin tier — rejected per explicit instruction.
  **Implemented by:** FR-NEW-003. **Round:** 1. **Code evidence:** `state.rs:37` (`require_admin`),
  `state.rs:47` (`require_owner_or_admin`), `mcp/server.rs:2385-2394` (`admin.set_purge_config`'s
  existing owner-or-admin pattern, the one NOT followed here).

- **DEC-004:** disk-usage read access is granted to platform admin AND project owner (not every
  project member). **Rationale:** proposed default, accepted by the user; the owner needs visibility
  to act on cleanup, ordinary members do not need billing-like data. **Alternatives considered:**
  admin-only, consistent with quota mutation itself — rejected as needlessly restrictive for the
  actor who can actually act on the information. **Implemented by:** FR-NEW-008. **Round:** 3.
  **Code evidence:** n/a (new behavior; authorization primitive reused from `state.rs:47`).

- **DEC-005:** the quota check runs inside the same database transaction as the existing `put_file`
  node lookup, reading a fresh `SUM(nodes.size)` rather than any pre-computed or cached total.
  **Rationale:** avoids a TOCTOU race where two concurrent writers each pass a stale check and
  jointly exceed the cap. **Alternatives considered:** a pre-check outside the transaction — rejected,
  reintroduces exactly the race this design rules out (see Phase 2 Approach B below).
  **Implemented by:** FR-NEW-009. **Round:** 3. **Code evidence:** `meta.rs:498-548`
  (`run_retrying`'s transaction, the existing node `SELECT` this check joins).

- **DEC-006:** `ERR_PROJECT_QUOTA_EXCEEDED` maps to HTTP `507` (Insufficient Storage), not `429`
  (used by the unrelated, already-existing `ERR_WRITE_QUOTA_EXCEEDED`, `errors.rs:190`).
  **Rationale:** `507` is the semantically correct status for a durable capacity limit, distinct from
  a transient per-session rate limit. **Alternatives considered:** reuse `429` for superficial
  consistency with the other quota error — rejected, conflates two different failure semantics for
  any monitoring/dashboard consumer distinguishing capacity exhaustion from rate limiting.
  **Implemented by:** FR-NEW-009. **Round:** 5 (derived). **Code evidence:** `errors.rs:114`
  (existing `write_quota_exceeded` → 429 mapping inside `http_status`, the pattern deliberately NOT
  followed here; corrected per DRIFT-002, which also notes `errors.rs:190` is only the matching unit
  test assertion, not the mapping).

- **DEC-007:** git bulk import/push does not pre-check its total declared size against the cap
  atomically; it fails on whichever blob first breaches the cap via the same per-call `put_file`
  check, leaving earlier blobs in that import already committed. Only the already-atomic
  merge-apply path (`git/merge.rs:705`) keeps its existing all-or-nothing guarantee, which gets the
  same protection for free (the first blob to breach the cap aborts that merge's whole apply, via its
  existing rollback/refund logic). **Rationale:** avoids a second, separate "would the whole import
  exceed quota" prediction pass; accepted as a pragmatic default. **Alternatives considered:** a
  pre-flight sum-of-incoming-sizes check before starting any import — deferred as YAGNI, revisitable
  if partial-import-then-reject proves confusing in practice. **Implemented by:** FR-NEW-009 (applies
  uniformly, no special-casing for git). **Round:** 3. **Code evidence:** `tools/git.rs:3121` (bulk
  import charges `total_bytes` up front today, for the unrelated session quota, not atomically
  against the project cap), `git/merge.rs:705` (existing all-or-nothing apply, unmodified by this
  decision).

- **DEC-008:** the new project storage quota and the existing per-session `write_quota_bytes` rate
  limiter remain fully independent; neither replaces nor subsumes the other. **Rationale:** explicit
  non-goal (§3.2); they answer different questions (durable per-tenant capacity vs. transient
  per-session throughput). **Alternatives considered:** folding the project cap into the existing
  `SafetyManager`/session mechanism — rejected, conflates a persisted, admin-controlled, per-project
  value with an in-memory, non-persisted, per-session one. **Implemented by:** FR-NEW-012 (the one
  place the two interact: order of checks on rejection). **Round:** 1. **Code evidence:**
  `safety.rs:25-30,81,131-154` (the session mechanism, left untouched).

- **DEC-009 (Phase 2 approach choice):** extend `MetaBackend::put_file`'s signature with
  `quota_bytes: Option<i64>`, fetched once per call by the caller via
  `AdminBackend::get_quota(project_id)`, with the `SUM`/comparison executed inside `put_file`'s
  existing transaction. **Rationale:** the only approach that gives both a single enforcement point
  and transactional race-safety (DEC-005) without introducing a second, drift-prone maintained
  counter. **Alternatives considered:** (Approach B) a pre-check outside the transaction — rejected,
  reintroduces the TOCTOU race. (Approach C) a maintained `used_bytes` counter incremented/decremented
  alongside `tx_incref`/`tx_decref` (`meta.rs:276-320`) — rejected, that counter tracks deduplicated
  blob bytes (incremented only on first incref to a new sha, decremented only at refcount zero),
  exactly the blob-level accounting DEC-002 rejects; reworking it to track logical bytes would
  duplicate what a live `SUM(nodes.size)` already gives for free, with a second code path that can
  drift from the real node sizes. Best suited for Approach C: a future scale where a live `SUM` scan
  becomes too slow — not this project's scale today. **Implemented by:** FR-NEW-009, FR-NEW-010.
  **Round:** Phase 2. **Code evidence:** `meta.rs:276-320` (`tx_incref`/`tx_decref`, the mechanism
  Approach C would have piggybacked on and that this decision leaves untouched).

## 18. Implementability Gate

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 2 | 2 | NOT-IMPLEMENTABLE → amended → **IMPLEMENTABLE-WITH-DRIFT** (final) |

**Amendments applied:** FR-NEW-010 rewritten (the real write choke point is
`VolumeClient::write_bytes_atomic`/`copy_file`/`copy_tree` at `storage/volume.rs:147,196,218`, not
the ten `fs_ops.rs`/`doc.rs`/`git.rs`/`merge.rs` sites originally named — those all route through one
of these three methods already); new FR-NEW-014 added (authorization-before-existence check order
for `admin.set_project_quota`/`admin.get_project_quota`), with new tests E2E-NEW-051/052 and
retagged contrast tests E2E-NEW-003/007/037. Both were Nature F (two implementing agents would have
wired the quota lookup differently, and ordered the two checks differently, each producing an
observably different result for a public contract) and are resolved in place, not deferred.
**Drift registered:** DRIFT-001, DRIFT-002 (§19) — both Nature A, both amended in place where
evident (DRIFT-002's citation) or left as an implementation-time correction with a required test
(DRIFT-001), per §6.3 step 1.

## 19. Implementation Drift Register

#### DRIFT-001: `put_file`'s existing node lookup does not yet select `size`
- **Spec says:** FR-NEW-009 computes `new_total` "inside the same transaction as the existing node
  lookup (`meta.rs:498-515`)", presupposing that lookup already has `old_size_at_path` available.
- **Code does:** the existing `SELECT` in `put_file` (`storage/meta.rs:505-512`) fetches only
  `kind, ctime, sha256` — it does not select `size`.
- **Nature:** missing capability.
- **Resolution during implementation:** extend the existing `SELECT` to
  `SELECT kind, ctime, sha256, size FROM nodes WHERE volume_id=?1 AND path=?2` and read `size` into
  `old_size_at_path` before computing `new_total`.
- **Detected by:** no compiler error (the formula is specified in prose, not yet in code); the
  boundary tests E2E-NEW-032/033 (exact-cap allow/reject) will fail if `old_size_at_path` is wrong,
  which makes this unmissable at implementation time.
- **Blocks which requirement:** FR-NEW-009.
- **Status:** open.

#### DRIFT-002: DEC-006 cites the wrong line for the existing HTTP status mapping
- **Spec says:** DEC-006 cites "`errors.rs:190` (existing `write_quota_exceeded` → 429 mapping)".
- **Code does:** the real mapping arm is at `errors.rs:114` (`code::WRITE_QUOTA_EXCEEDED => 429`
  inside `http_status`); `errors.rs:190` is a unit-test assertion of the same value
  (`assert_eq!(ToolError::write_quota_exceeded("x").http_status(), 429)`), not the mapping itself.
- **Nature:** false statement (citation only; the value 429 itself is correct).
- **Resolution during implementation:** add the new `code::PROJECT_QUOTA_EXCEEDED => 507` arm in the
  `http_status` match at `errors.rs:114`, not near line 190.
- **Detected by:** code review at implementation time; no test fails either way since 507 is a new,
  distinct code path with its own E2E-NEW-015/024/027/042 assertions.
- **Blocks which requirement:** none, informational — corrected here rather than left open, since
  the fix is evident from the citation itself.
- **Status:** closed (corrected in place, see amended citation below).
