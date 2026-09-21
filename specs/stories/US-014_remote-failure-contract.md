# US-014: Remote timeout, lock release, and the frozen failure messages

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 14
> Depends On: US-009, US-010, US-011, US-013
> Complexity: M
> min_tier: 1
> Files touched: 4

## Objective

A hung remote stops holding a volume hostage: the operation aborts at a deadline and releases the per-repository write lock, so the next operation proceeds. The six remote failure modes get fixed codes and message prefixes, so a client can tell them apart without parsing prose.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # the deadline, enforced twice
crates/mcp-fs/src/git/repo.rs  # the per-repository write lock, a tokio::sync::Mutex at :44
crates/mcp-fs/src/tools/git.rs  # spawn_blocking(... block_on(f())) at :343-352
```

### Existing Patterns
- libgit2 work runs as `spawn_blocking(... block_on(f()))` (`crates/mcp-fs/src/tools/git.rs:343-352`). A `tokio::time::timeout` around that future returns but **does not stop the blocking thread** — that is DRIFT-009.
- The write lock is a `tokio::sync::Mutex` (`crates/mcp-fs/src/git/repo.rs:44`). Acquiring it in the async caller rather than inside the blocking closure is what makes a dropped future release it.
- `RemoteCallbacks::transfer_progress` is the only libgit2 hook available to return an abort; it does not fire during connect or the TLS handshake.
- Error codes come from the closed set at `crates/mcp-fs/src/errors.rs:9-22`. No new `ERR_*` constant is added (FR-NEW-049).

### Data Model (excerpt)
- No new entity. Adds `git.remote_timeout_secs` to `GitConfig` (`crates/mcp-fs/src/config.rs:447-457`), default 120.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-032:** `git.remote_timeout_secs`, configurable, default 120. **Implemented by:** FR-NEW-044. **Round:** 4. **Code evidence:** n/a

### Applicable NFRs
- **Every remote operation is time-bounded** (§7.1): `git.remote_timeout_secs`, default 120, applying to clone, push, fetch and pull.
- **The lock is released** (§9.2, `FR-609`): per-repository write serialization is what makes concurrent pushes safe, and a hung remote must not hold it.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Implementation Drift To Close In This Story

> Carried verbatim from the specification's Section 19. These are resolved **during** this story, not later. An entry still open when the branch merges moves to `specs/BACKLOG.md`, and that move is a decision someone signs, not a silence.

#### DRIFT-009: A blocking libgit2 call cannot be aborted at a deadline
- **Spec says:** FR-NEW-044, the server "SHALL abort it and fail with a distinct timeout error... releasing the per-repository write lock"; E2E-NEW-155 and E2E-NEW-156.
- **Code does:** libgit2 work runs as `spawn_blocking(... block_on(f()))` (`crates/mcp-fs/src/tools/git.rs:343-352`). A `tokio::time::timeout` around that future returns, but **does not stop the blocking thread**. `git2` 0.20.4 exposes no cancellation handle, only `RemoteCallbacks` progress callbacks, which do not fire during connect or the TLS handshake. The write lock is a `tokio::sync::Mutex` (`crates/mcp-fs/src/git/repo.rs:44`).
- **Nature:** missing capability
- **Resolution during implementation:** Acquire the write lock in the async caller rather than inside the blocking closure, so dropping the timed-out future releases it. Enforce the deadline twice: wrap the future in `tokio::time::timeout`, and return an abort from `RemoteCallbacks::transfer_progress` once the deadline has passed. Accept and document that an orphaned blocking thread can outlive the error until its socket times out.
- **Detected by:** E2E-NEW-156 fails, because the second operation blocks on the still-held lock.
- **Blocks which requirement:** FR-NEW-044.
- **Status:** resolved (e2e_new_156_a_timeout_releases_the_repository_lock, this commit)

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-044 [EARS-E]: Remote operations are bounded by a timeout
> WHEN a remote operation exceeds `git.remote_timeout_secs`, THE mcp-fs server SHALL abort it and fail with a distinct timeout error naming the host, releasing the per-repository write lock.

- **Inputs:** `git.remote_timeout_secs: 5` and a remote that never responds.
- **Outputs:** A timeout error naming the host within approximately five seconds. A subsequent operation on the same volume proceeds.
- **Business Rules:** The key defaults to 120 and applies to clone, push, fetch and pull.
- **Priority:** Must-have

#### FR-NEW-049 [EARS-U]: The four remote failure modes carry fixed codes and message prefixes
> The mcp-fs server SHALL report a non-fast-forward push as `ERR_INVALID_ARGUMENT` with the message prefix `push refused: not a fast-forward`, a dirty volume on pull as `ERR_INVALID_ARGUMENT` with the prefix `pull refused: volume has uncommitted changes`, a diverged pull with no strategy as `ERR_INVALID_ARGUMENT` with the prefix `pull refused: not a fast-forward`, and a remote timeout as `ERR_INTERNAL_ERROR` with the prefix `remote timeout`.

- **Inputs:** The four failures of FR-NEW-024, FR-NEW-029, FR-NEW-030 and FR-NEW-044.
- **Outputs:** Exactly those code and prefix pairs, each followed by the host or branch the owning requirement names.
- **Business Rules:** No new `ERR_*` constant is added; the set at `crates/mcp-fs/src/errors.rs:9-22` is closed. The prefixes are stable and are what E2E-NEW-076, E2E-NEW-120, E2E-NEW-126 and E2E-NEW-155 assert.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-003. "Distinct error" is satisfiable by a new code or by a message string; a client distinguishing failures observes a different contract in each case.

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

### The 7 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-155** — A remote operation times out
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Critical
- Given `git.remote_timeout_secs: 5` and a remote that accepts the connection and never responds
- When a clone runs
- Then it fails within approximately five seconds with a distinct timeout error naming the host

**E2E-NEW-156** — A timeout releases the repository lock
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Critical
- Given the timeout of E2E-NEW-155 has fired
- When another git operation on the same volume is issued
- Then it proceeds without waiting, proving the per-repository write lock was released

**E2E-NEW-157** — The timeout default is 120 and is configurable
- **Category:** Performance · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-044 · **Priority:** Medium
- When the config omits `git.remote_timeout_secs`
- Then the effective value is 120
- And setting it to 5 changes the effective value to 5

**E2E-NEW-181** — The non-fast-forward push message prefix is exact
- **Category:** Error · **Scenario:** SC-010 · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a push is refused as non-fast-forward
- Then the error is `ERR_INVALID_ARGUMENT` and the message begins `push refused: not a fast-forward`

**E2E-NEW-182** — The dirty-volume and diverged-pull prefixes are exact and different
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a pull is refused for a dirty volume, then for divergence with no strategy
- Then the first begins `pull refused: volume has uncommitted changes` and the second `pull refused: not a fast-forward`

**E2E-NEW-183** — The timeout code and prefix are exact
- **Category:** Error · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-049 · **Priority:** Critical
- When a remote operation exceeds the timeout
- Then the error is `ERR_INTERNAL_ERROR` and the message begins `remote timeout`

**E2E-NEW-184** — No new error constant is introduced
- **Category:** Edge · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-049 · **Priority:** High
- When the error code set is enumerated after implementation
- Then it still holds exactly the 14 constants of `crates/mcp-fs/src/errors.rs:9-22`

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Do not claim the blocking thread is killed. Accept and document that an orphaned blocking thread can outlive the error until its socket times out (DRIFT-009).
- Do not add a new error code for any of these failures (FR-NEW-049).

### Scope Boundary
The timeout, the lock-release restructuring, and the failure-message contract. The behaviours being named were built in US-009, US-011 and US-013; this story freezes their wording, it does not re-implement them.

## Non Regression

### Existing Tests That Must Pass
- The prefixes frozen here are what E2E-NEW-076, E2E-NEW-120, E2E-NEW-126 and E2E-NEW-155 assert; those tests belong to US-009, US-011 and this story and must all agree.
- A subsequent operation on the same volume proceeds after a timeout (E2E-NEW-156).
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
5. **Drift closed.** DRIFT-009 is resolved in this branch, by the resolution quoted above, and you can name the test that proves it.
