# US-017: The screen seeds and revokes, protected by a single-use csrf_token

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 17
> Depends On: US-015, US-016, US-004, US-007
> Complexity: L
> min_tier: 1
> Files touched: 2

## Objective

A person seeds or revokes a credential from the browser, and the two POST routes delegate to the same functions as `git.token_set` and `git.auth_revoke` rather than reimplementing their rules. Because the cookie is ambient, a cookie-authenticated POST without a matching single-use anti-forgery token is refused.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/tokens_screen.rs  # POST /app/tokens and POST /app/tokens/revoke
crates/mcp-fs/src/tools/git_auth.rs  # the token_set and auth_revoke functions being reused
```

### Existing Patterns
- The screen is a second adapter over the same operations, never a second implementation (FR-NEW-039). The two POST routes call the functions US-004 and US-007 built.
- **There is no existing CSRF machinery in the tree to reuse** (FR-NEW-058). The value is a UUIDv4 string held in server memory only, bound to the authenticated person, single use, consumed on first successful match.
- A request authenticated by a header rather than the cookie carries no ambient credential and is exempt (FR-NEW-048).

### Data Model (excerpt)
- In-memory anti-forgery tokens, bound to a person, single use. Never persisted.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-008:** The screen reads and writes but never displays a token value; it lists hosts, seeds and revokes. **Implemented by:** FR-NEW-038, FR-NEW-039. **Round:** 1. **Code evidence:** n/a
- **DEC-038:** The token screen accepts a read-only `mcpfs_token` cookie in addition to the two bearer headers, with cross-origin submissions refused by a single-use `csrf_token`. **Rationale:** a browser cannot attach an `Authorization` header to a top-level navigation, so a header-only screen is unreachable by the only client that can use it. **Alternatives considered:** requiring an authenticating gateway to inject the forwarded header, which adds no credential surface but makes the screen unusable without a gateway; a capability URL, rejected because it puts a credential in a URL, which FR-NEW-042 forbids elsewhere in this same specification. **Implemented by:** FR-NEW-047, FR-NEW-048, FR-NEW-058, FR-NEW-059. **Round:** 6 (audit round 1, finding F-002). **Code evidence:** `crates/mcp-fs/src/identity.rs:1-8`, `:188-193`

### Applicable NFRs
- **Cross-origin submissions are refused** (FR-NEW-048): a cookie is attached by the browser to any request to this origin, including one triggered by another site.
- **The defence rests on `csrf_token` alone** (FR-NEW-059): the server never emits the cookie, so `SameSite` is configured by whatever fronts the deployment, not here.

### Bounded Context
**Token Self-Service** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-039 [EARS-E]: The screen seeds and revokes
> WHEN a person submits a token for a declared host, or requests revocation of a held host, THE mcp-fs server SHALL apply the change under the same rules as `git.token_set` and `git.auth_revoke`.

- **Inputs:** A seed for `github.ibm.com`; a revoke of `github.com`.
- **Outputs:** The store reflects both; `git.auth_status` agrees with the screen.
- **Business Rules:** The screen is a second adapter over the same operations, never a second implementation.
- **Priority:** Must-have

#### FR-NEW-048 [EARS-O]: The token screen rejects cross-origin submissions
> IF a `POST` to a token screen route is authenticated by the `mcpfs_token` cookie and does not carry a matching anti-forgery token, THEN THE mcp-fs server SHALL reject it with HTTP 403 and SHALL NOT modify any stored credential.

- **Inputs:** A `POST /app/tokens` from another origin carrying the cookie and no anti-forgery field; a `POST` from the rendered screen carrying the field issued with that page.
- **Outputs:** `403` and no state change for the first; success for the second.
- **Business Rules:** `GET /app/tokens` issues a single-use anti-forgery token bound to the authenticated person, which both `POST` routes require. The server never emits the cookie (FR-NEW-059), so this defence rests on the anti-forgery token alone, not on cookie attributes. A request authenticated by a header rather than the cookie carries no ambient credential and is exempt.
- **Priority:** Must-have
- **Rationale:** A cookie is attached by the browser to any request to this origin, including one triggered by another site. Without this, FR-NEW-047 turns token seeding and revocation into cross-site-forgeable operations.

#### FR-NEW-058 [EARS-U]: The anti-forgery field and its rendering are named
> The mcp-fs server SHALL name the anti-forgery field `csrf_token`, SHALL render it in the `GET /app/tokens` page as `<input type="hidden" name="csrf_token" value="...">`, and SHALL require it as an `application/x-www-form-urlencoded` field of the same name on `POST /app/tokens` and `POST /app/tokens/revoke`.

- **Inputs:** `POST /app/tokens` body `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH&csrf_token=<value issued by the GET>`.
- **Outputs:** `303 See Other` with `Location: /app/tokens` when the value matches an unconsumed token issued to the same person; `403 application/json {"error":"ERR_FORBIDDEN","detail":"..."}` when it is missing, unknown, already consumed, or issued to a different person.
- **Business Rules:** The value is a UUIDv4 string, held in server memory only, bound to the authenticated person, single use, consumed on first successful match. A request authenticated by a header rather than the `mcpfs_token` cookie is exempt (FR-NEW-048). There is no existing CSRF machinery in the tree to reuse.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A1. The cookie, the routes and the two form fields were all named; this third field was not, so an agent would have had to invent it.

#### FR-NEW-059 [EARS-UB]: The server never emits the session cookie
> The mcp-fs server SHALL NOT emit a `Set-Cookie` header for `mcpfs_token` on any route.

- **Inputs:** `GET /app/tokens`, `POST /app/tokens` and `POST /app/tokens/revoke`, each authenticated by cookie and by header.
- **Outputs:** No response carries `Set-Cookie`.
- **Business Rules:** The cookie is issued by whatever fronts the deployment, which is also where `SameSite=Strict` is configured. FR-NEW-048's cross-origin defence therefore rests on `csrf_token` alone.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A2. FR-NEW-047 said the server never sets the cookie while FR-NEW-048 said it sets `SameSite=Strict`; the two could not both be satisfied, and `Set-Cookie` is directly observable.

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

**E2E-NEW-101** — The screen seeds a token
- **Category:** Core Journey · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** Critical
- When alice submits a token for `gitlab.acme.corp` through the screen
- Then `git.auth_status` subsequently reports that host as `valid`
- And a clone from that host uses the submitted token

**E2E-NEW-102** — The screen revokes a token
- **Category:** Core Journey · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** Critical
- Given alice holds tokens for `github.com` and `github.ibm.com`
- When she revokes `github.ibm.com` through the screen
- Then only that token is gone and `github.com` remains

**E2E-NEW-110** — An empty token submitted through the screen is rejected
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When an empty value is submitted for `github.com`
- Then it is rejected under the same rule as `git.token_set`, and nothing is stored

**E2E-NEW-112** — The screen and the tools agree
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When a token is seeded through the screen and then through `git.token_set` for another host
- Then `git.auth_status` reports both identically, proving one implementation behind two adapters

**E2E-NEW-113** — Injection in a submitted value is neutralised
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-039 · **Priority:** High
- When a token containing `<script>alert(1)</script>` and one containing `'; DROP TABLE oauth_tokens; --` are submitted
- Then neither is executed nor reflected unescaped, the table still exists, and the values round-trip intact if stored

