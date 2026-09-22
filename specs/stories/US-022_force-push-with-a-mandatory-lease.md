# US-022: Force push with a mandatory lease

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 22
> Depends On: US-021
> Complexity: L
> min_tier: 2
> Files touched: 2

## Objective

Allow a deliberate non-fast-forward push, gated behind a mandatory lease. The caller must state the remote sha it believes is current; if the remote has moved since, the push is refused and nothing is overwritten. This is stricter than git, where a bare `--force` is allowed.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/remote.rs
```

### Existing Patterns
The fast-forward refusal being extended is `crates/mcp-fs/src/git/remote.rs:744-748` (`non_fast_forward_error`); its message tail still says force is not supported and must change. Audit entries are written once per remote operation by the wrapper at `crates/mcp-fs/src/git/remote.rs:326-371`.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-903:** Force push requires a mandatory lease (`expected_remote_sha`); a bare `force: true` is rejected. **Rationale:** the user asked for a "double check". A warning string in a tool description depends on a model reading it; a lease mechanically prevents overwriting a remote that moved since the caller last looked. This is stricter than git, where `--force` without a lease is allowed. **Alternatives considered:** a bare boolean plus a loud description (rejected as unenforceable); protected-branch pattern matching from the archived DEC-013 recommendation (not adopted as the primary mechanism, since the lease covers the actual race). **Implemented by:** FR-NEW-155, FR-NEW-156, FR-NEW-157, FR-NEW-158, FR-NEW-159. **Round:** 1 (Q3, P1). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:315` and `crates/mcp-fs/src/git/remote.rs:746` (force refused today).

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: force push is leased and audited with the sha it destroyed.

### Bounded Context
**Remote** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

6 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-155 [EARS-E]: Force push updates a diverged remote ref
> WHEN `git.remote_push` is called with `force` `true` and an `expected_remote_sha` equal to the remote branch's current tip THE mcp-fs server SHALL update the remote ref to the local tip even though the update is not a fast-forward.

- **Inputs:** `mount_id`, `branch`, `force` (boolean, default `false`), `expected_remote_sha` (string), optional `remote`, optional `remote_branch` (the `local:remote` refspec form).
- **Outputs:** `{branch, remote, remote_branch, forced: true, overwritten_sha, new_sha}`.
- **Priority:** Must-have

### FR-NEW-156 [EARS-O]: The lease is mandatory when forcing
> IF `force` is `true` and `expected_remote_sha` is absent or empty THEN THE mcp-fs server SHALL reject `git.remote_push` with `ERR_INVALID_ARGUMENT` stating that a force push requires the expected remote sha, and SHALL NOT contact the remote.

- **Business Rules:** This is the whole safety design. A bare force flag is not accepted (DEC-903).
- **Priority:** Must-have

### FR-NEW-157 [EARS-O]: A stale lease rejects the push
> IF `expected_remote_sha` differs from the remote branch's actual current tip THEN THE mcp-fs server SHALL reject `git.remote_push` with an error naming both the expected sha and the actual sha, and SHALL leave the remote ref unchanged.

- **Priority:** Must-have

### FR-NEW-158 [EARS-O]: A lease without force is contradictory
> IF `expected_remote_sha` is supplied and `force` is `false` THEN THE mcp-fs server SHALL reject `git.remote_push` with `ERR_INVALID_ARGUMENT` rather than silently ignoring the parameter.

- **Business Rules:** Silently ignoring a lease would let a caller believe it had protection it did not have.
- **Priority:** Must-have

### FR-NEW-159 [EARS-E]: A forced push is audited with the sha it destroyed
> WHEN a forced push succeeds THE mcp-fs server SHALL write an audit entry recording the operation, the remote, the branch and the overwritten sha.

- **Business Rules:** The overwritten sha is the only server-side record of what was on the remote, making recovery from the provider's reflog possible.
- **Priority:** Must-have

### FR-NEW-160 [EARS-UB]: Force never applies implicitly
> The mcp-fs server SHALL NOT perform a non-fast-forward push when `force` is absent or `false`, and SHALL retain the existing refusal behaviour and error for that case.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

