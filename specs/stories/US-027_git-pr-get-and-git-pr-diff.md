# US-027: git.pr_get and git.pr_diff

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 27
> Depends On: US-025
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Read a pull request for review: its detail including review and check state, and its unified diff.

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

3 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-307 [EARS-E]: Get a pull request with review and check state
> WHEN `git.pr_get` is called with `pr_number` THE mcp-fs server SHALL return the normalized model enriched with its commits, its changed files, its `review_state` and its `checks_state`.

- **Business Rules:** `review_state` is one of `approved`, `changes_requested`, `review_required`, `none`. `checks_state` is one of `success`, `failure`, `pending`, `none`. Both are normalized across the providers' different underlying concepts (GitHub reviews plus check runs; GitLab approvals plus pipelines).
- **Priority:** Must-have

### FR-NEW-308 [EARS-O]: A failed enrichment sub-call is surfaced, not degraded
> IF the request for check or review state fails THEN THE mcp-fs server SHALL fail `git.pr_get` with the provider's error rather than returning a model with those fields silently set to `none`.

- **Business Rules:** A silently degraded "no failing checks" reading is worse than an error, because a caller would merge on it.
- **Priority:** Must-have

### FR-NEW-309 [EARS-E]: Get a pull request diff
> WHEN `git.pr_diff` is called with `pr_number` THE mcp-fs server SHALL return the unified diff for that pull request.

- **Business Rules:** The response is bounded by `git.max_pr_diff_mb` (default 12); a larger diff is truncated with an explicit `truncated: true` marker rather than streamed unbounded into the caller's context.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

14 tests: EdgeCase 6, Failure 3, Happy 5.
Scenarios covered: SC-927.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-732 — Happy — P0.** Routes of E2E-NEW-709 (GitHub side).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-307.
Then exactly 3 calls in order: `/repos/acme/api/pulls/42`, `/repos/acme/api/pulls/42/reviews`, `/repos/acme/api/commits/9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3/check-runs` (proving the head sha is taken from the first response); And `review_state=="approved"`, `checks_state=="success"`.

#### E2E-NEW-733 — Happy — P0.** Routes of E2E-NEW-709 (GitLab side).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-307.
Then exactly 2 calls: `/projects/acme%2Fapi/merge_requests/42` then `/projects/acme%2Fapi/merge_requests/42/approvals`; And `checks_state=="success"` derived from `head_pipeline.status`; And `review_state=="approved"` derived from `approved:true`.

#### E2E-NEW-734 — EdgeCase — P1.** GitHub payload with `"changed_files":0,"additions":0,"deletions":0,"commits":0`; GitLab payload with `"changes_count":"0"` and `diff_stats_summary` absent entirely.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303, FR-NEW-307.
Then both return `changed_files==0`, `additions==0`, `deletions==0`, `commits==0` (absent GitLab stats normalize to `0`, not `null`).

#### E2E-NEW-741 — Failure — P0.** get 200, reviews 200, check-runs -> 403 `{"message":"Resource not accessible by integration"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-308.
Then `ERR_FORBIDDEN`; message contains `check runs` and `Resource not accessible by integration`; And 3 calls recorded. (Explicit design decision under test: a failed sub-call is NOT degraded to `checks_state:"none"`, because silently reporting "no checks" on a merge decision is worse than failing.)

##### `git.pr_diff`

#### E2E-NEW-742 — Happy — P0.** Route `GET /repos/acme/api/pulls/42` with `accept == "application/vnd.github.v3.diff"` -> 200 body `UNIFIED_DIFF_SMALL` (content-type `text/plain`).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then exactly 1 call, and its `accept` field equals `"application/vnd.github.v3.diff"` exactly; And result `diff == UNIFIED_DIFF_SMALL` byte-for-byte; And `bytes == UNIFIED_DIFF_SMALL.len()`; And `truncated == false`.

