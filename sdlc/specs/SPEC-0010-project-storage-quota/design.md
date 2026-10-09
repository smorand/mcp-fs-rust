> Id: SPEC-0010
> Nature: FEAT
> Status: implemented
> Migrated from: specs/SPEC-0010_2026-10-05_14-49-04-project-storage-quota/spec.md on 2026-10-09
> From backlog: BL-0002 (legacy numbering, do not try to map to new backlog ids)

# Per-Project Storage Quota — Design

## 1. Components

| Component | Responsibility | Status |
|---|---|---|
| `storage::traits::Project.quota_bytes` | Holds the nullable byte cap on the in-memory project model | Implemented |
| `storage::rel::schema` column migration for `project.quota_bytes` | Adds the nullable `BigInt` column across every relational dialect | Implemented |
| `storage::traits::AdminBackend::set_quota` / `get_quota` | Trait methods to persist/read the cap | Implemented (`set_quota` wired to a tool; `get_quota` exists on the trait and is exercised only by tests, not by any production reader) |
| `mcp::server::McpServer::admin_set_project_quota` (`admin.set_project_quota` tool) | MCP entry point: authorization, validation, persistence | Implemented |
| `admin.get_project_quota` tool | Originally scoped read-back of `{max_mb, used_bytes}` | **Not implemented** |
| `admin.get_project_disk_usage` tool | Originally scoped size-sorted breakdown | **Not implemented** |
| `storage::traits::MetaBackend::usage_bytes` / `disk_usage` | Originally scoped live aggregate queries | **Not implemented** |
| `storage::traits::MetaBackend::put_file` quota parameter + in-transaction check | Originally scoped enforcement point | **Not implemented** (signature unchanged from pre-increment except for the unrelated `old_size` addition) |
| `storage::volume::VolumeClient` admin-backed quota lookup | Originally scoped wiring from the write choke point to `AdminBackend::get_quota` | **Not implemented** (`VolumeClient` has no `admin` field) |
| `errors::codes::PROJECT_QUOTA_EXCEEDED` | Originally scoped new error code, HTTP 507 mapping | **Not implemented** (zero occurrences anywhere in `crates/core/src`) |

## 2. Flows

### 2.1 Delivered flow: set/clear a project's storage limit
1. Caller invokes `admin.set_project_quota(project_id, max_mb)`.
2. `self.state.require_admin(&self.person)?` — platform-admin check, first (`mcp/server.rs:2609`).
3. `max_mb == Some(0)` → `ToolError::invalid_argument("max_mb must be > 0")`, no further action
   (`mcp/server.rs:2610-2612`).
4. `quota_bytes = max_mb.map(|m| i64::from(m) * 1_048_576)` (`mcp/server.rs:2613`).
5. `self.state.admin.set_quota(&a.project_id, quota_bytes)` → `UPDATE project SET
   quota_bytes=?1 WHERE id=?2`; zero affected rows → `ToolError::project_not_found`
   (`storage/admin.rs:490-501`).
6. Response echoes `{project_id, max_mb}` (`mcp/server.rs:2616-2620`).

There is no step 7: nothing downstream of this flow ever reads `quota_bytes` again in production
code.

### 2.2 Originally scoped, undelivered flow: write-time enforcement
The original design (DEC-005, DEC-009) called for `VolumeClient::write_bytes_atomic` / `copy_file`
/ `copy_tree` to fetch `AdminBackend::get_quota(project_id)` and pass it into `MetaBackend::put_file`,
which would compute `new_total = usage_in_tx - old_size_at_path + size` inside the same
transaction as the existing node lookup, rejecting with `ERR_PROJECT_QUOTA_EXCEEDED` when
`new_total > cap`. None of this exists. The one fragment of groundwork that did land is unrelated
on its own: `put_file`'s existing-node `SELECT` now also selects `size` into `old_size`
(`storage/meta.rs:910-917`, `934-941`, surfaced on `PutFileResult.old_size`), which is exactly what
DRIFT-001 in the original spec called for — but it is dead groundwork today, since nothing reads
`PutFileResult.old_size` to compare against a cap. It is covered only by a generic unit test
(`put_file_reports_old_size_of_overwritten_path`, `storage/meta.rs:1463`) that asserts the field is
populated correctly, not that it feeds any quota decision.

