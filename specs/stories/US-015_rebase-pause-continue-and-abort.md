# US-015: Rebase pause, continue and abort

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 15
> Depends On: US-003, US-014
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Make a rebase interruptible. A conflicting step pauses with its index and remaining todo persisted, `git.rebase_continue` resumes from exactly that step and can pause again, and `git.rebase_abort` restores the branch tip and every volume byte exactly.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Table declaration lives in `crates/mcp-fs/src/git/db.rs:66-90` (`fn schema()`), the owned-table list in `:42` (`TABLES`), and every query is scoped by `volume_id`.
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
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

7 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-219 [EARS-E]: A conflicting step pauses the rebase
> WHEN replaying a todo entry produces a conflict THE mcp-fs server SHALL stop at that entry, SHALL record an in-progress operation holding the operation type `rebase`, the index of the paused entry, the remaining todo and the conflict set, and SHALL return the conflict response of FR-NEW-170.

- **Business Rules:** Steps already replayed stay replayed; the pause is at a commit boundary, not mid-file.
- **Priority:** Must-have

### FR-NEW-220 [EARS-E]: A rebase can pause more than once
> WHEN a resumed rebase encounters a further conflicting entry THE mcp-fs server SHALL pause again at that entry and update the in-progress record's step index accordingly.

- **Priority:** Must-have

### FR-NEW-221 [EARS-E]: Continue a paused rebase
> WHEN `git.rebase_continue` is called with `resolutions` while a rebase is paused THE mcp-fs server SHALL resolve the paused entry per FR-NEW-174 through FR-NEW-179, SHALL commit that entry, and SHALL proceed through the remaining todo entries until the list is exhausted or a further conflict pauses it.

- **Priority:** Must-have

### FR-NEW-222 [EARS-O]: Continuing with unresolved conflicts is rejected
> IF `git.rebase_continue` is called while paths in the conflict set remain unresolved THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` listing the unresolved paths and SHALL keep the rebase paused at the same step.

- **Priority:** Must-have

### FR-NEW-223 [EARS-E]: Abort a rebase exactly
> WHEN `git.rebase_abort` is called THE mcp-fs server SHALL restore the branch ref to its exact pre-rebase sha, SHALL restore every volume file to its exact pre-rebase bytes, and SHALL delete the in-progress record.

- **Business Rules:** The pre-rebase tip is stored in the operation record precisely so abort is exact rather than approximate.
- **Priority:** Must-have

### FR-NEW-224 [EARS-O]: Continue or abort with no rebase in progress
> IF `git.rebase_continue` or `git.rebase_abort` is called when no rebase is in progress THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` stating that no rebase is in progress.

- **Priority:** Must-have

### FR-NEW-225 [EARS-UB]: Continue channels are not shared between operations
> The mcp-fs server SHALL NOT allow a continuation tool to advance an operation type other than the ones assigned to it in FR-NEW-241.

- **Business Rules:** Each rejects with `ERR_INVALID_ARGUMENT` naming the operation that is actually in progress and the tool pair that finishes it. Concretely: `git.rebase_continue` does not advance a cherry-pick or a revert, `git.cherry_pick_continue` does not advance a rebase or a revert, `git.revert_continue` does not advance a cherry-pick or a rebase, and `git.merge_resolve` advances none of the three.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

23 tests: EdgeCase 8, Failure 7, Happy 5, Integration 1, SideEffect 2.
Scenarios covered: SC-912, SC-915, SC-916, SC-917, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-613 — Rebase conflict: single pause on step 1**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-219., FR-NEW-186, FR-NEW-187, FR-NEW-188
- **Preconditions:** FX-FORK (the conflicting one: F1 sets `/a.txt` = `"a1\nFEAT1\n"`, C2 sets it to `"a1\nMAIN\n"`).
- **Given** volume currently equals F2's tree: `/a.txt == b"a1\nFEAT1\n"`, `/f.txt == b"f1\n"`. Snapshot both.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** result `status == "conflict"` (shared model).
- **And** `result["current_step"] == 0` (the zero-based index of the F1 entry, per FR-NEW-187; `todo[current_step]` is the entry awaiting resolution. The implementer must make `current_step` identify the F1 entry and the test asserts it resolves to `todo[current_step].sha == <sha-of-F1>`).
- **And** `result["conflicts"]` is exactly `["/a.txt"]`.
- **And** exactly one `git_operations` row exists with `volume_id='proj1'`, `op_type='rebase'`, `state='conflicted'` (the closed set of FR-NEW-188), `todo` deserializing to the 2-entry list submitted.
- **And** volume is byte-identical to the snapshot: `/a.txt == b"a1\nFEAT1\n"`, `/f.txt == b"f1\n"` — nothing applied.
- **And** `entry.db.get_ref("refs/heads/feature").target == <sha-of-F2>` — the ref has not moved.
- **Priority:** P0.

