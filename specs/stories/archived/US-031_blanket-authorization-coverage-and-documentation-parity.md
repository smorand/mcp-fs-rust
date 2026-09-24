# US-031: Blanket authorization coverage and documentation parity

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 31
> Depends On: US-030
> Complexity: M
> min_tier: 2
> Files touched: 4

## Objective

Close the cross-cutting gaps: every new tool is in the blanket authorization tests, and the documented tool surface matches the live registry.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
AGENTS.md
.agent_docs/tools.md
.agent_docs/git.md
```

### Existing Patterns
The blanket authorization test to extend is `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`crates/mcp-fs/src/tools/git.rs:2685`), and the tool-name list it walks is `crates/mcp-fs/src/tools/git.rs:2480-2493`.
Every git tool authorizes first: see the `state.authorize(mount_id, person)` pattern at `crates/mcp-fs/src/tools/git.rs:6-8` and the membership check at `crates/mcp-fs/src/tools/git.rs:386-396`.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-914:** Any project member can continue or abort an operation another member started. **Rationale:** authorization here is membership-based with no per-resource ownership, and the volume is shared; an operation only its author could clear would let one absent person block the whole project. **Implemented by:** FR-NEW-282. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:6-8` (membership-only authorization).

- **DEC-917:** Test design was performed by four independent sub-agents with fresh context, none of which wrote the requirements. **Rationale:** the author of a requirement is the worst person to test it, because the same blind spot produces both. Splitting by theme also kept each designer's output reviewable. **Implemented by:** n/a, process decision. **Round:** 4.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Volume** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

3 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-100 [EARS-E]: Universal authorization and normalization on every new git tool
> WHEN any tool introduced by this specification is called THE mcp-fs server SHALL authorize the caller against project membership for `mount_id` before performing any other work, and SHALL normalize every supplied path through `safety.normalize_path`.

- **Inputs:** `mount_id` (string, required on every tool), caller identity.
- **Outputs:** `ERR_FORBIDDEN` for a non-member, `ERR_UNAUTHENTICATED` when unidentified, `ERR_PROJECT_NOT_FOUND` for an unknown mount.
- **Business Rules:** Platform admin role confers no implicit access (`crates/mcp-fs/src/tools/git.rs:6-8`). No new tool bypasses the pattern.
- **Priority:** Must-have

### FR-NEW-347 [EARS-E]: The new tools are added to the blanket authorization tests
> WHEN the tools are registered THE mcp-fs server's test suite SHALL add each of them to the existing tests asserting that every git tool rejects a non-member and rejects a platform admin who is not a member.

- **Business Rules:** The existing test is `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`crates/mcp-fs/src/tools/git.rs:2685`). A tool absent from that list is an unguarded tool.
- **Priority:** Must-have

### FR-NEW-348 [EARS-E]: Documentation is updated with the tool surface
> WHEN the tool surface changes THE mcp-fs project SHALL update `AGENTS.md:5`, `:88`, `:155`, `.agent_docs/tools.md:1` and `:157`, and `.agent_docs/git.md`, to describe the new counts and the new tool families.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

24 tests: Failure 8, Integration 3, Security 13.
Scenarios covered: SC-901, SC-902, SC-903, SC-904, SC-907, SC-914, SC-918, SC-920, SC-924, SC-925, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-411 — Security — a non-member cannot create a branch**

**Category:** Security. **Scenario:** SC-901. **Requirements:** FR-NEW-100., FR-NEW-189
Preconditions: SEED-A.
- **When** `env.as_person("stranger@test.com", "git.branch_create", {"mount_id":"gitproj","name":"feature/evil","start_point":"<sha-C2>"})`.
- **Then** `code == ERR_FORBIDDEN` and `message.contains("stranger@test.com")`.
- **And** `db.get_ref("refs/heads/feature/evil")` is `None`.
- **And** the same assertion is repeated for `git.branch_switch`, `git.branch_delete`, `git.branch_reset` with the same person, each returning `ERR_FORBIDDEN`.
Priority: P0.

---

#### E2E-NEW-412 — Security — a platform admin who is not a member is forbidden**

