# US-023: OAuth scope request, validation and reporting

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 23
> Depends On: none
> Complexity: M
> min_tier: 2
> Files touched: 4

## Objective

Make the provider REST API reachable at all. The GitLab device flow currently requests a scope set that grants Git-over-HTTP only and no API access, so every merge-request tool would be unauthorized. This story adds the `api` scope, makes both providers' scopes configurable, and validates the required scope before any network call.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/git/oauth/device_flow.rs
crates/mcp-fs/src/git/oauth/store.rs
crates/mcp-fs/src/tools/git_auth.rs
crates/mcp-fs/src/config.rs
```

### Existing Patterns
Token storage is `crates/mcp-fs/src/git/oauth/store.rs` and `persistence.rs` (`scopes` column at `:50`); the device flow and its scope constants are `device_flow.rs:38-39`. The injectable client seam used by tests is `MockDeviceFlowClient` in `tools/git_auth.rs`.
Scopes are parsed from the token response (`device_flow.rs:366-392`), stored (`persistence.rs:129-146`) and reported (`tools/git_auth.rs:396`) but validated nowhere today.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-910:** The GitLab device flow requests the `api` scope in addition to its current scopes, and required scope is validated before the network call. **Rationale:** verified against GitLab's documentation, `write_repository` grants Git-over-HTTP access only and **no** REST API access, so every merge-request tool would be unauthorized; and failing late at the provider rather than early with a clear message contradicts the existing expired-token precedent. **Alternatives considered:** request nothing extra and let the provider reject, rejected as an unusable error experience. **Consequence accepted:** existing GitLab tokens must be re-granted. **Implemented by:** FR-NEW-330, FR-NEW-331, FR-NEW-332, FR-NEW-333, FR-MOD-109. **Round:** 3 (Q-A, resolved by research). **Code evidence:** `crates/mcp-fs/src/git/oauth/device_flow.rs:38` (GitHub `repo`), `:39` (GitLab `read_repository write_repository`), `crates/mcp-fs/src/git/oauth/persistence.rs:50` (scopes stored), `crates/mcp-fs/src/tools/git_auth.rs:396` (scopes reported, never validated).

- **DEC-911:** A token whose stored scope set is empty or unknown is attempted rather than pre-emptively refused, while a known-insufficient scope fails early. **Rationale:** `git.token_set` seeds PATs whose scopes the server cannot always enumerate; refusing them would make the tool useless for its main purpose. **Implemented by:** FR-NEW-334. **Round:** 3. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:128-159` (`git.token_set`).

### Applicable NFRs

- 7.2 Security: scope is checked before the network, mirroring the expired-token rule.
- 7.2 Security: tokens never appear in a response, log line or tracing span.

### Bounded Context
**Identity and credentials** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

7 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-330 [EARS-E]: The GitLab device flow requests API access
> WHEN the GitLab device flow requests authorization THE mcp-fs server SHALL include the `api` scope in the requested scope set.

- **Business Rules:** Today the constant is `read_repository write_repository` (`crates/mcp-fs/src/git/oauth/device_flow.rs:39`). GitLab documents `write_repository` as Git-over-HTTP only, granting **no** REST API access, so every merge-request tool would be unauthorized without this change. GitHub's existing `repo` scope (`device_flow.rs:38`) already covers the whole pull-request surface and is unchanged.
- **Priority:** Must-have

### FR-NEW-331 [EARS-U]: Requested scopes are configurable
> The mcp-fs server SHALL read the scope requested for each provider from configuration, defaulting to `repo` for GitHub and `api read_repository write_repository` for GitLab.

- **Inputs:** config keys `git.github_scope`, `git.gitlab_scope`.
- **Business Rules:** The constants are hardcoded today; a deployment whose provider policy differs cannot currently adjust them without a rebuild.
- **Priority:** Should-have

### FR-NEW-332 [EARS-E]: Required scope is validated before the network call
> WHEN a `git.pr_*` tool is called THE mcp-fs server SHALL compare the scopes recorded for that `(person, host)` against the scope set required for the operation and, when they are insufficient, SHALL reject with `ERR_FORBIDDEN` before issuing any request.

