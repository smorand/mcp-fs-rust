# Full Git Development Process — Design

## 1. Components

| Component | Role |
|---|---|
| `crates/core/src/tools/git.rs` | 25 of the 31 new tool registrations/handlers (PR tools live separately); shared merge engine extraction point; `Completion` enum branching merge-commit vs working-tree completion. |
| `crates/core/src/tools/git_pr.rs` (new) | The 6 PR tools and the provider-independent normalization layer. |
| `crates/core/src/git/provider/` (new) | Injectable provider REST client trait, GitHub/GitLab implementations, host-to-base-URL resolution, cross-host redirect refusal. |
| `crates/core/src/git/merge.rs` (new) | `ConflictResponse` and the shared three-way merge/apply engine reused by merge, rebase, cherry-pick, revert, pull, stash apply/pop. |
| `crates/core/src/git/db.rs` | `git_operations` table added to `schema()`; `TABLES` grows 3→4; CRUD for the operation record. |
| `crates/core/src/git/repo.rs` | Existing per-project write lock, now taken by every new ref-mutating tool. |
| `crates/core/src/git/oauth/device_flow.rs` | GitLab scope constant gains `api`; both provider scopes made configurable. |
| `crates/core/src/git/oauth/store.rs` | Scope-sufficiency query for FR-NEW-332. |
| `crates/core/src/config.rs` | Six new optional config keys. |
| `crates/core/src/tools/all.rs` | Registration wiring; count assertions updated at three of five sites. |
| `crates/core/src/tools/contract_golden.rs` | Frozen contract count 63→94. |

Unaffected: `api/dataplane.rs`, `api/openapi.rs` (git stays MCP-only, 38 REST routes unchanged), `storage/` blob layer (git objects keep the `git:{sha}` layout).

## 2. Flows

**Combine-operation flow (merge/rebase/cherry-pick/revert/pull/stash-apply-pop), shared by all six**: take the per-project write lock → resolve the operation's inputs → run the three-way merge via the shared engine → if clean, apply atomically (quota charge, then write, then move ref) and release the lock → if conflicting, persist a `git_operations` row (`op_type`, `state:"conflicted"`, `original_tip_sha`, conflict set) and return the shared `ConflictResponse`, applying nothing. Resolution flow: a resolve/continue call validates every named path is in the active conflict set (all-or-nothing), applies `ours`/`theirs`/content per path, and once the set is empty, charges quota and applies atomically, then deletes the row. Abort flow: restore `original_tip_sha` and byte-identical volume, delete the row.

**Rebase-specific flow**: pre-flight-validate the whole todo before any replay; hold the write lock across the entire multi-step replay (not released between steps); on each step's conflict, persist `current_step`/`total_steps`/remaining `todo`, release the per-step work (not the lock) and return; `rebase_continue` resumes exactly at the persisted step.

**PR flow**: resolve provider + API base from the volume's remote host via `git.hosts` and stored `instance_url` → validate recorded `(person, host)` scope against the operation's required scope before any request → call the injectable provider client → normalize the provider's payload into the FR-NEW-317 shape, scrubbing `raw` of authorization data → never follow a cross-host redirect.

## 3. Interfaces

