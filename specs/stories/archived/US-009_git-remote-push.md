# US-009: git.remote_push: send one branch, fast-forward only

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 9
> Depends On: US-008
> Complexity: L
> min_tier: 1
> Files touched: 3

## Objective

A volume's branch reaches its remote. The tool resolves `origin`, pushes the named branch under the same name, creates it when absent, reports the resulting remote sha, and advances the remote-tracking ref. A non-fast-forward is refused with a distinct error, because force is not supported.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # push, sharing the credential pipeline
crates/mcp-fs/src/tools/git.rs  # thin tool registration
crates/mcp-fs/src/git/db.rs  # list_remotes :298 reads origin; refs read unfiltered at :261-271
```

### Existing Patterns
- Export-before-read and import-after-write (`FR-604`, `FR-605` of the git spec) are the mechanism push uses to get the volume into a working directory.
- `git.status` returns every ref unfiltered (`crates/mcp-fs/src/git/db.rs:261-271` has no prefix filter), so the remote-tracking ref this story writes is directly observable.
- The credential comes from the US-008 pipeline; push constructs no `RemoteCallbacks` of its own.

### Data Model (excerpt)
- No new entity. Reads `git_remotes`; writes `refs/remotes/origin/{branch}`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-013:** Push is fast-forward only; no force. **Implemented by:** FR-NEW-024. **Round:** 2a. **Code evidence:** n/a
- **DEC-014:** Push takes an explicit `branch` and creates it on the remote when absent. **Implemented by:** FR-NEW-022, FR-NEW-023. **Round:** 2a. **Code evidence:** n/a

### Applicable NFRs
- **Idempotent remote operations** (§7.4): pushing twice with no intervening commit succeeds and reports `up_to_date` (FR-NEW-025).
- **Remote operations run off the request thread** (§7.1): push executes on the existing `on_git_thread` path.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-022 [EARS-E]: Push sends one named branch
> WHEN `git.remote_push` is called with a `branch` that exists locally, THE mcp-fs server SHALL push that branch to `origin` under the same name and SHALL report the branch, whether it was created or updated, the resulting remote sha, and the resolved `auth`.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main"}`.
- **Outputs:** `{"branch": "main", "created": false, "remote_sha": "<40 hex>", "auth": "github"}`.
- **Business Rules:** The remote branch name always equals the local one.
- **Priority:** Must-have

#### FR-NEW-023 [EARS-O]: Push creates a branch absent on the remote
> IF the branch named in `git.remote_push` does not exist on the remote, THEN THE mcp-fs server SHALL create it and SHALL report `created` as true.

- **Inputs:** Local branch `feature/x` absent on the remote.
- **Outputs:** `{"branch": "feature/x", "created": true, …}`; the remote then holds `refs/heads/feature/x`.
- **Priority:** Must-have

#### FR-NEW-025 [EARS-O]: An up-to-date push succeeds
> IF the remote branch already points at the sha being pushed, THEN THE mcp-fs server SHALL report success with an up-to-date indication rather than an error.

- **Inputs:** `git.remote_push` called twice in succession with no intervening commit.
- **Outputs:** Both calls succeed; the second reports `up_to_date` true.
- **Business Rules:** Push is idempotent.
- **Priority:** Must-have

#### FR-NEW-060 [EARS-U]: The push response shape is fixed
> The mcp-fs server SHALL return from `git.remote_push` an object with `branch`, a string; `created`, a boolean; `up_to_date`, a boolean; `remote_sha`, a 40 character hex string; and `auth`, a string, all five keys present on every successful call.

- **Inputs:** A push updating `main`; then the same push repeated with no intervening commit.
- **Outputs:** `{"branch":"main","created":false,"up_to_date":false,"remote_sha":"<40 hex>","auth":"github"}`, then the same object with `"up_to_date":true`.
- **Business Rules:** `created` and `up_to_date` are never both true. `remote_sha` is the sha the remote ref holds after the call.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A3. Fetch and `git.auth_status` had their shapes fixed exhaustively; push declared four keys in one requirement and a fifth in another, so `up_to_date` was sometimes present and sometimes absent.

#### FR-NEW-024 [EARS-E]: A non-fast-forward push is refused
> WHEN the remote rejects a push because it is not a fast-forward, THE mcp-fs server SHALL fail with a distinct error stating that the push was refused as non-fast-forward and that force is not supported.

