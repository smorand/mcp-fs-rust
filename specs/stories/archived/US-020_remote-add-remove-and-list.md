# US-020: Remote add, remove and list

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 20
> Depends On: US-005
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Let a volume have more than one remote. The storage already supports arbitrary names; only the writer was limited to a single `origin` created by clone, which meant a volume created by `git.init` could never push at all.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/db.rs
```

### Existing Patterns
URL validation, host resolution and the credential pipeline are `crates/mcp-fs/src/git/remote.rs` (`validate_remote_url` `:246-262`, `extract_host` `:693`, audit wrapper `:326-371`).
Remote rows are `git_remotes` (`crates/mcp-fs/src/git/db.rs:69-75`). Note `add_remote` (`:281-285`) is an upsert and `remove_remote` (`:287-296`) is an unconditional DELETE, so the duplicate and existence checks MUST live in the tool layer.
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

6 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-140 [EARS-E]: Add a remote
> WHEN `git.remote_add` is called with `name` and `url` THE mcp-fs server SHALL record that remote for the volume.

- **Inputs:** `mount_id`, `name`, `url`.
- **Outputs:** `{name, url, host, provider}`.
- **Business Rules:** The URL passes the existing validation: HTTPS only, no embedded credentials, host declared in `git.hosts`. Rejection happens before the row is written.
- **Priority:** Must-have

### FR-NEW-141 [EARS-O]: Reject a duplicate remote name
> IF `name` already exists as a remote THEN THE mcp-fs server SHALL reject `git.remote_add` with `ERR_INVALID_ARGUMENT` naming the existing remote and its URL, and SHALL NOT overwrite it.

- **Business Rules:** `RelationalGitDb::add_remote` is an upsert (`crates/mcp-fs/src/git/db.rs:281-285`), so the existence check MUST live in the tool layer above it. Calling the storage method blindly would silently rewrite the URL.
- **Priority:** Must-have

### FR-NEW-142 [EARS-E]: Remove a remote
> WHEN `git.remote_remove` is called with `name` THE mcp-fs server SHALL delete that remote and every remote-tracking ref under `refs/remotes/{name}/`.

- **Priority:** Must-have

### FR-NEW-143 [EARS-O]: Removing an unknown remote is not found
> IF `name` matches no remote THEN THE mcp-fs server SHALL reject `git.remote_remove` with `ERR_NOT_FOUND`.

- **Business Rules:** `RelationalGitDb::remove_remote` is an unconditional DELETE (`crates/mcp-fs/src/git/db.rs:287-296`) and therefore succeeds silently on a missing name; the existence check MUST live in the tool layer.
- **Priority:** Must-have

### FR-NEW-144 [EARS-E]: List remotes
> WHEN `git.remote_list` is called THE mcp-fs server SHALL return every remote for the volume with its `name`, `url`, resolved `host` and resolved `provider`.

- **Business Rules:** A volume with no remote returns an empty list, not an error.
- **Priority:** Must-have

### FR-NEW-145 [EARS-UB]: Remote tools never expose a token
> The mcp-fs server SHALL NOT include any token value in the output of `git.remote_list`, `git.remote_add` or any error they raise.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

14 tests: EdgeCase 3, Failure 5, Happy 2, Security 1, SideEffect 3.
Scenarios covered: SC-907.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-471 — Happy — `remote_add` then `remote_list`**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-140, FR-NEW-144.
Preconditions: SEED-A, plus `db.add_remote("origin","https://github.com/o/r.git")`.
- **When** `git.remote_add {"mount_id":"gitproj","name":"upstream","url":"https://github.ibm.com/team/r.git"}`.
- **Then** the response is `{"name":"upstream","url":"https://github.ibm.com/team/r.git"}`.
- **And** `db.list_remotes()` == `[("origin","https://github.com/o/r.git"),("upstream","https://github.ibm.com/team/r.git")]` (ordered by name, per `crates/mcp-fs/src/git/db.rs:298-308`).
- **And** `git.remote_list {"mount_id":"gitproj"}` returns `"remotes"` as exactly those two `{name,url}` objects in that order.
- **And** `safety.audit(OWNER, MOUNT)` ends with one `op == "git.remote_add"` entry whose `detail.contains("upstream")`.
Priority: P0.

---

#### E2E-NEW-472 — Failure — a duplicate remote name is refused, not upserted**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-141.
Preconditions: SEED-A, `db.add_remote("origin","https://github.com/o/r.git")`.
- **When** `git.remote_add {"name":"origin","url":"https://evil.test/x/y.git"}`.
- **Then** `code == ERR_NO_CLOBBER` and `message.contains("remote 'origin' already exists")`.
- **And** `db.list_remotes()` == `[("origin","https://github.com/o/r.git")]` — the URL was NOT overwritten.
Rationale: `RelationalGitDb::add_remote` is an upsert (`crates/mcp-fs/src/git/db.rs:281-285`), so the duplicate check MUST happen in the tool layer above it; this test is the guard against calling the upsert blindly.
Priority: P0.

---

#### E2E-NEW-473 — Failure — non-HTTPS URLs are refused, naming the scheme**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140.
Preconditions: SEED-A, no remotes.
- **When** `git.remote_add {"name":"r1","url":"ssh://git@github.com/o/r.git"}`, then `{"name":"r2","url":"git://github.com/o/r.git"}`, `{"name":"r3","url":"http://github.com/o/r.git"}`, `{"name":"r4","url":"file:///tmp/bare"}`, `{"name":"r5","url":"git@github.com:o/r.git"}`.
- **Then** r1..r4 each error with `code == ERR_INVALID_ARGUMENT` and `message.contains("only https is accepted")`, each naming its own scheme (`"ssh"`, `"git"`, `"http"`, `"file"` respectively) — the exact wording of `validate_remote_url` (`crates/mcp-fs/src/git/remote.rs:256-261`).
- **And** r5 errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("scp-style shorthand")` (`crates/mcp-fs/src/git/remote.rs:248-251`).
- **And** `db.list_remotes()` is empty.
Priority: P0.