- **Branch**: `git.branch_create {name, start_point?, checkout?}` → `{branch, sha, checked_out}`; `git.branch_switch {name}` → `{branch, sha, changed, files_changed}`; `git.branch_delete {name, force?}` → `{branch, sha, forced}`; `git.branch_reset {name, target_commit, force?}` → `{branch, old_sha, new_sha, checked_out, files_changed}`.
- **Stash**: `git.stash_save {message?}` → `{stash_id, sha, message, base_sha, branch, created_at, files_stashed}`; `git.stash_list {}` → `{stashes[], count}`; `git.stash_apply/pop {stash_id}` → `{stash_id, status, files_changed, dropped}`; `git.stash_drop {stash_id}` → `{stash_id, dropped}`.
- **Remotes**: `git.remote_add {name, url}` → `{name, url, host, provider}`; `git.remote_remove {name}` → `{name, removed}`; `git.remote_list {}` → `{remotes[], count}`.
- **Merge**: `git.merge {source_ref, squash?, message?}` → `{status, merge_commit, fast_forward, squashed, files_changed}` or conflict response; `git.merge_resolve {resolutions}` → `{status, merge_commit, remaining_conflicts, resolved_count, files_changed}`; `git.merge_abort {}` → `{status, operation, restored_sha}`.
- **Rebase**: `git.rebase {onto, todo}` / `git.rebase_continue {resolutions}` → `{status, branch, new_tip, replayed, dropped, squashed, current_step, total_steps}` or conflict response; `git.rebase_abort {}` → `{status, operation, branch, restored_sha}`.
- **Cherry-pick**: `git.cherry_pick {commit_sha, mainline?}` / `git.cherry_pick_continue {resolutions}` → `{status, new_sha, source_sha}` or conflict response; `git.cherry_pick_abort {}` → `{status, operation, restored_sha}`.
- **Reset**: `git.reset {target_ref, mode}` → `{mode, old_sha, new_sha, files_changed}`.
- **Revert**: `git.revert {commit_sha, mainline?, message?}` / `git.revert_continue {resolutions}` → `{status, new_sha, reverted_sha}` or conflict response; `git.revert_abort {}` → `{status, operation, restored_sha}`.
- **Remote push/fetch/pull** (modified): `remote`/`remote_branch` parameters; force push requires `force` + `expected_remote_sha`. `git.remote_push` response: `{branch, remote, remote_branch?, created, up_to_date, forced, overwritten_sha?, remote_sha, auth}` (AMENDED: `remote_sha` not `new_sha`; `remote` unconditional; `remote_branch`/`overwritten_sha` omitted when inapplicable).
- **PR surface**: `git.pr_create/get/merge/review` → the FR-NEW-317 object (`provider, host, number, title, body, state, draft, base, head, author, url, created_at, updated_at, commits, changed_files, additions, deletions, review_state, checks_state, mergeable, raw`); `git.pr_list` → `{pull_requests[] (minus commits/changed_files/additions/deletions/mergeable), count}`; `git.pr_diff` → `{pr_number, diff, truncated}`.
- **Status**: `git.status` gains an `operation` object `{op_type, source_ref, current_step, total_steps, remaining_conflicts, continue_with, abort_with}` when a combine operation is paused.
- **Conflict response** (shared across all six combine operations): `{status:"conflict", operation, operation_id, source_ref, current_step, total_steps, conflicts:[{path, ours:{exists,content}, theirs:{exists,content}, base:{exists,content}, binary, type_change}], continue_with, abort_with}`.

## 4. Data and state

`git_operations` (new, in the git relational index, at most one row per volume, primary key `(volume_id)`):

| Column | Type | Notes |
|---|---|---|
| `volume_id` | `TextKey(VOLUME_ID_LEN)` | Scopes every query. |
| `op_type` | `TextKey(32)` | One of `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply`, `stash_pop`. |
| `state` | `TextKey(32)` | One of `conflicted`, `running` — never `paused`. |
| `onto_sha` | `Text` | Rebase target. |
| `original_tip_sha` | `Text` | Branch tip before the operation began — makes abort exact. |
| `original_head_ref` | `Text` | HEAD's ref at operation start. |
| `source_sha` | `Text` | Commit/ref being applied (cherry-pick, revert, merge). |
| `todo` | `Text` | JSON array of remaining rebase todo entries. |
| `current_step` | `BigInt` | Zero-based paused-entry index. |
| `total_steps` | `BigInt` | Total todo entries. |
| `conflicts` | `Text` | JSON array of unresolved paths. |
| `resolutions` | `Text` | JSON object of resolutions accepted so far. |
| `stash_id` | `Text` | The stash entry being applied, retained for `stash_apply`. |
| `created_at` / `updated_at` | `BigInt` | Epoch millis. |

Entities reused without schema change: branches and stash refs in `git_refs` (`refs/heads/*`, `refs/stash/*`); remotes in `git_remotes` (writer was previously limited to `origin`, table was always generic); remote-tracking refs in `git_refs` under `refs/remotes/{remote}/*`; commit/tree/blob objects in the blob store under `git:{sha}`, indexed in `git_objects`; token/scope in `oauth_tokens.scopes` (only the requested scope string and its validation changed).

## 5. Configuration

All optional, defaulted:

| Key | Default | Purpose |
|---|---|---|
| `git.max_stash_entries` | `100` | Per-volume stash pool bound |
| `git.max_rebase_todo` | `200` | Rebase todo length bound |
| `git.max_pr_diff_mb` | `12` | `git.pr_diff` response bound |
| `git.provider_api_timeout_secs` | `30` | Provider REST call timeout |
| `git.github_scope` | `repo` | GitHub device-flow scope |
| `git.gitlab_scope` | `api read_repository write_repository` | GitLab device-flow scope |

## 6. Observability

