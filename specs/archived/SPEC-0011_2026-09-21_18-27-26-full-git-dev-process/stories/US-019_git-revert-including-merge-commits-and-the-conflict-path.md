# US-019: git.revert, including merge commits and the conflict path

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 19
> Depends On: US-003, US-017
> Complexity: L
> min_tier: 2
> Files touched: 1

## Objective

Undo a commit by adding its inverse rather than rewriting history, including the case that needs an explicit decision: reverting a merge commit requires choosing which parent is the mainline.

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

7 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-260 [EARS-E]: Revert creates an inverse commit
> WHEN `git.revert` is called with `commit_sha` THE mcp-fs server SHALL create a new commit on the current branch whose change is the inverse of the named commit, and SHALL leave the named commit in history.

- **Inputs:** `mount_id`, `commit_sha`, `mainline` (integer, optional), `message` (string, optional).
- **Outputs:** `{status, new_sha, reverted_sha}`.
- **Business Rules:** Default message is `Revert "{original subject}"`.
- **Priority:** Must-have

### FR-NEW-261 [EARS-O]: Reverting a merge commit requires a mainline
> IF `commit_sha` names a commit with more than one parent and `mainline` is absent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` explaining that reverting a merge requires choosing which parent is the mainline.

- **Priority:** Must-have

### FR-NEW-262 [EARS-O]: Mainline must be in range and only for merges
> IF `mainline` is supplied and is less than 1, or exceeds the commit's parent count, or the commit has a single parent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` naming the commit's actual parent count.

- **Priority:** Must-have

### FR-NEW-263 [EARS-E]: Revert of the initial commit is handled
> WHEN `commit_sha` names a commit with no parent THE mcp-fs server SHALL compute the inverse against the empty tree, producing a commit that removes everything the initial commit added.

- **Priority:** Should-have

### FR-NEW-264 [EARS-O]: A conflicting revert enters the conflict model
> IF the inverse change does not apply cleanly to the current tree THEN THE mcp-fs server SHALL record an in-progress operation of type `revert` and SHALL return the conflict response of FR-NEW-170.

- **Priority:** Must-have

### FR-NEW-265 [EARS-U]: Reverting a revert restores the original change
> The mcp-fs server SHALL produce, when a revert commit is itself reverted, a tree identical to the one before the first revert.

- **Priority:** Should-have

### FR-NEW-266 [EARS-E]: Continue and abort a revert
> WHEN `git.revert_continue` is called with `resolutions` THE mcp-fs server SHALL complete the paused revert and create the inverse commit, and WHEN `git.revert_abort` is called THE mcp-fs server SHALL restore HEAD and the volume to their exact pre-operation state and clear the record.

- **Business Rules:** These exist as their own pair rather than reusing the cherry-pick tools, per FR-NEW-241 and DEC-904, because `git revert --continue` and `git revert --abort` are the vocabulary the caller already has.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

25 tests: EdgeCase 4, Failure 9, Happy 7, SideEffect 5.
Scenarios covered: SC-918, SC-922, SC-923, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-675 — Revert happy: revert C3**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; volume `/a.txt == b"a1\na3\n"`. C3's diff added the line `a3` to `/a.txt`.
- **When** `git.revert {mount_id:"proj1", commit_sha:"<sha-of-C3>"}`.
- **Then** `status == "committed"`, `result["new_sha"]` is a new 40-char sha differing from every seed sha.
- **And** `git.log{ref_name:"main"}` = exactly **5** commits; `commits[0]["message"] == "Revert \"C3 edit a\""` exactly (the standard revert subject; asserted verbatim), `commits[1]["message"] == "C4 add d"`.
- **And** `commits[0]["parents"] == [<sha-of-C4>[..8]]`, parent count 1.
- **And** `<sha-of-C3>` still appears in the log at index 2 — the original stays in history.
- **And** volume: `/a.txt == b"a1\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"` (C4's addition survives; only C3's diff is inverted).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

