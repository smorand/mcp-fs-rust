# US-030: Tool registration, frozen contract and count assertions

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 30
> Depends On: US-029
> Complexity: M
> min_tier: 2
> Files touched: 4

## Objective

Register every new tool, regenerate the frozen contract, and update each hardcoded count assertion. Two of the seven count sites must deliberately not change.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/all.rs
crates/mcp-fs/src/tools/contract_golden.rs
tool-contract-golden.json
TOOL_CONTRACT.txt
```

### Existing Patterns
The frozen contract is regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current` (`crates/mcp-fs/src/tools/contract_golden.rs:46`). Count assertions live at `contract_golden.rs:131`, `tools/all.rs:78`, `:122`, `:141` and `crates/mcp-fs/src/tools/git.rs:2500`.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-916:** Tests are numbered from `E2E-NEW-400` rather than from 001. **Rationale:** the archived specification already owns `E2E-NEW-001` through `E2E-NEW-247`, and 93 of those exist as live Rust test function names; reusing them would make traceability ambiguous between two specifications. **Implemented by:** n/a, numbering convention. **Round:** 4. **Code evidence:** 93 `async fn e2e_new_NNN` functions under `crates/`, maximum id 247.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Volume** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

4 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-345 [EARS-E]: Every new tool enters the frozen contract
> WHEN the tools of this specification are registered THE mcp-fs server SHALL include each of them in the frozen tool contract, regenerated through `MCPFS_REWRITE_TOOL_CONTRACT=1`, so that `TOOL_CONTRACT.txt` and `tool-contract-golden.json` describe every name, description and `inputSchema`.

- **Priority:** Must-have

### FR-NEW-346 [EARS-E]: Every hardcoded tool count is updated
> WHEN the tool count changes THE mcp-fs server's test suite SHALL be updated at exactly these five sites: `crates/mcp-fs/src/tools/contract_golden.rs:131`, `crates/mcp-fs/src/tools/all.rs:78`, `:122`, `:141`, and `crates/mcp-fs/src/tools/git.rs:2500`.

- **Business Rules:** Five sites, counted directly. Two further count assertions exist and MUST NOT be touched: `crates/mcp-fs/src/tools/all.rs:64` asserts the admin-only registry (10 tools, git absent) and `:99` asserts the git-disabled registry (52 tools). Neither changes when git tools are added, and editing either breaks a passing test. This specification adds exactly **31** tools, enumerated in FR-NEW-349. The frozen contract count moves from 63 to **94** (`contract_golden.rs:131`), the git-family total from 18 to **49** (`all.rs:78`, currently `10 + 14 + 4`), and the `git.*` registry count from 14 to **45** (`git.rs:2500`).
- **Priority:** Must-have

### FR-NEW-349 [EARS-U]: The tool set added by this specification is exactly these 31 names
> The mcp-fs server SHALL register exactly the following 31 new tools and no others: `git.branch_create`, `git.branch_switch`, `git.branch_delete`, `git.branch_reset`, `git.stash_save`, `git.stash_list`, `git.stash_apply`, `git.stash_pop`, `git.stash_drop`, `git.remote_add`, `git.remote_remove`, `git.remote_list`, `git.merge`, `git.merge_resolve`, `git.merge_abort`, `git.rebase`, `git.rebase_continue`, `git.rebase_abort`, `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort`, `git.reset`, `git.revert`, `git.revert_continue`, `git.revert_abort`, `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review`.

- **Business Rules:** All 31 register through `git::register` (the PR tools are implemented in a new `git_pr.rs` but registered from that one entry point), so `crates/mcp-fs/src/tools/git.rs:2500` asserts 45 and its `ALL_GIT_TOOLS` literal becomes `[&str; 45]`. This list is the authoritative count. Every count assertion in FR-NEW-346 derives from it: 63 + 31 = 94 frozen contract tools, 18 + 31 = 49 git-family tools, 14 + 31 = 45 `git.*` tools excluding the four auth tools.
- **Priority:** Must-have

### FR-MOD-108 [EARS-E]: The git index owns a fourth table
> WHEN the git index declares the tables it owns THE mcp-fs server SHALL list four tables rather than three.

- **Original behavior:** `pub const TABLES: [&str; 3] = ["git_objects", "git_refs", "git_remotes"]` (`crates/mcp-fs/src/git/db.rs:42`).
- **New behavior (EARS):** the constant includes `git_operations`.
- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

