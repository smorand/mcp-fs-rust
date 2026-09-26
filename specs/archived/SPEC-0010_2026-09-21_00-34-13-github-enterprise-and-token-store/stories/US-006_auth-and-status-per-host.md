# US-006: git.auth takes a host, git.auth_status reports one entry per held host

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 6
> Depends On: US-003
> Complexity: L
> min_tier: 1
> Files touched: 2

## Objective

A person completes the device flow for a named host, so an enterprise GitLab token stops being filed under `gitlab.com` and lost. Status stops iterating a fixed provider list and starts enumerating what the person actually holds, reporting `valid` or `expired` per host and never a token value.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git_auth.rs  # git.auth :160-178 and poller :179-222; auth_status :104-116 and :225-252
crates/mcp-fs/src/git/oauth/store.rs  # list_ids :174 returns pairs for EVERY person, unfiltered
```

### Existing Patterns
- `git.auth` validates the provider with an exact match at `crates/mcp-fs/src/tools/git_auth.rs:160-162`, returns the device code immediately at `:163-178`, and spawns a detached poller at `:179-222`. Keep that shape.
- `auth_status` today iterates a fixed `PROVIDERS` constant and computes a boolean per provider (`crates/mcp-fs/src/tools/git_auth.rs:225-252`). It never enumerates the store — that is DRIFT-010.
- `instance_url` stays on the stored session unchanged (`crates/mcp-fs/src/git/oauth/store.rs:24-32`).
- `expires_at` is serialized by the existing `round_trip_iso` helper.

### Data Model (excerpt)
- No new entity. Reads `OAuthSession` keyed `(person, host)`; adds a per-person enumeration to `OAuthTokenStore`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-006:** The three auth tools gain an optional `host`. **Rationale:** with a per-host key, a provider-only revoke is ambiguous. **Implemented by:** FR-MOD-002, FR-MOD-003, FR-MOD-004. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:76-82`, `:104-116`, `:126-131`
- **DEC-018:** An omitted `host` on `git.auth` defaults to the provider's canonical public host. **Rationale:** every existing caller keeps working unchanged. **Implemented by:** FR-MOD-002. **Round:** 2b. **Code evidence:** n/a

### Implementer Decision: the `expires_at` representation

Section 15.1 of the specification leaves this open: *"Whether the implementation relaxes the column
or adopts a far-future sentinel is the implementer's choice; the observable requirement is that
`git.auth_status` reports such a token `valid` indefinitely and FR-NEW-018 never fires for it."*

**Decided: relax the column to nullable. `None` means non-expiring.** Settled by the user during
partitioning, so it is not re-litigated during implementation.

- `crates/mcp-fs/src/git/oauth/persistence.rs:39` becomes `Column::new("expires_at", ColumnType::Text)`.
  `Column::new` is already the nullable constructor and `instance_url` in the same column list already
  uses it.
- The load path reads `r.opt_text(4)?` instead of `r.text(4)?`. That accessor
  (`crates/mcp-fs/src/storage/rel/mod.rs:235`) is already used on the next line for `instance_url`, and
  `option_binds_null_or_the_inner_value` (`crates/mcp-fs/src/storage/rel/mod.rs:573`) proves an `Option`
  binds as NULL on every engine.
- `OAuthSession.expires_at` becomes `Option<DateTime<Utc>>`
  (`crates/mcp-fs/src/git/oauth/store.rs:28`), and `is_valid_at` becomes
  `self.expires_at.is_none_or(|e| e > now)` (`:34-36`), which states FR-NEW-014 directly rather than
  encoding it in a magic constant.
- FR-NEW-051's `expires_at: String|null` is then a direct serialization of that `Option`, with no
  sentinel-to-null mapping to forget in `git.auth_status` or on the token screen.

The schema change costs nothing extra because US-003 drops and recreates `oauth_tokens` anyway
(DRIFT-005, FR-NEW-011).

### Applicable NFRs
- **Strict per-person isolation** (§7.2, FR-NEW-040): a platform admin gains no access to another person's tokens. The naive `list_ids` would leak every person's hosts.
- **No token on any observable surface** (§7.2): status never includes a token value.

