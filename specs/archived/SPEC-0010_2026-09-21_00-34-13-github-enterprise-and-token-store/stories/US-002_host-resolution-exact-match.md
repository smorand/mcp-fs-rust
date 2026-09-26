# US-002: Resolve a remote URL to a provider by parsing and exact match

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 2
> Depends On: US-001
> Complexity: M
> min_tier: 1
> Files touched: 2

## Objective

A clone of `https://github.ibm.com/org/repo.git` finds the person's enterprise token instead of silently going anonymous, and a URL whose path merely contains the word `gitlab` stops being treated as GitLab. This replaces the substring guess at `crates/mcp-fs/src/tools/git.rs:683-691` with a parse and an exact map lookup, and makes an undeclared host a loud error.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # resolve_host, the only file comparing a hostname to the map
crates/mcp-fs/src/tools/git.rs  # :683-691 substring detection DELETED; :693-706 anonymous fallback DELETED
```

### Existing Patterns
- The code being replaced is `crates/mcp-fs/src/tools/git.rs:683-691` (lowercase the URL, `contains("github.com")`, `contains("gitlab")`) and its consumer at `:693-706`. Read both before deleting.
- Errors are `ToolError::<code>(msg)` carrying a stable `ERR_*` from `crates/mcp-fs/src/errors.rs:9-22`. That set is closed; add no new constant.

### Data Model (excerpt)
- No new entity. Consumes `HostEntry` and `Provider` from US-001.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-010:** An undeclared host is a configuration error, not a silent downgrade to anonymous. **Rationale:** explicit over implicit; public repositories are the exception here. **Accepted trade:** breaks clones of public repositories from undeclared hosts that work today. **Implemented by:** FR-NEW-007, FR-NEW-008. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:683-706`
- **DEC-015:** Host matching is exact only; no wildcards. **Rationale:** wildcards reintroduce the precedence ambiguity substring matching caused. **Implemented by:** FR-NEW-004, FR-NEW-006. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Transport and trust are explicit** (§7.2): anonymous access is never inferred, it is declared (FR-NEW-007, FR-NEW-008).
- **Usability** (§7.3): the undeclared-host message names the host and states it must be declared in `git.hosts`. A message naming only the failure does not satisfy FR-NEW-007.

### Bounded Context
**Host Resolution** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-006 [EARS-E]: Hostname resolution is by parsing and exact match
> WHEN resolving a remote URL to a provider, THE mcp-fs server SHALL extract the hostname by parsing the URL and SHALL match it against `git.hosts` by exact, case-insensitive equality.

- **Inputs:** `https://github.ibm.com/org/repo.git`; `https://exemple.test/mirrors/mygitlab-mirror.git`.
- **Outputs:** `github` for the first. No match for the second.
- **Business Rules:** Substring matching is prohibited. `github.ibm.com` matches only an entry spelled `github.ibm.com`. A URL whose path contains `gitlab` does not resolve to `gitlab`.
- **Priority:** Must-have
- **Rationale:** Directly fixes both halves of the defect at `git.rs:683-691`.

#### FR-NEW-007 [EARS-O]: An undeclared host is an error
> IF a remote URL's hostname is absent from `git.hosts`, THEN THE mcp-fs server SHALL fail the operation before any network call, naming the host and stating that it must be declared in `git.hosts`.

- **Inputs:** A clone of `https://git.unknown.test/a.git` with no matching entry.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `git.unknown.test`. No socket opened.
- **Business Rules:** Anonymous access is never inferred. It is declared.
- **Priority:** Must-have
- **Rationale:** DEC-010. This is a deliberate behaviour break from `git.rs:693-706`.

#### FR-NEW-008 [EARS-O]: A host declared anonymous carries no credential
> IF a remote URL's hostname resolves to provider `anonymous`, THEN THE mcp-fs server SHALL perform the operation with no credential and SHALL report `auth` as `"anonymous"`.

