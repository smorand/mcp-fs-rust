# US-003: git.merge_resolve, git.merge_abort and resolution mechanics

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 3
> Depends On: US-002
> Complexity: L
> min_tier: 2
> Files touched: 1

## Objective

Let a caller finish or abandon a conflicted merge. This story adds `git.merge_resolve` and `git.merge_abort` and the resolution mechanics behind them: per-file resolution by strategy or by caller-supplied content, partial resolution that keeps the operation open, and an atomic apply that charges the write quota before it writes.

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
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.
The existing global-strategy parser being replaced is `crates/mcp-fs/src/tools/git.rs:1447-1485` (`ConflictStrategy`, `parse_on_conflict`).

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

- **DEC-904:** Continue and abort tools are per operation and named after the git CLI (`git.rebase_continue`, `git.merge_abort`) rather than a single generic trio. **Rationale:** the MCP client is an LLM chat behaving like a terminal, and tool names are its primary affordance; `git rebase --continue` is vocabulary it already has. The engine underneath is shared, so this costs surface area, not duplicated logic. **Alternatives considered:** one generic `git.operation_continue` / `git.operation_abort` / `git.conflict_resolve` trio, rejected for discoverability despite being fewer tools. **Implemented by:** FR-NEW-196, FR-NEW-197, FR-NEW-221, FR-NEW-223, FR-NEW-225, FR-NEW-239. **Round:** 2 (approach exploration, Fork 2). **Code evidence:** n/a, new surface.

### Applicable NFRs

- 7.4 Reliability: refs are never advanced before their files land.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

11 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-196 [EARS-E]: Complete a conflicted merge
> WHEN `git.merge_resolve` is called with `resolutions` while a merge-family operation is in progress THE mcp-fs server SHALL apply those resolutions per FR-NEW-174 through FR-NEW-179 and, once the conflict set is empty, SHALL create the resulting commit and clear the in-progress record.

- **Inputs:** `mount_id`, `resolutions` (object mapping path to `"ours"`, `"theirs"`, or `{content}`).
- **Business Rules:** This one tool completes a conflicted `git.merge` **and** a conflicted `git.remote_pull`, because a pull's conflict is a merge conflict. It does not complete a rebase or a cherry-pick, which have their own continue tools (FR-NEW-221, FR-NEW-239).
- **Priority:** Must-have

### FR-NEW-197 [EARS-E]: Abort a conflicted merge
> WHEN `git.merge_abort` is called while a merge-family operation is in progress THE mcp-fs server SHALL restore HEAD and every volume file to their exact pre-merge values and SHALL delete the in-progress record.

- **Priority:** Must-have

### FR-NEW-198 [EARS-O]: Resolve or abort with no operation in progress
> IF `git.merge_resolve` or `git.merge_abort` is called when no merge-family operation is in progress THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` stating that no merge is in progress.

- **Priority:** Must-have

### FR-NEW-174 [EARS-E]: Resolve by per-file strategy
> WHEN a resolution call supplies, for a conflicting path, the string `ours` or `theirs` THE mcp-fs server SHALL take that path's content entirely from the named side.

- **Business Rules:** `ours` is the side the operation is applied onto; `theirs` is the side being applied. For a rebase this means `ours` is the new base and `theirs` is the commit being replayed, matching git's own convention.
- **Priority:** Must-have

### FR-NEW-175 [EARS-E]: Resolve by supplied content
> WHEN a resolution call supplies, for a conflicting path, an object carrying `content` THE mcp-fs server SHALL use exactly those bytes as that path's content.

- **Business Rules:** This is what lets an LLM caller merge the two sides itself rather than choosing a whole side. Bytes are used verbatim with no re-merge attempted.
- **Priority:** Must-have

### FR-NEW-176 [EARS-E]: Strategy and content mix freely
> WHEN one resolution call supplies strategies for some paths and content for others THE mcp-fs server SHALL apply each path's resolution independently.

- **Priority:** Must-have

### FR-NEW-177 [EARS-O]: Resolving a non-conflicting path is rejected
> IF a resolution call names a path that is not in the active operation's conflict set THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` naming that path, and SHALL NOT apply any part of the call.

- **Business Rules:** The call is all-or-nothing: a single bad path rejects the whole resolution rather than half-applying it.
- **Priority:** Must-have

