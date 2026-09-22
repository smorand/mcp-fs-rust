# US-010: Branch delete, branch_reset and tracking divergence

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 10
> Depends On: US-009
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Complete the branch lifecycle: delete a branch, force-move a branch pointer to an arbitrary commit, and report how far each branch is ahead of or behind its remote-tracking ref.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Ref reads and writes go through `RelationalGitDb` (`get_ref` `crates/mcp-fs/src/git/db.rs:214-234`, `list_refs` `:261`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

8 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-108 [EARS-E]: Delete a branch
> WHEN `git.branch_delete` is called with `name` THE mcp-fs server SHALL remove ref `refs/heads/{name}`.

- **Inputs:** `mount_id`, `name`, `force` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, sha, forced}`, where `sha` is the tip the deleted branch pointed at and `forced` reports whether `force` was required.
- **Business Rules:** The commits become unreachable but are not destroyed; no object is pruned by this operation.
- **Priority:** Must-have

### FR-NEW-109 [EARS-O]: Refuse to delete the checked-out branch
> IF `name` is the currently checked-out branch THEN THE mcp-fs server SHALL reject `git.branch_delete` with `ERR_INVALID_ARGUMENT` stating that the caller must switch away first.

- **Priority:** Must-have

### FR-NEW-110 [EARS-O]: Refuse to delete an unmerged branch without force
> IF `name` holds commits reachable from no other ref and `force` is `false` THEN THE mcp-fs server SHALL reject `git.branch_delete` with `ERR_INVALID_ARGUMENT` naming `force` as the override and naming each commit that the deletion leaves unreachable.

- **Priority:** Must-have

### FR-NEW-111 [EARS-E]: Force-move a branch pointer
> WHEN `git.branch_reset` is called with `name` and `target_commit` THE mcp-fs server SHALL set ref `refs/heads/{name}` to that commit.

- **Inputs:** `mount_id`, `name`, `target_commit`, `force` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, old_sha, new_sha, checked_out, files_changed}`, where `checked_out` states whether this branch was the checked-out one and therefore whether the volume was rewritten.
- **Business Rules:** This is the "replug" operation equivalent to `git branch -f`. `old_sha` is always reported so the prior tip is recoverable.
- **Priority:** Must-have

### FR-NEW-112 [EARS-O]: Branch-reset rewrites the volume only for the checked-out branch
> IF `name` is the currently checked-out branch THEN THE mcp-fs server SHALL additionally rewrite the volume to `target_commit`'s tree and SHALL report `checked_out` as `true`; otherwise it SHALL leave the volume untouched and report `checked_out` as `false`.

- **Priority:** Must-have

### FR-NEW-113 [EARS-O]: Refuse a non-fast-forward branch move without force
> IF the current tip of `name` is not an ancestor of `target_commit` and `force` is `false` THEN THE mcp-fs server SHALL reject `git.branch_reset` with `ERR_INVALID_ARGUMENT` naming `force` and naming each commit that the move leaves orphaned.

- **Priority:** Must-have

### FR-NEW-114 [EARS-O]: Reject an unknown branch-reset target
> IF `target_commit` resolves to no commit present in this volume's object store THEN THE mcp-fs server SHALL reject `git.branch_reset` with `ERR_NOT_FOUND` and SHALL NOT move the ref.

- **Priority:** Must-have

### FR-MOD-106 [EARS-E]: `git.branches` reports the current branch and tracking divergence
> WHEN `git.branches` is called THE mcp-fs server SHALL mark which branch is currently checked out and SHALL report, for each branch with a remote-tracking ref, how many commits it is ahead of and behind that ref.

- **Original behavior:** A flat list of branch names and shas.
- **Business Rules:** A branch with no remote-tracking ref reports `null` for both counts, which is distinguishable from `0`.
- **Priority:** Should-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