#### E2E-NEW-676 — Revert happy: revert a file-add commit**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; C4 added `/d.txt`.
- **When** `git.revert {commit_sha:"<sha-of-C4>"}`.
- **Then** `/d.txt` absent (`ERR_NOT_FOUND`); path set exactly `{"/a.txt","/b.txt"}`; `/a.txt == b"a1\na3\n"` unchanged.
- **And** `git.log` = 5 commits with `commits[0]["message"] == "Revert \"C4 add d\""`.
- **And** the revert commit's tree is identical to C3's tree: `git.diff {from_ref:"<sha-of-C3>", to_ref: result["new_sha"]}` returns an empty diff.
- **Priority:** P0.

---

#### E2E-NEW-677 — Revert happy: reverting a revert reapplies**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-265.
- **Preconditions:** E2E-NEW-676 executed; `<sha-of-R1>` = the revert commit; `/d.txt` absent.
- **When** `git.revert {commit_sha:"<sha-of-R1>"}`.
- **Then** `/d.txt == b"d1\n"` again, byte-identical to the original.
- **And** `git.log{ref_name:"main"}` = 6 commits, `commits[0]["message"] == "Revert \"Revert \\\"C4 add d\\\"\""` — i.e. the literal string `Revert "Revert \"C4 add d\""` as git renders nested reverts; asserted as the exact string produced by the implementation's subject rule, which the spec fixes as `Revert "<subject of the reverted commit>"`.
- **And** the new tip's tree is identical to C4's tree: `git.diff {from_ref:"<sha-of-C4>", to_ref: tip}` is empty.
- **And** all three commits C4, R1, R2 remain reachable with distinct shas.
- **Priority:** P0.

---

#### E2E-NEW-678 — Revert merge commit with mainline=1**

**Category:** Happy. **Scenario:** SC-923. **Requirements:** FR-NEW-261.
- **Preconditions:** FX-MERGE; `main` tip = `<sha-of-MG>` (parents `[<sha-of-M1>, <sha-of-S1>]`); volume has `/a.txt`,`/m.txt`,`/s.txt`.
- **When** `git.revert {commit_sha:"<sha-of-MG>", mainline:1}`.
- **Then** `status == "committed"`; the revert is computed against parent 1 (`M1`), so the *side* branch's contribution is removed: `/s.txt` absent, `/m.txt == b"m1\n"`, `/a.txt == b"a1\n"`.
- **And** `git.log{ref_name:"main"}` `commits[0]["message"] == "Revert \"Merge side into main\""` and `commits[0]["parents"] == [<sha-of-MG>[..8]]`, parent count 1 (the revert itself is not a merge).
- **And** `<sha-of-MG>` remains reachable at index 1.
- **Priority:** P0.

---

#### E2E-NEW-679 — Revert merge commit with mainline=2**

**Category:** EdgeCase. **Scenario:** SC-923. **Requirements:** FR-NEW-261, FR-NEW-262.
- **Preconditions:** FX-MERGE, same as above.
- **When** `git.revert {commit_sha:"<sha-of-MG>", mainline:2}`.
- **Then** the revert is computed against parent 2 (`S1`), so the *main* branch's contribution is removed: `/m.txt` absent, `/s.txt == b"s1\n"`, `/a.txt == b"a1\n"`.
- **And** the resulting tree differs from the E2E-NEW-678 result: the two runs produce different tip trees (asserted by comparing the two path sets: `{"/a.txt","/m.txt"}` vs `{"/a.txt","/s.txt"}`).
- **And** commit count on `main` == 5 in both runs (C1, M1/S1, MG, revert — exact list asserted per fixture layout).
- **Priority:** P0.

---

