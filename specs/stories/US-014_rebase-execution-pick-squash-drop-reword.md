# US-014: Rebase execution: pick, squash, drop, reword

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 14
> Depends On: US-013
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Replay the todo list: `pick` carries a commit over, `squash` folds it into its predecessor, `drop` omits it, `reword` replaces its message. Replayed commits get new shas and the originals become unreachable without being destroyed.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Commit creation follows `git.commit` (`crates/mcp-fs/src/tools/git.rs:439-517`): build the tree from the volume, take the write lock, import objects back. Git objects live in the blob store under `git:{sha}` and are exported to the bare repo before libgit2 reads, imported after it writes: `crates/mcp-fs/src/git/odb.rs:3-28`.
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-918:** Rebase supports `pick`, `squash`, `drop` and `reword` only; `edit`, `fixup`, `exec` and `break` are excluded. **Rationale:** `exec` would execute arbitrary commands on the server, which is a sandbox escape, and `edit`/`break` require an interactive pause with no conflict to resolve, which the operation model does not represent. `fixup` is `squash` without message concatenation and adds little. **Implemented by:** FR-NEW-210 through FR-NEW-216. **Round:** 1 (Q9 scope, refined at requirement drafting).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

5 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-211 [EARS-E]: `pick` replays a commit unchanged
> WHEN a todo entry's action is `pick` THE mcp-fs server SHALL replay that commit's change onto the current replay tip, preserving its original message and author, and SHALL set the committer to the caller.

- **Priority:** Must-have

### FR-NEW-212 [EARS-E]: `squash` folds a commit into its predecessor
> WHEN a todo entry's action is `squash` THE mcp-fs server SHALL combine that commit's change into the preceding replayed commit, producing one commit whose message concatenates both messages.

- **Priority:** Must-have

### FR-NEW-213 [EARS-E]: `drop` omits a commit
> WHEN a todo entry's action is `drop` THE mcp-fs server SHALL omit that commit's change from the replayed history entirely.

- **Priority:** Must-have

### FR-NEW-214 [EARS-E]: `reword` replaces a commit message
> WHEN a todo entry's action is `reword` THE mcp-fs server SHALL replay that commit with the supplied `message` and SHALL leave its tree identical to a `pick` of the same commit.

- **Priority:** Must-have

### FR-NEW-226 [EARS-E]: A rebase holds the write lock for its whole duration
> WHEN a rebase is replaying entries THE mcp-fs server SHALL hold the per-project git write lock across the entire replay, releasing it when the rebase completes, pauses or aborts.

- **Business Rules:** Releasing between entries would let a concurrent commit interleave into a partially replayed history.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

15 tests: Concurrency 2, DataIntegrity 1, EdgeCase 2, Failure 3, Happy 6, SideEffect 1.
Scenarios covered: SC-914, SC-915, SC-916, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-600 — Rebase happy: pick 2 commits onto main**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-211.
- **Preconditions:** FX-FORK. `main` tip = `<sha-of-C2>`, `feature` tip = `<sha-of-F2>`, checked out branch = `feature`. F1 edits `/a.txt` line 2 from nothing to `FEAT1`, and C2 sets it to `MAIN` — for this test only, change F1 to touch `/g.txt`=`"g1\n"` instead (no conflict); call this **FX-FORK-CLEAN** (F1 = `F1 feature edit`, adds `/g.txt`=`"g1\n"`; F2 = `F2 feature add`, adds `/f.txt`=`"f1\n"`).
- **Given** `git.log {ref_name:"feature"}` = `[F2, F1, C1]`.
- **When** `git.rebase {mount_id:"proj1", onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** result `status == "completed"` (the closed `git.rebase` status set of FR-NEW-199).
- **And** `git.log {ref_name:"feature"}` returns exactly 4 commits, messages in order `["F2 feature add", "F1 feature edit", "C2 main edit", "C1 base"]`. *Method:* map `commits[i]["message"]` (`git.rs:2161`).
- **And** `commits[0]["sha"] != <sha-of-F2>` and `commits[1]["sha"] != <sha-of-F1>` (new shas).
- **And** `commits[1]["parents"] == [<sha-of-C2>[..8]]` (short sha, `git.rs:2168`), `commits[0]["parents"] == [commits[1]["sha"][..8]]`.
- **And** every commit has exactly 1 parent: `commits[i]["parents"].len() == 1` for i in 0..3, `== 0` for `C1`.
- **And** `entry.db.get_ref("refs/heads/feature").target == commits[0]["sha"]` (`db.rs:214`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` (unchanged).
- **And** volume bytes: `/a.txt == b"a1\nMAIN\n"`, `/g.txt == b"g1\n"`, `/f.txt == b"f1\n"`. *Method:* `Env::read` (`git.rs:2465`), compared as bytes.
- **And** no `git_operations` row for `volume_id="proj1"`.
- **Cleanup:** `Fixture` drop (temp dirs). **Priority:** P0.

