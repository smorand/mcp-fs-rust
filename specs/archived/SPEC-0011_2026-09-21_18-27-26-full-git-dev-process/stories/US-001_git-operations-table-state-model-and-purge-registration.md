# US-001: git_operations table, state model and purge registration

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

Create the relational home for an operation that is paused mid-flight, so a conflicted merge or a half-finished rebase survives a server restart and is visible to every member of the project. Nothing in the codebase has this concept today. This story adds the table, its dialect definition across all three backends, and its registration for project purge.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/git/db.rs
crates/mcp-fs/src/storage/conformance.rs
```

### Existing Patterns
Table declaration lives in `crates/mcp-fs/src/git/db.rs:66-90` (`fn schema()`), the owned-table list in `:42` (`TABLES`), and every query is scoped by `volume_id`.
The dialect checklist for a new table is `.agent_docs/backends.md`; keyed text uses `ColumnType::TextKey(n)` because SQL Server cannot index unbounded text, while payload columns use `Text`.
Purge iterates `TABLES` at `crates/mcp-fs/src/storage/mod.rs:357`. Note that on SQLite `purge_repo` (`crates/mcp-fs/src/git/repo.rs:146-161`) deletes the index file outright and never reads `TABLES`, so the purge consequence is only observable on PostgreSQL and SQL Server.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-901:** An in-progress operation is persisted as a row in a new relational `git_operations` table. **Rationale:** it is the only option where a paused rebase survives a server restart and is visible to every project member; the alternatives lose or hide it. **Alternatives considered:** (B) mirror real git's control files inside the bare repo at `state/git-repos/{project}/`, rejected because that directory is documented as a rebuildable cache rather than the source of truth, so a cache rebuild would destroy authoritative state; (C) in-memory session state like the read-before-write guard in `crates/mcp-fs/src/safety.rs`, rejected because a restart would evaporate a paused rebase and leave the volume half-applied with no way to continue or abort, which is data-integrity damage rather than inconvenience, and because it inherits the known BL-003/BL-014 scaling defect. **Implemented by:** FR-NEW-275, FR-NEW-276, FR-NEW-277, FR-NEW-278. **Round:** 2 (approach exploration). **Code evidence:** `crates/mcp-fs/src/git/db.rs:42` (three tables today), `crates/mcp-fs/src/git/repo.rs:44` (in-process lock), `crates/mcp-fs/src/safety.rs` (in-memory session pattern).

### Applicable NFRs

- 7.4 Reliability: a paused operation survives a restart (FR-NEW-278).
- 7.7 Scalability: the per-project write lock stays in-process; single-replica assumption is retained.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

7 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-275 [EARS-U]: The operation record is a relational table
> The mcp-fs server SHALL persist every in-progress operation as a row in a new relational table `git_operations`, keyed by `volume_id`, holding at minimum: `op_type`, `state`, `onto_sha`, `original_tip_sha`, `todo`, `current_step`, `total_steps`, `conflicts`, `resolutions`, `created_at` and `updated_at`.

- **Business Rules:** `volume_id` scopes the row because one database holds every volume. One volume holds at most one in-progress operation. The table lives in the git index alongside `git_objects`, `git_refs` and `git_remotes` (`crates/mcp-fs/src/git/db.rs:42`).
- **Priority:** Must-have

### FR-NEW-276 [EARS-E]: The new table is registered for purge
> WHEN `git_operations` is added THE mcp-fs server SHALL include it in the `TABLES` constant of the git index so that deleting a project removes its operation rows.

- **Business Rules:** `pub const TABLES: [&str; 3]` at `crates/mcp-fs/src/git/db.rs:42` is commented "named once so a purge cannot miss one" and becomes `[&str; 4]`. Omitting this leaves orphaned rows behind after project deletion. This is its own requirement rather than a footnote precisely because it is easy to miss.
- **Priority:** Must-have

### FR-NEW-277 [EARS-U]: The new table works on every relational backend
> The mcp-fs server SHALL define `git_operations` through the existing dialect abstraction so it is created and queried identically on SQLite, PostgreSQL and SQL Server.

- **Business Rules:** Keyed text columns respect the SQL Server indexable-length ceiling by using `TextKey(n)`; free payload columns (`todo`, `conflicts`, `resolutions`) use unbounded `Text`. See `.agent_docs/backends.md`.
- **Priority:** Must-have

### FR-NEW-278 [EARS-U]: A paused operation survives a restart
> The mcp-fs server SHALL make a paused operation resumable after a server restart, with its `current_step`, `todo` and `conflicts` unchanged.

- **Business Rules:** This is the reason the record is relational rather than in-memory (DEC-901).
- **Priority:** Must-have

### FR-NEW-283 [EARS-U]: One operation per volume
> The mcp-fs server SHALL hold at most one in-progress operation row per `volume_id`, and SHALL scope every operation query by `volume_id`.

- **Priority:** Must-have

### FR-NEW-284 [EARS-E]: Completion and abort clear the record
> WHEN an operation completes successfully or is aborted THE mcp-fs server SHALL delete its `git_operations` row.

- **Business Rules:** A stale row would permanently block the volume; row removal is verified by test, not assumed.
- **Priority:** Must-have

### FR-NEW-285 [EARS-U]: The operation type is a six-value closed set
> The mcp-fs server SHALL store and report `op_type` as exactly one of `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply` or `stash_pop`, SHALL write `stash_pop` for a paused `git.stash_pop` and `stash_apply` for a paused `git.stash_apply`, and SHALL use that same value in the conflict response's `operation` field and in the `op_type` of the `operation` object returned by `git.status`.

- **Business Rules:** The two stash values are distinguished because `dropped` differs on completion: a resumed `stash_pop` deletes the entry, a resumed `stash_apply` keeps it, so the persisted row must remember which was called. `remote_pull` is recorded as `merge`, since a pull conflict is a merge conflict completed by the merge tools (FR-NEW-241). The Section 8.1 `op_type` row lists these six values.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

1 tests: SideEffect 1.
Scenarios covered: SC-917.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-620 — Rebase abort: row lifecycle**

**Category:** SideEffect. **Scenario:** SC-917. **Requirements:** FR-NEW-284.
- **Preconditions:** FX-FORK conflicting, rebase paused.
- **Given** exactly 1 `git_operations` row (`op_type='rebase'`).
- **When** `git.rebase_abort {}`.
- **Then** `SELECT count(*) ... WHERE volume_id='proj1'` == 0.
- **And** a second `git.rebase_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** a fresh `git.rebase` with the same todo pauses again identically (`current_step` identifies F1) — proving the cleared row did not poison state.
- **Priority:** P0.


## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not reimplement an operation in the tool layer: the engine lives in one place and the tool is a thin adapter. Do not build a `LIKE` pattern by hand; use `descendant_pattern` / `Dialect::escape_like_literal`. Do not block a request thread on the database or on libgit2.

### Scope Boundary
Only the requirements listed above. Anything else in the parent specification belongs to another story.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
