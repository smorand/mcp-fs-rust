# US-008: Branch creation and ref-name validation

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 8
> Depends On: US-005
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective

Let a caller create a branch. `git.branches` lists today but nothing can create one, so a volume has exactly the branch it was initialised or cloned with. This story adds `git.branch_create` and the ref-name validation every branch tool shares.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Tools are registered in `crates/mcp-fs/src/tools/git.rs:59-360` via `ToolSchema::new(...)`; parameter descriptions are frozen contract text.
Table declaration lives in `crates/mcp-fs/src/git/db.rs:66-90` (`fn schema()`), the owned-table list in `:42` (`TABLES`), and every query is scoped by `volume_id`.
Ref rows are `git_refs` (`crates/mcp-fs/src/git/db.rs:60-68`), keyed `(volume_id, name)` with `name` a `TextKey(400)`.
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

7 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-101 [EARS-E]: Create a branch
> WHEN `git.branch_create` is called with `name` and `start_point` THE mcp-fs server SHALL create ref `refs/heads/{name}` pointing at the commit `start_point` resolves to.

- **Inputs:** `mount_id`, `name` (string), `start_point` (string ref or sha, optional, defaults to current HEAD), `checkout` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, sha, checked_out}`.
- **Business Rules:** HEAD does not move unless `checkout` is `true`. Creation takes the per-project write lock.
- **Priority:** Must-have

### FR-NEW-102 [EARS-O]: Reject a duplicate branch name
> IF `name` already exists as a branch THEN THE mcp-fs server SHALL reject `git.branch_create` with `ERR_INVALID_ARGUMENT` naming the existing branch, and SHALL NOT move the existing ref.

- **Priority:** Must-have

### FR-NEW-103 [EARS-O]: Reject an unresolvable start point
> IF `start_point` resolves to no commit THEN THE mcp-fs server SHALL reject `git.branch_create` with `ERR_NOT_FOUND` naming the unresolved value.

- **Priority:** Must-have

### FR-NEW-104 [EARS-E]: Validate branch names
> WHEN a branch name is supplied to any tool THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` any name that is empty, exceeds 255 bytes, contains an ASCII control character, a space, `~`, `^`, `:`, `?`, `*`, `[`, `\`, or the sequence `..`, begins or ends with `/`, has any `/`-separated component beginning with `.`, or ends with `.lock`.

- **Business Rules:** Mirrors `git check-ref-format`. Valid UTF-8 beyond ASCII is accepted. The limit is **255 bytes of UTF-8, not 255 characters**; a multibyte name is measured after encoding. The 255-byte ceiling keeps `refs/heads/<name>` inside the `TextKey(400)` storage ceiling of `git_refs.name` (`crates/mcp-fs/src/git/db.rs:22`) on every backend, including SQL Server where that renders as `NVARCHAR(400)` (`crates/mcp-fs/src/storage/rel/dialect.rs:235`).
- **Priority:** Must-have

### FR-NEW-115 [EARS-E]: Every branch-mutating tool takes the write lock
> WHEN `git.branch_create`, `git.branch_switch`, `git.branch_delete` or `git.branch_reset` performs its mutation THE mcp-fs server SHALL hold the per-project git write lock for the whole mutation.

- **Business Rules:** The lock is `crates/mcp-fs/src/git/repo.rs:44`, already taken by commit, clone, push and pull.
- **Priority:** Must-have

### FR-NEW-116 [EARS-E]: Branch name length is measured in bytes
> WHEN a branch name is supplied THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` any name whose UTF-8 encoding exceeds 255 bytes, and any name having a `/`-separated component that begins with `.`.

- **Business Rules:** A 200-character name of three-byte codepoints is 600 bytes and is rejected. The rationale is a storage ceiling, not a byte-typed column: `git_refs.name` is `TextKey(400)` (`crates/mcp-fs/src/git/db.rs:22`), which the dialect layer renders as unbounded `TEXT` on SQLite and PostgreSQL but as `NVARCHAR(400)`, a 400-character bound, on SQL Server (`crates/mcp-fs/src/storage/rel/dialect.rs:218`, `:226`, `:235`). A 255-byte cap keeps `refs/heads/<name>` inside that ceiling on every backend. The Section 12 constant is `MAX_BRANCH_NAME_BYTES`.
- **Priority:** Must-have

