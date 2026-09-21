# US-010: git.remote_fetch: objects and remote-tracking refs, nothing else

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 10
> Depends On: US-008
> Complexity: M
> min_tier: 1
> Files touched: 3

## Objective

Fetch is the safe primitive: it downloads objects and moves `refs/remotes/origin/*`, and nothing a person sees in the filesystem moves. It uses one explicit refspec, takes no tags, and reports refs that no longer exist upstream rather than deleting them.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # fetch, sharing the credential pipeline
crates/mcp-fs/src/tools/git.rs  # thin tool registration; git.tags at :112-113 observes tag leakage
```

### Existing Patterns
- libgit2's default fetch follows tags automatically, which is observable through the existing `git.tags` tool (`crates/mcp-fs/src/tools/git.rs:112-113`). Disable it explicitly.
- `git.remote_pull` inherits this refspec, since its first step is this fetch (FR-NEW-062).

### Data Model (excerpt)
- No new entity. Writes `refs/remotes/origin/*`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-012:** Both `git.remote_fetch` and `git.remote_pull`, with pull refusing anything that is not a fast-forward unless a strategy is given. **Rationale:** the volume is the working tree, so unconstrained merging would need conflict representation in a simulated filesystem. **Implemented by:** FR-NEW-026 to FR-NEW-031. **Round:** 2a. **Code evidence:** n/a
- **DEC-037:** Fetch does not prune remote-tracking refs for branches deleted upstream; it reports them in `refs_stale`. **Rationale:** pruning is a destructive ref operation nobody asked for. **Alternatives considered:** pruning silently, as `git fetch --prune` does. Pruning is recorded in `specs/BACKLOG.md` under the standard-git divergence review. **Implemented by:** FR-NEW-056. **Round:** 6 (audit round 1, finding F-010). **Code evidence:** n/a

### Applicable NFRs
- **Idempotent** (§7.4): a fetch with nothing new succeeds with zero refs updated (EXC-011b).
- **Fetch is the safe primitive** (FR-NEW-027): every path in the volume is byte-for-byte identical before and after.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-026 [EARS-E]: Fetch updates objects and remote-tracking refs
> WHEN `git.remote_fetch` is called on a volume with an `origin`, THE mcp-fs server SHALL download new objects and update `refs/remotes/origin/*`, and SHALL report the refs updated and the object count.

- **Inputs:** `{"mount_id": "proj-a"}` with the remote two commits ahead.
- **Outputs:** `refs/remotes/origin/main` at the remote tip; response lists the updated ref.
- **Priority:** Must-have

#### FR-NEW-027 [EARS-UB]: Fetch never changes local refs or files
> The mcp-fs server SHALL NOT modify `refs/heads/*` or any file in the volume as a consequence of `git.remote_fetch`.

- **Inputs:** A fetch against a remote that is ahead.
- **Outputs:** Every path in the volume is byte-for-byte identical before and after. `refs/heads/main` unchanged.
- **Business Rules:** Fetch is the safe primitive. Nothing a person sees in the filesystem moves.
- **Priority:** Must-have

#### FR-NEW-050 [EARS-U]: The fetch response shape is fixed
> The mcp-fs server SHALL return from `git.remote_fetch` an object with `refs_updated`, an array of `{"ref": String, "old_sha": String|null, "new_sha": String}`; `refs_stale`, an array of ref names; `objects_fetched`, an integer; `up_to_date`, a boolean; and `auth`, a string.

- **Inputs:** A fetch bringing `refs/remotes/origin/main` forward two commits.
- **Outputs:** `{"refs_updated":[{"ref":"refs/remotes/origin/main","old_sha":"<40 hex>","new_sha":"<40 hex>"}],"refs_stale":[],"objects_fetched":7,"up_to_date":false,"auth":"github"}`.
- **Business Rules:** `old_sha` is null for a newly created remote-tracking ref. `up_to_date` is true exactly when `refs_updated` is empty.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-004. Push and pull spell their responses; fetch described its own in prose.

#### FR-NEW-062 [EARS-U]: Fetch uses one explicit refspec and takes no tags
> The mcp-fs server SHALL fetch with the single refspec `+refs/heads/*:refs/remotes/origin/*`, SHALL disable automatic tag following, and SHALL report in `refs_updated` only refs under `refs/remotes/origin/`.

- **Inputs:** A fetch from a remote holding branch `main` and tag `v1`.
- **Outputs:** `refs/remotes/origin/main` updated and listed; `refs/tags/v1` absent locally; `git.tags` unchanged.
- **Business Rules:** libgit2's default fetch follows tags automatically, which is observable through the existing `git.tags` tool (`crates/mcp-fs/src/tools/git.rs:112-113`). Tag synchronisation is out of scope and is recorded in `specs/BACKLOG.md`. `git.remote_pull` inherits this refspec, since its first step is this fetch.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A5.

#### FR-NEW-056 [EARS-O]: Fetch does not prune remote-tracking refs
> IF a branch present in `refs/remotes/origin/*` no longer exists on the remote, THEN THE mcp-fs server SHALL leave that remote-tracking ref in place and SHALL list it in the response field `refs_stale`.

- **Inputs:** `origin/feature-x` deleted upstream, then `git.remote_fetch`.
- **Outputs:** `refs/remotes/origin/feature-x` still resolves to its previous sha; the response contains `"refs_stale":["refs/remotes/origin/feature-x"]`.
- **Business Rules:** Pruning is a destructive ref operation and is out of scope. No local ref and no file changes (FR-NEW-027).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-010 and records DEC-037.

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

### The 14 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-086** — Fetch updates remote-tracking refs
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Critical
- Given the remote `main` two commits ahead
- When `git.remote_fetch` is called
- Then `refs/remotes/origin/main` equals the remote tip
- And the response lists that ref and a non-zero object count

**E2E-NEW-087** — Fetch downloads the objects
- **Category:** Side Effect · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Critical
- When a fetch brings two commits
- Then both commit objects and their trees and blobs are present in the volume's object store
- And `git.show` can read the fetched commit

**E2E-NEW-088** — Fetch leaves local refs and files untouched
- **Category:** Side Effect · **Scenario:** SC-011 · **Requirements:** FR-NEW-027 · **Priority:** Critical
- Given a snapshot of every file in the volume and of `refs/heads/main`
- When a fetch runs against a remote that is ahead
- Then every file is byte-for-byte identical afterwards
- And `refs/heads/main` is unchanged

**E2E-NEW-091** — An unreachable remote fails naming the host
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** High
- Given the remote refuses connections
- When a fetch runs
- Then the failure names the host and is distinguishable from a credential failure

**E2E-NEW-092** — Fetch when already current is idempotent
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** High
- When a fetch runs twice with no remote change
- Then both succeed and the second reports zero refs updated

**E2E-NEW-093** — Fetch of a deleted remote branch is handled
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-026 · **Priority:** Medium
- Given `origin/feature-x` existed and the remote deleted it
- When a fetch runs
- Then the outcome is deterministic and reported, and no local ref or file changes

**E2E-NEW-094** — Fetch of a new remote branch creates only a remote-tracking ref
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-027 · **Priority:** High
- Given the remote gained `feature-y`
- When a fetch runs
- Then `refs/remotes/origin/feature-y` exists
- And no `refs/heads/feature-y` is created and no file appears in the volume

**E2E-NEW-185** — The fetch response carries every declared field
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** Critical
- When a fetch advances `refs/remotes/origin/main` by two commits
- Then the response is `{"refs_updated":[{"ref":"refs/remotes/origin/main","old_sha":"<40 hex>","new_sha":"<40 hex>"}],"refs_stale":[],"objects_fetched":<int>,"up_to_date":false,"auth":"github"}`

**E2E-NEW-186** — `old_sha` is null for a new remote-tracking ref
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** High
- When a fetch first creates `refs/remotes/origin/feature-y`
- Then that entry's `old_sha` is null and `new_sha` is the remote tip

**E2E-NEW-187** — `up_to_date` is true exactly when nothing updated
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-050 · **Priority:** High
- When a fetch runs with no remote change
- Then `refs_updated` is empty and `up_to_date` is true

**E2E-NEW-199** — A deleted upstream branch is reported, not pruned
- **Category:** Edge · **Scenario:** SC-011 · **Requirements:** FR-NEW-056 · **Priority:** Critical
- Given `origin/feature-x` was deleted upstream
- When a fetch runs
- Then `refs/remotes/origin/feature-x` still resolves to its previous sha
- And the response contains `"refs_stale":["refs/remotes/origin/feature-x"]`

**E2E-NEW-215** — Fetch brings branches under the declared refspec
- **Category:** Feature · **Scenario:** SC-011 · **Requirements:** FR-NEW-062 · **Priority:** Critical
- Given a remote holding branch `main` and tag `v1`
- When a fetch runs
- Then `refs/remotes/origin/main` is updated and appears in `refs_updated`

**E2E-NEW-216** — Fetch takes no tags
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-062 · **Priority:** Critical
- Given the same remote
- When a fetch runs
- Then `refs/tags/v1` does not exist locally and `git.tags` returns exactly what it returned before

**E2E-NEW-217** — Pull inherits the fetch refspec
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-062 · **Priority:** High
- When a pull runs against a remote holding a new tag
- Then no tag is imported and `git.tags` is unchanged

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never prune a remote-tracking ref for a branch deleted upstream. Report it in `refs_stale` (DEC-037, FR-NEW-056).
- Never report a ref outside `refs/remotes/origin/` in `refs_updated` (FR-NEW-062).

### Scope Boundary
Fetch and its response shape. Do NOT advance a local branch or touch the working tree; that is pull (US-011).

## Non Regression

### Existing Tests That Must Pass
- `refs/heads/*` is never modified by a fetch, and no file in the volume changes (FR-NEW-027).
- `git.tags` output is unchanged by a fetch from a remote holding tags (FR-NEW-062).
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