Every destructive operation (branch delete/reset, reset, force push, stash drop, rebase, revert, cherry-pick, merge) writes an audit entry through the existing capped audit log, force push additionally recording the overwritten sha. Remote/provider operations emit the existing `git.remote` tracing span field set (operation, host, provider, branch, outcome, duration_ms). Token values, credentials and `Authorization` header contents are never traced — enforced by a test that installs a tracing subscriber and inspects every captured field value, not by inspection alone.

## 7. Decisions

- **DEC-001** (was DEC-901): in-progress operation persisted as a `git_operations` row, not real-git control files in the bare-repo cache (would be destroyed by a cache rebuild) nor in-memory session state (evaporates on restart, inherits BL-003/BL-014). Implements FR-NEW-275..278.
- **DEC-002** (was DEC-902): conflicts surfaced with both sides' content, resolved per file, instead of one global strategy — the caller (an LLM) can read both sides and decide; a global `theirs` silently discards local work. Implements FR-NEW-170..186.
- **DEC-003** (was DEC-903): force push requires a mandatory lease (`expected_remote_sha`); bare `force:true` rejected — a lease mechanically prevents overwriting a remote that moved, stricter than git's bare `--force`. Implements FR-NEW-155..159.
- **DEC-004** (was DEC-904): continue/abort tools are per-operation and named after the git CLI rather than one generic trio — tool names are the LLM's primary affordance. Implements FR-NEW-196/197/221/223/225/239.
- **DEC-005** (was DEC-905): `git.reset` offers `soft`/`hard` only, no `mixed` — with no index, `mixed` would be indistinguishable from `soft`. Implements FR-NEW-250..252.
- **DEC-006** (was DEC-906): the pull-request model is normalized with a `raw` passthrough — one stable shape for the frozen contract, `raw` preserves provider-specific fields. Implements FR-NEW-303.
- **DEC-007** (was DEC-907): arbitrary named remotes supported, reversing the prior single-`origin` limit — a volume from `git.init` had no `origin` and could never push/fetch/pull. Implements FR-NEW-140..147, FR-MOD-101/103.
- **DEC-008** (was DEC-908): `on_conflict` removed from `git.remote_pull` outright rather than kept as a backward-compatible fast path — user's explicit choice; one way to do things. Consequence: six shipped tests modified/removed. Implements FR-DEL-101, FR-MOD-104/105.
- **DEC-009** (was DEC-909): a delete-vs-modify conflict and a file-vs-directory type change are surfaced as conflicts, not refused up front — consistent with the model's premise of surfacing rather than refusing. Implements FR-NEW-180/183.
- **DEC-010** (was DEC-910): the GitLab device flow requests `api` in addition to its current scopes, validated before the network call — `write_repository` alone grants no REST API access. Consequence: existing GitLab tokens must be re-granted. Implements FR-NEW-330..333, FR-MOD-109.
- **DEC-011** (was DEC-911): a token with an empty/unknown stored scope set is attempted rather than pre-emptively refused, while a known-insufficient scope fails early — `git.token_set`-seeded PATs have unenumerable scopes. Implements FR-NEW-334.
- **DEC-012** (was DEC-912): purely local destructive operations (`reset --hard`, `branch_delete --force`, `branch_reset --force`) need no lease — a lease protects against a concurrent remote actor, not a risk on the caller's own volume; recoverability comes from `old_sha` and non-pruning instead. Implements FR-NEW-111/251/255.
- **DEC-013** (was DEC-913): stash entries stored as `refs/stash/*` in the existing `git_refs` table, not a new table — matches real git's model, no schema change, namespace verified unused. Implements FR-NEW-120/123.
- **DEC-014** (was DEC-914): any project member can continue or abort another member's operation — membership-based authorization with no per-resource ownership; a volume one absent author could block would be a denial of service on the whole project. Implements FR-NEW-282.
- **DEC-015** (was DEC-915): scope held to one spec rather than split into four by theme, partitioned into stories afterwards by `/plan-spec` — splitting by theme would have fragmented the shared conflict model across documents.
- **DEC-016** (was DEC-916): tests numbered from `E2E-NEW-400` rather than from 1 — the prior spec already owns `E2E-NEW-001..247` as live Rust test function names.
- **DEC-017** (was DEC-917): test design performed by four independent sub-agents with fresh context, none of which wrote the requirements — the author of a requirement is the worst person to test it.
- **DEC-018** (was DEC-918): rebase supports `pick`/`squash`/`drop`/`reword` only, excluding `edit`/`fixup`/`exec`/`break` — `exec` is a sandbox-escape risk; `edit`/`break` need an interactive pause with no conflict to resolve, which the operation model doesn't represent. Implements FR-NEW-210..216.