28 tests: Concurrency 1, EdgeCase 8, Failure 9, Happy 5, SideEffect 5.
Scenarios covered: SC-901, SC-908, SC-909, SC-910, SC-924, SC-930.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-413 — Happy — switch repoints HEAD**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-105., FR-NEW-189
Preconditions: SEED-B (HEAD on `main` = `<sha-C2>`; `release/1.0` = `<sha-C3>`), volume clean.
- **When** `git.branch_switch {"mount_id":"gitproj","name":"release/1.0"}`.
- **Then** the response is `{"branch":"release/1.0","sha":"<sha-C3>","changed":true,"files_changed":2}`.
- **And** `db.get_ref("HEAD")` = `{ target:"refs/heads/release/1.0", symbolic:true }`.
- **And** `git.status` returns `"branch":"release/1.0"`, `"head":"<sha-C3>"`.
Priority: P0.

---

#### E2E-NEW-424 — Failure — an unmerged branch needs force**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-110.
Preconditions: SEED-B (`release/1.0` = `<sha-C3>`, not reachable from `main` = `<sha-C2>`), HEAD on `main`.
- **When** `git.branch_delete {"name":"release/1.0"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("release/1.0")`, `message.contains("not merged")`, and `message.contains("force")`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
Priority: P0.

---

#### E2E-NEW-425 — Happy — force deletes an unmerged branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-108, FR-NEW-110.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branch_delete {"name":"release/1.0","force":true}`.
- **Then** the response is `{"branch":"release/1.0","sha":"<sha-C3>","forced":true}`.
- **And** `db.get_ref("refs/heads/release/1.0")` is `None`.
- **And** `git.branches` lists only `main`.
Priority: P0.

---

#### E2E-NEW-429 — EdgeCase — force does not defeat the self-delete guard**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-109.
Preconditions: SEED-A, HEAD on `main`.
- **When** `git.branch_delete {"name":"main","force":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("checked-out branch")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("HEAD").target == "refs/heads/main"`.
Priority: P1.

---

#### E2E-NEW-430 — Happy — reset a non-checked-out branch backwards**

**Category:** Happy. **Scenario:** SC-924. **Requirements:** FR-NEW-111., FR-NEW-189
Preconditions: SEED-B, HEAD on `main`. `release/1.0` = `<sha-C3>`, whose parent is `<sha-C1>`.
- **When** `git.branch_reset {"mount_id":"gitproj","name":"release/1.0","target_commit":"<sha-C1>","force":true}`.
- **Then** the response is `{"branch":"release/1.0","old_sha":"<sha-C3>","new_sha":"<sha-C1>","checked_out":false,"files_changed":0}`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C1>"`.
- **And** `git.log {"ref_name":"release/1.0"}` returns exactly one commit whose sha is `<sha-C1>`.
Priority: P0.

---

#### E2E-NEW-432 — Failure — a non-fast-forward reset without force names both shas**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-113.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C1>"}` (force omitted; `<sha-C1>` is an ancestor, so the move is a rewind, not a fast-forward).
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("not a fast-forward")`, `message.contains("<sha-C3>")` and `message.contains("<sha-C1>")`, and `message.contains("force")`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
Priority: P0.

---

#### E2E-NEW-433 — EdgeCase — a fast-forward reset needs no force**

**Category:** EdgeCase. **Scenario:** SC-924. **Requirements:** FR-NEW-113.
Preconditions: SEED-A, then `git.branch_create {"name":"feature/behind","start_point":"<sha-C1>"}`. `<sha-C2>` is a descendant of `<sha-C1>`.
- **When** `git.branch_reset {"name":"feature/behind","target_commit":"<sha-C2>"}` (force omitted).
- **Then** the call succeeds with `"old_sha":"<sha-C1>","new_sha":"<sha-C2>"`.
- **And** `db.get_ref("refs/heads/feature/behind").target == "<sha-C2>"`.
Priority: P0.

---

#### E2E-NEW-434 — Happy — resetting the checked-out branch hard-resets the volume**

**Category:** Happy. **Scenario:** SC-924. **Requirements:** FR-NEW-111, FR-NEW-112.
Preconditions: SEED-A, HEAD on `main` = `<sha-C2>`, volume clean.
- **When** `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}`.
- **Then** the response has `"checked_out": true`, `"old_sha":"<sha-C2>"`, `"new_sha":"<sha-C1>"`, `"files_changed":1`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C1>"` and `db.get_ref("HEAD").target == "refs/heads/main"` (HEAD stays symbolic on the same branch).
- **And** `git.status` reports `"head":"<sha-C1>"`, `"branch":"main"`.
Priority: P0.

