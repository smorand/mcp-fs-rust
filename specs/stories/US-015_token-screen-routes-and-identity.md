# US-015: The token screen: routes, and identity from three sources

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 15
> Depends On: US-004, US-007
> Complexity: L
> min_tier: 1
> Files touched: 3

## Objective

A person reaches a token screen in a browser. Because a browser cannot attach an `Authorization` header to a top-level navigation, the three screen routes additionally accept a read-only `mcpfs_token` cookie, verified exactly as a bearer token is. Nothing outside these routes gains cookie acceptance.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/app.rs  # router assembly; :5-7 records that identity is resolved inline per handler; 401 body shape at :10-11
crates/mcp-fs/src/identity.rs  # IdentityResolver; header-only resolution at :188-193
crates/mcp-fs/src/api/dataplane.rs  # :177-178 is the inline-resolution pattern to copy
```

### Existing Patterns
- **There is no identity middleware to place routes behind** (DRIFT-008). `crates/mcp-fs/src/app.rs:5-7` states identity verification happens inline at the top of the handler, and the REST plane repeats the resolution per handler (`crates/mcp-fs/src/api/dataplane.rs:177-178`). Copy that inline pattern; do not introduce a tower layer for this feature alone.
- The JWT in the cookie is verified exactly as a header bearer token is, by `IdentityResolver` (`crates/mcp-fs/src/identity.rs`), with the same RS256 verification and 30s clock skew.
- The `doc.open_editor` precedent (`crates/mcp-fs/src/tools/editor.rs:208`, `:293`) is explicitly **not** followed: it binds loopback with no token in the URL (DEC-028 supersedes DEC-007).

### Data Model (excerpt)
- No new persisted entity. A `TokenScreenSession` exists only as the in-memory anti-forgery state added in US-017.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-028:** The token screen is served from the main server behind the RS256 identity layer, superseding DEC-007. **Rationale:** the `doc.open_editor` precedent binds loopback with no token in the URL, so it is unauthenticated against other local processes and unreachable when the server is remote. **Alternatives considered:** mirroring the editor exactly; hardening the spawned session with a URL token and TTL. **Implemented by:** FR-NEW-037, FR-NEW-040. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/editor.rs:208`, `:216`, `:293`
- **DEC-038:** The token screen accepts a read-only `mcpfs_token` cookie in addition to the two bearer headers, with cross-origin submissions refused by a single-use `csrf_token`. **Rationale:** a browser cannot attach an `Authorization` header to a top-level navigation, so a header-only screen is unreachable by the only client that can use it. **Alternatives considered:** requiring an authenticating gateway to inject the forwarded header, which adds no credential surface but makes the screen unusable without a gateway; a capability URL, rejected because it puts a credential in a URL, which FR-NEW-042 forbids elsewhere in this same specification. **Implemented by:** FR-NEW-047, FR-NEW-048, FR-NEW-058, FR-NEW-059. **Round:** 6 (audit round 1, finding F-002). **Code evidence:** `crates/mcp-fs/src/identity.rs:1-8`, `:188-193`

### Applicable NFRs
- **Token screen authenticated** (§7.2, FR-NEW-037): the existing RS256 identity layer, not a loopback session.
- **Routes are registered only when `git.enabled` is true** (FR-NEW-046).

### Bounded Context
**Token Self-Service** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Implementation Drift To Close In This Story

> Carried verbatim from the specification's Section 19. These are resolved **during** this story, not later. An entry still open when the branch merges moves to `specs/BACKLOG.md`, and that move is a decision someone signs, not a silence.

