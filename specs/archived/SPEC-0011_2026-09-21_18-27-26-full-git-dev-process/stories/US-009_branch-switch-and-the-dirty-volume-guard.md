# US-009: Branch switch and the dirty-volume guard

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 9
> Depends On: US-008
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Let a caller move between branches. `git.branch_switch` repoints HEAD and rewrites the volume to the target tree, refusing when the volume holds uncommitted work.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
The dirty-volume comparison is `require_clean_volume` (`crates/mcp-fs/src/tools/git.rs:1991-2013`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Volume** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

3 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-105 [EARS-E]: Switch branches
> WHEN `git.branch_switch` is called with `name` THE mcp-fs server SHALL point HEAD at `refs/heads/{name}` and SHALL rewrite the volume so its contents equal that branch's commit tree exactly.

- **Inputs:** `mount_id`, `name`.
- **Outputs:** exactly `{branch, sha, changed, files_changed}`, where `changed` is a boolean stating whether the volume was rewritten and `files_changed` is the count of paths written plus removed.
- **Business Rules:** Files present only on the source branch are removed; files present only on the target are created; files differing are overwritten. The rewrite is atomic: a failure part-way leaves the volume as it was. Bytes written are charged to the write quota and audited.
- **Priority:** Must-have

### FR-NEW-106 [EARS-O]: Refuse to switch with a dirty volume
> IF the volume holds uncommitted changes relative to HEAD THEN THE mcp-fs server SHALL reject `git.branch_switch` with `ERR_INVALID_ARGUMENT` naming `git.commit` and `git.stash_save` as remedies, and SHALL leave the volume and HEAD untouched.

- **Business Rules:** "Dirty" means any modified, added or deleted path relative to HEAD, matching the existing pull dirty check.
- **Priority:** Must-have

### FR-NEW-107 [EARS-O]: Switching to the current branch is a no-op
> IF `name` is already the checked-out branch THEN THE mcp-fs server SHALL return success without writing to the volume and with `files_changed` both zero.

- **Priority:** Should-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

14 tests: Concurrency 1, EdgeCase 1, Failure 6, Happy 1, SideEffect 5.
Scenarios covered: SC-901, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-401 — Happy — create with checkout rewrites HEAD and the volume**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-101, FR-NEW-105.
Preconditions: SEED-A. `/src/lib.rs` exists only in `<sha-C2>`.
- **Given** the volume holds `/README.md` = `"alpha\n"` and `/src/lib.rs` = `"fn a() {}\n"`.
- **When** `git.branch_create {"mount_id":"gitproj","name":"hotfix/c1","start_point":"<sha-C1>","checkout":true}`.
- **Then** the response has `"checked_out": true` and `"sha":"<sha-C1>"`.
- **And** `db.get_ref("HEAD")` = `{ target:"refs/heads/hotfix/c1", symbolic:true }`.
- **And** a follow-up `git.status` returns `"branch":"hotfix/c1"` and `"head":"<sha-C1>"`.
- **And** `env.read("/README.md")` == `"alpha\n"` and `client.exists("/src/lib.rs")` is `false` (volume inspection via `env.f.state.stores.client(MOUNT)`).
Priority: P0.

---

#### E2E-NEW-408 — Failure — create+checkout is refused on a dirty volume**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-106, FR-NEW-101.
Preconditions: SEED-A, then `env.write("/README.md", "alpha MODIFIED\n")` (uncommitted).
- **When** `git.branch_create {"name":"feature/dirty","start_point":"<sha-C1>","checkout":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("uncommitted changes")`.
- **And** `env.read("/README.md") == "alpha MODIFIED\n"` (the caller's work is intact).
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
Verification: the dirty test reuses `require_clean_volume` (`crates/mcp-fs/src/tools/git.rs:1991-2009`); the message MUST be the branch-specific one, not the string `"pull refused"`, so the test also asserts `!message.contains("pull refused")`.
Priority: P0.

---

#### E2E-NEW-409 — SideEffect — the rejected create of E2E-NEW-408 leaves nothing behind**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-106, FR-NEW-101.
Preconditions: identical to E2E-NEW-408, run to its failure.
- **Then** `db.get_ref("refs/heads/feature/dirty")` is `None` (direct row check: no branch was created before the dirty check ran).
- **And** `safety.audit(OWNER, MOUNT)` contains no entry with `op == "git.branch_create"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured immediately before the call.
Priority: P0.

---

#### E2E-NEW-414 — SideEffect — switch rewrites the volume to the target tree**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105., FR-NEW-189
Preconditions: SEED-B; before the switch the volume holds `/README.md`=`"alpha\n"`, `/src/lib.rs`=`"fn a() {}\n"`, and no `/docs/rel.md`.
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `env.read("/docs/rel.md") == "rel\n"` (a path present only on the target).
- **And** `client.exists("/src/lib.rs")` is `false` (a path present only on the source is deleted).
- **And** `env.read("/README.md") == "alpha\n"` (a path common to both is untouched).
- **And** switching back with `git.branch_switch {"name":"main"}` restores `/src/lib.rs` to `"fn a() {}\n"` and removes `/docs/rel.md`.
Priority: P0.

---

#### E2E-NEW-415 — Failure — switch refuses a dirty volume and touches nothing**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-106.
Preconditions: SEED-B, then `env.write("/src/lib.rs", "fn a() { todo!() }\n")` (uncommitted).
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("uncommitted changes")` and `message.contains("release/1.0")`.
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
- **And** `env.read("/src/lib.rs") == "fn a() { todo!() }\n"` (the uncommitted edit survives).
- **And** `client.exists("/docs/rel.md")` is `false` (no partial checkout).
Priority: P0.

---

#### E2E-NEW-416 — Failure — switch to an unknown branch**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: SEED-A.
- **When** `git.branch_switch {"name":"release/9.9"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("branch 'release/9.9'")`.
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
Priority: P0.

---

#### E2E-NEW-417 — EdgeCase — switching to the current branch is a no-op**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-107.
Preconditions: SEED-A, volume clean, `let before = safety.bytes_written(OWNER, MOUNT);`.
- **When** `git.branch_switch {"name":"main"}`.
- **Then** the response is `{"branch":"main","sha":"<sha-C2>","changed":false,"files_changed":0}`.
- **And** `safety.bytes_written(OWNER, MOUNT) == before` (no quota charged for a no-op).
- **And** `env.read("/README.md") == "alpha\n"` and `env.read("/src/lib.rs") == "fn a() {}\n"`.
Priority: P1.

---

#### E2E-NEW-418 — Failure — switch under an exhausted quota**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: `Env::with_quota(N)` where `N` is sized to admit SEED-B's commits but not the checkout writes; concretely build SEED-B under `Env::with_quota(i64::MAX)` first is not possible in one env, so: build SEED-B under a normal env, then drain the quota by constructing the env with `Env::with_quota(60)` and performing SEED-B (whose committed bytes are `"alpha\n"`=6 + `"fn a() {}\n"`=10 + `"rel\n"`=4 = 20 written by `env.write`), then call switch with only 3 bytes of headroom remaining. The test asserts the pre-call `bytes_written` value explicitly so the headroom is not guessed.
- **When** `git.branch_switch {"name":"release/1.0"}` (which must write `/docs/rel.md`, 4 bytes).
- **Then** `code == ERR_WRITE_QUOTA_EXCEEDED` and `message.contains("session write quota of 60 bytes exceeded")` (the exact wording of `charge_write`, `crates/mcp-fs/src/safety.rs:136-138`).
Priority: P0.

---

#### E2E-NEW-419 — SideEffect — the quota-refused switch of E2E-NEW-418 left the volume untouched**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105, FR-NEW-184.
Preconditions: E2E-NEW-418 run to its failure.
- **Then** `client.exists("/docs/rel.md")` is `false`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (not deleted).
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the pre-call value (a rejected charge consumes nothing, `crates/mcp-fs/src/safety.rs:131-143`).
Rationale: the quota must be charged in a pre-flight pass, like `charge_pull_quota` + `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:1967-1989`, `:2079-2127`).
Priority: P0.

---

#### E2E-NEW-420 — SideEffect — a successful switch charges quota and writes one audit entry**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: SEED-B, `let before = safety.bytes_written(OWNER, MOUNT);`.
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `safety.bytes_written(OWNER, MOUNT) - before == 4` (only `/docs/rel.md`'s `"rel\n"` is written; the deletion of `/src/lib.rs` adds nothing, matching `charge_pull_quota`).
- **And** `safety.audit(OWNER, MOUNT)` ends with exactly one entry where `op == "git.branch_switch"`, `path == "/"`, and `detail.contains("release/1.0")` and `detail.contains("files_changed 2")`.
Priority: P0.

---

#### E2E-NEW-421 — Concurrency — two switches race**

**Category:** Concurrency. **Scenario:** SC-901. **Requirements:** FR-NEW-115.
Preconditions: SEED-B plus a third branch `feature/z` created from `<sha-C1>` with an extra committed file `/z.txt`=`"z\n"` -> `<sha-C4>`; HEAD on `main`, volume clean.
- **When** `tokio::join!` of `git.branch_switch {"name":"release/1.0"}` and `git.branch_switch {"name":"feature/z"}` on the same `Env`.
- **Then** both calls return `Ok` (or one returns `ERR_INVALID_ARGUMENT` "uncommitted changes" if it observes the other's intermediate tree; both outcomes are accepted and the test asserts the set of outcomes is one of those two).
- **And** the final `db.get_ref("HEAD").target` is exactly one of `"refs/heads/release/1.0"` or `"refs/heads/feature/z"`.
- **And** the volume matches that branch's tree exactly: if `release/1.0`, `/docs/rel.md`=="rel\n" and `client.exists("/z.txt")==false`; if `feature/z`, `/z.txt`=="z\n" and `client.exists("/docs/rel.md")==false`. No mixed state is tolerated.
Verification: relies on `entry.write_lock` (`crates/mcp-fs/src/git/repo.rs:44`), which `branch_switch` MUST hold for the whole checkout, exactly as `commit` does (`crates/mcp-fs/src/tools/git.rs:659`).
Priority: P1.

---

#### E2E-NEW-529 — git.branch_switch blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. Same preconditions.
**When** `git.branch_switch {mount_id:"proj", branch:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`, `git.merge_abort`.
**And** `HEAD` still symbolic to `refs/heads/main`; `/src/config.toml` still `port = 8000`.
**Verification:** error code + substrings; `entry.db.get_ref("HEAD")` symbolic target; file read.

#### E2E-NEW-820 — SideEffect — the no-op switch writes nothing at all**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-107.
- **Preconditions:** SEED-A, HEAD on `main`; capture `before_audit` (full `Vec<AuditEntry>`) and `before_bytes`.
- **When** `git.branch_switch {"mount_id":"gitproj","name":"main"}`.
- **Then** the response is exactly `{"branch":"main","sha":"<sha-C2>","changed":false,"files_changed":0}`.
- **And** `state.safety.audit(OWNER, MOUNT)` is equal element-for-element to `before_audit` — not even a zero-byte `git.branch_switch` entry.
- **And** `bytes_written == before_bytes`.
- **And** the modification time recorded for `/README.md` in the volume metadata is unchanged (the file was not rewritten with identical bytes).
- **Verification:** exact response equality; audit vector equality; quota equality; node metadata read via `state.stores.client(MOUNT)`.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-821 — Failure — the dirty check runs before the no-op shortcut**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-107, FR-NEW-106.
- **Preconditions:** SEED-A, HEAD on `main`; `fs.write_text {path:"/README.md", content:"alpha-DIRTY\n"}`.
- **When** `git.branch_switch {"name":"main"}` (switching to the branch already checked out, on a dirty volume).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"git.commit"` and `"git.stash_save"` — the no-op shortcut does not bypass the dirty guard, so the caller gets one consistent rule.
- **And** `/README.md` still reads exactly `b"alpha-DIRTY\n"` (the dirt is preserved, not silently discarded by a "harmless" rewrite).
- **And** `db.get_ref("HEAD")` is unchanged and `state.safety.audit(OWNER, MOUNT)` holds no `git.branch_switch` entry.
- **Verification:** error assertion; byte compare; ref read; audit scan.
- **Cleanup:** fixture drop. **Priority:** P1.

---

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