#### E2E-NEW-680 — Revert the initial commit (no parent)**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-263.
- **Preconditions:** FX-INIT: only `C1 base` with `/a.txt == b"a1\n"`; tip `<sha-of-C1>`.
- **When** `git.revert {commit_sha:"<sha-of-C1>"}`.
- **Then** `status == "committed"`; the inverse of "add `/a.txt` against the empty tree" removes it: `/a.txt` absent, volume path set is **empty**.
- **And** `git.log{ref_name:"main"}` = 2 commits, `commits[0]["message"] == "Revert \"C1 base\""`, `parents == [<sha-of-C1>[..8]]`.
- **And** the revert commit's tree is the empty tree (`git.show` lists zero files).
- **And** `<sha-of-C1>` still reachable.
- **Priority:** P0.

---

#### E2E-NEW-681 — Revert a non-tip commit**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; C2 added `/b.txt`; C3 and C4 do not touch `/b.txt`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}`.
- **Then** `/b.txt` absent; `/a.txt == b"a1\na3\n"` and `/d.txt == b"d1\n"` untouched (later commits are preserved).
- **And** `git.log` = 5 commits, `commits[0]["message"] == "Revert \"C2 add b\""`.
- **Priority:** P1.

---

#### E2E-NEW-682 — Revert side-effect: audit, quota, original reachable**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at C4; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.revert {commit_sha:"<sha-of-C3>"}` (rewrites `/a.txt` to 3 bytes).
- **Then** `audit()[n0..]` contains an entry with `op == "git.revert"` exactly and `detail` containing `<sha-of-C3>`.
- **And** `bytes_written() - q0 == 3` (only `/a.txt` is rewritten; `/b.txt`,`/d.txt` are unchanged by the inverse diff and are not re-written).
- **And** `entry.db.object_exists("<sha-of-C3>") == true` and `<sha-of-C3>` is in `git.log` — the original is untouched.
- **Priority:** P1.

---

#### E2E-NEW-683 — Revert conflict: inverse diff does not apply**

**Category:** Failure. **Scenario:** SC-922. **Requirements:** FR-NEW-264.
- **Preconditions:** FX-LINE at C4, plus C5 (`C5 rewrite a`) setting `/a.txt` = `"TOTALLY DIFFERENT\n"`, tip `<sha-of-C5>`. Reverting C3 (which added line `a3` to a file that no longer contains it) cannot apply cleanly.
- **When** `git.revert {commit_sha:"<sha-of-C3>"}`.
- **Then** `status == "conflict"`, `result["conflicts"] == ["/a.txt"]`.
- **And** exactly one `git_operations` row with `volume_id='proj1'`, `op_type='revert'`, in-progress state, `todo` holding `<sha-of-C3>`.
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C5>` — unchanged.
- **And** `/a.txt == b"TOTALLY DIFFERENT\n"` byte-identical; no conflict markers anywhere (`fs.grep` for marker lines returns zero matches).
- **Priority:** P0.

---

#### E2E-NEW-684 — Revert conflict: continue resolves, abort restores exactly**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-264, FR-NEW-284., FR-NEW-186
- **Preconditions:** E2E-NEW-683 state; `tip_before = <sha-of-C5>`; full volume byte map.
- **When (a)** `git.revert_continue {mount_id:"proj1", resolutions:{"/a.txt":{"content":"RESOLVED\n"}}}`.
- **Then (a)** `status == "committed"`; `/a.txt == b"RESOLVED\n"`; `git.log` `commits[0]["message"] == "Revert \"C3 edit a\""` with `parents == [<sha-of-C5>[..8]]`; zero `git_operations` rows.
- **When (b)** (separate run from the same precondition) `git.revert_abort {mount_id:"proj1"}`.
- **Then (b)** `entry.db.get_ref("refs/heads/main").target == tip_before`; every volume path and its bytes equal the map; zero rows; a second `git.revert_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no revert in progress"`.
- **And (b)** `git.revert_continue {}` with nothing in progress fails `ERR_INVALID_ARGUMENT` containing `"no revert in progress"`.
- **Priority:** P0.

---

