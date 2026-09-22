# US-016: Cherry-pick with continue and abort

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 16
> Depends On: US-003, US-015
> Complexity: L
> min_tier: 2
> Files touched: 1

## Objective

Apply a single commit from elsewhere onto the current branch, with the same pause, continue and abort machinery as rebase, and without silently duplicating a commit the branch already carries.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The existing three-way merge is `crates/mcp-fs/src/tools/git.rs:1835-1839` (libgit2 `merge_commits` + `MergeOptions::file_favor`); its refusal path is `crates/mcp-fs/src/tools/git.rs:1844-1849`.
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-904:** Continue and abort tools are per operation and named after the git CLI (`git.rebase_continue`, `git.merge_abort`) rather than a single generic trio. **Rationale:** the MCP client is an LLM chat behaving like a terminal, and tool names are its primary affordance; `git rebase --continue` is vocabulary it already has. The engine underneath is shared, so this costs surface area, not duplicated logic. **Alternatives considered:** one generic `git.operation_continue` / `git.operation_abort` / `git.conflict_resolve` trio, rejected for discoverability despite being fewer tools. **Implemented by:** FR-NEW-196, FR-NEW-197, FR-NEW-221, FR-NEW-223, FR-NEW-225, FR-NEW-239. **Round:** 2 (approach exploration, Fork 2). **Code evidence:** n/a, new surface.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

6 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-235 [EARS-E]: Cherry-pick applies one commit
> WHEN `git.cherry_pick` is called with `commit_sha` THE mcp-fs server SHALL apply that commit's change onto the checked-out branch as a new commit with a new sha.

- **Inputs:** `mount_id`, `commit_sha`.
- **Outputs:** `{status, new_sha, source_sha}`.
- **Business Rules:** The original author is preserved; the committer is the caller.
- **Priority:** Must-have

### FR-NEW-236 [EARS-O]: An already-present commit is reported
> IF the named commit is already an ancestor of the current branch THEN THE mcp-fs server SHALL return `status: "already_present"` and SHALL NOT create a duplicate commit.

- **Priority:** Must-have

### FR-NEW-237 [EARS-O]: An unknown cherry-pick source is not found
> IF `commit_sha` resolves to no commit THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND`.

- **Priority:** Must-have

### FR-NEW-238 [EARS-E]: A conflicting cherry-pick pauses
> WHEN applying the commit produces a conflict THE mcp-fs server SHALL record an in-progress operation of type `cherry_pick` and SHALL return the conflict response of FR-NEW-170.

- **Priority:** Must-have

### FR-NEW-239 [EARS-E]: Continue and abort a cherry-pick
> WHEN `git.cherry_pick_continue` is called with `resolutions` THE mcp-fs server SHALL complete the paused cherry-pick, and WHEN `git.cherry_pick_abort` is called THE mcp-fs server SHALL restore HEAD and the volume to their exact pre-operation state and clear the record.

- **Priority:** Must-have

### FR-NEW-240 [EARS-O]: Cherry-picking a merge commit requires a mainline
> IF `commit_sha` names a commit with more than one parent and `mainline` is absent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` explaining that a mainline parent must be chosen.

- **Inputs:** `mainline` (integer, optional, 1-based parent index).
- **Priority:** Should-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

