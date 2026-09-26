# US-007: git.auth_revoke removes exactly one host and never reaches across hosts

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 7
> Depends On: US-003, US-006
> Complexity: S
> min_tier: 1
> Files touched: 1

## Objective

Revoking a credential removes the one `(person, host)` the caller named and provably leaves every other host intact, because `github.com` and `github.ibm.com` are different hosts holding different tokens. `provider` becomes optional so the tool can be called by host alone.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git_auth.rs  # auth_revoke schema :126-131, provider required at :118-119 and read at :122
```

### Existing Patterns
- `provider` is required today (`crates/mcp-fs/src/tools/git_auth.rs:118-119`, read at `:122`). This story makes it optional while requiring at least one of `provider` and `host`.
- The disagreement rejection mirrors EXC-003b, naming both values.
- Revocation clears memory and persistence together, as the existing revoke path does.

### Data Model (excerpt)
- No new entity. Deletes one `oauth_tokens` row keyed `(person, host)`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-006:** The three auth tools gain an optional `host`. **Rationale:** with a per-host key, a provider-only revoke is ambiguous. **Implemented by:** FR-MOD-002, FR-MOD-003, FR-MOD-004. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:76-82`, `:104-116`, `:126-131`
- **DEC-029:** `git.auth_revoke` never revokes across hosts. **Rationale:** `github.com` and `github.ibm.com` are different hosts holding different tokens. **Implemented by:** FR-MOD-004. **Round:** 2b. **Code evidence:** n/a
- **DEC-030:** `git.auth_revoke` with `host` omitted defaults to the canonical public host, uniform with DEC-018. **Alternatives considered:** making `host` required, a breaking schema change; revoking all hosts for the provider. **Implemented by:** FR-MOD-004. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Idempotent** (§7.4): revoking a host with no stored token succeeds and reports nothing revoked (EXC-007b).
- **Strict per-person isolation** (§7.2): a person revokes only their own credential.

### Bounded Context
**Token Custody** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-MOD-004 [EARS-E]: `git.auth_revoke` revokes exactly one host (references `FR-629`)
> WHEN `git.auth_revoke` is called, THE mcp-fs server SHALL delete the token for exactly one `(person, host)` pair and SHALL NOT delete any other stored token.

- **Original behavior:** `git.auth_revoke(provider)` deleting the single `(person, provider)` entry (`git_auth.rs:116-127`).
- **New behavior (EARS):** IF `host` is omitted, THEN THE mcp-fs server SHALL target the provider's canonical public host.
- **Reason for change:** With per-host identity, a provider-only revoke would destroy a token for a host the caller never named. `github.com` and `github.ibm.com` are not the same host.
- **Business Rules:** Revoking a host with no stored token succeeds and reports nothing revoked.
- **Priority:** Must-have

---

#### FR-NEW-068 [EARS-U]: `git.auth_revoke` takes an optional provider and reports the resolved one
> The mcp-fs server SHALL declare both `provider` and `host` optional on `git.auth_revoke`, SHALL require at least one of the two, SHALL target the provider's canonical public host when only `provider` is given, SHALL target the named host when `host` is given, and SHALL set the response `provider` to the host's value in `git.hosts` when the host is declared and to null when it is not.

- **Inputs:** `{"provider":"github"}`; `{"host":"github.ibm.com"}`; `{"provider":"github","host":"github.ibm.com"}`; `{"host":"git.unknown.test"}`; `{}`.
- **Outputs:** `{"host":"github.com","provider":"github","revoked":true}`; `{"host":"github.ibm.com","provider":"github","revoked":true}`; the same for the third; `{"host":"git.unknown.test","provider":null,"revoked":false}`; `ERR_INVALID_ARGUMENT` naming both parameters for the fifth.
- **Business Rules:** A `host` whose `git.hosts` value disagrees with a supplied `provider` is rejected with `ERR_INVALID_ARGUMENT` naming both, mirroring EXC-003b. An undeclared host is **not** rejected here: revoking a host that can no longer be reached must stay possible, and reports `revoked:false` when nothing was stored. This supersedes the description of `provider` as required in Section 3.1 and in SC-007 step 2.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F1 of round 3. `provider` is required today (`crates/mcp-fs/src/tools/git_auth.rs:118-119`, read at `:122`), yet FR-NEW-051 and E2E-NEW-190 call the tool without it.

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

### The 8 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-066** — Revoke removes exactly one host
- **Category:** State Transition · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke` is called with `{"provider": "github", "host": "github.ibm.com"}`
- Then `github.ibm.com` is gone from memory and persistence
- And the `github.com` token is untouched and still usable for a clone

**E2E-NEW-067** — Revoke with an omitted host targets only the canonical host
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke` is called with `{"provider": "github"}`
- Then only `github.com` is revoked
- And `github.ibm.com` remains, proving revocation never crosses hosts

**E2E-NEW-068** — Revoking an absent host is idempotent
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-MOD-004 · **Priority:** High
- When `git.auth_revoke` is called twice for `github.ibm.com`
- Then both calls succeed and the second reports nothing revoked

**E2E-NEW-233** — Revoke by provider alone targets the canonical host
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- Given tokens for `github.com` and `github.ibm.com`
- When `git.auth_revoke {"provider":"github"}` is called
- Then the response is `{"host":"github.com","provider":"github","revoked":true}` and `github.ibm.com` remains

**E2E-NEW-234** — Revoke by host alone succeeds without a provider
- **Category:** Feature · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- When `git.auth_revoke {"host":"github.ibm.com"}` is called with no `provider`
- Then it succeeds with `{"host":"github.ibm.com","provider":"github","revoked":true}`

**E2E-NEW-235** — Revoke with neither parameter is rejected
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** Critical
- When `git.auth_revoke {}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming both `provider` and `host`

**E2E-NEW-236** — Revoke with a disagreeing provider and host is rejected
- **Category:** Error · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** High
- When `git.auth_revoke {"provider":"gitlab","host":"github.ibm.com"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming both values

**E2E-NEW-237** — Revoking an undeclared host is possible and reports a null provider
- **Category:** Edge · **Scenario:** SC-007 · **Requirements:** FR-NEW-068 · **Priority:** High
- When `git.auth_revoke {"host":"git.unknown.test"}` is called
- Then it returns `{"host":"git.unknown.test","provider":null,"revoked":false}` rather than an undeclared-host error

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never revoke across hosts for a provider (DEC-029, FR-MOD-004). That would destroy a credential the caller never named.
- Do not reject an undeclared host here: revoking a host that can no longer be reached must stay possible, reporting `revoked:false` (FR-NEW-068).

### Scope Boundary
`git.auth_revoke` only. The screen's revoke button reuses this function rather than reimplementing it (US-017).

## Non Regression

### Existing Tests That Must Pass
- **E2E-MOD-003** — `revoke_clears_the_token_and_is_idempotent` (`crates/mcp-fs/src/tools/git_auth.rs:545`) must now prove revoking `github.ibm.com` twice succeeds and `github.com` still authenticates a clone.
- The schema change is additive: an existing call passing only `provider` still works, targeting the canonical public host (DEC-030).
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