14 tests: DataIntegrity 1, EdgeCase 2, Failure 4, Happy 3, SideEffect 4.
Scenarios covered: SC-901, SC-924.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-422 — Happy — delete a merged branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-108., FR-NEW-189
Preconditions: SEED-A, then `git.branch_create {"name":"feature/merged","start_point":"<sha-C1>"}` (an ancestor of `main`, therefore merged).
- **When** `git.branch_delete {"mount_id":"gitproj","name":"feature/merged"}`.
- **Then** the response is `{"branch":"feature/merged","sha":"<sha-C1>","forced":false}`.
- **And** `db.get_ref("refs/heads/feature/merged")` is `None`.
- **And** `git.branches` lists only `main`.
Priority: P0.

---

#### E2E-NEW-423 — Failure — the checked-out branch cannot be deleted**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-109., FR-NEW-189
Preconditions: SEED-A, HEAD on `main`.
- **When** `git.branch_delete {"name":"main"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("'main' is the checked-out branch")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
Priority: P0.

---

#### E2E-NEW-426 — DataIntegrity — a force delete removes the ref, never the objects**

**Category:** DataIntegrity. **Scenario:** SC-901. **Requirements:** FR-NEW-108.
Preconditions: E2E-NEW-425 run to success.
- **Then** `git.show {"commit_sha":"<sha-C3>"}` still returns `commit.sha == "<sha-C3>"` and a non-empty `"diff"` (tool response).
- **And** `db.get_object("<sha-C3>")` returns `Some(row)` with `row.kind == "commit"` (direct `git_objects` check, `crates/mcp-fs/src/git/db.rs:143`).
Priority: P1.

---

#### E2E-NEW-427 — Failure — delete an unknown branch**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-108.
Preconditions: SEED-A.
- **When** `git.branch_delete {"name":"feature/ghost"}` and again with `{"force":true}`.
- **Then** both error with `code == ERR_NOT_FOUND` and `message.contains("branch 'feature/ghost'")` (force does not turn a missing branch into a success).
Priority: P1.

---

#### E2E-NEW-428 — SideEffect — the rejected deletes of E2E-NEW-423 and E2E-NEW-424 left every ref intact**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-109, FR-NEW-110.
Preconditions: SEED-B; run E2E-NEW-423's call and E2E-NEW-424's call in the same test, both to failure.
- **Then** `db.list_refs()` returns exactly the names `["HEAD","refs/heads/main","refs/heads/release/1.0"]`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
- **And** `safety.audit(OWNER, MOUNT)` contains no `op == "git.branch_delete"` entry.
Priority: P0.

---

#### E2E-NEW-431 — SideEffect — resetting another branch never touches the volume**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-112., FR-NEW-189
Preconditions: E2E-NEW-430 run to success; `let before = safety.bytes_written(OWNER, MOUNT);` captured before the reset.
- **Then** `env.read("/README.md") == "alpha\n"` and `env.read("/src/lib.rs") == "fn a() {}\n"` (the `main` tree, unchanged).
- **And** `client.exists("/docs/rel.md")` is `false` (it never was in the `main` tree).
- **And** `safety.bytes_written(OWNER, MOUNT) == before` (no volume write, so no charge).
- **And** `safety.audit(OWNER, MOUNT)` has exactly one `op == "git.branch_reset"` entry with `detail.contains("<sha-C3>")` and `detail.contains("<sha-C1>")`.
Priority: P0.

---

#### E2E-NEW-435 — SideEffect — the hard reset of E2E-NEW-434 removed the file and was accounted**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-112., FR-NEW-189
Preconditions: E2E-NEW-434 run to success, with `before = safety.bytes_written(OWNER, MOUNT)` captured first.
- **Then** `client.exists("/src/lib.rs")` is `false` and `env.read("/README.md") == "alpha\n"`.
- **And** `safety.bytes_written(OWNER, MOUNT) - before == 0` (only a delete, no blob bytes written, per `charge_pull_quota` semantics).
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `op == "git.branch_reset"` entry with `path == "/"` and `detail.contains("files_changed 1")`.
Priority: P0.

---

#### E2E-NEW-436 — Failure — an unknown target commit**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-114.
Preconditions: SEED-A.
- **When** `git.branch_reset {"name":"main","target_commit":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef","force":true}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("deadbeef")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (no volume rewrite was attempted).
Priority: P1.

---

#### E2E-NEW-437 — Failure — resetting an unknown branch**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-114.
Preconditions: SEED-A.
- **When** `git.branch_reset {"name":"feature/ghost","target_commit":"<sha-C1>","force":true}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("branch 'feature/ghost'")`.
- **And** `db.list_refs()` contains no `refs/heads/feature/ghost` row (a reset never creates a branch).
Priority: P1.

---

#### E2E-NEW-439 — SideEffect — the rejected resets of E2E-NEW-432 and E2E-NEW-438 left the refs untouched**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-113, FR-NEW-114.
Preconditions: run both calls in a single test built on SEED-B plus a dirty `/README.md`.
- **Then** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"` and `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
- **And** `safety.audit(OWNER, MOUNT)` contains no `op == "git.branch_reset"` entry.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before both calls.
Priority: P0.

---

#### E2E-NEW-442 — Happy — `git.branches` marks exactly one current branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the `"branches"` array has 2 entries; the entry with `"name":"main"` has `"current": true`, the entry with `"name":"release/1.0"` has `"current": false`.
- **And** `branches.iter().filter(|b| b["current"] == true).count() == 1`.
- **And** after `git.branch_switch {"name":"release/1.0"}`, the same call reports `"current":true` on `release/1.0` and `false` on `main`.
Priority: P0.

---

#### E2E-NEW-443 — Happy — ahead/behind vs the remote-tracking ref**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-A, then seed the tracking ref and divergence directly through the db and real commits:
`db.set_ref("refs/remotes/origin/main", "<sha-C1>", false)`; then two more local commits: write `/a.txt`=`"a\n"`, commit "c3" -> `<sha-C3>`; write `/b.txt`=`"b\n"`, commit "c4" -> `<sha-C4>`. `main` = `<sha-C4>`, tracking = `<sha-C1>`, and `<sha-C2>`,`<sha-C3>`,`<sha-C4>` are the three commits ahead. For the "behind" side, build the tracking ref from a sibling branch instead: create `feature/upstream` from `<sha-C1>`, commit `/u.txt`=`"u\n"` -> `<sha-U2>`, then `db.set_ref("refs/remotes/origin/main","<sha-U2>",false)`.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the `main` entry has `"upstream":"refs/remotes/origin/main"`, `"ahead": 3`, `"behind": 1` (commits `<sha-C2>`,`<sha-C3>`,`<sha-C4>` are on `main` only; `<sha-U2>` is on the tracking ref only; `<sha-C1>` is the merge base).
Verification: tool response fields, cross-checked against `git.log {"ref_name":"main"}` length (4 commits including `<sha-C1>`).
Priority: P0.

---

#### E2E-NEW-444 — EdgeCase — a branch with no upstream reports null, not zero**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-B, no `refs/remotes/*` rows at all (`db.list_refs()` asserted to contain none).
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** both entries have `"upstream": null`, `"ahead": null`, `"behind": null` (JSON `null`, asserted with `v["ahead"].is_null()`, explicitly NOT `0`).
Rationale: `0/0` would mean "in sync", which is a materially different statement from "no upstream".
Priority: P0.

---

#### E2E-NEW-445 — EdgeCase — `git.branches` on a repo with no commits**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: `git.init` only.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the response is `{"mount_id":"gitproj","branches":[]}` (an empty array, not an error).
- **And** `git.status` returns `"branch":"main"` with `"head": null` (existing behaviour, `status`, `crates/mcp-fs/src/tools/git.rs:512-538`), proving the current-branch marker has nothing to mark.
Priority: P1.

---

##### B) Stash

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
