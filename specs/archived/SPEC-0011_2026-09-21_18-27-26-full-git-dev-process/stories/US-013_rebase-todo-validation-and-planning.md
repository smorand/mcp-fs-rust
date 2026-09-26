# US-013: Rebase todo validation and planning

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 13
> Depends On: US-005
> Complexity: L
> min_tier: 2
> Files touched: 2

## Objective

Accept and validate an interactive rebase plan. The whole todo list is checked before any commit is replayed, because a rebase that fails half way through validation is worse than one that never starts.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/db.rs
```

### Existing Patterns
Tools are registered in `crates/mcp-fs/src/tools/git.rs:59-360` via `ToolSchema::new(...)`; parameter descriptions are frozen contract text.
Commit walking and ref resolution are `crates/mcp-fs/src/tools/git.rs:486-504` (`resolve_ref`) and the `git.log` handler.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-904:** Continue and abort tools are per operation and named after the git CLI (`git.rebase_continue`, `git.merge_abort`) rather than a single generic trio. **Rationale:** the MCP client is an LLM chat behaving like a terminal, and tool names are its primary affordance; `git rebase --continue` is vocabulary it already has. The engine underneath is shared, so this costs surface area, not duplicated logic. **Alternatives considered:** one generic `git.operation_continue` / `git.operation_abort` / `git.conflict_resolve` trio, rejected for discoverability despite being fewer tools. **Implemented by:** FR-NEW-196, FR-NEW-197, FR-NEW-221, FR-NEW-223, FR-NEW-225, FR-NEW-239. **Round:** 2 (approach exploration, Fork 2). **Code evidence:** n/a, new surface.

- **DEC-918:** Rebase supports `pick`, `squash`, `drop` and `reword` only; `edit`, `fixup`, `exec` and `break` are excluded. **Rationale:** `exec` would execute arbitrary commands on the server, which is a sandbox escape, and `edit`/`break` require an interactive pause with no conflict to resolve, which the operation model does not represent. `fixup` is `squash` without message concatenation and adds little. **Implemented by:** FR-NEW-210 through FR-NEW-216. **Round:** 1 (Q9 scope, refined at requirement drafting).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

5 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-210 [EARS-E]: Rebase replays a todo list onto a new base
> WHEN `git.rebase` is called with `onto` and `todo` THE mcp-fs server SHALL replay the todo entries in order onto the commit `onto` resolves to, and SHALL move the current branch to the final replayed commit.

- **Inputs:** `mount_id`, `onto` (ref or sha), `todo` (ordered array of `{action, sha, message?}` where action is `pick`, `squash`, `drop` or `reword`).
- **Outputs:** `{status, branch, new_tip, replayed, dropped, squashed, operation_id?}`.
- **Business Rules:** Replayed commits get new shas. Original commits become unreachable but are not destroyed.
- **Priority:** Must-have

### FR-NEW-215 [EARS-O]: A rebase todo is validated before any work
> IF the todo references an unknown sha, references a sha outside the range between `onto` and the branch tip, omits a commit that is inside that range, begins with a `squash` entry, contains a duplicate sha, contains an unknown action, or carries a `reword` with an empty or whitespace-only message THEN THE mcp-fs server SHALL reject `git.rebase` with `ERR_INVALID_ARGUMENT` naming the offending entry, and SHALL NOT create any commit, move any ref, or record an in-progress operation.

- **Business Rules:** Pre-flight validation in full before the first replay. A rebase that fails half-way through validation is worse than one that never starts.
- **Priority:** Must-have

### FR-NEW-216 [EARS-U]: The todo length is bounded
> The mcp-fs server SHALL reject a `git.rebase` whose todo holds more than `git.max_rebase_todo` entries with `ERR_INVALID_ARGUMENT` naming the limit.

- **Inputs:** config key `git.max_rebase_todo`, default `200`.
- **Priority:** Should-have

### FR-NEW-217 [EARS-O]: Rebase onto an ancestor with nothing to replay is a no-op
> IF `onto` is already the branch's parent such that no commit needs replaying THEN THE mcp-fs server SHALL return `status: "up_to_date"` without creating commits.

- **Priority:** Should-have

### FR-NEW-218 [EARS-O]: Rebase refuses a dirty volume
> IF the volume holds uncommitted changes THEN THE mcp-fs server SHALL reject `git.rebase` before any replay.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

25 tests: DataIntegrity 1, EdgeCase 5, Failure 13, Happy 1, SideEffect 5.
Scenarios covered: SC-914, SC-916, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-532 — git.rebase blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1. Same preconditions.
**When** `git.rebase {mount_id:"proj", onto:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; refs unchanged.
**Verification:** error code + substrings; all `refs/heads/*` sha map equality against snapshot.

#### E2E-NEW-607 — Rebase edge: onto an ancestor is a no-op**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
- **Preconditions:** FX-LINE, branch `main` tip `<sha-of-C4>`.
- **Given** `object_count_before = entry.db.count_objects()` (`db.rs:198`), `tip_before = <sha-of-C4>`, volume snapshot of `/a.txt`,`/b.txt`,`/d.txt`.
- **When** `git.rebase {onto:"<sha-of-C2>", todo:[{action:"pick", sha:"<sha-of-C3>"},{action:"pick", sha:"<sha-of-C4>"}]}` — C2 is already an ancestor of the todo range.
- **Then** result `status == "up_to_date"` with `replayed == 0` (the rebase no-op of FR-NEW-217; the test asserts `result["replayed"] == 0`).
- **And** `entry.db.get_ref("refs/heads/main").target == tip_before` — the tip is byte identical.
- **And** `entry.db.count_objects()` equals `object_count_before` (no new commit objects).
- **And** all three files byte-equal to their snapshot.
- **And** `state.safety.bytes_written(OWNER, MOUNT)` is unchanged from before the call.
- **Priority:** P0.

---

#### E2E-NEW-608 — Rebase edge: branch and onto identical**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
- **Preconditions:** FX-LINE, checked out `main`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-C4>"}]}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing substring `"onto"` and `"same"` — rebasing a branch onto itself has no commit range. *(If the chosen implementation makes this a no-op instead, this test asserts the no-op form of E2E-NEW-607; the spec requires one of the two, decided once, and the test asserts it exactly.)* Baseline assertion for the implementer: **reject**, message `git.rebase: 'onto' resolves to the same commit as the current branch tip, there is nothing to rebase`.
- **And** tip unchanged, no `git_operations` row.
- **Priority:** P1.

---

#### E2E-NEW-610 — Rebase side-effect: audit entries**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN; audit log drained by reading `state.safety.audit(OWNER, MOUNT)` and recording its length `n0` (`safety.rs:159`).
- **When** the E2E-NEW-600 rebase.
- **Then** `audit()[n0..]` contains at least one entry with `op == "git.rebase"` exactly (matching the tool-name convention at `git.rs:250`), `project == "proj1"`, and `detail` containing `"onto"` and the resolved onto sha.
- **And** every entry appended after `n0` has `op` in `{"git.rebase"}` — no foreign op string leaks (e.g. no `"git.checkout_file"`).
- **And** the entries are in chronological order (oldest first, `safety.rs:159`).
- **Priority:** P1.

---

#### E2E-NEW-611 — Rebase side-effect: write quota charged**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN; `q0 = bytes_written(OWNER, MOUNT)` (`safety.rs:163`); config default quota (large).
- **When** the E2E-NEW-600 rebase, which materializes `/a.txt`(8 bytes `"a1\nMAIN\n"`), `/g.txt`(3), `/f.txt`(3) into the volume.
- **Then** `bytes_written - q0 == 14` if the implementation rewrites the full resulting tree, **or** the exact documented subset. The test asserts the implementation's stated rule: **every byte written to the volume is charged**, so the assertion is `bytes_written - q0 == sum(len(bytes) for each file the rebase wrote)`, computed in the test from the final tree (`git.show` file list + `Env::read` lengths).
- **And** a second identical rebase attempt is not required; this test only asserts the charge.
- **Priority:** P1.

---

#### E2E-NEW-612 — Rebase side-effect: no in-progress row on success**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-284.
- **Preconditions:** FX-FORK-CLEAN; assert zero `git_operations` rows for `volume_id="proj1"` before.
- **When** the E2E-NEW-600 rebase (no conflict).
- **Then** `SELECT count(*) FROM git_operations WHERE volume_id='proj1'` == 0 after. *Method:* direct query through the git db handle, `volume_id` in the WHERE clause per the tenancy rule (AGENTS.md).
- **And** `git.rebase_continue {}` then fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **Priority:** P0.

---

#### E2E-NEW-622 — Rebase failure: todo omits a commit in range**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN (range `main..feature` = {F1, F2}). Snapshot tip and volume.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F2>"}]}` — F1 missing.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"todo"` and the omitted short sha `<sha-of-F1>[..8]` and the word `"missing"`. Baseline message: `git.rebase: todo is missing commit(s) in the rebase range: <short-F1>`.
- **And** tip unchanged, volume bytes unchanged, zero `git_operations` rows, `bytes_written` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-623 — Rebase failure: unknown sha in todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [pick F1, pick "0000000000000000000000000000000000000000"]`.
- **Then** error `ERR_NOT_FOUND`, message contains `"0000000"` and `"not found"` (mirrors the existing `commit '{sha}' not found` phrasing at `git.rs:727`).
- **And** tip unchanged; zero rows; `bytes_written` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-624 — Rebase failure: sha outside the rebase range**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN. `<sha-of-C1>` exists but is an ancestor of `onto`, not in `main..feature`.
- **When** `todo = [pick F1, pick F2, pick C1]`.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `<sha-of-C1>[..8]` and `"not in the rebase range"`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-626 — Rebase failure: todo longer than the bound**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-216.
- **Preconditions:** A project with `N_max + 1` commits on `feature` above `main`, where `N_max` is the configured bound (the spec fixes `N_max = 1000`; the test reads the constant from the tool module so it cannot drift).
- **When** `git.rebase` with a `todo` of `N_max + 1` entries.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"todo"`, the number `1000`, and `"at most"`.
- **And** a `todo` of exactly `N_max` entries is accepted by validation (this test asserts only that it does **not** fail with the bound message; it may still be slow, so the test uses `N_max` = the constant with a small fixture only for the reject side, and asserts the accept side with a synthetic `N_max`-length todo that fails later on a different, non-bound error, checking the message does **not** contain `"at most"`).
- **Priority:** P1.

---

#### E2E-NEW-627 — Rebase failure: empty todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[]}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"todo"` and `"empty"`.
- **And** tip unchanged; zero rows. *(Distinct from E2E-NEW-605, where commits are explicitly dropped.)*
- **Priority:** P1.

---

#### E2E-NEW-628 — Rebase failure: unknown action**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"fixup", sha:"<sha-of-F1>"}, {action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"fixup"` and the exact allowed set text `"pick"`, `"squash"`, `"drop"`, `"reword"`.
- **And** the same for `action:"PICK"` (uppercase) — rejected, proving the enum is exact-match like `parse_on_conflict` (`git.rs:1475-1482`).
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-631 — Rebase failure: unknown onto ref**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"refs/heads/nope", todo:[pick F1]}`.
- **Then** error `ERR_NOT_FOUND` containing `"nope"`.
- **And** the same for `onto:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-632 — Rebase failure: missing required arguments**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN.
- **When (a)** `git.rebase {mount_id:"proj1", todo:[pick F1]}` (no `onto`); **(b)** `git.rebase {mount_id:"proj1", onto:"main"}` (no `todo`); **(c)** `git.rebase {onto:"main", todo:[...]}` (no `mount_id`).
- **Then** each fails `ERR_INVALID_ARGUMENT` naming the missing parameter (`"onto"`, `"todo"`, `"mount_id"` respectively) — same shape as the existing required-arg path (`mcp/args.rs` via `a.str("mount_id")`, `git.rs:209`).
- **And** `todo` given as a string, and `todo` given as an object, both fail `ERR_INVALID_ARGUMENT` containing `"todo"` and `"array"`.
- **Priority:** P1.

