# US-021: Named remotes on push, fetch and pull, with local:remote refspec

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 21
> Depends On: US-020
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Let push, fetch and pull target a named remote, and let a local branch be pushed under a different name on the remote.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/remote.rs
```

### Existing Patterns
URL validation, host resolution and the credential pipeline are `crates/mcp-fs/src/git/remote.rs` (`validate_remote_url` `:246-262`, `extract_host` `:693`, audit wrapper `:326-371`).
The push path and its tracking-ref update are `crates/mcp-fs/src/tools/git.rs:1107-1262`; fetch is `crates/mcp-fs/src/tools/git.rs:1286-1350`.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-907:** Arbitrary named remotes are supported, reversing the archived spec's DEC-021 which deliberately limited the surface to a single `origin`. **Rationale:** a volume created by `git.init` has no `origin` and could therefore never push, fetch or pull; and fork-based workflows need a second remote. The storage already supports it (`git_remotes` is keyed by name); only the writer was limited. **Alternatives considered:** keep the single-remote limit, rejected because it blocks a standard development process, which is the point of this specification. **Implemented by:** FR-NEW-140 through FR-NEW-147, FR-MOD-101, FR-MOD-103. **Round:** 1 (Q7). **Code evidence:** `crates/mcp-fs/src/git/db.rs:69-75` (table already keyed by name), `:281-305` (add/remove/list already implemented and unused).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Remote** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

5 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-146 [EARS-O]: An unknown remote name fails before the network
> IF the `remote` parameter of `git.remote_push`, `git.remote_fetch` or `git.remote_pull` names a remote that does not exist THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` before opening any connection.

- **Priority:** Must-have

### FR-NEW-147 [EARS-E]: A named fetch touches only that remote's tracking refs
> WHEN `git.remote_fetch` runs against remote `{name}` THE mcp-fs server SHALL update refs under `refs/remotes/{name}/` only, and SHALL leave local branches, tags and volume files unchanged.

- **Priority:** Must-have

### FR-MOD-101 [EARS-E]: `git.remote_push` accepts a target remote (references archived spec FR-NEW-022)
> WHEN `git.remote_push` is called with `remote` THE mcp-fs server SHALL push to that remote instead of `origin`, defaulting to `origin` when the parameter is absent.

- **Original behavior:** Push always resolved `origin`; no remote could be named.
- **Reason for change:** BL-008. A volume created by `git.init` has no `origin` and could therefore not push at all.
- **Priority:** Must-have

### FR-MOD-102 [EARS-E]: `git.remote_push` accepts a distinct remote branch name (references archived spec FR-NEW-022)
> WHEN `git.remote_push` is called with `remote_branch` THE mcp-fs server SHALL push the local `branch` to that differently-named branch on the remote, defaulting to the local branch's name when absent.

- **Original behavior:** The remote branch name always equalled the local one.
- **Reason for change:** BL-009.
- **Priority:** Must-have

### FR-MOD-103 [EARS-E]: `git.remote_fetch` and `git.remote_pull` accept a target remote
> WHEN `git.remote_fetch` or `git.remote_pull` is called with `remote` THE mcp-fs server SHALL operate against that remote, defaulting to `origin`.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