27 tests: DataIntegrity 1, EdgeCase 6, Failure 9, Happy 6, SideEffect 5.
Scenarios covered: SC-918, SC-919, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-533 — git.cherry_pick blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1.
**When** `git.cherry_pick {mount_id:"proj", commit_sha:<C1>}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; refs unchanged.
**Verification:** as above.

#### E2E-NEW-559 — no conflict markers after rebase / cherry_pick / stash_pop conflicts

**Scenario:** SC-919. **Requirements:** FR-NEW-172.
**Category:** DataIntegrity. **Priority:** P0.
**Preconditions:** three sub-cases, each on a fresh fixture: (a) `git.rebase {onto:"feature"}` from SEED-CONFLICT; (b) `git.cherry_pick {commit_sha:<C1>}` from SEED-CONFLICT on main; (c) E2E-NEW-540's stash setup.
**Then** each returns `status == "conflict"` with `operation` equal to `"rebase"`, `"cherry_pick"`, `"stash_pop"` respectively, and in each case the full-volume marker scan finds nothing.
**Verification:** JSON `operation` field; marker scan.
**Cleanup:** `git.merge_abort` in each.

#### E2E-NEW-640 — Cherry-pick happy: apply F2 onto main**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN; checked out branch `main` (tip `<sha-of-C2>`); volume equals C2's tree (`/a.txt == b"a1\nMAIN\n"`). F2 was authored with `author_name:"Ada"`, `author_email:"ada@example.test"` (the `git.commit` optional author args, `git.rs:194-199`), caller of the pick is `OWNER`.
- **When** `git.cherry_pick {mount_id:"proj1", commit_sha:"<sha-of-F2>"}`.
- **Then** result `status == "committed"` and `result["new_sha"]` is a 40-char sha `!= <sha-of-F2>` (`git.cherry_pick` returns `new_sha`, not `commit_sha`, per FR-NEW-199).
- **And** `git.log{ref_name:"main"}` = 3 commits, messages `["F2 feature add","C2 main edit","C1 base"]`.
- **And** `commits[0]["author"] == "Ada"` and `commits[0]["author_email"] == "ada@example.test"` — author preserved (`git.rs:2164-2165`).
- **And** the committer is the caller: `git.show {commit_sha: result["new_sha"]}` reports committer email `"owner@test.com"`. *Method:* `git.show` must expose committer; if it does not today, assert via `repo.find_commit(oid).committer().email()` in the test.
- **And** `commits[0]["parents"] == [<sha-of-C2>[..8]]` and parent count == 1.
- **And** volume: `/f.txt == b"f1\n"`, `/a.txt == b"a1\nMAIN\n"` (untouched).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

#### E2E-NEW-641 — Cherry-pick happy: only the diff is applied**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN plus an extra `main` commit C2b (`C2b add keep`, `/keep.txt`=`"keep\n"`), so `main`'s tree = `{a.txt, keep.txt}`. F2 adds `/f.txt` only.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** the new tip's tree has exactly 3 entries: `a.txt`, `keep.txt`, `f.txt`.
- **And** `/keep.txt == b"keep\n"` and `/a.txt == b"a1\nMAIN\n"` — the source commit's *tree* did not replace the target tree; only its diff was applied.
- **And** `/g.txt` (introduced by F1, not picked) is absent: `read_bytes("/g.txt")` -> `ERR_NOT_FOUND`.
- **Priority:** P0.

---

#### E2E-NEW-642 — Cherry-pick happy: two sequential picks**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN, on `main`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F1>"}` then `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `git.log{ref_name:"main"}` = 4 commits, messages `["F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** the two new shas differ from each other and from `<sha-of-F1>`,`<sha-of-F2>`.
- **And** `commits[0]["parents"] == [commits[1]["sha"][..8]]`, `commits[1]["parents"] == [<sha-of-C2>[..8]]`.
- **And** volume contains `/g.txt == b"g1\n"` and `/f.txt == b"f1\n"`.
- **Priority:** P1.

---

#### E2E-NEW-643 — Cherry-pick side-effect: audit and quota**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN on `main`; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}` (writes `/f.txt`, 3 bytes).
- **Then** `audit()[n0..]` contains exactly one entry with `op == "git.cherry_pick"`, `path` = the affected path (`"/f.txt"`) or the mount root, and `detail` containing `<sha-of-F2>` (full or short, asserted verbatim against the implementation's chosen format, which the spec fixes as the **full** source sha).
- **And** `bytes_written() - q0 == 3`.
- **And** no audit entry with op `"git.commit"` was appended (the pick is its own audited operation, not a nested commit).
- **Priority:** P1.

---

#### E2E-NEW-644 — Cherry-pick side-effect: source branch untouched**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN on `main`; record `feature_tip = <sha-of-F2>` and `git.log{ref_name:"feature"}`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == feature_tip`.
- **And** `git.log{ref_name:"feature"}` is JSON-equal to the recorded log.
- **And** `entry.db.get_ref("HEAD")` is still symbolic pointing at `refs/heads/main` (`db.rs:214`, symbolic flag).
- **Priority:** P0.

---

#### E2E-NEW-645 — Cherry-pick conflict: nothing applied**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-171., FR-NEW-186, FR-NEW-187, FR-NEW-188
- **Preconditions:** FX-FORK (conflicting): on `main` (tip C2, `/a.txt == b"a1\nMAIN\n"`), pick F1 which sets `/a.txt` to `"a1\nFEAT1\n"`. Snapshot volume and `main` tip.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F1>"}`.
- **Then** result `status == "conflict"`, `result["conflicts"] == ["/a.txt"]`.
- **And** exactly one `git_operations` row with `volume_id='proj1'`, `op_type='cherry_pick'`, in-progress state, and `todo` holding the single source sha `<sha-of-F1>`.
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` — unchanged.
- **And** `/a.txt == b"a1\nMAIN\n"` — byte-identical to the snapshot; no markers.
- **Priority:** P0.

---

#### E2E-NEW-646 — Cherry-pick continue with `ours`**

**Category:** Happy. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-174.
- **Preconditions:** E2E-NEW-645 state.
- **When** `git.cherry_pick_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** `status == "committed"`, a new commit sha `!= <sha-of-F1>`.
- **And** `/a.txt == b"a1\nMAIN\n"` (ours = the current branch side).
- **And** `git.log{ref_name:"main"}` = 3 commits, `commits[0]["message"] == "F1 feature edit"`, `parents == [<sha-of-C2>[..8]]`.
- **And** zero `git_operations` rows.
- **And** with `"theirs"` instead (separate run from the same precondition), `/a.txt == b"a1\nFEAT1\n"`.
- **Priority:** P0.

---

#### E2E-NEW-647 — Cherry-pick abort: exact restoration**

**Category:** Happy. **Scenario:** SC-919. **Requirements:** FR-NEW-239.
- **Preconditions:** E2E-NEW-645 state; `tip_before = <sha-of-C2>`, full volume byte map recorded before the pick.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}`.
- **Then** `entry.db.get_ref("refs/heads/main").target == tip_before`, and the on-disk `refs/heads/main` equals it too.
- **And** every volume path and its bytes equal the recorded map; the path set is identical.
- **And** zero `git_operations` rows.
- **And** a subsequent `git.cherry_pick_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`.
- **Priority:** P0.

---

#### E2E-NEW-648 — Cherry-pick edge: commit already in current history**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
- **Preconditions:** FX-LINE, on `main` (tip `<sha-of-C4>`); `<sha-of-C2>` is an ancestor of the tip.
- **When** `git.cherry_pick {commit_sha:"<sha-of-C2>"}`.
- **Then** result `status == "already_applied"` (asserted verbatim) and `result["new_sha"] == "<sha-of-C2>"`, with a `message` containing `"already"` and `"history"`. *(Reported, not an error, not a duplicate.)*
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C4>` — tip unchanged.
- **And** `git.log{ref_name:"main"}` still has exactly 4 commits and `commits` contains `<sha-of-C2>` exactly once.
- **And** `bytes_written` and `audit().len()` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-649 — Cherry-pick edge: equivalent content already applied under a different sha**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
- **Preconditions:** FX-FORK-CLEAN on `main`; first run `git.cherry_pick {commit_sha:"<sha-of-F2>"}` successfully, producing `<sha-of-P1>` whose diff is identical to F2's.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}` a second time.
- **Then** the result is **not** `"already_applied"` by sha (F2 is not an ancestor), and the outcome is the documented empty-diff behaviour: `status == "empty"` with `message` containing `"no changes"`, and `entry.db.get_ref("refs/heads/main").target == <sha-of-P1>` (no empty commit created).
- **And** `git.log{ref_name:"main"}` still has exactly 3 commits.
- **Priority:** P1.

---

#### E2E-NEW-650 — Cherry-pick edge: merge commit**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-240.
- **Preconditions:** FX-MERGE; on a separate branch `other` (off C1) so MG is not an ancestor.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>"}`.
- **Then** error `ERR_INVALID_ARGUMENT`, message containing `"merge commit"` and `"2 parents"` and naming `git.revert`'s `mainline` concept or stating the pick has no mainline parameter. Baseline: `git.cherry_pick: '<short-MG>' is a merge commit with 2 parents and cannot be cherry-picked; pick one of its parents instead`.
- **And** branch tip unchanged; zero rows; `bytes_written` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-651 — Cherry-pick edge: empty-diff commit**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** a branch `feature` whose commit `E1` (`E1 empty`) has a tree identical to its parent's (created by committing twice with no volume change, allowed since `git.commit` builds from the volume, `git.rs:664`). `main` is at C1.
- **When** `git.cherry_pick {commit_sha:"<sha-of-E1>"}` on `main`.
- **Then** `status == "empty"`, message contains `"no changes"`, tip unchanged, commit count on `main` unchanged, zero rows.
- **Priority:** P1.

---

#### E2E-NEW-652 — Cherry-pick failure: unknown sha**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
- **When** `git.cherry_pick {commit_sha:"0000000000000000000000000000000000000000"}` on FX-LINE.
- **Then** `ERR_NOT_FOUND` with message containing `"0000000000000000000000000000000000000000"` and `"not found"` (matching `git.rs:727` phrasing).
- **And** tip unchanged, zero rows, `bytes_written` unchanged, `audit().len()` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-653 — Cherry-pick failure: malformed sha**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
- **When** `commit_sha` = `"zzzz"`, then `""`, then `"HEAD~1"`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"commit_sha"` (a non-hex value is an argument error, not a lookup miss; distinct from E2E-NEW-652). For `"HEAD~1"` the message additionally states that a full or abbreviated sha is required.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-654 — Cherry-pick failure: missing `commit_sha`**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **When** `git.cherry_pick {mount_id:"proj1"}`.
- **Then** `ERR_INVALID_ARGUMENT` naming `"commit_sha"`.
- **Priority:** P1.

---

#### E2E-NEW-655 — Cherry-pick failure: continue with nothing in progress**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-224, FR-NEW-225.
- **Preconditions:** FX-LINE, no op in progress.
- **When** `git.cherry_pick_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`.
- **And** a **rebase** paused (E2E-NEW-613) then `git.cherry_pick_continue {}` also fails with the same message — the two operations do not share a continue channel.
- **Priority:** P0.

---

#### E2E-NEW-656 — Cherry-pick failure: abort with nothing in progress**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-224, FR-NEW-225.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}` on FX-LINE.
- **Then** `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`; tip unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-657 — Cherry-pick failure: empty repo (no HEAD target)**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** `git.init` only, no commit — the state where `git.log` returns `{"commits": []}` (`git.rs:569`).
- **When** `git.cherry_pick {commit_sha:"<any 40-hex>"}`.
- **Then** `ERR_NOT_FOUND` containing `"not found"` (the sha cannot exist) — and for a repo that has objects but an unborn HEAD, `ERR_INVALID_ARGUMENT` containing `"no commit on the current branch"`.
- **And** `entry.db.get_ref("refs/heads/main")` is still `None` (`db.rs:214`).
- **Priority:** P1.

---

#### E2E-NEW-698 — Rebase and cherry-pick are mutually exclusive**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-283.
- **Preconditions:** FX-FORK conflicting; rebase paused.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`, then `git.revert {commit_sha:"<sha-of-C2>"}`, then `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"rebase in progress"` and naming `git.rebase_continue` / `git.rebase_abort`.
- **And** still exactly 1 `git_operations` row with unchanged `current_step`; branch tip and volume unchanged.
- **Priority:** P1.

---

#### E2E-NEW-858 — SideEffect — the cherry-pick row is typed and single-stepped**

**Category:** SideEffect. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-275.
- **Preconditions:** FX-FORK, HEAD on `main` (= `<sha-of-C2>`); `<sha-of-F1>` edits `/a.txt` line 2 to `FEAT1` and conflicts with C2.
- **When** `git.cherry_pick {mount_id:"proj1", commit_sha:"<sha-of-F1>"}`.
- **Then** `status == "conflict"`, `operation == "cherry_pick"`, `continue_with == "git.cherry_pick_continue"`, `abort_with == "git.cherry_pick_abort"`.
- **And** the single `git_operations` row has `op_type == "cherry_pick"`, `current_step == null`, `total_steps == null` (single-step operations report null per FR-NEW-186), `original_tip_sha == "<sha-of-C2>"`, `conflicts` holding exactly one element whose `path` is `/a.txt`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C2>"` and `/a.txt` reads exactly `b"a1\nMAIN\n"`.
- **Verification:** response fields; relational row field-by-field; ref read; byte compare.
- **Cleanup:** `git.cherry_pick_abort`. **Priority:** P0.

#### E2E-NEW-859 — EdgeCase — a cherry-pick delete/modify conflict surfaces a null side**

**Category:** EdgeCase. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-180.
- **Preconditions:** `proj1`: `C1` commits `/lib/util.rs` = `"pub fn a() {}\n"`; on `side`, `S1` deletes it; on `main`, `C2` rewrites it to `"pub fn a() { 1 }\n"`. HEAD on `main`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-S1>"}`.
- **Then** `status == "conflict"` with one entry `path == "/lib/util.rs"`, `theirs.exists == false` (the picked commit deleted it), `ours.exists == true` with content `"pub fn a() { 1 }\n"`, `base.exists == true` with content `"pub fn a() {}\n"`.
- **When** `git.cherry_pick_continue {resolutions:[{path:"/lib/util.rs", strategy:"theirs"}]}`.
- **Then** `status == "committed"`, `VolumeClient::read_bytes("/lib/util.rs")` errors `ERR_NOT_FOUND`, and the new commit's tree has no `lib/util.rs` entry.
- **And** the new commit's single parent is `<sha-of-C2>` and its sha differs from `<sha-of-S1>`.
- **Verification:** conflict entry fields; completion response; volume read error; `git.show` tree; parent assertions.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-861 — Happy — `mainline:1` picks the merge's change relative to the first parent**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-240, FR-NEW-235.
- **Preconditions:** FX-MERGE; branch `other` off `C1` checked out, volume holds `/a.txt` = `"a1\n"` only.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:1}`.
- **Then** `status == "committed"` and `new_sha` differs from `<sha-of-MG>`.
- **And** `/s.txt` reads exactly `b"s1\n"` (the side branch's change, which is what `MG` added relative to parent 1 `M1`).
- **And** `/m.txt` is absent: `VolumeClient::read_bytes("/m.txt")` errors `ERR_NOT_FOUND` (the mainline's own change was not pulled in).
- **And** the new commit has exactly 1 parent, equal to `<sha-of-C1>`, and `source_sha == "<sha-of-MG>"` in the response.
- **Verification:** response fields; two volume reads; `parent_count()`/`parent_id(0)`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-875 — SideEffect — status names the cherry-pick tools for a paused cherry-pick**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-NEW-238.
- **Preconditions:** the paused cherry-pick of E2E-NEW-858.
- **When** `git.status {mount_id:"proj1"}`.
- **Then** `operation` (the object named by FR-NEW-281) is exactly `{"op_type":"cherry_pick","source_ref":"<sha-of-F1>","current_step":null,"total_steps":null,"remaining_conflicts":["/a.txt"],"continue_with":"git.cherry_pick_continue","abort_with":"git.cherry_pick_abort"}`.
- **And** the strings `"git.merge_resolve"`, `"git.rebase_continue"` appear nowhere in the serialized status response.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}` then `git.status` again.
- **Then** the response has no `operation` (the object named by FR-NEW-281) key and `head == "<sha-of-C2>"`.
- **Verification:** exact JSON equality; substring absence; post-abort key absence.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-918 — SideEffect — an `already_present` cherry-pick changes nothing observable**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
Existing angles: 648 and 649 both assert the reported status. This one asserts the absence of side effects.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; `<sha-of-C2>` is an ancestor. Capture `before_audit`, `before_bytes`, `before_objects`, `before_log = git.log {ref_name:"main"}`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-C2>"}`.
- **Then** the call is `Ok` with `status == "already_present"` and `new_sha` is `null`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"` and `git.log {ref_name:"main"}` equals `before_log` element-for-element.
- **And** `COUNT(*) FROM git_objects` equals `before_objects`, `bytes_written` equals `before_bytes`, and `audit(OWNER, MOUNT)` gains at most one entry whose `detail` contains `"already_present"`.
- **And** no `git_operations` row exists.
- **Verification:** response fields; ref and log equality; relational count; quota; audit inspection.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-919 — EdgeCase — a sha that names a non-commit object**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
Existing angles: 652 (unknown sha), 653 (malformed sha). This one covers a sha that *exists* but is the wrong object type.
- **Preconditions:** FX-LINE; `<sha-BLOB>` = the blob sha of `/a.txt`'s content and `<sha-TREE>` = the root tree sha of `<sha-of-C4>`, both read via `git.show`/`git2` and both satisfying `db.object_exists(..) == true`.
- **When** `git.cherry_pick {commit_sha:"<sha-BLOB>"}`, then `git.cherry_pick {commit_sha:"<sha-TREE>"}`.
- **Then** both assert error `ERR_NOT_FOUND` whose message contains the supplied sha and the words `"not a commit"` — an existing object of the wrong type is reported as no commit, not as an internal error.
- **And** neither call leaves a `git_operations` row, and `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `db.object_exists("<sha-BLOB>")` is still `true` (the failed lookup pruned nothing).
- **Verification:** two error assertions; relational count; ref read; `object_exists`.
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