---

#### E2E-NEW-614 — Rebase conflict: continue with `theirs` completes**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-174.
- **Preconditions:** E2E-NEW-613 state (paused on F1, conflict on `/a.txt`).
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:{"/a.txt":"theirs"}}` where `theirs` = the commit being replayed (F1).
- **Then** result `status == "completed"` (the closed `git.rebase_continue` status set of FR-NEW-199).
- **And** `/a.txt == b"a1\nFEAT1\n"` exactly (theirs = F1's side).
- **And** `git.log{ref_name:"feature"}` = 4 commits, messages `["F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** both replayed shas differ from `<sha-of-F1>`/`<sha-of-F2>`.
- **And** zero `git_operations` rows for `volume_id='proj1'`.
- **And** `/a.txt` contains no occurrence of `"<<<<<<<"`, `"======="`, `">>>>>>>"`.
- **Priority:** P0.

---

#### E2E-NEW-615 — Rebase multi-pause: steps 2 and 4 both conflict**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-220.
- **Preconditions:** **FX-FORK4** (conflicting variant): `main` = C1, C2 where C2 sets `/a.txt` = `"a1\nMAIN\n"`. `feature` = C1, F1, F2, F3, F4 where F1 adds `/f1.txt`=`"1\n"` (clean), **F2** sets `/a.txt` = `"a1\nFEAT2\n"` (conflicts with C2), F3 adds `/f3.txt`=`"3\n"` (clean), **F4** sets `/a.txt` = `"a1\nFEAT4\n"` (conflicts with the replayed result).
- **When (1)** `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}`.
- **Then (1)** `status == "conflict"`, `todo[current_step].sha == <sha-of-F2>`, `conflicts` holding exactly one element whose `path` is `/a.txt`, one `git_operations` row `state` in-progress, `current_step` persisted as the F2 index, branch ref still `<sha-of-F4>`, volume byte-identical to the pre-call snapshot.
- **When (2)** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}`.
- **Then (2)** `status == "conflict"` again, `todo[current_step].sha == <sha-of-F4>` — resumption skipped F1 (already applied) and F3 (clean) and stopped exactly at F4.
- **And (2)** `git.log` of the in-progress head (exposed by the conflict envelope as `result["head"]`, or asserted after completion) — at minimum, the `git_operations` row's `current_step` identifies F4 and the row count is still exactly 1.
- **When (3)** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}`.
- **Then (3)** `status == "completed"`.
- **And** `git.log{ref_name:"feature"}` = exactly 6 commits, messages in order `["F4 feat four","F3 feat three","F2 feat two","F1 feat one","C2 main edit","C1 base"]`.
- **And** every one of the four replayed shas differs from `<sha-of-F1>`..`<sha-of-F4>`; each has exactly 1 parent; `commits[3]["parents"] == [<sha-of-C2>[..8]]`.
- **And** volume: `/a.txt == b"a1\nFEAT4\n"`, `/f1.txt == b"1\n"`, `/f3.txt == b"3\n"`, no conflict markers in any file.
- **And** zero `git_operations` rows.
- **And** exactly **two** conflict pauses occurred (the test counts `status=="conflict"` responses == 2).
- **Priority:** P0.

---

#### E2E-NEW-619 — Rebase abort: exact restoration**