### Bounded Context
**Token Custody** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Implementation Drift To Close In This Story

> Carried verbatim from the specification's Section 19. These are resolved **during** this story, not later. An entry still open when the branch merges moves to `specs/BACKLOG.md`, and that move is a decision someone signs, not a silence.

#### DRIFT-010: `git.auth_status` never enumerates the token store
- **Spec says:** FR-MOD-003, "SHALL report one entry per stored `(person, host)`".
- **Code does:** `auth_status` iterates a fixed `PROVIDERS` constant and computes a boolean per provider (`crates/mcp-fs/src/tools/git_auth.rs:225-252`); it never enumerates the store. The only enumeration available is `OAuthTokenStore::list_ids` (`crates/mcp-fs/src/git/oauth/store.rs:174`), which returns pairs for **every person, unfiltered**.
- **Nature:** missing capability
- **Resolution during implementation:** Add a per-person enumeration to `OAuthTokenStore`, for example `list_for_person(person) -> Vec<(String, OAuthSession)>` filtering on the lowercased person part of the key, and build the response from it. **Using `list_ids` naively would leak every person's hosts and violate FR-NEW-040.**
- **Detected by:** E2E-MOD-002 fails, because a host with no token would still be listed. E2E-NEW-162 fails, because bob would see alice's hosts.
- **Blocks which requirement:** FR-MOD-003, FR-NEW-038, and the isolation guarantee of FR-NEW-040.
- **Status:** resolved (`e2e_new_070_status_distinguishes_expired_from_absent`, `list_for_person_never_leaks_another_persons_hosts`; this commit)

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-MOD-002 [EARS-E]: `git.auth` accepts a host (references `FR-624`)
> WHEN `git.auth` is called, THE mcp-fs server SHALL accept an optional `host` and SHALL store the resulting token keyed `(person, host)`.

- **Original behavior:** `git.auth(provider, instance_url?)`, storing keyed `(person, provider)` (`crates/mcp-fs/src/tools/git_auth.rs:76-92`, `:195-207`).
- **New behavior (EARS):** IF `host` is omitted, THEN THE mcp-fs server SHALL use the provider's canonical public host, `github.com` for `github` and `gitlab.com` for `gitlab`.
- **Reason for change:** Token identity is per host; a provider-only flow cannot express which host was authorised.
- **Business Rules:** A `host` mapping to a provider other than the one given is rejected, naming both. A `host` mapping to `generic` or `anonymous` is rejected, because the device flow exists only for `github` and `gitlab`.
- **Priority:** Must-have

#### FR-NEW-066 [EARS-E]: `instance_url` and `host` must agree on `git.auth`
> WHEN `git.auth` is called with a non-null `instance_url`, THE mcp-fs server SHALL parse its hostname, SHALL use that hostname as the host when `host` is omitted, and SHALL reject the call with `ERR_INVALID_ARGUMENT` naming both values when a supplied `host` differs from it.

- **Inputs:** `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}`; `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp","host":"gitlab.com"}`; `{"provider":"gitlab","host":"gitlab.acme.corp"}`.
- **Outputs:** The first stores under `(person, gitlab.acme.corp)`; the second is rejected naming both hosts; the third proceeds and keeps `instance_url` null on the stored session.
- **Business Rules:** The derived host must still be declared in `git.hosts` and map to the given provider (EXC-003b). `instance_url` remains on the stored session unchanged (`crates/mcp-fs/src/git/oauth/store.rs:24-32`). The canonical-host default of FR-MOD-002 applies only when both `host` and `instance_url` are absent.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A9. Without this, an enterprise GitLab token obtained through the device flow could be stored under `gitlab.com` and never found again.

#### FR-MOD-003 [EARS-E]: `git.auth_status` reports per host (references `FR-629`)
> WHEN `git.auth_status` is called, THE mcp-fs server SHALL report one entry per stored `(person, host)`, each naming the host, the provider, a validity of `valid` or `expired`, and the expiry, and SHALL NOT include any token value.