15 tests: DataIntegrity 1, EdgeCase 3, Failure 1, Happy 1, Integration 7, SideEffect 2.
Scenarios covered: SC-904, SC-912, SC-925, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-551 — git_operations row created on conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-275, FR-MOD-108., FR-NEW-188
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**Then** exactly 1 row for this `volume_id` with `op_type == "merge"`, `state == "conflicted"`, `current_step == null`, `total_steps == null` (single-step operations report null per FR-NEW-186), and a conflict set deserializing to exactly `["/f/000.txt","/f/001.txt","/f/002.txt"]` (sorted).
**Verification:** `SELECT op_type, state, current_step, total_steps, conflicts FROM git_operations WHERE volume_id = ?`, field-by-field equality.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-554 — the row is scoped by volume_id

**Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277, FR-MOD-108.
**Category:** SideEffect. **Priority:** P0.
**Preconditions:** two projects `proj-a`, `proj-b` in the **same** relational store, both SEED-CONFLICT, OWNER a member of both.
**When** `git.merge` conflicts on `proj-a` only.
**Then** `SELECT COUNT(*) WHERE volume_id = <a>` == 1 and `WHERE volume_id = <b>` == 0.
**And** `git.commit {mount_id:"proj-b", message:"ok"}` **succeeds**, proving the guard does not leak across volumes (the `volume_id`-in-every-WHERE rule).
**Verification:** two relational counts; successful commit sha returned.
**Cleanup:** abort on `proj-a`.

#### E2E-NEW-692 — Registry count and names**

**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-346.
- **When** `register(&mut ToolRegistry::new())`.
- **Then** `r.len() == 45` (14 today at `git.rs:2500`, plus the 31 tools enumerated in FR-NEW-349, which includes `git.revert_continue` and `git.revert_abort`). The test asserts the number matching the shipped `ALL_GIT_TOOLS` array, which must be updated in the same change.
- **And** every new name resolves through `r.resolve(name)`, including its underscore form (`git_rebase_continue`) per the dot/underscore-tolerant resolver (AGENTS.md, `mcp/registry.rs`).
- **Priority:** P0.

---

#### E2E-NEW-693 — Frozen tool contract**

**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345.
- **When** the golden contract test runs (`tools/contract_golden.rs`).
- **Then** `tool-contract-golden.json` contains each new tool with its exact `description` and serialized `inputSchema`, and `TOOL_CONTRACT.txt` documents each new tool's parameters and return shape.
- **And** the test fails if any parameter description differs by one character (the contract is frozen; regeneration only via `MCPFS_REWRITE_TOOL_CONTRACT=1`).
- **And** the header count in `AGENTS.md`/`TOOL_CONTRACT.txt` (`63 tools`) is updated to match.
- **Priority:** P0.

---

#### E2E-NEW-695 — `git_operations` rows are volume-scoped**

**Category:** DataIntegrity. **Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277, FR-MOD-108.
- **Preconditions:** two projects `proj1` and `proj2`, both members of `OWNER`, each with FX-FORK conflicting seeded independently.
- **When** a rebase is paused in `proj1` **and** a cherry-pick is paused in `proj2`.
- **Then** `SELECT ... WHERE volume_id='proj1'` returns exactly 1 row with `op_type='rebase'`, and `volume_id='proj2'` exactly 1 row with `op_type='cherry_pick'`.
- **And** `git.rebase_abort {mount_id:"proj1"}` leaves `proj2`'s row present and its branch tip and volume untouched.
- **And** `git.rebase_continue {mount_id:"proj2"}` fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"` — `proj2` has a cherry-pick, not a rebase.
- **Priority:** P0.

---

#### E2E-NEW-799 — EdgeCase — P0.** The frozen contract tests (`tools/contract_golden.rs`, regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1`).

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-345, FR-NEW-346.
Then the frozen-contract registry asserted at `crates/mcp-fs/src/tools/contract_golden.rs:131` has `len() == 94` (63 today, plus the 31 tools of FR-NEW-349, of which these 6 are the PR family). Note the enabled full registry is a different number, asserted separately at `crates/mcp-fs/src/tools/all.rs:122`; And each of `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` appears in `tool-contract-golden.json` with a `mount_id` required string parameter (the family convention); And `git.pr_list.inputSchema.properties.state.description` names exactly `open, closed, merged, all`; And `git.pr_merge.strategy` names exactly `merge, squash, rebase`; And `git.pr_review.verdict` names exactly `approve, request_changes, comment`; And `TOOL_CONTRACT.txt` contains the same six entries.

---

##### D.z Two decisions the implementer must not re-open silently

1. **Missing scope is `ERR_FORBIDDEN`, absent/expired/provider-rejected credential is `ERR_UNAUTHENTICATED`** (E2E-NEW-776 vs E2E-NEW-785/E2E-NEW-772). Both are checked before the network; only the code differs, and clients branch on it.
2. **A failed sub-call in `pr_get` is an error, never a degraded field** (E2E-NEW-741). Reporting `checks_state:"none"` when the checks endpoint returned 403 would make a merge decision on invented data.