13 tests: EdgeCase 3, Failure 3, Happy 3, SideEffect 4.
Scenarios covered: SC-907, SC-908, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-479 — EdgeCase — removing `origin` makes push report "no origin remote"**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-142, FR-NEW-146.
Preconditions: SEED-R.
- **When** `git.remote_remove {"name":"origin"}`, then `git.remote_push {"mount_id":"gitproj","branch":"main"}`.
- **Then** the remove succeeds and `db.list_remotes()` is empty.
- **And** the push errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("has no origin remote")` — the exact wording of `require_origin` (`crates/mcp-fs/src/git/remote.rs:448`).
- **And** `safety.audit(OWNER, MOUNT)` contains one `op == "git.remote_push"` entry with `detail.contains("outcome error")` (the early-failure path still audits exactly once, `crates/mcp-fs/src/tools/git.rs:446-470`).
Priority: P1.

---

#### E2E-NEW-481 — Happy — fetching a named remote**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-147, FR-MOD-103.
Preconditions: SEED-R2. Advance the second bare repo: `let sha_u2 = advance_bare_remote(&remote_dir2, "refs/heads/main", "upstream2\n");`. Driven at the internal-function level (`fetch_branch_inner(store, MOUNT, &url2, None, "anonymous".into())`, extended with the remote name `"upstream"`), because `file://` cannot reach it through the tool (`crates/mcp-fs/src/git/remote.rs:256`).
- **When** the fetch runs against `upstream`.
- **Then** the response `"refs_updated"` contains `"refs/remotes/upstream/main"` and the response `"objects_fetched"` is `> 0`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-U2>"` (direct `git_refs` check).
Priority: P0.

---

#### E2E-NEW-482 — SideEffect — a named fetch touches only that remote's namespace**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-147.
Preconditions: E2E-NEW-481, with `db.set_ref("refs/remotes/origin/main","<sha-R1>",false)` seeded before the fetch and `let before: Vec<GitRefRow> = db.list_refs()` captured.
- **Then** after the `upstream` fetch, `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` (unchanged).
- **And** `db.list_refs()` contains no name starting with `"refs/remotes/origin/"` other than `refs/remotes/origin/main`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (a fetch never moves a local branch).
- **And** the volume is unchanged: `env.read("/README.md") == "alpha\n"`, and `safety.bytes_written(OWNER, MOUNT)` equals its pre-fetch value (fetch charges no quota, `crates/mcp-fs/src/tools/git.rs:1332-1334`).
Priority: P0.

---

#### E2E-NEW-483 — Failure — fetching an undeclared remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-146, FR-MOD-103.
Preconditions: SEED-R (`origin` only). Driven through the registered `git.remote_fetch` tool (no network is reached, so the https rule is irrelevant).
- **When** `git.remote_fetch {"mount_id":"gitproj","remote":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")` and `message.contains("gitproj")`.
- **And** `db.list_refs()` contains no name starting with `"refs/remotes/upstream/"`.
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `git.remote_fetch` entry, with `detail.contains("outcome error")` (one audit per call, never two, `crates/mcp-fs/src/git/remote.rs:334-371`).
Priority: P0.

---

#### E2E-NEW-484 — Happy — pushing with `remote_branch` creates a differently named remote branch**

