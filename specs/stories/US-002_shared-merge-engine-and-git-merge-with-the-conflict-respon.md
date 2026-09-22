# US-002: Shared merge engine and git.merge, with the conflict response contract

> Parent Spec: `specs/2026-09-21_18-27-26-full-git-dev-process.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 2
> Depends On: US-001
> Complexity: L
> min_tier: 2
> Files touched: 2

## Objective

Give the server a single, shared way to combine two histories and to report what it could not combine. This story extracts the existing pull merge into a reusable engine, defines the one conflict response shape every combining operation will return, and ships `git.merge` as its first consumer so the contract is observable and testable. This is the largest story in the backlog and deliberately so: the conflict contract has no verifiable surface until a tool exposes it.

### Carried drift from the parent specification

This story closes `DRIFT-001` from the specification's Implementation Drift Register. It is reproduced verbatim; do not start by copying the existing error path.

#### DRIFT-001: The pull path has no conflict-success response to copy
- **Spec says:** every combine operation returns `status: "conflict"` with both sides' content and applies nothing (FR-NEW-170, FR-NEW-186), and Section 12's stash tests describe this as "mirroring the pull conflict model".
- **Code does:** the current pull has no such response. When libgit2's merge still has conflicts, it returns an error: `ToolError::internal("git.remote_pull: the merge left unresolved conflicts that 'ours'/'theirs' cannot represent")` at `crates/mcp-fs/src/tools/git.rs:1844-1849`. There is no `status: "conflict"` success path anywhere in the tree.
- **Nature:** missing capability.
- **Resolution during implementation:** build the conflict-success response once, as new code, per FR-NEW-186, and route all six combine operations through it. Do not treat `git.rs:1844` as a template: it is the error path being replaced. The three-way merge itself (`git.rs:1835-1839`) and `apply_pull_changes_atomically` (`:2079-2127`) are the reusable parts; the conflict reporting is not.
- **Detected by:** `E2E-NEW-462` and every conflict test in band B fail against the current code, which returns `ERR_INTERNAL_ERROR` where the spec requires an `Ok` response carrying `status: "conflict"`.
- **Blocks which requirement:** FR-NEW-170, FR-NEW-186, and every requirement depending on the conflict model.
- **Status:** open

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
crates/mcp-fs/src/git/db.rs
```

### Existing Patterns
The existing three-way merge is `crates/mcp-fs/src/tools/git.rs:1835-1839` (libgit2 `merge_commits` + `MergeOptions::file_favor`); its refusal path is `crates/mcp-fs/src/tools/git.rs:1844-1849`.
The atomic volume write is `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:2079-2127`): pass 1 pre-checks every write target, pass 2 deletes then writes. Quota is charged before the apply by `charge_pull_quota` (`crates/mcp-fs/src/tools/git.rs:1967-1989`).
Git objects live in the blob store under `git:{sha}` and are exported to the bare repo before libgit2 reads, imported after it writes: `crates/mcp-fs/src/git/odb.rs:3-28`.
`Repository` is `Send` but not `Sync`; every libgit2 call runs on the blocking pool via `on_git_thread` (`crates/mcp-fs/src/tools/git.rs:217-232`).
The per-project write lock is `crates/mcp-fs/src/git/repo.rs:44`, taken today at `crates/mcp-fs/src/tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`. Follow those call sites: acquire it in the async caller, never inside the blocking closure.
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-901:** An in-progress operation is persisted as a row in a new relational `git_operations` table. **Rationale:** it is the only option where a paused rebase survives a server restart and is visible to every project member; the alternatives lose or hide it. **Alternatives considered:** (B) mirror real git's control files inside the bare repo at `state/git-repos/{project}/`, rejected because that directory is documented as a rebuildable cache rather than the source of truth, so a cache rebuild would destroy authoritative state; (C) in-memory session state like the read-before-write guard in `crates/mcp-fs/src/safety.rs`, rejected because a restart would evaporate a paused rebase and leave the volume half-applied with no way to continue or abort, which is data-integrity damage rather than inconvenience, and because it inherits the known BL-003/BL-014 scaling defect. **Implemented by:** FR-NEW-275, FR-NEW-276, FR-NEW-277, FR-NEW-278. **Round:** 2 (approach exploration). **Code evidence:** `crates/mcp-fs/src/git/db.rs:42` (three tables today), `crates/mcp-fs/src/git/repo.rs:44` (in-process lock), `crates/mcp-fs/src/safety.rs` (in-memory session pattern).

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

- **DEC-904:** Continue and abort tools are per operation and named after the git CLI (`git.rebase_continue`, `git.merge_abort`) rather than a single generic trio. **Rationale:** the MCP client is an LLM chat behaving like a terminal, and tool names are its primary affordance; `git rebase --continue` is vocabulary it already has. The engine underneath is shared, so this costs surface area, not duplicated logic. **Alternatives considered:** one generic `git.operation_continue` / `git.operation_abort` / `git.conflict_resolve` trio, rejected for discoverability despite being fewer tools. **Implemented by:** FR-NEW-196, FR-NEW-197, FR-NEW-221, FR-NEW-223, FR-NEW-225, FR-NEW-239. **Round:** 2 (approach exploration, Fork 2). **Code evidence:** n/a, new surface.