- **Inputs:** `public.example.org: anonymous` and a clone of `https://public.example.org/a.git`.
- **Outputs:** Clone succeeds; response contains `"auth": "anonymous"`.
- **Business Rules:** No token lookup occurs for an anonymous host.
- **Priority:** Must-have

#### FR-MOD-001 [EARS-E]: Provider resolution replaces substring detection (references `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md`)
> WHEN `git.remote_clone` resolves the provider for a URL, THE mcp-fs server SHALL use the `git.hosts` map with exact hostname matching instead of substring inspection of the URL.

- **Original behavior:** `lower.contains("github.com")` then `lower.contains("gitlab")`, else no provider and `auth: "anonymous"` (`crates/mcp-fs/src/tools/git.rs:683-706`).
- **New behavior (EARS):** WHEN resolving a provider, THE mcp-fs server SHALL parse the hostname and match it exactly against `git.hosts`, failing per FR-NEW-007 on a miss.
- **Reason for change:** The substring test is both too narrow, missing `github.ibm.com`, and too broad, matching any URL whose path contains `gitlab`.
- **Business Rules:** As FR-NEW-006 and FR-NEW-007.
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

**E2E-NEW-011** — Regression: an enterprise GitHub host resolves to github
- **Category:** Security · **Scenario:** SC-004 · **Requirements:** FR-NEW-006, FR-MOD-001 · **Priority:** Critical
- Given `github.ibm.com: github` and a token stored for `(alice@test.com, github.ibm.com)`
- When `git.remote_clone` is called with `https://github.ibm.com/org/repo.git`
- Then the resolved provider is `github`
- And the stored token is supplied as the credential
- And the response reports `"auth": "github"`, not `"anonymous"`
- *This is the too-narrow half of the defect at `crates/mcp-fs/src/tools/git.rs:683-691`.*

**E2E-NEW-012** — Regression: a path containing "gitlab" does not resolve to gitlab
- **Category:** Security · **Scenario:** SC-005 · **Requirements:** FR-NEW-006, FR-NEW-007, FR-MOD-001 · **Priority:** Critical
- Given the reference map, which does not declare `exemple.test`
- And a token stored for `(alice@test.com, gitlab.acme.corp)`
- When `git.remote_clone` is called with `https://exemple.test/mirrors/mygitlab-mirror.git`
- Then the call fails as an undeclared host, naming `exemple.test`
- And no token is looked up, and the `gitlab.acme.corp` token is never supplied to any remote
- *This is the too-broad half of the same defect.*

**E2E-NEW-015** — Host matching is case-insensitive
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-006 · **Priority:** High
- Given `github.ibm.com: github`
- When a clone URL of `https://GitHub.IBM.COM/org/repo.git` is resolved
- Then the provider is `github`

**E2E-NEW-016** — Near-miss hostnames do not match
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-006 · **Priority:** Critical
- Given only `github.com: github`
- When each of `ithub.com`, `github.com.evil.test`, `notgithub.com`, `github.co` is resolved
- Then every one fails as an undeclared host
- And none resolves to `github`

**E2E-NEW-018** — A declared anonymous host clones without a credential
- **Category:** Feature · **Scenario:** SC-004 · **Requirements:** FR-NEW-008 · **Priority:** Critical
- Given `public.example.org: anonymous` and no token stored for that host
- When `git.remote_clone` is called with `https://public.example.org/o/r.git`
- Then the clone succeeds
- And the response reports `"auth": "anonymous"`
- And no token lookup was performed for `(alice@test.com, public.example.org)`

**E2E-NEW-019** — An undeclared host fails before any network activity
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given the reference map and a listener recording outbound connections
- When `git.remote_clone` is called with `https://git.unknown.test/o/r.git`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming `git.unknown.test`
- And the message states the host must be declared in `git.hosts`
- And zero outbound connections were attempted

