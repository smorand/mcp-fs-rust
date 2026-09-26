# US-024: Provider client seam, base URL resolution and transport safety

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 24
> Depends On: US-023
> Complexity: M
> min_tier: 2
> Files touched: 4

## Objective

Build the seam every pull-request tool calls through: provider and API base URL resolved from the remote's host, an injectable client a test can replace, and the transport rules that keep a token from leaking.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/git/provider/mod.rs
crates/mcp-fs/src/git/provider/github.rs
crates/mcp-fs/src/git/provider/gitlab.rs
crates/mcp-fs/src/config.rs
```

### Existing Patterns
`reqwest` is already used for outbound HTTP by the device flow (`tools/git_auth.rs:39`). The injectable-client pattern to copy is the device flow's client trait plus `MockDeviceFlowClient` in the `git_auth` tests.
URL validation, host resolution and the credential pipeline are `crates/mcp-fs/src/git/remote.rs` (`validate_remote_url` `:246-262`, `extract_host` `:693`, audit wrapper `:326-371`).

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-906:** The pull request model is normalized across providers, with a `raw` passthrough field. **Rationale:** the tool contract is golden-file frozen, so it must describe one stable shape; a caller writes one handler for GitHub and GitLab. The `raw` field means normalization never loses provider-specific information. **Alternatives considered:** pass each provider's native JSON straight through, rejected because the frozen contract could not meaningfully describe two shapes under one tool name. **Implemented by:** FR-NEW-303. **Round:** 2 (approach exploration, Fork 3). **Code evidence:** n/a, new surface.

### Applicable NFRs

- 7.2 Security: no cross-host redirect is followed by the provider client.
- 7.1 Performance: provider REST calls carry `git.provider_api_timeout_secs` (default 30).

### Bounded Context
**Provider collaboration** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

6 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-300 [EARS-E]: Provider and API base resolve from the remote host
> WHEN any `git.pr_*` tool is called THE mcp-fs server SHALL resolve the target provider and REST API base URL from the host of the volume's remote URL, using the existing `git.hosts` map and the stored `instance_url`.

- **Business Rules:** Resolution goes through the existing host map (`git.hosts`), which is what makes GitHub Enterprise Server and self-hosted GitLab work with no new configuration. GitHub Enterprise uses the host's `/api/v3` base; gitlab uses `/api/v4` on the instance URL.
- **Priority:** Must-have

### FR-NEW-301 [EARS-O]: Unsupported providers are rejected
> IF the resolved provider is `generic` or `anonymous` THEN THE mcp-fs server SHALL reject every `git.pr_*` tool with `ERR_NOT_SUPPORTED`, naming the host and stating that pull-request operations require a `github` or `gitlab` provider.

- **Priority:** Must-have

### FR-NEW-302 [EARS-O]: A volume with no usable remote is rejected
> IF the volume has no remote, or the selected remote's URL cannot be parsed THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` before any network call.

- **Priority:** Must-have

### FR-NEW-314 [EARS-UB]: No token ever leaves through a PR tool
> The mcp-fs server SHALL NOT include any token value in a `git.pr_*` response, in the `raw` payload, in any error message, in any log line, or in any tracing span field.

- **Business Rules:** The `raw` passthrough is scrubbed of request authorization data before being returned.
- **Priority:** Must-have

### FR-NEW-315 [EARS-E]: The provider client is injectable
> WHEN the server is assembled THE mcp-fs server SHALL obtain its provider REST client through an abstraction that a test can replace with a fake returning canned provider responses.

- **Business Rules:** Mirrors the existing device-flow client seam used by `MockDeviceFlowClient`. Without this the entire PR surface is untestable offline.
- **Priority:** Must-have

### FR-NEW-316 [EARS-UB]: The provider client does not follow cross-host redirects
> The mcp-fs server SHALL NOT follow an HTTP redirect issued by a provider API to a host other than the resolved API host, and SHALL fail the call instead.

- **Business Rules:** Following a redirect would replay the Authorization header to an attacker-chosen host.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