### FR-NEW-178 [EARS-O]: Partial resolution keeps the operation in progress
> IF a resolution call resolves some but not all paths in the conflict set THEN THE mcp-fs server SHALL retain the in-progress operation, SHALL record the resolutions supplied so far, and SHALL return the remaining unresolved paths.

- **Priority:** Must-have

### FR-NEW-179 [EARS-O]: An invalid strategy value is rejected
> IF a resolution value is a string other than `ours` or `theirs` THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` naming the accepted values.

- **Priority:** Must-have

### FR-NEW-184 [EARS-E]: Applying a completed resolution is atomic
> WHEN the final conflicting path of an operation is resolved THE mcp-fs server SHALL write every resulting file to the volume as a single all-or-nothing unit, SHALL charge the write quota before writing, and SHALL advance the ref only after every file write has succeeded.

- **Business Rules:** Follows the existing `charge_pull_quota` then `apply_pull_changes_atomically` sequence (`crates/mcp-fs/src/tools/git.rs:1967-1989`, `:2079-2127`).
- **Priority:** Must-have

### FR-NEW-185 [EARS-O]: Quota exhaustion mid-resolution changes nothing
> IF the write quota is insufficient for the resolved result THEN THE mcp-fs server SHALL reject with `ERR_WRITE_QUOTA_EXCEEDED`, SHALL leave the volume unchanged, SHALL NOT move the ref, and SHALL retain the in-progress operation so the caller can retry.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

30 tests: Concurrency 1, DataIntegrity 3, EdgeCase 6, Failure 11, Happy 5, SideEffect 4.
Scenarios covered: SC-903, SC-904, SC-905, SC-916, SC-918, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-503 — Resolve by strategy "ours"

**Scenario:** SC-904. **Requirements:** FR-NEW-174, FR-NEW-196.
**Category:** Happy. **Priority:** P0. **Preconditions:** E2E-NEW-502 state (merge in progress, one conflict).
**When** `git.merge_resolve {mount_id:"proj", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
**Then** `status == "merged"`, `merge_commit` is 40-hex, `remaining_conflicts == []`.
**And** `/src/config.toml` line 2 is exactly `port = 8000` (main's side).
**And** `git_operations` count for this volume is 0.
**Verification:** JSON fields; file read split by `\n`; relational count.
**Cleanup:** fixture drop.

#### E2E-NEW-504 — Resolve by literal content

**Scenario:** SC-905. **Requirements:** FR-NEW-175, FR-NEW-196.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT + `git.merge feature` returning conflict.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"[server]\nport = 9000\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}]}`.
**Then** `status == "merged"`.
**And** `read_bytes("/src/config.toml")` equals that exact 64-byte string, not `port = 8000` and not `port = 9090` — proving caller content wins over both sides.
**And** the new commit's tree blob for that path has the same sha as a blob of the supplied bytes.
**Verification:** byte-vector equality; `repo.find_commit(merge_commit).tree().get_path("src/config.toml").id()` compared with `Oid::hash_object(ObjectType::Blob, bytes)`.
**Cleanup:** fixture drop.

#### E2E-NEW-505 — Mixed strategy and content in one call

**Scenario:** SC-905. **Requirements:** FR-NEW-176.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-MULTI(3) → conflicts on `/f/000.txt`, `/f/001.txt`, `/f/002.txt`.
**When** one `git.merge_resolve` with `[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"theirs"},{path:"/f/002.txt",content:"line-MANUAL\n"}]`.
**Then** `status == "merged"`, `remaining_conflicts == []`.
**And** `/f/000.txt` == `line-MAIN\n`; `/f/001.txt` == `line-FEATURE\n`; `/f/002.txt` == `line-MANUAL\n`.
**Verification:** three exact string reads.
**Cleanup:** fixture drop.

#### E2E-NEW-507 — merge_abort restores pre-merge state

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `head_before = refs/heads/main` target and `bytes_before = read_bytes("/src/config.toml")`; run `git.merge feature` → conflict.
**When** `git.merge_abort {mount_id:"proj"}`.
**Then** result `status == "aborted"`, `operation == "merge"`.
**And** `refs/heads/main` target equals `head_before` exactly; `HEAD` still symbolic to `refs/heads/main`.
**And** `read_bytes("/src/config.toml") == bytes_before`.
**And** `git_operations` count == 0.
**Verification:** sha string equality; byte-vector equality; relational count.
**Cleanup:** fixture drop.

#### E2E-NEW-518 — merge_resolve naming a path that is not in conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-177.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT + conflict on `/src/config.toml`; `/keep.txt` exists and is not in conflict.
**When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `/keep.txt` and `not in conflict`.
**And** the merge is still in progress: `git.status` still reports `remaining_conflicts == ["/src/config.toml"]`; `git_operations` count == 1.
**And** no commit was created.
**Verification:** error code + substrings; `git.status` JSON; relational count; log length.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-521 — strategy and content both supplied for one path

**Scenario:** SC-905. **Requirements:** FR-NEW-176, FR-NEW-179.
**Category:** Failure. **Priority:** P0. **Preconditions:** conflict on `/src/config.toml`.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"ours", content:"x\n"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `strategy` and `content` and `exactly one`.
**And** op still in progress (`git_operations` count == 1), nothing written: `/src/config.toml` still `port = 8000`.
**Verification:** error code + substrings; relational count; file read.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-522 — neither strategy nor content

**Scenario:** SC-905. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P0.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml"}]}`.
**Then** `ERR_INVALID_ARGUMENT` containing `exactly one`; op still in progress.
**Verification:** as E2E-NEW-521.

#### E2E-NEW-523 — strategy is case-sensitive

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"Ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `'ours'` and `'theirs'` and `Ours` — matching the existing exact-lowercase convention at `git.rs:1474-1478`.
**And** op still in progress.
**Verification:** error code + three substrings; relational count.

#### E2E-NEW-524 — unknown strategy value

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `strategy:"union"`.
**Then** `ERR_INVALID_ARGUMENT` containing `union`, `'ours'`, `'theirs'`.
**Verification:** as above.

#### E2E-NEW-525 — empty resolutions list

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `git.merge_resolve {resolutions: []}`.
**Then** `ERR_INVALID_ARGUMENT` containing `resolutions` and `at least one`.
**And** op still in progress with the same conflict set.
**Verification:** error code + substrings; `git.status`.

#### E2E-NEW-527 — same path twice in one resolutions array

**Scenario:** SC-904. **Requirements:** FR-NEW-177.
**Category:** Failure. **Priority:** P1.
**When** `resolutions:[{path:"/src/config.toml",strategy:"ours"},{path:"/src/config.toml",strategy:"theirs"}]`.
**Then** `ERR_INVALID_ARGUMENT` containing `/src/config.toml` and `duplicate`.
**And** op still in progress; file unchanged (`port = 8000`).
**Verification:** error code + substrings; file read.

#### E2E-NEW-536 — merge exceeding the write quota

**Scenario:** SC-903. **Requirements:** FR-NEW-185.
**Category:** Failure. **Priority:** P0.
**Preconditions:** `SafetyConfig.max_write_bytes` set to 10 bytes in the fixture tweak. SEED-FF where the feature commit adds `/docs/readme.md` = 64 bytes of `a`.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_WRITE_QUOTA_EXCEEDED` (the code `safety.charge_write` raises, `safety.rs:131`).
**And** `/docs/readme.md` does **not** exist in the volume (`fs.exists` false) — the charge precedes the apply, exactly as on the pull path (`git.rs:1928-1931`).
**And** `refs/heads/main` still `C0`; `git_operations` count == 0.
**Verification:** error code; `fs.exists`; ref sha; relational count.

#### E2E-NEW-537 — resolution content exceeding the quota

**Scenario:** SC-904. **Requirements:** FR-NEW-185.
**Category:** Failure. **Priority:** P1.
**Preconditions:** `max_write_bytes` = 40; SEED-CONFLICT merge in progress (conflict charged 0 bytes so far).
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:<200 'z' chars>}]}`.
**Then** `ERR_WRITE_QUOTA_EXCEEDED`.
**And** `/src/config.toml` still reads `port = 8000` (unchanged main side), no commit created, op still in progress (count == 1).
**Verification:** error code; file read; log length; relational count.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-549 — quota charged exactly the merged bytes

**Scenario:** SC-903. **Requirements:** FR-NEW-185.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-AUTOMERGE; record `before = safety.bytes_written(OWNER,"proj")`.
**When** the auto-merge completes, writing the 64-byte merged `/src/config.toml`.
**Then** `bytes_written - before == 64` exactly (the length of the merged file), matching `charge_pull_quota`'s "written blobs only, deletes add nothing" rule (`git.rs:1967-1985`).
**Verification:** `safety.bytes_written` delta compared with `merged_bytes.len()`.

#### E2E-NEW-550 — conflicted merge charges nothing

**Scenario:** SC-904. **Requirements:** FR-NEW-171, FR-NEW-185.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `before`.
**When** `git.merge` returns `status:"conflict"`.
**Then** `bytes_written - before == 0` exactly.
**Verification:** `safety.bytes_written` delta.

#### E2E-NEW-555 — partial resolve shrinks the conflict set

**Scenario:** SC-904. **Requirements:** FR-NEW-178.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**When** `git.merge_resolve {resolutions:[{path:"/f/001.txt", strategy:"theirs"}]}`.
**Then** the response is `status == "conflict"` (still), `remaining_conflicts == ["/f/000.txt","/f/002.txt"]`, `resolved_count == 1`.
**And** the `git_operations` row persists with a conflict set of exactly those 2 paths.
**And** **nothing is in the volume yet**: `/f/001.txt` still reads `line-MAIN\n`, not `line-FEATURE\n` — partial resolution buffers, it does not write.
**Verification:** JSON fields; relational row payload; exact file read.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-561 — mid-apply failure leaves the volume untouched

**Scenario:** SC-904. **Requirements:** FR-NEW-184.
**Category:** DataIntegrity. **Priority:** P0.
**Preconditions:** SEED-MULTI(3) auto-mergeable variant (feature and main touch distinct files `/f/000.txt` and `/f/002.txt`), plus a pre-existing **directory** at `/f/001.txt` created directly in the volume so the merged tree's write to `/f/001.txt` is refused by the pass-1 pre-check (`check_path_writable`, `git.rs:2121-2140`). Snapshot `before` as in E2E-NEW-560.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `/f/001.txt` and `already exists as a directory`.
**And** the rebuilt map `==` `before`: in particular `/f/000.txt` was **not** written even though it sorts first, proving pass 1 gates every write before pass 2 moves any byte.
**And** `refs/heads/main` unchanged; `git_operations` count == 0; `bytes_written` delta == 0.
**Verification:** error code + substrings; map equality; ref sha; relational count; quota delta.

#### E2E-NEW-562 — abort restores volume and HEAD exactly

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-MULTI(3); snapshot `before` map, `head_sha`, `head_symbolic_target`, `bytes_written_before`.
**When** merge conflicts, one path is partially resolved, then `git.merge_abort`.
**Then** rebuilt map `==` `before`; `refs/heads/main` == `head_sha`; `HEAD` symbolic target == `head_symbolic_target`; `git_operations` count == 0.
**And** `bytes_written - bytes_written_before == 0` (abort writes nothing and therefore charges nothing).
**Verification:** map equality; two ref reads; relational count; quota delta.

#### E2E-NEW-567 — unicode content conflict

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-UNICODE.
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts[0].ours.content` equals exactly `bonjour: kafé ☕\nadieu: naïve\n` and `theirs.content` exactly `bonjour: caffè 🇮🇹\nadieu: naïve\n`, compared as UTF-8 **byte vectors**, not `String` after any normalization (a NFC/NFD change would fail).
**When** resolved with `content:"bonjour: café ☕🇮🇹\nadieu: naïve\n"`.
**Then** `read_bytes("/i18n/fr.txt")` equals exactly those bytes, and its length equals `"bonjour: café ☕🇮🇹\nadieu: naïve\n".as_bytes().len()`.
**Verification:** byte-vector equality on all three.

#### E2E-NEW-571 — resolution content is the empty string

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT conflict in progress.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:""}]}`.
**Then** `status == "merged"` (an explicit empty resolution is valid, distinct from a missing field per E2E-NEW-522).
**And** `read_bytes("/src/config.toml").len() == 0`; the merge commit's tree entry for that path is the empty blob `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`.
**And** `bytes_written` delta == 0 for this write.
**Verification:** byte length; blob oid equality against the known empty-blob sha; quota delta.

#### E2E-NEW-572 — CRLF and no trailing newline preserved

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT conflict in progress.
**When** resolved with `content:"[server]\r\nport = 7000\r\nretries = 2"` (CRLF, no final newline).
**Then** `read_bytes` equals exactly those 41 bytes: last byte is `0x32` (`2`), not `0x0A`; the file contains exactly two `0x0D 0x0A` pairs and no bare `0x0A`.
**Verification:** byte-vector equality plus explicit last-byte and CR-count assertions.

#### E2E-NEW-575 — merge re-runs cleanly after an abort

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT; merge → conflict → abort.
**When** `git.merge {source_ref:"feature"}` again.
**Then** it returns a `status == "conflict"` response **identical** to the first one (assert full JSON equality against the first response), proving abort left no residue.
**When** resolved with `strategy:"theirs"`.
**Then** `status == "merged"`; `/src/config.toml` line 2 == `port = 9090`; exactly one `git.merge` audit entry per attempt (3 entries total: merge, merge_abort, merge, merge_resolve → assert the exact `op` sequence `["git.merge","git.merge_abort","git.merge","git.merge_resolve"]`).
**Verification:** whole-JSON equality; file read; audit `op` vector equality.

#### E2E-NEW-576 — resolving across two sequential partial calls

**Scenario:** SC-904. **Requirements:** FR-NEW-178.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**When** call 1 resolves `/f/000.txt` with `strategy:"ours"`; call 2 resolves `/f/001.txt` with `content:"line-X\n"` and `/f/002.txt` with `strategy:"theirs"`.
**Then** call 1 returns `status == "conflict"` with `remaining_conflicts.len() == 2`; call 2 returns `status == "merged"`.
**And** final contents: `/f/000.txt` == `line-MAIN\n`, `/f/001.txt` == `line-X\n`, `/f/002.txt` == `line-FEATURE\n` — the call-1 decision survived across calls.
**And** exactly one commit created with 2 parents; `git_operations` count == 0.
**Verification:** JSON per call; three file reads; `parent_count()`; relational count.

#### E2E-NEW-578 — per-volume isolation of resolution

**Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** `proj-a` and `proj-b`, both SEED-CONFLICT, both with a merge in progress, OWNER a member of both.
**When** `git.merge_resolve {mount_id:"proj-a", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
**Then** `proj-a` merges: its `/src/config.toml` line 2 == `port = 8000`, its row is gone.
**And** `proj-b` is untouched: its row still exists with the same conflict set, and its `/src/config.toml` still line 2 == `port = 8000` with no commit created.
**Verification:** per-volume relational reads; per-volume file reads scoped through each project's `VolumeClient`; `git.log` length for `proj-b`.
**Cleanup:** abort on `proj-b`.

#### E2E-NEW-617 — Rebase conflict: resolution by literal content**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-175.
- **Preconditions:** E2E-NEW-613 state (paused, conflict on `/a.txt`).
- **When** `git.rebase_continue {resolutions:{"/a.txt":{"content":"a1\nMERGED\n"}}}` (the shared model's literal form).
- **Then** `status == "completed"`.
- **And** `/a.txt` bytes == `b"a1\nMERGED\n"` exactly.
- **And** the replayed F1 commit's tree contains that exact blob: `git.show {commit_sha: commits[1]["sha"]}` diff shows `/a.txt` with `MERGED`.
- **And** no conflict markers anywhere.
- **Priority:** P1.

---

#### E2E-NEW-658 — Cherry-pick failure: quota exhausted**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-185.
- **Preconditions:** FX-FORK-CLEAN on `main`, built with `c.safety.write_quota_bytes` set so that the remaining quota is **1** byte while the pick must write `/f.txt` (3 bytes). *Method:* `Env::build(|c| c.safety.write_quota_bytes = N)` mirroring `safety.rs:339`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `ERR_WRITE_QUOTA_EXCEEDED` with message `session write quota of {N} bytes exceeded` (exact text, `safety.rs:135-137`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` — no commit created.
- **And** `/f.txt` absent from the volume; `/a.txt == b"a1\nMAIN\n"`.
- **And** `bytes_written()` unchanged (a rejected write consumes no quota, `safety.rs:349-352`).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

#### E2E-NEW-871 — SideEffect — partial resolutions recorded before the restart are still applied after it**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-278, FR-NEW-178.
- **Preconditions:** as E2E-NEW-870, paused on three paths.
- **Given** `git.merge_resolve {resolutions:[{path:"/f/000.txt", content:"line-MERGED\n"}]}` returns the remaining unresolved paths `["/f/001.txt","/f/002.txt"]` and the row's `resolutions` column holds the content for `/f/000.txt`.
- **When** the store is dropped and rebuilt, then `git.merge_resolve {resolutions:[{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}`.
- **Then** `status == "merged"`.
- **And** `/f/000.txt` reads exactly `b"line-MERGED\n"` — the literal content supplied *before* the restart, not a side.
- **And** `/f/001.txt` reads `b"line-MAIN\n"` and `/f/002.txt` reads `b"line-FEATURE\n"`.
- **And** no `git_operations` row remains.
- **Verification:** three byte-exact reads; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-908 — Failure — abort after a partial resolve discards the recorded resolutions**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-178, FR-NEW-197.
Existing angles: 555 (SideEffect, shrunk set), 576 (EdgeCase, two sequential calls complete it). This one covers the abort path out of a partial state.
- **Preconditions:** SEED-MULTI(3) on `proj`; merge conflicts on three paths; `git.merge_resolve {resolutions:[{path:"/f/000.txt", content:"line-X\n"}]}` records one resolution and returns the two remaining.
- **When** `git.merge_abort {mount_id:"proj"}`.
- **Then** the call succeeds and no `git_operations` row remains.
- **And** `/f/000.txt` reads exactly `b"line-MAIN\n"` — the pre-merge main-side bytes, not `line-X\n`; recorded resolutions are state of the operation, not of the volume.
- **And** `/f/001.txt` and `/f/002.txt` also read `b"line-MAIN\n"`.
- **And** a fresh `git.merge {source_ref:"feature"}` conflicts again on all **three** paths, with no memory of the earlier partial resolution.
- **And** `git.merge_resolve {resolutions:[{path:"/f/001.txt",strategy:"ours"}]}` before the fresh merge (that is, right after the abort) errors `ERR_INVALID_ARGUMENT` containing `"no merge is in progress"`.
- **Verification:** relational count; three byte reads; re-merge conflict array; error assertion.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-910 — DataIntegrity — the ref advances only after the last file write lands**

**Category:** DataIntegrity. **Scenario:** SC-904. **Requirements:** FR-NEW-184.
Existing angles: 419 (volume unchanged after a failed switch), 561 (volume unchanged after a mid-apply failure). Neither asserts the *ref* ordering, which is what this one pins.
- **Preconditions:** SEED-MULTI(3) on `proj`, merge paused; a fault injected into `VolumeClient` so the write of the **third** file (`/f/002.txt`) fails with an I/O error (the same injection technique E2E-NEW-561 uses); capture `<sha-PRE>` = `refs/heads/main` before the resolve.
- **When** `git.merge_resolve {resolutions:[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}`.
- **Then** the call asserts error `ERR_INTERNAL_ERROR`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-PRE>"` — the ref never advanced, so no commit claims files that are not on disk.
- **And** all three files read their pre-resolve bytes `b"line-MAIN\n"` (the two successful writes were rolled back with the third).
- **And** the `git_operations` row is retained with its three paths, so a retry after clearing the fault completes the merge and *then* moves the ref to a new sha.
- **Verification:** error code; ref equality; three byte compares; relational row; retry with ref change.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-911 — EdgeCase — a 250-path conflict set resolved in one `merge_resolve` call**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-196, FR-NEW-184.
Existing angles: 503 and 504 both resolve a single path. This one covers bulk.
- **Preconditions:** SEED-MULTI(250) on `proj`; `git.merge {source_ref:"feature"}` returns `conflicts.len() == 250`.
- **When** `git.merge_resolve` is called once with 250 resolutions: `strategy:"ours"` for the 125 even-indexed paths, `strategy:"theirs"` for the 125 odd-indexed paths.
- **Then** `status == "merged"` and `files_changed == 250`.
- **And** `/f/000.txt` reads `b"line-MAIN\n"`, `/f/001.txt` reads `b"line-FEATURE\n"`, and the same alternation holds for a sampled `/f/248.txt` and `/f/249.txt`, checked byte-exactly.
- **And** `bytes_written(OWNER, MOUNT)` increased by exactly the sum of the 250 resolved file sizes.
- **And** the merge commit has exactly 2 parents and no `git_operations` row remains.
- **Verification:** response fields; four byte reads; quota arithmetic; `parent_count()`; relational count.
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
