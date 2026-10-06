# US-0001: `trash_entries` persistence layer

> Parent Spec: specs/SPEC-0011_2026-10-05_23-49-49-trash-listing-and-recovery/spec.md
> Spec ID: SPEC-0011
> Epic: n/a
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: M
> min_tier: 2
> Files touched: 5

## Objective
Add a new `trash_entries` table (one row per soft-deleted top-level path, per volume) and the
trait-level CRUD methods that read and write it. This is pure infrastructure: nothing calls it yet.
It exists so US-0002 (delete writes a row) and beyond can be built against a stable persistence
contract across all three relational backends.

## Technical Context

### Stack
Rust 2024, rusqlite (bundled), sqlx (optional, PostgreSQL, feature `postgres`), tiberius-ng (optional,
SQL Server, feature `sqlserver`). The relational layer speaks only `storage::rel::RelationalDb` /
`storage::rel::RelationalTx` — never a driver directly.

### Relevant File Structure
```
crates/core/src/storage/
  traits.rs       # AdminBackend / MetaBackend trait definitions — add trash_entries methods here
  meta.rs         # the generic MetaBackend impl shared by all three dialects
  rel/
    schema.rs     # DDL / migrations, one block per dialect
    sqlite.rs
    postgres.rs
    sqlserver.rs
```