#### E2E-NEW-685 — Revert failure: merge commit without mainline**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-261.
- **Preconditions:** FX-MERGE at `<sha-of-MG>`; volume snapshot.
- **When** `git.revert {commit_sha:"<sha-of-MG>"}` (no `mainline`).
- **Then** `ERR_INVALID_ARGUMENT` whose message contains `"merge commit"`, `"mainline"`, and `"1"` and `"2"`. Baseline: `git.revert: '<short-MG>' is a merge commit with 2 parents; pass mainline (1 or 2) to choose which parent the revert is computed against`.
- **And** tip == `<sha-of-MG>`; volume path set and bytes equal the snapshot; `bytes_written` unchanged; zero rows.
- **Priority:** P0.

---

#### E2E-NEW-686 — Revert failure: mainline out of range**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **Preconditions:** FX-MERGE (2 parents).
- **When** `mainline: 3`, then `mainline: 99`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"mainline"`, the supplied number, and `"2 parents"`. Baseline: `git.revert: mainline 3 is out of range, '<short-MG>' has 2 parents`.
- **And** tip unchanged; volume unchanged.
- **Priority:** P0.

---

#### E2E-NEW-687 — Revert failure: mainline is 1-based**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **When** `mainline: 0`, then `mainline: -1`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"1"` (stating parents are numbered from 1).
- **And** `mainline: "1"` (string instead of integer) fails `ERR_INVALID_ARGUMENT` naming `"mainline"` and `"integer"`.
- **And** tip unchanged.
- **Priority:** P0.

---

#### E2E-NEW-688 — Revert failure: mainline supplied for a non-merge commit**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **Preconditions:** FX-LINE at C4; C3 has exactly 1 parent.
- **When** `git.revert {commit_sha:"<sha-of-C3>", mainline:1}`.
- **Then** `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"not a merge commit"` — the argument is refused rather than silently ignored, so a caller confusing two commits learns about it.
- **And** tip == `<sha-of-C4>`; volume unchanged; zero rows.
- **Priority:** P1.

---

#### E2E-NEW-689 — Revert failure: unknown and malformed sha**

**Category:** Failure. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **When** on FX-LINE: `commit_sha:"0000000000000000000000000000000000000000"`; then `commit_sha:"zzz"`; then no `commit_sha`.
- **Then** respectively `ERR_NOT_FOUND` containing the sha and `"not found"`; `ERR_INVALID_ARGUMENT` containing `"commit_sha"`; `ERR_INVALID_ARGUMENT` naming `"commit_sha"`.
- **And** in all three cases tip == `<sha-of-C4>`, volume bytes unchanged, `bytes_written` unchanged, `audit().len()` unchanged, zero rows.
- **Priority:** P0.

---

##### Cross-cutting (E2E-NEW-690 .. E2E-NEW-699)

---

