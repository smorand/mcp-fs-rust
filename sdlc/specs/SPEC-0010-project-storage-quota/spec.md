> Id: SPEC-0010
> Nature: FEAT
> Status: implemented
> Migrated from: specs/SPEC-0010_2026-10-05_14-49-04-project-storage-quota/spec.md on 2026-10-09
> From backlog: BL-0002 (legacy numbering, do not try to map to new backlog ids)

# Per-Project Storage Quota

## 1. Summary

A platform admin can set, clear, or correct a maximum storage size (in megabytes) on a project
by calling a dedicated tool. The value is persisted as a nullable byte count, where no value
means unlimited. As shipped, this tool only records the admin's intent: it validates the input,
enforces that only a platform admin can call it, and persists the value. It does not yet enforce
the cap against writes, nor expose any way to read back the quota or a per-project usage
breakdown — those parts of the originally specified feature were not built.

## 2. Current State

### 2.1 How it works today — functional

- A project record carries an optional maximum storage size, expressed internally as a byte
  count, defaulting to "no limit" for both new and pre-existing projects.
- A platform admin can set this value, expressed in megabytes, by naming a project and a
  megabyte count. Omitting the count clears the limit back to unlimited.
- Only a platform admin may change this value; the project's owner has no authority over it,
  unlike other per-project settings that allow an owner to self-serve.
- Setting a megabyte count of zero is rejected with a literal validation message.
- Naming a project that does not exist is rejected, but only after the caller has already been
  confirmed to be a platform admin — a non-admin naming a nonexistent project is told they are
  forbidden, not that the project is missing, so a non-admin cannot use this call to probe which
  project ids exist.
- Lowering the limit below what a project currently stores is accepted outright: nothing is
  deleted or modified as a result, because nothing currently reads this value back to compare it
  against anything.
- Nothing in the system currently reads the stored limit, computes how much a project is using,
  rejects a write for exceeding it, or lists a project's files by size. The value, once set, has
  no observable effect on any other operation.

### 2.2 Existing specs governing this area

The platform's project record and its per-session (not per-project) write-rate limiter are
governed by the platform foundation specification. This increment only adds one new field to
the project record; it does not change the per-session write-rate limiter, and the two remain
unrelated: one is a transient, in-memory, per-session budget, the other is a persisted,
admin-set, per-project value that (as shipped) nothing yet checks.

### 2.3 Existing test coverage

| Area | Confidence |
|---|---|
| Setting a valid quota, by an admin | Direct test, passing |
| Clearing a previously set quota | Direct test, passing |
| Rejecting a megabyte count of zero | Direct test, passing |
| Rejecting a non-admin caller | Direct test, passing |
| Authorization checked before project-existence | Direct test, passing |
| Setting a quota on a nonexistent project | Direct test, passing |
| Lowering below current usage evicts nothing | Direct test, passing (trivially: nothing reads usage) |
| Accepting the smallest valid non-zero value | Direct test, passing |
| Reading back a quota together with live usage | No test — capability does not exist |
| Any write being rejected for exceeding a quota | No test — capability does not exist |
| A per-project disk-usage breakdown, sorted by size | No test — capability does not exist |

## 3. Scope

### 3.1 Delivered
- A nullable maximum storage size on each project, defaulting to unlimited.
- One administrative action to set or clear that value, restricted to a platform admin, with
  input validation and an authorization-before-existence check ordering.

### 3.2 Not delivered (originally scoped, not built)
- Enforcement of the stored limit against any write operation (`write`, `append`, `copy`,
  document generation, git pushes/imports/merges, or the equivalent REST upload path).
- Any way to read the stored limit back, or to see how much a project is currently using.
- The per-project, size-sorted disk-usage breakdown.
- The dedicated rejection behavior and message that was meant to accompany enforcement.

### 3.3 Non-Goals (as originally scoped, still true)
- No aggregate, platform-wide quota across every project.
- No alerting or notification when a project nears its limit.
- No change to the existing, unrelated per-session write-rate limiter.
- No automatic eviction or cleanup when an admin lowers a project's limit below current usage.

## 4. Actors

| Actor | Role |
|---|---|
| **Platform admin** | Sole authority to set or clear a project's storage limit. |
| **Project owner** | Has no authority over the storage limit in this increment. |
| **Any other caller** | Unaffected; the stored value has no effect on any operation today. |

## 5. Usage Scenarios