---

#### E2E-NEW-438 — Failure — resetting the checked-out branch with a dirty volume**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-112, FR-NEW-106.
Preconditions: SEED-A, then `env.write("/README.md","alpha DIRTY\n")`.
- **When** `git.branch_reset {"name":"main","target_commit":"<sha-C1>"}` (force omitted).
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("uncommitted changes")`.
- **And** `env.read("/README.md") == "alpha DIRTY\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
Note: the dirty guard is checked before the fast-forward guard, so the message must be the dirty one; the test asserts `!message.contains("not a fast-forward")` to pin the ordering.
Priority: P0.

---

#### E2E-NEW-465 — EdgeCase — a stash taken on one branch applies on another**

**Category:** EdgeCase. **Scenario:** SC-930. **Requirements:** FR-NEW-129.
Preconditions: SEED-B (branches `main` and `release/1.0`), HEAD on `main`; `env.write("/shared.txt","s\n")`; `git.stash_save {"message":"cross"}` -> `<sha-S1>`; `git.branch_switch {"name":"release/1.0"}`.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}`.
- **Then** the response has `"status":"applied"` and `"files_changed":1`.
- **And** `env.read("/shared.txt") == "s\n"` while `git.status` still reports `"branch":"release/1.0"`.
- **And** `env.read("/docs/rel.md") == "rel\n"` (the target branch's own content is preserved).
Priority: P1.

---

#### E2E-NEW-489 — Failure — `force:true` without `expected_remote_sha` is refused outright**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Preconditions: SEED-R. Driven through the registered `git.remote_push` tool so the rejection is proven to happen before any credential resolution or network call; the URL is the https placeholder `https://github.com/o/r.git` stored as origin, and the test asserts the error is the lease one, not the scheme one.
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","force":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("expected_remote_sha is required when force is true")`.
- **And** `assert!(!e.message.contains("only https is accepted"))` (proving the lease check runs ahead of URL validation, i.e. ahead of any network attempt).
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` (no tracking ref written).
Priority: P0.

---

#### E2E-NEW-490 — Happy — a matching lease permits a non-fast-forward push**

**Category:** Happy. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Preconditions: SEED-R; `let sha_r2 = advance_bare_remote(&remote_dir, "refs/heads/main", "remote-only\n");` so the bare repo's `main` is `<sha-R2>` and the local `main` (`<sha-C2>`) is NOT a descendant of it. Driven at the internal-function level.
- **Given** bare-repo inspection confirms `refs/heads/main` == `<sha-R2>`.
- **When** the push runs with `force:true, expected_remote_sha:"<sha-R2>"`.
- **Then** the response is `{"branch":"main","created":false,"up_to_date":false,"forced":true,"remote_sha":"<sha-C2>","overwritten_sha":"<sha-R2>"}`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-C2>`.
Priority: P0.

---

#### E2E-NEW-491 — SideEffect — the audit entry records the overwritten sha**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-159.
Preconditions: E2E-NEW-490 run to success.
- **Then** `safety.audit(OWNER, MOUNT)` contains exactly one entry with `op == "git.remote_push"`, `path == "/"`, whose `detail` satisfies all of: `contains("outcome ok")`, `contains("<sha-R2>")` (the overwritten sha), `contains("<sha-C2>")` (the new sha), `contains("forced true")`.
- **And** `assert!(!detail.contains("ghp_"))` and the detail contains no `"@"`-bearing URL (no credential leak, consistent with `crates/mcp-fs/src/git/remote.rs:326-333`).
- **And** exactly one `git.remote_push` entry exists, not two (the single-audit invariant of `run_remote_operation`).
Priority: P0.

---

#### E2E-NEW-492 — Failure — a stale lease is rejected naming both shas**