#### E2E-NEW-860 — Failure — an out-of-range mainline names the actual parent count**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-240, FR-NEW-262.
- **Preconditions:** FX-MERGE (`MG` has parents `[M1, S1]`), HEAD on a branch `other` off `C1`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:3}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"3"` and `"2"` (the supplied index and the commit's actual parent count).
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:0}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"0"` — the index is 1-based.
- **When** `git.cherry_pick {commit_sha:"<sha-of-M1>", mainline:1}` (a single-parent commit).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"1 parent"`.
- **And** after all three calls, `db.get_ref("refs/heads/other").target` is unchanged and no `git_operations` row exists.
- **Verification:** three error assertions; ref read; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-866 — Happy — reverting the initial commit empties the tree**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-263, FR-NEW-260.
- **Preconditions:** FX-INIT (only `C1`, `/a.txt` = `"a1\n"`, no parent), HEAD on `main` = `<sha-of-C1>`.
- **When** `git.revert {mount_id:"proj1", commit_sha:"<sha-of-C1>"}`.
- **Then** `status == "committed"`, `reverted_sha == "<sha-of-C1>"`, `new_sha` a 40-hex string different from it.
- **And** `VolumeClient::read_bytes("/a.txt")` errors `ERR_NOT_FOUND` and `fs.list {path:"/"}` returns an empty entry array.
- **And** `git.show {commit_sha:new_sha}` lists zero files in the resulting tree and reports exactly 1 parent, `<sha-of-C1>`.
- **And** the commit message is exactly `Revert "C1 base"`.
- **Verification:** response fields; volume read error; `fs.list`; `git.show`; message equality.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-867 — SideEffect — the reverted initial commit stays in history and stays readable**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-263, FR-NEW-255.
- **Preconditions:** as E2E-NEW-866, after the revert.
- **When** `git.log {ref_name:"main"}` is read.
- **Then** it holds exactly 2 commits, messages `["Revert \"C1 base\"", "C1 base"]`, and `commits[1]["sha"] == "<sha-of-C1>"`.
- **And** `db.object_exists("<sha-of-C1>")` is `true` and `git.show {commit_sha:"<sha-of-C1>"}` still lists `/a.txt` with content `"a1\n"`.
- **And** `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}` restores `/a.txt` to exactly `b"a1\n"`, proving the revert removed content from the working tree only, never from the object store.
- **Verification:** log array; `object_exists`; `git.show` payload; reset plus byte compare.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-868 — EdgeCase — a chain of three reverts lands on the removed state**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-265.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `C2` adds `/b.txt` = `"b1\n"`. HEAD on `main` = `<sha-of-C2>`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}` -> `R1`; `git.revert {commit_sha:R1}` -> `R2`; `git.revert {commit_sha:R2}` -> `R3`.
- **Then** after `R1`, `/b.txt` is absent; after `R2`, `/b.txt` reads exactly `b"b1\n"`; after `R3`, `/b.txt` is absent again.
- **And** the tree of `R3` is identical to the tree of `R1`: `git.diff {from_ref:R1, to_ref:R3}` returns an empty diff (no hunks).
- **And** `/a.txt` reads exactly `b"a1\n"` at every step (untouched throughout).
- **Verification:** three volume states; `git.diff` emptiness; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

#### E2E-NEW-869 — SideEffect — each revert is its own commit and the original is never rewritten**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-265, FR-NEW-260.
- **Preconditions:** as E2E-NEW-868 after `R1` and `R2`.
- **When** `git.log {ref_name:"main"}` is read.
- **Then** it holds exactly 4 commits with distinct shas, messages in order `["Revert \"Revert \\\"C2 add b\\\"\"", "Revert \"C2 add b\"", "C2 add b", "C1 base"]`.
- **And** `commits[2]["sha"] == "<sha-of-C2>"` — the original commit object is the same one, not a rewritten copy.
- **And** every commit has exactly 1 parent (`parents.len() == 1`) except `C1` (`0`); a revert is a forward commit, not a history edit.
- **And** `git.show {commit_sha:"<sha-of-C2>"}` still reports it adding `/b.txt` with `"b1\n"`.
- **Verification:** log messages and shas; parent counts; `git.show`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-922 — SideEffect — a conflicted revert creates no commit and leaves the volume byte-identical**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-264, FR-NEW-171.
Existing angles: 683 (the conflict is reported), 684 (continue/abort). This one asserts the "applies nothing" half.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `C2` sets `/a.txt` = `"a1\na2\n"`; `C3` sets `/a.txt` = `"a1\nREWRITTEN\n"`. HEAD on `main` = `<sha-of-C3>`. Capture `before_objects`, `before_bytes`, `before_audit`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}` (its inverse does not apply to `C3`'s tree).
- **Then** `status == "conflict"`, `operation == "revert"`, one entry `path == "/a.txt"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C3>"` and `git.log {ref_name:"main"}` still holds 3 commits.
- **And** `/a.txt` reads exactly `b"a1\nREWRITTEN\n"`, and the volume contains no file holding `"<<<<<<<"`, `"======="` or `">>>>>>>"` (scan every path under `/`).
- **And** `COUNT(*) FROM git_objects` equals `before_objects` and `bytes_written` equals `before_bytes`.
- **And** exactly one `git_operations` row exists with `op_type == "revert"`.
- **Verification:** response fields; ref and log; byte compare plus marker scan; relational counts; quota.
- **Cleanup:** `git.revert_abort` (or `git.cherry_pick_abort` per the registered revert abort tool). **Priority:** P0.

