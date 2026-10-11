# US-0017: Conversion Markdown under the limit

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0016
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Documented uploads and conversions write a Markdown file; this story checks it against the project limit, keeps the upload when only the Markdown is refused, and reports the refusal as the documentation outcome.

## Requirements covered
- SPEC-0020/FR-NEW-014: IF the Markdown file of a conversion requested with an upload would exceed the limit THEN THE Storage context SHALL keep the uploaded file and report the refusal of FR-NEW-001 as the documentation outcome.

## Independent Test
`make check`; then upload a 30 000 byte document whose conversion yields 20 000 bytes with 1 000 000 used: the source exists, no Markdown file exists, usage is 1 030 000.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): Per volume `volume_usage` counter maintained in the node transactions, checked under a per volume in process admission lock held across the operation's writes. **Obliged by:** SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B), conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`, `:1693`, `:1728`.
- DEC-008 (SPEC-0020): Conversion sibling: project delta counts the sibling only; artifacts and conversion text stay out (session charge unchanged). **Obliged by:** FR-NEW-002, FR-NEW-014. **Source:** spec. **Evidence:** SPEC-0019 `land_conversion`. **Rationale:** artifacts are not project files. **Alternatives considered:** counting artifacts (contradicts FR-NEW-002).
### Data model
**`volume_usage`** (new, meta schema, PK `volume_id`): `volume_id TextKey`, `used_bytes BigInt`.

- `put_file` adds `new_size - old_size` (it already reads the old size, `meta.rs:1615`); a new path
  adds `new_size`. `delete_file` subtracts the node size; `remove_subtree` subtracts the sum of the
  removed files; `rename` changes nothing (trash is a rename, so trashed files stay counted). All in
  the existing transactions, `volume_id` in every `WHERE`.
- Backfill: when the row is missing, `INSERT ... SELECT SUM(size) FROM nodes WHERE volume_id=? AND
  mode=file` under the admission lock, once.
- Migration additive; rollback: drop the table (it is derived data).

No change to the project table: `quota_bytes` exists (`storage/traits.rs:148`).
### Contracts
### 5.1 Refusal

New code `ERR_PROJECT_QUOTA_EXCEEDED` in `errors.rs` (16 codes), HTTP 507, constructor
`ToolError::project_quota_exceeded(limit_mb, used, requested)` producing exactly
`project storage quota of <N> MB exceeded: <U> bytes used, <W> bytes requested` (N = `quota_bytes /
1 048 576`, a whole number since it is set in MB).

### 5.2 Admission

```rust
// crates/core/src/quota.rs
pub(crate) struct Admission { _guard: tokio::sync::OwnedMutexGuard<()> }
pub(crate) async fn admit(state: &AppState, mount_id: &str, person: &str,
                          session_bytes: i64, delta: i64) -> Result<Admission>;
```

`delta` is the net change of FR-NEW-003 (new minus previous size per written path, minus the size of
each path a version control operation removes). `delta <= 0` always passes the project check
(FR-NEW-004). The session budget keeps today's amount (`session_bytes`, unchanged per path).

### 5.3 Read

MCP tool `admin.get_project_quota` (params `project_id`; read only, idempotent, not open world) returns
`{"project_id": "<id>", "max_mb": <int|null>, "used_bytes": <int>}`. REST:
`GET /api/admin/projects/{project_id}/quota`, same function, same body. Gate: an admin gets
`project '<id>' not found` for a missing project; anyone else gets
`'<person>' is not the owner of '<id>'` (`ERR_FORBIDDEN`) when the project is missing or not theirs.
Tool count 107 to 108.

### 5.4 Entry points switched to `admit`