---

#### E2E-NEW-633 — Rebase failure: duplicate sha in todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [pick F1, pick F2, pick F2]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `<sha-of-F2>[..8]` and `"duplicate"`.
- **And** tip unchanged; zero rows.
- **Priority:** P1.

---

#### E2E-NEW-634 — Rebase failure: validation happens before any work**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN; record `tip`, full volume byte map, `q0 = bytes_written`, `n0 = audit().len()`, `obj0 = count_objects()`.
- **When** each of the five rejection cases in turn: E2E-NEW-622 (omitted), -223 (unknown), -224 (out of range), -225 (leading squash), -229 (empty reword message).
- **Then** after **all five**: tip string-equal, volume byte map equal, `bytes_written == q0`, `count_objects() == obj0`, `audit().len() == n0` (a rejected call records no audit entry and no quota charge, consistent with `safety.rs:338-352`).
- **And** zero `git_operations` rows throughout.
- **Priority:** P0.

---

#### E2E-NEW-638 — Rebase failure: commit while a rebase is paused**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-279.
- **Preconditions:** FX-FORK conflicting, rebase paused.
- **When** `git.commit {mount_id:"proj1", message:"sneaky"}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"rebase in progress"` — the paused op owns the volume, otherwise the pending replay would commit against a foreign tree.
- **And** branch tip still `<sha-of-F2>`; `git.log` contains no commit with message `"sneaky"`.
- **And** after `git.rebase_abort {}`, `git.commit {message:"sneaky"}` succeeds — the guard is lifted.
- **Priority:** P1.