### SC-001: Admin sets a project's storage limit
**Actor:** Platform admin.
**Flow:** Admin names a project and a megabyte count. The system checks the caller is a platform
admin, validates the count is absent or at least 1, persists the value, and confirms back what
was stored.
**Exceptions:**
- Caller is not a platform admin → rejected, nothing stored.
- Megabyte count is exactly zero → rejected with a literal validation message, nothing stored.
- Named project does not exist → rejected, but only evaluated after the admin check passes.

### SC-002 (not delivered): Admin reads a project's limit and current usage
Originally scoped; no code path exists to serve this today.

### SC-003 (not delivered, moot): A write stays under the limit
Originally scoped to describe unchanged behavior under quota; moot since nothing checks the
limit against any write.

### SC-004 (not delivered): A write would exceed the limit
Originally scoped; no rejection of this kind occurs today regardless of the stored limit's
value.

### SC-005 (not delivered): Admin or owner reads a size-sorted usage breakdown
Originally scoped; no code path exists to serve this today.

## 6. Functional Requirements

Legacy `FR-NEW-xxx` ids are kept verbatim; renumbering would break commit references. Each
requirement is assessed against the current code.

| ID | Requirement | Status | Evidence | Confidence notes |
|---|---|---|---|---|
| FR-NEW-001 | Project record persists an optional storage limit, defaulting to unlimited for new and pre-existing projects. | Implemented | `crates/core/src/storage/traits.rs:148` (`Project.quota_bytes: Option<i64>`); `crates/core/src/storage/admin.rs:104` (column migration); `crates/core/src/storage/admin.rs:190` (new project defaults to `None`) | High confidence, verified directly. |
| FR-NEW-002 | Admin sets the limit; absent/`None` clears it; value is stored as megabytes × 1,048,576. | Implemented | `crates/core/src/mcp/server.rs:2604-2622` (tool `admin.set_project_quota`); `crates/core/src/storage/admin.rs:490-501` (`set_quota`) | High confidence. |
| FR-NEW-003 | Non-admin caller is rejected, no mutation occurs. | Implemented | `crates/core/src/mcp/server.rs:2609` (`self.state.require_admin(&self.person)?` runs first); test `e2e_new_003_non_admin_cannot_set_the_quota` at `crates/core/src/mcp/server.rs:4405` | High confidence. |
| FR-NEW-004 | A zero megabyte count is rejected with the literal message `"max_mb must be > 0"`. | Implemented | `crates/core/src/mcp/server.rs:2610-2612` | High confidence; message matches spec literally. |
| FR-NEW-005 | Admin can read back `{project_id, max_mb, used_bytes}` with `used_bytes` computed live. | Not implemented | No `admin.get_project_quota` tool and no `usage_bytes`/equivalent trait method exist anywhere in `crates/core/src` (verified: no matches for `get_project_quota` or `usage_bytes`). | Capability absent, not a stale citation — confirmed by repo-wide search. |
| FR-NEW-006 | Non-admin caller of the read is rejected. | Not implemented | No such tool exists to reject a call against. | N/A, moot. |
| FR-NEW-007 | Admin or owner can read a size-sorted disk-usage breakdown, capped and truncation-flagged. | Not implemented | No `admin.get_project_disk_usage` tool and no `disk_usage` trait method exist anywhere in `crates/core/src`. | Capability absent, confirmed by repo-wide search. |
| FR-NEW-008 | Non-owner, non-admin caller of the breakdown is rejected. | Not implemented | No such tool exists. | N/A, moot. |
| FR-NEW-009 | Writes are checked, in-transaction, against the stored limit and rejected with a dedicated error when they would exceed it. | Not implemented | `crates/core/src/storage/meta.rs:906-975` (`put_file`): the signature has no `quota_bytes` parameter and performs no comparison against any cap; repo-wide search for `PROJECT_QUOTA_EXCEEDED` and `"Quota exhausted"` returns zero matches anywhere in `crates/core/src`. | Confirmed absent, not a drifted citation: the enforcement mechanism was never added. |
| FR-NEW-010 | The write choke point (`write_bytes_atomic`/`copy_file`/`copy_tree`) fetches the project's limit and threads it into the write. | Not implemented | `crates/core/src/storage/volume.rs` has no `admin` field and no reference to any quota lookup (verified: zero case-insensitive matches for `admin`/`quota` in that file). | Confirmed absent. |
| FR-NEW-011 | Deduplicated blobs still count at full declared size for the limit. | Not implemented | No enforcement path exists to apply this rule to. | N/A, moot — depends on FR-NEW-009/010. |
| FR-NEW-012 | A rejected write must not consume the unrelated per-session write-rate budget, and the limit check must run before that budget is charged. | Not implemented | No rejection of this kind exists; the ordering guarantee has nothing to order. | N/A, moot. |
| FR-NEW-013 | Lowering the limit below current usage is accepted and evicts nothing; only subsequent over-limit writes are rejected. | Partially true, by construction | `crates/core/src/storage/admin.rs:490-501` (`set_quota` only updates the column, touches no `nodes` rows); test `e2e_new_029_lowering_below_usage_does_not_evict` at `crates/core/src/mcp/server.rs:4458` | True today only because nothing evicts anything at all, not because a deliberate no-eviction rule was implemented against real enforcement. |
| FR-NEW-014 | Authorization is checked before project-existence, so a non-admin naming a nonexistent project gets "forbidden", not "not found". | Implemented | `crates/core/src/mcp/server.rs:2609-2615` (admin check at line 2609, before `set_quota`'s existence check at `storage/admin.rs:496-499`); test `e2e_new_051_non_admin_nonexistent_project_is_forbidden_not_not_found` at `crates/core/src/mcp/server.rs:4546` | High confidence, verified ordering in code and by test. |

## 7. Non-Functional Requirements

### 7.1 Performance
Not applicable beyond the single `UPDATE project SET quota_bytes=...` statement the set path
performs; no read-time aggregate query exists in the shipped scope.

### 7.2 Security
Reuses the existing platform-admin authorization check already used elsewhere in the system; no
new authentication mechanism. The authorization-before-existence ordering (FR-NEW-014) prevents a
non-admin from using this call to enumerate which project ids exist.

### 7.3 Usability
As shipped, an admin has no way to confirm what value they stored except by trusting the tool's
own echoed response at call time; there is no read-back capability (FR-NEW-005 not implemented).

### 7.4 Reliability
The set operation is a single statement; its own atomicity is unremarkable. No cross-operation
atomicity guarantee is relevant since no write-time check exists to be atomic with.

### 7.5 Observability
No new tracing was found or expected, since no rejection path (the originally-planned
`ERR_PROJECT_QUOTA_EXCEEDED`) exists to trace.

### 7.6 Deployment
The schema change is additive and applies uniformly across every supported relational backend
via the existing migration mechanism.

### 7.7 Scalability
Not applicable to the delivered scope.

## 8. Glossary

| Term | Definition |
|---|---|
| **storage limit / quota_bytes** | The persisted, nullable byte cap stored on a project; `None`/absent means unlimited. As shipped, nothing reads or enforces this value. |
| **platform admin** | The only actor authorized to set or clear a project's storage limit. |
| **project owner** | The person who owns a project; has no authority over the storage limit in this increment. |
| **used_bytes / disk-usage breakdown** | Concepts from the original scope that were never implemented; no code computes either. |

## 9. Confidence notes

- All "Implemented" rows above were verified directly against current source (file:line cited),
  not inferred from the old draft spec or its Implementability Gate verdict.
- All "Not implemented" rows were verified by repository-wide search for the exact identifiers
  the original spec specified (`get_project_quota`, `get_project_disk_usage`, `usage_bytes`,
  `disk_usage`, `PROJECT_QUOTA_EXCEEDED`, `"Quota exhausted"`), all returning zero matches in
  `crates/core/src`. This is a genuine capability gap, not a line-number drift in an otherwise
  correct citation.
- The original spec's Status was `Draft` with an Implementability Gate verdict of
  `IMPLEMENTABLE-WITH-DRIFT`; the gate's own Decisions Log and Drift Register describe the full
  originally-planned scope (FR-NEW-001 through FR-NEW-014) as a coherent, resolvable design. Only
  a subset of that design (FR-NEW-001 through 004, and 014) reached the codebase. The remainder
  (FR-NEW-005 through 012, and the enforcement-dependent half of FR-NEW-013) stayed unbuilt. This
  is the central finding of this migration; see the companion design document's "Findings for
  backlog" section.
- Test ids from the original spec's E2E suite that exist in the current test code:
  `E2E-NEW-001, 002, 003, 004, 029, 037, 049, 051` (all under `admin.set_project_quota`, `SC-001`
  only; grep-verified at `crates/core/src/mcp/server.rs:4358-4567`). None of `E2E-NEW-005` through
  `028`, `030` through `036`, `038` through `048`, `050`, or `052` exist in the current test
  suite.
