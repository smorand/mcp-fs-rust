# US-028: git.pr_merge with provider refusals surfaced

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 28
> Depends On: US-027
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Merge a pull request through the provider with a chosen strategy, surfacing the provider's refusals faithfully rather than reporting a success that did not happen.

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

2 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-310 [EARS-E]: Merge a pull request
> WHEN `git.pr_merge` is called with `pr_number` and `strategy` THE mcp-fs server SHALL ask the provider to merge it using that strategy.

- **Inputs:** `mount_id`, `pr_number`, `strategy` (`merge`|`squash`|`rebase`), `commit_title` (optional), `commit_message` (optional).
- **Business Rules:** The merge happens on the provider. Local refs and remote-tracking refs are **not** updated; the response states that a `git.remote_fetch` is required to observe it locally.
- **Priority:** Must-have

### FR-NEW-311 [EARS-O]: Provider merge refusals are surfaced faithfully
> IF the provider refuses the merge because required checks fail, required reviews are missing, the branch is protected, the strategy is disabled for that repository, or the pull request is already merged or closed THEN THE mcp-fs server SHALL surface the provider's status and message, and SHALL NOT report success.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

19 tests: EdgeCase 1, Failure 12, Happy 5, SideEffect 1.
Scenarios covered: SC-925, SC-926, SC-928.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-718 — Failure — P0.** Branch pre-flight 200; `POST /repos/acme/api/pulls` -> 422 `{"message":"Validation Failed","errors":[{"message":"A pull request already exists for acme:feature/pr-tools."}]}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `A pull request already exists for acme:feature/pr-tools.` and contains `github.com`; And 2 calls.

#### E2E-NEW-719 — Failure — P0.** `POST .../merge_requests` -> 409 `{"message":["Another open merge request already exists for this source branch: !42"]}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Another open merge request already exists for this source branch: !42`.

#### E2E-NEW-720 — Failure — P0.** POST -> 403 `{"message":"Resource not accessible by personal access token"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_FORBIDDEN`; message contains `Resource not accessible by personal access token`; And message contains `github.com`; And message does NOT contain `ghp_`.

#### E2E-NEW-729 — Failure — P1.** GitLab 500 `{"message":"500 Internal Server Error"}`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-311.
Then `ERR_INTERNAL_ERROR`; message contains `gitlab.com` and `500`; And message does NOT contain `glpat_`.

#### E2E-NEW-750 — Happy — P0.** `PUT /repos/acme/api/pulls/42/merge` -> 200 `{"sha":"3c0ffee1234567890abcdef1234567890abcdef1","merged":true,"message":"Pull Request successfully merged"}`; then get routes return the merged payload of E2E-NEW-735.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then the PUT body equals exactly `{"merge_method":"merge"}`; And the result normalized object has `state=="merged"` and `raw["sha"]=="3c0ffee1234567890abcdef1234567890abcdef1"`.

#### E2E-NEW-751 — Happy — P0.** As E2E-NEW-750 with `strategy:"squash"`. Then PUT body `{"merge_method":"squash"}`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

#### E2E-NEW-752 — Happy — P0.** As E2E-NEW-750 with `strategy:"rebase"`. Then PUT body `{"merge_method":"rebase"}`; And exactly one PUT (GitHub needs no pre-rebase call).

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

#### E2E-NEW-753 — Happy — P0.** `PUT /projects/acme%2Fapi/merge_requests/42/merge` -> 200 `GL_MR_42_OPEN` with `"state":"merged"`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When strategy `merge` on `acme-lab`. Then PUT body exactly `{"squash":false}`; And result `state=="merged"`.

#### E2E-NEW-754 — Happy — P0.** Same with `squash`. Then PUT body exactly `{"squash":true}`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

#### E2E-NEW-755 — SideEffect — P0.** Routes: `POST /projects/acme%2Fapi/merge_requests/42/rebase` -> 202 `{"rebase_in_progress":true}`; `GET /projects/acme%2Fapi/merge_requests/42?include_rebase_in_progress=true` -> 200 `{"rebase_in_progress":false,"merge_error":null}`; `PUT .../merge` -> 200 merged payload.

**Category:** SideEffect. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When strategy `rebase` on `acme-lab`.
Then calls in exactly this order: `POST .../rebase`, `GET ...?include_rebase_in_progress=true`, `PUT .../merge`; And PUT body `{"squash":false}`; And result `state=="merged"`.

#### E2E-NEW-756 — Failure — P1.** `strategy:"fast-forward"`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
Then `ERR_INVALID_ARGUMENT`; message contains `strategy must be one of merge, squash, rebase`; And no calls.

#### E2E-NEW-757 — Failure — P0.** PUT -> 405 `{"message":"Merge commits are not allowed on this repository.","documentation_url":"..."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Merge commits are not allowed on this repository.` and contains `strategy 'merge'`; And the tool did NOT silently retry with another strategy (exactly 1 PUT recorded).

#### E2E-NEW-758 — Failure — P0.** PUT -> 405 `{"message":"Required status check \"ci/build\" is expected."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Required status check "ci/build" is expected.`

#### E2E-NEW-759 — Failure — P0.** GitLab PUT -> 405 `{"message":"405 Method Not Allowed"}` and the MR get shows `"merge_status":"cannot_be_merged","head_pipeline":{"status":"failed"}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `405 Method Not Allowed` and contains `gitlab.com`.

#### E2E-NEW-760 — Failure — P1.** GitHub PUT -> 409 `{"message":"Head branch was modified. Review and try the merge again.","sha":"..."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Head branch was modified. Review and try the merge again.`

#### E2E-NEW-761 — Failure — P0.** GitHub PUT -> 403 `{"message":"4 of 4 required status checks are expected. Protected branch rules not met."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_FORBIDDEN`; message contains `Protected branch rules not met.`; And no `ghp_` substring in the message.

#### E2E-NEW-762 — EdgeCase — P0.** GitHub PUT -> 405 `{"message":"Pull Request is not mergeable"}` and the pre-merge `GET /repos/acme/api/pulls/42` returns the merged payload (`merged:true`).

**Category:** EdgeCase. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `pull request 42 is already merged`; And the PUT was NEVER sent (the pre-read short-circuits): recorded calls contain exactly the GET.

##### `git.pr_review`

#### E2E-NEW-779 — Failure — P0.** Same `["read_api","read_repository"]` token.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-lab", pr_number:42, strategy:"squash"}`.
Then `ERR_FORBIDDEN`; message contains `missing scope 'api' required by git.pr_merge`; And message does NOT offer `read_api` as a remedy; And no calls.

#### E2E-NEW-780 — Failure — P0.** Seed `github.com` with scopes `["public_repo"]`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then `ERR_FORBIDDEN`; message contains `token for host github.com is missing scope 'repo' required by git.pr_merge`; And no calls.

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
