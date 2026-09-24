# US-025: git.pr_create and the normalized pull-request model

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 25
> Depends On: US-024
> Complexity: M
> min_tier: 2
> Files touched: 2

## Objective

Open a pull request on GitHub or a merge request on GitLab, and define the one normalized object shape every PR tool returns so a caller writes a single handler for both providers.

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
Provider and instance URL resolution reuses the `git.hosts` map and the stored `instance_url` (`crates/mcp-fs/src/git/oauth/store.rs:41`).

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-906:** The pull request model is normalized across providers, with a `raw` passthrough field. **Rationale:** the tool contract is golden-file frozen, so it must describe one stable shape; a caller writes one handler for GitHub and GitLab. The `raw` field means normalization never loses provider-specific information. **Alternatives considered:** pass each provider's native JSON straight through, rejected because the frozen contract could not meaningfully describe two shapes under one tool name. **Implemented by:** FR-NEW-303. **Round:** 2 (approach exploration, Fork 3). **Code evidence:** n/a, new surface.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Provider collaboration** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

4 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-303 [EARS-U]: One normalized response model across providers
> The mcp-fs server SHALL return, for every `git.pr_*` tool, a response using provider-independent field names, and SHALL populate them identically for a GitHub pull request and a GitLab merge request in the same logical state.

- **Outputs:** exactly the key set of FR-NEW-317: `provider`, `host`, `number`, `title`, `body`, `state` (`open`|`closed`|`merged`), `draft` (a separate boolean, NOT folded into `state`), `base`, `head`, `author`, `url`, `created_at`, `updated_at`, `commits`, `changed_files`, `additions`, `deletions`, `review_state`, `checks_state`, `mergeable`, and `raw` carrying the provider's untouched payload.
- **Business Rules:** The frozen tool contract describes one shape; a caller writes one handler. `raw` is the escape hatch for provider-specific fields so normalization never loses information.
- **Priority:** Must-have

### FR-NEW-304 [EARS-E]: Create a pull request
> WHEN `git.pr_create` is called with `base`, `head` and `title` THE mcp-fs server SHALL create a pull request on GitHub or a merge request on GitLab and SHALL return the normalized model.

- **Inputs:** `mount_id`, `base`, `head`, `title`, `body` (optional), `draft` (boolean, optional, default `false`), `remote` (optional, default `origin`).
- **Priority:** Must-have

### FR-NEW-305 [EARS-O]: Create fails early when the head branch is not on the remote
> IF `head` does not exist on the target remote THEN THE mcp-fs server SHALL reject `git.pr_create` with `ERR_NOT_FOUND` naming the branch and `git.remote_push` as the remedy, before issuing any create request.

- **Priority:** Must-have

### FR-NEW-317 [EARS-U]: The normalized pull-request object key set
> The mcp-fs server SHALL return, from `git.pr_create`, `git.pr_get`, `git.pr_merge` and `git.pr_review`, an object with exactly the keys `provider`, `host`, `number`, `title`, `body`, `state`, `draft`, `base`, `head`, `author`, `url`, `created_at`, `updated_at`, `commits`, `changed_files`, `additions`, `deletions`, `review_state`, `checks_state`, `mergeable`, `raw`, in that order.

- **Outputs:** `state` is one of `open`, `closed`, `merged`. `draft` is a separate boolean and is NOT folded into `state`.
- **Business Rules:** `commits` and `changed_files` are counts, `additions` and `deletions` are line counts, `mergeable` is a tri-state boolean (`true`, `false`, `null` when the provider has not computed it). `git.pr_list` returns the same object minus `commits`, `changed_files`, `additions`, `deletions` and `mergeable`. This supersedes the Section 8.4 sketch, which folded `draft` into `state` and omitted `provider` and `host`.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

18 tests: EdgeCase 6, Failure 5, Happy 4, Integration 2, Security 1.
Scenarios covered: SC-925, SC-927.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-709 — Integration — P0. The normalization proof.**

**Category:** Integration. **Scenario:** SC-927. **Requirements:** FR-NEW-303., FR-NEW-317
Preconditions: GitHub routes `/repos/acme/api/pulls/42` -> `GH_PR_42_OPEN`, `/pulls/42/reviews` -> `GH_REVIEWS_APPROVED`, `/commits/9f1c.../check-runs` -> `GH_CHECKS_SUCCESS`. GitLab routes `/projects/acme%2Fapi/merge_requests/42` -> `GL_MR_42_OPEN`, `/merge_requests/42/approvals` -> `GL_APPROVALS_APPROVED`.
When `git.pr_get{mount_id:"acme-api", pr_number:42}` -> `gh`, then `git.pr_get{mount_id:"acme-lab", pr_number:42}` -> `gl`.
Then `gh.as_object().keys().collect::<Vec<_>>() == gl.as_object().keys().collect::<Vec<_>>()` (order included); And for every key except `provider`, `host`, `url`, `raw`: `gh[k] == gl[k]`, specifically `number==42`, `title=="Add PR tools"`, `body=="Adds git.pr_* over the provider REST API."`, `state=="open"`, `draft==false`, `base=="main"`, `head=="feature/pr-tools"`, `author=="smorand"`, `created_at=="2026-09-01T10:00:00+00:00"`, `updated_at=="2026-09-02T11:30:00+00:00"`, `commits==3`, `changed_files==4`, `additions==120`, `deletions==11`, `review_state=="approved"`, `checks_state=="success"`, `mergeable==true`; And `gh["provider"]=="github"`, `gl["provider"]=="gitlab"`; And `gh["url"]=="https://github.com/acme/api/pull/42"`, `gl["url"]=="https://gitlab.com/acme/api/-/merge_requests/42"`; And `gh["raw"]["head"]["sha"]` exists while `gl["raw"]["iid"]==42` (raw is provider-shaped and NOT normalized).
Note the deliberate traps: GitLab `changes_count` is the STRING `"4"` and must normalize to the number `4`; GitLab timestamps carry `.000Z` and must normalize to the same `+00:00` form as GitHub.