**Category:** Security. **Scenario:** SC-901. **Requirements:** FR-NEW-100.
Preconditions: SEED-A. `ADMIN` is the platform admin fixture id and is not a member of `gitproj`.
- **When** `env.as_person(ADMIN, "git.branch_create", {...,"name":"feature/admin","start_point":"<sha-C2>"})`.
- **Then** `code == ERR_FORBIDDEN`.
- **And** `db.get_ref("refs/heads/feature/admin")` is `None`.
Rationale: mirrors `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`crates/mcp-fs/src/tools/git.rs:2685`); the four new branch tools must be added to that list too.
Priority: P0.

---

#### E2E-NEW-441 — Security — a non-member cannot reset or delete**

**Category:** Security. **Scenario:** SC-924. **Requirements:** FR-NEW-100.
Preconditions: SEED-B.
- **When** `env.as_person("stranger@test.com", ...)` calls `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}` and `git.branch_delete {"name":"release/1.0","force":true}`.
- **Then** both error with `code == ERR_FORBIDDEN`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
- **And** `safety.audit("stranger@test.com", MOUNT)` is empty.
Priority: P0.

---

#### E2E-NEW-468 — Security — the stash family is membership gated**

**Category:** Security. **Scenario:** SC-902. **Requirements:** FR-NEW-100.
Preconditions: E2E-NEW-446 state.
- **When** `env.as_person("stranger@test.com", ...)` calls each of `git.stash_save {"message":"x"}`, `git.stash_list {}`, `git.stash_apply {"stash_id":"<sha-S1>"}`, `git.stash_pop {"stash_id":"<sha-S1>"}`, `git.stash_drop {"stash_id":"<sha-S1>"}`.
- **Then** all five error with `code == ERR_FORBIDDEN` and `message.contains("stranger@test.com")`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some` and `env.read("/src/lib.rs") == "fn a() {}\n"`.
- **And** the same five calls as `ADMIN` also return `ERR_FORBIDDEN`.
- **And** a call with an empty `person` returns `ERR_UNAUTHENTICATED` (driven through the fixture's unauthenticated path, matching `crate::identity`).
Priority: P0.

---

#### E2E-NEW-488 — Security — remote management is membership gated**

**Category:** Security. **Scenario:** SC-907. **Requirements:** FR-NEW-100, FR-NEW-145.
Preconditions: SEED-R.
- **When** `env.as_person("stranger@test.com", ...)` calls `git.remote_add {"name":"evil","url":"https://evil.test/x.git"}`, `git.remote_remove {"name":"origin"}`, `git.remote_list {}`.
- **Then** all three error with `code == ERR_FORBIDDEN`.
- **And** `db.list_remotes()` == `[("origin", <url>)]` (no row added, none removed).
- **And** the same three calls as `ADMIN` also return `ERR_FORBIDDEN`.
Priority: P0.

---

##### D) Force push with lease

---

#### E2E-NEW-511 — git.merge without mount_id

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"feature"}` (no `mount_id`).
**Then** `ERR_INVALID_ARGUMENT`, message contains `mount_id`.
**And** `git_operations` count == 0; `refs/heads/main` unchanged.
**Verification:** error code + substring; relational count.
**Cleanup:** none.

#### E2E-NEW-512 — Non-member cannot merge

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT; `stranger@acme.test` is not a member of `proj`.
**When** `env.as_person("stranger@acme.test","git.merge",{mount_id:"proj",source_ref:"feature"})`.
**Then** `ERR_FORBIDDEN`, message contains `stranger@acme.test` and `proj` (shape per `errors.rs:176`: `ERR_FORBIDDEN: 'a@b.c' is not a member of 'p'`).
**And** no `git_operations` row; the audit log for `stranger@acme.test` on `proj` contains no `git.merge` entry.
**Verification:** error code + two substrings; `safety.audit("stranger@acme.test","proj")` is empty.
**Cleanup:** none.

#### E2E-NEW-513 — Unknown mount

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0.
**When** `git.merge {mount_id:"does-not-exist", source_ref:"feature"}` as OWNER.
**Then** `ERR_PROJECT_NOT_FOUND`, message contains `does-not-exist`.
**Verification:** error code + substring.

#### E2E-NEW-514 — Unauthenticated caller

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P1.
**When** the merge tool is invoked with an empty/absent identity.
**Then** `ERR_UNAUTHENTICATED`.
**And** no `git_operations` row for `proj`.
**Verification:** error code; relational count.

#### E2E-NEW-526 — resolution path escaping the volume

**Scenario:** SC-904. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0.
**When** `git.merge_resolve {resolutions:[{path:"../../etc/passwd", content:"root\n"}]}`.
**Then** `ERR_PATH_OUT_OF_BOUNDS` (the code `safety.normalize_path` raises, `safety.rs:89`).
**And** no file named `passwd` exists anywhere in the volume (`fs.glob "**/passwd"` returns 0 matches); op still in progress.
**Verification:** error code; glob count; relational count.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-639 — Rebase failure: non-member caller**

**Category:** Security. **Scenario:** SC-914. **Requirements:** FR-NEW-100.
- **Preconditions:** FX-FORK-CLEAN; `stranger@test.com` is not a member of `proj1` (mirrors `git.rs:2714`).
- **When** as `stranger@test.com`: `git.rebase {onto:"main", todo:[pick F1]}`, `git.rebase_continue {}`, `git.rebase_abort {}`.
- **Then** each fails `ERR_FORBIDDEN` with message `'stranger@test.com' is not a member of 'proj1'` (exact phrasing from `errors.rs:176`).
- **And** tip unchanged; zero rows.
- **And** the same three calls as a **platform admin who is not a member** also fail `ERR_FORBIDDEN` (admin gets no implicit file access, `git.rs:6-8`).
- **Priority:** P0.

---

##### B. Cherry-pick (E2E-NEW-640 .. E2E-NEW-659)

---

#### E2E-NEW-659 — Cherry-pick failure: non-member caller**

**Category:** Security. **Scenario:** SC-918. **Requirements:** FR-NEW-100.
- **When** as `stranger@test.com`: `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort` on `proj1`.
- **Then** each fails `ERR_FORBIDDEN` with `'stranger@test.com' is not a member of 'proj1'`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

##### C. Reset (E2E-NEW-660 .. E2E-NEW-674)

---

#### E2E-NEW-674 — Reset failure: non-member caller**

**Category:** Security. **Scenario:** SC-920. **Requirements:** FR-NEW-100.
- **When** as `stranger@test.com`: `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `ERR_FORBIDDEN`, `'stranger@test.com' is not a member of 'proj1'`.
- **And** tip and volume unchanged.
- **Priority:** P0.