**Category:** Failure. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: SEED-R; capture `<sha-R1>` (the remote tip the caller believes in); then `let sha_r2 = advance_bare_remote(&remote_dir, "refs/heads/main", "someone-else\n");` so the remote has moved to `<sha-R2>` behind the caller's back. Driven at the internal-function level.
- **When** the push runs with `force:true, expected_remote_sha:"<sha-R1>"`.
- **Then** `code == ERR_INVALID_ARGUMENT`.
- **And** `message.contains("<sha-R1>")` AND `message.contains("<sha-R2>")` AND `message.contains("expected")` AND `message.contains("actual")` (both shas, labelled).
- **And** `assert!(!message.starts_with("push refused: not a fast-forward"))` (the lease failure is a distinct identity from the FF refusal at `crates/mcp-fs/src/git/remote.rs:744-748`).
Priority: P0.

---

#### E2E-NEW-493 — SideEffect — a rejected lease leaves the remote and the tracking ref untouched**

**Category:** SideEffect. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: E2E-NEW-492 run to its failure, with `db.set_ref("refs/remotes/origin/main","<sha-R1>",false)` seeded before the call.
- **Then** bare-repo inspection: `refs/heads/main` in `remote_dir` == `<sha-R2>`, byte-for-byte what the third party left (no partial ref update, no new ref).
- **And** the bare repo contains no reference whose target is `<sha-C2>`: iterate `repo.references()` and assert none resolves to `<sha-C2>`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` (unchanged: the tracking ref is only advanced after a successful push, `crates/mcp-fs/src/tools/git.rs:1257-1259`).
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and the volume is unchanged.
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `git.remote_push` entry with `detail.contains("outcome error")`.
Priority: P0.

---

#### E2E-NEW-494 — Failure — `force:false` preserves today's fast-forward refusal verbatim**

**Category:** Failure. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
Preconditions: SEED-R; `advance_bare_remote(&remote_dir, "refs/heads/main", "extra\n")` -> `<sha-R2>` (the exact setup of the existing `e2e_new_076` test, `crates/mcp-fs/src/tools/git.rs:4445-4460`). Driven at the internal-function level.
- **When** the push runs with `force` omitted entirely, and again with `force:false`.
- **Then** both error with `code == ERR_INVALID_ARGUMENT` and `message == "push refused: not a fast-forward: branch 'main', force is not supported"` is NOT asserted verbatim (the tail changes once force exists); instead assert `message.starts_with("push refused: not a fast-forward")` and `message.contains("branch 'main'")`, matching the frozen prefix asserted at `crates/mcp-fs/src/git/remote.rs:1163-1165`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R2>`, unchanged.
- **And** `db.get_ref("refs/remotes/origin/main")` is `None`.
Note for the implementer: `non_fast_forward_error` (`crates/mcp-fs/src/git/remote.rs:744-748`) currently ends with `"force is not supported"`. That clause becomes false once force exists and MUST be replaced (suggested: `"pass force with expected_remote_sha to overwrite"`), which requires updating the two existing assertions at `crates/mcp-fs/src/git/remote.rs:1165` and `crates/mcp-fs/src/tools/git.rs:4455` — both only assert the prefix, so both keep passing.
Priority: P0.

---

#### E2E-NEW-495 — EdgeCase — the "must not exist" lease, 40 zeros**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Preconditions: SEED-R; the bare repo has `refs/heads/main` == `<sha-R1>` but no `refs/heads/sandbox`. Driven at the internal-function level.
- **When** the push runs with `branch:"main", remote_branch:"sandbox", force:true, expected_remote_sha:"0000000000000000000000000000000000000000"`.
- **Then** the response is `{"created":true,"forced":true,"remote_sha":"<sha-C2>","overwritten_sha":null}`.
- **And** bare-repo inspection: `refs/heads/sandbox` == `<sha-C2>`.
- **And** a second run of the same call now errors `ERR_INVALID_ARGUMENT` with `message.contains("0000000000000000000000000000000000000000")` and `message.contains("<sha-C2>")`, because the branch now exists and the zero lease no longer matches.
- **And** after that second, rejected call, `refs/heads/sandbox` is still `<sha-C2>`.
Priority: P1.

---

