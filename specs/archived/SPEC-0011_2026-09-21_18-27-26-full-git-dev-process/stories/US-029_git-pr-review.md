# US-029: git.pr_review

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 29
> Depends On: US-027
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Submit a review verdict on a pull request: approve, request changes, or comment.

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

### FR-NEW-312 [EARS-E]: Post a review
> WHEN `git.pr_review` is called with `pr_number` and `verdict` THE mcp-fs server SHALL submit that review to the provider.

- **Inputs:** `mount_id`, `pr_number`, `verdict` (`approve`|`request_changes`|`comment`), `body` (string, optional).
- **Business Rules:** `request_changes` and `comment` require a non-empty `body`; `approve` does not. GitLab maps `approve` to its approve endpoint and the other two to a merge-request note.
- **Priority:** Must-have

### FR-NEW-313 [EARS-O]: An unknown pull request number is not found
> IF `pr_number` matches no pull request THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` naming the number and the repository.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

17 tests: Failure 9, Happy 5, Integration 2, SideEffect 1.
Scenarios covered: SC-926, SC-927, SC-928.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-728 — Failure — P0.** 404 `{"message":"Not Found","documentation_url":"https://docs.github.com/rest"}`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `acme/api` and `github.com` and `Not Found`.

#### E2E-NEW-738 — Failure — P0.** `/repos/acme/api/pulls/9999` -> 404 `{"message":"Not Found"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `pull request 9999` and `acme/api`; And exactly 1 call (no reviews/checks fan-out after a 404).

#### E2E-NEW-739 — Failure — P0.** `/projects/acme%2Fapi/merge_requests/9999` -> 404 `{"message":"404 Not found"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `merge request 9999`; And exactly 1 call.

#### E2E-NEW-740 — Failure — P1.** `pr_number:0`, then `pr_number:-1`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then both `ERR_INVALID_ARGUMENT` with message containing `pr_number must be a positive integer`; And no calls in either.

#### E2E-NEW-747 — Failure — P1.** 404 on both providers.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313, FR-NEW-309.
Then `ERR_NOT_FOUND`; message contains `42`.

#### E2E-NEW-763 — Happy — P0.** `POST /repos/acme/api/pulls/42/reviews` -> 200 `{"id":7001,"state":"APPROVED","user":{"login":"dev"},"body":""}`; then get routes for the normalized return.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
When `git.pr_review{mount_id:"acme-api", pr_number:42, verdict:"approve"}`.
Then POST body exactly `{"event":"APPROVE"}` (no `body` key when none supplied); And result `review_state=="approved"`.

#### E2E-NEW-764 — Happy — P0.** verdict `request_changes`, body `"Please split the merge logic out of the tool layer."`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then POST body exactly `{"event":"REQUEST_CHANGES","body":"Please split the merge logic out of the tool layer."}`; And canned response `{"id":7002,"state":"CHANGES_REQUESTED"}` normalizes to `review_state=="changes_requested"`.

#### E2E-NEW-765 — Happy — P0.** verdict `comment`, body `"Nit: typo in the doc comment."`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then POST body exactly `{"event":"COMMENT","body":"Nit: typo in the doc comment."}`; And `review_state` from the subsequent get is `"review_required"` (a comment is not an approval).

#### E2E-NEW-766 — Happy — P0.** `POST /projects/acme%2Fapi/merge_requests/42/approve` -> 201 `{"id":42,"iid":42,"state":"opened"}`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then exactly one POST to `/projects/acme%2Fapi/merge_requests/42/approve` with body `null` or `{}` (assert the recorded body is `{}`); And `review_state=="approved"` from the follow-up approvals read.

#### E2E-NEW-767 — SideEffect — P0.** Routes `POST .../42/unapprove` -> 201 `{}`, `POST .../42/notes` -> 201 `{"id":9001,"body":"Please split the merge logic out of the tool layer."}`.

**Category:** SideEffect. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
When verdict `request_changes` with that body on `acme-lab`.
Then calls in exactly this order: `POST /projects/acme%2Fapi/merge_requests/42/unapprove`, then `POST /projects/acme%2Fapi/merge_requests/42/notes` with body exactly `{"body":"Please split the merge logic out of the tool layer."}`; And result `review_state=="changes_requested"` (GitLab has no such state natively; this is the normalization under test).

#### E2E-NEW-768 — Happy — P0.** verdict `comment` on `acme-lab`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then exactly ONE call, `POST .../42/notes` (no `unapprove`, no `approve`).

#### E2E-NEW-769 — Failure — P1.** verdict `"lgtm"`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then `ERR_INVALID_ARGUMENT`; message contains `verdict must be one of approve, request_changes, comment`; And no calls.

#### E2E-NEW-770 — Failure — P1.** verdict `request_changes` with `body` omitted, then with `body:"   "`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then both `ERR_INVALID_ARGUMENT` with message containing `verdict 'request_changes' requires a non-empty body`; And no calls in either.

#### E2E-NEW-771 — Failure — P1.** POST reviews -> 422 `{"message":"Unprocessable Entity","errors":["Can not approve your own pull request"]}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-311, FR-NEW-312.
Then `ERR_INVALID_ARGUMENT`; message contains `Can not approve your own pull request`.

#### E2E-NEW-772 — Failure — P0.** `POST .../42/approve` -> 401 `{"message":"401 Unauthorized"}` in the GitLab self-approval case, plus a second sub-case 403 `{"message":"Members can not approve their own merge request"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-311, FR-NEW-312.
Then the 403 case is `ERR_FORBIDDEN` with message containing `Members can not approve their own merge request`; And the 401 case is `ERR_UNAUTHENTICATED` with a message containing `gitlab.com` and containing `re-authenticate with git.auth` (a provider 401 means the credential is no longer accepted, and the caller must be told the remedy); And neither message contains `glpat_`.

##### OAuth scope extension and validation

#### E2E-NEW-796 — Integration — P0.** GitHub route queues: branch pre-flight 200; `POST /pulls` 201 `GH_PR_42_OPEN`; `GET /pulls/42` queued twice (first the open payload, then the merged payload of E2E-NEW-735); `/pulls/42/reviews` -> `[]` then `GH_REVIEWS_APPROVED`; `/commits/9f1c.../check-runs` -> `GH_CHECKS_SUCCESS`; `POST /pulls/42/reviews` -> 200 `{"id":7001,"state":"APPROVED"}`; `PUT /pulls/42/merge` -> 200 merged.

**Category:** Integration. **Scenario:** SC-928. **Requirements:** FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312.
When, in order: `pr_create` -> `pr_get` -> `pr_review{approve}` -> `pr_merge{squash}`.
Then create returns `state=="open"`, `review_state=="review_required"`, `checks_state=="success"`; And the first `pr_get` returns `state=="open"`, `review_state=="review_required"`; And after the review, `review_state=="approved"`; And merge returns `state=="merged"`; And the recorded call sequence method+path matches, in order: `GET /repos/acme/api/branches/feature/pr-tools`, `POST /repos/acme/api/pulls`, `GET /repos/acme/api/pulls/42`, `GET /repos/acme/api/pulls/42/reviews`, `GET /repos/acme/api/commits/9f1c.../check-runs`, `POST /repos/acme/api/pulls/42/reviews`, `GET /repos/acme/api/pulls/42`, `PUT /repos/acme/api/pulls/42/merge`.

#### E2E-NEW-797 — Integration — P0.** The GitLab mirror of E2E-NEW-796 on `acme-lab` (`POST /merge_requests`, `GET /merge_requests/42`, `GET .../approvals`, `POST .../approve`, `PUT .../merge` with `{"squash":true}`).

**Category:** Integration. **Scenario:** SC-928. **Requirements:** FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312.
Then each of the four tool results has EXACTLY the same key set and key order as its GitHub counterpart from E2E-NEW-796 (assert pairwise on the four result objects, as in E2E-NEW-709); And the state transitions match: `open -> open -> approved -> merged`.

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