##### `git.pr_create`

#### E2E-NEW-710 — Happy — P0.** Routes: `GET /repos/acme/api/branches/feature/pr-tools` -> 200 `{"name":"feature/pr-tools","commit":{"sha":"9f1c..."}}`; `POST /repos/acme/api/pulls` -> 201 `GH_PR_42_OPEN`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"feature/pr-tools", title:"Add PR tools", body:"Adds git.pr_* over the provider REST API."}`.
Then calls in order: `("GET",".../branches/feature/pr-tools")` then `("POST","https://api.github.com","/repos/acme/api/pulls")` with body exactly `{"base":"main","head":"feature/pr-tools","title":"Add PR tools","body":"Adds git.pr_* over the provider REST API.","draft":false}`; And result `number==42`, `state=="open"`, `draft==false`.

#### E2E-NEW-711 — Happy — P0.** Routes: `GET /projects/acme%2Fapi/repository/branches/feature/pr-tools` -> 200; `POST /projects/acme%2Fapi/merge_requests` -> 201 `GL_MR_42_OPEN`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When same args on `acme-lab`.
Then POST body exactly `{"source_branch":"feature/pr-tools","target_branch":"main","title":"Add PR tools","description":"Adds git.pr_* over the provider REST API."}`; And result `number==42`, `provider=="gitlab"`.

#### E2E-NEW-712 — Happy — P0.** Same as E2E-NEW-710 on `acme-ent`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304, FR-NEW-300.
Then both calls have `base_url == "https://github.ibm.com/api/v3"`; And result `host=="github.ibm.com"`.

#### E2E-NEW-713 — Happy — P1.** Title `"Ajout des outils PR — été 2026 ✅"`, body `"Résumé:\n- prise en charge des MR\n- 日本語テスト"`. Mount `acme-self`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
Then the POST body `title` and `description` are byte-identical to the inputs (no escaping, no NFC/NFD rewriting); And response echo normalizes back to the same strings.

#### E2E-NEW-714 — Failure — P0.** Route `GET /repos/acme/api/branches/feature/ghost` -> 404 `{"message":"Branch not found"}`. No POST route registered.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-305.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"feature/ghost", title:"x"}`.
Then `ERR_NOT_FOUND`; message contains `head branch 'feature/ghost' does not exist on host 'github.com'`; And `mock.calls().len() == 1` and its method/path is the GET branch pre-flight (proving no POST).

#### E2E-NEW-715 — Failure — P0.** Same with `acme-lab`, route `GET /projects/acme%2Fapi/repository/branches/feature/ghost` -> 404 `{"message":"404 Branch Not Found"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-305.
Then `ERR_NOT_FOUND`; message contains `head branch 'feature/ghost' does not exist on host 'gitlab.com'`; And exactly 1 call, the GET.

#### E2E-NEW-716 — Failure — P1.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"main", title:"x"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `base and head must differ, both are 'main'`; And no calls.

#### E2E-NEW-717 — Failure — P1.** `title: "   "`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
Then `ERR_INVALID_ARGUMENT`; message contains `title must not be empty or whitespace-only`; And no calls.

#### E2E-NEW-721 — EdgeCase — P1.** Two sub-cases in one test.

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-304, FR-NEW-303.
(a) GitHub with `draft:true`: POST body includes `"draft":true`; canned 201 payload is `GH_PR_42_OPEN` with `"draft":true`; result `draft==true`, `state=="open"`.
(b) GitLab with `draft:true`: POST body `title == "Draft: Add PR tools"` (GitLab has no draft flag) and canned 201 payload has `"title":"Draft: Add PR tools","draft":true`; result `draft==true` and `title=="Add PR tools"` (the `Draft: ` prefix is stripped in normalization, so the two providers agree).

#### E2E-NEW-735 — EdgeCase — P0.** GitHub: `"state":"closed","merged":true,"merged_at":"2026-09-03T08:00:00Z","mergeable":null`. GitLab: `"state":"merged","merge_status":"cannot_be_merged"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="merged"`; And GitHub `mergeable==null` (JSON null, not `false`).