**E2E-NEW-053** — A missing token fails, and does not fall back to anonymous
- **Category:** Error · **Scenario:** SC-004 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given `github.ibm.com: github` and no token for `(alice@test.com, github.ibm.com)`
- When a clone is attempted
- Then it fails naming the host and instructing authentication
- And the response does not report `"auth": "anonymous"`, and no clone is attempted

**E2E-NEW-055** — A non-member is refused before any network call
- **Category:** Security · **Scenario:** SC-004 · **Requirements:** FR-NEW-006 · **Priority:** Critical
- Given `bob@test.com` is not a member of `proj-a`
- When bob calls `git.remote_clone` for `proj-a`
- Then the call fails with `ERR_FORBIDDEN`
- And no host resolution, no token lookup and no outbound connection occur

**E2E-NEW-057** — A generic host clones with its token
- **Category:** Edge · **Scenario:** SC-004 · **Requirements:** FR-NEW-006 · **Priority:** High
- Given `git.acme.internal: generic` and a token seeded for it
- When a clone of `https://git.acme.internal/o/r.git` runs
- Then the credential `oauth2:<token>` is supplied and the response reports `"auth": "generic"`

**E2E-NEW-058** — The undeclared-host error is distinct from a malformed-URL error
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** High
- When a clone is attempted with `https://git.unknown.test/o/r.git` and then with `not-a-url`
- Then the two failures carry distinguishable messages
- And the second names malformation, not declaration

**E2E-NEW-059** — A URL with no host is rejected as malformed
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** High
- When a clone is attempted with `https:///org/repo.git`
- Then the call fails as malformed, and the message does not instruct declaring an empty host

**E2E-NEW-060** — Push, fetch and pull all fail alike on an undeclared host
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given a volume whose recorded `origin` is on an undeclared host
- When each of `git.remote_push`, `git.remote_fetch`, `git.remote_pull` is called
- Then all three fail with the same undeclared-host error naming the host, before any network call

**E2E-NEW-061** — A trailing-dot hostname does not match
- **Category:** Edge · **Scenario:** SC-005 · **Requirements:** FR-NEW-006 · **Priority:** Medium
- When a clone is attempted with `https://github.com./o/r.git` while only `github.com` is declared
- Then the behaviour is deterministic and documented: the call fails as an undeclared host

**E2E-NEW-062** — An IDN hostname is handled deterministically
- **Category:** Edge · **Scenario:** SC-005 · **Requirements:** FR-NEW-006 · **Priority:** Medium
- Given `xn--gthub-0na.com: github` declared in punycode
- When a clone of `https://gïthub.com/o/r.git` is attempted
- Then resolution is performed on the punycode form and the outcome matches the declared entry

**E2E-NEW-083** — Push to an anonymous host attempts with no credential
- **Category:** Edge · **Scenario:** SC-009 · **Requirements:** FR-NEW-008 · **Priority:** Medium
- Given an `origin` on `public.example.org`, declared `anonymous`
- When a push runs
- Then no credential is supplied and the remote's rejection, if any, is surfaced verbatim

**E2E-NEW-090** — Fetch with a missing token is rejected before the network
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-007 · **Priority:** Critical
- Given no token for the origin's host, which is declared `github`
- When a fetch runs
- Then it fails naming the host, with no outbound connection

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never match a hostname by substring, prefix or suffix. Exact, case-insensitive equality only (FR-NEW-006).
- Do not fall back to `anonymous` when the lookup misses. That is the defect this story exists to remove (DEC-010).

### Scope Boundary
Resolution and its two policy outcomes only. Do NOT add URL scheme or userinfo validation (US-008), and do NOT record `origin` (US-008).

## Non Regression

### Existing Tests That Must Pass
- A clone from a host declared `anonymous` still succeeds with no credential and reports `auth: "anonymous"` — this is the one path that behaves as it did before.
- Membership authorization still runs before any network call (`ERR_FORBIDDEN`, EXC-004e), unchanged from the existing clone path.
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