#### E2E-NEW-496 — Failure — a malformed `expected_remote_sha`**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Preconditions: SEED-R. Driven through the registered tool (no network reached).
- **When** `git.remote_push {"branch":"main","force":true,"expected_remote_sha": X}` for X in `["", "deadbeef", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", "<sha-C2> "]` (the last has a trailing space).
- **Then** each errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("expected_remote_sha")` and `message.contains("40")`.
- **And** for the `"deadbeef"` case, `assert!(!e.message.contains("only https is accepted"))` (validated before URL handling, so before the network).
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R1>` after all four calls.
Priority: P1.

---

#### E2E-NEW-497 — Failure — a lease supplied without force**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-158.
Preconditions: SEED-R.
- **When** `git.remote_push {"branch":"main","force":false,"expected_remote_sha":"<sha-R1>"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("expected_remote_sha")` and `message.contains("force")`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R1>`, unchanged.
Rationale: silently ignoring a lease would let a caller believe it had protection it did not have.
Priority: P2.

---

#### E2E-NEW-498 — Concurrency — the remote moves between the lease read and the push**

**Category:** Concurrency. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: SEED-R; the local `main` = `<sha-C2>`; the bare repo's `main` = `<sha-R1>`. Driven at the internal-function level. The race is made deterministic, not timing-dependent: the test calls the push with `force:true, expected_remote_sha:"<sha-R1>"` while a second task mutates the bare repo with `advance_bare_remote(&remote_dir,"refs/heads/main","racer\n")` -> `<sha-R3>` before the push's ref-update phase. Determinism is obtained by running the advance first and only then issuing the push with the now-stale `<sha-R1>` lease (the same technique the existing racing-push test uses, `crates/mcp-fs/src/tools/git.rs:4557` `e2e_new_084_concurrent_pushes_serialize`, plus the note at `:4600`).
- **Then** the push errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("<sha-R1>")` and `message.contains("<sha-R3>")`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R3>` (the racer's commit survives intact; the losing push overwrote nothing).
- **And** the racer's file content is intact: cloning `remote_dir` into a temp dir and reading the file written by `advance_bare_remote` yields `"racer\n"`.
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` or equals its pre-call value, never `<sha-C2>`.
- **And** a second part of the same test proves the two-writer case: `tokio::join!` of two force pushes carrying the same `expected_remote_sha:"<sha-R1>"` but different local tips (`<sha-C2>` on `main`, `<sha-C5>` on a second branch pushed to the same `remote_branch:"main"`) yields exactly one `Ok` and one `Err(ERR_INVALID_ARGUMENT)`, and the bare repo's `refs/heads/main` equals the winner's sha.
Priority: P0.

---

##### A.z Implementation notes (band A)