- **DEC-909:** A delete-versus-modify conflict, and a file-versus-directory type change, are surfaced as conflicts rather than refused up front with `ERR_INVALID_ARGUMENT`. **Rationale:** the shared model's premise is that combine operations surface rather than refuse; refusing would send the caller back to manual work for a case it can decide. **Alternatives considered:** up-front refusal, which the independent test designer flagged as the equally defensible branch. **Implemented by:** FR-NEW-180, FR-NEW-183. **Round:** 4 (raised by the test designer, resolved at test-plan review). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.4 Reliability: every volume write is all-or-nothing; a conflict leaves the volume byte-identical.
- 7.5 Observability: every destructive operation writes an audit entry.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

15 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-186 [EARS-U]: The conflict response has exactly one serialized shape
> The mcp-fs server SHALL return, for every conflict of every combine operation, an object with exactly the keys `status`, `operation`, `operation_id`, `source_ref`, `current_step`, `total_steps`, `conflicts`, `continue_with`, `abort_with`, where each `conflicts` element has exactly the keys `path`, `ours`, `theirs`, `base`, `binary`, `type_change`, and each of `ours`, `theirs` and `base` is an object with exactly `exists` (boolean) and `content` (string, `null` when `exists` is false or when `binary` is true).

- **Inputs:** none; this governs the response of `git.merge`, `git.remote_pull`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_apply` and `git.stash_pop`.
- **Outputs:** the shape above. `current_step` and `total_steps` are `null` for single-step operations (merge, remote_pull, cherry_pick, revert, stash_apply, stash_pop); a rebase ALWAYS reports integers, never null, whatever its todo length. `source_ref` is `null` where not applicable.
- **Business Rules:** In the conflict RESPONSE, `conflicts` is an array of objects and is never a bare array of path strings. Two other places legitimately hold a plain array of path strings and are not governed by this rule: the `git_operations.conflicts` column (Section 8.1), which persists the unresolved paths, and the `remaining_conflicts` field of the `operation` object that `git.status` returns (FR-NEW-281). The keys `conflicting_paths`, `resolve_with`, `target_ref` and `step` are emitted by no tool. Any test in Section 12 asserting an older form is rewritten against this shape.
- **Priority:** Must-have

### FR-NEW-187 [EARS-U]: Step indices are zero-based
> The mcp-fs server SHALL report `current_step` as the zero-based index of the paused todo entry and `total_steps` as the count of todo entries, so that `todo[current_step]` is the entry awaiting resolution.

- **Outputs:** `current_step` (integer, 0 or greater), `total_steps` (integer, 1 or greater).
- **Business Rules:** `git.status`'s `operation` object (FR-NEW-281) and the `git_operations` row (FR-NEW-275) use the same base. A resumed rebase that has completed entry index 0 and paused on entry index 1 reports `current_step` as 1.
- **Priority:** Must-have

### FR-NEW-188 [EARS-U]: The operation state column is a closed set
> The mcp-fs server SHALL store `git_operations.state` as exactly one of `conflicted` or `running`, SHALL write `conflicted` whenever an operation pauses awaiting resolution, and SHALL NOT write any other value.

- **Business Rules:** `paused` is not a valid value. Every test asserts `conflicted` for a paused operation.
- **Priority:** Must-have

### FR-NEW-199 [EARS-U]: Every tool's response key set, in one place
> The mcp-fs server SHALL return, from each tool below, an object with exactly the keys listed for it and no others.

| Tool | Response keys |
|---|---|
| `git.branch_create` | `branch`, `sha`, `checked_out` |
| `git.branch_switch` | `branch`, `sha`, `changed`, `files_changed` |
| `git.branch_delete` | `branch`, `sha`, `forced` |
| `git.branch_reset` | `branch`, `old_sha`, `new_sha`, `checked_out`, `files_changed` |
| `git.stash_save` | `stash_id`, `sha`, `message`, `base_sha`, `branch`, `created_at`, `files_stashed` |
| `git.stash_list` | `stashes` (array of `{stash_id, message, base_sha, branch, created_at}`), `count` |
| `git.stash_apply` | `stash_id`, `status`, `files_changed`, `dropped` |
| `git.stash_pop` | `stash_id`, `status`, `files_changed`, `dropped` |
| `git.stash_drop` | `stash_id`, `dropped` |
| `git.remote_add` | `name`, `url`, `host`, `provider` |
| `git.remote_remove` | `name`, `removed` |
| `git.remote_list` | `remotes` (array of `{name, url, host, provider}`), `count` |
| `git.merge` | `status`, `merge_commit`, `fast_forward`, `squashed`, `files_changed` |
| `git.merge_resolve` | `status`, `merge_commit`, `remaining_conflicts`, `resolved_count`, `files_changed` |
| `git.merge_abort` | `status`, `operation`, `restored_sha` |
| `git.rebase` | `status`, `branch`, `new_tip`, `replayed`, `dropped`, `squashed`, `current_step`, `total_steps` |
| `git.rebase_continue` | the same keys as `git.rebase` |
| `git.rebase_abort` | `status`, `operation`, `branch`, `restored_sha` |
| `git.cherry_pick` | `status`, `new_sha`, `source_sha` |
| `git.cherry_pick_continue` | the same keys as `git.cherry_pick` |
| `git.cherry_pick_abort` | `status`, `operation`, `restored_sha` |
| `git.reset` | `mode`, `old_sha`, `new_sha`, `files_changed` |
| `git.revert` | `status`, `new_sha`, `reverted_sha` |
| `git.revert_continue` | the same keys as `git.revert` |
| `git.revert_abort` | `status`, `operation`, `restored_sha` |
| `git.pr_create`, `git.pr_get`, `git.pr_merge`, `git.pr_review` | the normalized pull-request object of FR-NEW-317 |
| `git.pr_list` | `pull_requests` (array of the FR-NEW-317 object minus `commits`, `changed_files`, `additions`, `deletions`, `mergeable`), `count` |
| `git.pr_diff` | `pr_number`, `diff`, `truncated` |
| `git.remote_push` | `branch`, `remote`, `remote_branch`, `created`, `up_to_date`, `forced`, `overwritten_sha`, `new_sha` |

- **Business Rules:**
  - Any operation that can conflict returns, INSTEAD of the keys above, the conflict response of FR-NEW-186 when it conflicts. The two shapes are distinguished by `status`.
  - `status` values are closed per tool: `git.merge` is one of `merged`, `conflict`, `already_up_to_date`; `git.rebase` and `git.rebase_continue` are one of `completed`, `conflict`, `up_to_date`; `git.cherry_pick` and `git.revert` and their continuations are one of `committed`, `conflict`, `already_present`; `git.stash_apply` and `git.stash_pop` are one of `applied`, `conflict`; every abort tool returns exactly `aborted`.
  - `remaining_conflicts` is ALWAYS an array of path strings, never a count, in every tool and every object that carries it, including the `operation` object of FR-NEW-281.
  - `overwritten_sha` is `null` unless the push was forced; `merge_commit` is `null` when `status` is `already_up_to_date`; `restored_sha` is the exact pre-operation tip an abort restored.
  - The keys `commit_sha` and `parents` are emitted by no tool in this specification: a commit's parent count is asserted on the commit object read back through `git.show`, not on the operation's response.
  - This table is authoritative. Where any earlier Outputs line or any Section 12 assertion names a different key for one of these tools, this table wins and the other text is read as superseded.
- **Priority:** Must-have

### FR-NEW-286 [EARS-U]: The `git.status` operation object key set
> The mcp-fs server SHALL populate the `operation` object of FR-NEW-281 with exactly the keys `op_type`, `source_ref`, `current_step`, `total_steps`, `remaining_conflicts`, `continue_with` and `abort_with`.

- **Business Rules:** `op_type` takes the six values of FR-NEW-285. `current_step` and `total_steps` follow FR-NEW-187, so they are `null` for single-step operations and integers for a rebase. `remaining_conflicts` is an array of path strings.
- **Priority:** Must-have

### FR-NEW-241 [EARS-U]: Every pausable operation has exactly one pair of completion tools
> The mcp-fs server SHALL route the completion and abandonment of each operation type to exactly one pair of tools, as follows: `merge`, `remote_pull` and `stash_apply` to `git.merge_resolve` and `git.merge_abort`; `rebase` to `git.rebase_continue` and `git.rebase_abort`; `cherry_pick` to `git.cherry_pick_continue` and `git.cherry_pick_abort`; `revert` to `git.revert_continue` and `git.revert_abort`.

- **Business Rules:** A pull is completed by the merge tools because a pull conflict is a merge conflict, and a stash apply likewise, because applying a stash is a merge against HEAD. The conflict response's `continue_with` and `abort_with` fields always name the correct pair, so the caller never infers it (FR-NEW-170). Without this requirement an implementer must guess which tool finishes a conflicted revert.
- **Priority:** Must-have

### FR-NEW-170 [EARS-E]: One conflict response shape for every combine operation
> WHEN `git.remote_pull`, `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_apply` or `git.stash_pop` encounters a genuine content conflict THE mcp-fs server SHALL return a response carrying `status: "conflict"`, the `operation` type, the `operation_id`, and a `conflicts` array in which each element carries `path`, `ours`, `theirs` and `base`.

- **Outputs:** `ours`, `theirs` and `base` each carry the full content of that side, or an explicit null when that side deleted the path. Binary content is reported with a `binary: true` marker and omitted bytes rather than corrupted text.
- **Business Rules:** One shape, identical field names, regardless of which operation produced it. That is what lets a caller write one handler.
- **Priority:** Must-have

### FR-NEW-171 [EARS-UB]: A conflict applies nothing
> The mcp-fs server SHALL NOT write any file to the volume, create any commit, or move any ref when a combine operation ends in a conflict response.

- **Business Rules:** The volume must be byte-identical to its pre-operation state. This is stronger than "mostly unchanged" and is tested as such.
- **Priority:** Must-have

### FR-NEW-172 [EARS-UB]: Conflict markers never enter the volume
> The mcp-fs server SHALL NOT write the sequences `<<<<<<<`, `=======` or `>>>>>>>` into any volume file as a result of any operation defined in this specification.

- **Business Rules:** Retains the existing prohibition. The volume is a filesystem other tools read; a marker there would be silently interpreted as file content.
- **Priority:** Must-have

### FR-NEW-173 [EARS-E]: Auto-mergeable divergence needs no caller decision
> WHEN a combine operation finds that the two sides changed disjoint regions THE mcp-fs server SHALL complete the merge automatically, without returning a conflict response and without requiring any resolution call.

- **Priority:** Must-have

### FR-NEW-190 [EARS-E]: Merge a ref into the current branch
> WHEN `git.merge` is called with `source_ref` THE mcp-fs server SHALL merge that ref into the checked-out branch and, on success, SHALL create a commit with exactly two parents and update the volume to the merged tree.

- **Inputs:** `mount_id`, `source_ref`, `squash` (boolean, optional, default `false`), `message` (string, optional).
- **Outputs:** exactly `{status, merge_commit, fast_forward, squashed, files_changed}` per FR-NEW-199.
- **Business Rules:** Default commit message is `Merge {source_ref} into {current_branch}` when `message` is absent.
- **Priority:** Must-have

### FR-NEW-192 [EARS-O]: An already-merged source is a reported no-op
> IF `source_ref` is already an ancestor of the current branch THEN THE mcp-fs server SHALL return success with `status: "already_up_to_date"` and `merge_commit: null`, SHALL create no commit, and SHALL NOT modify the volume.

- **Priority:** Must-have

### FR-NEW-193 [EARS-O]: A fast-forwardable merge fast-forwards
> IF the current branch is an ancestor of `source_ref` and `squash` is `false` THEN THE mcp-fs server SHALL advance the branch to `source_ref` without creating a merge commit, and SHALL report `status: "merged"` with `fast_forward: true`.

- **Priority:** Should-have

### FR-NEW-194 [EARS-O]: Merge refuses a dirty volume
> IF the volume holds uncommitted changes THEN THE mcp-fs server SHALL reject `git.merge` with `ERR_INVALID_ARGUMENT` naming commit and stash as remedies, before performing any merge work.

- **Priority:** Must-have

### FR-NEW-195 [EARS-O]: An unknown source ref is not found
> IF `source_ref` resolves to no commit THEN THE mcp-fs server SHALL reject `git.merge` with `ERR_NOT_FOUND`.

- **Priority:** Must-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

36 tests: Concurrency 1, DataIntegrity 4, EdgeCase 9, Failure 5, Happy 4, SideEffect 13.
Scenarios covered: SC-903, SC-904, SC-916, SC-929.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-500 — Auto-mergeable divergence completes automatically

**Scenario:** SC-903. **Requirements:** FR-NEW-190, FR-NEW-173.
**Category:** Happy. **Priority:** P0.
**Preconditions:** SEED-AUTOMERGE.
**Given** `main` = `C2` (line 2 `port = 8000`), `feature` = `C1` (line 5 `timeout = 60`), merge base `C0`.
**When** `git.merge {mount_id:"proj", source_ref:"feature"}`.
**Then** the result JSON is exactly `status == "merged"`, `conflicts` field absent, `merge_commit` a 40-hex string, `files_changed == 1`.
**And** `env.read("/src/config.toml")` equals byte-for-byte:
```
[server]
port = 8000
host = "0.0.0.0"
workers = 4
timeout = 60
retries = 2
```
**And** no `git_operations` row exists for this `volume_id` (SQL `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` → 0).
**Verification:** JSON field equality; `VolumeClient::read_text`; direct relational query.
**Cleanup:** fixture drop.

#### E2E-NEW-501 — Merge of a strictly-ahead branch

**Scenario:** SC-903. **Requirements:** FR-NEW-193.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-FF.
**When** `git.merge {source_ref:"feature"}` from `main` at `C0`.
**Then** `status == "merged"`, `fast_forward == true`, `merge_commit` equals `feature`'s tip sha.
**And** `/docs/readme.md` reads exactly `hello\n`; `refs/heads/main` target equals the feature tip.
**And** the created commit count is unchanged: `git.log ref_name:"main"` returns exactly 2 entries.
**Verification:** `entry.db.get_ref("refs/heads/main")`, `git.log` length.
**Cleanup:** fixture drop.

#### E2E-NEW-502 — Conflict response shape

**Scenario:** SC-904. **Requirements:** FR-NEW-170., FR-NEW-186
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"feature"}`.
**Then** the response satisfies, field by field: `status == "conflict"`, `operation == "merge"`, `source_ref == "feature"`, `conflicts.len() == 1`, `conflicts[0].path == "/src/config.toml"`, `conflicts[0].ours.content` decodes to the SEED-CONFLICT main text (`port = 8000`), `conflicts[0].theirs.content` decodes to the feature text (`port = 9090`), `conflicts[0].base.content` decodes to the `C0` text (`port = 8080`), all three `exists == true`, `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`.
**And** `refs/heads/main` still targets `C2` exactly.
**And** `env.read("/src/config.toml")` still contains `port = 8000` and none of `<<<<<<<`, `=======`, `>>>>>>>`.
**Verification:** exact JSON equality against a constructed `serde_json::json!` literal for the non-content fields; decoded content string equality; ref read.
**Cleanup:** call `git.merge_abort`.

