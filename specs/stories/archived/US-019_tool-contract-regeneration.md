# US-019: Regenerate the frozen tool contract at 63 tools

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 19
> Depends On: US-004, US-006, US-007, US-008, US-009, US-010, US-011, US-013, US-014
> Complexity: S
> min_tier: 1
> Files touched: 4

## Objective

The frozen contract records the four new tools and the three modified schemas, bringing it to 63 tools, with every required parameter declared as the specification fixes it. This lands last, because regenerating before every schema is final means regenerating repeatedly and reviewing each diff.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
TOOL_CONTRACT.txt  # the human-readable authority
tool-contract-golden.json  # machine-checked, never hand-edited
crates/mcp-fs/src/tools/contract_golden.rs  # tool_contract_golden_is_current at :129
```

### Existing Patterns
- Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`, then review the diff. The golden file is **never** hand-edited (AGENTS.md).
- Three tests compare every name, description and `inputSchema` against the golden, serialized, so a reordered schema key fails too.
- Parameter descriptions are the LLM-facing docs and are frozen: treat any edit as a deliberate contract change.

### Data Model (excerpt)
- No new entity.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a

### Applicable NFRs
- **The contract is the authority** (AGENTS.md): `TOOL_CONTRACT.txt` for humans, `tool-contract-golden.json` machine-checked on every test run.

### Bounded Context
**Host Resolution / Token Custody / Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-045 [EARS-U]: The frozen tool contract is regenerated
> The mcp-fs server SHALL expose, and the frozen contract SHALL record, the four new tools `git.token_set`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull`, together with the three modified schemas `git.auth`, `git.auth_status` and `git.auth_revoke`, bringing the contract to 63 tools.

- **Inputs:** `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`.
- **Outputs:** Updated `TOOL_CONTRACT.txt` and `tool-contract-golden.json`; all three contract tests pass.
- **Business Rules:** The golden file is regenerated, never hand-edited. The token screen is an HTTP route and SHALL NOT appear in the tool contract.
- **Priority:** Must-have

#### FR-NEW-071 [EARS-U]: The four new tool schemas declare their required parameters
> The mcp-fs server SHALL declare `host` and `token` required and `expires_at` optional and nullable on `git.token_set`; `mount_id` and `branch` required on `git.remote_push`; `mount_id` required on `git.remote_fetch`; and `mount_id` and `branch` required with `on_conflict` optional and nullable on `git.remote_pull`.

- **Inputs:** `git.remote_push {"mount_id":"proj-a"}`; `git.remote_pull {"mount_id":"proj-a"}`; `git.token_set {"host":"github.ibm.com","token":"ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"}`; `git.remote_fetch {"mount_id":"proj-a"}`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming `branch` for the first two; success for the last two.
- **Business Rules:** `host` stays optional on `git.auth`, `git.auth_status` and `git.auth_revoke` (FR-MOD-002, FR-MOD-003, FR-NEW-068), so those three schema changes remain additive. These lists are what `tool-contract-golden.json` records after regeneration (FR-NEW-045).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F4 of round 3. The frozen contract records a `required` list per tool, so an unstated one is a user-visible artifact left to the implementer.

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

**E2E-NEW-158** — All new tools are registered
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When the registry is enumerated with git enabled
- Then `git.token_set`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull` are present
- And none is present when git is disabled, consistent with `FR-611`

**E2E-NEW-159** — New tool schemas match the frozen contract
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When each new tool's `inputSchema` is compared to `tool-contract-golden.json`
- Then each matches exactly, including key order

**E2E-NEW-160** — Modified tool schemas match the frozen contract
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-045 · **Priority:** Critical
- When `git.auth`, `git.auth_status` and `git.auth_revoke` are compared to the golden file
- Then each matches, each exposes `host` as optional, and no previously required parameter became optional or vice versa

**E2E-NEW-244** — A push without `branch` is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.remote_push {"mount_id":"proj-a"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming `branch`, with no default to the checked-out branch

**E2E-NEW-245** — A pull without `branch` is rejected
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.remote_pull {"mount_id":"proj-a"}` is called
- Then it fails with `ERR_INVALID_ARGUMENT` naming `branch`

**E2E-NEW-246** — The optional parameters are genuinely optional
- **Category:** Feature · **Scenario:** SC-002 · **Requirements:** FR-NEW-071 · **Priority:** Critical
- When `git.token_set` is called without `expires_at` and `git.remote_fetch` with only `mount_id`
- Then both succeed
- And the regenerated contract records exactly the required lists FR-NEW-071 declares

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never hand-edit `tool-contract-golden.json`.
- Do not regenerate before every schema in US-004 through US-014 is final (§14.1 step 9).

### Scope Boundary
Contract regeneration and the required-parameter declarations. Change no behaviour.

## Non Regression

### Existing Tests That Must Pass
- `git_remote_clone_schema_matches_the_contract` (`crates/mcp-fs/src/tools/git.rs`) must pass against the regenerated contract.
- `tool_contract_golden_is_current` (`crates/mcp-fs/src/tools/contract_golden.rs:129`) must pass.
- The three schema changes are additive — `host` is optional everywhere — so existing callers keep working (§9.5).
- The token screen is an HTTP route and must NOT appear in the tool contract (FR-NEW-045).
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