- **Inputs:** A remote branch holding a commit the volume lacks.
- **Outputs:** A dedicated error distinguishable from a credential failure and from a protected-branch rejection. The remote branch is unchanged.
- **Business Rules:** The remote is authoritative on fast-forwardness. No local pre-flight check overrides it.
- **Priority:** Must-have

#### FR-NEW-061 [EARS-E]: A successful push advances the remote-tracking ref
> WHEN `git.remote_push` succeeds for a branch, THE mcp-fs server SHALL set `refs/remotes/origin/{branch}` to the pushed sha, creating that ref when it is absent.

- **Inputs:** A push of `main` at sha S into a volume whose `refs/remotes/origin/main` is at an older sha.
- **Outputs:** `git.status` lists `refs/remotes/origin/main` at S. `refs/heads/main` and every volume file are unchanged.
- **Business Rules:** A refused push (FR-NEW-024) advances nothing. An up-to-date push leaves the ref where it is.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A4. `git.status` returns every ref unfiltered (`crates/mcp-fs/src/git/db.rs:261-271` has no prefix filter), so two implementations would produce different visible output after an identical push.

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run the suite through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly or with an ad hoc command. `--all-features` matters: without it the two optional relational drivers are never compiled.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| `alice@test.com` | Primary person fixture, used by every test below | fixture | ready |
| `bob@test.com` | Second person, for isolation assertions | fixture | ready |
| `proj-a` | Mount id used by every volume-scoped test | fixture | ready |
| `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` | GitHub-shaped token fixture | fixture | ready |
| `glpat_1111222233334444555566667777` | GitLab-shaped token fixture | fixture | ready |
| Reference host map | `github.com: github`, `github.ibm.com: github`, `gitlab.acme.corp: gitlab`, `git.acme.internal: generic`, `public.example.org: anonymous` | fixture | ready |
| `git.unknown.test` | Host deliberately absent from the map | fixture | ready |

### The 17 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-071** — Push updates an existing remote branch
- **Category:** Core Journey · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** Critical
- Given `proj-a` cloned from `https://github.ibm.com/org/repo.git`, one new local commit on `main`
- When `git.remote_push` is called with `{"mount_id": "proj-a", "branch": "main"}`
- Then the response reports `{"branch": "main", "created": false, "remote_sha": <local tip>, "auth": "github"}`
- And the remote's `refs/heads/main` equals the local tip

**E2E-NEW-072** — Push creates a branch absent on the remote
- **Category:** State Transition · **Scenario:** SC-009 · **Requirements:** FR-NEW-023 · **Priority:** Critical
- Given a local branch `feature/x` that the remote does not have
- When it is pushed
- Then the response reports `created: true`
- And the remote holds `refs/heads/feature/x` at the local tip

**E2E-NEW-073** — Push leaves the volume unchanged
- **Category:** Side Effect · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When a push succeeds
- Then every file in the volume is byte-for-byte identical to before
- And `refs/heads/main` is unchanged locally

**E2E-NEW-074** — Push is idempotent when already up to date
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-025 · **Priority:** Critical
- When `git.remote_push` is called twice with no commit in between
- Then both succeed and the second reports `up_to_date: true`

**E2E-NEW-076** — A non-fast-forward push is refused with a distinct error
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** Critical
- Given the remote `main` holds a commit the volume lacks
- When `git.remote_push` is called for `main`
- Then it fails with an error stating the push was refused as non-fast-forward and that force is not supported
- And the remote's `refs/heads/main` is unchanged

**E2E-NEW-077** — The non-fast-forward error is distinguishable from an auth error
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** High
- When a non-fast-forward push and a credential-rejected push are each attempted
- Then the two failures carry different, machine-distinguishable error identities

**E2E-NEW-078** — A branch absent locally is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When `git.remote_push` names `no-such-branch`
- Then it fails naming that branch, before any network call

**E2E-NEW-079** — A protected-branch rejection surfaces the remote's reason
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- Given the remote rejects `main` with a protected-branch message
- When the push runs
- Then the failure includes the remote's own reason and names the branch

**E2E-NEW-081** — A non-member cannot push
- **Category:** Security · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** Critical
- When `bob@test.com`, not a member of `proj-a`, pushes
- Then it fails with `ERR_FORBIDDEN` before host resolution or any token lookup

