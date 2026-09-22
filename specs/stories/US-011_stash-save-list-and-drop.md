# US-011: Stash save, list and drop

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 11
> Depends On: US-009
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Give the caller somewhere to put uncommitted work. Stash entries are commits under `refs/stash/*`, so this needs no new table. This story adds save, list and drop; it is what makes the dirty-volume refusals elsewhere actionable.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/db.rs
```

### Existing Patterns
Stash entries are refs under `refs/stash/*` in the existing `git_refs` table; the namespace is unused today (zero occurrences under `crates/`). Git objects live in the blob store under `git:{sha}` and are exported to the bare repo before libgit2 reads, imported after it writes: `crates/mcp-fs/src/git/odb.rs:3-28`.
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-913:** Stash entries are stored as refs under `refs/stash/*` in the existing `git_refs` table rather than in the new operation table or a table of their own. **Rationale:** it is how real git models a stash, it needs no schema change, and the namespace is verified unused. **Implemented by:** FR-NEW-120, FR-NEW-123. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/git/db.rs:60-68` (`git_refs`), and `refs/stash` has zero occurrences under `crates/`.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

8 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-120 [EARS-E]: Save a stash
> WHEN `git.stash_save` is called THE mcp-fs server SHALL capture the volume's current state as a commit object stored under `refs/stash/{stash_id}`, and SHALL then rewrite the volume to match HEAD's tree.

- **Inputs:** `mount_id`, `message` (string, optional).
- **Outputs:** exactly `{stash_id, sha, message, base_sha, branch, created_at, files_stashed}`. `branch` is the branch checked out at save time; `created_at` is epoch milliseconds.
- **Business Rules:** `stash_id` is server-generated, opaque and stable. The stash commit records `base_sha`, the HEAD it was taken against. `refs/stash` is an unused namespace today (verified: zero occurrences under `crates/`), and stash refs are stored in the existing `git_refs` table, so no new table is required for stash.
- **Priority:** Must-have

### FR-NEW-121 [EARS-O]: Refuse to stash a clean volume
> IF the volume holds no changes relative to HEAD THEN THE mcp-fs server SHALL reject `git.stash_save` with `ERR_INVALID_ARGUMENT` stating there is nothing to stash, and SHALL NOT create a ref.

- **Priority:** Must-have

### FR-NEW-122 [EARS-E]: List stashes
> WHEN `git.stash_list` is called THE mcp-fs server SHALL return every stash entry for the volume as an object with exactly the keys `stash_id`, `message`, `base_sha`, `branch` and `created_at`, most recent first.

- **Business Rules:** The pool is per volume, not per branch. An empty pool returns an empty list, not an error.
- **Priority:** Must-have

### FR-NEW-123 [EARS-UB]: Stash refs are not branches
> The mcp-fs server SHALL NOT list `refs/stash/*` entries in `git.branches`, and SHALL NOT allow them to be checked out by `git.branch_switch`.

- **Priority:** Must-have

### FR-NEW-127 [EARS-E]: Drop a stash
> WHEN `git.stash_drop` is called with `stash_id` THE mcp-fs server SHALL delete that entry without touching the volume.

- **Priority:** Must-have

### FR-NEW-128 [EARS-O]: Unknown stash id
> IF `stash_id` matches no entry THEN THE mcp-fs server SHALL reject `git.stash_apply`, `git.stash_pop` and `git.stash_drop` with `ERR_NOT_FOUND`.

- **Priority:** Must-have

### FR-NEW-130 [EARS-U]: Bound the stash pool
> The mcp-fs server SHALL reject `git.stash_save` with `ERR_INVALID_ARGUMENT` once the volume holds `git.max_stash_entries` entries, naming `git.stash_drop` as the remedy.

- **Inputs:** config key `git.max_stash_entries`, default `100`.
- **Priority:** Should-have

### FR-NEW-131 [EARS-U]: Stash entry fields are fixed
> The mcp-fs server SHALL return, for each `git.stash_list` entry, exactly the keys `stash_id`, `message`, `base_sha`, `branch` and `created_at`, where `branch` is the branch checked out at save time and `created_at` is epoch milliseconds, ordered newest first.

- **Business Rules:** `base_sha` is mandatory because FR-NEW-129 makes it the only anchor once the branch a stash was taken from is deleted. Dropping it would make a stash unapplicable after branch deletion.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

20 tests: Concurrency 1, DataIntegrity 1, EdgeCase 5, Failure 6, Happy 3, SideEffect 4.
Scenarios covered: SC-902.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-446 — Happy — `stash_save` creates an entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: SEED-A, then `env.write("/src/lib.rs","fn a() { 1 }\n")` and `env.write("/notes.txt","draft\n")` (one modification, one addition, both uncommitted).
- **When** `git.stash_save {"mount_id":"gitproj","message":"wip: login"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","message":"wip: login","branch":"main","files_stashed":2}` where `<sha-S1>` matches `^[0-9a-f]{40}$`.
- **And** `git.stash_list` returns one entry whose `"stash_id"` equals the response's `"stash_id"`.
Priority: P0.

---

#### E2E-NEW-447 — SideEffect — `stash_save` reverts the volume to HEAD**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: E2E-NEW-446 run to success.
- **Then** `env.read("/src/lib.rs") == "fn a() {}\n"` (reverted to the `<sha-C2>` content).
- **And** `client.exists("/notes.txt")` is `false` (the untracked-in-HEAD addition is removed).
- **And** `env.read("/README.md") == "alpha\n"`.
- **And** a follow-up `git.stash_save {"message":"second"}` errors with `ERR_INVALID_ARGUMENT` "nothing to stash", proving the volume is now provably clean against HEAD (the same `require_clean_volume` comparison, `crates/mcp-fs/src/tools/git.rs:1991-2009`).
Priority: P0.

---

#### E2E-NEW-448 — SideEffect — the stash lives in `git_refs` under `refs/stash/`**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-123., FR-NEW-131
Preconditions: E2E-NEW-446 run to success, `<sha-S1>` captured.
- **Then** `db.get_ref(&format!("refs/stash/{sha_s1}"))` returns `Some(GitRefRow { target: "<sha-S1>", symbolic: false })` (direct `git_refs` row check, `crates/mcp-fs/src/git/db.rs:214`).
- **And** `db.list_refs()` contains exactly one name starting with `"refs/stash/"`.
- **And** `db.get_object("<sha-S1>")` returns `Some(row)` with `row.kind == "commit"` (the snapshot is a real object, not a dangling ref).
Priority: P0.

---

#### E2E-NEW-449 — Failure — `stash_save` on a clean volume**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-121.
Preconditions: SEED-A, volume clean (no writes after `git.commit "c2"`).
- **When** `git.stash_save {"message":"nothing here"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("nothing to stash")` and `message.contains("no uncommitted changes")`.
- **And** `db.list_refs()` contains no `refs/stash/` name.
- **And** the error is explicitly distinguishable from the dirty-volume refusal: `assert!(!message.contains("uncommitted changes: commit or discard"))`.
Priority: P0.

---

#### E2E-NEW-450 — Failure — `stash_save` in a repo with no commits**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-121.
Preconditions: `git.init` only, then `env.write("/scratch.txt","s\n")`.
- **When** `git.stash_save {"message":"early"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("HEAD")` and `message.contains("no commits")`.
- **And** `env.read("/scratch.txt") == "s\n"` (the caller's file is NOT reverted away by a failed save).
- **And** `db.list_refs()` contains no `refs/stash/` name.
Priority: P1.

---

#### E2E-NEW-451 — SideEffect — `stash_save` charges quota and audits**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: SEED-A, `env.write("/src/lib.rs","fn a() { 1 }\n")` (13 bytes), `before = safety.bytes_written(OWNER, MOUNT)`.
- **When** `git.stash_save {"message":"wip"}`.
- **Then** `safety.bytes_written(OWNER, MOUNT) - before == 10` (the revert rewrites `/src/lib.rs` back to `"fn a() {}\n"`, 10 bytes; only written blobs are charged).
- **And** `safety.audit(OWNER, MOUNT)` ends with exactly one entry where `op == "git.stash_save"`, `path == "/"`, `detail.contains("wip")` and `detail.contains("files_stashed 1")`.
Priority: P1.

---

#### E2E-NEW-452 — EdgeCase — unicode and very long stash messages**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-122.
Preconditions: SEED-A, `env.write("/notes.txt","x\n")`. `let msg = format!("réunion 日本 🚀 {}", "m".repeat(980));` (asserted `msg.chars().count() == 1000`).
- **When** `git.stash_save {"message": msg}`.
- **Then** the response `"message"` equals `msg` byte-for-byte.
- **And** `git.stash_list`'s single entry has `"message"` equal to `msg` byte-for-byte (no truncation, no lossy conversion).
- **And** a `git.stash_save` with `message` omitted entirely (a second stash after another write) yields `"message"` equal to the default `"WIP on main"`.
Priority: P2.

---

#### E2E-NEW-453 — Happy — `stash_list` is newest first with the full field shape**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-122., FR-NEW-131
Preconditions: SEED-A; `env.write("/a.txt","1\n")`; `stash_save {"message":"first"}` -> `<sha-S1>`; `env.write("/b.txt","2\n")`; `stash_save {"message":"second"}` -> `<sha-S2>`.
- **When** `git.stash_list {"mount_id":"gitproj"}`.
- **Then** `entries.len() == 2`, `entries[0]["stash_id"] == "<sha-S2>"`, `entries[0]["message"] == "second"`, `entries[1]["stash_id"] == "<sha-S1>"`, `entries[1]["message"] == "first"`.
- **And** every entry has the keys exactly `["stash_id","message","base_sha","branch","created_at"]` with `base_sha` equal to the HEAD sha at save time and `"branch" == "main"` and `created_at` an integer `> 0`.
Priority: P0.

---

#### E2E-NEW-454 — EdgeCase — `stash_list` on an empty pool**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-122., FR-NEW-131
Preconditions: SEED-A, no stash ever saved.
- **When** `git.stash_list {"mount_id":"gitproj"}`.
- **Then** the response is exactly `{"mount_id":"gitproj","stashes":[]}` and the call is `Ok`, not `ERR_NOT_FOUND`.
Priority: P0.

---

#### E2E-NEW-460 — Failure — `stash_drop` with an unknown id**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-446 state.
- **When** `git.stash_drop {"stash_id":"not-a-sha"}` and again with `"ffffffffffffffffffffffffffffffffffffffff"`.
- **Then** the first errors `code == ERR_INVALID_ARGUMENT` with `message.contains("not-a-sha")` and `message.contains("40")` (malformed id); the second errors `code == ERR_NOT_FOUND`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some`.
Priority: P1.

---

#### E2E-NEW-461 — Happy — `stash_drop` removes the entry without touching the volume**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-127.
Preconditions: E2E-NEW-446 state (volume clean at `<sha-C2>`).
- **When** `git.stash_drop {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","dropped":true}`.
- **And** `git.stash_list` returns `"stashes": []` and `db.get_ref("refs/stash/<sha-S1>")` is `None`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` and `client.exists("/notes.txt")` is `false` (drop never restores anything).
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before the drop.
Priority: P0.

---

#### E2E-NEW-467 — EdgeCase — the stash pool cap boundary**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-130.
Preconditions: SEED-A. Loop `i in 0..100`: `env.write(&format!("/s{i}.txt"), &format!("{i}\n"))` then `git.stash_save {"message": format!("s{i}")}`, each asserted `Ok`.
- **Given** `git.stash_list` returns exactly 100 entries.
- **When** `env.write("/s100.txt","100\n")` then `git.stash_save {"message":"s100"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("100")` and `message.contains("stash")` and `message.contains("drop")`.
- **And** `git.stash_list` still returns exactly 100 entries (the oldest was NOT silently evicted, unlike the audit ring at `crates/mcp-fs/src/safety.rs:145-152`).
- **And** `env.read("/s100.txt") == "100\n"` (the refused save did not revert the caller's work).
Priority: P2.

---

#### E2E-NEW-469 — Concurrency — two `stash_save` calls race**

**Category:** Concurrency. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-115.
Preconditions: SEED-A; `env.write("/p.txt","p\n")` and `env.write("/q.txt","q\n")` (both uncommitted).
- **When** `tokio::join!` of `git.stash_save {"message":"A"}` and `git.stash_save {"message":"B"}`.
- **Then** exactly one of two accepted shapes holds, asserted explicitly: either both succeed with two distinct `stash_id` values, or one succeeds and the other errors `ERR_INVALID_ARGUMENT` "nothing to stash" (the loser observed the already-reverted volume).
- **And** `git.stash_list`'s entry count equals the number of successful calls.
- **And** every listed `stash_id` has a matching `refs/stash/{id}` row, and the ids are pairwise distinct.
- **And** the final volume matches `<sha-C2>` exactly: `client.exists("/p.txt") == false` and `client.exists("/q.txt") == false`.
Priority: P1.

---

#### E2E-NEW-470 — DataIntegrity — stash refs never leak into branch or tag listings**

**Category:** DataIntegrity. **Scenario:** SC-902. **Requirements:** FR-NEW-123.
Preconditions: E2E-NEW-453 state (two stashes).
- **When** `git.branches`, `git.tags`, and `git.status` are called.
- **Then** `git.branches`'s `"branches"` contains no entry whose `"full_ref"` starts with `"refs/stash/"` (it has exactly one entry, `main`).
- **And** `git.tags`'s `"tags"` is `[]`.
- **And** `git.status`'s `"refs"` DOES contain both `refs/stash/<sha-S1>` and `refs/stash/<sha-S2>` (status lists every non-symbolic ref, `crates/mcp-fs/src/tools/git.rs:530-534`; this test pins that the stash is visible there and nowhere else).
Priority: P1.

---

##### C) Remotes

---

#### E2E-NEW-822 — SideEffect — drop touches neither the volume nor the other entries**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-127, FR-NEW-122.
- **Preconditions:** SEED-A; write `/README.md` = `"alpha-1\n"`, `git.stash_save {message:"wip one"}` -> `<stash-1>`; write `/README.md` = `"alpha-2\n"`, `git.stash_save {message:"wip two"}` -> `<stash-2>`. Volume back at `<sha-C2>`'s tree.
- **When** `git.stash_drop {"mount_id":"gitproj","stash_id":"<stash-1>"}`.
- **Then** the response is exactly `{"stash_id":"<stash-1>","dropped":true}`.
- **And** `git.stash_list` returns exactly one entry, `stash_id == "<stash-2>"`, `message == "wip two"`.
- **And** `db.get_ref("refs/stash/<stash-1>")` is `None` and `db.get_ref("refs/stash/<stash-2>")` is unchanged.
- **And** `/README.md` still reads exactly `b"alpha\n"` and `bytes_written` is unchanged from before the drop.
- **And** `db.object_exists("<stash-1>")` is still `true` — the ref is removed, the commit object is not pruned by the drop.
- **Verification:** exact response; stash list; two ref reads; byte compare; quota; `object_exists`.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-823 — Failure — dropping the same id twice**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-127, FR-NEW-128.
- **Preconditions:** SEED-A plus one stash `<stash-1>` as above.
- **When** `git.stash_drop {"stash_id":"<stash-1>"}` succeeds, then the identical call is issued a second time.
- **Then** the second call asserts error `ERR_NOT_FOUND` containing `"<stash-1>"` — drop is not idempotent-silent, an already-gone id is reported.
- **And** `git.stash_list` returns `[]` (an empty array, not an error).
- **And** `state.safety.audit(OWNER, MOUNT)` holds exactly **one** entry whose `op == "git.stash_drop"`, from the successful call only.
- **Verification:** error assertion; empty-list equality; audit count by op.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-824 — Failure — the refusal names the limit and the remedy**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-130.
- **Preconditions:** `Env::build` with `c.git.max_stash_entries = 3`; SEED-A; three stashes saved (`"wip 1"`, `"wip 2"`, `"wip 3"`), volume made dirty again with `/README.md` = `"alpha-4\n"`.
- **When** `git.stash_save {"mount_id":"gitproj","message":"wip 4"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"3"` and `"git.stash_drop"`.
- **And** `git.stash_list` still returns exactly 3 entries with messages `["wip 3","wip 2","wip 1"]` (newest first).
- **And** `/README.md` still reads exactly `b"alpha-4\n"` — the refused save did not rewrite the volume to HEAD.
- **And** `db.list_refs()` holds exactly 3 refs under `refs/stash/`.
- **Verification:** error code + two substrings; stash list; byte compare; ref list filter.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-825 — EdgeCase — dropping one entry at the cap re-opens a slot**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-130, FR-NEW-127.
- **Preconditions:** as E2E-NEW-824, at the cap of 3, volume dirty with `/README.md` = `"alpha-4\n"`, save already refused once.
- **When** `git.stash_drop {"stash_id":"<stash-1>"}` then `git.stash_save {"message":"wip 4"}`.
- **Then** the save succeeds and its response `message == "wip 4"`.
- **And** `git.stash_list` returns exactly 3 entries, messages `["wip 4","wip 3","wip 2"]` — the count is a live check, not a monotonic counter.
- **And** `/README.md` reads exactly `b"alpha\n"` (the successful save rewrote the volume to HEAD's tree).
- **Verification:** success response; stash list ordering; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

---

#### E2E-NEW-895 — EdgeCase — dirt that is only an added file is still stashable**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-121, FR-NEW-120.
Existing angles: 449 and 450 are both refusals. This one is the boundary on the other side.
- **Preconditions:** SEED-A, clean volume; `fs.write_text {path:"/scratch/new.txt", content:"n\n"}` (a file in neither the HEAD tree nor any commit).
- **When** `git.stash_save {"message":"wip added only"}`.
- **Then** the call succeeds with `files_stashed == 1` and `base_sha == "<sha-C2>"`.
- **And** `VolumeClient::read_bytes("/scratch/new.txt")` errors `ERR_NOT_FOUND` — the volume was rewritten to HEAD's tree, which does not contain it.
- **And** `git.stash_apply {"stash_id":"<stash-1>"}` restores it to exactly `b"n\n"`.
- **And** a second `git.stash_save` immediately after the first (volume now clean) errors `ERR_INVALID_ARGUMENT` containing `"nothing to stash"`, contrasting the two states.
- **Verification:** success response; volume read error; byte-exact restore; contrasting error.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-896 — Failure — a stash ref cannot be checked out**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-123.
Existing angles: 448 (row exists), 470 (absent from `git.branches`). This one is the `branch_switch` half of the requirement.
- **Preconditions:** SEED-A plus one stash `<stash-1>` (`refs/stash/<stash-1>` exists).
- **When** `git.branch_switch {"mount_id":"gitproj","name":"<stash-1>"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"<stash-1>"`.
- **When** `git.branch_switch {"name":"refs/stash/<stash-1>"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` or `ERR_NOT_FOUND` naming the value; in either case `db.get_ref("HEAD")` is still `{target:"refs/heads/main", symbolic:true}`.
- **And** `git.branch_delete {"name":"<stash-1>"}` also errors, and `db.get_ref("refs/stash/<stash-1>")` still exists afterwards — the stash namespace is not reachable through branch tools at all.
- **Verification:** three error assertions; HEAD read; stash ref read.
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