14 tests: EdgeCase 3, Failure 2, Security 8, SideEffect 1.
Scenarios covered: SC-907, SC-925, SC-926.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-706 — Failure — P0.** Extra mount `acme-sh` with origin `https://git.sourcehut.test/acme/api.git`, host NOT in `git.hosts`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_get{mount_id:"acme-sh", pr_number:42}`.
Then `ERR_INVALID_ARGUMENT`; message contains `host 'git.sourcehut.test' is not declared in git.hosts` (same shape as `tools/git.rs:820-825`); And no calls.

#### E2E-NEW-708 — Failure — P1.** Mount `acme-scp` with origin `git@github.com:acme/api.git`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-302.
When `git.pr_list{mount_id:"acme-scp", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `remote url 'git@github.com:acme/api.git'` and `scp`; And no calls.

#### E2E-NEW-722 — SideEffect — P0.** After the successful E2E-NEW-710 create.

**Category:** SideEffect. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then the audit sink (`state.safety` capped audit) holds exactly one entry for this call whose fields include `op=="git.pr_create"`, `mount=="acme-api"`, `person=="dev@test.com"`, `host=="github.com"`, `provider=="github"`, `pr_number==42`; And the serialized entry does NOT contain `ghp_TESTTOKEN_github_001` nor the substring `ghp_`.

##### `git.pr_list`

#### E2E-NEW-786 — Security — P0.** Every one of the 6 tools, GitHub and GitLab (12 invocations), each routed to 401 whose body deliberately echoes the credential: `{"message":"Bad credentials: ghp_TESTTOKEN_github_001"}` / `{"message":"401 Unauthorized for glpat_TESTTOKEN_gl_003"}`.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then each call returns `ERR_UNAUTHENTICATED`; And for each, `err.message` does NOT contain `ghp_TESTTOKEN_github_001`, `glpat_TESTTOKEN_gl_003`, nor the prefixes `ghp_` / `glpat_` (redaction applied to the provider body, same duty as `remote.rs:292-303`); And each message still contains the host and a remedy.

#### E2E-NEW-787 — Security — P0.** Install a `tracing_subscriber` test layer capturing every span name and every field value (string-rendered) for the duration.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
When the 12 invocations of E2E-NEW-786 run, plus 12 successful invocations.
Then no captured field value and no span name contains `ghp_` or `glpat_`; And the spans that exist are named `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` and carry fields `mount`, `person`, `host`, `provider` with the expected values.

#### E2E-NEW-788 — Security — P0.** After the 24 invocations of E2E-NEW-787.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then every audit entry serialized to JSON is scanned: none contains `ghp_`, `glpat_`, or the literal seeded token values; And an entry exists per invocation (no silent audit gap on the failure path).

#### E2E-NEW-791 — Security — P0.** Seed two tokens on the SAME host: `dev@test.com`/`github.com` -> `ghp_TESTTOKEN_github_001`, and `alice@test.com`/`github.com` -> `ghp_ALICE_777`. Both are members of `acme-api`.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
When `dev@test.com` calls `git.pr_list{mount_id:"acme-api"}`, then `alice@test.com` calls the same.
Then the first recorded request's `authorization_seen == "token ghp_TESTTOKEN_github_001"`; And the second's `== "token ghp_ALICE_777"`; And neither request ever carried the other value (assert across all recorded calls).

#### E2E-NEW-792 — Security — P0.**

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-315.
When one successful call per provider.
Then the GitHub request's `authorization_seen == "token ghp_TESTTOKEN_github_001"` and the GitLab request's `== "Bearer glpat_TESTTOKEN_gl_003"`; And the header is present on EVERY recorded request including the branch pre-flight and the sub-calls of `pr_get` (assert on all of them, so no sub-call is issued unauthenticated).

#### E2E-NEW-794 — Security — P0.** `GET /repos/acme/api/pulls/42` -> 302 with header `Location: https://evil.test/repos/acme/api/pulls/42`. No route registered for the evil host.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-316.
When `git.pr_get{mount_id:"acme-api", pr_number:42}`.
Then `ERR_INTERNAL_ERROR`; message contains `refused redirect from host 'api.github.com' to 'evil.test'`; And exactly 1 recorded call, whose `base_url` is `https://api.github.com` (the credential was never re-sent to the redirect target); And no recorded call has `base_url` containing `evil.test`.

#### E2E-NEW-795 — Security — P1.** `GET /repos/acme/api/pulls` (list) -> 200 with a 64 MiB JSON body.

**Category:** Security. **Scenario:** SC-926. **Requirements:** FR-NEW-315, FR-NEW-316.
When `git.pr_list{mount_id:"acme-api", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `response from host 'github.com' exceeds the 16 MiB limit`; And the tool returns within 10 seconds (assert with `tokio::time::timeout`), proving the body was capped rather than fully buffered and parsed.
(Note: `pr_diff` has its own 5 MiB truncation contract, E2E-NEW-745; the JSON cap is a hard refusal because a half-read JSON document cannot be parsed.)

##### Lifecycle integration and contract

#### E2E-NEW-902 — EdgeCase — `remote_list` resolves host and provider for an enterprise host**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-144, FR-NEW-300.
Existing angles: 471 (Happy, one remote), 480 (EdgeCase, empty). This one covers the resolved `host`/`provider` fields.
- **Preconditions:** fixture config with `git.hosts` mapping `github.ibm.com: github`, `gitlab.example.test: gitlab`, `git.acme.internal: generic`; remotes added: `ent` -> `https://github.ibm.com/acme/api.git`, `self` -> `https://gitlab.example.test/acme/api.git`, `plain` -> `https://git.acme.internal/acme/api.git`.
- **When** `git.remote_list {"mount_id":"gitproj"}`.
- **Then** the response holds three entries whose `(name, host, provider)` triples are exactly `("ent","github.ibm.com","github")`, `("self","gitlab.example.test","gitlab")`, `("plain","git.acme.internal","generic")`.
- **And** each entry's `url` is the exact string supplied, unmodified (no normalization, no `.git` stripping).
- **And** the entries are ordered deterministically (by `name`), asserted as an exact array.
- **Verification:** exact array equality on the response.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-923 — EdgeCase — an unsupported provider is rejected before any token is read**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-301, FR-NEW-332.
Existing angles: 704 and 705 both assert the rejection on the two unsupported provider kinds. This one asserts the *ordering* relative to the token lookup.
- **Preconditions:** Band D, mount `acme-generic` (`git.acme.internal` -> `generic`); **no** token seeded for `dev@test.com` on that host.
- **When** `git.pr_create {mount_id:"acme-generic", base:"main", head:"feature/x", title:"t"}`.
- **Then** assert error `ERR_NOT_SUPPORTED` containing `"git.acme.internal"`, `"github"` and `"gitlab"`.
- **And** the error is **not** `ERR_FORBIDDEN` or an authentication error, proving the provider check precedes both the token lookup and the scope check.
- **And** `mock.calls().is_empty()` — not even the head-branch pre-flight was issued.
- **And** the same call on `acme-pub` (`anonymous` provider) errors identically with `"public.example.org"`.
- **Verification:** error code and substrings; code inequality assertions; mock call list; second mount.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-924 — EdgeCase — a remote whose URL has no project path**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-302.
Existing angles: 707 (no origin at all), 708 (scp shorthand). This one covers a parseable URL with no usable slug.
- **Preconditions:** Band D fixture plus a mount `acme-bare` whose origin is `https://github.com/` (host declared, path empty), and a mount `acme-one` whose origin is `https://github.com/acme` (one path segment, no repository).
- **When** `git.pr_list {mount_id:"acme-bare"}` then `git.pr_list {mount_id:"acme-one"}`.
- **Then** both assert error `ERR_INVALID_ARGUMENT` whose message contains the offending URL and the word `"repository"`.
- **And** `mock.calls().is_empty()` for both — the parse failure precedes any network call.
- **And** neither message contains `"ghp_"`.
- **Verification:** two error assertions; mock call list; substring absence.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-927 — Security — a same-host redirect is followed, a cross-host one is not**

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-316.
Existing angles: 794 (cross-host 302 refused), 795 (large body). This one pins the permitted case so the rule is "cross-host", not "no redirects".
- **Preconditions:** Band D, mount `acme-api`. Mock route queue for `GET /repos/acme/api/pulls/42`: first response `301` with header `Location: https://api.github.com/repositories/99/pulls/42`; the route `GET /repositories/99/pulls/42` returns `200` with a full payload. Supporting reviews/check-runs routes return `200`.
- **When** `git.pr_get {mount_id:"acme-api", pr_number:42}`.
- **Then** the call succeeds and `number == 42`.
- **And** `mock.calls()` shows the redirected request was issued against `base_url == "https://api.github.com"` with `path == "/repositories/99/pulls/42"`.
- **And** in the same test, replacing the `Location` with `https://evil.test/repos/acme/api/pulls/42` makes the call assert error containing `"evil.test"` and `"redirect"`, and `mock.calls()` records **no** request to `evil.test` and no `authorization_seen` value for that host.
- **Verification:** success response; recorded call path; contrasting error assertion; recorded-call scan for the foreign host.
- **Cleanup:** fixture drop. **Priority:** P0.


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