### FR-NEW-189 [EARS-U]: Branch tool response fields are fixed
> The mcp-fs server SHALL return exactly `{branch, sha, checked_out}` from `git.branch_create`, exactly `{branch, sha, changed, files_changed}` from `git.branch_switch`, exactly `{branch, sha, forced}` from `git.branch_delete`, and exactly `{branch, old_sha, new_sha, checked_out, files_changed}` from `git.branch_reset`.

- **Business Rules:** These names are authoritative over any earlier Outputs line. The names `volume_rewritten`, `previous_sha`, `deleted_sha`, `files_written` and `files_removed` are emitted by no tool.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

11 tests: EdgeCase 4, Failure 5, Happy 1, SideEffect 1.
Scenarios covered: SC-901.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-400 — Happy — create a branch at an explicit start point**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-101.
Preconditions: SEED-A.
- **Given** `refs/heads/main` = `<sha-C2>` and `HEAD` = symbolic `refs/heads/main`.
- **When** `git.branch_create {"mount_id":"gitproj","name":"feature/login","start_point":"<sha-C2>"}`.
- **Then** the response is exactly `{"branch":"feature/login","sha":"<sha-C2>","checked_out":false}` (tool response fields).
- **And** `db.get_ref("refs/heads/feature/login")` returns `GitRefRow { target: "<sha-C2>", symbolic: false }` (direct `git_refs` row check).
- **And** `db.get_ref("HEAD")` still returns `target:"refs/heads/main", symbolic:true`.
- **And** `git.branches` lists exactly two entries with `full_ref` `refs/heads/feature/login` and `refs/heads/main`.
Cleanup: none (tempdir fixture).
Priority: P0.

---

#### E2E-NEW-402 — Failure — an existing branch name is refused**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-102.
Preconditions: SEED-A.
- **When** `git.branch_create {"mount_id":"gitproj","name":"main","start_point":"<sha-C1>"}`.
- **Then** the call errors with `code == ERR_NO_CLOBBER` and `message.contains("branch 'main' already exists")`.
- **And** `db.get_ref("refs/heads/main").target` is still `<sha-C2>` (the existing branch was NOT moved to `<sha-C1>`).
Priority: P0.

---

#### E2E-NEW-403 — Failure — an unresolvable start point**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-103.
Preconditions: SEED-A.
- **When** `git.branch_create {"mount_id":"gitproj","name":"feature/x","start_point":"nosuchref"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("nosuchref")`.
- **And** `db.get_ref("refs/heads/feature/x")` is `None`.
Note: `resolve_ref` treats an all-hex name as a raw sha (`crates/mcp-fs/src/tools/git.rs:486-504`), so the start point MUST contain a non-hex character (`nosuchref` contains `n`,`s`,`u`,`r`) for this to resolve to `None` rather than to a bogus sha. A second assertion covers the hex case: `start_point:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"` also errors `ERR_NOT_FOUND` with `message.contains("deadbeef")`, because the commit object is absent.
Priority: P0.

---

