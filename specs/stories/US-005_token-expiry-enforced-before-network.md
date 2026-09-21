# US-005: An expired token fails the operation before any socket opens

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 5
> Depends On: US-002, US-003
> Complexity: S
> min_tier: 1
> Files touched: 3

## Objective

A remote operation carrying an expired stored token fails immediately, naming the host and telling the person to re-authenticate, instead of opening a connection and surfacing an opaque transport error. The expired token stays in the store so status can report `expired` rather than absent.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # the credential-supply step of the pipeline
crates/mcp-fs/src/git/oauth/store.rs  # get_token :148-150 ignores expiry; is_valid_at :34; has_valid_token :169
```

### Existing Patterns
- The defect: the clone path uses `get_token` (`crates/mcp-fs/src/tools/git.rs:704`), which returns the session **without checking expiry** (`crates/mcp-fs/src/git/oauth/store.rs:148-150`). The expiry logic already exists at `OAuthSession::is_valid_at` (`:34`) and `has_valid_token` (`:169`); neither is on the clone path.
- Error codes come from the closed set at `crates/mcp-fs/src/errors.rs:9-22`. `ERR_UNAUTHENTICATED` is the code for both failures here; no new constant is added.

### Data Model (excerpt)
- No new entity. Reads `OAuthSession.expires_at`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-019:** An expired stored token is treated as absent-with-a-reason; operations fail fast telling the person to re-authenticate. **Implemented by:** FR-NEW-018. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:704`, `crates/mcp-fs/src/git/oauth/store.rs:148-150`, `:169`
- **DEC-020:** An expired token is left in the store, not deleted on detection. **Rationale:** lets status report `expired` rather than absent; deletion on a read path is a surprising side effect. **Implemented by:** FR-NEW-019. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Fail before the network** (§7.2, FR-NEW-018): no socket is opened when the stored token has expired.
- **Usability** (§7.3): the message names the host and the remedy, re-authentication.

### Bounded Context
**Token Custody** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-018 [EARS-E]: Expiry is enforced before the network
> WHEN a remote operation resolves a stored token whose `expires_at` is in the past, THE mcp-fs server SHALL fail the operation before opening any network connection, naming the host and instructing the person to re-authenticate.

- **Inputs:** A stored token for `(alice@test.com, github.ibm.com)` with `expires_at` one hour in the past, then `git.remote_clone`.
- **Outputs:** An error naming `github.ibm.com` and referencing re-authentication. No socket is opened.
- **Business Rules:** The clone path currently uses `get_token` (`crates/mcp-fs/src/tools/git.rs:704`), which ignores expiry (`store.rs:148-150`), unlike `has_valid_token` (`store.rs:169`).
- **Priority:** Must-have

#### FR-NEW-019 [EARS-UB]: An expired token is not deleted
> The mcp-fs server SHALL NOT delete a stored token as a consequence of detecting that it has expired.

- **Inputs:** A remote operation that fails per FR-NEW-018.
- **Outputs:** `git.auth_status` still lists the host, reporting `expired`.
- **Business Rules:** Deletion on a read path is a surprising side effect and loses the distinction between expired and absent.
- **Priority:** Must-have

#### FR-NEW-070 [EARS-U]: The credential-absent and credential-expired failures carry fixed codes and prefixes
> The mcp-fs server SHALL report a remote operation against a declared non-anonymous host for which the person holds no token as `ERR_UNAUTHENTICATED` with the message prefix `no token for host`, and a remote operation whose stored token has expired as `ERR_UNAUTHENTICATED` with the message prefix `token expired for host`, each followed by the hostname and an instruction to authenticate with `git.auth` or `git.token_set`.

- **Inputs:** Clone, push, fetch and pull against `github.ibm.com` with no stored token; then the same four with a token expired one hour ago.
- **Outputs:** `ERR_UNAUTHENTICATED: no token for host github.ibm.com ...` for the first four; `ERR_UNAUTHENTICATED: token expired for host github.ibm.com ...` for the second four. No socket is opened in any of the eight.
- **Business Rules:** The two prefixes are distinct so a client can tell absence from expiry, which is the distinction FR-NEW-019 exists to preserve. No new `ERR_*` constant is introduced; `ERR_UNAUTHENTICATED` already exists at `crates/mcp-fs/src/errors.rs:9`.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F3 of round 3. Every other rejection in this specification pins its code; these two named only the message content.

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

