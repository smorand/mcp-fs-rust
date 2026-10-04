# US-0003: Atime/mtime tracking on read & write

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 3
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective

Activates `nodes.atime`, which exists in the schema but is never written anywhere today
(`storage/meta.rs:133`, read back unchanged at `core/fs_ops.rs:246`). Content-reading tools bump
`atime`; mutating tools bump both `atime` and `mtime`; metadata/listing tools touch neither. This
is the foundational signal every purge-sweep story (US-0005, US-0006) measures staleness against.

## Technical Context

### Stack
Rust 2024, tokio, the shared `core/fs_ops.rs` engine both MCP tools and REST routes call through.

### Relevant File Structure
```
crates/core/src/
  core/fs_ops.rs    (every fs.* operation's engine — wire the bump calls here)
  storage/meta.rs    (add the atime/mtime UPDATE query)
```

### Existing Patterns
- `nodes` schema: `mtime`, `ctime`, `atime` all `Double`/REAL (`storage/meta.rs:117-153`,
  `atime` declared at `:133`). `stat` reads it back verbatim (`core/fs_ops.rs:246`).
- The membership gate and every `fs.*` operation share one engine in `core/fs_ops.rs` — REST
  (`api/dataplane.rs`) and MCP (`mcp/server.rs`) both call into it, so wiring the bump here covers
  both surfaces for free; do not add REST-specific or MCP-specific bump code.
