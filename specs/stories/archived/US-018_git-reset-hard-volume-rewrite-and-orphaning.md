# US-018: git.reset hard: volume rewrite and orphaning

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 18
> Depends On: US-017
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Move the branch pointer and rewrite the volume to match, discarding uncommitted changes. Commits above the target are orphaned but never pruned, which is what makes a mistaken hard reset recoverable through the reported `old_sha`.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-905:** `git.reset` offers `soft` and `hard` only; there is no `mixed` mode. **Rationale:** `mixed` differs from `soft` only in what it does to the index, and this server has no index, so the two would be indistinguishable. Offering a third mode that silently behaves like the first would be a trap. **Alternatives considered:** accept `mixed` as an alias of `soft` for familiarity, rejected as misleading. **Implemented by:** FR-NEW-250, FR-NEW-251, FR-NEW-252. **Round:** 1 (P4). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:196-222` (commit takes the whole volume, no index).

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Volume** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

2 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-251 [EARS-E]: Hard reset moves the pointer and the volume
> WHEN `git.reset` is called with `mode` `hard` THE mcp-fs server SHALL move the current branch ref to `target_ref` and SHALL rewrite the volume so its contents equal that commit's tree exactly, discarding uncommitted changes.

- **Priority:** Must-have

### FR-NEW-255 [EARS-U]: Reset orphans rather than destroys
> The mcp-fs server SHALL leave commits that become unreachable through a reset present in the object store, and SHALL NOT prune them as part of the reset.

- **Business Rules:** This is what makes a mistaken reset recoverable by `git.branch_reset` back to the reported `old_sha`.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

16 tests: DataIntegrity 2, EdgeCase 5, Failure 3, Happy 2, SideEffect 4.
Scenarios covered: SC-914, SC-917, SC-920, SC-921, SC-924, SC-930.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-609 — Rebase side-effect: originals unreachable but objects retained**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-255.
- **Preconditions:** E2E-NEW-600 executed (record `<sha-of-F1>`, `<sha-of-F2>`).
- **Then** neither original sha appears in `git.log` for any ref: for each ref in `entry.db.list_refs()` (`db.rs:261`), no `commits[i]["sha"]` equals them.
- **And** `entry.db.object_exists("<sha-of-F1>") == true` and same for F2 (`db.rs:131`) — orphaned, not destroyed.
- **And** `git.show {commit_sha:"<sha-of-F1>"}` still succeeds and returns `commit.message == "F1 feature edit"`.
- **Priority:** P0.

---

#### E2E-NEW-661 — Reset hard to C2**

**Category:** Happy. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE on `main`, volume = C4's tree.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>`.
- **And** `git.log{ref_name:"main"}` = 2 commits `["C2 add b","C1 base"]`.
- **And** volume exactly equals C2's tree: `/a.txt == b"a1\n"` (C3's edit reverted), `/b.txt == b"b1\n"`, and `/d.txt` absent (`read_bytes` -> `ERR_NOT_FOUND`).
- **And** the volume path set is exactly `{"/a.txt","/b.txt"}`. *Method:* recursive `fs.list`.
- **Priority:** P0.

---

#### E2E-NEW-662 — Reset hard deletes files added after the target**

**Category:** Happy. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE plus C5 (`C5 add e`, `/e.txt`=`"e1\n"`, nested `/sub/deep.txt`=`"deep\n"`), tip `<sha-of-C5>`.
- **When** `git.reset {target_ref:"<sha-of-C4>", mode:"hard"}`.
- **Then** `/e.txt` and `/sub/deep.txt` both absent (`ERR_NOT_FOUND` on read), and the directory `/sub` is gone from `fs.list` output.
- **And** the remaining path set is exactly `{"/a.txt","/b.txt","/d.txt"}` with bytes `"a1\na3\n"`, `"b1\n"`, `"d1\n"`.
- **And** tip == `<sha-of-C4>`.
- **Priority:** P0.

---

#### E2E-NEW-663 — Reset hard on a dirty volume**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE on `main` at `<sha-of-C4>`; then, **without committing**, `Env::write("/a.txt", "DIRTY\n")` and `Env::write("/new.txt", "n\n")`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `/a.txt == b"a1\n"` (C2's content — the uncommitted edit is discarded).
- **And** `/new.txt` absent (`ERR_NOT_FOUND`) — an untracked-at-target file is removed by hard reset.
- **And** the path set is exactly `{"/a.txt","/b.txt"}`.
- **And** tip == `<sha-of-C2>`.
- **Priority:** P0.

---

#### E2E-NEW-666 — Reset: orphaned commits survive as objects**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-255.
- **Preconditions:** E2E-NEW-661 executed (hard reset from C4 to C2).
- **Then** `entry.db.object_exists("<sha-of-C3>") == true` and `object_exists("<sha-of-C4>") == true` (`db.rs:131`).
- **And** `git.show {commit_sha:"<sha-of-C4>"}` succeeds, `commit.message == "C4 add d"`.
- **And** neither sha appears in `git.log` of any ref returned by `entry.db.list_refs()`.
- **And** `git.checkout_file {commit_sha:"<sha-of-C4>", path:"/d.txt"}` succeeds and writes `b"d1\n"` back into the volume — an orphan is still readable (`git.rs:224-257`).
- **Priority:** P0.