#### E2E-NEW-510 — Merging an already-fully-merged branch is a no-op

**Scenario:** SC-903. **Requirements:** FR-NEW-192.
**Category:** Happy. **Priority:** P1. **Preconditions:** SEED-FF; run `git.merge feature` once (E2E-NEW-501).
**When** `git.merge {source_ref:"feature"}` a second time.
**Then** the call **succeeds** (no `Err`), `status == "already_up_to_date"`, `merge_commit` is `null`.
**And** `refs/heads/main` target is unchanged from after the first merge.
**And** `git.log "main"` length is unchanged (no empty commit created).
**Verification:** `Result::is_ok()`, field equality, ref sha equality, log length.
**Cleanup:** fixture drop.

#### E2E-NEW-515 — Unknown source_ref

**Scenario:** SC-903. **Requirements:** FR-NEW-195.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"nope"}`.
**Then** `ERR_NOT_FOUND`, message contains `nope`.
**And** no fetch, no row, `refs/heads/main` unchanged at `C2`.
**Verification:** error code + substring; ref sha.

#### E2E-NEW-516 — Merging the checked-out branch into itself

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** SEED-CONFLICT, HEAD on `main`.
**When** `git.merge {source_ref:"main"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `main` and `itself`.
**And** no commit created (`git.log` length unchanged), no row.
**Verification:** error code + substrings; log length; relational count.

#### E2E-NEW-517 — Dirty volume refuses the merge

**Scenario:** SC-903. **Requirements:** FR-NEW-194.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, then `env.write("/src/config.toml", "[server]\nport = 1\n")` **without committing**.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `uncommitted changes` (same wording family as `git.rs:2006-2010`).
**And** `/src/config.toml` still reads exactly `[server]\nport = 1\n` — the dirty content was neither committed nor overwritten.
**And** no row, no commit.
**Verification:** error code + substring; exact file bytes; relational count; log length.

#### E2E-NEW-538 — merge on a volume with no HEAD

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** `git.init` only, no commit.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no checked-out branch` (matching the wording family at `git.rs:1500-1504`).
**Verification:** error code + substring.

#### E2E-NEW-539 — merge on a non-initialized volume

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** project seeded, `git.init` never called.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `proj` and `never initialized` (wording family at `git.rs:1546-1551`).
**Verification:** error code + substrings.

#### E2E-NEW-542 — merge commit has exactly 2 parents

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge → conflict → resolve `strategy:"ours"`.
**Then** with `oid = merge_commit`: `repo.find_commit(oid).parent_count() == 2` exactly; `parent_id(0) == C2` (target tip first); `parent_id(1) == C1` (source tip second) — the ordering `git.rs:1884` already establishes for pull.
**Verification:** libgit2 reads through the repo entry, plus `git.show` reporting `parents: [C2, C1]`.
**Cleanup:** fixture drop.

#### E2E-NEW-545 — target ref moves to the merge commit

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-503.
**Then** `entry.db.get_ref("refs/heads/main").target == merge_commit`, and the libgit2 reference `refs/heads/main` resolves to the same oid (both stores updated, as `git.rs:1891-1897` does for pull).
**Verification:** relational ref read + `repo.find_reference` read, asserted equal.

#### E2E-NEW-546 — source ref is untouched

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P1. **Preconditions:** E2E-NEW-503; snapshot `refs/heads/feature` before.
**Then** after the merge, `refs/heads/feature` still targets `C1` exactly, and the full `refs/heads/*` map differs from the snapshot in exactly one key, `refs/heads/main`.
**Verification:** `list_refs()` map diff, asserted to be a single key.

#### E2E-NEW-547 — exactly one audit entry per completed merge

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-AUTOMERGE; snapshot `safety.audit(OWNER,"proj").len()`.
**When** `git.merge {source_ref:"feature"}` succeeds.
**Then** the audit gains exactly 1 entry; its `op` is exactly `"git.merge"`, its `path` is exactly `"/"` (the convention at `git/remote.rs:363-368`), and its `detail` contains `outcome ok` and `source feature`.
**And** the detail contains no 40-hex token-like secret and no credentialed URL.
**Verification:** audit length delta; exact `op` and `path` string equality; substring checks.

#### E2E-NEW-548 — conflicted merge still audits, with outcome conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge` returns `status:"conflict"`.
**Then** exactly 1 new audit entry with `op == "git.merge"` and `detail` containing `outcome conflict` and `conflicts 1`.
**And** a subsequent successful `git.merge_resolve` adds exactly 1 more entry with `op == "git.merge_resolve"` — two operations, two rows, never one merged row.
**Verification:** audit length deltas of 1 and 1; exact `op` strings.

#### E2E-NEW-552 — row removed on full resolve

**Scenario:** SC-904. **Requirements:** FR-NEW-284., FR-NEW-188
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-551 state.
**When** all three are resolved in one call.
**Then** `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` == 0.
**Verification:** relational count.

#### E2E-NEW-553 — row removed on abort

**Scenario:** SC-904. **Requirements:** FR-NEW-284., FR-NEW-186
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-551 state.
**When** `git.merge_abort`.
**Then** count == 0.
**Verification:** relational count.

#### E2E-NEW-556 — no commit object on a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-171.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `git.log "main"` shas.
**When** `git.merge` conflicts.
**Then** `git.log "main"` returns the identical sha list; no object in `entry.objects` has a commit whose message contains `Merge`.
**Verification:** log sha vector equality; ODB scan for commit objects created after the snapshot (count == 0).

#### E2E-NEW-557 — no conflict markers after a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-172.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-CONFLICT + SEED-MULTI(3) + SEED-BINARY files all present; merge conflicts.
**Then** walking **every** file in the volume via `fs.glob "**/*"` and reading raw bytes, none contains the byte sequences `<<<<<<<`, `=======`, or `>>>>>>>`.
**And** the same holds after a partial resolve and again after `git.merge_abort`.
**Verification:** glob + `read_bytes` + `windows(7).any(...)` over each of the three literals, asserted false, at all three points in time.

#### E2E-NEW-560 — volume byte-identical after a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-171., FR-NEW-186
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-MULTI(3) plus `/keep.txt` = `k\n`.
**Given** `before: BTreeMap<String, Vec<u8>>` built from `fs.glob "**/*"` + `read_bytes`.
**When** `git.merge` conflicts.
**Then** the same map rebuilt after is `==` to `before`: same key set, same bytes, no extra file, no removed file.
**Verification:** whole-map equality assertion (prints the differing key on failure).

#### E2E-NEW-568 — empty file on one side

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-EMPTY (base `/flag` is 0 bytes; feature `on\n`, main `off\n`).
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts[0].base.exists == true`, `base.content == ""`; `ours.content == "off\n"`; `theirs.content == "on\n"`.
**When** resolved with `strategy:"theirs"`.
**Then** `/flag` reads exactly `on\n`.
**Verification:** JSON fields with explicit empty string and `0`; file read.

#### E2E-NEW-569 — 250 conflicting files

**Scenario:** SC-904. **Requirements:** FR-NEW-170, FR-NEW-275.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-MULTI(250).
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts.len() == 250` exactly, paths are `/f/000.txt … /f/249.txt` sorted ascending, and each carries all three sides.
**And** exactly **one** `git_operations` row exists (not 250).
**When** one `git.merge_resolve` supplies 250 resolutions, all `strategy:"theirs"`.
**Then** `status == "merged"`; all 250 files read exactly `line-FEATURE\n`; `git_operations` count == 0; exactly one commit created with exactly 2 parents; `bytes_written` delta == `250 * "line-FEATURE\n".len()`.
**Verification:** vector length + sorted path equality; relational counts; 250 file reads in a loop asserting equality; `parent_count()`; quota delta arithmetic.

#### E2E-NEW-573 — unrelated histories have no common ancestor

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P1.
**Preconditions:** `main` = `C0` committing `/a.txt` = `alpha\n`; an orphan branch `island` created with no parent, committing `/a.txt` = `omega\n`.
**When** `git.merge {source_ref:"island"}`.
**Then** either `ERR_INVALID_ARGUMENT` containing `unrelated histories`, or `status == "conflict"` with `conflicts[0].base.exists == false` and `base.content == null`. The test pins the conflict branch and asserts `base.exists == false` explicitly.
**And** `ours.content == "alpha\n"`, `theirs.content == "omega\n"`.
**Verification:** JSON fields including explicit `false`/`null`.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-574 — conflicting path at the length ceiling

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P2.
**Preconditions:** a conflicting file at `/` + 240 `d` chars + `/x.txt`, within the configured `max_path_len`; the same seeded on both branches with different content (`A\n` vs `B\n`).
**When** `git.merge` then resolve by `strategy:"ours"`.
**Then** the conflict response carries that exact path string (no truncation, compared character by character); the resolve succeeds; the file reads exactly `A\n`.
**And** a second run with `max_path_len` lowered below that length fails with `ERR_INVALID_ARGUMENT` from `ensure_path_fits` (`safety.rs:59`) and writes nothing.
**Verification:** exact path string equality; file read; error code in the second run + full-volume byte-map equality.

#### E2E-NEW-577 — two concurrent merges, exactly one wins

**Scenario:** SC-929. **Requirements:** FR-NEW-283.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** two `git.merge {source_ref:"feature"}` calls are issued with `tokio::join!` on the same `mount_id`.
**Then** exactly one returns `status == "conflict"` and exactly one returns `ERR_INVALID_ARGUMENT` containing `in progress` — never two conflict responses, never two rows.
**And** `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` == 1.
**And** the full-volume byte map equals the pre-call snapshot.
**Verification:** partition the two results and assert one of each; relational count; map equality.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-616 — Rebase conflict: volume untouched at the pause**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-171., FR-NEW-187
- **Preconditions:** FX-FORK4 conflicting; full byte snapshot of every volume file (`/a.txt`,`/f1.txt`,`/f2-side files`...) taken via a recursive listing plus `read_bytes`.
- **When** the E2E-NEW-615 step (1) rebase.
- **Then** for every path in the snapshot, current bytes == snapshot bytes; and the set of paths in the volume is identical to the snapshot set (no additions, no deletions). *Method:* `fs.list` recursive + byte compare.
- **Priority:** P0.

---

#### E2E-NEW-618 — Rebase conflict: markers never enter the volume**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-172.
- **Preconditions:** FX-FORK conflicting.
- **When** rebase pauses (E2E-NEW-613), then `git.rebase_continue {resolutions:{"/a.txt":"ours"}}`.
- **Then** at **both** points, every file in the volume is scanned and none contains `"<<<<<<< "`, `"||||||| "`, `"======="` at line start, or `">>>>>>> "`. *Method:* recursive `fs.grep` for `^(<<<<<<<|=======|>>>>>>>)` returning zero matches (grep goes through `Fnmatch`, AGENTS.md).
- **And** with `"ours"`, `/a.txt == b"a1\nMAIN\n"` (the onto side).
- **Priority:** P0.

---

#### E2E-NEW-835 — SideEffect — an automatic merge records nothing to resolve**

**Category:** SideEffect. **Scenario:** SC-903. **Requirements:** FR-NEW-173, FR-NEW-284.
- **Preconditions:** SEED-AUTOMERGE on `proj`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** the response has no `continue_with`/`abort_with` key, no `operation_id` key and no `conflicts` key; `status == "merged"`.
- **And** `SELECT COUNT(*) FROM git_operations WHERE volume_id='proj'` is `0` both immediately after the call and after a `git.status` call.
- **And** `git.merge_resolve {resolutions:[{path:"/src/config.toml",strategy:"ours"}]}` immediately afterwards errors `ERR_INVALID_ARGUMENT` containing `"no merge is in progress"` — there is genuinely nothing left open.
- **And** exactly one audit entry with `op == "git.merge"` exists, whose `detail` contains `"merged"` and not `"conflict"`.
- **Verification:** JSON key absence; relational count twice; follow-up error; audit scan.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-842 — SideEffect — the no-op merge charges no quota and writes one audit line at most**

**Category:** SideEffect. **Scenario:** SC-903. **Requirements:** FR-NEW-192.
- **Preconditions:** SEED-FF, `git.merge {source_ref:"feature"}` run once; capture `before_bytes = bytes_written(OWNER, MOUNT)` and `before_audit = audit(OWNER, MOUNT)`.
- **When** `git.merge {source_ref:"feature"}` is called a second time.
- **Then** `status == "already_up_to_date"` and the call is `Ok`.
- **And** `bytes_written(OWNER, MOUNT) == before_bytes` exactly (no file was rewritten with identical bytes).
- **And** any new audit entry has `detail` containing `"already_up_to_date"`; no entry claims a merge commit sha.
- **And** `db.get_ref("refs/heads/main").target` equals the value captured before the second call.
- **And** no `git_operations` row exists.
- **Verification:** quota equality; audit diff inspection; ref equality; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-843 — EdgeCase — merging a raw sha that is already an ancestor**

**Category:** EdgeCase. **Scenario:** SC-903. **Requirements:** FR-NEW-192, FR-NEW-195.
- **Preconditions:** SEED-FF after one merge; `<sha-C0>` is the base commit, an ancestor of `main`.
- **When** `git.merge {source_ref:"<sha-C0>"}` (a 40-hex sha, not a branch name).
- **Then** `status == "already_up_to_date"`, `merge_commit` is `null`, the call is `Ok`.
- **When** `git.merge {source_ref:"main"}` (the checked-out branch itself, by name).
- **Then** `status == "already_up_to_date"` as well — an ancestor is an ancestor whether named by ref or by sha.
- **And** `git.log {ref_name:"main"}` has the same length before and after both calls.
- **Verification:** two response objects; log length equality.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-845 — SideEffect — the fast-forward creates no new object at all**

**Category:** SideEffect. **Scenario:** SC-903. **Requirements:** FR-NEW-193.
- **Preconditions:** SEED-FF; capture `before_objects = COUNT(*) FROM git_objects WHERE volume_id='proj'` and `<sha-FEAT>` = `feature`'s tip.
- **When** `git.merge {source_ref:"feature"}` with `squash` absent.
- **Then** `status == "fast_forward"` and the response's `merge_commit` is `null`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-FEAT>"` exactly (the branch was moved, not merged).
- **And** `COUNT(*) FROM git_objects WHERE volume_id='proj'` equals `before_objects` — no commit, tree or blob was created.
- **And** `git.log {ref_name:"main"}` and `git.log {ref_name:"feature"}` return identical arrays.
- **Verification:** status/`null` field; ref equality; relational count equality; array equality.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-846 — EdgeCase — dirt consisting only of a new untracked file still refuses**

**Category:** EdgeCase. **Scenario:** SC-903. **Requirements:** FR-NEW-194.
- **Preconditions:** SEED-AUTOMERGE (a merge that would otherwise succeed with no conflict).
- **Given** `fs.write_text {path:"/notes.md", content:"todo\n"}` — a file present in neither tree, so `git.status.changes == [{"path":"/notes.md","status":"added"}]`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"` — "dirty" includes additions, matching the pull dirty check.
- **And** `/notes.md` still reads exactly `b"todo\n"` and `/src/config.toml` line 2 is still `port = 8000` (the automerge did not run).
- **And** `db.get_ref("refs/heads/main").target` is unchanged.
- **Verification:** error code + substrings; two byte compares; ref read.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-847 — SideEffect — the dirty refusal records no operation and no fetch-like side effect**

**Category:** SideEffect. **Scenario:** SC-903. **Requirements:** FR-NEW-194, FR-NEW-171.
- **Preconditions:** SEED-CONFLICT (a merge that would otherwise conflict and create a row); `/src/config.toml` line 6 rewritten to `retries = 9` without committing.
- **Given** `before_objects = COUNT(*) FROM git_objects WHERE volume_id='proj'`, `before_refs = db.list_refs()`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"`.
- **And** `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `0` — the refusal happens *before* any merge work, so no in-progress row is left to block the volume.
- **And** `COUNT(*) FROM git_objects` equals `before_objects` and `db.list_refs()` equals `before_refs` element-for-element.
- **And** a following `git.merge {source_ref:"feature"}`, after `git.commit {message:"retries"}`, reaches the normal conflict response — proving the first refusal left the volume usable.
- **Verification:** error assertion; three relational/ref comparisons; follow-up call.
- **Cleanup:** `git.merge_abort`. **Priority:** P0.

---

#### E2E-NEW-848 — EdgeCase — a well-formed but absent 40-hex sha**

**Category:** EdgeCase. **Scenario:** SC-903. **Requirements:** FR-NEW-195.
- **Preconditions:** SEED-CONFLICT.
- **When** `git.merge {source_ref:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"` — `resolve_ref` treats an all-hex name as a raw sha (`git.rs:486-504`), so the failure must come from the absent object, not from name parsing.
- **When** `git.merge {source_ref:"refs/heads/ghost"}` (a full ref path that does not exist).
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/heads/ghost"`.
- **And** after both calls `db.get_ref("refs/heads/main").target` is unchanged and no `git_operations` row exists.
- **Verification:** two error assertions; ref read; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

#### E2E-NEW-849 — SideEffect — the unknown source is rejected before the dirty check writes anything**

**Category:** SideEffect. **Scenario:** SC-903. **Requirements:** FR-NEW-195, FR-NEW-171.
- **Preconditions:** SEED-CONFLICT; capture `before_audit` and `before_bytes`.
- **When** `git.merge {source_ref:"nope", squash:true}` (squash set, so a naive implementation would have taken the squash path).
- **Then** assert error `ERR_NOT_FOUND` containing `"nope"`.
- **And** `audit(OWNER, MOUNT)` equals `before_audit` element-for-element and `bytes_written` equals `before_bytes`.
- **And** `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `0`.
- **And** `/src/config.toml` is byte-identical to its pre-call content.
- **Verification:** error assertion; audit/quota equality; relational count; byte compare.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### E2E-NEW-870 — EdgeCase — a paused merge survives the reopen with its conflict set intact**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-278, FR-NEW-275.
- **Preconditions:** SEED-MULTI(3) on `proj`; `git.merge {source_ref:"feature"}` conflicts on `/f/000.txt`, `/f/001.txt`, `/f/002.txt`.
- **Given** the `git_operations` row is read and captured field by field.
- **When** the `GitRepoStore` is dropped and rebuilt against the same config (the reopen technique of E2E-NEW-697), then `git.status {mount_id:"proj"}` is called.
- **Then** `operation.op_type == "merge"` and `remaining_conflicts == ["/f/000.txt","/f/001.txt","/f/002.txt"]`.
- **And** the re-read `git_operations` row equals the captured row field for field, `updated_at` included (a reopen is not a write).
- **And** `git.merge_resolve {resolutions:[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}` completes the merge across the restart boundary.
- **Verification:** row capture/compare; `git.status` JSON; completion call.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-953 — The conflict response names the correct completion pair for each operation**
- **Category:** EdgeCase | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-170
- **Preconditions:** As E2E-NEW-951, one conflicted operation of each of the five types.
- **Steps:**
  - Given each conflicted operation's response
  - Then `continue_with` and `abort_with` are exactly: merge -> `git.merge_resolve` / `git.merge_abort`; remote_pull -> `git.merge_resolve` / `git.merge_abort`; stash_apply -> `git.merge_resolve` / `git.merge_abort`; rebase -> `git.rebase_continue` / `git.rebase_abort`; cherry_pick -> `git.cherry_pick_continue` / `git.cherry_pick_abort`; revert -> `git.revert_continue` / `git.revert_abort`
  - And calling exactly the tool named in `continue_with` always advances the operation
- **Verification:** exact string equality on both JSON fields, then a successful continuation call per operation.
- **Cleanup:** complete or abort each. **Priority:** P0

## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not treat the existing refusal at `crates/mcp-fs/src/tools/git.rs:1844-1849` as a template for the conflict response: it is the error path being replaced. Do not write conflict markers into the volume under any circumstance. Do not emit `conflicting_paths`, `resolve_with`, a `step` object, `target_ref`, `present` or `size`.

### Scope Boundary
The conflict contract, the engine extraction and `git.merge` itself. Resolution and abort are US-003; squash is US-006; the pull rework is US-007.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
The existing pull tests must stay green through the extraction: it is a refactor first and an extension second. Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