- Detached write: use `spawn_best_effort` from US-0001 (`core/fs_ops.rs`, this story's sibling).

### Decisions That Govern This Story
- **DEC-001** (Section 17): `atime` means true last-access, updated on every content read; every
  write also bumps it alongside `mtime`.
- **DEC-005**: `grep` counts as a content access (bumps `atime`) because it reads file content to
  search it, unlike pure metadata ops (`stat`/`glob`/`list`/`tree`), which never touch `atime`.
- **DEC-010**: the bump is async, best-effort (via US-0001's helper), to avoid write-amplifying
  the hot read path.

### Applicable NFRs
Section 7.5 (Observability): a failed bump is logged, never surfaced to the caller (enforced by
US-0001's helper, consumed here).

## Functional Requirements

### FR-NEW-001 [EARS-E]: Content reads bump `atime`
> WHEN a content-reading tool or REST route (`fs.read`, `fs.read_bytes`, `fs.read_lines`,
> `fs.read_section`, `fs.head`, `fs.tail`, `fs.extract_text`, `fs.grep`, and their REST GET
> equivalents) completes successfully THE system SHALL update the node's `atime` to the current
> time, asynchronously and best-effort (a failed bump never surfaces as a tool error).

- **Inputs / Outputs:** no change to any tool's input/output shape — the bump is a pure side
  effect.
- **Business Rules:** "Live file" exclusion (see FR-NEW-007 in US-0005) does not apply here — this
  story bumps `atime` on any live file read, trashed or not is irrelevant to the read path itself.

### FR-NEW-002 [EARS-E]: Writes bump both `atime` and `mtime`
> WHEN a mutating tool or REST route (`fs.write`, `fs.edit`, `fs.copy`, `fs.move`, and every
> other content-mutating `fs.*` tool and its REST equivalent) completes successfully THE system
> SHALL update the node's `atime` and `mtime` to the current time, via the same asynchronous
> best-effort mechanism as FR-NEW-001.

### FR-NEW-003 [EARS-UB]: Metadata/listing ops never touch `atime`
> The system SHALL NOT update `atime` as a result of `fs.stat`, `fs.glob`, `fs.list`, `fs.tree`,
> or any `admin.*` listing tool.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seeded node with fixed `atime`/`mtime` | File row with `atime = T0` set directly via DB write before the test acts | fixture | ready |

### E2E-NEW-001: Content read bumps atime
- **Category:** happy
- **Scenario:** SC-002 (precondition)
- **Requirements:** FR-NEW-001
- **Preconditions:** file `a.txt` seeded with `atime = T0`.
- **Steps:** Given file `a.txt` with `atime = T0` / When `fs.read(mount_id, "a.txt")` succeeds /
  Then, polled within 2000ms (or awaited via US-0001's test-only `Notify` hook if exposed to this
  call site), `node.atime > T0`.
- **Priority:** Critical

### E2E-NEW-002: Content read bumps atime via REST
- **Category:** happy
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001
- **Steps:** Given same setup as E2E-NEW-001 / When `GET /api/fs/{mount}/read?path=a.txt` returns
  200 / Then atime bumped (same assertion), proving REST and MCP share the mechanism via
  `core/fs_ops.rs`.
- **Priority:** Critical

### E2E-NEW-003: Content read bumps atime, every read tool
- **Category:** happy (table-driven)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001
- **Steps:** Repeat E2E-NEW-001 for each of `fs.read_bytes`, `fs.read_lines`, `fs.read_section`,
  `fs.head`, `fs.tail`, `fs.extract_text`, `fs.grep`. Each: atime bumped, `mtime` UNCHANGED.
- **Priority:** Critical

### E2E-NEW-004: Read never bumps mtime
- **Category:** side-effect (regression guard)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001
- **Steps:** Given `fs.read` succeeds / Then `mtime` is byte-for-byte unchanged (guards against
  FR-NEW-002's mechanism accidentally firing on a read).
- **Priority:** Critical

### E2E-NEW-005: Read on missing path fails, no stray row
- **Category:** failure
- **Requirements:** FR-NEW-001
- **Steps:** Given path does not exist / When `fs.read(mount_id, "missing.txt")` / Then fails
  `ERR_NOT_FOUND` and no row is created or touched.
- **Priority:** High

### E2E-NEW-006: Concurrent reads converge to one consistent atime
- **Category:** edge (concurrency)
- **Requirements:** FR-NEW-001
- **Steps:** Given 20 concurrent `fs.read` calls on the same file / When all complete / Then
  exactly one node row exists, atime within 2s of test "now", no panic, no lock error.
- **Priority:** High

**Note on E2E-NEW-007:** owned by US-0001 (not restated here as a test header); this story's
implementation of FR-NEW-001 must satisfy it regardless (the bump's failure path never surfaces
as a tool error).

### E2E-NEW-008: Metadata ops never bump atime
- **Category:** state (regression)
- **Requirements:** FR-NEW-003
- **Steps:** Given file with `atime = T0` / When `fs.stat`, `fs.glob`, `fs.list`, `fs.tree` each
  called / Then atime stays exactly `T0` after each, polled for 2s to rule out delayed async bump.
- **Priority:** Critical

### E2E-NEW-009: Admin listing never bumps any atime
- **Category:** state (regression)
- **Requirements:** FR-NEW-003
- **Steps:** Given `admin.list_projects`-style listing calls / When called / Then no `nodes.atime`
  row anywhere is touched (snapshot before/after, byte-identical).
- **Priority:** High

### E2E-NEW-010: Write bumps both atime and mtime
- **Category:** happy
- **Requirements:** FR-NEW-002
- **Steps:** Given file `b.txt`, `atime=T0, mtime=T0` / When `fs.write(mount_id, "b.txt", "new
  content")` succeeds / Then both atime and mtime poll `> T0`.
- **Priority:** Critical

### E2E-NEW-011: Write bumps both, every mutating tool
- **Category:** happy (table-driven)
- **Requirements:** FR-NEW-002
- **Steps:** Table-driven over `fs.edit`, `fs.copy`, `fs.move` (fresh fixture each). Each: atime
  and mtime both bumped on the resulting node.
- **Priority:** Critical

### E2E-NEW-012: Write bumps both via REST
- **Category:** happy
- **Requirements:** FR-NEW-002
- **Steps:** REST equivalent of `fs.write`. Then both timestamps bumped via REST path too.
- **Priority:** High

### E2E-NEW-013: Failed write touches neither timestamp
- **Category:** failure
- **Requirements:** FR-NEW-002
- **Steps:** Given a write that fails (e.g. write-quota exceeded, existing `ERR_WRITE_QUOTA_EXCEEDED`)
  / When called / Then neither atime nor mtime changes.
- **Priority:** High

### E2E-NEW-014: Partial-operation write touches neither side's timestamp wrongly
- **Category:** edge (partial op)
- **Requirements:** FR-NEW-002
- **Steps:** Given `fs.copy` with a source that exists and a destination whose parent does not
  exist (fails per existing engine behavior) / When called / Then source timestamps unchanged,
  destination absent.
- **Priority:** Medium

## Constraints

### Files Not to Touch
`tools/admin.rs`, `cli.rs`, `app.rs`, `storage/rel/schema.rs` (already done in US-0002).

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not add a synchronous, same-transaction atime update — DEC-010 explicitly rejected that
(write-amplifies the hot read path). Do not duplicate the bump logic in `api/dataplane.rs`;
wire it once in `core/fs_ops.rs`.

### Scope Boundary
Only the bump mechanism and its wiring into existing read/write call sites. No purge logic, no
new tools, no config reads beyond what US-0002 already added (this story doesn't even need
US-0002's config, only US-0001's helper).

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace`, especially every existing `fs_ops.rs` read/write test — none of their
assertions should change, since atime/mtime were previously unobserved by any assertion.

### Behaviors That Must Not Change
Read/write tool response shapes are unchanged — the bump is a pure side effect, invisible to the
caller's return value.

### API Contracts to Preserve
Every existing `fs.*` tool and REST route signature, unchanged.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