---

#### E2E-NEW-667 — Reset: target_ref forms**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE plus a branch `old` pointing at `<sha-of-C2>` and a tag `v1` pointing at `<sha-of-C2>` (`entry.db.set_ref`, `db.rs:235`).
- **When** in three separate runs from the same start: `target_ref:"old"`, `target_ref:"refs/heads/old"`, `target_ref:"v1"`, each with `mode:"hard"`; plus `target_ref:"<sha-of-C2>[..8]"` (abbreviated).
- **Then** each yields `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` and volume path set `{"/a.txt","/b.txt"}`.
- **And** `target_ref:"HEAD"` resolves to the current branch tip and is a no-op (E2E-NEW-665 semantics).
- **Priority:** P1.

---

#### E2E-NEW-668 — Reset side-effect: audit and quota for hard**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE at C4; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}` (writes `/a.txt` 3 bytes, `/b.txt` 3 bytes; deletes `/d.txt`).
- **Then** `audit()[n0..]` contains an entry with `op == "git.reset"` exactly, `detail` containing `"hard"` and `<sha-of-C2>`.
- **And** `bytes_written() - q0 == 6` (only bytes actually written; deletions are not charged, consistent with `charge_write` taking `num_bytes`, `safety.rs:131`). If the implementation skips rewriting a file whose bytes are already correct, the assertion becomes `== 3` — the spec fixes the rule as **write every file of the target tree**, so `6` is the asserted value.
- **And** a soft reset in the same session appends an audit entry with `op == "git.reset"` and `detail` containing `"soft"`, and adds `0` to `bytes_written`.
- **Priority:** P1.

---

#### E2E-NEW-671 — Reset failure: `mixed` is rejected**

**Category:** Failure. **Scenario:** SC-920. **Requirements:** FR-NEW-252.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"mixed"}`.
- **Then** `ERR_INVALID_ARGUMENT` whose message contains `"mixed"`, `"soft"` and `"hard"`. Baseline: `git.reset: mode must be 'soft' or 'hard', got 'mixed'` (same shape as `parse_on_conflict`, `git.rs:1481`), plus the reason: there is no index.
- **And** tip == `<sha-of-C4>`; volume unchanged; `bytes_written` unchanged.
- **And** the frozen `inputSchema` for `git.reset` lists `mode` with exactly the two allowed values in its description (checked against `tool-contract-golden.json`).
- **Priority:** P0.

---

#### E2E-NEW-672 — Reset failure: case-sensitive mode**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-252.
- **When** `mode:"HARD"`, then `mode:"Soft"`, then `mode:""`, then `mode:"hard "` (trailing space).
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"soft"` and `"hard"`.
- **And** tip unchanged in all four cases.
- **Priority:** P1.

---

#### E2E-NEW-673 — Reset failure: hard blocked by quota leaves everything in place**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-251, FR-NEW-185.
- **Preconditions:** FX-LINE at C4; env built with a quota that leaves 1 remaining byte; volume snapshot; `tip = <sha-of-C4>`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of {N} bytes exceeded` (`safety.rs:135-137`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C4>` — the pointer did **not** move (quota is checked before the ref update; the operation is all-or-nothing).
- **And** every volume path and its bytes equal the snapshot, including `/d.txt == b"d1\n"`.
- **And** `bytes_written()` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-862 — SideEffect — the refused hard reset leaves the dirty volume alone**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-253, FR-NEW-251.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; `fs.write_text {path:"/a.txt", content:"a1\na3\nLOCAL\n"}` makes the volume dirty; capture `before_audit`, `before_bytes`.
- **When** `git.reset {mount_id:"proj1", target_ref:"nosuchref", mode:"hard"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"nosuchref"`.
- **And** `/a.txt` still reads exactly `b"a1\na3\nLOCAL\n"` — a hard reset discards uncommitted work, so an unknown target must not begin discarding before resolving.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `audit(OWNER, MOUNT)` equals `before_audit` and `bytes_written` equals `before_bytes`.
- **Verification:** error assertion; byte compare; ref read; audit/quota equality.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-865 — EdgeCase — a hard reset to the current tip still discards uncommitted changes**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-254, FR-NEW-251.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; `fs.write_text {path:"/a.txt", content:"CLOBBERED\n"}` and `fs.delete {path:"/d.txt"}` make the volume dirty in two ways.
- **When** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the call succeeds, `old_sha == new_sha == "<sha-of-C4>"` and `files_changed > 0` (the ref is a no-op, the volume is not).
- **And** `/a.txt` reads exactly `b"a1\na3\n"` and `/d.txt` reads exactly `b"d1\n"` — both restored from `<sha-of-C4>`'s tree.
- **And** `git.status` reports `dirty == false` with an empty `changes` array.
- **And** the same sequence with `mode:"soft"` leaves the dirt in place (`/a.txt` == `b"CLOBBERED\n"`), contrasting the two modes at the same no-op ref.
- **Verification:** response fields; two byte compares; `git.status`; second-run compare.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-894 — SideEffect — `branch_reset` audits both shas so the move is recoverable**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-111, FR-NEW-255.
Existing angles: 430 and 434 are both Happy paths. This one is the audit/recovery record.
- **Preconditions:** SEED-B; `release/1.0` = `<sha-C3>`, not checked out.
- **When** `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C1>","force":true}`.
- **Then** the response is exactly `{"branch":"release/1.0","old_sha":"<sha-C3>","new_sha":"<sha-C1>","files_changed": 0}`.
- **And** exactly one new audit entry has `op == "git.branch_reset"` and a `detail` containing both `"<sha-C3>"` and `"<sha-C1>"`.
- **And** `db.object_exists("<sha-C3>")` is `true`.
- **And** replaying the recovery from the audit line alone works: `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C3>","force":true}` restores the ref to `<sha-C3>`.
- **Verification:** exact response; audit entry `op` and `detail`; `object_exists`; recovery call plus ref read.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-899 — DataIntegrity — a stash applies after its base commit became unreachable**

