# US-011: git.remote_pull: fast-forward, applied atomically

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 11
> Depends On: US-010
> Complexity: L
> min_tier: 1
> Files touched: 2

## Objective

A volume behind its remote catches up: the branch ref advances and the files become the new tree, or nothing happens at all. The pull refuses a dirty volume before fetching and refuses a branch that is not the checked-out one, because the volume is the working tree and there is no staging area.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # pull: fetch, ancestry test, apply
crates/mcp-fs/src/tools/git.rs  # thin tool registration; clone's per-file tolerance at :785-791 is the ANTI-pattern here
```

### Existing Patterns
- Ancestry is tested with `Repository::graph_descendant_of` (`git2-0.20.4/src/repo.rs:2623`).
- Pull is deliberately unlike `git.remote_clone`, which tolerates per-file failures and reports them in `skipped` (`crates/mcp-fs/src/tools/git.rs:785-791`). A half-applied clone is recoverable by re-cloning; a branch advanced over stale files leaves the volume silently inconsistent with its own HEAD (DEC-023).

### Data Model (excerpt)
- No new entity. Advances `refs/heads/{branch}` and rewrites volume files.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-012:** Both `git.remote_fetch` and `git.remote_pull`, with pull refusing anything that is not a fast-forward unless a strategy is given. **Rationale:** the volume is the working tree, so unconstrained merging would need conflict representation in a simulated filesystem. **Implemented by:** FR-NEW-026 to FR-NEW-031. **Round:** 2a. **Code evidence:** n/a
- **DEC-023:** Pull is atomic; a failed file write leaves the ref unadvanced. **Rationale:** unlike clone, which is recoverable by re-cloning, a pull that advances the ref over stale files leaves the volume silently inconsistent with its own HEAD. **Implemented by:** FR-NEW-035. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:785-791`
- **DEC-025:** A dirty volume still refuses the pull; the "commit or discard" guidance is now true because committing no longer strands the volume. **Implemented by:** FR-NEW-029. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Pull is atomic** (§7.4, FR-NEW-035): no ref advances over a partially written tree.
- **Idempotent** (§7.4): a pull with nothing new succeeds with no change (EXC-012b).
- **Usability** (§7.3): the dirty-volume message says to commit or discard first.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

### Open Question Carried From The Spec
§15.2 records an open TBD: dirty-volume detection is a full tree walk on every pull and no performance budget was set at interview. Implement the correct behaviour; if the walk proves costly on a large volume, raise it rather than inventing a cheaper signal.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-028 [EARS-E]: Pull applies a fast-forward
> WHEN `git.remote_pull` is called and the local branch tip is an ancestor of the fetched remote tip, THE mcp-fs server SHALL advance the local branch to the remote tip, update the volume's files to that tree, and report the old sha, the new sha and the count of files changed.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main"}` with the local branch two commits behind.
- **Outputs:** `{"old_sha": …, "new_sha": …, "files_changed": 3, "merged": false}`.
- **Business Rules:** Ancestry is tested with `Repository::graph_descendant_of` (`git2-0.20.4/src/repo.rs:2623`). No merge commit is created.
- **Priority:** Must-have

#### FR-NEW-029 [EARS-O]: Pull refuses a dirty volume
> IF the volume's files differ from the current branch tip when `git.remote_pull` is called, THEN THE mcp-fs server SHALL refuse the pull before fetching, instructing the caller to commit or discard the changes first.

- **Inputs:** `fs.write` to `/README.md`, then `git.remote_pull`.
- **Outputs:** A dedicated error. The volume is untouched and no fetch occurs.
- **Business Rules:** The volume is the working tree; there is no staging area. Overwriting uncommitted edits silently is prohibited.
- **Priority:** Must-have

#### FR-NEW-063 [EARS-O]: Pull targets only the checked-out branch
> IF the `branch` given to `git.remote_pull` is not the branch that `HEAD` currently points at, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` before fetching, naming both the requested branch and the checked-out branch.

- **Inputs:** A volume whose `HEAD` is `refs/heads/main`, called with `{"mount_id":"proj-a","branch":"feature/x"}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `feature/x` and `main`. No fetch, no ref change, no file change.
- **Business Rules:** The volume is the working tree and has exactly one checked-out branch, so applying another branch's tree to it is never correct. `branch` remains required so the caller states its intent explicitly.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A6. FR-NEW-028 required the volume's files to be updated to the pulled tree while `branch` was a free parameter, so one implementation would overwrite the working tree with a foreign branch's content.

#### FR-NEW-035 [EARS-UB]: Pull is atomic
> The mcp-fs server SHALL NOT advance a branch ref when any part of applying the resulting tree to the volume fails.

- **Inputs:** A pull in which one file write fails.
- **Outputs:** `refs/heads/main` unchanged; every volume file unchanged; an error naming the failing path.
- **Business Rules:** Deliberately unlike `git.remote_clone`, which tolerates per-file failures and reports them in `skipped` (`crates/mcp-fs/src/tools/git.rs:785-791`). A half-applied clone is recoverable by re-cloning into a fresh volume; a branch advanced over stale files leaves the volume silently inconsistent with its own HEAD.
- **Priority:** Must-have

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

