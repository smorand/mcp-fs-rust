# US-004: git.token_set: seed a token the caller already holds

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 4
> Depends On: US-002, US-003
> Complexity: M
> min_tier: 1
> Files touched: 2

## Objective

A calling agent holding a person's personal access token hands it to the server directly, instead of forcing that person through an interactive flow for a credential they already possess. The tool validates the host against the map, bounds the token, parses the expiry, and never echoes the value back.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git_auth.rs  # tool registration at :60-131; store_token call site at :199-211
crates/mcp-fs/src/git/oauth/store.rs  # store_token :115-123, upserts memory then persistence :129-141
```

### Existing Patterns
- `OAuthTokenStore::store_token` (`crates/mcp-fs/src/git/oauth/store.rs:115`) is public today but reachable from exactly one place, the device-flow poller (`crates/mcp-fs/src/tools/git_auth.rs:199-211`). This story adds the second caller.
- Tool registration and schema building follow the existing `git.auth*` tools in `crates/mcp-fs/src/tools/git_auth.rs:60-131` and the declarative builder in `crates/mcp-fs/src/mcp/schema.rs`.
- `expires_at` is serialized by the existing `round_trip_iso` helper; the stored type is `DateTime<Utc>`.
- Exact, lowercase value matching mirrors `git.auth`'s provider check at `crates/mcp-fs/src/tools/git_auth.rs:160-162`.

### Data Model (excerpt)
- No new entity. Writes an `OAuthSession` keyed `(person, host)` per US-003.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-016:** The seeding tool is named `git.token_set`. **Implemented by:** FR-NEW-013. **Round:** 2b. **Code evidence:** n/a
- **DEC-017:** `git.token_set` takes an optional `expires_at`; absent means non-expiring. **Rationale:** a PAT may genuinely never expire, and a fabricated expiry would wrongly trigger the expiry failure path. **Implemented by:** FR-NEW-014. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/git/oauth/store.rs:115-123`
- **DEC-033:** Seeded tokens longer than 8192 characters are rejected. **Rationale:** a denial-of-service bound; the codebase already uses bounded `TextKey` columns because SQL Server cannot index unbounded text. **Implemented by:** FR-NEW-015. **Round:** 4. **Code evidence:** `crates/mcp-fs/src/git/oauth/persistence.rs:33-34`

### Applicable NFRs
- **Bounded token size** (§7.2): FR-NEW-015, 8192 characters, a denial-of-service bound on `git.token_set`.
- **No token on any observable surface** (§7.2): the response contains no substring of the token.
- **Encryption at rest** (§7.2): the value is encrypted under `MCPFS_TOKEN_KEY`, unchanged.

### Bounded Context
**Token Custody** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-013 [EARS-E]: The seeding tool stores a supplied token
> WHEN `git.token_set` is called with a declared `host` and a non-empty `token`, THE mcp-fs server SHALL store that token for `(person, host)` and SHALL return success without echoing the token.

- **Inputs:** `{"host": "github.ibm.com", "token": "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH", "expires_at": null}`.
- **Outputs:** `{"host": "github.ibm.com", "provider": "github", "stored": true, "persistent": true}`. The response contains no substring of the token.
- **Business Rules:** The provider is resolved from the map, never supplied by the caller. Calls `OAuthTokenStore::store_token` (`crates/mcp-fs/src/git/oauth/store.rs:115`).
- **Priority:** Must-have
- **Rationale:** `store_token` is public but reachable only from the device-flow poller (`crates/mcp-fs/src/tools/git_auth.rs:199-211`).

#### FR-NEW-014 [EARS-O]: An absent expiry means non-expiring
> IF `git.token_set` is called without `expires_at`, THEN THE mcp-fs server SHALL store the token as non-expiring.

- **Inputs:** `expires_at` omitted or null.
- **Outputs:** A stored session that never fails the expiry test of FR-NEW-018.
- **Business Rules:** A personal access token can genuinely have no expiry; fabricating one would wrongly trigger FR-NEW-018.
- **Priority:** Must-have

#### FR-NEW-015 [EARS-O]: The seeding tool rejects an unusable token
> IF `git.token_set` receives a token that is empty, whitespace-only, or longer than 8192 characters, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` and SHALL store nothing.

- **Inputs:** `""`, `"   "`, or a 8193-character string.
- **Outputs:** `ERR_INVALID_ARGUMENT`. `git.auth_status` reports no token for that host.
- **Business Rules:** 8192 is the bound; 8192 exactly is accepted, 8193 is rejected.
- **Priority:** Must-have

#### FR-NEW-016 [EARS-O]: The seeding tool rejects an undeclared or anonymous host
> IF `git.token_set` names a host absent from `git.hosts`, or a host declared `anonymous`, THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT`, naming the host.

