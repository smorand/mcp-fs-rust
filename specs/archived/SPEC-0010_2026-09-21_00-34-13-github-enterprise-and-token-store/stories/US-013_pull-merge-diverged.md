# US-013: A diverged pull: refuse, or merge under one global strategy

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 13
> Depends On: US-011
> Complexity: L
> min_tier: 1
> Files touched: 2

## Objective

When local and remote have diverged, the caller either states a strategy or the operation stops: divergence is never resolved implicitly. With `ours` or `theirs`, a three-way merge resolves every conflicting file by that one strategy and creates a merge commit, and no conflict marker ever enters the volume. This is the specification's stated principal risk.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # the merge path
crates/mcp-fs/src/tools/git.rs  # on_conflict parameter
```

### Existing Patterns
- `MergeOptions::file_favor(FileFavor)` (`git2-0.20.4/src/merge.rs:133-136`), `FileFavor` mapping to `GIT_MERGE_FILE_FAVOR_{NORMAL,OURS,THEIRS,UNION}` (`git2-0.20.4/src/call.rs`), and `Repository::merge_commits` (`git2-0.20.4/src/repo.rs:2177`). Conflict resolution is delegated entirely to libgit2's `file_favor`; this story writes no merge algorithm.
- Exact lowercase value matching mirrors `git.auth`'s provider check (`crates/mcp-fs/src/tools/git_auth.rs:160-162`).

### Data Model (excerpt)
- No new entity. Creates a merge commit with two parents on `refs/heads/{branch}`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-012:** Both `git.remote_fetch` and `git.remote_pull`, with pull refusing anything that is not a fast-forward unless a strategy is given. **Rationale:** the volume is the working tree, so unconstrained merging would need conflict representation in a simulated filesystem. **Implemented by:** FR-NEW-026 to FR-NEW-031. **Round:** 2a. **Code evidence:** n/a
- **DEC-024:** `git.remote_pull` takes an optional `on_conflict` of `ours` or `theirs`; absent refuses a non-fast-forward, present runs a three-way merge with that global strategy. **Rationale:** closes the divergence dead end without interactive resolution. **Alternatives considered:** documenting the dead end; adding `git.discard_changes`; allowing a hard reset that destroys local commits. **Implemented by:** FR-NEW-030, FR-NEW-031, FR-NEW-032, FR-NEW-033. **Round:** 2b. **Code evidence:** `git2-0.20.4/src/merge.rs:133-136`, `git2-0.20.4/src/repo.rs:2177`
- **DEC-026:** The merge commit is authored by the authenticated person. **Implemented by:** FR-NEW-034. **Round:** 2b. **Code evidence:** n/a
- **DEC-027:** The merge commit message is auto-generated and records the strategy; no override. **Rationale:** the resolution stays visible in `git.log` permanently. **Implemented by:** FR-NEW-034. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Principal risk mitigation** (§9.5): the merge path is the largest new surface, mitigated by delegating resolution to `file_favor`, by forbidding conflict markers (FR-NEW-032), and by atomicity (FR-NEW-035).
- **Usability** (§7.3): the refusal names `on_conflict` as the way to merge.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-030 [EARS-O]: A diverged pull without a strategy is refused
> IF the local branch is not an ancestor of the fetched remote tip and `on_conflict` is absent, THEN THE mcp-fs server SHALL refuse the pull with a distinct error naming `on_conflict` as the way to merge, and SHALL retain the fetched objects and updated remote-tracking refs.

- **Inputs:** Diverged branches, `on_conflict` omitted.
- **Outputs:** A dedicated error. `refs/remotes/origin/main` is updated; `refs/heads/main` and the volume's files are unchanged.
- **Business Rules:** The fetch genuinely happened and its results are kept. Refusal applies only to the apply step. Divergence is never resolved implicitly: the caller states a strategy or the operation stops.
- **Priority:** Must-have

#### FR-NEW-031 [EARS-O]: A diverged pull with a strategy merges
> IF the local branch is not an ancestor of the fetched remote tip and `on_conflict` is `ours` or `theirs`, THEN THE mcp-fs server SHALL perform a three-way merge resolving every conflicting file by that single strategy, and SHALL create a merge commit with two parents.

- **Inputs:** `{"mount_id": "proj-a", "branch": "main", "on_conflict": "theirs"}` with both sides having edited `/src/lib.rs`.
- **Outputs:** `{"merged": true, "strategy": "theirs", "merge_commit": "<40 hex>", "conflicts_resolved": 1}`. `/src/lib.rs` holds the remote's content.
- **Business Rules:** Implemented with `MergeOptions::file_favor` (`git2-0.20.4/src/merge.rs:133-136`) set to `FileFavor::Ours` or `FileFavor::Theirs`, and `Repository::merge_commits` (`src/repo.rs:2177`). The merge commit's first parent is the previous local tip, its second the fetched remote tip.
- **Priority:** Must-have

#### FR-NEW-032 [EARS-UB]: No conflict markers ever enter the volume
> The mcp-fs server SHALL NOT write conflict markers, index conflict entries, or any partially merged representation into the volume.

- **Inputs:** Any merge under FR-NEW-031.
- **Outputs:** No file in the volume contains `<<<<<<<`, `=======` or `>>>>>>>` introduced by the merge.
- **Business Rules:** The global strategy resolves every conflict; there is nothing left to represent.
- **Priority:** Must-have

#### FR-NEW-033 [EARS-O]: An invalid conflict strategy is rejected
> IF `on_conflict` holds any value other than `ours` or `theirs`, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT`, naming the accepted values.