**Category:** Happy. **Scenario:** SC-917. **Requirements:** FR-NEW-223.
- **Preconditions:** FX-FORK conflicting. Before the rebase: `tip_before = entry.db.get_ref("refs/heads/feature").target` (== `<sha-of-F2>`), and a full map `{path -> bytes}` of the volume (`/a.txt` = `"a1\nFEAT1\n"`, `/f.txt` = `"f1\n"`).
- **When** `git.rebase` pauses (E2E-NEW-613), **then** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** result `status == "aborted"` (every abort tool reports `aborted`, per FR-NEW-199).
- **And** `entry.db.get_ref("refs/heads/feature").target == tip_before` — string equality on the full 40-char sha.
- **And** the on-disk reference agrees: `repo.find_reference("refs/heads/feature").target().to_string() == tip_before` (the dual write at `git.rs:702-711` must hold for the new ops too).
- **And** the volume path set equals the snapshot key set, and for each path bytes are equal — byte-for-byte, not string-trimmed.
- **And** `git.log{ref_name:"feature"}` is JSON-equal to the log captured before the rebase.
- **And** zero `git_operations` rows for `volume_id='proj1'`.
- **Priority:** P0.

---

#### E2E-NEW-621 — Rebase abort mid multi-pause (after one continue)**

**Category:** EdgeCase. **Scenario:** SC-917. **Requirements:** FR-NEW-223.
- **Preconditions:** FX-FORK4 conflicting. Snapshot `tip_before = <sha-of-F4>` and full volume byte map.
- **When** rebase pauses at F2; `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}` pauses at F4; **then** `git.rebase_abort {}`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == tip_before`.
- **And** the volume byte map equals the snapshot exactly — including `/a.txt == b"a1\nFEAT4\n"`, i.e. the already-applied F1/F2/F3 replay is fully undone.
- **And** zero `git_operations` rows.
- **And** `git.log{ref_name:"feature"}` messages == `["F4 feat four","F3 feat three","F2 feat two","F1 feat one","C1 base"]` with the **original** shas F1..F4 (equality on all four full shas).
- **Priority:** P0.

---

#### E2E-NEW-635 — Rebase failure: continue with nothing in progress**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-224.
- **Preconditions:** FX-LINE, no operation in progress.
- **When** `git.rebase_continue {mount_id:"proj1"}` and `git.rebase_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** both fail `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** tip and volume unchanged.
- **Priority:** P0.

---

#### E2E-NEW-636 — Rebase failure: abort with nothing in progress**

**Category:** Failure. **Scenario:** SC-917. **Requirements:** FR-NEW-224.
- **Preconditions:** FX-LINE, no operation in progress.
- **When** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** tip unchanged, zero rows.
- **Priority:** P0.

---

#### E2E-NEW-637 — Rebase failure: starting a second rebase while one is paused**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-283, FR-NEW-225.
- **Preconditions:** FX-FORK conflicting, rebase paused on F1 (E2E-NEW-613); record the row's `current_step` and `todo`.
- **When** `git.rebase {onto:"main", todo:[pick F1, pick F2]}` again.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"rebase"` and `"already in progress"`, and naming `git.rebase_continue` and `git.rebase_abort` as the ways out.
- **And** there is still exactly **1** `git_operations` row, with the same `current_step` and `todo` as recorded — the second call did not overwrite state.
- **And** volume unchanged.
- **Priority:** P0.

---

