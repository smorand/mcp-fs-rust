# US-005: In-progress guard and git.status operation reporting

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 5
> Depends On: US-003
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

Stop a caller from interleaving a second history operation into a volume that already has one paused, and make the paused operation visible. This story adds the in-progress guard across every ref-mutating tool and the `operation` object on `git.status`.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/db.rs
```

### Existing Patterns
Table declaration lives in `crates/mcp-fs/src/git/db.rs:66-90` (`fn schema()`), the owned-table list in `:42` (`TABLES`), and every query is scoped by `volume_id`.
The existing `git.status` handler is `crates/mcp-fs/src/tools/git.rs:512-538`; `refs_under` filtering is `crates/mcp-fs/src/tools/git.rs:539-554`.
Every git tool authorizes first: see the `state.authorize(mount_id, person)` pattern at `crates/mcp-fs/src/tools/git.rs:6-8` and the membership check at `crates/mcp-fs/src/tools/git.rs:386-396`.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-901:** An in-progress operation is persisted as a row in a new relational `git_operations` table. **Rationale:** it is the only option where a paused rebase survives a server restart and is visible to every project member; the alternatives lose or hide it. **Alternatives considered:** (B) mirror real git's control files inside the bare repo at `state/git-repos/{project}/`, rejected because that directory is documented as a rebuildable cache rather than the source of truth, so a cache rebuild would destroy authoritative state; (C) in-memory session state like the read-before-write guard in `crates/mcp-fs/src/safety.rs`, rejected because a restart would evaporate a paused rebase and leave the volume half-applied with no way to continue or abort, which is data-integrity damage rather than inconvenience, and because it inherits the known BL-003/BL-014 scaling defect. **Implemented by:** FR-NEW-275, FR-NEW-276, FR-NEW-277, FR-NEW-278. **Round:** 2 (approach exploration). **Code evidence:** `crates/mcp-fs/src/git/db.rs:42` (three tables today), `crates/mcp-fs/src/git/repo.rs:44` (in-process lock), `crates/mcp-fs/src/safety.rs` (in-memory session pattern).

- **DEC-914:** Any project member can continue or abort an operation another member started. **Rationale:** authorization here is membership-based with no per-resource ownership, and the volume is shared; an operation only its author could clear would let one absent person block the whole project. **Implemented by:** FR-NEW-282. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:6-8` (membership-only authorization).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

5 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-279 [EARS-S]: An in-progress operation blocks other mutations
> WHILE an operation is in progress for a volume THE mcp-fs server SHALL reject `git.commit`, `git.branch_switch`, `git.branch_create`, `git.branch_delete`, `git.branch_reset`, `git.reset`, `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_save`, `git.stash_apply`, `git.stash_pop`, `git.remote_pull` and `git.remote_clone` with `ERR_INVALID_ARGUMENT` naming the active operation type and the tools that finish it.

- **Priority:** Must-have

### FR-NEW-280 [EARS-S]: Read-only tools stay available during an operation
> WHILE an operation is in progress THE mcp-fs server SHALL continue to serve `git.status`, `git.log`, `git.show`, `git.diff`, `git.branches`, `git.tags`, `git.blame`, `git.stash_list`, `git.remote_list` and `git.remote_fetch` normally.

- **Business Rules:** `git.remote_fetch` is included because it touches only remote-tracking refs and never the volume or local branches.
- **Priority:** Must-have

### FR-NEW-281 [EARS-E]: Status reports the active operation
> WHEN `git.status` is called while an operation is in progress THE mcp-fs server SHALL include an `operation` object carrying `op_type`, `current_step`, `total_steps`, the unresolved conflicting paths and the names of the tools that continue and abort it.

- **Priority:** Must-have

### FR-NEW-282 [EARS-U]: An operation is not owned by one person
> The mcp-fs server SHALL allow any member of the project to resolve, continue or abort an operation another member started.

- **Business Rules:** Authorization is membership-based with no per-operation ownership; the volume is shared, so a paused operation that only its author could clear would be a denial of service on the whole project.
- **Priority:** Must-have

### FR-MOD-107 [EARS-E]: `git.status` reports the in-progress operation
> WHEN `git.status` is called THE mcp-fs server SHALL include the `operation` object of FR-NEW-281 when an operation is in progress, and SHALL omit it otherwise.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

8 tests: Concurrency 2, EdgeCase 1, Failure 4, Happy 1.
Scenarios covered: SC-904, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-509 — git.status reports the active operation

**Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-MOD-107., FR-NEW-187
**Category:** Happy. **Priority:** P1. **Preconditions:** SEED-MULTI(3), `git.merge feature` → conflict, then resolve only `/f/000.txt`.
**When** `git.status {mount_id:"proj"}`.
**Then** the response contains `operation` (the object named by FR-NEW-281) with exactly `{"op_type":"merge","source_ref":"feature","current_step":null,"total_steps":null,"remaining_conflicts":["/f/001.txt","/f/002.txt"],"continue_with":"git.merge_resolve","abort_with":"git.merge_abort"}`.
**And** the pre-existing `git.status` fields (`head`, `branch`, `refs`) are still present and unchanged relative to a `git.status` taken before the merge, except the added key — proving the schema is additive.
**Verification:** JSON subset equality both ways.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-519 — merge_resolve with nothing in progress

**Scenario:** SC-904. **Requirements:** FR-NEW-198.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, no merge started.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml",strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no operation in progress`.
**And** volume bytes and `refs/heads/main` unchanged.
**Verification:** error code + substring; byte compare; ref sha.

#### E2E-NEW-520 — merge_abort with nothing in progress

**Scenario:** SC-904. **Requirements:** FR-NEW-198., FR-NEW-186
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, no merge started.
**When** `git.merge_abort {mount_id:"proj"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no operation in progress`.
**And** `refs/heads/main` still `C2`; volume bytes unchanged.
**Verification:** error code + substring; ref + bytes.

#### E2E-NEW-528 — git.commit blocked while a merge is in progress

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge in progress.
**When** `git.commit {mount_id:"proj", message:"sneak"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains all of: `merge`, `in progress`, `git.merge_resolve`, `git.merge_abort`.
**And** `git.log "main"` length unchanged (no commit `sneak`).
**Verification:** error code + four substrings; log contents scanned for `sneak`.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-530 — second git.merge blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279, FR-NEW-283.
**Category:** Failure. **Priority:** P0. Same preconditions.
**When** `git.merge {source_ref:"feature"}` again.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`.
**And** exactly one `git_operations` row still (count == 1), and its stored conflict set is unchanged.
**Verification:** error code; relational count + row payload equality against the pre-call snapshot.

#### E2E-NEW-579 — read-only tools remain available during an in-progress operation

**Scenario:** SC-929. **Requirements:** FR-NEW-280.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge in progress.
**When** `git.log`, `git.show {commit_sha:<C2>}`, `git.diff {from:<C0>, to:<C2>}`, `git.branches`, `git.status`, and `fs.read_text {path:"/src/config.toml"}` are each called.
**Then** all six return `Ok`. `fs.read_text` returns exactly the SEED-CONFLICT main text (`port = 8000`). `git.log "main"` returns exactly 2 entries.
**And** `git_operations` count is still 1 afterwards (no read cleared it).
**Verification:** six `is_ok()` assertions; exact content assertions; relational count.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-580 — another member can abort the operation

**Scenario:** SC-929. **Requirements:** FR-NEW-282.
**Category:** Concurrency. **Priority:** P1. **Preconditions:** SEED-CONFLICT; `second@acme.test` added as a member of `proj`; merge started by OWNER and conflicting.
**When** `env.as_person("second@acme.test","git.merge_abort",{mount_id:"proj"})`.
**Then** it succeeds (`status == "aborted"`) — authorization is membership only, no per-operation ownership.
**And** `git_operations` count == 0; the volume byte map equals the pre-merge snapshot.
**And** the abort audit entry is recorded under `second@acme.test`, not OWNER: `safety.audit("second@acme.test","proj")` has exactly 1 entry with `op == "git.merge_abort"`, and `safety.audit(OWNER,"proj")` gained none.
**Verification:** `is_ok()` + field; relational count; map equality; two per-person audit reads.

---

#### E2E-NEW-814 — EdgeCase — the key is absent when nothing is in progress**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-MOD-107.
- **Preconditions:** SEED-CONFLICT, nothing started.
- **When** `git.status {mount_id:"proj"}`.
- **Then** the response has **no** `operation` (the object named by FR-NEW-281) key at all (`resp.get("operation").is_none()`), not a `null` and not an empty object.
- **When** `git.merge {source_ref:"feature"}` conflicts, then `git.merge_abort`.
- **Then** a second `git.status` again has no `operation` (the object named by FR-NEW-281) key, and its `head`, `branch` and `refs` fields are equal key-for-key to the first `git.status` response.
- **Verification:** `serde_json::Value::get` returning `None`; whole-object equality of the two status responses.
- **Cleanup:** fixture drop. **Priority:** P1.

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