## 3. Interfaces

### Delivered

**MCP tool `admin.set_project_quota`**
- Input: `project_id: String`, `max_mb: Option<u32>`.
- Output: `{"project_id": String, "max_mb": Option<u32>}`.
- Errors: `ERR_FORBIDDEN` (non-admin), `ERR_INVALID_ARGUMENT` (`max_mb = Some(0)`),
  `ERR_PROJECT_NOT_FOUND` (project does not exist, checked after authorization).

### Not delivered
- `admin.get_project_quota` (no such tool).
- `admin.get_project_disk_usage` (no such tool).
- No REST surface was ever expected for `admin.*` (consistent with the rest of the `admin.*`
  family, which has no REST equivalent), so this is not itself a gap.

## 4. Data and state

| Table/column | Shape | Status |
|---|---|---|
| `project.quota_bytes` | nullable `BigInt`, `NULL` = unlimited | Implemented, via `ColumnMigration` (`storage/admin.rs:104`) |
| Derived `used_bytes` (`SUM(nodes.size)` per `volume_id`) | not stored, meant to be computed live | Never computed anywhere; no query exists |

No other schema element from the original design (no new column, no new table) was added or is
needed for the delivered scope.

## 5. Configuration

No new configuration keys. No feature flag: the field is additive, defaults to `NULL`, and (as
shipped) has no behavioral effect to gate.

## 6. Observability

Nothing to observe: no rejection path exists, so no tracing was added beyond whatever generic
tool-call logging already covers `admin.set_project_quota` like any other tool.

## 7. Decisions

Legacy Decisions Log entries (`DEC-001` through `DEC-009` in the pre-move spec) are renumbered
below. Entries describing the undelivered enforcement mechanism are kept for traceability since
they document intent that may still be picked up later (see Findings).

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | Quota mutation is restricted to platform admin only; the project owner has no authority over it, unlike the owner-or-admin pattern used for purge configuration. | `mcp/server.rs:2609` (`require_admin`, not `require_owner_or_admin`) | Deliberate asymmetry, per explicit product instruction in the original interview. |
| DEC-002 | Authorization is checked before project-existence for both set and (originally) read, so a non-admin cannot distinguish "forbidden" from "not found" by probing project ids. | `mcp/server.rs:2609` runs before `storage/admin.rs:496-499`'s existence check; test `e2e_new_051...` at `mcp/server.rs:4546` | Prevents enumeration of valid project ids by an unauthorized caller. |
| DEC-003 | A zero megabyte count is invalid; `None`/absent clears the quota to unlimited. | `mcp/server.rs:2610-2613` | `0` is a meaningless cap (nothing could ever be written); unlimited needs its own distinct representation. |
| DEC-004 (undelivered) | The originally planned error for exceeding a limit was the literal string `"Quota exhausted"`, mapped to HTTP 507, deliberately distinct from the existing, unrelated 429 session-rate-limit error. | No code; originally specified only. | Kept for traceability — if enforcement is built later, this decision should still hold unless explicitly revisited. |
| DEC-005 (undelivered) | The originally planned enforcement point was `MetaBackend::put_file`, checked in the same transaction as the existing node lookup, to avoid a TOCTOU race between concurrent writers. | `storage/meta.rs:906-975` shows no such check exists today. | Kept for traceability; the race-safety argument (checking inside the existing write transaction) remains valid design guidance if this is picked up. |
| DEC-006 (undelivered) | Quota was meant to count declared/logical file size, including trashed files, not deduplicated blob bytes, so `fs.copy` could not dodge the cap via deduplication. | No code; originally specified only. | Kept for traceability. |

## 8. Requirement to code map