#### E2E-NEW-803 — Happy — a second resolution call covering the rest lets the continue through**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-222, FR-NEW-221, FR-NEW-178.
- **Preconditions:** Band C harness with SEED-MULTI(3) ported onto `proj1`: `C0` commits `/f/000.txt`, `/f/001.txt`, `/f/002.txt` each `"line-A\n"`; `feature` = `C0` + `FM1` rewriting all three to `"line-FEATURE\n"`; `main` = `C0` + `CM1` rewriting all three to `"line-MAIN\n"`. HEAD on `feature`.
- **Given** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-FM1>"}]}` returns `status == "conflict"` with `conflicts[*].path == ["/f/000.txt","/f/001.txt","/f/002.txt"]`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/000.txt",strategy:"theirs"},{path:"/f/001.txt",strategy:"theirs"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/f/002.txt"` and `"unresolved"` (partial resolutions are recorded, the continue itself is refused).
- **When** `git.rebase_continue {resolutions:[{path:"/f/002.txt",strategy:"ours"}]}` — only the remaining path.
- **Then** `status == "completed"`, `replayed == 1`.
- **And** `/f/000.txt` == `b"line-FEATURE\n"`, `/f/001.txt` == `b"line-FEATURE\n"`, `/f/002.txt` == `b"line-MAIN\n"` (the resolutions of both calls were combined).
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **Verification:** error then success on the same tool; byte-exact `Env::read`; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-804 — Failure — continue with an empty resolutions array keeps the pause exactly**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-222.
- **Preconditions:** FX-FORK (F1 conflicts with C2 on `/a.txt` line 2). `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}` -> `status == "conflict"`, paused at entry index `0`.
- **Given** the `git_operations` row for `volume_id = "proj1"` holds `op_type == "rebase"`, `current_step == 0` (F1 is todo entry index 0, zero-based per FR-NEW-187), `total_steps == 2`, and a conflict set whose single element has `path == "/a.txt"`.
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:[]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"/a.txt"` and `"unresolved"`.
- **And** the `git_operations` row is still present with `current_step == 0`, `total_steps == 2` and a conflict set whose single element has `path == "/a.txt"` — byte-identical to the pre-call row except `updated_at`.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"` (the pre-rebase tip, unmoved).
- **And** `/a.txt` reads exactly `b"a1\nMAIN\n"` (the onto side; nothing was written).
- **And** a following `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}` succeeds, proving the pause was resumable and not corrupted.
- **Verification:** error code + substrings; two direct `git_operations` row reads compared field by field; ref read; byte compare; recovery call.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-805 — EdgeCase — repeated partial continues never advance the step index**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-222, FR-NEW-178, FR-NEW-177.
- **Preconditions:** as E2E-NEW-803 (three conflicting paths, one todo entry).
- **When** `git.rebase_continue {resolutions:[{path:"/f/000.txt",strategy:"ours"}]}` — rejected, `current_step` still reads `0`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/001.txt",strategy:"ours"}]}` — rejected, `current_step` still reads `0`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/999.txt",strategy:"ours"}]}` (a path not in the conflict set).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/f/999.txt"` and `"not in conflict"`; the call is all-or-nothing, so the previously recorded resolutions for `/f/000.txt` and `/f/001.txt` are retained and `/f/999.txt` is recorded nowhere.
- **And** after all three calls `current_step == 0` and `total_steps == 1` (a rebase always reports integers, never null; only single-step operations report null per FR-NEW-186), and `conflicts` still lists all three original paths.
- **And** `db.get_ref("refs/heads/feature").target` equals `<sha-of-FM1>` throughout (read before the first continue and after the third, compared for equality).
- **And** a final `git.rebase_continue {resolutions:[{path:"/f/002.txt",strategy:"ours"}]}` completes the rebase, confirming the two earlier partial resolutions survived three rejections.
- **Verification:** three error assertions; `git_operations` row read after each call; ref equality; terminal success.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-812 — Failure — `git.rebase_continue` cannot finish a paused pull**

**Category:** Failure. **Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-NEW-225.
- **Preconditions:** as E2E-NEW-508 (pull driven through `pull_branch`), paused with `conflicts[0].path == "/src/config.toml"`.
- **When** `git.rebase_continue {mount_id:"proj", resolutions:[{path:"/src/config.toml",strategy:"theirs"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"pull"`, `"git.merge_resolve"` and `"git.merge_abort"`.
- **And** the `git_operations` row still has `op_type == "merge"` (a pull is recorded as a merge, FR-NEW-285) and `conflicts == ["/src/config.toml"]`.
- **And** `/src/config.toml` line 2 is still `port = 8000` (the local side; nothing applied).
- **And** `git.cherry_pick_continue` with the same resolutions errors identically.
- **Verification:** error code + substrings on both calls; relational row read; byte compare.
- **Cleanup:** `git.merge_abort`. **Priority:** P0.

#### E2E-NEW-852 — SideEffect — the paused row holds every field the spec names**