**E2E-NEW-176** — A cookie-authenticated POST without the anti-forgery token is refused
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- When `POST /app/tokens` carries the cookie and no anti-forgery field
- Then the response is `403` and no credential is stored

**E2E-NEW-177** — A POST carrying the issued anti-forgery token succeeds
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- Given `GET /app/tokens` issued an anti-forgery token bound to `alice@test.com`
- When `POST /app/tokens` carries the cookie and that token
- Then the seeding succeeds

**E2E-NEW-178** — Another person's anti-forgery token is refused
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** Critical
- When alice submits with an anti-forgery token issued to `bob@test.com`
- Then the response is `403` and nothing is stored

**E2E-NEW-179** — An anti-forgery token is single use
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** High
- When the same anti-forgery token is replayed on a second `POST`
- Then the second is refused with `403`

**E2E-NEW-180** — A header-authenticated POST is exempt
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-048 · **Priority:** High
- When `POST /app/tokens` is authenticated by `Authorization` rather than the cookie, with no anti-forgery field
- Then it succeeds, because no ambient credential was used

**E2E-NEW-203** — The anti-forgery field is rendered with its exact name
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When `GET /app/tokens` is rendered for an authenticated person
- Then the HTML contains `<input type="hidden" name="csrf_token" value="<uuid>">`

**E2E-NEW-204** — A POST carrying `csrf_token` succeeds
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When `POST /app/tokens` submits `host=github.ibm.com&token=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH&csrf_token=<issued>`
- Then the response is `303` and the token is stored

**E2E-NEW-205** — A missing or consumed `csrf_token` returns the exact error body
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-058 · **Priority:** Critical
- When the field is omitted, then replayed after being consumed
- Then both return `403` with body `{"error":"ERR_FORBIDDEN","detail":"..."}` and nothing is stored

**E2E-NEW-206** — No response ever sets the cookie
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-059 · **Priority:** Critical
- When every route of the server is exercised, authenticated by cookie and by header
- Then no response carries a `Set-Cookie` header naming `mcpfs_token`

**E2E-NEW-207** — Cross-origin defence does not depend on cookie attributes
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-059, FR-NEW-058 · **Priority:** Critical
- Given the cookie arrives without any `SameSite` attribute, as an upstream component set it
- When a cross-origin `POST` carries the cookie and no `csrf_token`
- Then it is refused with `403`

**E2E-NEW-208** — The screen works behind a component that sets the cookie
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-059 · **Priority:** High
- Given an upstream component issued `mcpfs_token` and the server never re-issues it
- When the person navigates to `GET /app/tokens`
- Then the screen renders, proving the server needs no cookie-issuing capability

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never emit a `Set-Cookie` header for `mcpfs_token` on any route (FR-NEW-059).
- Never accept a `csrf_token` twice, and never accept one issued to a different person (FR-NEW-058).

### Scope Boundary
The two POST routes and the anti-forgery mechanism. Do NOT re-validate hosts or tokens here; delegate.

## Non Regression

### Existing Tests That Must Pass
- Seeding and revoking through the screen obey exactly the rules of `git.token_set` and `git.auth_revoke`, including the undeclared-host and anonymous-host rejections and the 8192-character bound.
- `git.auth_status` agrees with the screen after any mutation (FR-NEW-039).
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