## 8. Requirement to code map

| Requirement group | Code evidence |
|---|---|
| Shared conflict engine | `crates/core/src/git/merge.rs` — `ConflictResponse`; the conflict-success path was built as new code, not adapted from the old pull-refusal error path (DRIFT-001, resolved). |
| Stash-resolve completion branching | `crates/core/src/tools/git.rs:5248` — `enum Completion { MergeCommit, WorkingTree { drop_stash: bool } }`; `:5193-5194` dispatch on `GitOpType::StashApply`/`StashPop`. |
| `git.remote_push` response | `crates/core/src/tools/git.rs:3470` (`"remote": remote` unconditional), `:3473` (`"remote_sha": outcome.remote_sha`). |
| `git_operations` table / `TABLES` | `crates/core/src/git/db.rs`. |
| GitLab scope | `crates/core/src/git/oauth/device_flow.rs`. |
| PR provider seam | `crates/core/src/git/provider/`. |
| Tool contract | `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, `crates/core/src/tools/contract_golden.rs`. |

### FINDINGS FOR BACKLOG

None. All four external drift files resolved: the US-002 test-homing drift (`2026-09-22_07-49-37`) was a `/plan-spec` process note, not a spec defect, resolved by re-homing at implementation; the US-007 wire-shape undercounting (`2026-09-22_11-59-56`) was accepted as-is per its own recommendation (no spec edit needed, replacement coverage exists); the US-012 stash-resolve defect (`2026-09-22_14-24-41`) is resolved in code, verified at `crates/core/src/tools/git.rs:5248`; the US-022 `remote_sha`/`remote` key disagreement (`2026-09-22_23-33-21`) is resolved in code, verified at `crates/core/src/tools/git.rs:3470,3473`, and the spec amendment is folded into FR-NEW-199 and this document's Section 3.

## 9. Legacy mapping

Source: specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md (pre-move, renumbered to SPEC-0012); plus specs/drift/2026-09-22_07-49-37.md, 2026-09-22_11-59-56.md, 2026-09-22_14-24-41.md, 2026-09-22_23-33-21.md

| Old id | New id | Note |
|---|---|---|
| US-001 git_operations table, state model and purge registration | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-002 Shared merge engine and git.merge, with the conflict response contract | SPEC-0012 | done, implemented, not re-tracked in v2 queue (carries DRIFT-001, resolved; carries external drift `2026-09-22_07-49-37`, resolved via re-homing) |
| US-003 git.merge_resolve, git.merge_abort and resolution mechanics | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-004 Conflict edge semantics: delete/modify, both-deleted, binary, type change | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-005 In-progress guard and git.status operation reporting | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-006 Squash merge | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-007 git.remote_pull on the shared conflict model, on_conflict removed | SPEC-0012 | done, implemented, not re-tracked in v2 queue (external drift `2026-09-22_11-59-56` accepted as-is) |
| US-008 Branch creation and ref-name validation | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-009 Branch switch and the dirty-volume guard | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-010 Branch delete, branch_reset and tracking divergence | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-011 Stash save, list and drop | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-012 Stash apply and pop, with the conflict path | SPEC-0012 | done, implemented, not re-tracked in v2 queue (external drift `2026-09-22_14-24-41` resolved in code, commit a148421) |
| US-013 Rebase todo validation and planning | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-014 Rebase execution: pick, squash, drop, reword | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-015 Rebase pause, continue and abort | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-016 Cherry-pick with continue and abort | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-017 git.reset soft: pointer-only move | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-018 git.reset hard: volume rewrite and orphaning | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-019 git.revert, including merge commits and the conflict path | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-020 Remote add, remove and list | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-021 Named remotes on push, fetch and pull, with local:remote refspec | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-022 Force push with a mandatory lease | SPEC-0012 | done, implemented, not re-tracked in v2 queue (external drift `2026-09-22_23-33-21` resolved in code, commits b741566/aec67cd) |
| US-023 OAuth scope request, validation and reporting | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-024 Provider client seam, base URL resolution and transport safety | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-025 git.pr_create and the normalized pull-request model | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-026 git.pr_list with state filtering | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-027 git.pr_get and git.pr_diff | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-028 git.pr_merge with provider refusals surfaced | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-029 git.pr_review | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-030 Tool registration, frozen contract and count assertions | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
| US-031 Blanket authorization coverage and documentation parity | SPEC-0012 | done, implemented, not re-tracked in v2 queue |