**Category:** SideEffect. **Scenario:** SC-916. **Requirements:** FR-NEW-219, FR-NEW-275., FR-NEW-187
- **Preconditions:** FX-FORK4; pre-rebase `feature` tip `<sha-of-F4>`, `main` tip `<sha-of-C2>`.
- **When** `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}` pauses at entry index `0`.
- **Then** the single `git_operations` row for `volume_id='proj1'` has exactly: `op_type == "rebase"`, `state == "conflicted"`, `onto_sha == "<sha-of-C2>"`, `original_tip_sha == "<sha-of-F4>"`, `current_step == 0` (the pause is on F1, todo index 0), `total_steps == 4`, `conflicts` parsing to `["/a.txt"]`, `resolutions` parsing to an empty object, and `created_at == updated_at`.
- **And** `todo` parses to the four entries in the supplied order with actions `["pick","pick","pick","pick"]` and the four original shas.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F4>"` — the branch has not moved while paused at todo index 0.
- **Verification:** direct relational row read, field by field; JSON parse of `todo`/`conflicts`/`resolutions`; ref read.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

#### E2E-NEW-853 — EdgeCase — a conflict on the last todo entry pauses with `current_step == total_steps - 1`**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-219, FR-NEW-220.
- **Preconditions:** FX-FORK4 reordered so the conflicting entries come last: todo `[pick F2, pick F4, pick F1]` is invalid (FR-NEW-215 requires the full range), so instead use **FX-TAIL**: `main` = C1 + C2 (`/a.txt` line 2 = `MAIN`); `feature` = C1 + T1 (adds `/t1.txt` = `"t1\n"`) + T2 (sets `/a.txt` line 2 = `FEAT`, conflicting with C2).
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-T1>"},{action:"pick",sha:"<sha-of-T2>"}]}`.
- **Then** `status == "conflict"`, `current_step == 1` and `total_steps == 2` (the last of two entries is zero-based index 1, per FR-NEW-187).
- **And** the `git_operations` row has `current_step == 1`, `total_steps == 2`.
- **And** `/t1.txt` reads exactly `b"t1\n"` — step 1 stayed replayed, the pause is at a commit boundary (the replay tip holds T1's replayed commit, and `git.log` of the replay tip shows it).
- **And** `/a.txt` reads exactly `b"a1\nMAIN\n"` (step 2 applied nothing).
- **And** `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}` finishes the rebase with `replayed == 2`.
- **Verification:** the response's top-level `current_step` and `total_steps`; relational row; two byte reads; completion call.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-854 — SideEffect — the step index moves forward between pauses and never backwards**

**Category:** SideEffect. **Scenario:** SC-916. **Requirements:** FR-NEW-220, FR-NEW-275.
- **Preconditions:** FX-FORK4 (F1 and F3 conflict on `/a.txt`, F2 and F4 do not). Rebase started, paused on F1, which is todo index 0.
- **Given** the row reads `current_step == 0`, `total_steps == 4`, `updated_at == created_at`.
- **When** `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}`.
- **Then** the response is `status == "conflict"` with `current_step == 2` and `total_steps == 4` — todo indices 0 and 1 are done, the pause is at index 2 (the third entry), zero-based per FR-NEW-187.
- **And** the row now reads `current_step == 2`, `total_steps == 4`, `conflicts` holding exactly one element whose `path` is `/a.txt`, `resolutions` back to an empty object (the previous step's resolutions were consumed, not carried over), and `updated_at > created_at`.
- **And** `original_tip_sha` and `onto_sha` are unchanged from the first read (the abort target is fixed at start).
- **And** `/f2.txt` reads exactly `b"f2\n"` (entry 2 replayed during the resume).
- **Verification:** two relational row reads compared field by field; the response's `current_step` and `total_steps`; byte read.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

#### E2E-NEW-855 — EdgeCase — three pauses in one rebase**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-220, FR-NEW-221.
- **Preconditions:** **FX-FORK3C**: `main` = C1 + C2 (`/a.txt` line 2 = `MAIN`); `feature` = C1 + G1 + G2 + G3 where each of G1, G2, G3 sets `/a.txt` line 2 to `G1`, `G2`, `G3` respectively (every entry conflicts with the new base in turn).
- **When** `git.rebase {onto:"main", todo:[pick G1, pick G2, pick G3]}` then `git.rebase_continue` three times, each with `[{path:"/a.txt", strategy:"theirs"}]`.
- **Then** the three responses are, in order, `status == "conflict"` with `current_step == 0`, `status == "conflict"` with `current_step == 1`, then on the third continue `status == "completed"` with `replayed == 3`.

  (the first `conflict` comes from `git.rebase` itself; the first and second `rebase_continue` return the next conflict; the third returns `ok`.)
- **And** `/a.txt` reads exactly `b"a1\nG3\n"` — the last replayed side wins, each step having been resolved with `theirs`.
- **And** `git.log {ref_name:"feature"}` holds 5 commits with messages `["G3","G2","G1","C2 main edit","C1 base"]`.
- **And** no `git_operations` row remains.
- **Verification:** ordered response assertions; byte read; log messages; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-856 — EdgeCase — continue with literal content rather than a side**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-175.
- **Preconditions:** FX-FORK, rebase paused on `/a.txt` (ours = `"a1\nMAIN\n"`, theirs = `"a1\nFEAT1\n"`).
- **When** `git.rebase_continue {resolutions:[{path:"/a.txt", content:"a1\nMAIN+FEAT1\n"}]}`.
- **Then** `status == "completed"` and `replayed == 2`.
- **And** `/a.txt` reads exactly `b"a1\nMAIN+FEAT1\n"` — the supplied bytes verbatim, neither side's content and no re-merge attempted.
- **And** the commit created for the resolved entry has `message == "F1 feature edit"` (the original message is kept; only the content was supplied).
- **And** `/f.txt` reads `b"f1\n"` (entry 2 replayed after the resume).
- **Verification:** status/`replayed`; byte-exact read; `git.log` message; byte read.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-857 — Failure — continue while a merge, not a rebase, is in progress**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-225, FR-NEW-224.
- **Preconditions:** Band C `proj1` seeded like SEED-CONFLICT so that `git.merge {source_ref:"feature"}` pauses with `/src/config.toml` conflicting; the `git_operations` row has `op_type == "merge"`.
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"merge"`, `"git.merge_resolve"` and `"git.merge_abort"`.
- **And** the row still has `op_type == "merge"`, `conflicts == ["/src/config.toml"]` and no recorded resolution.
- **And** `git.rebase_abort` errors identically (containing `"merge"`), so neither rebase channel can touch the merge.
- **And** `git.merge_resolve` with the same resolution then succeeds.
- **Verification:** two error assertions; relational row read; completion call.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-874 — EdgeCase — the step counters track a multi-step rebase**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-NEW-220.
- **Preconditions:** FX-FORK4; rebase started with a 4-entry todo, paused at entry 1.
- **When** `git.status` is called, then `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"ours"}]}` pauses again, then `git.status` is called a second time.
- **Then** the first status reports `current_step == 0` and `total_steps == 4`, the second reports `current_step == 2` and `total_steps == 4` (F1 and F3 are todo indices 0 and 2, zero-based per FR-NEW-187).
- **And** both report `remaining_conflicts == ["/a.txt"]` and `continue_with == "git.rebase_continue"`, `abort_with == "git.rebase_abort"`.
- **And** the second status's `head` is still the pre-rebase `<sha-of-F4>`, because the branch ref moves only at completion.
- **Verification:** two `git.status` responses compared to exact objects; ref/field equality.
- **Cleanup:** `git.rebase_abort`. **Priority:** P1.

#### E2E-NEW-876 — Happy — a second member continues a rebase the first member started**

**Category:** Happy. **Scenario:** SC-929. **Requirements:** FR-NEW-282, FR-NEW-221.
- **Preconditions:** FX-FORK on `proj1` with `owner@test.com` and `second@test.com` both members (`admin.add_member`); `owner@test.com` starts `git.rebase {onto:"main", todo:[pick F1, pick F2]}`, paused on `/a.txt`.
- **When** `Env::as_person("second@test.com")` calls `git.rebase_continue {resolutions:[{path:"/a.txt", strategy:"theirs"}]}`.
- **Then** `status == "completed"` and `replayed == 2`.
- **And** `/a.txt` reads exactly `b"a1\nFEAT1\n"`.
- **And** the replayed commits' committer email is `"second@test.com"` while the author email stays `"owner@test.com"` — the continuation is attributed to whoever finished it, the authorship to whoever wrote it.
- **And** `audit("second@test.com", MOUNT)` holds an entry whose `op` is `"git.rebase_continue"`, and `audit(OWNER, MOUNT)` holds the earlier `"git.rebase"` entry.
- **And** no `git_operations` row remains.
- **Verification:** response; byte read; `git.log` author/committer fields; two audit reads; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-912 — EdgeCase — `merge_resolve` refuses to advance a rebase and says which tool would**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-198, FR-NEW-225.
Existing angles: 519 and 520 both cover "nothing in progress". This one covers "something else in progress".
- **Preconditions:** FX-FORK on `proj1`; rebase paused on `/a.txt` with `op_type == "rebase"`.
- **When** `git.merge_resolve {mount_id:"proj1", resolutions:[{path:"/a.txt", strategy:"ours"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"rebase"`, `"git.rebase_continue"` and `"git.rebase_abort"` — it does not merely say "no merge in progress", it names what *is* in progress.
- **When** `git.merge_abort {mount_id:"proj1"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"rebase"` and `"git.rebase_abort"`; the rebase is still paused (`git_operations` row unchanged, `current_step == 1`).
- **And** `git.rebase_abort` then succeeds and the row is gone.
- **Verification:** two error assertions with three substrings each; relational row read; abort success.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-914 — EdgeCase — dropping a commit a later entry depends on pauses instead of silently succeeding**

**Category:** EdgeCase. **Scenario:** SC-915. **Requirements:** FR-NEW-213, FR-NEW-219.
Existing angles: 602 (drop a leaf), 605 (drop everything). This one covers a dependent drop.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `main` = C1 + C2 (adds `/m.txt`); `feature` = C1 + D1 (adds `/dep.txt` = `"v1\n"`) + D2 (rewrites `/dep.txt` to `"v2\n"`). HEAD on `feature`.
- **When** `git.rebase {onto:"main", todo:[{action:"drop", sha:"<sha-of-D1>"},{action:"pick", sha:"<sha-of-D2>"}]}`.
- **Then** the result is `status == "conflict"` with one entry `path == "/dep.txt"`, `ours.exists == false` (the file does not exist on the new base after the drop), `theirs.exists == true` with content `"v2\n"`, `base.exists == true` with content `"v1\n"` — the dependency loss is surfaced, not silently resolved.
- **And** the `git_operations` row has `op_type == "rebase"`, `current_step == 1`, `total_steps == 2` (the last of two entries is zero-based index 1).
- **When** `git.rebase_continue {resolutions:[{path:"/dep.txt", strategy:"theirs"}]}`.
- **Then** `status == "completed"` and `/dep.txt` reads exactly `b"v2\n"`, with `git.log {ref_name:"feature"}` holding 3 commits (D1 dropped).
- **Verification:** conflict entry fields; relational row; completion response; byte read; log length.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-951 — Each operation type routes to exactly one completion pair**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-225
- **Preconditions:** Repo `gitproj` on `main` with commits `C1`, `C2`. Four independent runs, each starting from a clean volume at `C2`.
- **Steps:**
  - Given a conflicted `git.merge` of branch `feature` (both sides changed `/a.txt` line 1)
  - Then `git.merge_resolve {"/a.txt":"ours"}` completes it and `git.rebase_continue`, `git.cherry_pick_continue` and `git.revert_continue` each return `ERR_INVALID_ARGUMENT` whose message contains `merge`
  - And the same matrix holds for a conflicted `git.rebase` (only `git.rebase_continue` advances it), a conflicted `git.cherry_pick` (only `git.cherry_pick_continue`), a conflicted `git.revert` (only `git.revert_continue`), and a conflicted `git.stash_pop` (only `git.merge_resolve`)
  - And a conflicted `git.remote_pull` is advanced only by `git.merge_resolve`
- **Verification:** each call's error code and message substring; `SELECT op_type FROM git_operations WHERE volume_id='gitproj'` unchanged after every rejected call.
- **Cleanup:** abort each operation. **Priority:** P0

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