#### E2E-NEW-743 — Happy — P0.** Route `GET /projects/acme%2Fapi/merge_requests/42/changes` -> 200

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
`{"changes":[{"old_path":"src/tools/pr.rs","new_path":"src/tools/pr.rs","new_file":true,"deleted_file":false,"renamed_file":false,"diff":"@@ -0,0 +1,3 @@\n+pub fn register() {}\n"}]}`.
Then `diff` equals exactly:
```
diff --git a/src/tools/pr.rs b/src/tools/pr.rs
new file mode 100644
--- /dev/null
+++ b/src/tools/pr.rs
@@ -0,0 +1,3 @@
+pub fn register() {}
```
(the per-file headers are synthesized so GitLab output is a valid unified diff like GitHub's); And `truncated == false`.

#### E2E-NEW-744 — EdgeCase — P1.** GitHub 200 with empty body; GitLab 200 `{"changes":[]}`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then both return `diff==""`, `bytes==0`, `truncated==false`; And no error.

#### E2E-NEW-745 — EdgeCase — P0.** GitHub 200 body of 12 MiB (`"+x".repeat()` padded to exactly 12 * 1024 * 1024 bytes).

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `truncated == true`; And `bytes == 5 * 1024 * 1024` (the declared cap); And `diff.len() == 5 * 1024 * 1024`; And `diff` is a prefix of the source body; And the result carries `"note"` containing `diff truncated at 5 MiB; fetch the branch with git.remote_fetch for the full change`.

#### E2E-NEW-746 — EdgeCase — P1.** Body containing `+// été ✅ 日本語\r\n+second\r\n`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `diff` is byte-identical including the `\r\n` pairs; And `bytes` counts BYTES not chars (assert `bytes == body.as_bytes().len()`).

#### E2E-NEW-748 — Failure — P1.** GitHub 200 with `content-type: text/html` and body `<!DOCTYPE html><html>...`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `ERR_INTERNAL_ERROR`; message contains `expected a unified diff from github.com, got content-type 'text/html'`; And the message does NOT contain `<!DOCTYPE` (no body dumping).

#### E2E-NEW-749 — EdgeCase — P2.** GitLab change entry with `"diff":"Binary files a/logo.png and b/logo.png differ\n"` and `"new_file":false`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `diff` contains the line `Binary files a/logo.png and b/logo.png differ` verbatim, preceded by `diff --git a/logo.png b/logo.png`.

##### `git.pr_merge`

#### E2E-NEW-778 — Happy — P0.** Seed `gitlab.com` with scopes `["read_api","read_repository"]`; routes of E2E-NEW-733.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-332.
When `git.pr_get{mount_id:"acme-lab", pr_number:42}`.
Then it succeeds with `number==42`; And 2 calls recorded.

#### E2E-NEW-878 — Failure — a GitLab approvals sub-call 500 fails `pr_get`**

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-308, FR-NEW-307.
- **Preconditions:** Band D, mount `acme-lab`, person `dev@test.com`. Mock routes: `GET /projects/acme%2Fapi/merge_requests/42` -> `200` with a complete MR payload whose `head_pipeline.status` is `"success"`; `GET /projects/acme%2Fapi/merge_requests/42/approvals` -> `500` body `{"message":"500 Internal Server Error"}`.
- **When** `git.pr_get {mount_id:"acme-lab", pr_number:42}`.
- **Then** assert error whose message contains `"500"` and `"approvals"`; the call is `Err`, not an `Ok` carrying `review_state: "none"`.
- **And** the response body of the failed call is not returned as a partial model: no `Ok` value is produced at all.
- **And** `mock.calls()` holds exactly the two requests, in order, so the failure was surfaced at the first failing sub-call rather than after retries.
- **And** the error message contains neither `"glpat"` nor the token value.
- **Verification:** error assertion; `is_err()`; recorded call list; substring absence.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-879 — EdgeCase — a genuinely empty check set reports `none` and succeeds**

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-308, FR-NEW-307.
- **Preconditions:** Band D, mount `acme-api`. Mock routes: `GET /repos/acme/api/pulls/42` -> `200`; `GET /repos/acme/api/pulls/42/reviews` -> `200` body `[]`; `GET /repos/acme/api/commits/<head_sha>/check-runs` -> `200` body `{"total_count":0,"check_runs":[]}`.
- **When** `git.pr_get {mount_id:"acme-api", pr_number:42}`.
- **Then** the call succeeds with `checks_state == "none"` and `review_state == "none"`.
- **And** all three requests were issued (`mock.calls().len() == 3`), so `none` here is an observed empty set, not a swallowed error — the distinction FR-NEW-308 exists to protect.
- **And** re-running the same call with the check-runs route replaced by a `500` makes it `Err`, asserted in the same test, so the two paths are contrasted directly.
- **Verification:** success response fields; call count; contrasting error assertion.
- **Cleanup:** fixture drop. **Priority:** P0.

---

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