---

#### E2E-NEW-601 — Rebase happy: squash F2 into F1**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-212.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"squash", sha:"<sha-of-F2>"}]}`.
- **Then** `git.log {ref_name:"feature"}` has exactly **3** commits: messages `["F1 feature edit\n\nF2 feature add", "C2 main edit", "C1 base"]`. *Method:* exact string equality on `commits[0]["message"]` after `trim` (`git.rs:2161` already trims).
- **And** `commits[0]["parents"] == [<sha-of-C2>[..8]]` — the squashed commit sits directly on C2.
- **And** volume contains both `/g.txt == b"g1\n"` and `/f.txt == b"f1\n"` (the squashed tree is F2's tree).
- **And** neither `<sha-of-F1>` nor `<sha-of-F2>` appears in any `commits[i]["sha"]`.
- **Priority:** P0.

---

#### E2E-NEW-602 — Rebase happy: drop**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-213.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"drop", sha:"<sha-of-F2>"}]}`.
- **Then** `git.log{ref_name:"feature"}` = 3 commits, messages `["F1 feature edit","C2 main edit","C1 base"]`.
- **And** `/g.txt` reads `b"g1\n"`; reading `/f.txt` fails with `ERR_NOT_FOUND` (the dropped commit's file never enters the volume). *Method:* `VolumeClient::read_bytes("/f.txt")` returns `Err`, code `ERR_NOT_FOUND`.
- **And** the tip's tree has exactly 2 entries `a.txt`, `g.txt`. *Method:* `git.show {commit_sha: tip}` file list.
- **Priority:** P0.

---

#### E2E-NEW-603 — Rebase happy: reword**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"reword", sha:"<sha-of-F1>", message:"F1 reworded"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** `commits[1]["message"] == "F1 reworded"` exactly, `commits[0]["message"] == "F2 feature add"`.
- **And** `commits[1]["sha"] != <sha-of-F1>`.
- **And** the tree of `commits[1]` is byte-identical to F1's tree: `git.diff {from_ref: <sha-of-F1>, to_ref: commits[1]["sha"]}` returns an empty diff (no hunks).
- **And** `commits[1]["author_email"] == "owner@test.com"` (preserved from the original, `git.rs:2165`).
- **Priority:** P0.

---

#### E2E-NEW-604 — Rebase happy: squash chain of three**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-212.
- **Preconditions:** FX-FORK4 with F1..F4 rewritten to be conflict-free: F1 adds `/f1.txt`=`"1\n"`, F2 adds `/f2.txt`=`"2\n"`, F3 adds `/f3.txt`=`"3\n"`, F4 adds `/f4.txt`=`"4\n"`. Call this **FX-FORK4-CLEAN**.
- **When** `todo = [pick F1, squash F2, squash F3, pick F4]`, `onto:"main"`.
- **Then** `git.log{ref_name:"feature"}` has exactly **4** commits: `["F4 feat four", "F1 feat one\n\nF2 feat two\n\nF3 feat three", "C2 main edit", "C1 base"]`.
- **And** the squashed commit's tree contains `/f1.txt`,`/f2.txt`,`/f3.txt` and **not** `/f4.txt`.
- **And** volume has all four files with bytes `"1\n","2\n","3\n","4\n"`.
- **Priority:** P0.

---

#### E2E-NEW-605 — Rebase edge: all commits dropped**

**Category:** EdgeCase. **Scenario:** SC-915. **Requirements:** FR-NEW-213.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [drop F1, drop F2]`, `onto:"main"`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == <sha-of-C2>` — byte equal to `main`'s tip.
- **And** `git.log{ref_name:"feature"}` == `git.log{ref_name:"main"}` (same JSON value).
- **And** volume equals C2's tree exactly: `/a.txt == b"a1\nMAIN\n"`, `/g.txt` and `/f.txt` both absent (`ERR_NOT_FOUND`).
- **And** no `git_operations` row remains.
- **Priority:** P1.

---

#### E2E-NEW-606 — Rebase happy: single commit**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-211.
- **Preconditions:** FX-FORK-CLEAN, but `feature` = C1, F1 only.
- **When** `todo = [pick F1]`, `onto:"main"`.
- **Then** 3 commits on `feature`: `["F1 feature edit","C2 main edit","C1 base"]`; tip sha != `<sha-of-F1>`; `parents == [<sha-of-C2>[..8]]`.
- **Priority:** P1.

---

#### E2E-NEW-625 — Rebase failure: todo starts with squash**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215, FR-NEW-212.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"squash", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"squash"` and `"first"` — baseline: `git.rebase: todo cannot start with 'squash', there is no preceding commit to squash into`.
- **And** tip unchanged; zero rows; volume unchanged.
- **Priority:** P0.

---

#### E2E-NEW-629 — Rebase failure: reword to an empty message**

**Category:** Failure. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"reword", sha:"<sha-of-F1>", message:""},{action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"reword"` and `"message"` and `"empty"`.
- **And** tip unchanged; volume unchanged; zero rows — rejected before any replay.
- **Priority:** P0.

---

#### E2E-NEW-630 — Rebase failure: reword to whitespace-only**

**Category:** Failure. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `reword` with `message: "   \n\t \n"`.
- **Then** error `ERR_INVALID_ARGUMENT` with the same substrings as E2E-NEW-629 — because `git2::message_prettify` (used at `git.rs:690`) would reduce this to the empty string, so validation must run on the prettified form, not the raw one.
- **And** tip unchanged.
- **Priority:** P1.

---

#### E2E-NEW-694 — Write lock serializes the new ops**

**Category:** Concurrency. **Scenario:** SC-916. **Requirements:** FR-NEW-226.
- **Preconditions:** FX-FORK-CLEAN; a long-running rebase (todo of 50 clean picks on a fixture with 50 commits).
- **When** the rebase future and a `git.commit {message:"racer"}` future are driven concurrently with `tokio::join!`.
- **Then** both complete without error, and the final `git.log{ref_name:"feature"}` is internally consistent: exactly `50 + 1 (racer) + 2` commits, each with exactly 1 parent, and the parent chain is unbroken (every `commits[i]["parents"][0] == commits[i+1]["sha"][..8]`).
- **And** no commit is missing from the chain — proving the rebase held `entry.write_lock` (`repo.rs:44`) for its whole duration exactly as `git.commit` does (`git.rs:660`).
- **Priority:** P0.

---

#### E2E-NEW-699 — Ref duality: db and on-disk agree after every new mutating op**

**Category:** DataIntegrity. **Scenario:** SC-929. **Requirements:** FR-NEW-115, FR-NEW-226.
- **Preconditions:** FX-LINE / FX-FORK-CLEAN / FX-MERGE as needed.
- **When** in sequence on the same env: a clean rebase, a cherry-pick, a `reset --soft`, a `reset --hard`, a revert, and a `rebase_abort` after a paused rebase.
- **Then** after **each** step, for every ref in `entry.db.list_refs()` (`db.rs:261`) that is non-symbolic, `repo.find_reference(name).target().to_string()` equals the db `target` — the dual write invariant established by `git.commit` (`git.rs:702-711`) holds for every new operation.
- **And** the symbolic `HEAD` row still points at the expected `refs/heads/...` and `repo.head()` agrees.
- **And** `entry.objects` contains every new commit sha (`object_exists`, `db.rs:131`), proving the `import_from_repo` step (`git.rs:700`) was not skipped by the new code paths.
- **Priority:** P0.

---

##### C.z Implementation notes (band C)

1. Five assertions in this spec pin a decision the conflict-model author left open. Each is written as a **fixed baseline** so the test is writable without guessing: `current_step` identifies the todo entry (E2E-NEW-613), branch == onto is a rejection not a no-op (E2E-NEW-608), `todo` bound is 1000 (E2E-NEW-626), hard reset writes every file of the target tree (E2E-NEW-665, E2E-NEW-668), cherry-pick of a merge is rejected (E2E-NEW-650). If the implementation chooses otherwise, change the baseline in `TOOL_CONTRACT.txt` and the test together, never only the test.
2. The revert conflict pair `git.revert_continue` / `git.revert_abort` is now mandated explicitly by FR-NEW-241 and FR-NEW-266, and is exercised by E2E-NEW-684. Revert conflicts are NOT routed through `git.cherry_pick_continue`; FR-NEW-225 forbids it.
3. Tests live next to the existing git tool tests (`crates/mcp-fs/src/tools/git.rs` `mod tests`, harness at `:2437-2476`), reusing `Env`, `MOUNT`, `OWNER`. Byte-level volume assertions need a `read_bytes` helper alongside the existing `read` (`git.rs:2465`), and a `git_operations` row-count helper on the test side.
4. Quality gate unchanged: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

#### E2E-NEW-834 — EdgeCase — a rebase step whose sides are disjoint replays without pausing**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-173, FR-NEW-211.
- **Preconditions:** Band C, `proj1`: `C1` commits `/cfg.toml` = `"a = 1\nb = 2\nc = 3\nd = 4\ne = 5\n"`; `main` adds `C2` setting line 1 to `"a = 99\n"`; `feature` off `C1` adds `F1` setting line 5 to `"e = 99\n"`. HEAD on `feature`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"}]}`.
- **Then** `status == "completed"` — no `conflicts` key, no `operation_id` key in the response.
- **And** `/cfg.toml` reads exactly `b"a = 99\nb = 2\nc = 3\nd = 4\ne = 99\n"` (both edits present).
- **And** `git.log {ref_name:"feature"}` holds 3 commits, the tip's single parent being `<sha-of-C2>`.
- **And** no `git_operations` row for `volume_id = "proj1"` was created at any point (asserted immediately after the call).
- **Verification:** JSON key absence; byte-exact read; log parents; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-913 — SideEffect — `pick` preserves author identity and timestamp, and sets the committer to the caller**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-211.
Existing angles: 600 and 606 are both Happy replays. This one pins the identity rule FR-NEW-211 states.
- **Preconditions:** FX-FORK-CLEAN where F1 and F2 were committed by `author@test.com` (a member), with author timestamps `T1` and `T2` captured from `git.log` before the rebase; `second@test.com` is also a member and performs the rebase.
- **When** `Env::as_person("second@test.com")` calls `git.rebase {onto:"main", todo:[pick F1, pick F2]}`.
- **Then** the replayed commits' `author_email` are both `"author@test.com"` and their author timestamps are exactly `T1` and `T2`, unchanged.
- **And** their `committer_email` are both `"second@test.com"` and their committer timestamps are >= the test's start time (new commits).
- **And** the replayed commit messages are exactly `"F1 feature edit"` and `"F2 feature add"`.
- **And** their shas differ from `<sha-of-F1>` and `<sha-of-F2>` (the committer change alone guarantees a new sha).
- **Verification:** `git.log` author/committer fields and timestamps; message equality; sha inequality.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-917 — Concurrency — `git.branch_create` cannot interleave into a running rebase**

**Category:** Concurrency. **Scenario:** SC-916. **Requirements:** FR-NEW-226, FR-NEW-115.
Existing angles: 694 (rebase vs `git.commit`), 699 (ref consistency sweep). This one covers a branch tool against the held lock.
- **Preconditions:** FX-FORK-CLEAN with 50 replayable commits on `feature` so the replay takes measurable time; a barrier that releases both futures simultaneously (the technique of E2E-NEW-694).
- **When** `git.rebase {onto:"main", todo:[pick S1 .. pick S50]}` and `git.branch_create {name:"race/x", start_point:"feature"}` are driven concurrently with `tokio::join!`.
- **Then** both futures complete; neither panics; neither returns `ERR_INTERNAL_ERROR`.
- **And** `refs/heads/race/x` points at **either** `<sha-of-S50>` (created before the rebase took the lock) **or** the rebase's final tip — never at an intermediate replayed sha. Asserted as membership in that two-element set, with the intermediate shas enumerated from `git.log` and asserted absent.
- **And** `git.log {ref_name:"feature"}` holds exactly 52 commits with no duplicate sha.
- **And** no `git_operations` row remains.
- **Verification:** `tokio::join!` results; ref value set membership; log length and sha uniqueness; relational count.
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