- **Original behavior:** One entry per provider (`git_auth.rs:97-115`).
- **New behavior (EARS):** WHEN `provider` or `host` is supplied, THE mcp-fs server SHALL filter the report to matching entries.
- **Reason for change:** One person can hold several tokens per provider, one per host.
- **Business Rules:** An expired token is reported as `expired`, never omitted, which is what FR-NEW-019 preserves.
- **Priority:** Must-have

#### FR-NEW-051 [EARS-U]: The modified auth tool response shapes are fixed
> The mcp-fs server SHALL return from `git.auth_status` the object `{"statuses":[{"host":String,"provider":String,"validity":"valid"|"expired","expires_at":String|null,"scopes":[String]}]}` for every call, and SHALL return from `git.auth_revoke` the object `{"host":String,"provider":String|null,"revoked":Boolean}`.

- **Inputs:** `git.auth_status {}` for a person holding a valid `github.com` token and an expired `github.ibm.com` token; `git.auth_revoke {"host":"git.unknown.test"}`.
- **Outputs:** Two entries in `statuses`, ordered by `host` ascending; `{"host":"git.unknown.test","provider":null,"revoked":false}`.
- **Business Rules:** The `authenticated` key is removed. The single-provider answer no longer carries a shape distinct from the list. `expires_at` is serialized by the existing `round_trip_iso` helper and is null for a non-expiring token. A host with no stored token is never listed.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-005. The current shape is `authenticated: bool` plus `provider`, and it differs between the single and list answers, so "reports per host" did not determine the response.

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

### The 19 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-044** — Device flow stores under the named host
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "github", "host": "github.ibm.com"}` and the fake flow authorises
- Then the token is stored for `(alice@test.com, github.ibm.com)`
- And nothing is stored for `(alice@test.com, github.com)`

**E2E-NEW-045** — An omitted host defaults to the canonical public host
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "github"}` and the flow authorises
- Then the token is stored for `(alice@test.com, github.com)`
- And the same call with `{"provider": "gitlab"}` stores under `gitlab.com`

**E2E-NEW-046** — A host mapped to a different provider is rejected
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** Critical
- When `git.auth` is called with `{"provider": "gitlab", "host": "github.ibm.com"}`
- Then the call fails with `ERR_INVALID_ARGUMENT` naming both `gitlab` and `github`
- And no device code is requested

**E2E-NEW-047** — A generic host is rejected for the device flow
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` names `git.acme.internal`, declared `generic`
- Then the call fails, stating the device flow exists only for `github` and `gitlab`

**E2E-NEW-048** — An anonymous host is rejected for the device flow
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` names `public.example.org`
- Then the call fails for the same reason as E2E-NEW-047

**E2E-NEW-049** — An unknown provider is still rejected before any HTTP call
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-MOD-002 · **Priority:** High
- When `git.auth` is called with `{"provider": "GitHub"}`
- Then the call fails with `ERR_INVALID_ARGUMENT` and no device code is requested
- *Preserves the existing exact-match behaviour at `crates/mcp-fs/src/tools/git_auth.rs:160-162`.*

**E2E-NEW-050** — An undeclared host is rejected for the device flow
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-MOD-002, FR-NEW-007 · **Priority:** High
- When `git.auth` names `git.unknown.test`
- Then the call fails naming that host, and no device code is requested