#### DRIFT-008: There is no identity middleware to place routes behind
- **Spec says:** FR-NEW-037, "authenticated by the existing RS256 identity layer"; §7.2, "The existing RS256 identity layer, not a loopback session".
- **Code does:** No identity middleware exists. `crates/mcp-fs/src/app.rs:5-7` states that identity verification "happens inline at the top of that handler, which is the only guarded route", and the REST plane repeats the resolution per handler (`crates/mcp-fs/src/api/dataplane.rs:177-178`).
- **Nature:** bypassed abstraction
- **Resolution during implementation:** Resolve identity inline at the top of each token screen handler using `state.identity.resolve`, exactly as `dataplane.rs:177-178` does, returning the 401 body shape documented at `app.rs:10-11`. Do not introduce a tower layer for this feature alone. FR-NEW-047's three-source resolution is implemented in that inline call.
- **Detected by:** E2E-NEW-172 and E2E-NEW-173 fail if a handler omits the inline check, since no layer would have performed it.
- **Blocks which requirement:** FR-NEW-037, FR-NEW-047, FR-NEW-048.
- **Status:** resolved (E2E-NEW-172, E2E-NEW-173, E2E-NEW-171, this commit)

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-037 [EARS-U]: The token screen is served by the main server behind identity
> The mcp-fs server SHALL serve the per-person token screen as a route on the main HTTP server, authenticated by the existing RS256 identity layer.

- **Inputs:** A browser request carrying a valid RS256 JWT.
- **Outputs:** The screen, scoped to the authenticated person.
- **Business Rules:** Not a tool-spawned loopback session. The `doc.open_editor` precedent binds `127.0.0.1` with no token in the URL (`crates/mcp-fs/src/tools/editor.rs:208`, `:293`), which is unauthenticated against other local processes and unreachable when the server is remote.
- **Priority:** Must-have
- **Rationale:** DEC-028, superseding DEC-007.

#### FR-NEW-046 [EARS-U]: The token screen route names are fixed
> The mcp-fs server SHALL serve the token screen at `GET /app/tokens`, SHALL accept seeding at `POST /app/tokens` with the `application/x-www-form-urlencoded` fields `host` and `token`, and SHALL accept revocation at `POST /app/tokens/revoke` with the field `host`.

- **Inputs:** `GET /app/tokens`; `POST /app/tokens` body `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`; `POST /app/tokens/revoke` body `host=github.ibm.com`.
- **Outputs:** `200 text/html` for the `GET`; `303 See Other` with `Location: /app/tokens` on a successful `POST`; `400 application/json {"error":"ERR_INVALID_ARGUMENT","detail":"..."}` on a rejected host or token; `401` when unauthenticated.
- **Business Rules:** The three routes are registered only when `git.enabled` is true. Both `POST` routes delegate to the same functions as `git.token_set` and `git.auth_revoke` (FR-NEW-039), never to a second implementation.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-001 and resolves what Section 15 previously left as a TBD. A public HTTP surface an agent would otherwise have to invent.

#### FR-NEW-047 [EARS-E]: The token screen resolves identity from three sources
> WHEN a request reaches a token screen route, THE mcp-fs server SHALL resolve the person from the configured forwarded header, then the `Authorization` header, then the `mcpfs_token` cookie, and SHALL reject the request with HTTP 401 when none yields a verified identity.

- **Inputs:** A request carrying `X-Forwarded-Authorization: Bearer <jwt>`; one carrying `Authorization: Bearer <jwt>`; one carrying `Cookie: mcpfs_token=<jwt>`; one carrying none.
- **Outputs:** The screen for the first three; `401 {"error":"ERR_UNAUTHENTICATED","detail":"..."}` for the fourth.
- **Business Rules:** The cookie is read only: the server never sets, refreshes or clears it. The JWT in the cookie is verified exactly as a header bearer token is, by `IdentityResolver`. No route outside `/app/tokens*` gains cookie acceptance.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-002. A browser cannot attach an `Authorization` header to a top-level navigation, so the header-only resolution at `crates/mcp-fs/src/identity.rs:188-193` makes the screen unreachable by the only client that can use it.

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

