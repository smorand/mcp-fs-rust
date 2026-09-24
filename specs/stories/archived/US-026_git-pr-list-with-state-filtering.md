# US-026: git.pr_list with state filtering

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 26
> Depends On: US-025
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

List pull requests with state filtering, returning the normalized list item shape.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git_pr.rs
crates/mcp-fs/src/git/provider/mod.rs
```

### Existing Patterns
Tools are registered in `crates/mcp-fs/src/tools/git.rs:59-360` via `ToolSchema::new(...)`; parameter descriptions are frozen contract text.
The provider client seam from US-024 is the only route to the network.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-906:** The pull request model is normalized across providers, with a `raw` passthrough field. **Rationale:** the tool contract is golden-file frozen, so it must describe one stable shape; a caller writes one handler for GitHub and GitLab. The `raw` field means normalization never loses provider-specific information. **Alternatives considered:** pass each provider's native JSON straight through, rejected because the frozen contract could not meaningfully describe two shapes under one tool name. **Implemented by:** FR-NEW-303. **Round:** 2 (approach exploration, Fork 3). **Code evidence:** n/a, new surface.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Provider collaboration** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

1 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-306 [EARS-E]: List pull requests
> WHEN `git.pr_list` is called THE mcp-fs server SHALL return the normalized model for each pull request matching `state`.

- **Inputs:** `mount_id`, `state` (`open`|`closed`|`merged`|`all`, optional, default `open`), `remote` (optional).
- **Business Rules:** An empty result is an empty array, not an error. An unsupported `state` value is rejected before the call.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

15 tests: EdgeCase 4, Failure 5, Happy 6.
Scenarios covered: SC-925, SC-926.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-700 — Happy — P0.** Mock routes `GET /repos/acme/api/pulls` -> 200 `[]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
Given mount `acme-api` with origin `https://github.com/acme/api.git` and a valid `github.com` token.
When `git.pr_list{mount_id:"acme-api", state:"open"}`.
Then result is `{"pull_requests":[],"count":0}`; And `mock.calls()` has length 1 and equals `("GET", "https://api.github.com", "/repos/acme/api/pulls", [("state","open"),("per_page","100")], None, None)`.
Cleanup: standard.

#### E2E-NEW-701 — Happy — P0.** Same route under base `https://github.ibm.com/api/v3`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
Given mount `acme-ent`, token for `github.ibm.com` with `instance_url = "https://github.ibm.com"`.
When `git.pr_list{mount_id:"acme-ent", state:"open"}`.
Then the single recorded call has `base_url == "https://github.ibm.com/api/v3"` and `path == "/repos/acme/api/pulls"`; And result `count == 0`.

#### E2E-NEW-702 — Happy — P0.** Route `GET /projects/acme%2Fapi/merge_requests` -> 200 `[]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then recorded call `("GET","https://gitlab.com/api/v4","/projects/acme%2Fapi/merge_requests",[("state","opened"),("per_page","100")])`; And `count == 0`.

#### E2E-NEW-703 — Happy — P0.** Token for `gitlab.example.test` carries `instance_url = "https://gitlab.example.test"`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_list{mount_id:"acme-self", state:"open"}`.
Then `base_url == "https://gitlab.example.test/api/v4"`; And path as in E2E-NEW-702; And exactly 1 call.

#### E2E-NEW-704 — Failure — P0.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-301.
When `git.pr_list{mount_id:"acme-generic", state:"open"}`.
Then `err.code == "ERR_NOT_SUPPORTED"`; And `err.message` contains `host 'git.acme.internal' is declared 'generic' in git.hosts; pull request tools exist only for github and gitlab`; And `mock.calls().is_empty()`.

#### E2E-NEW-705 — Failure — P0.** Same with `acme-pub`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-301.
Then `ERR_NOT_SUPPORTED`; message contains `host 'public.example.org' is declared 'anonymous' in git.hosts`; And no calls.