| Entry | Today | delta | session_bytes |
|---|---|---|---|
| write, write_bytes, upload, append, edit kinds, write_docx (`fs_ops.rs:575`, `:611`, `:841`, `:905`, `:934`, `:1295`, `:1749`) | `charge_write(len)` | new size minus previous size | unchanged |
| copy file / folder (`fs_ops.rs` `copy_path`, `volume.rs:209`, `:222`) | file copy charged, folder copy not | size of every copied file minus previous size of overwritten targets | unchanged (0 for folder copy) |
| archive extraction (`tools/archive.rs:183-187`) | one charge of declared total | sum of entry sizes minus previous sizes | unchanged |
| conversion sibling (`core/fs_ops.rs` `land_conversion`) | one charge of sibling + set + text | sibling bytes minus previous sibling size (artifacts excluded, FR-NEW-002) | unchanged |
| git apply (`git/merge.rs:691` `charge_quota`, callers listed in spec 14.3) | sum of written blobs | written blobs minus previous sizes minus removed paths | unchanged |
| `git.checkout_file` (`tools/git.rs:1403`), clone (`tools/git.rs:3121`) | direct charge | as above | unchanged |
| aborts (`tools/git.rs:1600`, `:1643`, `:1685`, `:1727`) | none | not checked (FR-NEW-012) | unchanged |
### Paths
| Path | Action |
|---|---|
| `crates/core/src/core/fs_ops.rs` | modify: `land_conversion` and documented write call `admit` with the sibling delta |
| `crates/core/src/quota.rs` | modify if needed |
| `crates/core/src/tools/quota_e2e.rs` | create or extend: the story's e2e tests |
### Existing patterns
- Tool method and admin gate: `admin_set_project_quota` (`crates/core/src/mcp/server.rs:2774-2796`), `require_owner` (`crates/core/src/storage/admin.rs:403-414`).
- Session charge: `SafetyManager::charge_write` (`crates/core/src/safety.rs:130-136`).
- Node transaction with old size: `put_file` (`crates/core/src/storage/meta.rs:1615`); meta table pattern `trash_entries` (`meta.rs` schema).
- Version control apply: `merge::charge_and_apply` (`crates/core/src/git/merge.rs:669-707`).
- Tool e2e harness: `crates/core/src/tools/artifacts.rs` `mod e2e` (SPEC-0019).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

Shared setup unless stated: project `Atlas` has a limit of 1 MB (1 048 576 bytes) and uses
1 000 000 bytes (48 576 bytes left). Mia (`mia@example.com`) and Olivia (`olivia@example.com`,
owner) are members with no session budget used; Ada (`ada@example.com`) is a platform admin; Otto
(`otto@example.com`) belongs to no project. Each member's session budget is 262 144 bytes.

### SPEC-0020/E2E-018
- Category: failure; scenario: SC-001; requirements: FR-NEW-014, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given document conversion is on and `/d.pdf` of 30 000 bytes converts to a 20 000 byte Markdown file. When Mia uploads `/d.pdf` requesting conversion. Then the upload is accepted and its documentation outcome reads `project storage quota of 1 MB exceeded: 1030000 bytes used, 20000 bytes requested`. And no Markdown file exists, and usage is 1 030 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-019
- Category: side effect; scenario: SC-001; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given `/e.pdf` of 10 000 bytes converts to a 5 000 byte Markdown file with 4 images and 2 tables totalling 120 000 bytes. When Mia uploads it requesting conversion. Then both are accepted. And usage is 1 015 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-058
- Category: happy; scenario: SC-001; requirements: FR-NEW-014
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given document conversion is on and `/f.pdf` of 10 000 bytes converts to a 5 000 byte Markdown file. When Mia uploads `/f.pdf` requesting conversion. Then the upload and its Markdown file are accepted. And usage is 1 015 000 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-059
- Category: failure; scenario: SC-001; requirements: FR-NEW-014, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given document conversion is on. When Mia uploads `/g.pdf` of 50 000 bytes requesting conversion. Then the upload is refused with `project storage quota of 1 MB exceeded: 1000000 bytes used, 50000 bytes requested`. And neither `/g.pdf` nor its Markdown file exists.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0020/E2E-060
- Category: edge; scenario: SC-001; requirements: FR-NEW-014, FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (REST harness for the read route parity) / `crates/core/src/tools/quota_e2e.rs`
- Given document conversion is on and `/h.pdf` of 40 000 bytes converts to an 8 576 byte Markdown file. When Mia uploads `/h.pdf` requesting conversion. Then both are accepted. And usage is exactly 1 048 576 bytes.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files; existing acceptance tests except declared count bumps.
- Dependencies not to add: any crate.
- Patterns to avoid: a `WHERE` without `volume_id`; a second implementation in `api/dataplane.rs`; charging the session budget before the project check; sleeps for synchronization; dashes as punctuation.
- Scope boundary: multi file (US-0018).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes, SPEC-0010's 8 quota tests included; session refusal message unchanged.
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0017: Conversion Markdown under the limit`, body `Spec: SPEC-0020` and the requirement ids.