---

#### E2E-NEW-952 — `git.revert_continue` is rejected while a rebase is paused**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-225
- **Preconditions:** A rebase of `feature` onto `main` paused at step 2 with `/a.txt` conflicting.
- **Steps:**
  - Given `git_operations` holds one row with `op_type='rebase'`, `current_step=2`
  - When `git.revert_continue {"resolutions":{"/a.txt":"ours"}}` is called
  - Then the response is `ERR_INVALID_ARGUMENT` and the message contains `rebase` and `git.rebase_continue`
  - And the `git_operations` row still has `op_type='rebase'` and `current_step=2`, unchanged
  - And no commit was created: `git.log` length is identical to before the call
- **Verification:** error code and substrings; row field equality; log length.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0

#### E2E-NEW-954 — `git.revert_continue` completes a conflicted revert**
- **Category:** Happy | **Scenario:** SC-922 | **Requirements:** FR-NEW-266, FR-NEW-264
- **Preconditions:** `C1` commits `/cfg.txt` = `mode=a\n`. `C2` changes it to `mode=b\n`. `C3` changes it to `mode=c\n`. HEAD is `C3`.
- **Steps:**
  - Given `git.revert {"commit_sha":"<sha-of-C2>"}` returns `status:"conflict"` because the inverse of `C2` does not apply over `C3`, with `/cfg.txt` in the conflict set
  - And `git_operations` holds one row with `op_type='revert'`
  - When `git.revert_continue {"resolutions":{"/cfg.txt":{"content":"mode=a\n"}}}` is called
  - Then a new commit exists whose parent is `<sha-of-C3>` and whose `/cfg.txt` blob is exactly `mode=a\n`
  - And `/cfg.txt` in the volume is exactly `mode=a\n`
  - And `SELECT COUNT(*) FROM git_operations WHERE volume_id='gitproj'` is 0
  - And `<sha-of-C2>` is still reachable from the new tip
- **Verification:** commit parent and blob bytes via `git.show`; volume bytes; row count; `git.log` contains `C2`.
- **Cleanup:** none. **Priority:** P0

#### E2E-NEW-955 — `git.revert_abort` restores the tip and every byte exactly**
- **Category:** SideEffect | **Scenario:** SC-922 | **Requirements:** FR-NEW-266, FR-NEW-171
- **Preconditions:** As E2E-NEW-954, paused on the conflicted revert. Snapshot the branch tip sha and a map of every volume path to its bytes beforehand.
- **Steps:**
  - When `git.revert_abort {}` is called
  - Then `refs/heads/main` equals the snapshotted `<sha-of-C3>`
  - And the rebuilt path-to-bytes map equals the snapshot exactly, key for key and byte for byte
  - And `SELECT COUNT(*) FROM git_operations WHERE volume_id='gitproj'` is 0
  - And no commit object was created: `git.log` length equals the pre-revert length
- **Verification:** ref value; full volume byte map comparison; row count; log length.
- **Cleanup:** none. **Priority:** P0

#### E2E-NEW-956 — `git.revert_continue` with no revert in progress**
- **Category:** Failure | **Scenario:** SC-922 | **Requirements:** FR-NEW-266
- **Preconditions:** Clean volume at `C3`, `git_operations` empty.
- **Steps:**
  - When `git.revert_continue {"resolutions":{}}` is called
  - Then the response is `ERR_INVALID_ARGUMENT` and the message contains `no revert is in progress`
  - And no row is created in `git_operations`
- **Verification:** error code and substring; row count is 0.
- **Cleanup:** none. **Priority:** P1

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