#### E2E-NEW-707 — Failure — P0.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-302.
When `git.pr_list{mount_id:"acme-nor", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `volume 'acme-nor' has no origin remote` (`remote.rs:448`); And no calls.

#### E2E-NEW-723 — Happy — P0.** Route `GET /repos/acme/api/pulls?state=open` -> 200 `[GH_PR_42_OPEN, GH_PR_43_OPEN]` where 43 is 42 with `"number":43,"title":"Bump deps","head":{"ref":"chore/bump","sha":"aa11..."}`.

**Category:** Happy. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `count==2`; And `pull_requests[0]` has EXACTLY the 16 list keys of 1.5 in order (the FR-NEW-317 set minus `commits`, `changed_files`, `additions`, `deletions`, `mergeable`; `raw` IS present); And `pull_requests[0]["number"]==42`, `[1]["number"]==43`; And no `commits`, `changed_files`, `additions`, `deletions` or `mergeable` key present in a list item.

#### E2E-NEW-724 — Happy — P0.** Four calls on `acme-lab` with `state` = open, closed, merged, all; each routed to 200 `[]`.

**Category:** Happy. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then the four recorded query `state` values are exactly `"opened"`, `"closed"`, `"merged"`, `"all"`; And the equivalent GitHub run on `acme-api` records `"open"`, `"closed"`, `"closed"`, `"all"`, and for `merged` the client filters the response to items with `merged_at != null` (feed `[GH_PR_42_OPEN(merged_at:null), GH_PR_40_MERGED(merged_at:"2026-08-30T08:00:00Z")]` and assert `count==1`, `pull_requests[0]["number"]==40`, `state=="merged"`).

#### E2E-NEW-725 — EdgeCase — P1.** Both providers routed to 200 `[]`.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then both return exactly `{"pull_requests":[],"count":0}`; And `count` is the number `0`, not `null`.

#### E2E-NEW-726 — Failure — P1.** `state:"draft"`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `ERR_INVALID_ARGUMENT`; message contains `state must be one of open, closed, merged, all`; And no calls.

#### E2E-NEW-727 — EdgeCase — P1.** Page 1: 100 items, header `Link: <https://api.github.com/repos/acme/api/pulls?state=open&per_page=100&page=2>; rel="next"`. Page 2: 50 items, no Link header.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `count==50+100==150`; And exactly 2 calls, the second with query containing `("page","2")`; And numbers are contiguous 1..=150 in response order.

#### E2E-NEW-730 — EdgeCase — P1.** List on `acme-ent` with 1 item.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-300, FR-NEW-306.
Then `base_url == "https://github.ibm.com/api/v3"`; And `pull_requests[0]["host"] == "github.ibm.com"`.

#### E2E-NEW-731 — EdgeCase — P1.** List on `acme-self` with 1 item.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-300, FR-NEW-306.
Then `base_url == "https://gitlab.example.test/api/v4"`; And `pull_requests[0]["host"] == "gitlab.example.test"`.

##### `git.pr_get`

#### E2E-NEW-776 — Failure — P0.** Seed `dev@test.com`/`gitlab.com` with scopes `["read_repository","write_repository"]` (today's device-flow grant). No mock routes registered.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-332, FR-NEW-333.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then `ERR_FORBIDDEN`; message equals-contains `token for host gitlab.com is missing scope 'api' (or 'read_api') required by git.pr_list; re-authenticate with git.auth or seed a sufficient token with git.token_set`; And `mock.calls().is_empty()` (checked BEFORE the network, mirroring `store.rs:233-249`).
Design note under test: the code is `ERR_FORBIDDEN`, not `ERR_UNAUTHENTICATED`, because the credential is valid and merely too narrow; `ERR_UNAUTHENTICATED` stays reserved for absent/expired/rejected tokens.

## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not reimplement an operation in the tool layer: the engine lives in one place and the tool is a thin adapter. Do not build a `LIKE` pattern by hand; use `descendant_pattern` / `Dialect::escape_like_literal`. Do not block a request thread on the database or on libgit2.

### Scope Boundary
Only the requirements listed above. Anything else in the parent specification belongs to another story.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