**E2E-NEW-084** — Concurrent pushes serialize
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-022 · **Priority:** High
- When two pushes to `main` on `proj-a` are issued concurrently
- Then they serialize on the per-repository write lock (`FR-609`)
- And the second either reports up to date or fails as non-fast-forward, never corrupting state

**E2E-NEW-085** — A push racing a remote update fails cleanly
- **Category:** Edge · **Scenario:** SC-010 · **Requirements:** FR-NEW-024 · **Priority:** Medium
- Given the remote advances between the pipeline reading it and the push
- When the push runs
- Then the remote's rejection is authoritative and the same non-fast-forward error is produced

**E2E-NEW-209** — The push response carries all five keys on an update
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** Critical
- When a push updates `main`
- Then the response is exactly `{"branch":"main","created":false,"up_to_date":false,"remote_sha":"<40 hex>","auth":"github"}`

**E2E-NEW-210** — The push response carries all five keys when up to date
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** Critical
- When the same push is repeated with no intervening commit
- Then the response carries the same five keys with `"up_to_date":true`, and no key is absent

**E2E-NEW-211** — `created` and `up_to_date` are never both true
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-060 · **Priority:** High
- When a branch is created on the remote, then pushed again unchanged
- Then the first reports `created:true, up_to_date:false` and the second `created:false, up_to_date:true`

**E2E-NEW-212** — A successful push advances the remote-tracking ref
- **Category:** Side Effect · **Scenario:** SC-009 · **Requirements:** FR-NEW-061 · **Priority:** Critical
- When `main` is pushed at sha S
- Then `git.status` lists `refs/remotes/origin/main` at S
- And `refs/heads/main` and every volume file are unchanged

**E2E-NEW-213** — Push creates the remote-tracking ref when absent
- **Category:** Feature · **Scenario:** SC-009 · **Requirements:** FR-NEW-061 · **Priority:** High
- When `feature/x` is pushed for the first time
- Then `refs/remotes/origin/feature/x` exists at the pushed sha

**E2E-NEW-214** — A refused push advances nothing
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-061 · **Priority:** Critical
- When a push is refused as non-fast-forward
- Then `refs/remotes/origin/main` is exactly where it was before the call

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never force-push, and never add a force parameter (DEC-013).
- Never run a local pre-flight fast-forward check that overrides the remote: the remote is authoritative (FR-NEW-024).
- Never push a branch under a name different from the local one (FR-NEW-022).

### Scope Boundary
Push and its response shape. Do NOT implement fetch (US-010) or pull (US-011). The exact message prefix for the non-fast-forward refusal is fixed in US-014; this story raises the distinct failure, US-014 freezes its wording.

## Non Regression

### Existing Tests That Must Pass
- The volume is never modified by a push: `refs/heads/*` and every file are unchanged (FR-NEW-061).
- A refused push advances nothing, including the remote-tracking ref (FR-NEW-061).
- The whole suite stays green: `cargo test --workspace` with `--all-features`, plus `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`.

### Behaviors That Must Not Change
- Platform admin manages projects and membership and gains **no** implicit file or token access.
- `volume_id` scopes every row of `nodes`, `blob_refs` and `git_*`; it belongs in EVERY `WHERE` clause.
- Secrets come from the environment only. Never log a token or a key.
- A store never speaks a driver: it speaks `storage::rel::RelationalDb`. Never block a request thread on the database.

### API Contracts to Preserve
- `mount_id` stays required on every `fs.*` and `git.*` tool.
- Errors stay `ToolError::<code>` carrying a stable `ERR_*` from the closed set at `crates/mcp-fs/src/errors.rs:9-22`.
- Tool parameter names stay snake_case; parameter descriptions are frozen LLM-facing docs.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5. In addition, before declaring this story done:

1. **Spec compliance.** Every FR above is satisfied by code you can point at, and every test above passes by its own assertion, not by a weakened one.
2. **One implementation.** You added no second copy of host resolution, URL validation, credential supply or timeout. `git2::RemoteCallbacks` is still constructed in exactly one file; `git.hosts` is still read in exactly one file.
3. **No invented decision.** Every choice you made traces to a `DEC-` entry quoted above or to an explicit FR. If you had to decide something this story does not settle, you stopped and said so rather than choosing.
4. **No token anywhere.** No token value reaches a response, a log line, a tracing record, an audit entry, an error message or an HTTP body.