#### E2E-NEW-404 — Failure — an invalid branch name is refused before any write**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A.
- **When** `git.branch_create` is called once per name in `["", "bad..name", "has space", "/leading", "trailing/", "tip.lock", "wild*card", "caret^name"]`, each with `start_point:"<sha-C2>"`.
- **Then** every call errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("is not a valid branch name")`.
- **And** `db.list_refs()` contains exactly `["HEAD","refs/heads/main"]` afterwards (direct `git_refs` check: no partial row for any of the eight names).
Priority: P0.

---

#### E2E-NEW-405 — EdgeCase — a unicode branch name round-trips exactly**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-104, FR-NEW-101., FR-NEW-116
Preconditions: SEED-A.
- **When** `git.branch_create {"name":"feature/café-日本","start_point":"<sha-C2>"}`.
- **Then** the response `"branch"` is exactly `"feature/café-日本"` (NFC bytes as written, not normalized).
- **And** `db.get_ref("refs/heads/feature/café-日本").target == "<sha-C2>"`.
- **And** `git.branches` contains an entry whose `"name"` is exactly `"feature/café-日本"` and whose `"full_ref"` is `"refs/heads/feature/café-日本"`.
- **And** `git.branch_switch {"name":"feature/café-日本"}` succeeds and `git.status` reports `"branch":"feature/café-日本"`.
Priority: P1.

---

#### E2E-NEW-406 — EdgeCase — a 255-byte branch name is accepted**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A. `let name = format!("f/{}", "a".repeat(253));` (exactly 255 chars, asserted in-test with `assert_eq!(name.chars().count(), 255)`).
- **When** `git.branch_create {"name":name,"start_point":"<sha-C2>"}`.
- **Then** the call succeeds and `db.get_ref(&format!("refs/heads/{name}")).target == "<sha-C2>"`.
Priority: P2.

---

#### E2E-NEW-407 — Failure — a 256-byte branch name is refused**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A. `let name = format!("f/{}", "a".repeat(254));` (256 chars, asserted).
- **When** `git.branch_create {"name":name,"start_point":"<sha-C2>"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("256")` and `message.contains("255")`.
- **And** `db.list_refs()` is unchanged (still 2 rows).
Priority: P2.

---

#### E2E-NEW-410 — Failure — create in a repo with no commits**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-103.
Preconditions: `git.init` only (no commit). `db.get_ref("refs/heads/main")` is `None`.
- **When** `git.branch_create {"name":"feature/x","start_point":"HEAD"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("HEAD")` and `message.contains("no commits")`.
- **And** `db.list_refs()` contains no `refs/heads/feature/x` row.
Priority: P1.

---

#### E2E-NEW-818 — EdgeCase — refs are case-sensitive, so `Main` is not a duplicate of `main`**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-102, FR-NEW-104.
- **Preconditions:** SEED-A (`refs/heads/main` = `<sha-C2>`).
- **When** `git.branch_create {"mount_id":"gitproj","name":"Main","start_point":"<sha-C1>"}`.
- **Then** the call succeeds with `{"branch":"Main","sha":"<sha-C1>","checked_out":false}`.
- **And** `db.get_ref("refs/heads/Main").target == "<sha-C1>"` and `db.get_ref("refs/heads/main").target == "<sha-C2>"` — two distinct rows.
- **And** `db.list_refs()` holds exactly `["HEAD","refs/heads/Main","refs/heads/main"]`.
- **And** a repeat `git.branch_create {"name":"Main","start_point":"<sha-C2>"}` now errors `ERR_NO_CLOBBER` containing `"branch 'Main' already exists"`, and `refs/heads/Main` is still `<sha-C1>`.
- **Verification:** two ref reads; ref list equality; repeat-call error assertion.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-819 — SideEffect — a refused duplicate writes no audit entry and charges no quota**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-102.
- **Preconditions:** SEED-A; capture `before_audit = state.safety.audit(OWNER, MOUNT).len()` and `before_bytes = state.safety.bytes_written(OWNER, MOUNT)`.
- **When** `git.branch_create {"name":"main","start_point":"<sha-C1>","checkout":true}` (duplicate **and** requesting a checkout, so a naive implementation would have rewritten the volume first).
- **Then** assert error `ERR_NO_CLOBBER` containing `"branch 'main' already exists"`.
- **And** `state.safety.audit(OWNER, MOUNT).len() == before_audit` and `bytes_written == before_bytes`.
- **And** `db.get_ref("HEAD")` is still `{target:"refs/heads/main", symbolic:true}` and `/src/lib.rs` still reads `b"fn a() {}\n"` (no `<sha-C1>` checkout leaked through).
- **Verification:** audit length and quota compare; ref read; byte compare.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-893 — EdgeCase — a full ref path as `start_point`**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-103, FR-NEW-101.
Existing angles: 403 (unknown short name), 410 (repo with no commits). This one covers fully-qualified ref paths.
- **Preconditions:** SEED-B (`main` = `<sha-C2>`, `release/1.0` = `<sha-C3>`).
- **When** `git.branch_create {"name":"from-full","start_point":"refs/heads/release/1.0"}`.
- **Then** the call succeeds with `sha == "<sha-C3>"` and `db.get_ref("refs/heads/from-full").target == "<sha-C3>"`.
- **When** `git.branch_create {"name":"from-ghost","start_point":"refs/heads/ghost"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/heads/ghost"`.
- **When** `git.branch_create {"name":"from-remote","start_point":"refs/remotes/origin/main"}` with no such tracking ref.
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/remotes/origin/main"`, and `db.list_refs()` contains no `refs/heads/from-remote`.
- **Verification:** success response and ref read; two error assertions; ref list.
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