**Category:** Happy. **Scenario:** SC-908. **Requirements:** FR-MOD-102.
Preconditions: SEED-R (local `main` = `<sha-C2>`, bare remote `refs/heads/main` = `<sha-R1>`). Driven at the internal-function level through the `call_push_branch` helper pattern (`crates/mcp-fs/src/tools/git.rs:4326`), extended with `remote_branch`.
- **When** `push_branch_inner(..., branch="main", remote_branch=Some("sandbox"), ...)`.
- **Then** the response is `{"branch":"main","remote_branch":"sandbox","created":true,"up_to_date":false,"remote_sha":"<sha-C2>"}`.
- **And** bare-repo inspection: `Repository::open_bare(&remote_dir).find_reference("refs/heads/sandbox")` resolves to `<sha-C2>`.
- **And** `find_reference("refs/heads/main")` on the same bare repo still resolves to `<sha-R1>` (the remote's own `main` was not touched).
Priority: P0.

---

#### E2E-NEW-485 — SideEffect — the tracking ref follows the remote-side name**

**Category:** SideEffect. **Scenario:** SC-908. **Requirements:** FR-MOD-102.
Preconditions: E2E-NEW-484 run to success.
- **Then** `db.get_ref("refs/remotes/origin/sandbox").target == "<sha-C2>"` (direct `git_refs` check; today's code writes `refs/remotes/origin/{branch}`, `crates/mcp-fs/src/tools/git.rs:1259`, which would be the wrong name here).
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` (no tracking ref was created under the local name).
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (the local branch is unmoved).
- **And** the volume is byte-identical: `env.read("/README.md") == "alpha\n"`, `env.read("/src/lib.rs") == "fn a() {}\n"`, and `safety.bytes_written(OWNER, MOUNT)` is unchanged (push charges no quota, `crates/mcp-fs/src/tools/git.rs:1155-1158`).
Priority: P0.

---

#### E2E-NEW-486 — Failure — pushing to an undeclared remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-146, FR-MOD-101.
Preconditions: SEED-R (`origin` only). Driven through the registered `git.remote_push` tool.
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","remote":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")`.
- **And** bare-repo inspection: `refs/heads/main` in `remote_dir` is still `<sha-R1>`, and `find_reference("refs/heads/upstream")` errors (no branch created anywhere).
- **And** `db.list_refs()` contains no `refs/remotes/upstream/` name.
Priority: P0.

---

#### E2E-NEW-487 — EdgeCase — `remote` omitted defaults to `origin`**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-MOD-101.
Preconditions: SEED-R2 (both `origin` and `upstream` declared, pointing at two different bare repos). Driven at the internal-function level so the push really lands.
- **When** the push runs with `remote` absent from the arguments.
- **Then** bare-repo inspection of `remote_dir` (the `origin` repo) shows `refs/heads/main` == `<sha-C2>`.
- **And** bare-repo inspection of `remote_dir2` (the `upstream` repo) shows `refs/heads/main` still == `<sha-U1>` (untouched).
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-C2>"` and `db.get_ref("refs/remotes/upstream/main")` is `None`.
- **And** the same defaulting is asserted for `git.remote_fetch` and `git.remote_pull` with `remote` omitted, via their generated JSON schema: `resolve("git.remote_push").schema["properties"]["remote"]["default"] == "origin"` on all three (registry/schema check, the pattern of `crates/mcp-fs/src/tools/git.rs:2572-2613`).
Priority: P0.

---

#### E2E-NEW-872 — Happy — `git.remote_fetch` runs during a paused merge and updates only tracking refs**

**Category:** Happy. **Scenario:** SC-929. **Requirements:** FR-NEW-280, FR-NEW-147.
- **Preconditions:** `proj` seeded like SEED-CONFLICT plus a bare remote (`seed_bare_remote`) registered as `origin`; `git.merge {source_ref:"feature"}` paused on `/src/config.toml`. `advance_bare_remote` moves the remote to `<sha-R2>`. Fetch is driven through `fetch_branch_inner` (`file://` caveat).
- **When** the fetch runs for `origin`.
- **Then** it succeeds and `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"`.
- **And** `db.get_ref("refs/heads/main").target` is unchanged (the paused merge's target branch did not move).
- **And** `/src/config.toml` is byte-identical to its paused state (line 2 `port = 8000`).
- **And** the `git_operations` row is unchanged field for field, so the fetch neither cleared nor disturbed the pause.
- **And** the merge is still completable: `git.merge_resolve {resolutions:[{path:"/src/config.toml",strategy:"ours"}]}` returns `status == "merged"`.
- **Verification:** ref reads; byte compare; row compare; completion call.
- **Cleanup:** tempdir drop. **Priority:** P0.

#### E2E-NEW-890 — SideEffect — pushing to `upstream` leaves `origin`'s tracking ref alone**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-MOD-101.
Existing angles: 486 (Failure, unknown remote), 487 (EdgeCase, omitted remote defaults to origin). This one is the cross-remote side effect.
- **Preconditions:** SEED-R2 (`origin` -> `<url>` at `<sha-R1>`, `upstream` -> `<url2>` at `<sha-U1>`), both fetched so `refs/remotes/origin/main` = `<sha-R1>` and `refs/remotes/upstream/main` = `<sha-U1>`. Drives `push_branch_inner` with `remote: "upstream"`, `force:true`, `expected_remote_sha:"<sha-U1>"`.
- **When** the push runs.
- **Then** the bare repo at `<url2>` has `refs/heads/main` == `<sha-C2>` and the bare repo at `<url>` still has `refs/heads/main` == `<sha-R1>`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-C2>"` and `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`.
- **And** the audit entry's `detail` contains `"upstream"` and not `"origin"`.
- **Verification:** two bare-repo ref reads; two local tracking-ref reads; audit detail.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

#### E2E-NEW-891 — Failure — an invalid `remote_branch` name is refused before contacting the remote**

**Category:** Failure. **Scenario:** SC-908. **Requirements:** FR-MOD-102, FR-NEW-104.
Existing angles: 484 (Happy), 485 (SideEffect). This one is the validation angle.
- **Preconditions:** SEED-R. Drives the registered `git.remote_push` tool (a validation test).
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","remote_branch":"bad..name"}`, and again with `"has space"`, `"trailing/"`, `"tip.lock"`.
- **Then** every call asserts error `ERR_INVALID_ARGUMENT` containing `"is not a valid branch name"` — the remote-side name is validated by the same rule as a local one.
- **And** the bare remote's ref list is element-for-element equal to the list captured before the calls (no `refs/heads/bad..name` was attempted).
- **And** no audit entry with `op == "git.remote_push"` exists.
- **Verification:** four error assertions; bare-repo `references()` list compare; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-892 — EdgeCase — `git.remote_pull {remote:"upstream"}` pulls upstream and leaves origin untouched**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-MOD-103.
Existing angles: 481 (Happy, fetch from upstream), 483 (Failure, unknown remote). This one covers `remote_pull`, which neither touches.
- **Preconditions:** SEED-R2; local `main` = `<sha-C2>` is an ancestor of `upstream`'s tip `<sha-U2>` (fast-forwardable); `origin` is at `<sha-R1>`, unrelated. Drives `pull_branch` directly.
- **When** the pull runs with `remote: "upstream"`, `branch: "main"`.
- **Then** `status == "fast_forward"` (or `"merged"` if divergent seeds are used) and `db.get_ref("refs/heads/main").target == "<sha-U2>"`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-U2>"` while `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`, unchanged.
- **And** the volume holds `upstream`'s file `/up.txt` with exactly its committed bytes, and none of `origin`'s files.
- **Verification:** status; three ref reads; byte-exact volume reads.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

#### E2E-NEW-904 — SideEffect — a fetch leaves local branches and the volume byte-identical**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-147.
Existing angles: 481 (Happy), 482 (other remote's refs untouched). This one asserts the local side is untouched.
- **Preconditions:** SEED-R with the remote advanced to `<sha-R2>` carrying `/hello.txt` = `"remote-2\n"`; local `main` = `<sha-C2>` with `/README.md` = `"alpha\n"`, `/src/lib.rs` = `"fn a() {}\n"`. Drives `fetch_branch_inner`.
- **Given** `before_refs = db.list_refs()` and the byte content of every volume file.
- **When** the fetch runs for `origin`.
- **Then** `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("HEAD")` unchanged; the only difference between `before_refs` and the new `list_refs()` is the one `refs/remotes/origin/*` entry.
- **And** `/README.md` and `/src/lib.rs` are byte-identical to their captured content, and `/hello.txt` does **not** exist in the volume (`read_bytes` errors `ERR_NOT_FOUND`) even though its object was fetched.
- **And** `bytes_written(OWNER, MOUNT)` is unchanged (a fetch writes objects, not volume bytes, and charges no file quota).
- **Verification:** ref reads and list diff; byte compares; read error; quota equality.
- **Cleanup:** tempdir drop. **Priority:** P0.


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