**E2E-NEW-063** — Status reports one entry per host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_status` is called with no arguments
- Then two entries are returned, each with host, provider `github`, validity and expiry

**E2E-NEW-064** — Status filters by provider and by host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** High
- Given tokens for `github.com`, `github.ibm.com` and `gitlab.acme.corp`
- When called with `{"provider": "github"}` then `{"host": "github.ibm.com"}`
- Then the first returns two entries and the second exactly one

**E2E-NEW-065** — Status never includes a token value
- **Category:** Security · **Scenario:** SC-007 · **Requirements:** FR-MOD-003, FR-NEW-043 · **Priority:** Critical
- When `git.auth_status` returns entries for seeded tokens
- Then the serialized response contains no substring of length 8 or more from any stored token

**E2E-NEW-069** — Status with no tokens returns an empty list
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-003 · **Priority:** High
- When `git.auth_status` is called by a person holding no tokens
- Then it succeeds and returns an empty collection, not an error

**E2E-NEW-070** — Status distinguishes expired from absent
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-003, FR-NEW-019 · **Priority:** Critical
- Given an expired token for `github.ibm.com` and nothing for `gitlab.acme.corp`
- When `git.auth_status` is called
- Then `github.ibm.com` appears with validity `expired`
- And `gitlab.acme.corp` does not appear at all

**E2E-NEW-161** — Existing callers keep working without `host`
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-MOD-002, FR-MOD-003, FR-MOD-004 · **Priority:** Critical
- When `git.auth`, `git.auth_status` and `git.auth_revoke` are each called with exactly the arguments valid before this change
- Then all three succeed, defaulting to the canonical public host

**E2E-NEW-188** — `git.auth_status` returns the declared shape
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** Critical
- Given a valid `github.com` token and an expired `github.ibm.com` token
- When `git.auth_status {}` is called
- Then the response is `{"statuses":[{"host":"github.com",...,"validity":"valid",...},{"host":"github.ibm.com",...,"validity":"expired",...}]}` ordered by host ascending
- And no `authenticated` key appears anywhere

**E2E-NEW-189** — `expires_at` is null for a non-expiring token
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** High
- Given a token seeded with no `expires_at`
- When `git.auth_status` is called
- Then that entry's `expires_at` is null and its `validity` is `valid`

**E2E-NEW-190** — Revoking an absent host returns the declared shape
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-051 · **Priority:** Critical
- When `git.auth_revoke {"host":"git.unknown.test"}` is called
- Then the response is exactly `{"host":"git.unknown.test","provider":null,"revoked":false}`

**E2E-NEW-227** — `instance_url` supplies the host when host is omitted
- **Category:** Feature · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** Critical
- Given `gitlab.acme.corp: gitlab` declared
- When `git.auth` is called with `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}` and the flow succeeds
- Then the token is stored under `(alice@test.com, gitlab.acme.corp)`
- And nothing is stored under `gitlab.com`

**E2E-NEW-228** — A disagreeing `instance_url` and `host` are rejected
- **Category:** Error · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** Critical
- When `git.auth` is called with `{"provider":"gitlab","instance_url":"https://gitlab.acme.corp","host":"gitlab.com"}`
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `gitlab.acme.corp` and `gitlab.com`
- And no device code is requested

**E2E-NEW-229** — The canonical default applies only when both are absent
- **Category:** Edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-066 · **Priority:** High
- When `git.auth` is called with `{"provider":"gitlab"}` alone
- Then the token is stored under `gitlab.com`, and the stored `instance_url` is null

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never build the status response from `list_ids` naively: it returns pairs for every person and would violate FR-NEW-040 (DRIFT-010).
- Never list a host for which the person holds no token (FR-NEW-051).

### Scope Boundary
`git.auth` and `git.auth_status`. Do NOT change `git.auth_revoke` (US-007) and do NOT regenerate the frozen contract (US-019).

## Non Regression

### Existing Tests That Must Pass
- These existing tests change and must pass: `git_auth_schema_matches_the_contract` (git_auth.rs:371, schema gains `host`), `git_auth_status_schema_has_no_required_parameter` (:390, gains `host`, still no required parameter), `pending_then_success_stores_the_token` (:421, assert storage keyed by host), `auth_status_reports_one_provider` (:487), `auth_status_reports_all_providers_when_none_is_given` (:507).
- **E2E-MOD-002** — `an_expired_token_reports_as_unauthenticated` (:526) must now report the host with validity `expired`, distinct from absent, and a host with no token is not listed at all.
- **E2E-MOD-004** — `gitlab_keeps_the_instance_url_on_the_stored_session` (:454): the session keys on the host derived from `instance_url`, still carries `instance_url`, and nothing is stored under `gitlab.com`.
- The schema change is additive: `host` is optional, so every existing caller keeps working through canonical-host defaulting (DEC-018).
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
5. **Drift closed.** DRIFT-010 is resolved in this branch, by the resolution quoted above, and you can name the test that proves it.