### Existing Patterns
The `nodes` and `blob_refs` tables are the model to follow for a new table: every row is scoped by
`volume_id` in its primary key and in every `WHERE` clause (`.agent_docs/architecture.md`,
AGENTS.md's own Conventions: *"`volume_id` scopes every row of `nodes`, `blob_refs` and `git_*`... It
belongs in EVERY `WHERE` clause: omitting it leaks or corrupts another volume's rows."*). Look at how
`nodes` is declared in `storage/rel/schema.rs` and mirrored in `sqlite.rs`/`postgres.rs`/`sqlserver.rs`
to see the three-dialect pattern this story must repeat. SQL Server cannot index `NVARCHAR(MAX)`, so
any keyed text column there uses the project's existing `TextKey(n)` bounded-length convention
(AGENTS.md: *"`TextKey(n)` bounded keys (SQL Server cannot index `NVARCHAR(MAX)`, so keyed text has a
length ceiling there)"*) — `trash_path` is a primary-key column and must use it.

A `LIKE` pattern (needed later by US-0004's `path_prefix` filter, but worth knowing now since it
constrains how you design the query method) is always built with `descendant_pattern` /
`Dialect::escape_like_literal`, never by hand (AGENTS.md Conventions).

### Data Model (excerpt)
New table `trash_entries` (spec §8), one row per volume per soft-deleted top-level path:

| Column | Type | Notes |
|---|---|---|
| `volume_id` | string | part of the composite primary key |
| `trash_path` | string (bounded per `TextKey` on SQL Server) | part of the composite primary key |
| `original_path` | string | the path to restore to |
| `size` | integer | copied from the node at delete time |
| `kind` | string, `'file'` \| `'dir'` | |
| `deleted_at` | string (RFC3339) | matches `admin.list_deleted_projects`'s convention |
| `deleted_by` | string, nullable | `null` for sweep-initiated deletes |

No changes to `NodeRow` or any existing table.

### Decisions That Govern This Story
- **DEC-001** (spec §17): "New files/sweep both write a `trash_entries` row rather than
  reconstructing `original_path` from the trash path string going forward. Rationale: the existing
  flatten encoding is lossy/ambiguous (`/a/b.txt` and `/a__b.txt` collide); an explicit row removes
  the ambiguity for all future deletes." This story builds the row this decision depends on.

### Applicable NFRs
- **7.4 Reliability** (spec §7.4): "The `trash_entries` insert/delete happens in the same
  transaction as the corresponding `rename` (`run_retrying`, per `storage/meta.rs:701`'s existing
  transactional pattern) — a failed rename never leaves an orphaned `trash_entries` row, and a failed
  insert never leaves an untracked rename (both roll back together)." This story's methods must be
  callable from inside the same transaction a caller (US-0002/US-0003) already holds — do not open a
  new transaction internally; take `&mut RelationalTx` (or this project's equivalent transaction
  handle type) as a parameter, matching how `rename` itself is implemented at `storage/meta.rs:701`.
- **7.7 Scalability** (spec §7.7): `trash_entries` is indexed by `(volume_id, trash_path)` as its
  primary key; add a secondary index on `(volume_id, deleted_at)` to support `fs.trash_list`'s
  ordering (built in US-0004, but the index belongs here with the table).

### Bounded Context
**Trash tracking** (spec §4.5): "Recording and querying what has been soft-deleted and from
where." Key entity: `trash_entries` row.

## Functional Requirements

### FR-NEW-001: `trash_entries` table
- **EARS:** [EARS-U] "The system SHALL persist one `trash_entries` row per soft-deleted top-level
  path, with columns `volume_id, trash_path (primary key together with volume_id), original_path,
  size, kind ('file' | 'dir'), deleted_at (RFC3339 string), deleted_by (nullable string)`, added to
  `storage/rel/schema.rs` and implemented across `storage/rel/sqlite.rs`, `storage/rel/postgres.rs`,
  `storage/rel/sqlserver.rs` per the dialect checklist in `.agent_docs/backends.md`."
- **Inputs / Outputs:** no public API yet — this story exposes trait methods only, consumed by later
  stories: `insert_trash_entry`, `delete_trash_entry`, `get_trash_entry`, `list_trash_entries`
  (exact signatures below).
- **Business Rules:** every row is scoped by `volume_id`; `trash_path` is bounded-length on SQL
  Server via `TextKey`.

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until
> zero failures. Run tests through `cargo test --workspace` (per AGENTS.md's Key Commands). Never
> run test files directly.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| in-memory/temp sqlite volume | standard test fixture already used by every `storage/` test | existing fixture pattern | ready |

This story has **no spec-numbered `E2E-NEW-*` test** — every test in the spec's suite that exercises
`trash_entries` does so through `fs.delete`/`sweep_project_files` (built in US-0002/US-0003), because
the table has no observable behavior independent of a writer. Per the spec's own test-design review
(§12.1, "the table alone has no spec-listed E2E test; integration through delete/sweep exercises
it"), this story's floor-check verification is a **direct unit test of the new trait methods**, not a
numbered spec test:

### Unit verification (write this test; it is this story's acceptance criterion)
- **Category:** Happy / Edge
- **Driver:** direct function call against the trait, on each of the three backends
  (`cargo test --workspace --all-features` to compile and run the postgres/sqlserver paths too; see
  `.agent_docs/testing.md` for the opt-in infra those suites need)
- **Preconditions:** an empty test volume, any `volume_id`.
- **Steps:**
  - Given an empty `trash_entries` table for `volume_id="v1"`.
  - When `insert_trash_entry(tx, "v1", "/.mcp_trash/123__a.txt", "/a.txt", 5, "file",
    "2026-01-01T00:00:00Z", Some("alice"))` is called inside an open transaction.
  - Then `get_trash_entry(tx, "v1", "/.mcp_trash/123__a.txt")` returns the row with every field
    round-tripped exactly, including `deleted_by = Some("alice")`.
  - And inserting a second row with `deleted_by: None` round-trips a real SQL `NULL`, not an empty
    string (assert on the raw column, not just the deserialized `Option`).
  - And `delete_trash_entry(tx, "v1", "/.mcp_trash/123__a.txt")` removes exactly that row; a second
    delete of the same key is a no-op (no error), confirming idempotent delete.
  - And `list_trash_entries(tx, "v1", path_prefix="", limit=10, offset=0)` returns rows ordered by
    `deleted_at` descending.
  - And a second `volume_id="v2"` row is invisible to any query scoped to `"v1"` (volume isolation).
- **Priority:** Critical

## Constraints

### Files Not to Touch
- `crates/core/src/storage/meta.rs`'s existing `nodes`/`blob_refs` query methods — add new methods,
  do not modify existing ones.
- `crates/core/src/core/fs_ops.rs`, `crates/core/src/purge.rs` — not touched by this story (US-0002,
  US-0003).

### Dependencies Not to Add
- No new crate. This is pure SQL/trait work on the existing `rusqlite`/`sqlx`/`tiberius-ng` stack.

### Patterns to Avoid
- Do not hand-build a `LIKE` pattern for the prefix filter method; even though US-0004 is the first
  caller, if you add a prefix-matching method here, it must go through `descendant_pattern` /
  `Dialect::escape_like_literal` like every other `LIKE` in this codebase.

### Scope Boundary
- This story adds the table and its CRUD trait methods only. No caller, no MCP tool, no backfill
  logic (that is FR-NEW-011, in US-0004/US-0005).

## Non Regression

### Existing Tests That Must Pass
- `cargo test --workspace` stays green — this is a purely additive table, no existing table or query
  is touched.

### Behaviors That Must Not Change
- Nothing about `nodes`/`blob_refs` or any existing query changes.

### API Contracts to Preserve
- n/a — no public API in this story.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
