# US-007: git.remote_pull on the shared conflict model, on_conflict removed

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 7
> Depends On: US-003, US-004
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Move `git.remote_pull` onto the shared conflict model. A divergent pull now merges automatically when it can and surfaces conflicts when it cannot, instead of refusing unless a global strategy was supplied. The `on_conflict` parameter is removed, which is a breaking change to a shipped tool contract.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/remote.rs
```

### Existing Patterns
The current pull path, including fetch, divergence detection and the merge it performs, is `crates/mcp-fs/src/tools/git.rs:1447-1900`. The parameter being removed is declared at `crates/mcp-fs/src/tools/git.rs:353-378`.
URL validation, host resolution and the credential pipeline are `crates/mcp-fs/src/git/remote.rs` (`validate_remote_url` `:246-262`, `extract_host` `:693`, audit wrapper `:326-371`).
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

- **DEC-908:** The `on_conflict` parameter is removed from `git.remote_pull` outright rather than kept as a backward-compatible fast path. **Rationale:** user's explicit choice when offered both; one way to do things. **Alternatives considered:** keep it as a shorthand for a caller that has already decided, which would have been backward compatible and preserved the archived spec's tests. **Consequence accepted:** a breaking change to a shipped contract, and six shipped tests modified or removed. **Implemented by:** FR-DEL-101, FR-MOD-104, FR-MOD-105. **Round:** 4 (post-test-plan decision). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:353-378` (parameter), `:1447-1485` (`ConflictStrategy`, `parse_on_conflict`).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Remote** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

3 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-MOD-104 [EARS-E]: A diverged pull merges instead of refusing (references archived spec FR-NEW-029, FR-NEW-031)
> WHEN `git.remote_pull` finds the local branch diverged from the fetched remote tip THE mcp-fs server SHALL perform a three-way merge, completing it automatically when no conflict arises and returning the conflict response of FR-NEW-170 when one does.

- **Original behavior:** A diverged pull was refused outright unless `on_conflict` was supplied, and when supplied a single global `ours`/`theirs` strategy was applied to every conflicting file (`crates/mcp-fs/src/tools/git.rs:1835-1843`).
- **New behavior (EARS):** as stated above.
- **Reason for change:** DEC-902. The caller cannot make a sensible global choice without seeing the conflicts.
- **Business Rules:** Fetched objects and updated remote-tracking refs are retained whether or not the merge completes, preserving the behaviour the archived spec established.
- **Priority:** Must-have

### FR-MOD-105 [EARS-E]: A conflicted pull is completed by the merge tools
> WHEN a pull is paused by a conflict THE mcp-fs server SHALL accept `git.merge_resolve` and `git.merge_abort` as the tools that complete or undo it.

- **Original behavior:** No resolution tool existed; resolution was supplied up front.
- **Priority:** Must-have

### FR-DEL-101: The `on_conflict` parameter of `git.remote_pull` (references archived spec FR-NEW-031)
- **Description:** `git.remote_pull` accepted `on_conflict` with value `ours` or `theirs`, applied globally to every conflicting file, and refused a diverged pull outright when it was absent.
- **Reason:** Superseded by the shared conflict model (DEC-902, DEC-908). A global strategy forces the caller to decide before it can see what it is deciding about, and silently discards one side of every conflicting file.
- **Cleanup:** Remove the parameter from the `git.remote_pull` schema (`crates/mcp-fs/src/tools/git.rs:353-378`), remove `ConflictStrategy` and `parse_on_conflict` (`:1447-1485`) or fold them into the shared model's strategy parsing, regenerate the frozen contract, and update the shipped tests listed in Section 12.4.
- **Note:** This is a breaking change to a shipped tool contract. It is deliberate and is recorded in Section 13.

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

22 tests: DataIntegrity 1, EdgeCase 2, Failure 11, Happy 4, SideEffect 4.
Scenarios covered: SC-911, SC-912, SC-913, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-508 — Conflicted pull finished by git.merge_resolve

**Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-MOD-104, FR-NEW-174.
**Category:** Happy. **Priority:** P0.
**Preconditions:** a local bare repo served over `file://` driven through `pull_branch` directly (the same technique the existing tests use, since `resolve_clone_credential` rejects `file://` at the tool boundary — `git.rs:1730-1740` doc comment). Remote `main` has `/src/config.toml` line 2 = `port = 9090`; local `main` has `port = 8000`; common base `port = 8080`.
**When** `git.remote_pull {mount_id:"proj", branch:"main"}` (no `on_conflict`).
**Then** `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`, `conflicts[0].path == "/src/config.toml"`.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"theirs"}]}` — the *same* tool, not a pull-specific one.
**Then** `status == "merged"`; `/src/config.toml` line 2 == `port = 9090`; the merge commit has exactly 2 parents, parent 0 = old local tip, parent 1 = fetched remote tip (mirrors `git.rs:1878-1887`).
**And** `refs/remotes/origin/main` was advanced by the fetch even though the apply was deferred (preserves `git.rs:1820-1823` behaviour).
**Verification:** `parent_count()`, `parent_id(0/1)`, ref reads, file read.
**Cleanup:** fixture drop.

#### E2E-NEW-531 — git.remote_pull blocked while a merge is in progress

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT with an `origin` configured; merge in progress.
**When** `git.remote_pull {mount_id:"proj", branch:"main"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`, `git.merge_resolve`.
**And** **no network call happened**: `refs/remotes/origin/main` is unchanged, and the audit log contains no new `git.remote_pull` entry (per `git/remote.rs:363-369`, a real attempt always writes exactly one).
**Verification:** error code + substrings; ref sha; audit entry count delta == 0.

#### E2E-NEW-535 — the removed on_conflict parameter

**Scenario:** SC-912. **Requirements:** FR-DEL-101, FR-MOD-104.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT with origin.
**Given** today `git.remote_pull` declares `on_conflict` and `parse_on_conflict` accepts `"ours"` (`git.rs:1467-1479`, schema at `git.rs:353`).
**When** `git.remote_pull {mount_id:"proj", branch:"main", on_conflict:"ours"}`.
**Then** the call fails with `ERR_INVALID_ARGUMENT` whose message names `on_conflict` as removed and points at `git.merge_resolve`.
**And** the `git.remote_pull` JSON Schema no longer contains a property named `on_conflict`, and `tool-contract-golden.json` reflects that removal.
**And** the *old* global behaviour is gone: no merge commit is created by this call.
**Verification:** error code + substrings; `registry.resolve("git.remote_pull").schema` property-key assertion; `git.log` length unchanged.

#### E2E-NEW-558 — no conflict markers after a conflicted pull

**Scenario:** SC-912. **Requirements:** FR-NEW-172, FR-MOD-104.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** E2E-NEW-508 setup, pull returns conflict.
**Then** the same full-volume marker scan finds nothing, before and after `git.merge_resolve`.
**Verification:** as E2E-NEW-557.

#### E2E-NEW-811 — Failure — `on_conflict` is rejected even on a pull that would fast-forward**

**Category:** Failure. **Scenario:** SC-912. **Requirements:** FR-DEL-101.
- **Preconditions:** SEED-R; the bare remote advanced by one commit so the local `main` is strictly behind (a pure fast-forward pull, no divergence). Drives the registered tool.
- **When** `git.remote_pull {mount_id:"gitproj", branch:"main", on_conflict:"ours"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"on_conflict"` — the unknown parameter is refused by schema validation before the fast-forward path is reached.
- **And** `db.get_ref("refs/heads/main").target` is still `<sha-C2>` (no fast-forward happened).
- **And** `db.get_ref("refs/remotes/origin/main")` is unchanged (no fetch happened either).
- **Verification:** error assertion; two ref reads compared against values captured before the call.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

#### E2E-NEW-813 — SideEffect — `git.merge_abort` on a paused pull keeps the fetched objects**

**Category:** SideEffect. **Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-MOD-104, FR-NEW-197.
- **Preconditions:** as E2E-NEW-508, paused; the fetched remote tip is `<sha-R2>`.
- **Given** `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"` and `db.object_exists("<sha-R2>")` is `true`.
- **When** `git.merge_abort {mount_id:"proj"}`.
- **Then** the call succeeds and no `git_operations` row remains.
- **And** `db.get_ref("refs/heads/main").target` equals the pre-pull local tip exactly.
- **And** `/src/config.toml` is byte-identical to its pre-pull bytes (line 2 `port = 8000`).
- **And** `db.get_ref("refs/remotes/origin/main").target` is still `<sha-R2>` and `db.object_exists("<sha-R2>")` is still `true` — the abort undoes the merge, not the fetch (`git.rs:1820-1823` behaviour preserved).
- **And** a second `git.remote_pull` re-enters the same conflict without re-downloading, proving the objects are local.
- **Verification:** relational count; ref reads; byte compare; `object_exists`; repeat pull.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-940 — Happy — a divergent pull with disjoint changes merges with no caller decision**

**Category:** Happy. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-DEL-101.
- **Preconditions:** SEED-PULL-DISJOINT. Local `main` = `<sha-L1>`, remote `main` = `<sha-R1>`, merge base `<sha-C0>`.
- **Given** `git.status` reports `dirty == false` and `db.get_ref("refs/heads/main").target == "<sha-L1>"`.
- **When** the pull runs with `branch: "main"` and **no** `on_conflict` parameter (it no longer exists).
- **Then** the response has `status == "merged"`, no `conflicts` key, no `continue_with`/`abort_with` key and no `operation_id` key — the headline change from the old refuse-unless-`on_conflict` model.
- **And** `/src/api.rs` reads exactly `b"pub fn get() {}\npub fn post() {}\n"` (the remote's change).
- **And** `/docs/readme.md` reads exactly `b"docs v2\n"` (the local change, preserved).
- **And** `db.get_ref("refs/heads/main").target` equals the response's `merge_commit`, a 40-hex sha different from both `<sha-L1>` and `<sha-R1>`.
- **And** no `git_operations` row exists for `volume_id='proj'`, and no `git.merge_resolve` call was needed at any point.
- **Verification:** JSON key absence; two byte-exact reads; ref read and sha inequality; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-941 — Failure — a divergent pull is still refused on a dirty volume**

**Category:** Failure. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-194.
- **Preconditions:** SEED-PULL-DISJOINT plus `fs.write_text {path:"/docs/readme.md", content:"docs LOCAL WIP\n"}` (uncommitted).
- **When** the pull runs with `branch: "main"`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"` — auto-merging does not mean merging over unsaved work.
- **And** `/docs/readme.md` still reads exactly `b"docs LOCAL WIP\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-L1>"` and no `git_operations` row exists.
- **And** after `git.commit {message:"wip"}`, the same pull succeeds with `status == "merged"` — the refusal was about dirt, not about divergence.
- **Verification:** error code + substrings; byte compare; ref read; relational count; recovery run.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-942 — Failure — the automatic merge is refused when the quota cannot cover it**

**Category:** Failure. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-185, FR-NEW-184.
- **Preconditions:** SEED-PULL-DISJOINT built on `Env::with_quota` sized below the merged result's byte total; local `main` = `<sha-L1>`.
- **When** the pull runs with `branch: "main"`.
- **Then** assert error `ERR_WRITE_QUOTA_EXCEEDED`.
- **And** `/src/api.rs` reads exactly `b"pub fn get() {}\n"` (the pre-pull local bytes) and `/docs/readme.md` reads exactly `b"docs v2\n"` — nothing was partially written.
- **And** `db.get_ref("refs/heads/main").target == "<sha-L1>"` (no merge commit, no ref move).
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` and `db.object_exists("<sha-R1>")` is `true` — fetched objects and tracking refs are retained, per FR-MOD-104's business rule, so a retry after raising the quota costs no second download.
- **And** after raising the quota, the retry returns `status == "merged"`.
- **Verification:** error code; two byte compares; three ref/object reads; retry.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-943 — EdgeCase — disjoint *regions of the same file* also merge automatically**

**Category:** EdgeCase. **Scenario:** SC-911. **Requirements:** FR-NEW-173, FR-MOD-104.
- **Preconditions:** **SEED-PULL-SAMEFILE**: `C0` commits `/src/config.toml` = `"[server]\nport = 8080\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"`; the remote's `R1` sets line 2 to `port = 9090`; the local `L1` sets line 6 to `retries = 7`. Clean volume, diverged.
- **When** the pull runs with `branch: "main"`.
- **Then** `status == "merged"` with no `conflicts` key.
- **And** `/src/config.toml` reads exactly:
```
[server]
port = 9090
host = "0.0.0.0"
workers = 4
timeout = 30
retries = 7
```
  byte for byte, including the trailing newline — both edits present, one file, no markers.
- **And** the file contains none of `"<<<<<<<"`, `"======="`, `">>>>>>>"`.
- **And** no `git_operations` row exists.
- **Verification:** byte-exact whole-file compare; marker substring absence; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-944 — SideEffect — the automatic pull merge leaves exactly the expected refs, parents and audit**

**Category:** SideEffect. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-NEW-184.
- **Preconditions:** SEED-PULL-DISJOINT; capture `before_bytes` and `before_audit`.
- **When** the pull of E2E-NEW-940 runs and succeeds.
- **Then** the merge commit has exactly 2 parents: `parent_id(0) == "<sha-L1>"` (the local tip) and `parent_id(1) == "<sha-R1>"` (the fetched remote tip), matching the existing ordering at `git.rs:1878-1887`.
- **And** its message is exactly `Merge remote-tracking branch 'origin/main' into main`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`.
- **And** exactly one new audit entry has `op == "git.remote_pull"` with a `detail` containing `"merged"` and not `"conflict"`.
- **And** `bytes_written(OWNER, MOUNT) - before_bytes` equals the byte length of `/src/api.rs`'s new content plus any other file actually rewritten, computed in-test from the merged tree rather than restated as a literal.
- **And** no `git_operations` row was created at any point during the call (checked immediately after).
- **Verification:** `parent_count()`/`parent_id()`; message equality; ref read; audit entry; quota arithmetic; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-945 — Happy — a pull conflict resolved with bytes belonging to neither side**

**Category:** Happy. **Scenario:** SC-913. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-175, FR-NEW-196.
- **Preconditions:** SEED-PULL-CONFLICT.
- **Given** the pull returns `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), `conflicts[0].path == "/src/config.toml"`, `ours.content` line 2 `port = 8000`, `theirs.content` line 2 `port = 9090`, `base.content` line 2 `port = 8080`, `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`.
- **When** `git.merge_resolve {mount_id:"proj", resolutions:[{path:"/src/config.toml", content:"[server]\nport = 8443\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}]}`.
- **Then** `status == "merged"` and `files_changed == 1`.
- **And** `/src/config.toml` reads exactly those supplied bytes, line 2 being `port = 8443` — a value present on neither side, used verbatim with no re-merge.
- **And** the merge commit has exactly 2 parents, `parent_id(0) == "<sha-L1>"`, `parent_id(1) == "<sha-R1>"`.
- **And** `db.get_ref("refs/heads/main").target` equals that commit and no `git_operations` row remains.
- **Verification:** conflict response fields; byte-exact whole-file compare; `parent_count()`/`parent_id()`; ref read; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-946 — Failure — supplied content is refused for a binary path in a pull conflict**

**Category:** Failure. **Scenario:** SC-913. **Requirements:** FR-NEW-182, FR-MOD-105.
- **Preconditions:** **SEED-PULL-BINARY**: `C0` commits `/assets/logo.png` = bytes `89 50 4E 47 0D 0A 1A 0A 00 01`; the remote sets the last byte to `0x02`, the local sets it to `0x03`. The pull returns `status == "conflict"` with `conflicts[0].binary == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", content:"anything"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"binary"`, `"ours"` and `"theirs"`.
- **And** `/assets/logo.png` reads exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]` (the local bytes; nothing applied).
- **And** the `git_operations` row is retained with `op_type == "merge"` (a pull is recorded as a merge, FR-NEW-285) and the path unresolved.
- **And** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", strategy:"theirs"}]}` then succeeds and the last byte is `0x02`.
- **Verification:** error code + substrings; byte-slice equality; relational row; recovery call plus byte read.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-947 — Failure — one bad path rejects the whole content resolution**

**Category:** Failure. **Scenario:** SC-913. **Requirements:** FR-NEW-177, FR-NEW-175, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT extended so two paths conflict: `/src/config.toml` and `/src/app.rs` (both sides edit line 1 differently). The pull pauses with both in the conflict set.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"port = 8443\n"}, {path:"/src/app.rs", content:"fn main() {}\n"}, {path:"/docs/readme.md", content:"docs\n"}]}` where `/docs/readme.md` is **not** in the conflict set.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/docs/readme.md"` and `"not in conflict"`.
- **And** the `git_operations` row's `resolutions` column is still an empty object — the call is all-or-nothing, so the two valid entries were not recorded either.
- **And** `/src/config.toml`, `/src/app.rs` and `/docs/readme.md` are byte-identical to their pre-resolve content.
- **And** re-issuing the call without the third entry succeeds: `status == "merged"` and `/src/config.toml` reads exactly `b"port = 8443\n"`.
- **Verification:** error code + substrings; relational row column; three byte compares; retry.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-948 — EdgeCase — empty supplied content produces an empty file, not a deletion**

**Category:** EdgeCase. **Scenario:** SC-913. **Requirements:** FR-NEW-175, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT, paused on `/src/config.toml`.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:""}]}`.
- **Then** `status == "merged"`.
- **And** `/src/config.toml` **exists** and `VolumeClient::read_bytes` returns a zero-length slice — an empty string is content, not an instruction to delete.
- **And** the merge commit's tree contains an entry for `src/config.toml` (asserted via `git.show`), and per the content-addressing rule an empty file stores no blob, so `COUNT(*) FROM blob_refs` did not grow for it.
- **And** a contrasting resolution in the same test, `{path:"/src/config.toml", strategy:"theirs"}` on a fresh run, yields the non-empty remote content, so the empty case is not a silent fallback.
- **Verification:** volume read length; `git.show` tree entry; blob-ref count; contrasting fresh run.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

#### E2E-NEW-949 — SideEffect — the resolved pull charges exactly the written bytes and clears the row**

**Category:** SideEffect. **Scenario:** SC-913. **Requirements:** FR-NEW-184, FR-NEW-284, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT, paused; capture `paused_bytes = bytes_written(OWNER, MOUNT)` and `paused_audit = audit(OWNER, MOUNT)`.
- **Given** while paused, `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `1` and `bytes_written` equals its pre-pull value (a conflict charges nothing).
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"<69-byte resolved config>"}]}` where the content's byte length is computed in-test as `resolved.len()`.
- **Then** `bytes_written(OWNER, MOUNT) - paused_bytes == resolved.len()` exactly — one charge, for the one file actually written.
- **And** `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `0`.
- **And** exactly one new audit entry appeared since `paused_audit`, with `op == "git.merge_resolve"` (or `"git.remote_pull"` per the registered audit op) and a `detail` containing `"merged"` and `"/src/config.toml"`.
- **And** `git.merge_abort` immediately afterwards errors `ERR_INVALID_ARGUMENT` containing `"no merge is in progress"`, confirming the record is truly cleared and not merely marked.
- **Verification:** quota arithmetic; relational counts before and after; audit diff; follow-up error.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-MOD-401 — `e2e_new_118_a_diverged_pull_with_theirs_merges`**
**Category:** Happy. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** a divergent `git.remote_pull` called with `on_conflict: "theirs"` merged, taking the remote side of every conflicting file through one global `MergeOptions::file_favor`, and committed in the same call.
- **Validates now:** the same divergence produces a conflict response instead of a silent global resolution, and the caller finishes it per file with `git.merge_resolve`. The `on_conflict` parameter no longer exists.
- **Given** a volume whose local `main` and remote `main` have diverged, both sides having changed the same lines of `/shared.txt`, and a clean volume.
- **When** `git.remote_pull {mount_id, branch:"main"}` is called with no `on_conflict` parameter.
- **Then** the call returns `Ok` with `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), and `conflicts[0].path == "/shared.txt"` carrying `ours`, `theirs` and `base` sides.
- **And** no commit is created and the volume is byte-identical to its pre-pull state.
- **And** exactly one `git_operations` row exists for this `volume_id`.
- **When** `git.merge_resolve {mount_id, resolutions:[{path:"/shared.txt", strategy:"theirs"}]}`.
- **Then** `status == "merged"`, a merge commit exists, `/shared.txt` holds the remote side byte-for-byte, and the `git_operations` row is gone.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

#### E2E-MOD-402 — `e2e_new_128_a_diverged_pull_with_ours_keeps_local_content`**
**Category:** Happy. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** a divergent pull with `on_conflict: "ours"` kept the local content of every conflicting file.
- **Validates now:** the identical guarantee, reached through the shared conflict model with a per-file `ours` strategy rather than a global parameter.
- **Given** the divergence of E2E-MOD-401, `/shared.txt` conflicting on both sides.
- **When** `git.remote_pull {mount_id, branch:"main"}`.
- **Then** `status == "conflict"` and nothing is applied.
- **When** `git.merge_resolve {mount_id, resolutions:[{path:"/shared.txt", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `/shared.txt` holds the local pre-pull bytes exactly.
- **And** the merge commit has exactly two parents, the second being the fetched remote tip.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

#### E2E-MOD-403 — `e2e_new_129_non_conflicting_changes_from_both_sides_are_kept`**
**Category:** Happy. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** with `on_conflict` supplied, a divergent pull touching disjoint files kept both sides' changes. Without `on_conflict` the same pull was refused.
- **Validates now:** the pull completes automatically with no caller decision and no strategy parameter at all. This is the test that carries scenario SC-911.
- **Given** local `main` and remote `main` diverged, the local side having changed `/local_only.txt` and the remote side `/remote_only.txt`, no path changed on both sides, volume clean.
- **When** `git.remote_pull {mount_id, branch:"main"}` with no `on_conflict` and no other resolution parameter.
- **Then** the call returns `status == "merged"` (not `"conflict"`, and not an error).
- **And** a merge commit with exactly two parents exists and `refs/heads/main` points at it.
- **And** `/local_only.txt` and `/remote_only.txt` both hold their respective changed bytes.
- **And** no `git_operations` row was ever created for this `volume_id`.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

#### E2E-MOD-404 — `e2e_new_125_a_refused_diverged_pull_keeps_the_fetched_objects`**
**Category:** DataIntegrity. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** when a divergent pull was refused outright, the objects fetched during that pull and the remote-tracking ref were retained rather than rolled back.
- **Validates now:** the same retention guarantee, but the trigger is a conflict response that leaves the operation in progress instead of a refusal.
- **Given** a divergence that conflicts on `/shared.txt`.
- **When** `git.remote_pull {mount_id, branch:"main"}` returns `status == "conflict"`.
- **Then** every object fetched during that call is present in the object store, asserted by reading the remote tip commit object back.
- **And** `refs/remotes/origin/main` equals the fetched remote tip.
- **And** `refs/heads/main` is unmoved.
- **And** a second `git.remote_pull` is rejected by the in-progress guard rather than re-fetching.
- **Cleanup:** `git.merge_abort`, then fixture drop.
- **Priority:** P0.

---

#### E2E-DEL-401 — remove `e2e_new_117_a_diverged_pull_with_no_strategy_is_refused`**
File: `crates/mcp-fs/src/tools/git.rs`.
**Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-DEL-101.
**Reason:** the test asserts that a divergent `git.remote_pull` without `on_conflict` is
refused. FR-MOD-104 makes that same pull auto-merge when it can and return a conflict
response when it cannot, and FR-DEL-101 removes the parameter the refusal was built
around. The refusal no longer exists, so the test cannot be adapted: the positive
behaviour that replaces it is pinned by E2E-MOD-403 (auto-merge) and E2E-MOD-401
(conflict response), so nothing is lost by deleting it.

---

#### E2E-DEL-402 — remove `e2e_new_126_the_diverged_pull_error_names_the_remedy`**
File: `crates/mcp-fs/src/tools/git.rs`.
**Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-DEL-101.
**Reason:** the test asserts the exact wording of the diverged-pull refusal error, telling
the caller to re-run the pull with `on_conflict`. Both the error and the parameter it
names are deleted. The replacement guidance now travels in the conflict response's
`continue_with`/`abort_with` field, which is asserted by E2E-NEW-502 and E2E-MOD-401.

## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not keep `on_conflict` as a hidden alias; it is removed outright. Do not write a pull-specific resolver: a pull conflict is completed by `git.merge_resolve`.

### Scope Boundary
The pull path only. The tests listed as `E2E-MOD-401..404` and `E2E-DEL-401..402` in the parent specification are rewritten or removed as part of this story.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
Fetched objects and updated remote-tracking refs are retained whether or not the merge completes, exactly as today. Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