- **Inputs:** `{"host": "git.unknown.test", "token": "ghp_x"}`; `{"host": "public.example.org", "token": "ghp_x"}` where that host is `anonymous`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming the host. Nothing stored.
- **Business Rules:** An anonymous host holds no credential by definition.
- **Priority:** Must-have

#### FR-NEW-017 [EARS-O]: Seeding fails loudly when configured persistence fails
> IF `git.token_set` is called while relational persistence is configured and the persistence write fails, THEN THE mcp-fs server SHALL fail the call and SHALL NOT report success.

- **Inputs:** A seeding call with the backing store unreachable.
- **Outputs:** `ERR_INTERNAL_ERROR` or the retryable database error; the response never claims the token was stored.
- **Business Rules:** `store_token` writes memory first, then persistence (`store.rs:129-141`). Reporting success for a token that vanishes on restart is a silent data-loss defect.
- **Priority:** Must-have

#### FR-NEW-052 [EARS-E]: `expires_at` is an RFC 3339 string on input
> WHEN `git.token_set` receives a non-null `expires_at`, THE mcp-fs server SHALL parse it as an RFC 3339 timestamp, SHALL convert it to UTC, and SHALL reject any other form with `ERR_INVALID_ARGUMENT` naming the parameter.

- **Inputs:** `"2027-01-01T00:00:00Z"`; `"2027-01-01T01:00:00+01:00"`; `1798761600`; `"tomorrow"`.
- **Outputs:** The first two store the same instant; the last two are rejected.
- **Business Rules:** An `expires_at` already in the past is accepted and stored; the token then reports `expired` per FR-MOD-003 rather than being rejected at seeding.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-006. The stored type is `DateTime<Utc>` and the codebase serializes with a deliberately non-standard seven-digit fractional form, so the accepted input form had three defensible readings.

#### FR-NEW-064 [EARS-O]: A failed persistence write leaves no in-memory token
> IF the persistence write of `git.token_set` or of the device-flow poller fails, THEN THE mcp-fs server SHALL remove the in-memory entry for that `(person, host)` before returning the error, restoring the entry that was present before the call when one existed.

- **Inputs:** A seed for `(alice@test.com, github.ibm.com)` with the backing store unreachable, on an empty store; then the same with a prior valid token present.
- **Outputs:** First case: `git.auth_status` lists no entry for that host and a clone fails per EXC-004a. Second case: the prior token is still present and still clones.
- **Business Rules:** `store_token` writes memory first, then persistence (`crates/mcp-fs/src/git/oauth/store.rs:129-141`). Reporting a failure while leaving a live credential in memory makes observable state depend on process lifetime.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A7, and completes FR-NEW-017, which required the call to fail but not what it left behind.

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

### The 20 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-020** — Seeding stores a token for a declared host
- **Category:** Core Journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** Critical
- Given `alice@test.com` authenticated and `github.ibm.com: github`
- When `git.token_set` is called with `{"host": "github.ibm.com", "token": "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"}`
- Then the response is `{"host": "github.ibm.com", "provider": "github", "stored": true, "persistent": true}`
- And `git.auth_status` lists `github.ibm.com` with provider `github` and validity `valid`

**E2E-NEW-021** — Seeding derives the provider from the map, not the caller
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** High
- Given `git.acme.internal: generic`
- When `git.token_set` is called for that host
- Then the stored session's provider is `generic`
- And the tool schema exposes no `provider` parameter

**E2E-NEW-022** — A seeded token authenticates a clone end to end
- **Category:** Core Journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-006 · **Priority:** Critical
- Given a token seeded for `(alice@test.com, github.ibm.com)`
- When `git.remote_clone` is called with `https://github.ibm.com/org/repo.git`
- Then the credential supplied to the remote is `oauth2:ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- And the clone succeeds with `"auth": "github"`

**E2E-NEW-023** — The response never echoes the token
- **Category:** Security · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-043 · **Priority:** Critical
- When `git.token_set` succeeds with token `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- Then the serialized response contains no substring of length 8 or more from the token

**E2E-NEW-024** — An empty token is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** Critical
- When `git.token_set` is called with `{"host": "github.ibm.com", "token": ""}`
- Then the call fails with `ERR_INVALID_ARGUMENT`
- And `git.auth_status` reports no token for `github.ibm.com`

**E2E-NEW-025** — A whitespace-only token is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** High
- When the token is `"   \t\n "`
- Then the call fails with `ERR_INVALID_ARGUMENT` and stores nothing

**E2E-NEW-026** — A token over the bound is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** High
- When the token is 8193 `a` characters
- Then the call fails with `ERR_INVALID_ARGUMENT` and stores nothing