### The 16 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-114** — A fast-forward pull advances the branch
- **Category:** Core Journey · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** Critical
- Given the local `main` two commits behind and the volume clean
- When `git.remote_pull` is called with `{"mount_id": "proj-a", "branch": "main"}`
- Then the response reports the old sha, the new sha equal to the remote tip, and `merged: false`
- And `refs/heads/main` equals the remote tip

**E2E-NEW-115** — A fast-forward pull updates the volume's files
- **Category:** Side Effect · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** Critical
- Given the incoming commits add `/docs/new.md` and modify `/README.md`
- When the pull runs
- Then `/docs/new.md` exists with the incoming content and `/README.md` matches the new tip
- And the reported `files_changed` equals 2

**E2E-NEW-116** — A fast-forward pull creates no merge commit
- **Category:** State Transition · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** High
- When a fast-forward pull completes
- Then the new tip is the fetched remote commit itself, with its original parent count and sha
- And no commit authored by the server exists

**E2E-NEW-120** — A dirty volume refuses the pull
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** Critical
- Given `fs.write` has modified `/README.md` since the last commit
- When a pull that would fast-forward is attempted
- Then it is refused, instructing the caller to commit or discard first
- And the modified `/README.md` retains the uncommitted content
- And no fetch occurred

**E2E-NEW-121** — A volume with an added uncommitted file is dirty
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** High
- Given `/scratch.txt` was written and never committed
- When a pull is attempted
- Then it is refused as dirty, and `/scratch.txt` survives untouched

**E2E-NEW-122** — A volume with a deleted uncommitted file is dirty
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-029 · **Priority:** High
- Given a tracked file was deleted and not committed
- When a pull is attempted
- Then it is refused as dirty

**E2E-NEW-123** — Committing then pulling succeeds via merge
- **Category:** State Transition · **Scenario:** SC-012 · **Requirements:** FR-NEW-029, FR-NEW-031 · **Priority:** Critical
- Given a dirty volume and a remote that has advanced
- When the caller commits, then pulls with `{"on_conflict": "ours"}`
- Then the pull succeeds with a merge commit
- *This proves the "commit or discard" guidance is not a dead end: the committed divergence is resolvable.*

**E2E-NEW-127** — The diverged error differs from the dirty error
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-029, FR-NEW-030 · **Priority:** High
- When one pull is refused as dirty and another as diverged
- Then the two failures are machine-distinguishable

**E2E-NEW-131** — An up-to-date pull is idempotent
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-028 · **Priority:** High
- When a pull runs twice with no remote change
- Then both succeed, the second reporting no change and `files_changed` of 0

**E2E-NEW-132** — Pull is atomic when a file write fails
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-035 · **Priority:** Critical
- Given a fast-forward whose tree contains one path the volume cannot write
- When the pull runs
- Then it fails naming that path
- And `refs/heads/main` is unchanged and every other file retains its pre-pull content
- *Deliberately unlike clone, which reports `skipped` and continues (`crates/mcp-fs/src/tools/git.rs:785-791`).*

**E2E-NEW-134** — A degenerate diverged pull is a fast-forward
- **Category:** Edge · **Scenario:** SC-013 · **Requirements:** FR-NEW-028 · **Priority:** Medium
- Given the local branch turns out to be a strict ancestor after fetching
- When the pull runs with `on_conflict` supplied
- Then it applies as a fast-forward and creates no merge commit

**E2E-NEW-137** — Merge is atomic when a write fails
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-035 · **Priority:** Critical
- Given a merge whose tree contains one unwritable path
- When the pull runs
- Then no merge commit is created, the ref is not advanced, and every file retains its pre-merge content

**E2E-NEW-138** — A dirty volume refuses the merge too
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-029 · **Priority:** Critical
- Given a dirty volume and a diverged remote, with `on_conflict` supplied
- When the pull runs
- Then it is refused as dirty before any merge is attempted

**E2E-NEW-218** — Pulling a branch other than HEAD is refused
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** Critical
- Given `HEAD` points at `refs/heads/main`
- When `git.remote_pull` is called with `{"mount_id":"proj-a","branch":"feature/x"}`
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `feature/x` and `main`
- And no fetch occurred, no ref changed and no file changed

**E2E-NEW-219** — Pulling the checked-out branch proceeds
- **Category:** Feature · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** Critical
- Given `HEAD` points at `refs/heads/main`
- When the pull names `main`
- Then it proceeds normally

**E2E-NEW-220** — The branch check runs before the network
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-063 · **Priority:** High
- Given a connection recorder and a volume whose `HEAD` is `main`
- When a pull names `feature/x`
- Then zero outbound connections were attempted

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never overwrite uncommitted edits silently (FR-NEW-029, DEC-025).
- Never apply another branch's tree to the working tree (FR-NEW-063).
- Never advance the ref when any part of applying the tree fails (FR-NEW-035).

### Scope Boundary
The fast-forward path, the dirty guard and the checked-out-branch guard. Do NOT implement the write-quota charge (US-012) or the merge (US-013).

## Non Regression

### Existing Tests That Must Pass
- Fetch behaviour from US-010 is unchanged: pull's first step is exactly that fetch, same refspec, no tags.
- A pull that refuses leaves `refs/heads/*` and every volume file untouched.
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