**E2E-NEW-105** — An unauthenticated request is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-037 · **Priority:** Critical
- When the route is requested with no `Authorization` header
- Then the identity layer rejects it and no token metadata is returned

**E2E-NEW-106** — An invalid or expired JWT is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-037 · **Priority:** Critical
- When the route is requested with a JWT signed by the wrong key, then with one expired beyond the 30-second skew
- Then both are rejected, consistent with `crates/mcp-fs/src/identity.rs`

**E2E-NEW-164** — The screen route serves HTML to an authenticated person
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `GET /app/tokens` is requested with a verified identity
- Then the response is `200` with `Content-Type: text/html`

**E2E-NEW-165** — Seeding through the route redirects
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens` is submitted with `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH`
- Then the response is `303` with `Location: /app/tokens`
- And `git.auth_status` subsequently lists `github.ibm.com`

**E2E-NEW-166** — Revocation through the route redirects
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens/revoke` is submitted with `host=github.ibm.com`
- Then the response is `303` and the token is gone

**E2E-NEW-167** — A rejected host returns 400 with the error body
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** Critical
- When `POST /app/tokens` is submitted with `host=git.unknown.test&token=ghp_x`
- Then the response is `400` with body `{"error":"ERR_INVALID_ARGUMENT","detail":"..."}` naming the host

**E2E-NEW-168** — The screen routes are absent when git is disabled
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-046 · **Priority:** High
- Given `git.enabled` is false
- When `GET /app/tokens` is requested
- Then the route is not registered and the response is `404`

**E2E-NEW-169** — Identity resolves from the forwarded header
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When `GET /app/tokens` carries `X-Forwarded-Authorization: Bearer <valid jwt for alice@test.com>`
- Then the screen renders scoped to `alice@test.com`

**E2E-NEW-170** — Identity resolves from the `Authorization` header
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the same request carries `Authorization: Bearer <valid jwt>` instead
- Then the screen renders identically

**E2E-NEW-171** — Identity resolves from the cookie
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the request carries only `Cookie: mcpfs_token=<valid jwt>` and no bearer header
- Then the screen renders scoped to that person
- *This is the only path a browser navigation can take.*

**E2E-NEW-172** — A request with no credential is rejected
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When `GET /app/tokens` carries neither header nor cookie
- Then the response is `401` with body `{"error":"ERR_UNAUTHENTICATED","detail":"..."}`

**E2E-NEW-173** — A cookie holding an invalid JWT is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When the cookie holds a JWT signed by the wrong key, then one expired beyond the 30-second skew
- Then both yield `401`, proving the cookie is verified exactly as a header bearer token is

**E2E-NEW-174** — No route outside the screen accepts the cookie
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** Critical
- When an `/api/fs` request and an MCP request each carry only `Cookie: mcpfs_token=<valid jwt>`
- Then both are rejected as unauthenticated

**E2E-NEW-175** — The server never sets the cookie
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-047 · **Priority:** High
- When every response from every route is inspected
- Then none carries a `Set-Cookie` header for `mcpfs_token`

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never set, refresh or clear the cookie: it is read only (FR-NEW-047, FR-NEW-059).
- Never put a credential in a URL. A capability URL was rejected precisely because FR-NEW-042 forbids it elsewhere in this same specification (DEC-038).

### Scope Boundary
Routes, identity resolution and the page shell. Listing content is US-016; seeding, revocation and CSRF are US-017.

## Non Regression

### Existing Tests That Must Pass
- No route outside `/app/tokens*` gains cookie acceptance (FR-NEW-047).
- The MCP endpoint and the REST plane keep resolving identity exactly as they do today; `crates/mcp-fs/src/identity.rs:188-193` behaviour is extended for these routes only.
- The token screen is an HTTP route and does NOT appear in the tool contract (FR-NEW-045).
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
5. **Drift closed.** DRIFT-008 is resolved in this branch, by the resolution quoted above, and you can name the test that proves it.
