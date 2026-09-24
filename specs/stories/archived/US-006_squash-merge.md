# US-006: Squash merge

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 6
> Depends On: US-002
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective

Add the squash option to `git.merge`: one commit carrying the cumulative difference of the source branch, with the source's individual commits absent from the target's history.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The existing three-way merge is `crates/mcp-fs/src/tools/git.rs:1835-1839` (libgit2 `merge_commits` + `MergeOptions::file_favor`); its refusal path is `crates/mcp-fs/src/tools/git.rs:1844-1849`.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git object store** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

1 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-191 [EARS-E]: Squash merge produces one single-parent commit
> WHEN `git.merge` is called with `squash` `true` THE mcp-fs server SHALL create exactly one commit with exactly one parent holding the cumulative difference of the source ref, and the source ref's individual commits SHALL NOT appear in the target branch's history.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

5 tests: EdgeCase 1, Failure 1, Happy 1, SideEffect 2.
Scenarios covered: SC-906.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-506 — Squash merge

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** Happy. **Priority:** P0.
**Preconditions:** SEED-FF extended: feature has two commits — `Cf1` adds `/docs/readme.md` = `hello\n`, `Cf2` adds `/docs/guide.md` = `guide\n`. main at `C0`.
**When** `git.merge {source_ref:"feature", squash:true}`.
**Then** `status == "merged"`, `squashed == true`, one `merge_commit` sha returned.
**And** both `/docs/readme.md` (`hello\n`) and `/docs/guide.md` (`guide\n`) are present in the volume with exactly those bytes.
**Verification:** two reads; `squashed` field.
**Cleanup:** fixture drop.

#### E2E-NEW-541 — squash given a non-boolean

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** Failure. **Priority:** P2.
**When** `git.merge {source_ref:"feature", squash:"yes"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `squash`.
**And** no commit created, no row.
**Verification:** error code + substring; log length; relational count.

#### E2E-NEW-543 — squash commit has exactly 1 parent

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-506.
**Then** `repo.find_commit(merge_commit).parent_count() == 1` exactly, and `parent_id(0) == C0` (the pre-merge target tip).
**Verification:** as above.

#### E2E-NEW-544 — squash omits the source's individual commits

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-506 (feature has `Cf1`, `Cf2`).
**Then** `git.log {ref_name:"main"}` contains neither `Cf1` nor `Cf2` sha, and has exactly 2 entries (`C0`, squash commit).
**And** the squash commit's tree contains both `/docs/readme.md` and `/docs/guide.md` — the cumulative diff is present even though the commits are not.
**Verification:** log sha set assertion; `tree().get_path()` for both files.

#### E2E-NEW-844 — EdgeCase — `squash:true` on a fast-forwardable merge commits instead of fast-forwarding**

**Category:** EdgeCase. **Scenario:** SC-906. **Requirements:** FR-NEW-193, FR-NEW-191.
- **Preconditions:** SEED-FF (`main` = `C0`; `feature` = `C0` + one commit adding `/docs/readme.md` = `hello\n`).
- **When** `git.merge {source_ref:"feature", squash:true}`.
- **Then** `status == "merged"` (not `"fast_forward"`) and `merge_commit` is a 40-hex string different from `feature`'s tip.
- **And** that commit has exactly 1 parent, equal to `<sha-C0>`.
- **And** `db.get_ref("refs/heads/main").target` equals `merge_commit`, not `feature`'s tip.
- **And** `/docs/readme.md` reads exactly `b"hello\n"`.
- **And** `git.log {ref_name:"main"}` does not contain `feature`'s original commit sha.
- **Verification:** status field; `parent_count()`/`parent_id(0)`; ref read; byte compare; log sha scan.
- **Cleanup:** fixture drop. **Priority:** P1.

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