---

#### E2E-NEW-697 — Paused operation survives a repo-store reopen**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-278.
- **Preconditions:** FX-FORK conflicting; rebase paused at F1; record the row's `current_step`, `todo`, `conflicts`.
- **When** the in-process `GitRepoStore` entry is dropped and reopened via `get_or_open_repo` (the cold-open path documented at `repo.rs:19-21`).
- **Then** the `git_operations` row is still present with identical `current_step`, `todo`, `conflicts`.
- **And** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}` completes with the same result asserted in E2E-NEW-614.
- **Priority:** P1.

---

#### E2E-NEW-800 — Happy — a rebase runs once the dirt is committed**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-218, FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature` = `<sha-of-F2>`, `main` = `<sha-of-C2>`.
- **Given** `fs.write_text {path:"/scratch.txt", content:"tmp\n"}` makes the volume dirty, then `git.commit {message:"F3 scratch"}` -> `<sha-of-F3>`, so `git.status` reports `"dirty": false` and zero entries in `changes`.
- **When** `git.rebase {mount_id:"proj1", onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"},{action:"pick",sha:"<sha-of-F3>"}]}`.
- **Then** `status == "completed"` and `replayed == 3`.
- **And** `git.log {ref_name:"feature"}` holds exactly 5 commits, messages in order `["F3 scratch","F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** `/scratch.txt` reads exactly `b"tmp\n"`.
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **Verification:** `git.log` message list; `Env::read` byte compare; direct relational query.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-801 — Failure — a dirty volume refuses the rebase before any replay**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-218.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature` = `<sha-of-F2>`.
- **Given** `fs.write_text {path:"/a.txt", content:"a1\nDIRTY\n"}` (tracked file modified, not committed).
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"`.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"` (unchanged) and `db.get_ref("refs/heads/main").target == "<sha-of-C2>"`.
- **And** `/a.txt` still reads exactly `b"a1\nDIRTY\n"` (the dirt is preserved, not discarded).
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no entry whose `op` is `"git.rebase"`.
- **Verification:** error code + substrings; `db.get_ref`; `Env::read`; relational count; audit scan.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-802 — EdgeCase — dirt that is only a deletion still refuses**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-218.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature`.
- **Given** `fs.delete {path:"/g.txt"}` (tracked file removed, nothing modified, nothing added), so `git.status.changes == [{"path":"/g.txt","status":"deleted"}]`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"` — a deletion counts as dirty exactly as a modification does.
- **And** `/g.txt` is still absent: `VolumeClient::read_bytes("/g.txt")` errors `ERR_NOT_FOUND` (the refusal did not restore it either).
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"`.
- **And** a second run, after `git.commit {message:"drop g"}`, succeeds — proving the refusal was about dirt and not about the todo.
- **Verification:** error code + substring; volume read; ref read; second call `is_ok()`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-815 — SideEffect — status names the rebase tools, not the merge tools**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-MOD-107, FR-NEW-281.
- **Preconditions:** FX-FORK4 on `proj1`; `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}` pauses at entry index `0` on `/a.txt`.
- **When** `git.status {mount_id:"proj1"}`.
- **Then** `operation` (the object named by FR-NEW-281) is exactly `{"op_type":"rebase","source_ref":null,"current_step":0,"total_steps":4,"remaining_conflicts":["/a.txt"],"continue_with":"git.rebase_continue","abort_with":"git.rebase_abort"}`.
- **And** `continue_with`/`abort_with` does **not** contain `"git.merge_resolve"` or `"git.merge_abort"`.
- **And** the top-level `branch` field is still `"feature"` and `head` equals the pre-rebase `<sha-of-F4>` (the branch has not moved while paused at step 1).
- **Verification:** exact JSON object equality; substring absence; ref/field equality.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