---

#### E2E-NEW-474 — Failure — a URL carrying userinfo is refused without echoing the secret**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140, FR-NEW-145.
Preconditions: SEED-A, no remotes.
- **When** `git.remote_add {"name":"leaky","url":"https://alice:ghp_secret@github.com/o/r.git"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`.
- **And** `assert!(!e.message.contains("ghp_secret"))` and `assert!(!e.message.contains("alice:ghp_secret"))` (same property already proven for clone at `crates/mcp-fs/src/git/remote.rs:1069-1076`).
- **And** `db.list_remotes()` is empty.
- **And** `safety.audit(OWNER, MOUNT)` contains no entry whose `detail.contains("ghp_secret")`.
Priority: P0.

---

#### E2E-NEW-475 — SideEffect — the rejected adds of E2E-NEW-472/E2E-NEW-473 wrote no row**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-141, FR-NEW-140.
Preconditions: run E2E-NEW-472's and all five of E2E-NEW-473's calls in one test, starting from `db.add_remote("origin","https://github.com/o/r.git")`.
- **Then** `db.list_remotes()` == `[("origin","https://github.com/o/r.git")]` exactly (direct `git_remotes` check: one row, original URL).
- **And** `git.remote_list` returns exactly one entry.
Priority: P0.

---

#### E2E-NEW-476 — Failure — an invalid remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140.
Preconditions: SEED-A.
- **When** `git.remote_add` with `name` in `["", "has space", "with/slash", "-leading", "a".repeat(256)]` and a valid `url` `"https://github.com/o/r.git"`.
- **Then** each errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("is not a valid remote name")`.
- **And** `db.list_remotes()` is empty.
Priority: P1.

---

#### E2E-NEW-477 — Happy — `remote_remove` deletes the row**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-142.
Preconditions: SEED-R2 (`origin` and `upstream` rows present).
- **When** `git.remote_remove {"mount_id":"gitproj","name":"upstream"}`.
- **Then** the response is `{"name":"upstream","removed":true}`.
- **And** `db.list_remotes()` == `[("origin", <url>)]` (the `upstream` row is gone, `origin` untouched).
- **And** `safety.audit(OWNER, MOUNT)` ends with one `op == "git.remote_remove"` entry whose `detail.contains("upstream")`.
Priority: P0.

---

#### E2E-NEW-478 — Failure — removing an unknown remote is not a silent no-op**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-143.
Preconditions: SEED-R (`origin` only).
- **When** `git.remote_remove {"name":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")`.
- **And** `db.list_remotes()` still == `[("origin", <url>)]`.
Rationale: `RelationalGitDb::remove_remote` is an unconditional DELETE (`crates/mcp-fs/src/git/db.rs:287-296`) and therefore silently succeeds on a missing name; the existence check MUST live in the tool.
Priority: P0.

---

#### E2E-NEW-480 — EdgeCase — `remote_list` with no remotes**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-144.
Preconditions: SEED-A, `git.init` done, `db.list_remotes()` asserted empty.
- **When** `git.remote_list {"mount_id":"gitproj"}`.
- **Then** the response is exactly `{"mount_id":"gitproj","remotes":[]}` and the call is `Ok`.
Priority: P1.

---

#### E2E-NEW-826 — EdgeCase — remote names are case-sensitive**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-143, FR-NEW-141.
- **Preconditions:** SEED-R (remote `origin` -> `<url>`).
- **When** `git.remote_remove {"mount_id":"gitproj","name":"Origin"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"Origin"` (the casing as supplied, echoed back).
- **And** `db.list_remotes()` still holds exactly one row `{name:"origin", url:"<url>"}` — `RelationalGitDb::remove_remote` is an unconditional DELETE (`git/db.rs:287-296`), so this proves the existence check sits in the tool layer above it.
- **And** `git.remote_add {"name":"Origin","url":"<url>"}` then succeeds, and `db.list_remotes()` holds two rows, `origin` and `Origin`.
- **Verification:** error assertion; `list_remotes` row compare; follow-up add.
- **Cleanup:** tempdir drop. **Priority:** P1.

#### E2E-NEW-827 — SideEffect — a failed remove leaves the tracking refs intact**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-143, FR-NEW-142.
- **Preconditions:** SEED-R plus a completed `fetch_branch_inner` so `refs/remotes/origin/main` = `<sha-R1>` exists.
- **When** `git.remote_remove {"name":"upstream"}` (never added).
- **Then** assert error `ERR_NOT_FOUND` containing `"upstream"`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` — the delete of `refs/remotes/{name}/*` mandated by FR-NEW-142 did not run with an empty or wildcard name.
- **And** `db.list_refs()` is element-for-element equal to the list captured before the call.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no `git.remote_remove` entry.
- **Verification:** error assertion; ref read; ref-list vector equality; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### E2E-NEW-900 — EdgeCase — re-adding a remote with the identical URL is still a duplicate**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-141.
Existing angles: 472 (different URL), 475 (no new row). This one removes the "it is idempotent, so allow it" escape.
- **Preconditions:** SEED-R (`origin` -> `<url>`).
- **When** `git.remote_add {"mount_id":"gitproj","name":"origin","url":"<url>"}` — byte-identical name and URL.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"origin"` and `"<url>"` (the error names the existing remote and its URL).
- **And** `db.list_remotes()` still holds exactly one row.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no new `git.remote_add` entry.
- **And** the same call with the URL differing only by a trailing `.git` also errors, so near-identity is not special-cased.
- **Verification:** two error assertions; `list_remotes` length; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

#### E2E-NEW-901 — SideEffect — `remote_remove` deletes only that remote's tracking refs**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-142.
Existing angles: 477 (Happy), 479 (EdgeCase, push after removing origin). This one asserts the ref sweep is scoped.
- **Preconditions:** SEED-R2 with both remotes fetched and two branches each, so `db.list_refs()` contains `refs/remotes/origin/main`, `refs/remotes/origin/dev`, `refs/remotes/upstream/main`, `refs/remotes/upstream/dev`.
- **When** `git.remote_remove {"name":"upstream"}`.
- **Then** the call succeeds and `db.list_remotes()` holds exactly `[{name:"origin", url:"<url>"}]`.
- **And** `db.list_refs()` contains `refs/remotes/origin/main` and `refs/remotes/origin/dev` with their original targets, and contains no ref whose name starts with `refs/remotes/upstream/`.
- **And** `refs/heads/main` and `HEAD` are unchanged, and the volume bytes are unchanged (a remote removal is not a working-tree operation).
- **And** `db.object_exists("<sha-U1>")` is still `true` — the fetched objects are not pruned by the removal.
- **Verification:** `list_remotes`; ref list prefix filtering; ref reads; byte compare; `object_exists`.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

#### E2E-NEW-903 — Security — remote tools stay silent about a stored token**

**Category:** Security. **Scenario:** SC-907. **Requirements:** FR-NEW-145.
Existing angles: 474 (userinfo in the URL), 488 (non-member). This one covers a token that genuinely exists in the store.
- **Preconditions:** fixture with `git.hosts` mapping `github.com: github`; `OAuthTokenStore::store_token` seeds `(owner@test.com, github.com)` with token `ghp_SECRET_VALUE_zzz`; remote `origin` -> `https://github.com/acme/api.git` added.
- **When** `git.remote_list {"mount_id":"gitproj"}` and `git.remote_add {"name":"origin","url":"https://github.com/acme/api.git"}` (the second fails as a duplicate).
- **Then** neither the serialized success response nor the error message contains `"ghp_"` or `"ghp_SECRET_VALUE_zzz"`.
- **And** `git.remote_add {"name":"bad","url":"https://github.com/acme/../../x"}` errors, and that message likewise contains neither.
- **And** the tracing spans captured for the three calls (`tracing_subscriber::fmt::TestWriter` capture) contain neither string.
- **Verification:** substring absence over serialized responses, error strings and captured log output.
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