---

##### D. Revert (E2E-NEW-675 .. E2E-NEW-689)

---

#### E2E-NEW-690 — All eight new tools require `mount_id`**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-345, FR-NEW-347.
- **Preconditions:** FX-LINE.
- **When** each of `git.rebase`, `git.rebase_continue`, `git.rebase_abort`, `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort`, `git.reset`, `git.revert` (plus `git.revert_continue`, `git.revert_abort` if the revert pair is registered) is called with its other required args but no `mount_id`.
- **Then** each fails `ERR_INVALID_ARGUMENT` naming `"mount_id"` — matching the convention that `mount_id` is required on every `git.*` tool (`git.rs:209`, `:236`, and the schema builders at `:196-199`).
- **Priority:** P0.

---

#### E2E-NEW-691 — Unknown `mount_id` on all new tools**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-347.
- **When** each new tool is called with `mount_id:"does-not-exist"` and otherwise valid args.
- **Then** each fails `ERR_PROJECT_NOT_FOUND` with message `project 'does-not-exist' not found` (exact phrasing, `errors.rs:249`).
- **And** `proj1`'s tip and volume are untouched.
- **Priority:** P0.

---

#### E2E-NEW-696 — Unauthenticated caller**

**Category:** Security. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-347.
- **Preconditions:** the server route path with no valid identity (no forwarded header, no `Authorization`; `identity.rs`).
- **When** any of the eight new tools is invoked over `/mcp`.
- **Then** the response carries `ERR_UNAUTHENTICATED` and the operation is not performed (tip unchanged).
- **Priority:** P0.

---

#### E2E-NEW-789 — Security — P0.** Caller `outsider@test.com`, who is not a member of any mount. All routes canned 200 so a leak would succeed loudly.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-100, FR-NEW-347.
When each of `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` is called on `acme-api` and on `acme-lab` (12 calls).
Then all 12 return `ERR_FORBIDDEN` with a message containing `'outsider@test.com' is not a member of 'acme-api'` / `'acme-lab'` (the shape `state.rs:61` produces, cf. `errors.rs` display test); And `mock.calls().is_empty()` after all 12 (authorization precedes host resolution, credential lookup and network).

#### E2E-NEW-790 — Security — P0.** Request with no identity (the `ToolCtx.person` empty, as `require_identity` expects, `git_auth.rs`).

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-100, FR-NEW-347.
When the same 12 calls.
Then all return `ERR_UNAUTHENTICATED`; And `mock.calls().is_empty()`.