**E2E-NEW-027** — An undeclared host is rejected for seeding
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-016 · **Priority:** Critical
- When `git.token_set` names `git.unknown.test`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming that host, and stores nothing

**E2E-NEW-028** — An anonymous host is rejected for seeding
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-016 · **Priority:** Critical
- When `git.token_set` names `public.example.org`, declared `anonymous`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming that host
- And the message states that an anonymous host holds no credential

**E2E-NEW-029** — An unauthenticated caller is rejected
- **Category:** Security · **Scenario:** SC-002 · **Requirements:** FR-NEW-013 · **Priority:** Critical
- Given a context whose `person` is empty
- When `git.token_set` is called
- Then the call fails with `ERR_UNAUTHENTICATED`, mirroring `crates/mcp-fs/src/tools/git_auth.rs:143-148`

**E2E-NEW-030** — A failing persistence write fails the call
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-017 · **Priority:** Critical
- Given persistence configured and its backing store made unavailable
- When `git.token_set` is called
- Then the call fails and the response never reports `stored: true`
- And the caller can distinguish this from a validation failure

**E2E-NEW-031** — Seeding twice overwrites, leaving one token
- **Category:** State Transition · **Scenario:** SC-002 · **Requirements:** FR-NEW-013, FR-NEW-009 · **Priority:** High
- When `git.token_set` is called for `github.ibm.com` with `ghp_FIRST…` then `ghp_SECOND…`
- Then exactly one token is stored for `(alice@test.com, github.ibm.com)`
- And a subsequent clone supplies `ghp_SECOND…`

**E2E-NEW-034** — A token at exactly the bound is accepted
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-015 · **Priority:** Medium
- When the token is exactly 8192 characters
- Then the call succeeds and the token is stored intact

**E2E-NEW-035** — Memory-only operation is reported explicitly
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-014, FR-NEW-017 · **Priority:** High
- Given `MCPFS_TOKEN_KEY` is unset, so persistence is disabled (`FR-628`)
- When `git.token_set` succeeds
- Then the response reports `persistent: false`
- And the response states that the token lives only for the process lifetime

**E2E-NEW-191** — An RFC 3339 `expires_at` is accepted in any offset
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** Critical
- When `git.token_set` is called with `"2027-01-01T00:00:00Z"` and then `"2027-01-01T01:00:00+01:00"`
- Then both store the same instant

**E2E-NEW-192** — A non-RFC-3339 `expires_at` is rejected
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** Critical
- When `expires_at` is `1798761600` or `"tomorrow"`
- Then each is rejected with `ERR_INVALID_ARGUMENT` naming the parameter, and nothing is stored

**E2E-NEW-193** — A past `expires_at` is accepted and reports expired
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-052 · **Priority:** High
- When a token is seeded with `expires_at` one hour in the past
- Then seeding succeeds
- And `git.auth_status` reports that host `expired`, and a clone fails per FR-NEW-018

**E2E-NEW-221** — A failed persistence write leaves no in-memory token
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-064 · **Priority:** Critical
- Given an empty store and persistence made unavailable
- When `git.token_set` is called for `github.ibm.com` and fails
- Then `git.auth_status` lists no entry for that host
- And a clone of that host fails per EXC-004a rather than succeeding

**E2E-NEW-222** — A failed write restores the previous token
- **Category:** Error · **Scenario:** SC-002 · **Requirements:** FR-NEW-064 · **Priority:** Critical
- Given a valid token already stored for `github.ibm.com`
- When a re-seed fails at the persistence write
- Then the previous token is still present and still authenticates a clone

**E2E-NEW-223** — The device-flow poller rolls back identically
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-064 · **Priority:** High
- When the poller authorises successfully but its persistence write fails
- Then no in-memory token remains for that `(person, host)`

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never accept a provider from the caller. It is resolved from the map (FR-NEW-013).
- Never fabricate an expiry when `expires_at` is absent. Absent means non-expiring (DEC-017, FR-NEW-014).
- Never report success for a token that will not survive restart (FR-NEW-017), and never leave a live credential in memory after a failed persistence write (FR-NEW-064).

### Scope Boundary
The seeding tool and its input validation. Do NOT enforce expiry on remote operations (US-005) and do NOT build the web screen (US-015 to US-017).

## Non Regression

### Existing Tests That Must Pass
- The device-flow poller keeps working and keeps storing through the same `store_token` call site (`crates/mcp-fs/src/tools/git_auth.rs:199-211`), now keyed by host.
- `store_token`'s upsert semantics are unchanged: seeding twice for one `(person, host)` overwrites (EXC-002d).
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