#### E2E-NEW-806 — Happy — deleting a project removes its paused-operation row**

**Category:** Happy. **Scenario:** SC-929. **Requirements:** FR-NEW-276, FR-MOD-108, FR-NEW-284.
- **Preconditions:** the opt-in PostgreSQL suite (`docker-compose.test.yml`, `.agent_docs/testing.md`), `infra.git.backend = "postgres"`, project `proj1` seeded with FX-FORK and member `owner@test.com`; `ADMIN` is platform admin.
- **Given** `git.rebase {mount_id:"proj1", onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}` returns `status == "conflict"`, and `SELECT COUNT(*) FROM git_operations WHERE volume_id = 'proj1'` returns `1`.
- **When** `admin.delete_project {"project_id":"proj1"}` is called as `ADMIN`.
- **Then** the call succeeds.
- **And** `SELECT COUNT(*) FROM git_operations WHERE volume_id = 'proj1'` returns `0`.
- **And** `SELECT COUNT(*) FROM git_refs WHERE volume_id = 'proj1'` returns `0` and the same for `git_objects` and `git_remotes` — the new table is purged by the same loop, not by a special case.
- **And** `crate::git::db::TABLES.len() == 4` and `TABLES.contains(&"git_operations")` (compile-time constant asserted in the same test, so a regression to `[&str; 3]` fails the build at the `len()` assertion).
- **Verification:** direct `RelationalDb` counts on the shared PostgreSQL database; constant assertion.
- **Cleanup:** drop the PostgreSQL schema created by the fixture. **Priority:** P0.

---

#### E2E-NEW-808 — EdgeCase — on SQLite the index file carries the rows away with it**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-276, FR-MOD-108.
- **Preconditions:** default SQLite build, `Fixture::with_config(|c| c.git.enabled = true)`, project `gitproj` seeded like the shipped `deleting_a_project_purges_its_git_state_when_git_is_enabled` test (`crates/mcp-fs/src/tools/admin.rs:864-881`), FX-FORK content.
- **Given** a paused rebase exists and `config.git_db_path("gitproj").exists()` is `true`.
- **When** `admin.delete_project {"project_id":"gitproj"}` as `ADMIN`, then `admin.create_project {"project_id":"gitproj","owner":"owner@test.com"}` — the same id recreated.
- **Then** `config.git_db_path("gitproj").exists()` is `false` immediately after the delete (the `-wal` and `-shm` siblings too).
- **And** after the recreate, `git.status {mount_id:"gitproj"}` carries **no** `operation` (the object named by FR-NEW-281) key — the recreated project does not inherit the old paused rebase.
- **And** `git.rebase_continue {mount_id:"gitproj", resolutions:[{path:"/a.txt",strategy:"ours"}]}` errors `ERR_INVALID_ARGUMENT` containing `"no rebase is in progress"`.
- **And** the conformance suite entry (`crates/mcp-fs/src/storage/conformance.rs:441`) is extended so the same purge loop is asserted to clear a seeded `git_operations` row on **every** engine under test, not only PostgreSQL.
- **Verification:** filesystem existence checks; JSON key absence; error assertion; conformance suite run.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-810 — EdgeCase — `on_conflict` is absent from the frozen schema**