#### E2E-NEW-877 — Failure — a non-member still cannot touch another project's operation**

**Category:** Security. **Scenario:** SC-929. **Requirements:** FR-NEW-282, FR-NEW-100.
- **Preconditions:** as E2E-NEW-876, paused; `stranger@test.com` is a member of no project; `ADMIN` is platform admin and a member of none.
- **When** `Env::as_person("stranger@test.com")` calls `git.rebase_continue {mount_id:"proj1", resolutions:[{path:"/a.txt",strategy:"ours"}]}`, then `git.rebase_abort {mount_id:"proj1"}`, then `git.status {mount_id:"proj1"}`.
- **Then** all three assert error `ERR_FORBIDDEN` — membership is checked before any operation lookup, so ownership-free does not mean access-free.
- **And** the same three calls as `ADMIN` also assert `ERR_FORBIDDEN` (platform admin confers no implicit file access, `tools/git.rs:386-396`).
- **And** none of the six calls reveals the operation type: no message contains `"rebase"`.
- **And** the `git_operations` row is unchanged field for field, and `second@test.com` can still complete it.
- **Verification:** six error assertions; substring absence; row compare; completion call.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-957 — The documented tool count matches the live registry**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-348, FR-NEW-346
- **Preconditions:** A registry built with git enabled.
- **Steps:**
  - Given the integer parsed from the tool-count claim in `AGENTS.md` and the one in `.agent_docs/tools.md`
  - Then both equal the frozen-contract length, 94
  - And both files' git-family count equals 49
- **Verification:** parse the counts from the two files at test time and compare to `reg.len()`; no hardcoded expectation beyond the registry itself.
- **Cleanup:** none. **Priority:** P1

#### E2E-NEW-958 — `.agent_docs/tools.md` lists every registered git tool**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-348, FR-NEW-349
- **Steps:**
  - Given the set of registered tool names beginning `git.`
  - Then every one of them appears verbatim in `.agent_docs/tools.md`
  - And the file names no `git.` tool that is not registered
- **Verification:** set equality between the registry and the names parsed from the document.
- **Cleanup:** none. **Priority:** P1

#### E2E-NEW-959 — A tool missing from the documentation fails the parity test**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-348
- **Steps:**
  - Given a registry with one extra tool `git.undocumented_probe` registered in the test only
  - When the parity assertion of E2E-NEW-958 runs against the real `.agent_docs/tools.md`
  - Then it fails, naming `git.undocumented_probe`
- **Verification:** the assertion's own failure message; this test asserts the guard actually catches a gap rather than passing vacuously.
- **Cleanup:** none. **Priority:** P1

#### E2E-MOD-406 — `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`git.rs:2685`)**
**Category:** Security. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-347.
- **Validated before:** a platform admin who is not a project member receives `ERR_FORBIDDEN` from each of the 14 shipped git tools.
- **Validates now:** the same, over all 45 `git.*` tools plus the 4 `git.auth*` tools. A new tool that forgets the membership gate fails this test rather than shipping.
- **Given** a project the caller is not a member of, and a caller holding the platform admin role.
- **When** every registered `git.*` tool is invoked in turn with a minimal valid argument set for that mount.
- **Then** every call errors with `code == ERR_FORBIDDEN`.
- **And** no ref, no node and no `git_operations` row is written for that volume.
- **Cleanup:** none.
- **Priority:** P0.

---

#### E2E-MOD-409 — `tool-contract-golden.json` and `TOOL_CONTRACT.txt` regeneration**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-348.
- **Validated before:** the 63-tool contract.
- **Validates now:** the 87-tool contract, regenerated rather than hand edited.
- **Given** the 31 new tools of FR-NEW-349 registered with their schemas.
- **When** `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current` is run and the diff reviewed.
- **Then** `tool_contract_golden_is_current` passes on a clean checkout of the regenerated files.
- **And** the diff adds exactly 24 tool entries and changes no existing entry; any change to a shipped schema in that diff is a defect, not an accepted regeneration.
- **Cleanup:** none.
- **Priority:** P0.

---

`e2e_new_134_a_degenerate_diverged_pull_is_a_fast_forward` is explicitly **kept unchanged**:
a divergence that turns out to be a fast-forward still fast-forwards, and the conflict model
does not touch that path.

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