- **Inputs:** `{"on_conflict": "union"}`, `{"on_conflict": "OURS"}`, `{"on_conflict": ""}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` listing `ours` and `theirs`. Nothing fetched is applied.
- **Business Rules:** Matching is exact and lowercase, mirroring `git.auth`'s exact provider check (`git_auth.rs:160-162`).
- **Priority:** Must-have

#### FR-NEW-034 [EARS-U]: The merge commit records author and strategy
> The mcp-fs server SHALL author every merge commit with the authenticated person's identity and SHALL set its message to an auto-generated string naming the source ref, the target branch and the conflict strategy applied.

- **Inputs:** A merge by `alice@test.com` of `origin/main` into `main` with `theirs`.
- **Outputs:** Commit author and committer are `alice@test.com`; message is `Merge origin/main into main (conflicts resolved: theirs)`.
- **Business Rules:** No caller-supplied message parameter exists. The strategy stays visible in `git.log` permanently.
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

### The 11 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-117** — A diverged pull with no strategy is refused
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** Critical
- Given local and remote `main` have each gained a distinct commit, and `on_conflict` is omitted
- When the pull runs
- Then it fails with an error stating the pull is not a fast-forward and naming `on_conflict`
- And `refs/heads/main` and every volume file are unchanged

**E2E-NEW-118** — A diverged pull with `theirs` merges
- **Category:** Core Journey · **Scenario:** SC-015 · **Requirements:** FR-NEW-031, FR-NEW-034 · **Priority:** Critical
- Given both sides modified `/src/lib.rs`, local to `fn local()` and remote to `fn remote()`
- When the pull runs with `{"on_conflict": "theirs"}`
- Then a merge commit is created with two parents, the first the previous local tip and the second the fetched remote tip
- And `/src/lib.rs` contains `fn remote()`
- And the response reports `{"merged": true, "strategy": "theirs", "conflicts_resolved": 1}`

**E2E-NEW-125** — A refused diverged pull keeps the fetched objects
- **Category:** Side Effect · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** Critical
- When a pull is refused per E2E-NEW-117
- Then `refs/remotes/origin/main` equals the remote tip
- And the fetched commit objects are present and readable by `git.show`
- *The fetch genuinely happened; only the apply step was refused.*

**E2E-NEW-126** — The diverged-pull error names the remedy
- **Category:** Error · **Scenario:** SC-013 · **Requirements:** FR-NEW-030 · **Priority:** High
- When a pull is refused as diverged
- Then the message names the `on_conflict` parameter and its accepted values

**E2E-NEW-128** — A diverged pull with `ours` keeps local content
- **Category:** Feature · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** Critical
- Given both sides modified `/src/lib.rs`
- When the pull runs with `{"on_conflict": "ours"}`
- Then `/src/lib.rs` contains the local content
- And a merge commit still exists with both parents

**E2E-NEW-129** — Non-conflicting changes from both sides are both kept
- **Category:** Data Integrity · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** Critical
- Given local added `/a.txt` and remote added `/b.txt`, with no common file touched
- When the pull runs with `{"on_conflict": "ours"}`
- Then both `/a.txt` and `/b.txt` exist afterwards
- And `conflicts_resolved` is 0
- *The strategy applies only to conflicting files; it must not discard the other side's non-conflicting work.*

**E2E-NEW-130** — A merged branch then pushes as a fast-forward
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-031, FR-NEW-022 · **Priority:** Critical
- Given a merge completed per E2E-NEW-118
- When `git.remote_push` is called for `main`
- Then it succeeds as a fast-forward
- *This closes the loop: divergence is recoverable end to end.*

**E2E-NEW-135** — An invalid conflict strategy is rejected
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-033 · **Priority:** Critical
- When `on_conflict` is `union`, `OURS`, `Theirs`, `""` or `normal`
- Then each is rejected with `ERR_INVALID_ARGUMENT` naming `ours` and `theirs`
- And nothing is applied

**E2E-NEW-136** — No conflict markers enter the volume
- **Category:** Data Integrity · **Scenario:** SC-015 · **Requirements:** FR-NEW-032 · **Priority:** Critical
- Given a merge resolving three conflicting files
- When the pull completes with either strategy
- Then no file in the volume contains `<<<<<<<`, `=======` or `>>>>>>>` introduced by the merge

**E2E-NEW-139** — A conflict-free merge still creates a merge commit
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-031 · **Priority:** High
- Given diverged branches with no overlapping file
- When the pull runs with `{"on_conflict": "ours"}`
- Then a merge commit with two parents is created and `conflicts_resolved` is 0

**E2E-NEW-140** — The merge commit records author and strategy
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-034 · **Priority:** Critical
- When `alice@test.com` merges `origin/main` into `main` with `theirs`
- Then the commit's author and committer are `alice@test.com`
- And its message is exactly `Merge origin/main into main (conflicts resolved: theirs)`
- And `git.log` shows that message

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never write conflict markers, index conflict entries, or any partially merged representation into the volume (FR-NEW-032).
- Never accept a strategy other than exactly `ours` or `theirs`, lowercase (FR-NEW-033).
- Never add a caller-supplied merge message parameter (DEC-027).

### Scope Boundary
The diverged path only. The fast-forward path is US-011 and is not re-implemented here.

## Non Regression

### Existing Tests That Must Pass
- A refused diverged pull keeps the fetched objects and the updated remote-tracking refs: the fetch genuinely happened (FR-NEW-030).
- Atomicity and the quota charge from US-011 and US-012 apply to the merged tree too.
- A degenerate merge where the local branch is actually an ancestor is handled as a fast-forward with no merge commit (EXC-015f).
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