1. **`non_fast_forward_error`'s message tail becomes a lie** once `force` exists (`crates/mcp-fs/src/git/remote.rs:744-748` says `"force is not supported"`). Change it; both existing assertions check only the prefix (`crates/mcp-fs/src/git/remote.rs:1165`, `crates/mcp-fs/src/tools/git.rs:4455`), so they stay green.
2. **`add_remote` is an upsert and `remove_remote` is an unconditional DELETE** (`crates/mcp-fs/src/git/db.rs:281-296`). The duplicate check (E2E-NEW-472) and the existence check (E2E-NEW-478) must be implemented in the tool layer; reusing the db methods alone makes those two tests fail.
3. **`refs_under` filters by prefix** (`crates/mcp-fs/src/tools/git.rs:539-554`), so `refs/stash/*` will never leak into `git.branches`, but `status` lists every non-symbolic ref (`:530-534`) and therefore WILL show stash refs — test E2E-NEW-470 pins that asymmetry deliberately; do not "fix" it.
4. **Every new tool must be added to the golden contract** (`tool-contract-golden.json`, regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1`) and to the three per-tool lists in `crates/mcp-fs/src/tools/git.rs:2480-2493` and `:2644+`, plus the forbidden-for-non-member list at `:2685`, or those existing tests fail.
5. **Write-lock discipline**: E2E-NEW-421, E2E-NEW-440, E2E-NEW-469 and E2E-NEW-498 only hold if `branch_switch`, `branch_reset`, `stash_save/apply/pop` and the push path each acquire `entry.write_lock` in the async caller, never inside the blocking closure (the DRIFT-009 rule documented at `crates/mcp-fs/src/tools/git.rs:1216-1223`).

6. **Four contract constants are invented by this specification** (`MAX_BRANCH_NAME_BYTES=255`, `MAX_STASH_ENTRIES=100`, the `refs/stash/{sha}` id scheme, the `status:"conflict"` shape). They are not in the code today; tests E2E-NEW-406, E2E-NEW-407, E2E-NEW-467, E2E-NEW-448, E2E-NEW-462 and E2E-NEW-463 assert them. If the design owner picks different values, those six tests change and nothing else.

#### E2E-NEW-828 — EdgeCase — an empty-string lease with `force:false` is treated as absent**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-158, FR-NEW-156.
- **Preconditions:** SEED-R; local `main` is strictly ahead of remote `<sha-R1>` (a plain fast-forward push). Drives `push_branch_inner`.
- **When** the push is called with `force: false` and `expected_remote_sha: ""`.
- **Then** the push **succeeds**: an empty lease is absent, not a contradiction, so the non-forcing fast-forward path runs. Response `forced` is `false` and `overwritten_sha` is absent.
- **And** the bare remote's `refs/heads/main` reads the new local tip (`git2::Repository::open_bare` + `find_reference`).
- **And** the same call with `expected_remote_sha: "<sha-R1>"` (a real value) and `force:false` asserts error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` and `"force"` — contrasting the two cases in one test so the boundary is explicit.
- **Verification:** success response fields; bare-repo ref read; second-call error assertion.
- **Cleanup:** tempdir drop. **Priority:** P1.

#### E2E-NEW-829 — SideEffect — the contradiction is caught before any network contact**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-158.
- **Preconditions:** SEED-R with the bare remote at `<sha-R1>`; local `main` diverged from it. Drives the registered `git.remote_push` tool (this is a validation test).
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","force":false,"expected_remote_sha":"<sha-R1>"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` and `"force"`.
- **And** the bare remote's `refs/heads/main` still equals `<sha-R1>` exactly.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no `git.remote_push` entry.
- **And** the remote's `objects/` directory holds no object whose sha equals the local tip (nothing was uploaded before the rejection).
- **Verification:** error assertion; bare-repo ref read; audit scan; `Repository::odb().exists(oid)` on the bare repo.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-830 — EdgeCase — a create-only force push audits the all-zero lease**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-159, FR-NEW-155.
- **Preconditions:** SEED-R; the bare remote has **no** `refs/heads/sandbox`. Drives `push_branch_inner`.
- **When** the push runs with `branch:"main"`, `remote_branch:"sandbox"`, `force:true`, `expected_remote_sha:"0000000000000000000000000000000000000000"`.
- **Then** the response has `forced == true`, `overwritten_sha == "0000000000000000000000000000000000000000"` and `new_sha == "<sha-C2>"`.
- **And** exactly one audit entry with `op == "git.remote_push"` is appended, whose `detail` contains `"forced"`, `"sandbox"` and the 40 zeros.
- **And** the bare remote now has `refs/heads/sandbox` = `<sha-C2>` and `refs/heads/main` still `<sha-R1>`.
- **Verification:** response fields; audit entry `op`/`detail` substrings; two bare-repo ref reads.
- **Cleanup:** tempdir drop. **Priority:** P1.

#### E2E-NEW-831 — Failure — a lease-rejected force push writes no audit entry**

**Category:** Failure. **Scenario:** SC-910. **Requirements:** FR-NEW-159, FR-NEW-157.
- **Preconditions:** SEED-R at `<sha-R1>`, then `advance_bare_remote` moves the remote to `<sha-R2>`; the caller still holds `<sha-R1>`. Drives `push_branch_inner`.
- **Given** `before_audit = state.safety.audit(OWNER, MOUNT)`.
- **When** the push runs with `force:true`, `expected_remote_sha:"<sha-R1>"`.
- **Then** assert error whose message contains both `"<sha-R1>"` and `"<sha-R2>"`.
- **And** `state.safety.audit(OWNER, MOUNT)` is element-for-element equal to `before_audit` — a destroyed-sha audit line is written only when something was actually destroyed, so the audit log never claims a force push that did not happen.
- **And** the bare remote's `refs/heads/main` is still `<sha-R2>`.
- **Verification:** error substrings; audit vector equality; bare-repo ref read.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-832 — EdgeCase — a fast-forward push with `force:false` still succeeds**

**Category:** EdgeCase. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
- **Preconditions:** SEED-R; local `main` contains `<sha-R1>` in its ancestry and adds one commit `<sha-C9>`. Drives `push_branch_inner`.
- **When** the push runs with `force` absent entirely.
- **Then** the push succeeds, the response has `forced == false` and no `overwritten_sha` key.
- **And** the bare remote's `refs/heads/main` == `<sha-C9>`.
- **And** the audit entry's `detail` does not contain the string `"forced"`.
- **Verification:** response key absence; bare-repo ref read; audit detail substring absence.
- **Cleanup:** tempdir drop. **Priority:** P0.

#### E2E-NEW-833 — SideEffect — the refused non-fast-forward leaves the remote byte-identical**

**Category:** SideEffect. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
- **Preconditions:** SEED-R, remote advanced to `<sha-R2>` with `/hello.txt` = `"remote-2\n"`; local `main` diverged at `<sha-C2>`. Drives `push_branch_inner` with `force:false`.
- **When** the push runs.
- **Then** assert error containing `"not a fast-forward"` and `"force"`.
- **And** the bare remote's `refs/heads/main` == `<sha-R2>`, and reading `/hello.txt` out of that commit's tree via `git2` gives exactly `b"remote-2\n"`.
- **And** `db.get_ref("refs/remotes/origin/main")` on the local side is unchanged (a refused push does not fabricate a tracking update).
- **And** no audit entry with `op == "git.remote_push"` was added.
- **Verification:** error substrings; bare-repo ref + blob read; local ref read; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-905 — SideEffect — a forced push advances the local tracking ref to the new sha**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Existing angles: 490 (Happy), 495 (EdgeCase, absent branch). This one asserts the local tracking ref follows.
- **Preconditions:** SEED-R at `<sha-R1>`, fetched so `refs/remotes/origin/main` = `<sha-R1>`; local `main` = `<sha-C2>` diverged from it. Drives `push_branch_inner` with `force:true`, `expected_remote_sha:"<sha-R1>"`.
- **When** the push runs.
- **Then** the response has `forced == true`, `overwritten_sha == "<sha-R1>"`, `new_sha == "<sha-C2>"`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-C2>"` — the local view of the remote matches what was just written, so a subsequent lease read is correct without a fetch.
- **And** the bare remote's `refs/heads/main` == `<sha-C2>`.
- **And** an immediately following force push with `expected_remote_sha:"<sha-C2>"` and the same tip succeeds as a no-change update, proving the tracking ref value is usable as the next lease.
- **Verification:** response fields; local tracking ref read; bare-repo ref read; follow-up push.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-906 — EdgeCase — a whitespace-only lease with `force:true` is refused before the network**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Existing angles: 489 (absent), 496 (malformed). This one covers whitespace, the value that trims to empty.
- **Preconditions:** SEED-R, remote at `<sha-R1>`, local diverged. Drives the registered `git.remote_push` tool.
- **When** `git.remote_push {"branch":"main","force":true,"expected_remote_sha":"   "}`, and again with `"\t\n"`.
- **Then** both assert error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` — a whitespace lease is an absent lease, not a lease that fails to match.
- **And** the message does **not** contain `"<sha-R1>"`: this is the missing-lease error (FR-NEW-156), not the stale-lease error (FR-NEW-157), and the two are distinguishable by the caller.
- **And** the bare remote's `refs/heads/main` == `<sha-R1>` and no `git.remote_push` audit entry exists.
- **Verification:** two error assertions; substring absence; bare-repo ref read; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P1.


## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not allow a force push without `expected_remote_sha`; the lease is mandatory, not advisory. Do not silently ignore a lease supplied with `force:false`.

### Scope Boundary
The push path only. Named remotes are US-021.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
A push without `force` keeps today's fast-forward-only refusal. Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