### The 10 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-054** — A credential rejected by the remote does not delete the token
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-019 · **Priority:** High
- Given a stored, unexpired token the remote rejects
- When a clone is attempted
- Then the call fails with the transport error, naming the host
- And the token is still present in the store afterwards

**E2E-NEW-095** — Clone with an expired token fails before the network
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given a token for `(alice@test.com, github.ibm.com)` expiring one hour ago
- When `git.remote_clone` runs
- Then the call fails naming `github.ibm.com` and instructing re-authentication
- And zero outbound connections were attempted
- *Today the clone path uses `get_token` (`crates/mcp-fs/src/tools/git.rs:704`), which ignores expiry (`crates/mcp-fs/src/git/oauth/store.rs:148-150`); this test fails before the change.*

**E2E-NEW-096** — Push, fetch and pull all enforce expiry
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given an expired token for the origin's host
- When each of push, fetch and pull runs
- Then all three fail with the same expiry error and make no network call

**E2E-NEW-097** — An expired token survives the failure
- **Category:** Security · **Scenario:** SC-014 · **Requirements:** FR-NEW-019 · **Priority:** Critical
- When an operation fails per E2E-NEW-095
- Then the token is still stored, and `git.auth_status` reports `github.ibm.com` as `expired`

**E2E-NEW-098** — The expiry error differs from the missing-token error
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** High
- When one operation runs with an expired token and another with none
- Then the two failures are machine-distinguishable, so a caller can decide whether to re-seed or to authenticate

**E2E-NEW-099** — A token expiring exactly now is treated as expired
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Medium
- Given `expires_at` equal to the current instant
- When an operation runs
- Then the boundary is handled per `OAuthSession::is_valid_at` (`crates/mcp-fs/src/git/oauth/store.rs:34`) and the outcome is asserted explicitly rather than left to chance

**E2E-NEW-119** — Pull with an expired token fails before fetching
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-018 · **Priority:** Critical
- Given an expired token for the origin's host
- When a pull runs
- Then it fails with the expiry error, no fetch occurs, and no remote-tracking ref changes

**E2E-NEW-241** — A missing token yields the fixed code and prefix on all four tools
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- Given `github.ibm.com` declared `github` with no stored token
- When clone, push, fetch and pull each run
- Then all four fail with `ERR_UNAUTHENTICATED` and a message beginning `no token for host github.ibm.com`
- And no socket is opened in any of the four

**E2E-NEW-242** — An expired token yields its own fixed prefix
- **Category:** Error · **Scenario:** SC-014 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- Given a token for that host expired one hour ago
- When the same four operations run
- Then all four fail with `ERR_UNAUTHENTICATED` and a message beginning `token expired for host github.ibm.com`

**E2E-NEW-243** — Absence and expiry are machine-distinguishable
- **Category:** Edge · **Scenario:** SC-014 · **Requirements:** FR-NEW-070 · **Priority:** Critical
- When one operation runs with no token and another with an expired one
- Then both carry `ERR_UNAUTHENTICATED` and their message prefixes differ, so a client can choose between seeding and authenticating

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never delete a token because it was found expired (DEC-020, FR-NEW-019). Deletion on a read path is a surprising side effect and loses the expired-versus-absent distinction.

### Scope Boundary
The expiry gate and its two message contracts. Do NOT delete expired tokens, and do NOT implement status reporting of `expired` (US-006).

## Non Regression

### Existing Tests That Must Pass
- A token stored with no expiry never fails this test and the operation proceeds (EXC-014a, FR-NEW-014).
- `has_valid_token` (`crates/mcp-fs/src/git/oauth/store.rs:169`) keeps its existing behaviour; this story puts an equivalent check on the path that lacked it.
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
