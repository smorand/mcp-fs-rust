# US-016: The screen lists held hosts, ordered, and never another person's

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 16
> Depends On: US-015, US-006
> Complexity: M
> min_tier: 1
> Files touched: 2

## Objective

The screen shows a person which hosts they hold credentials for and whether each is still valid, ordered by host so the page is stable. It never renders a token value, never sends one to the browser, and never exposes another person's tokens, including to a platform admin.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/tokens_screen.rs  # the GET handler's body
crates/mcp-fs/src/git/oauth/store.rs  # the per-person enumeration added in US-006
```

### Existing Patterns
- The screen reads the same per-person enumeration `git.auth_status` reads (DRIFT-010, US-006). It is a second adapter over one operation, never a second implementation.
- Ordering is by host ascending, identical to the ordering FR-NEW-051 fixes for `git.auth_status`.

### Data Model (excerpt)
- No new entity. Renders `OAuthSession` metadata: host, provider, validity.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-008:** The screen reads and writes but never displays a token value; it lists hosts, seeds and revokes. **Implemented by:** FR-NEW-038, FR-NEW-039. **Round:** 1. **Code evidence:** n/a
- **DEC-028:** The token screen is served from the main server behind the RS256 identity layer, superseding DEC-007. **Rationale:** the `doc.open_editor` precedent binds loopback with no token in the URL, so it is unauthenticated against other local processes and unreachable when the server is remote. **Alternatives considered:** mirroring the editor exactly; hardening the spawned session with a URL token and TTL. **Implemented by:** FR-NEW-037, FR-NEW-040. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/tools/editor.rs:208`, `:216`, `:293`

### Applicable NFRs
- **Strict per-person isolation** (§7.2, FR-NEW-040): platform admin gains no implicit access. This is the rule the whole project holds — platform admin manages projects and membership, not files or credentials.
- **No token on any observable surface** (§7.2): the HTTP response body contains no substring of any token.

### Bounded Context
**Token Self-Service** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-038 [EARS-E]: The screen lists held hosts without values
> WHEN a person opens the token screen, THE mcp-fs server SHALL list every host for which that person holds a token, with its provider and a validity of `valid` or `expired`, and SHALL NOT transmit any token value to the browser.

- **Inputs:** A person holding tokens for `github.com` (valid) and `github.ibm.com` (expired).
- **Outputs:** Two rows. The HTTP response body contains no substring of either token.
- **Priority:** Must-have

#### FR-NEW-040 [EARS-UB]: The screen never exposes another person's tokens
> The mcp-fs server SHALL NOT allow any person, including a platform admin, to view, seed or revoke a token belonging to a different person through the token screen.

- **Inputs:** `bob@test.com`, holding the platform admin claim, requesting `alice@test.com`'s tokens.
- **Outputs:** `ERR_FORBIDDEN`; no data about alice's tokens in the response.
- **Business Rules:** Platform admin manages projects and membership and gains no implicit access.
- **Priority:** Must-have

#### FR-NEW-067 [EARS-U]: The token screen orders rows by host
> The mcp-fs server SHALL render the token screen rows ordered by host ascending, identically to the ordering FR-NEW-051 fixes for `git.auth_status`.

- **Inputs:** A person holding tokens for `github.ibm.com` and `github.com`.
- **Outputs:** `github.com` renders before `github.ibm.com`.
- **Business Rules:** The screen reads the same per-person enumeration the tool reads (DRIFT-010).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A10. Row order is what the person sees, and the screen is declared a second adapter over the same operation.

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

### The 9 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-100** — The screen lists held hosts
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** Critical
- Given `alice@test.com` holds a valid token for `github.com` and an expired one for `github.ibm.com`
- When she requests the token screen route with a valid RS256 JWT
- Then the response lists two rows: `github.com / github / valid` and `github.ibm.com / github / expired`

**E2E-NEW-103** — The screen offers only declared hosts
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** High
- When the screen renders its host selector under the reference map
- Then it offers `github.com`, `github.ibm.com`, `gitlab.acme.corp` and `git.acme.internal`
- And it does not offer `public.example.org`, which is `anonymous`

**E2E-NEW-104** — No token value reaches the browser
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-038, FR-NEW-043 · **Priority:** Critical
- Given alice holds `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` for `github.com`
- When the screen is rendered and every asset it references is fetched
- Then no response body contains any substring of length 8 or more from that token

**E2E-NEW-107** — One person cannot read another's tokens
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-040 · **Priority:** Critical
- Given alice holds a token for `github.ibm.com`
- When bob requests the screen, including any parameter naming alice
- Then the response contains nothing about alice's tokens and the request is refused

**E2E-NEW-108** — A platform admin cannot read another person's tokens
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-040 · **Priority:** Critical
- Given bob holds the platform admin claim and alice holds a token
- When bob attempts to view or revoke alice's token through the screen
- Then the attempt fails with `ERR_FORBIDDEN`
- *Platform admin manages projects and membership and gains no implicit access.*

**E2E-NEW-109** — Seeding an undeclared host through the screen is rejected
- **Category:** Security · **Scenario:** SC-006 · **Requirements:** FR-NEW-038 · **Priority:** Critical
- When a request is submitted directly for `git.unknown.test`, bypassing the rendered selector
- Then the server rejects it naming the host, proving validation is server-side

**E2E-NEW-230** — The screen orders rows by host
- **Category:** Feature · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given tokens for `github.ibm.com` and `github.com`
- When the screen renders
- Then `github.com` appears before `github.ibm.com`

**E2E-NEW-231** — The screen and the tool agree on order
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given tokens for four hosts
- When the screen and `git.auth_status` are compared
- Then the host sequences are identical

**E2E-NEW-232** — An empty token list renders without error
- **Category:** Error · **Scenario:** SC-006 · **Requirements:** FR-NEW-067 · **Priority:** High
- Given a person holding no tokens
- When the screen renders
- Then it returns `200` with an empty list and no ordering failure

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never transmit a token value to the browser, not even masked (FR-NEW-038).
- Never build the list from an unfiltered store enumeration (DRIFT-010).

### Scope Boundary
The listing, its ordering and its isolation. Mutations are US-017.

## Non Regression

### Existing Tests That Must Pass
- `git.auth_status` and the screen agree, because they read the same enumeration (FR-NEW-039).
- A host with no stored token is never listed, on the screen as in the tool (FR-NEW-051).
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
