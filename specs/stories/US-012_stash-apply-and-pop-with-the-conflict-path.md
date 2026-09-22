# US-012: Stash apply and pop, with the conflict path

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 12
> Depends On: US-003, US-011
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Let a caller get stashed work back, including when it no longer applies cleanly. A conflicting apply or pop goes through the shared conflict model and, critically, never drops the entry until resolution has fully succeeded.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
The existing three-way merge is `crates/mcp-fs/src/tools/git.rs:1835-1839` (libgit2 `merge_commits` + `MergeOptions::file_favor`); its refusal path is `crates/mcp-fs/src/tools/git.rs:1844-1849`.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-913:** Stash entries are stored as refs under `refs/stash/*` in the existing `git_refs` table rather than in the new operation table or a table of their own. **Rationale:** it is how real git models a stash, it needs no schema change, and the namespace is verified unused. **Implemented by:** FR-NEW-120, FR-NEW-123. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/git/db.rs:60-68` (`git_refs`), and `refs/stash` has zero occurrences under `crates/`.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

5 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-124 [EARS-E]: Apply a stash
> WHEN `git.stash_apply` is called with `stash_id` THE mcp-fs server SHALL apply that entry's changes onto the current volume state and SHALL retain the entry.

- **Outputs:** exactly `{stash_id, status, files_changed, dropped}` per FR-NEW-132.
- **Priority:** Must-have

### FR-NEW-125 [EARS-E]: Pop a stash
> WHEN `git.stash_pop` is called with `stash_id` THE mcp-fs server SHALL apply that entry's changes onto the current volume state and, only once the application has fully succeeded, SHALL delete the entry.

- **Business Rules:** The order matters: an application that ends in conflict leaves the entry intact so no work is lost.
- **Priority:** Must-have

### FR-NEW-126 [EARS-O]: A conflicting stash application enters the shared conflict model
> IF applying a stash produces a genuine content conflict THEN THE mcp-fs server SHALL return the conflict response defined in FR-NEW-170, SHALL apply nothing to the volume, SHALL record an in-progress operation of type `stash_apply`, and SHALL retain the stash entry regardless of whether `git.stash_apply` or `git.stash_pop` was called.

- **Priority:** Must-have

### FR-NEW-129 [EARS-U]: Stash entries outlive their source branch
> The mcp-fs server SHALL keep a stash entry listable and applicable after the branch it was created on is deleted, and SHALL apply it against whichever branch is checked out at apply time.

- **Business Rules:** A stash references `base_sha` and a diff, never a branch name.
- **Priority:** Should-have

### FR-NEW-132 [EARS-U]: Stash apply and pop return one key set
> The mcp-fs server SHALL return from `git.stash_apply` and `git.stash_pop` an object with exactly the keys `stash_id`, `status`, `files_changed` and `dropped`, where `status` is `applied` or `conflict`, and `dropped` is `false` for `git.stash_apply` and `true` for a `git.stash_pop` that fully succeeded.

- **Business Rules:** The key `applied` is emitted by no tool; the boolean lives in `dropped` and the outcome in `status`. A `git.stash_pop` that ends in conflict reports `status: "conflict"` and `dropped: false`, because FR-NEW-126 retains the entry until resolution succeeds. `git.stash_drop` returns exactly `{stash_id, dropped}` with `dropped` always `true`.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

13 tests: DataIntegrity 1, EdgeCase 2, Failure 6, Happy 3, SideEffect 1.
Scenarios covered: SC-902, SC-930.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-455 — Happy — `stash_apply` restores and keeps the entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-124., FR-NEW-131
Preconditions: E2E-NEW-446 state (stash `<sha-S1>` holds the `/src/lib.rs` modification and `/notes.txt` addition; volume clean at `<sha-C2>`).
- **When** `git.stash_apply {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","status":"applied","files_changed":2,"dropped":false}`.
- **And** `env.read("/src/lib.rs") == "fn a() { 1 }\n"` and `env.read("/notes.txt") == "draft\n"`.
- **And** `git.stash_list` still returns one entry with `"stash_id":"<sha-S1>"`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some`.
Priority: P0.

---

#### E2E-NEW-456 — Happy — `stash_pop` restores and removes the entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-125.
Preconditions: E2E-NEW-446 state.
- **When** `git.stash_pop {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","status":"applied","files_changed":2,"dropped":true}`.
- **And** `env.read("/src/lib.rs") == "fn a() { 1 }\n"` and `env.read("/notes.txt") == "draft\n"`.
- **And** `git.stash_list` returns `"stashes": []`.
Priority: P0.

---

#### E2E-NEW-457 — SideEffect — pop deletes the `refs/stash/` row**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-125.
Preconditions: E2E-NEW-456 run to success.
- **Then** `db.get_ref("refs/stash/<sha-S1>")` is `None` (direct `git_refs` check).
- **And** `db.list_refs()` contains no name starting with `"refs/stash/"`.
- **And** `db.get_object("<sha-S1>")` is still `Some` (the ref is dropped, the object is not deleted; delete is a GC concern, not this tool's).
- **And** `safety.audit(OWNER, MOUNT)` ends with one entry `op == "git.stash_pop"`, `detail.contains("<sha-S1>")` and `detail.contains("dropped true")`.
Priority: P0.

---

#### E2E-NEW-458 — Failure — `stash_apply` with an unknown id**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-446 state (one real stash present).
- **When** `git.stash_apply {"stash_id":"0123456789abcdef0123456789abcdef01234567"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("0123456789abcdef0123456789abcdef01234567")` and `message.contains("stash")`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (the volume is untouched).
- **And** `git.stash_list` still returns exactly the one real entry.
Priority: P0.

---

#### E2E-NEW-459 — Failure — `stash_pop` with an unknown id leaves the pool intact**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-453 state (two stashes `<sha-S1>`, `<sha-S2>`).
- **When** `git.stash_pop {"stash_id":"ffffffffffffffffffffffffffffffffffffffff"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("ffffffff")`.
- **And** `git.stash_list` still returns 2 entries with ids `<sha-S2>`, `<sha-S1>` in that order.
- **And** `db.list_refs()` still contains both `refs/stash/<sha-S1>` and `refs/stash/<sha-S2>`.
Priority: P0.

---

#### E2E-NEW-462 — Failure — a conflicting `stash_apply` returns `status:"conflict"` and writes nothing**

**Category:** Failure. **Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-170.
Preconditions: SEED-A; `env.write("/src/lib.rs","fn a() { STASHED }\n")`; `stash_save {"message":"conflicting"}` -> `<sha-S1>` (volume reverted to `"fn a() {}\n"`); then `env.write("/src/lib.rs","fn a() { LOCAL }\n")` and `git.commit "c3"` -> `<sha-C3>` so the same path diverged on both sides.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}`.
- **Then** the call returns `Ok` (NOT an `ERR_*`) with `"status" == "conflict"` and `"conflicts"` holding exactly one element whose `path` is `/src/lib.rs` and `"files_changed" == 0`.
- **And** `env.read("/src/lib.rs") == "fn a() { LOCAL }\n"` (no conflict marker, no partial write; the same refusal discipline as `crates/mcp-fs/src/tools/git.rs:1844-1851`).
- **And** `assert!(!env.read("/src/lib.rs").contains("<<<<<<<"))`.
- **And** `git.stash_list` still contains `<sha-S1>`.
Priority: P0.

---

#### E2E-NEW-463 — DataIntegrity — a conflicting `stash_pop` does not drop the entry**

**Category:** DataIntegrity. **Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-171.
Preconditions: identical to E2E-NEW-462.
- **When** `git.stash_pop {"stash_id":"<sha-S1>"}`.
- **Then** the response has `"status" == "conflict"` and `"dropped" == false`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some(GitRefRow { target: "<sha-S1>", .. })` (the stashed work is not lost).
- **And** `git.stash_list` returns exactly one entry, id `<sha-S1>`.
- **And** `env.read("/src/lib.rs") == "fn a() { LOCAL }\n"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before the pop.
Priority: P0.

---

#### E2E-NEW-464 — EdgeCase — the stash pool survives deletion of its origin branch**

**Category:** EdgeCase. **Scenario:** SC-930. **Requirements:** FR-NEW-129.
Preconditions: SEED-A; `git.branch_create {"name":"feature/temp","start_point":"<sha-C2>","checkout":true}`; `env.write("/tmp.txt","t\n")`; `git.stash_save {"message":"on temp"}` -> `<sha-S1>` (entry records `"branch":"feature/temp"`); `git.branch_switch {"name":"main"}`.
- **When** `git.branch_delete {"name":"feature/temp","force":true}`, then `git.stash_list`.
- **Then** the delete succeeds and `db.get_ref("refs/heads/feature/temp")` is `None`.
- **And** `git.stash_list` still returns exactly one entry with `"stash_id":"<sha-S1>"` and `"branch":"feature/temp"` (the recorded origin name survives as data even though the branch is gone).
- **And** `db.get_ref("refs/stash/<sha-S1>")` is `Some`.
- **And** `git.stash_pop {"stash_id":"<sha-S1>"}` then succeeds with `"status":"applied"` and `env.read("/tmp.txt") == "t\n"`.
Priority: P0.

---

#### E2E-NEW-466 — Failure — `stash_apply` under an exhausted quota**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-124, FR-NEW-185.
Preconditions: `Env::with_quota(40)`; SEED-A built through `env.write` (`"alpha\n"` 6 + `"fn a() {}\n"` 10 = 16 charged); `env.write("/big.txt", &"x".repeat(20))` (20 more -> 36 charged); `git.stash_save {"message":"big"}` -> `<sha-S1>` (the revert deletes `/big.txt`, charging 0, so 36 stays). Remaining headroom is 4 bytes, asserted in-test via `assert_eq!(safety.bytes_written(OWNER, MOUNT), 36)`.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}` (it must write 20 bytes).
- **Then** `code == ERR_WRITE_QUOTA_EXCEEDED` and `message.contains("session write quota of 40 bytes exceeded")`.
- **And** `client.exists("/big.txt")` is `false` (nothing partially applied).
- **And** `git.stash_list` still contains `<sha-S1>` and `db.get_ref("refs/stash/<sha-S1>")` is `Some`.
- **And** `safety.bytes_written(OWNER, MOUNT) == 36` (a rejected charge consumes nothing).
Priority: P0.

---

#### E2E-NEW-540 — stash_pop conflict, then a bad resolve

**Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-177., FR-NEW-186
**Category:** Failure. **Priority:** P1.
**Preconditions:** `C0` commits `/notes.md` = `todo\n`. Write `/notes.md` = `todo local\n`, `git.stash_save`. Commit `/notes.md` = `todo upstream\n` → `C1`.
**When** `git.stash_pop {mount_id:"proj"}`.
**Then** `status == "conflict"`, `operation == "stash_pop"`, `conflicts[0].path == "/notes.md"`, `continue_with`/`abort_with` includes `"git.merge_resolve"` and `"git.merge_abort"` — the same shared pair.
**When** `git.merge_resolve {resolutions:[{path:"/other.md", strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT` containing `/other.md` and `not in conflict`.
**And** `/notes.md` still reads exactly `todo upstream\n`, contains no `<<<<<<<`, and the stash entry is not dropped (`git.stash_list` length still 1).
**Verification:** JSON fields; error code + substrings; file bytes; stash list length.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-897 — EdgeCase — one stash applied onto two branches in turn**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-124, FR-NEW-129.
Existing angles: 455 (Happy), 466 (quota failure). This one exercises retention across repeated applies.
- **Preconditions:** SEED-B; on `main`, write `/README.md` = `"alpha-wip\n"`, `git.stash_save {"message":"wip"}` -> `<stash-1>`.
- **When** `git.stash_apply {"stash_id":"<stash-1>"}` on `main`, then `git.commit {message:"take wip"}`, then `git.branch_switch {"name":"release/1.0"}`, then `git.stash_apply {"stash_id":"<stash-1>"}` again.
- **Then** both applies return `status == "applied"`.
- **And** after the second apply, `/README.md` reads exactly `b"alpha-wip\n"` on `release/1.0` too.
- **And** `git.stash_list` still returns exactly one entry, `<stash-1>`, after both applies — apply retains, always.
- **And** `db.get_ref("refs/stash/<stash-1>")` is unchanged in target across both applies.
- **Verification:** two response statuses; byte-exact read; stash list length; ref equality.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-898 — Failure — a quota-exhausted `stash_pop` keeps the entry**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-125, FR-NEW-185.
Existing angles: 456 (Happy), 457 (SideEffect, entry gone after a clean pop). This one proves the delete-after-success ordering under a failure the happy path never hits.
- **Preconditions:** `Env::with_quota` sized so that the pop's write exceeds it; SEED-A; one stash `<stash-1>` whose application writes `/README.md` = `"alpha-wip\n"`.
- **When** `git.stash_pop {"stash_id":"<stash-1>"}`.
- **Then** assert error `ERR_WRITE_QUOTA_EXCEEDED`.
- **And** `git.stash_list` still returns exactly one entry, `<stash-1>` with its original `message` and `base_sha` — the entry is deleted only once the application has fully succeeded.
- **And** `/README.md` reads exactly `b"alpha\n"` (nothing partially applied).
- **And** after raising the quota, `git.stash_pop {"stash_id":"<stash-1>"}` succeeds and `git.stash_list` returns `[]`.
- **Verification:** error code; stash list contents; byte compare; recovery run.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-950 — Happy — stash taken on a deleted branch applies cleanly onto another**

**Category:** Happy. **Scenario:** SC-930. **Requirements:** FR-NEW-129, FR-NEW-124, FR-NEW-122.
This is the happy path SC-930 previously lacked; the existing
464 and 465 are EdgeCase assertions on listability and on cross-branch application, and
462/463/540 are failures.
- **Preconditions:** SEED-B (`main` = `<sha-C2>`, `release/1.0` = `<sha-C3>`, HEAD on `main`).
- **Given** `git.branch_create {name:"feature/tmp", start_point:"<sha-C2>", checkout:true}`; `fs.write_text {path:"/src/lib.rs", content:"fn a() {}\nfn b() {}\n"}`; `git.stash_save {message:"wip b"}` -> `<stash-1>` with `base_sha == "<sha-C2>"`, and the volume back at `<sha-C2>`'s tree (`/src/lib.rs` == `"fn a() {}\n"`).
- **When** `git.branch_switch {name:"main"}`, then `git.branch_delete {name:"feature/tmp"}` (merged into nothing new, so no `force` needed since its tip equals `main`'s), then `git.stash_list`, then `git.stash_apply {stash_id:"<stash-1>"}`.
- **Then** `git.stash_list` returns exactly one entry: `{stash_id:"<stash-1>", message:"wip b", base_sha:"<sha-C2>"}` — the deleted branch name appears nowhere in it.
- **And** the apply returns `status == "applied"` with `files_changed == 1`, no `conflicts` key.
- **And** `/src/lib.rs` reads exactly `b"fn a() {}\nfn b() {}\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (an apply does not commit) and `git.status` reports `dirty == true` with `changes == [{"path":"/src/lib.rs","status":"modified"}]`.
- **And** `git.stash_list` still returns the entry afterwards (apply retains), and `db.get_ref("refs/heads/feature/tmp")` is `None`.
- **Verification:** stash list object equality; apply response; byte-exact read; ref reads; `git.status` change list.
- **Cleanup:** fixture drop. **Priority:** P0.

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