- **Business Rules:** Mirrors the existing rule that an expired token fails before the network. Scopes are already stored (`crates/mcp-fs/src/git/oauth/persistence.rs:50`) and reported (`crates/mcp-fs/src/tools/git_auth.rs:396`) but are validated nowhere in the tree today.
- **Priority:** Must-have

### FR-NEW-333 [EARS-E]: The scope error is actionable
> WHEN a scope check fails THE mcp-fs server SHALL name the missing scope, the host, and `git.auth` and `git.token_set` as the tools that grant a replacement.

- **Priority:** Must-have

### FR-NEW-334 [EARS-O]: A token with unrecorded scopes is not assumed sufficient
> IF the stored scope set for a `(person, host)` is empty or unknown, as it is for a token seeded through `git.token_set` without scope information, THEN THE mcp-fs server SHALL attempt the operation and SHALL surface the provider's authorization failure verbatim rather than pre-emptively refusing.

- **Business Rules:** Deliberate asymmetry: a *known-insufficient* scope fails early (FR-NEW-332), an *unknown* scope is given the benefit of the doubt, because refusing it would make `git.token_set` useless for any PAT whose scopes the server cannot enumerate.
- **Priority:** Must-have

### FR-NEW-335 [EARS-E]: Auth status reports PR capability
> WHEN `git.auth_status` is called THE mcp-fs server SHALL report, per entry, the scopes held and whether they are sufficient for the pull-request surface.

- **Priority:** Should-have

### FR-MOD-109 [EARS-E]: The GitLab scope constant includes API access
> WHEN the GitLab device flow builds its scope string THE mcp-fs server SHALL include `api`.

- **Original behavior:** `read_repository write_repository` (`crates/mcp-fs/src/git/oauth/device_flow.rs:39`).
- **Reason for change:** Without it every merge-request tool is unauthorized. See FR-NEW-330.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

17 tests: EdgeCase 8, Failure 4, Happy 4, SideEffect 1.
Scenarios covered: SC-914, SC-925, SC-926, SC-927, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-773 — Happy — P0.** Unit-level on `device_flow.rs`: run `HttpDeviceFlowClient::with_github_urls` against a local one-shot HTTP server (the pattern already supported at `device_flow.rs:162-171`) extended for GitLab, or assert the constant directly.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-330, FR-MOD-109.
Then the GitLab scope constant equals exactly `"api read_repository write_repository"` (was `"read_repository write_repository"` at `device_flow.rs:39`); And the form field `scope` sent to `{instance}/oauth/authorize_device` equals that same string.

#### E2E-NEW-774 — Happy — P0.** Same for GitHub.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-330, FR-NEW-331.
Then the GitHub scope constant is still exactly `"repo"` (`device_flow.rs:38`) and the posted `scope` form field equals `"repo"`. (Guards against a copy-paste widening of the GitHub scope while fixing GitLab.)

#### E2E-NEW-775 — Happy — P0.** `FakeFlow` scripted with `TokenPoll::granted("glpat_FAKE_999", vec!["api".into(),"read_repository".into(),"write_repository".into()], now+1h)`, host `gitlab.com`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-335, FR-NEW-330.
When `git.auth{provider:"gitlab", host:"gitlab.com"}` then wait via `eventually(...)` for the poller, then `git.auth_status{host:"gitlab.com"}`.
Then `statuses[0]["scopes"] == ["api","read_repository","write_repository"]`; And `statuses[0]["validity"] == "valid"`; And the persisted `oauth_tokens.scopes` row round-trips the same three values after reopening the store with `with_persistence`.

#### E2E-NEW-781 — Failure — P0.** Seed via the real tool: `git.token_set{host:"gitlab.com", token:"glpat_SEEDED_005"}` (which stores `Vec::new()` scopes, `git_auth.rs:505`).

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then `ERR_FORBIDDEN`; message contains `token for host gitlab.com has no recorded scopes` and contains `re-authenticate with git.auth`, and contains `git.token_set`; And no calls. (A seeded token is NOT assumed sufficient.)
And: the same seeding against `github.com` then `git.pr_list{mount_id:"acme-api"}` fails identically, so the rule is not GitLab-specific.