**Category:** EdgeCase. **Scenario:** SC-912. **Requirements:** FR-DEL-101, FR-NEW-345.
- **When** the registry is built and `r.resolve("git.remote_pull").unwrap().schema` is read.
- **Then** `schema["properties"]` has exactly the key set `["mount_id","branch","remote"]` and `"on_conflict"` is absent.
- **And** `tool-contract-golden.json`'s entry for `git.remote_pull` contains no `on_conflict` and `TOOL_CONTRACT.txt` mentions the string `on_conflict` zero times.
- **And** the symbols `ConflictStrategy` and `parse_on_conflict` resolve nowhere under `crates/` (compile failure if reintroduced; asserted by their absence from the golden contract's regenerated output).
- **Verification:** JSON key-set equality on the registered schema; golden contract text search.
- **Cleanup:** none. **Priority:** P0.

#### E2E-NEW-931 — Integration — every hardcoded tool-count site reports the new numbers**

**Category:** Integration. **Scenario:** SC-925. **Requirements:** FR-NEW-346, FR-NEW-345.
Existing angles: 692 (registration count for the 8 in-progress-guard tools), 799 (frozen contract). This one sweeps the seven enumerated sites together.
- **Preconditions:** a full registry built by `register_all`.
- **When** the counts are read at each site named by FR-NEW-346.
- **Then** `contract_golden.rs:131`'s expected total is `94` and matches `registry.len()`.
- **And** `all.rs:78`, `:122` and `:141` each assert the total or the family subtotal they own, recomputed from the registry rather than restated, summing to `94`; `all.rs:64` (admin-only, 10) and `:99` (git-disabled, 52) are unaffected and unchanged.
- **And** `git.rs:2500`'s `git.*` family count is `45`, equal to `registry.names().filter(|n| n.starts_with("git.")).count()`.
- **And** `tool-contract-golden.json` holds exactly `94` entries and `TOOL_CONTRACT.txt` documents exactly `94` tool names, with the git-family subset being exactly the 49 the registry exposes (set equality, so a name present in one and not the other fails).
- **Verification:** registry counts; golden JSON array length; contract text name extraction and set comparison.
- **Cleanup:** none. **Priority:** P0.

---

#### E2E-NEW-960 — The 31 enumerated names are exactly the new tool set**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-349, FR-NEW-345
- **Steps:**
  - Given the 18 tool names shipped before this specification and the 31 names enumerated in FR-NEW-349
  - Then the registered git-family set equals the union of the two, with no extra and none missing
  - And the union has exactly 49 members, and `reg.len()` is exactly 94
  - And every one of the 31 has an entry in `tool-contract-golden.json` carrying a required string `mount_id`, except `git.stash_list` and `git.remote_list` which still require `mount_id` per the family convention
- **Verification:** set equality against a literal list of the 31 names; registry length; golden-file lookup per name.
- **Cleanup:** none. **Priority:** P0

#### E2E-NEW-961 — Registering an unlisted git tool fails the enumeration test**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-349
- **Steps:**
  - Given a registry with a 32nd new tool `git.rogue` registered
  - When the set equality of E2E-NEW-960 is evaluated
  - Then it fails, naming `git.rogue` as present but not enumerated
- **Verification:** the assertion's failure message; proves the enumeration is a real gate and not a comment.
- **Cleanup:** none. **Priority:** P1

#### E2E-MOD-405 — tool name list and count at `crates/mcp-fs/src/tools/git.rs:2480-2504`**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-346.
- **Validated before:** the git family registers exactly 14 tools, whose names are listed literally, with `assert_eq!(r.len(), 14)` at `:2500`.
- **Validates now:** `git::register` registers exactly 45 tools; the 31 names introduced by this specification are added to the `ALL_GIT_TOOLS` literal, which becomes `[&str; 45]`.
- **Given** a registry built with `git.enabled = true`.
- **When** the git family is registered.
- **Then** `r.len() == 45`.
- **And** the resolved name set equals the literal list, extended with `git.branch_create`, `git.branch_switch`, `git.branch_delete`, `git.branch_reset`, `git.stash_save`, `git.stash_list`, `git.stash_apply`, `git.stash_pop`, `git.stash_drop`, `git.merge`, `git.merge_resolve`, `git.merge_abort`, `git.rebase`, `git.rebase_continue`, `git.rebase_abort`, `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort`, `git.reset`, `git.revert`, `git.remote_add`, `git.remote_remove`, `git.remote_list` and the pull-request family, exactly as frozen in `TOOL_CONTRACT.txt`.
- **Cleanup:** none.
- **Priority:** P0.

---

#### E2E-MOD-407 — tool count assertions in `crates/mcp-fs/src/tools/all.rs` (`:78`, `:122`, `:141` only)**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-346.
- **Validated before:** five hardcoded totals over the registered tool surface, computed against 63 tools with a 14-tool git family.
- **Validates now:** the same five assertions against the new totals, 94 tools with a 49-tool git family, including the git-disabled variants where the git family contributes zero.
- **Given** registries built in each configuration the five assertions cover (git enabled, git disabled, search enabled, search disabled).
- **When** `register_all` runs.
- **Then** each of the five counts equals its updated value, and each remains internally consistent with the family-level counts asserted next to it.
- **Cleanup:** none.
- **Priority:** P0.

---

#### E2E-MOD-408 — frozen contract count at `crates/mcp-fs/src/tools/contract_golden.rs:131`**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-346.
- **Validated before:** the frozen contract contains exactly 63 tools.
- **Validates now:** exactly 94.
- **Given** the regenerated `tool-contract-golden.json`.
- **When** the golden contract test runs.
- **Then** the tool count is 94 and every name, description and serialized `inputSchema` matches the golden byte for byte, key order included.
- **Cleanup:** none.
- **Priority:** P0.

---

## Constraints

### Files Not to Touch
`crates/mcp-fs/src/tools/all.rs:64` (admin-only registry, 10) and `:99` (git-disabled registry, 52) MUST NOT be edited: neither count moves when git tools are added, and changing either breaks a passing test. `crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not hand-edit `tool-contract-golden.json` or `TOOL_CONTRACT.txt`; regenerate them and review the diff.

### Scope Boundary
Registration, the frozen contract and the count assertions. Documentation parity is US-031.

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
