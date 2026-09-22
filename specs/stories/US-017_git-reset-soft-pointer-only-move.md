# US-017: git.reset soft: pointer-only move

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 17
> Depends On: US-005
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective

Move the current branch pointer without touching the volume. This is the half of reset that discards nothing, and it is what makes a mistaken commit recoverable.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Ref updates are dual-written to the relational index and the bare repo; see `git.commit` (`crates/mcp-fs/src/tools/git.rs:702-711`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-905:** `git.reset` offers `soft` and `hard` only; there is no `mixed` mode. **Rationale:** `mixed` differs from `soft` only in what it does to the index, and this server has no index, so the two would be indistinguishable. Offering a third mode that silently behaves like the first would be a trap. **Alternatives considered:** accept `mixed` as an alias of `soft` for familiarity, rejected as misleading. **Implemented by:** FR-NEW-250, FR-NEW-251, FR-NEW-252. **Round:** 1 (P4). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:196-222` (commit takes the whole volume, no index).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

4 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-250 [EARS-E]: Soft reset moves the pointer only
> WHEN `git.reset` is called with `mode` `soft` THE mcp-fs server SHALL move the current branch ref to `target_ref` and SHALL NOT modify any volume file.

- **Inputs:** `mount_id`, `target_ref`, `mode` (`soft` or `hard`, required).
- **Outputs:** exactly `{mode, old_sha, new_sha, files_changed}`.
- **Priority:** Must-have

### FR-NEW-252 [EARS-O]: Mode is required and closed
> IF `mode` is absent, or is any value other than `soft` or `hard` THEN THE mcp-fs server SHALL reject `git.reset` with `ERR_INVALID_ARGUMENT` naming the two accepted values.

- **Business Rules:** There is deliberately no `mixed` mode, because with no index it would be indistinguishable from `soft` (DEC-905).
- **Priority:** Must-have

### FR-NEW-253 [EARS-O]: An unknown reset target changes nothing
> IF `target_ref` resolves to no commit THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` and SHALL leave the branch ref and volume untouched.

- **Priority:** Must-have

### FR-NEW-254 [EARS-O]: Resetting to the current tip is a no-op
> IF `target_ref` resolves to the branch's current tip THEN THE mcp-fs server SHALL return success having changed nothing.

- **Priority:** Should-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

10 tests: Concurrency 1, EdgeCase 3, Failure 3, Happy 1, SideEffect 2.
Scenarios covered: SC-920, SC-921, SC-924, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-440 — Concurrency — reset races a commit on the same branch**

**Category:** Concurrency. **Scenario:** SC-924. **Requirements:** FR-NEW-115.
Preconditions: SEED-A, volume clean; `env.write("/new.txt","n\n")` staged in the volume so the commit has something to record.
- **When** `tokio::join!` of `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}` and `git.commit {"message":"c3"}`.
- **Then** both calls return `Ok` (the reset may also return `ERR_INVALID_ARGUMENT` "uncommitted changes" if it runs first and observes `/new.txt`; the test accepts exactly these two shapes and asserts nothing else).
- **And** `db.get_ref("refs/heads/main").target` is exactly one of `<sha-C1>` or the sha returned by `git.commit`, never an interleaved third value.
- **And** if the commit won the race, `git.log {"ref_name":"main"}`'s first commit sha equals the commit response's `commit_sha`; if the reset won, `git.log`'s first sha is `<sha-C1>`.
Verification: `git.commit` holds `entry.write_lock` for its whole body (`crates/mcp-fs/src/tools/git.rs:659`); `branch_reset` must hold the same lock.
Priority: P1.

---

#### E2E-NEW-534 — git.reset blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1.
**When** `git.reset {mount_id:"proj", ref_name:<C0>}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; `refs/heads/main` still `C2`; volume bytes unchanged.
**Verification:** error code; ref sha; byte compare.

#### E2E-NEW-660 — Reset soft to C2**

**Category:** Happy. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
- **Preconditions:** FX-LINE on `main` (tip `<sha-of-C4>`); volume = C4's tree: `/a.txt == b"a1\na3\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"`.
- **When** `git.reset {mount_id:"proj1", target_ref:"<sha-of-C2>", mode:"soft"}`.
- **Then** result `status == "aborted"`, `result["restored_sha"] == "<sha-of-C2>"` (every abort tool reports `aborted` and the restored tip, per FR-NEW-199).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` exactly, and the on-disk ref matches.
- **And** `git.log{ref_name:"main"}` = exactly 2 commits, messages `["C2 add b","C1 base"]`.
- **And** volume unchanged: `/a.txt == b"a1\na3\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"` — soft does not touch the working tree.
- **And** `bytes_written()` unchanged (soft writes nothing).
- **Priority:** P0.

---

#### E2E-NEW-664 — Reset soft on a dirty volume keeps the dirt**

**Category:** EdgeCase. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
- **Preconditions:** same dirty state as E2E-NEW-663.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"soft"}`.
- **Then** `/a.txt == b"DIRTY\n"` and `/new.txt == b"n\n"` — untouched.
- **And** `/d.txt == b"d1\n"` still present although C4 is no longer reachable.
- **And** tip == `<sha-of-C2>`.
- **And** a following `git.commit {message:"after soft reset"}` produces a commit whose parent is `<sha-of-C2>` and whose tree contains `/a.txt`=`"DIRTY\n"`, `/b.txt`, `/d.txt`, `/new.txt` — proving soft preserved exactly the volume state (`git.rs:664`, `:679-688`).
- **Priority:** P0.

---

#### E2E-NEW-665 — Reset to the current tip is a no-op in both modes**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-254.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; volume snapshot; `q0 = bytes_written()`; `obj0 = count_objects()`.
- **When (a)** `git.reset {target_ref:"<sha-of-C4>", mode:"soft"}`; **(b)** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** after each: tip string-equal to `<sha-of-C4>`; `git.log` JSON-equal to the pre-call log; every volume path and its bytes equal the snapshot; `count_objects() == obj0`.
- **And** for **(a)** `bytes_written() == q0`. For **(b)**, if the implementation rewrites the identical tree it may charge; the spec fixes the rule as **hard reset to the current tip writes nothing**, so `bytes_written() == q0` is asserted for (b) as well.
- **Priority:** P0.

---

#### E2E-NEW-669 — Reset failure: unknown target_ref**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-253.
- **When** on FX-LINE: `git.reset {target_ref:"nope", mode:"hard"}`, then `target_ref:"0000000000000000000000000000000000000000"`.
- **Then** each fails `ERR_NOT_FOUND` with the given value in the message and the word `"not found"`.
- **And** tip == `<sha-of-C4>`; volume path set and bytes unchanged; `bytes_written` unchanged.
- **Priority:** P0.

---

#### E2E-NEW-670 — Reset failure: missing mode**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-252.
- **When** `git.reset {mount_id:"proj1", target_ref:"<sha-of-C2>"}`.
- **Then** `ERR_INVALID_ARGUMENT` naming `"mode"`.
- **And** tip and volume unchanged.
- **And** `git.reset {mount_id:"proj1", mode:"hard"}` (no `target_ref`) fails `ERR_INVALID_ARGUMENT` naming `"target_ref"`.
- **Priority:** P0.

---

#### E2E-NEW-863 — EdgeCase — a branch name that existed and was deleted**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-253.
- **Preconditions:** FX-LINE plus `git.branch_create {name:"tmp/old", start_point:"<sha-of-C2>"}`, then `git.branch_delete {name:"tmp/old"}`.
- **When** `git.reset {target_ref:"tmp/old", mode:"soft"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"tmp/old"` — the object `<sha-of-C2>` still exists, but the *ref* does not, and refs are resolved as refs.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"soft"}` (the sha the deleted branch pointed at).
- **Then** the call succeeds with `old_sha == "<sha-of-C4>"`, `new_sha == "<sha-of-C2>"` — the orphaned commit is still reachable by sha.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C2>"`.
- **Verification:** error assertion; success response fields; ref read.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-864 — SideEffect — the no-op reset reports equal shas and charges nothing**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-254, FR-NEW-250.
- **Preconditions:** FX-LINE, clean volume, HEAD on `main` = `<sha-of-C4>`; capture `before_audit`, `before_bytes`.
- **When** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the response is exactly `{"mode":"hard","old_sha":"<sha-of-C4>","new_sha":"<sha-of-C4>","files_changed": 0}`.
- **And** `bytes_written == before_bytes` — `files_changed: 0` is not cosmetic, no bytes were charged.
- **And** `audit(OWNER, MOUNT)` holds at most one new entry, and if present its `detail` contains `"no-op"` and no file path.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **Verification:** exact response equality; quota equality; audit inspection; ref read.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-920 — SideEffect — a soft reset forward leaves the volume behind and the status dirty**

**Category:** SideEffect. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
Existing angles: 660 (soft backwards), 664 (soft keeps existing dirt). This one covers a forward soft move, where the volume becomes dirty *because of* the reset.
- **Preconditions:** FX-LINE; `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}` first, so `main` = `<sha-of-C2>` and the volume matches `C2`'s tree (`/a.txt` = `"a1\n"`, `/b.txt` = `"b1\n"`, no `/d.txt`). Volume clean.
- **When** `git.reset {target_ref:"<sha-of-C4>", mode:"soft"}`.
- **Then** the response is exactly `{"mode":"soft","old_sha":"<sha-of-C2>","new_sha":"<sha-of-C4>","files_changed": 0}`.
- **And** `/a.txt` still reads exactly `b"a1\n"` and `VolumeClient::read_bytes("/d.txt")` errors `ERR_NOT_FOUND` — nothing from `C4`'s tree was written.
- **And** `git.status` reports `dirty == true` with `changes` containing `{"path":"/a.txt","status":"modified"}` and `{"path":"/d.txt","status":"deleted"}` relative to the new HEAD.
- **And** `bytes_written` is unchanged from before the reset.
- **Verification:** exact response; byte read and read error; `git.status` change list; quota equality.
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