**Category:** DataIntegrity. **Scenario:** SC-930. **Requirements:** FR-NEW-129, FR-NEW-255.
Existing angles: 464 (branch deleted), 465 (applied on a different branch). This one removes the *commit* from every ref's reachability, not just the branch name.
- **Preconditions:** SEED-B; `git.branch_switch {"name":"release/1.0"}`; write `/docs/rel.md` = `"rel-wip\n"`; `git.stash_save {"message":"wip rel"}` -> `<stash-1>` with `base_sha == "<sha-C3>"`.
- **When** `git.branch_switch {"name":"main"}`, `git.branch_delete {"name":"release/1.0","force":true}` (so `<sha-C3>` is reachable from no ref), then `git.stash_apply {"stash_id":"<stash-1>"}` on `main`.
- **Then** the apply returns `status == "applied"` (or the conflict response if the bytes clash; with `/docs/rel.md` absent on `main` it applies cleanly).
- **And** `/docs/rel.md` reads exactly `b"rel-wip\n"`.
- **And** `db.object_exists("<sha-C3>")` is still `true` — the stash's `base_sha` object survived the branch delete, which is precisely why the diff could be computed.
- **And** `git.stash_list` still lists `<stash-1>` with `base_sha == "<sha-C3>"`.
- **Verification:** apply response; byte-exact read; `object_exists`; stash list.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-916 — EdgeCase — aborting after one clean step removes the replayed commit from the branch but keeps its object**

**Category:** EdgeCase. **Scenario:** SC-917. **Requirements:** FR-NEW-223, FR-NEW-255.
Existing angles: 619 (abort after an immediate conflict), 621 (abort mid-multi-pause). This one pins what happens to the *already replayed* commits' objects.
- **Preconditions:** **FX-TAIL** (as E2E-NEW-853): `feature` = C1 + T1 (clean) + T2 (conflicts). Rebase started, T1 replayed as `<sha-T1'>`, paused at entry 2.
- **Given** `<sha-T1'>` is read from `git.log` of the replay tip while paused, and `db.object_exists("<sha-T1'>")` is `true`.
- **When** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** `db.get_ref("refs/heads/feature").target == "<sha-of-T2>"` (the exact pre-rebase tip).
- **And** `git.log {ref_name:"feature"}` contains neither `<sha-T1'>` nor any commit whose parent is `<sha-of-C2>`.
- **And** `/a.txt` reads exactly `b"a1\nFEAT\n"` and `/t1.txt` reads exactly `b"t1\n"` — the pre-rebase volume bytes, restored exactly.
- **And** `db.object_exists("<sha-T1'>")` is still `true`: abort unreaches, it does not prune (consistent with FR-NEW-255 for reset).
- **And** no `git_operations` row remains.
- **Verification:** ref read; log sha scan; two byte compares; `object_exists`; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-921 — DataIntegrity — a mistaken hard reset is fully recovered from the reported `old_sha`**

**Category:** DataIntegrity. **Scenario:** SC-921. **Requirements:** FR-NEW-255, FR-NEW-111.
Existing angles: 609 and 666 both assert the orphaned objects still exist. This one completes the loop the requirement exists for: actual recovery.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; capture the byte content of `/a.txt`, `/b.txt`, `/d.txt`.
- **When** `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}` (three commits discarded), then `git.branch_reset {name:"main", target_commit:<old_sha from that response>, force:true}`, then `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the first response's `old_sha == "<sha-of-C4>"` and after the reset `/b.txt` and `/d.txt` are absent.
- **And** after the recovery, `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `/a.txt`, `/b.txt` and `/d.txt` are byte-identical to the captured pre-reset content.
- **And** `git.log {ref_name:"main"}` holds exactly the original 4 commits with their original shas.
- **Verification:** response field; two volume states; ref read; three byte compares; log sha list.
- **Cleanup:** fixture drop. **Priority:** P0.


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