#### E2E-NEW-736 — EdgeCase — P0.** GitHub `"state":"closed","merged":false,"merged_at":null`. GitLab `"state":"closed"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="closed"`; And neither is `"merged"` (the regression this guards: GitHub closed+merged look alike without `merged_at`).

#### E2E-NEW-737 — EdgeCase — P1.** GitHub `"draft":true,"state":"open"`. GitLab `"draft":true,"state":"opened","title":"Draft: Add PR tools"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="open"` and `draft==true`; And both `title=="Add PR tools"`.

#### E2E-NEW-777 — Failure — P0.** Same seeding, `git.pr_create{mount_id:"acme-lab", base:"main", head:"feature/pr-tools", title:"x"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-332, FR-NEW-333.
Then `ERR_FORBIDDEN`; message contains `missing scope 'api'` and `git.pr_create`; And no calls — in particular the head-branch pre-flight of E2E-NEW-714 did NOT run, proving the scope gate precedes every network call including pre-flights.

#### E2E-NEW-793 — Security — P0.** `pr_get` on both providers with the routes of E2E-NEW-709.

**Category:** Security. **Scenario:** SC-927. **Requirements:** FR-NEW-303, FR-NEW-314.
Then `result["raw"]` deep-equals the exact canned get payload (`GH_PR_42_OPEN` / `GL_MR_42_OPEN`) and nothing more; And the serialized `result` contains no key named `authorization`, `token`, `headers`, or `request`; And the full serialized result contains no `ghp_`/`glpat_` substring.

#### E2E-NEW-798 — EdgeCase — P1.** Get and list payloads whose title is `"Ajout des outils PR — été 2026 ✅"` and body/description is `"Résumé:\n- prise en charge des MR\n- 日本語テスト"` on both providers.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then `pr_get` returns those exact strings for `title` and `body` on both; And the corresponding `pr_list` item `title` matches byte-for-byte; And the string length in chars is asserted (`title.chars().count() == 32`) so a mojibake round trip fails loudly.

#### E2E-NEW-925 — EdgeCase — the head-branch pre-flight passes and the create follows, in that order**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-305, FR-NEW-304.
Existing angles: 714 and 715 are both the missing-branch failure. This one is the success path plus the call ordering.
- **Preconditions:** Band D, mount `acme-api`, token with `["repo"]`. Mock routes: `GET /repos/acme/api/branches/feature/login` -> `200` body `{"name":"feature/login","commit":{"sha":"abc123..."}}`; `POST /repos/acme/api/pulls` -> `201` with a full PR payload, `number: 7`.
- **When** `git.pr_create {mount_id:"acme-api", base:"main", head:"feature/login", title:"Add login", body:"why"}`.
- **Then** the call succeeds with `number == 7`, `state == "open"`, `base == "main"`, `head == "feature/login"`.
- **And** `mock.calls()` holds exactly two entries in this order: `("GET","https://api.github.com","/repos/acme/api/branches/feature/login")` then `("POST","https://api.github.com","/repos/acme/api/pulls")`.
- **And** the POST body is exactly `{"base":"main","head":"feature/login","title":"Add login","body":"why","draft":false}`.
- **And** a rerun with the GET route replaced by `404` issues **one** call only (the GET) and errors `ERR_NOT_FOUND` containing `"feature/login"` and `"git.remote_push"`, asserted in the same test so the ordering guarantee is visible from both sides.
- **Verification:** response fields; ordered call list; exact body equality; contrasting rerun.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-926 — Integration — one injected client serves a GitHub and a GitLab mount in the same process**

**Category:** Integration. **Scenario:** SC-925. **Requirements:** FR-NEW-315, FR-NEW-300, FR-NEW-303.
Existing angles: 792 and 795 are both Security assertions on the client. This one proves the seam itself is a real injection point across providers.
- **Preconditions:** one `MockProviderApi` instance registered via `register_with(reg, tokens, flow, Some(api.clone()))`; routes for `GET /repos/acme/api/pulls/1` (GitHub, `200`) and `GET /projects/acme%2Fapi/merge_requests/1` plus its approvals route (GitLab, `200`), with the supporting reviews/check-runs routes.
- **When** `git.pr_get {mount_id:"acme-api", pr_number:1}` then `git.pr_get {mount_id:"acme-lab", pr_number:1}`.
- **Then** both succeed and their responses have the identical key set, in the identical order, differing only in `provider` (`"github"` vs `"gitlab"`), `host` and `url`.
- **And** `mock.calls()` records the GitHub calls against `base_url == "https://api.github.com"` and the GitLab calls against `base_url == "https://gitlab.com/api/v4"`, proving base resolution flowed through the single injected client.
- **And** `mock.calls()` contains no request with `status == 599` (nothing unrouted), so no real network client was constructed as a fallback.
- **Verification:** two response key-set comparisons; recorded `base_url` values; absence of the `599` sentinel.
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