| FR | Code |
|---|---|
| FR-NEW-001 | `storage/traits.rs:148`, `storage/admin.rs:104`, `storage/admin.rs:190` |
| FR-NEW-002 | `mcp/server.rs:2604-2622`, `storage/admin.rs:490-501` |
| FR-NEW-003 | `mcp/server.rs:2609` |
| FR-NEW-004 | `mcp/server.rs:2610-2612` |
| FR-NEW-005 through FR-NEW-012 | none — not implemented |
| FR-NEW-013 | `storage/admin.rs:490-501` (true only because the function touches no `nodes` rows; not a deliberate no-eviction guard against real enforcement) |
| FR-NEW-014 | `mcp/server.rs:2609-2615`, `storage/admin.rs:496-499` |

## 9. Legacy mapping

Source: specs/SPEC-0010_2026-10-05_14-49-04-project-storage-quota/spec.md (pre-move)

| Old id | New id | Note |
|---|---|---|
| FR-NEW-001 | FR-NEW-001 | kept verbatim, implemented |
| FR-NEW-002 | FR-NEW-002 | kept verbatim, implemented |
| FR-NEW-003 | FR-NEW-003 | kept verbatim, implemented |
| FR-NEW-004 | FR-NEW-004 | kept verbatim, implemented |
| FR-NEW-005 | FR-NEW-005 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-006 | FR-NEW-006 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-007 | FR-NEW-007 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-008 | FR-NEW-008 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-009 | FR-NEW-009 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-010 | FR-NEW-010 | kept verbatim, not implemented — moved to Findings |
| FR-NEW-011 | FR-NEW-011 | kept verbatim, not implemented, dependent on FR-NEW-009/010 |
| FR-NEW-012 | FR-NEW-012 | kept verbatim, not implemented, dependent on FR-NEW-009/010 |
| FR-NEW-013 | FR-NEW-013 | kept verbatim, trivially true today, not a deliberate guard |
| FR-NEW-014 | FR-NEW-014 | kept verbatim, implemented |
| DEC-001..DEC-009 | DEC-001..DEC-006 | consolidated; DRIFT-002 (wrong citation for the 429/507 mapping) is moot, since the 507 mapping was never added at all |
| DRIFT-001 (put_file lookup lacks `size`) | — | the cited gap was closed (`put_file` now selects and returns `old_size`), but the quota check that was meant to consume it was never added — see Findings |
| DRIFT-002 (DEC-006 cites wrong line for 429/507 mapping) | — | moot: no 507 mapping exists at all |

### FINDINGS FOR BACKLOG

1. **Enforcement was never built.** The original spec's core value proposition — a write that
   would exceed a project's declared storage limit gets rejected — does not exist in the
   codebase. `storage::traits::MetaBackend::put_file` has no `quota_bytes` parameter, performs no
   cap comparison, and `storage::volume::VolumeClient` has no reference to `AdminBackend` or any
   quota lookup at all. An admin can set an arbitrarily low limit on a project and every write to
   that project will continue to succeed without bound. Recommend a new backlog item to implement
   FR-NEW-009 through FR-NEW-012 (and FR-NEW-011's dedup-bypass guarantee) as originally
   designed, including the `ERR_PROJECT_QUOTA_EXCEEDED` → HTTP 507 error code.

2. **No way to read back what was set, or see usage.** `admin.get_project_quota` was never built
   (FR-NEW-005/006), so an admin who sets a quota has no API to confirm it later or see how close
   a project is to its limit. Recommend a backlog item for this read path; it has no dependency on
   finding 1 and can land independently.

3. **No disk-usage breakdown.** `admin.get_project_disk_usage` (FR-NEW-007/008) was never built.
   Recommend a backlog item; independent of findings 1 and 2.

4. **`AdminBackend::get_quota` exists but is production-dead.** The trait method is implemented
   and correct (`storage/admin.rs:505-510`) and is exercised by tests, but no production code path
   calls it — it was clearly built as a stepping stone toward finding 1 or 2 and then left
   unconnected. Worth noting in whichever backlog item picks up finding 1 or 2, so the
   implementer knows this piece is already done and tested.

5. **Partial feature shipped without a corresponding note in `AGENTS.md`'s "Behaviour worth
   knowing".** Given only a quarter of the originally scoped surface shipped, and it currently has
   zero observable effect on any other operation, this is arguably correct as-is (there is no
   user-facing behavior yet worth noting) — but once finding 1 lands, `AGENTS.md` should gain the
   line the original spec's §10 called for.
