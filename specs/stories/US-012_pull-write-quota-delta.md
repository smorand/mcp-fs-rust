# US-012: A pull charges the write quota for the bytes it actually writes

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 12
> Depends On: US-011
> Complexity: M
> min_tier: 1
> Files touched: 2

## Objective

Pulling a 10 KB change into a 200 MB repository charges 10 KB, not 200 MB, so an ordinary pull on a large repository is not refused by the quota. The charge happens before the first write, and an insufficient quota refuses without writing anything and without advancing the ref.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # the charge, computed on the tree the pull will apply
crates/mcp-fs/src/safety.rs  # the session write quota
```

### Existing Patterns
- Clone charges its whole tree (`crates/mcp-fs/src/tools/git.rs:778-782`) because on a clone every file genuinely is new. Pull deliberately differs (DEC-036).
- The quota lives in `crates/mcp-fs/src/safety.rs` alongside the write-quota and audit machinery.

### Data Model (excerpt)
- No new entity.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-036:** A pull charges against the write quota only the bytes of files whose path or content actually changes, not the whole target tree. **Rationale:** charging a 200 MB tree for a 10 KB change would refuse ordinary pulls on large repositories. **Alternatives considered:** charging the whole tree, as clone does, where every file genuinely is new. **Implemented by:** FR-NEW-055. **Round:** 6 (audit round 1, finding F-009). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:778-782`

### Applicable NFRs
- **Refuse before writing** (FR-NEW-069): the charge precedes the first write, and a refusal advances no ref.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-036 [EARS-E]: Pull and merge charge the write quota first
> WHEN `git.remote_pull` computes the tree it will apply, THE mcp-fs server SHALL charge the session write quota before writing any file, on the basis fixed by FR-NEW-069, and SHALL refuse the operation without writing anything when the quota is insufficient.

- **Inputs:** A pull whose tree exceeds `safety.write_quota_bytes`.
- **Outputs:** A quota error; no file written; the ref not advanced.
- **Business Rules:** The basis of the charge is the changed blobs only, fixed by FR-NEW-069. This differs deliberately from clone, which charges its whole tree (`git.rs:778-782`) because on a clone every file genuinely is new.
- **Priority:** Must-have

#### FR-NEW-055 [EARS-E]: The pull quota charge covers only the files written
> WHEN `git.remote_pull` computes the tree it will apply, THE mcp-fs server SHALL charge the summed byte size of only those blobs whose path or content differs from the current volume state, and SHALL charge nothing for a path left untouched.

- **Inputs:** A pull advancing a 200 MB tree in which one 10 KB file changed, with `safety.write_quota_bytes` set to 1 MB.
- **Outputs:** The pull succeeds, charging 10240 bytes.
- **Business Rules:** The charge happens before the first write; an insufficient quota refuses without writing and without advancing the ref (FR-NEW-035). This differs deliberately from clone, which charges the whole tree because every file is new.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-009 and records DEC-036.

#### FR-NEW-069 [EARS-E]: The pull quota charge is the delta
> WHEN `git.remote_pull` computes the tree it will apply, whether by fast-forward or by merge, THE mcp-fs server SHALL charge against the session write quota the summed byte size of only those blobs it will actually write, SHALL charge nothing for a path whose content is unchanged, SHALL perform that charge before the first write, and SHALL refuse the operation without writing anything and without advancing the ref when the quota is insufficient.

- **Inputs:** A 200 MB tree in which one 10 KB file changed, with `safety.write_quota_bytes` of 1 MB; a merged tree whose changed blobs sum to 4 MB, with `safety.write_quota_bytes` of 1 MB.
- **Outputs:** The first succeeds, charging 10240 bytes; the second is refused, no merge commit created and `refs/heads/main` unchanged.
- **Business Rules:** This is the single authority on the basis of the charge. FR-NEW-036 defers to it and FR-NEW-055 is subsumed by it. Clone continues to charge its whole tree (`crates/mcp-fs/src/tools/git.rs:778-782`).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F2 of round 3. FR-NEW-036 required the total tree while FR-NEW-055 required only the delta, and E2E-NEW-133 and E2E-NEW-198 asserted opposite outcomes for the same situation. Both could not pass.

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

### The 6 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-133** — Pull charges the write quota before writing
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-036 · **Priority:** Critical
- Given `safety.write_quota_bytes` lower than the summed size of the blobs the pull would actually write
- When a pull is attempted
- Then it fails with a quota error, no file is written, and the ref is not advanced

**E2E-NEW-141** — A merge exceeding the quota is refused
- **Category:** Edge · **Scenario:** SC-015 · **Requirements:** FR-NEW-036 · **Priority:** High
- Given the merged tree's changed blobs exceed the write quota
- When the pull runs
- Then it fails before writing, creates no merge commit, and leaves the ref unchanged

**E2E-NEW-198** — A pull charges only the changed bytes
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-055 · **Priority:** Critical
- Given a 200 MB tree in which one 10 KB file changed, and `safety.write_quota_bytes` set to 1 MB
- When the pull runs
- Then it succeeds, having charged 10240 bytes, not the tree total

**E2E-NEW-238** — A pull charges only the blobs it writes
- **Category:** Performance · **Scenario:** SC-012 · **Requirements:** FR-NEW-069 · **Priority:** Critical
- Given a 200 MB tree in which one 10 KB file changed, and `safety.write_quota_bytes` of 1 MB
- When the pull runs
- Then it succeeds, having charged 10240 bytes

**E2E-NEW-239** — A merge charges only its changed blobs
- **Category:** Error · **Scenario:** SC-015 · **Requirements:** FR-NEW-069 · **Priority:** Critical
- Given a merged tree whose changed blobs sum to 4 MB and a quota of 1 MB
- When the pull runs with `on_conflict`
- Then it is refused, no merge commit is created and `refs/heads/main` is unchanged

**E2E-NEW-240** — An unchanged path is charged nothing
- **Category:** Edge · **Scenario:** SC-012 · **Requirements:** FR-NEW-069 · **Priority:** High
- Given a fast-forward whose tree is byte-identical to the current volume
- When the pull runs with a quota of 0 bytes
- Then it succeeds, having charged nothing

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never charge a path whose content is unchanged (FR-NEW-069).
- Never charge the whole target tree on a pull. That contradiction was resolved in favour of the delta in audit round 3.

### Scope Boundary
The quota basis for pull, on both the fast-forward and the merge path. FR-NEW-069 is the single authority: FR-NEW-036 defers to it and FR-NEW-055 is subsumed by it.

## Non Regression

### Existing Tests That Must Pass
- Clone's whole-tree charge is unchanged (`crates/mcp-fs/src/tools/git.rs:778-782`).
- Atomicity from US-011 holds: a quota refusal leaves the volume and the ref untouched.
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