#### E2E-NEW-782 — EdgeCase — P1.** Seed `gitlab.com` with scopes `["everything","super_admin","*"]`.

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
When `git.pr_get{mount_id:"acme-lab", pr_number:42}`.
Then `ERR_FORBIDDEN` with message containing `missing scope 'api' (or 'read_api')`; And no calls. (No wildcard interpretation, no "looks powerful" heuristic.)

#### E2E-NEW-783 — Happy — P0.** Seeds: `github.com` -> `["repo"]`; `gitlab.com` -> `["read_repository","write_repository"]`; `gitlab.example.test` -> `["api"]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-335.
When `git.auth_status{}` (no filters).
Then `statuses` has 3 entries sorted by host ascending: `github.com`, `gitlab.com`, `gitlab.example.test`; And each entry keeps the existing 5 keys (`host, provider, validity, expires_at, scopes`, `git_auth.rs:383-390`) and gains exactly two: `"pr_read"` and `"pr_write"` booleans; And `github.com` -> `pr_read==true, pr_write==true`; `gitlab.com` -> `pr_read==false, pr_write==false`; `gitlab.example.test` -> `pr_read==true, pr_write==true`.

#### E2E-NEW-784 — EdgeCase — P1.** Seed `gitlab.com` with scopes `["api"]` only (no `read_repository`).

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-332.
When `git.pr_get`, then `git.pr_diff`, then `git.pr_review{verdict:"comment", body:"ok"}` with all routes canned 200.
Then all three succeed; And `git.auth_status` reports `pr_read==true, pr_write==true`. (`api` subsumes `read_api`: no test may require both.)

#### E2E-NEW-785 — Failure — P0.** Seed `github.com` with scopes `["repo"]` and `expires_at = Utc::now() - 1h`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then `ERR_UNAUTHENTICATED`; message starts with `token expired for host github.com` and contains `authenticate with git.auth or git.token_set for host github.com` (byte-compatible with `store.rs:245-248` and its test at `store.rs:817-840`); And no calls. (Expiry wins over the scope check; a token both expired and narrow reports expiry.)

##### Security

#### E2E-NEW-807 — SideEffect — the purge is scoped to the deleted volume only**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-NEW-276, FR-NEW-283.
- **Preconditions:** as E2E-NEW-806 with two projects, `proj1` and `proj2`, both FX-FORK, both members of `owner@test.com`, sharing one PostgreSQL database.
- **Given** both volumes hold a paused rebase: `SELECT volume_id FROM git_operations ORDER BY volume_id` returns exactly `["proj1","proj2"]`.
- **When** `admin.delete_project {"project_id":"proj1"}`.
- **Then** `SELECT volume_id FROM git_operations` returns exactly `["proj2"]` — one row, `proj2`'s.
- **And** the surviving row's `op_type == "rebase"`, `current_step == 1`, `total_steps == 2` and `conflicts` holding exactly one element whose `path` is `/a.txt`, unchanged from before the delete (field-by-field compare against the row read beforehand, ignoring `updated_at`).
- **And** `git.status {mount_id:"proj2"}` still reports `operation.op_type == "rebase"`.
- **And** `git.rebase_continue {mount_id:"proj2", resolutions:[{path:"/a.txt",strategy:"theirs"}]}` completes normally, proving the neighbouring delete did not disturb `proj2`.
- **Verification:** relational row list; field compare; tool responses.
- **Cleanup:** delete `proj2`; drop the schema. **Priority:** P0.

---

