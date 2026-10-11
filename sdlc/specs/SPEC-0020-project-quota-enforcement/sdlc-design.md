# SPEC-0020: Project storage quota enforcement, design

> Spec: sdlc/specs/SPEC-0020-project-quota-enforcement/sdlc-spec.md
> Nature: FEAT
> Depth: L
> Security: n/a
> Designed on: 2026-10-11
> Target tier: 2 (default, no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json` tier)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, MCP, REST, persistence | existing Rust workspace (rmcp tools, axum REST, `RelationalDb` meta and admin stores) | manifest | DEC-001 |

No dependency added or removed, so no context7 lookup. Skill loaded: `rust` (make check gate,
ToolError in core, tracing on boundaries, typed tool parameters, e2e on real entry points).

## 2. Approaches

### Chosen: A, a per volume usage counter kept in the node transactions, checked under a per volume admission lock

A row `volume_usage(volume_id, used_bytes)` in the meta store is updated in the same transaction as
every node byte change (`put_file`, `delete_file`, `remove_subtree`; `rename` changes nothing). Every
growing operation passes one admission function that takes a per volume async lock, reads the counter,
computes the net change, refuses or charges the session budget, then performs its writes before
releasing the lock.

- Pros: exact usage at O(1) per check (NFR 7.1 at 100 000 files); usage maintained where bytes change,
  so purge, delete and copy stay correct with no extra hook; the lock gives FR-NEW-008 and the all or
  nothing check of FR-NEW-007 without a reservation protocol.
- Cons and risks: single replica (in process lock), already accepted by SPEC-0019 DEC-008 and covered
  by BL-0015; one backfill per volume.
- Affected scope: storage/meta, a new `quota` module, every write entry (fs_ops, archive, docs,
  artifacts, git), tools/admin, mcp, api, errors.

### Rejected: B, aggregate query at each check

`SUM(size)` over `nodes` per volume at every write. Rejected (DEC-002): O(files) per write, against NFR
7.1, and still needs the lock for FR-NEW-008.

### Rejected: C, conditional update in the store only (no lock)

`UPDATE volume_usage SET used = used + ? WHERE used + ? <= limit` inside `put_file`. Rejected (DEC-002):
multi file operations write file by file, so a refusal midway breaks FR-NEW-007 and FR-NEW-013; it
would need a reservation table.

## 3. Architecture

```
 every growing entry point (fs_ops writes, archive, conversion sibling, git apply, checkout_file, clone)
        |
        v
 quota::admit(state, mount_id, person, session_bytes, delta) -> Admission (guard)
        |   1. lock volume (per volume tokio mutex, in process)
        |   2. usage = meta.volume_usage()  (backfilled once from SUM(nodes.size))
        |   3. limit = admin.get_project(mount).quota_bytes
        |   4. delta > 0 and limit set and usage + delta > limit -> ERR_PROJECT_QUOTA_EXCEEDED
        |   5. safety.charge_write(person, mount, session_bytes)   (only after 4 passed)
        v
 writes through VolumeClient -> meta.put_file / delete_file adjust volume_usage in their transaction
        |
 guard dropped after the last write
 admin.get_project_quota -> tools/admin.rs -> {project_id, max_mb, used_bytes}  (MCP and REST)
```

Bounded contexts: Storage = `quota` module and the counter in `storage/meta.rs`; Administration =
`tools/admin.rs` read and its gate; Session budget = `safety.rs`, unchanged except its call order.

## 4. Data model

**`volume_usage`** (new, meta schema, PK `volume_id`): `volume_id TextKey`, `used_bytes BigInt`.

- `put_file` adds `new_size - old_size` (it already reads the old size, `meta.rs:1615`); a new path
  adds `new_size`. `delete_file` subtracts the node size; `remove_subtree` subtracts the sum of the
  removed files; `rename` changes nothing (trash is a rename, so trashed files stay counted). All in
  the existing transactions, `volume_id` in every `WHERE`.
- Backfill: when the row is missing, `INSERT ... SELECT SUM(size) FROM nodes WHERE volume_id=? AND
  mode=file` under the admission lock, once.
- Migration additive; rollback: drop the table (it is derived data).

No change to the project table: `quota_bytes` exists (`storage/traits.rs:148`).

## 5. Contracts and interfaces

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

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | modify: `volume_usage` table, counter in node transactions, backfill, read | FR-NEW-002 | US-0015 |
| `crates/core/src/storage/conformance.rs` | modify: usage counter case | FR-NEW-002 | US-0015 |
| `crates/core/src/quota.rs` | create: admission lock, check, usage read | FR-NEW-001, 006, 008 | US-0015, US-0016 |
| `crates/core/src/tools/admin.rs`, `mcp/server.rs`, `api/dataplane.rs`, `api/openapi.rs` | modify: `admin.get_project_quota` and its route | FR-NEW-009, 010 | US-0015 |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, `tools/contract_golden.rs` | regenerate, counts 107 to 108 | contract | US-0015 |
| `crates/core/src/errors.rs` | modify: `ERR_PROJECT_QUOTA_EXCEEDED` | FR-NEW-001 | US-0016 |
| `crates/core/src/core/fs_ops.rs` | modify: single file entries and copy use `admit` | FR-NEW-003 to 006, 013 | US-0016 |
| `crates/core/src/core/fs_ops.rs` (`land_conversion`, documented write) | modify | FR-NEW-014 | US-0017 |
| `crates/core/src/tools/archive.rs`, `storage/volume.rs` (folder copy delta) | modify | FR-NEW-007 | US-0018 |
| `crates/core/src/git/merge.rs`, `crates/core/src/tools/git.rs` | modify: `charge_quota` and direct paths use `admit` | FR-NEW-003, 012 | US-0019 |
| `crates/core/src/tools/quota_e2e.rs` | create: e2e tests of the spec | all | US-0015 to US-0020 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/architecture.md`, `.agent_docs/api.md` | modify | docs | US-0020 |

Affected existing tests: keep all. Tool count assertions bumped (107 to 108) in US-0015; the error
code count tests in `errors.rs` bumped (15 to 16) in US-0016. SPEC-0010's 8 quota tests keep passing
(FR-MOD-001 keeps the set behavior).

## 7. Compatibility and risks

- Projects with a limit start refusing growing writes as soon as this ships, including those already
  over (FR-MOD-001, declared).
- Session budget refusals keep their message; they now come after the project check (FR-NEW-006).
- Ordering: US-0015 lands the counter before any enforcement reads it.
- Risk: a write path not switched to `admit` stays unchecked; mitigation: the version control paths
  are enumerated in spec 14.3 and each has an e2e test.
- Single replica lock (BL-0015). Rollback: remove the `admit` calls; the counter is harmless.

## 9. Test strategy

| Tests | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 to E2E-069 | functional e2e | `McpServer` tool calls; version control tests with local repos as `tools/git.rs` tests do; REST harness for the read route parity | `crates/core/src/tools/quota_e2e.rs` | per story file |
| usage counter case | integration | `RelationalDb` | `crates/core/src/storage/conformance.rs` | US-0015 |

Test data: generated (byte buffers of exact sizes, archives built in memory, local git repos).
Concurrency tests synchronize with tokio barriers, never sleeps.

## 10. Decisions log

- **DEC-001:** Reuse the existing stack; no new dependency. **Obliged by:** n/a (no code impact):
  existing project. **Rationale:** everything needed exists. **Alternatives considered:** none.
  **Source:** existing code. **Evidence:** `Cargo.toml`.
- **DEC-002:** Per volume `volume_usage` counter maintained in the node transactions, checked under a
  per volume in process admission lock held across the operation's writes. **Obliged by:**
  SPEC-0020/FR-NEW-002, 007, 008, 013, NFR 7.1. **Rationale:** O(1) exact check, all or nothing for
  multi file operations, no overshoot. **Alternatives considered:** aggregate query per check (B),
  conditional update without lock (C). **Source:** design. **Evidence:** `storage/meta.rs:1615`,
  `:1693`, `:1728`.
- **DEC-003:** Net change computed by the caller (it knows the previous sizes and removed paths) and
  passed to `admit`. **Obliged by:** FR-NEW-003, FR-NEW-004. **Rationale:** one admission per
  operation. **Alternatives considered:** per file checks inside `put_file` (breaks FR-NEW-007).
  **Source:** design. **Evidence:** `git/merge.rs:691-707`.
- **DEC-004:** Order inside `admit`: project check first, session `charge_write` only after it passes.
  **Obliged by:** FR-NEW-006. **Rationale:** a refusal charges nothing. **Alternatives considered:**
  charge then refund (racy with other session writes). **Source:** spec. **Evidence:**
  `safety.rs:130-136`.
- **DEC-005:** Error code `ERR_PROJECT_QUOTA_EXCEEDED`, HTTP 507 Insufficient Storage, message per
  FR-NEW-001. **Obliged by:** FR-NEW-001 and spec 14.3 (distinct category). **Rationale:** 507 names
  storage exhaustion, distinct from the 429 rate style session refusal. **Alternatives considered:**
  reuse 429 (indistinguishable for HTTP consumers by status), 413 (about one request body).
  **Source:** design. **Evidence:** `errors.rs:111-134`.
- **DEC-006:** Read tool `admin.get_project_quota` returning `project_id`, `max_mb` (null when
  unlimited), `used_bytes`, plus REST `GET /api/admin/projects/{project_id}/quota` on the same
  function. **Obliged by:** FR-NEW-009, constraint 3.3. **Rationale:** mirrors
  `admin.set_project_quota` labels. **Alternatives considered:** adding usage to `admin.list_projects`
  (wider read, other gate). **Source:** existing code. **Evidence:** `mcp/server.rs:2789-2792`.
- **DEC-007:** Read gate: admin then existence; non admin gets the owner refusal whether the project
  exists or not. **Obliged by:** FR-NEW-010. **Rationale:** no probing of project ids. **Alternatives
  considered:** reuse `require_owner_or_admin` (checks existence first for non admins,
  `state.rs:47-54`). **Source:** spec. **Evidence:** `storage/admin.rs:403-414`.
- **DEC-008:** Conversion sibling: project delta counts the sibling only; artifacts and conversion
  text stay out (session charge unchanged). **Obliged by:** FR-NEW-002, FR-NEW-014. **Source:** spec.
  **Evidence:** SPEC-0019 `land_conversion`. **Rationale:** artifacts are not project files.
  **Alternatives considered:** counting artifacts (contradicts FR-NEW-002).
- **DEC-009:** Aborts, moves, trash, restore, push never call `admit`. **Obliged by:** FR-NEW-012.
  **Rationale:** they add no project bytes. **Alternatives considered:** checking with delta 0
  (needless lock). **Source:** spec. **Evidence:** `tools/git.rs:1600-1727`.
- **DEC-010:** Stories at `min_tier` 1 when they touch the contract trio or many entry points, 2
  otherwise. **Obliged by:** n/a (no code impact): routing only. **Rationale:** vertical slices.
  **Alternatives considered:** horizontal split (forbidden). **Source:** design. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0015 | Project usage and quota read | P1 | FR-NEW-002, 009, 010, 011 | main | 1 | no |
| US-0016 | Single file writes refused over the limit | P1 | FR-NEW-001, 003, 004, 005, 006, 013 | US-0015 | 1 | no |
| US-0017 | Conversion Markdown under the limit | P1 | FR-NEW-014 | US-0016 | 2 | no |
| US-0018 | Archive and folder copy all or nothing | P1 | FR-NEW-007 | US-0017 | 2 | no |
| US-0019 | Version control operations under the limit | P1 | FR-NEW-003 (version control), FR-NEW-012 (aborts) | US-0018 | 1 | no |
| US-0020 | Lowered limit, exempt operations, concurrency | P2 | FR-MOD-001, FR-NEW-008, FR-NEW-012 | US-0019 | 1 | yes |