---

#### E2E-NEW-850 — EdgeCase — exactly `max_rebase_todo` entries are accepted**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-216, FR-NEW-210.
- **Preconditions:** `Env::build` with default `c.git.max_rebase_todo = 200`; `proj1` seeded with `C1` then 200 commits `S1..S200`, each adding `/s/{i:03}.txt` = `"s{i}\n"`, on branch `feature`; `main` = `C1` plus one unrelated commit `C2` adding `/m.txt` = `"m\n"`.
- **When** `git.rebase {onto:"main", todo:[pick S1, ..., pick S200]}` (exactly 200 entries, `assert_eq!(todo.len(), 200)` in-test).
- **Then** the call succeeds with `status == "completed"` and `replayed == 200`.
- **And** `git.log {ref_name:"feature"}` holds exactly 202 commits and `/s/200.txt` reads `b"s200\n"`.
- **And** `/m.txt` reads `b"m\n"` (the new base is present).
- **Verification:** success response; log length; two byte reads.
- **Cleanup:** fixture drop. **Priority:** P2.

#### E2E-NEW-873 — EdgeCase — the whole read-only set answers during a paused rebase**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-280.
- **Preconditions:** FX-FORK4 on `proj1` with one stash saved beforehand and a tag `v1` created at `<sha-of-C1>`; rebase paused at step 1.
- **When** each of `git.status`, `git.log {ref_name:"feature"}`, `git.show {commit_sha:"<sha-of-C1>"}`, `git.diff {from_ref:"<sha-of-C1>", to_ref:"<sha-of-C2>"}`, `git.branches`, `git.tags`, `git.blame {path:"/a.txt"}`, `git.stash_list`, `git.remote_list` is called while paused.
- **Then** all nine calls return `Ok` — none errors with `ERR_INVALID_ARGUMENT` about an operation in progress.
- **And** `git.branches` lists `main` and `feature` with `feature` marked current, `git.tags` lists `v1`, `git.stash_list` returns the one saved entry, `git.remote_list` returns `[]`.
- **And** immediately afterwards `git.commit {message:"nope"}` errors `ERR_INVALID_ARGUMENT` containing `"rebase"` — the contrast proves the nine successes are a deliberate allowance, not a missing guard.
- **Verification:** nine `is_ok()` assertions plus payload checks; one contrasting error assertion.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

---

#### E2E-NEW-915 — SideEffect — an `up_to_date` rebase writes nothing at all**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
Existing angles: 607 and 608 are both EdgeCase no-op assertions on the response. This one asserts the absence of side effects.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; capture `before_audit`, `before_bytes`, `before_objects = COUNT(*) FROM git_objects WHERE volume_id='proj1'`.
- **When** `git.rebase {onto:"<sha-of-C3>", todo:[{action:"pick", sha:"<sha-of-C4>"}]}` where `C3` is already `C4`'s parent, so nothing needs replaying.
- **Then** `status == "up_to_date"`, `replayed == 0`, `dropped == 0`, `squashed == 0`, no `operation_id` key.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"` (the same sha, not a rewritten equivalent).
- **And** `COUNT(*) FROM git_objects` equals `before_objects` — no new commit or tree object was created and thrown away.
- **And** `audit(OWNER, MOUNT)` equals `before_audit` element-for-element, `bytes_written` equals `before_bytes`, and no `git_operations` row exists.
- **Verification:** response fields; ref equality; relational counts; audit/quota equality.
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