#### E2E-NEW-816 — EdgeCase — the granted scopes are persisted verbatim from the flow**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-MOD-109, FR-NEW-330, FR-NEW-335.
- **Preconditions:** Band D fixture, `MockDeviceFlowClient` returning a token response whose `scope` field is exactly `"api read_repository write_repository"`, person `dev@test.com`, host `gitlab.example.test`.
- **When** `git.auth {mount_id:"acme-self"}` runs the device flow to completion.
- **Then** the recorded authorization request's `scope` parameter is exactly `"api read_repository write_repository"` (space separated, `api` first).
- **And** `OAuthTokenStore` holds, for `(dev@test.com, gitlab.example.test)`, `scopes == ["api","read_repository","write_repository"]` in that order.
- **And** `git.auth_status` reports that entry with `"scopes":["api","read_repository","write_repository"]` and `"pr_capable": true`.
- **And** a following `git.pr_list {mount_id:"acme-self"}` issues the request without any scope pre-check rejection (`mock.calls().len() == 1`).
- **Verification:** recorded request field equality; token store read; `git.auth_status` JSON; mock call count.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-817 — Failure — a token carrying only the old scope pair is refused before the network**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-MOD-109, FR-NEW-332.
- **Preconditions:** Band D fixture; seed `(dev@test.com, gitlab.com)` with token `glpat_TESTTOKEN_old_005`, `scopes == ["read_repository","write_repository"]` (the pre-change constant, a known and known-insufficient set).
- **When** `git.pr_get {mount_id:"acme-lab", pr_number:42}`.
- **Then** assert error `ERR_FORBIDDEN` whose message contains `"api"`, `"gitlab.com"`, `"git.auth"` and `"git.token_set"`.
- **And** `mock.calls().is_empty()` — no request was issued, so the mock's `599 UNROUTED` default was never reached either.
- **And** the stored token is untouched: `scopes` still `["read_repository","write_repository"]`, token value unchanged.
- **And** the error message contains neither `"glpat"` nor the token value.
- **Verification:** error code + four substrings; mock call list empty; token store read; substring absence.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-851 — Failure — a lowered `git.max_rebase_todo` is honoured and named**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-216, FR-NEW-331.
- **Preconditions:** `Env::build` with `c.git.max_rebase_todo = 5`; `feature` holds 6 commits `S1..S6` off `C1`; `main` = `C1` + `C2`.
- **When** `git.rebase {onto:"main", todo:[pick S1 .. pick S6]}` (6 entries).
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"5"` and `"6"` — both the configured limit and the supplied length.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-S6>"` (unchanged) and no `git_operations` row exists.
- **When** the first entry is dropped from the todo, leaving 5, and the range is adjusted to start at `S2`'s parent.
- **Then** the rebase succeeds — the bound is a count check on the list, not a repository-size check.
- **Verification:** error code + two substrings; ref read; relational count; second-call success.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-880 — EdgeCase — a configured `git.gitlab_scope` overrides the default**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-331, FR-NEW-330.
- **Preconditions:** Band D fixture built with `git.gitlab_scope: "api"` and `git.github_scope: "repo public_repo"`; `MockDeviceFlowClient` recording the authorization request.
- **When** `git.auth {mount_id:"acme-lab"}` runs the GitLab flow, and `git.auth {mount_id:"acme-api"}` runs the GitHub flow.
- **Then** the recorded GitLab request's `scope` is exactly `"api"` (not the three-scope default) and the GitHub request's `scope` is exactly `"repo public_repo"`.
- **And** the stored token for `(dev@test.com, gitlab.com)` has `scopes == ["api"]`.
- **And** `git.pr_list {mount_id:"acme-lab"}` passes the scope pre-check (`mock.calls().len() == 1`), because `api` alone satisfies the PR surface's requirement.
- **Verification:** two recorded request fields; token store read; mock call count.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-881 — EdgeCase — with no configuration the defaults are exactly the documented strings**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-331, FR-MOD-109.
- **Preconditions:** Band D fixture with neither `git.github_scope` nor `git.gitlab_scope` set in the YAML.
- **When** both device flows are run.
- **Then** the recorded GitHub `scope` is exactly `"repo"` and the recorded GitLab `scope` is exactly `"api read_repository write_repository"` — character for character, including the ordering.
- **And** `ServerConfig` parsed from that YAML reports `git.github_scope == "repo"` and `git.gitlab_scope == "api read_repository write_repository"` (the defaults live in the config type, so a caller reading config sees the same values the flow sends).
- **And** the config round-trips: serializing and re-parsing it yields the same two strings.
- **Verification:** recorded request fields; config struct fields; serialize/parse round-trip.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-928 — EdgeCase — the scope error names the host and both remedy tools**

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-333.
Existing angles: 776 and 777 both assert the rejection occurs. This one pins the exact actionable text and its safety.
- **Preconditions:** Band D; token for `(dev@test.com, gitlab.example.test)` seeded with `scopes == ["read_repository"]` and token value `glpat_TESTTOKEN_narrow_006`.
- **When** `git.pr_merge {mount_id:"acme-self", pr_number:9, strategy:"squash"}`.
- **Then** assert error `ERR_FORBIDDEN` whose message contains all four of `"api"`, `"gitlab.example.test"`, `"git.auth"` and `"git.token_set"`.
- **And** the message contains neither `"glpat"` nor `"glpat_TESTTOKEN_narrow_006"` nor `"read_repository"`-prefixed token material — naming the *missing* scope is required, echoing the token is not.
- **And** `mock.calls().is_empty()`.
- **And** the same assertion holds for `git.pr_review`, `git.pr_diff` and `git.pr_get` on the same mount, so the message is uniform across the family.
- **Verification:** four error assertions; substring presence and absence; mock call list.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-929 — EdgeCase — an unknown-scope token that the provider accepts works end to end**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
Existing angles: 781 and 782 both assert the call is attempted. This one carries it through to a success, which is the point of the asymmetry.
- **Preconditions:** Band D; `git.token_set` seeds `(dev@test.com, github.com)` with token `ghp_PAT_no_scopes_007` and **no** scope information (`scopes == []`). Mock routes for `GET /repos/acme/api/pulls?state=open&per_page=100` -> `200` with two PR payloads.
- **When** `git.pr_list {mount_id:"acme-api"}`.
- **Then** the call succeeds and returns exactly two normalized entries with `number` `11` and `12`, both `state == "open"`.
- **And** `mock.calls().len() == 1` and its `authorization_seen == "token ghp_PAT_no_scopes_007"` — the unknown scope set was given the benefit of the doubt and the token was actually used.
- **And** replacing the route with a `403` body `{"message":"Resource not accessible by personal access token"}` makes the call error with a message containing that provider text verbatim, asserted in the same test: the provider's refusal is surfaced, not pre-empted or reworded.
- **Verification:** success response; call count and recorded authorization; contrasting error with verbatim provider message.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-930 — EdgeCase — `auth_status` reports an empty scope set as unknown, not as insufficient**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-335, FR-NEW-334.
Existing angles: 775 and 783 both cover tokens with known scopes. This one covers the unknown case the status screen must not misreport.
- **Preconditions:** Band D with three seeded tokens for `dev@test.com`: `github.com` with `["repo"]`, `gitlab.com` with `["read_repository","write_repository"]`, and `github.ibm.com` seeded through `git.token_set` with `scopes == []`.
- **When** `git.auth_status {}`.
- **Then** the entry for `github.com` reports `"scopes":["repo"]` and `"pr_capable": true`.
- **And** the entry for `gitlab.com` reports `"pr_capable": false` and a `"missing_scopes":["api"]` field.
- **And** the entry for `github.ibm.com` reports `"scopes":[]` and `"pr_capable": null` — explicitly unknown, distinguishable from both `true` and `false`, matching the FR-NEW-334 asymmetry.
- **And** no entry contains any token value: the serialized response contains neither `"ghp_"` nor `"glpat"`.
- **Verification:** three entry objects asserted field by field; `null` vs `false` distinguished by `Value::is_null()`; substring absence.
- **Cleanup:** fixture drop. **Priority:** P1.


## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not log, trace or return a token or a scope-bearing header. Do not pre-emptively refuse a token whose stored scope set is empty or unknown; attempt it and surface the provider's answer.

### Scope Boundary
Scope request, storage, validation and reporting. The provider client itself is US-024.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
Existing tokens keep working for clone, push, fetch and pull. Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
