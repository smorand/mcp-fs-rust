> Id: SPEC-0012
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0011; new id SPEC-0012 (collision avoidance)

# Full Git Development Process

## 1. Summary

mcp-fs shipped a git surface (18 tools) with clone/push/fetch/pull, an encrypted per-`(person, host)` OAuth token store, and a host map resolving provider and instance URL, but no *development process*: no branch create/switch/delete, no stash, no local merge, no rebase, no cherry-pick, no reset, no revert, no force push, no second remote, no pull-request surface. This spec closed that gap: 31 new tools covering branch lifecycle, stash, local merge/squash, interactive rebase, cherry-pick, reset, revert, named remotes, leased force push, and a full pull-request surface (create/list/get/diff/merge/review) for GitHub and GitLab including self-hosted/Enterprise deployments.

Two structural additions carried the rest: a new `git_operations` relational table recording a paused multi-step operation (merge/rebase/cherry-pick/revert/stash conflict) so it survives a restart and is visible to every project member; and a single **shared conflict model** replacing the old global `ours`/`theirs` pull strategy — the server auto-merges what it can and surfaces per-file conflicts (both sides' content) for the caller to resolve by strategy or by supplied content.

The audience is an LLM acting as an interactive terminal, which drove tool naming (mirrors git CLI vocabulary) and the decision to surface conflicts back to the caller rather than resolve them by a global policy.

## 2. Current State

### 2.1 Functional

Prior to this spec, mcp-fs registered 14 `git.*` tools plus 4 `git.auth*` tools (63 tools total in the frozen contract: 35 `fs.*`, 10 `admin.*`, 18 git-family). Architecture facts that constrained this work: the volume **is** the working tree (no index, no staging, `git.commit` commits the whole volume); git objects live in the blob store under `git:{sha}` (the bare repo on disk is a rebuildable cache); the git relational index owned exactly three tables (`git_objects`, `git_refs`, `git_remotes`); a per-project write lock serializes ref-mutating operations; every git tool authorizes on project membership only (platform admin gets no implicit access); pull volume writes were atomic; git was MCP-only (no REST routes); `refs/stash` was an unused namespace.

The pull path was the template the new combine operations reused: resolve credentials, take the write lock, fetch, fast-forward or three-way-merge via libgit2 with a single global `MergeOptions::file_favor`, refusing outright on residual conflicts.

### 2.2 Related specs

SPEC-0007 (git: objects, HTTP smart protocol, original 18-tool surface), SPEC-0011-legacy-archived (GitHub Enterprise and token store: host map, per-host token store, remote pipeline, pull with `on_conflict` — implemented, 19 stories; this spec supersedes part of its pull behaviour, see Section 9).

### 2.3 Test coverage

The implementation delivered 31 stories (all status `done`), adding roughly 536 new E2E tests (`E2E-NEW-4xx` through `9xx`) plus 9 modified/removed shipped tests, bringing the suite from 1377 to 2010 passing tests. A post-implementation converge audit found 161 of 162 requirements PASS with both implementation and a non-ignored asserting test; the one gap (FR-NEW-199 wire-shape disagreement) was closed. See `stories/_index.md` coverage section for the full gate numbers (0 unassigned FRs, 0 unassigned tests, 0 uncovered scenarios, 0 floor-check violations).

## 3. Scope

### In scope

- Branch lifecycle: create, switch, delete, force-move (`branch_reset`).
- Stash: save, list, apply, pop, drop — stored as `refs/stash/*`.
- Local merge (two-parent and squash), with resolve/abort.
- Shared conflict model reused by pull, merge, rebase, cherry-pick, revert, stash apply/pop.
- Interactive rebase: ordered todo of pick/squash/drop/reword, pausable, resumable, abortable.
- Cherry-pick with continue/abort.
- Reset in `soft` and `hard` only (no `mixed`).
- Revert, including merge commits with explicit mainline.
- Remotes: add/remove/list, `remote` and `remote_branch` parameters on push/fetch/pull.
- Force push gated behind a mandatory lease (`expected_remote_sha`).
- Pull requests/merge requests: create/list/get/diff/merge/review, GitHub and GitLab including Enterprise/self-hosted.
- OAuth scope: GitLab device flow gains `api`; scope validated before any network call.
- `git_operations` table: records a paused operation, reported by `git.status`, enforced as a guard against interleaved mutations.
- Closes BL-004, BL-005, BL-006, BL-008, BL-009, BL-010 (via reset/stash), BL-007 (in the agreed form), resolves part of BL-013.

### Out of scope

Multi-replica coordination (BL-003, BL-014, write lock stays an in-process `tokio::sync::Mutex`); Azure DevOps (BL-012); pluggable credential providers (BL-011); non-HTTPS transports; submodules, tag push, `git gc`, partial clone, worktrees, hooks, notes, bisect; conflict markers in the volume (prohibition retained); REST exposure of git (stays MCP-only); a staging area; rebase actions beyond pick/squash/drop/reword.

## 4. Actors

| Actor | Description |
|---|---|
| Caller | MCP client (in practice an LLM chat). Authenticated as a `person`, must be a project member. |
| Project member | Shares one volume and one git history; can encounter and resolve an operation another member paused. |
| Platform admin | Manages projects/membership; explicitly no implicit file or git access. |
| Provider | GitHub or GitLab (incl. Enterprise/self-hosted), reached over HTTPS for transport and, newly, for the PR REST API. |

## 5. Usage Scenarios

Cross-cutting exceptions (not repeated per scenario): non-member → `ERR_FORBIDDEN`; unauthenticated → `ERR_UNAUTHENTICATED`; unknown `mount_id` → `ERR_PROJECT_NOT_FOUND`; git disabled → tool not registered; write quota exhausted → `ERR_WRITE_QUOTA_EXCEEDED`; for remote operations, undeclared host / missing or expired token / insufficient scope rejected before any network call.

- **SC-901 Branch lifecycle** — create (no HEAD move), switch (volume rewrite), commit, switch back, force-delete. Exceptions: duplicate name, unknown switch target, dirty-volume switch refusal, delete-checked-out refusal, delete-unmerged-without-force refusal, switch-to-current is a no-op.
- **SC-902 Stash to switch away from dirty work** — save captures diff under `refs/stash/*` and reverts volume to HEAD; switch now succeeds; later pop reapplies and removes the entry. Exceptions: stash a clean volume, unknown stash id, stash apply conflict (entry retained), apply (vs pop) leaves entry listed.
- **SC-903 Clean local merge** — two-parent commit, disjoint changes. Exceptions: already-merged no-op, unknown ref, dirty volume refusal.
- **SC-904 Merge conflict resolved by per-file strategy** — conflict response listing ours/theirs/base; `merge_resolve` with `{"path":"theirs"}` completes it. Exceptions: resolving a non-conflicting path, partial resolution stays in progress, `merge_abort` restores exactly, invalid strategy value rejected.
- **SC-905 Merge conflict resolved by supplied content** — `{"content": "<bytes>"}` used verbatim; strategy and content mix freely across paths.
- **SC-906 Local squash merge** — single commit, single parent; conflicts use the same model.
- **SC-907 Second remote** — add/fetch/push/remove `upstream`, `origin` untouched. Exceptions: duplicate name, non-HTTPS URL, remove unknown remote, operate on unknown remote name before any network call.
- **SC-908 Push under a different remote branch name** — `remote_branch` differs from local; divergence refusal / force-with-lease apply identically.
- **SC-909 / SC-910 Force push accepted / rejected by the lease** — lease (`expected_remote_sha`) must match the remote's actual tip; mismatch names both shas and leaves the remote untouched.
- **SC-911 Divergent pull that merges automatically** — no caller decision required when sides touch disjoint files.
- **SC-912 / SC-913 Divergent pull conflict, resolved by strategy / by supplied content** — same conflict response shape as `git.merge`, completed by `git.merge_resolve` (not a pull-specific tool).
- **SC-914 Simple rebase, no conflicts** — todo `pick`/`pick` replayed onto new base with new shas. Exceptions: nothing-to-replay no-op, sha outside range or unknown rejected before any work, todo omitting an in-range commit rejected, leading `squash` rejected, todo-too-long rejected.
- **SC-915 Interactive rebase with squash and reword** — fold + reword; one entry fewer in output history; all-`drop` collapses onto `onto`.
- **SC-916 Rebase conflict, pause, resolve, continue** — pauses mid-todo, `rebase_continue` resumes at the exact step, can pause again; exactly one `git_operations` row throughout.
- **SC-917 Rebase abort** — branch tip and every byte restored exactly.
- **SC-918 / SC-919 Cherry-pick without / with conflict** — new commit, original author preserved, caller is committer; already-present commit reported not duplicated; conflict uses `cherry_pick_continue`/`cherry_pick_abort`.
- **SC-920 / SC-921 Reset soft / hard** — soft moves pointer only; hard also rewrites volume; `mode` is required and closed (`soft`|`hard`), no `mixed`.
- **SC-922 / SC-923 Revert normal / merge commit** — inverse commit; merge revert requires explicit `mainline`; reverting a revert restores the original change.
- **SC-924 Force-move a branch pointer** — `branch_reset`; rewrites volume only when that branch is checked out.
- **SC-925..928 Pull request lifecycle** — create/list/get+diff+review/merge, normalized across GitHub and GitLab; unsupported provider / missing scope / provider rejection surfaced faithfully, never swallowed, never containing a token.
- **SC-929 An operation is already in progress** — every other ref-mutating tool rejected naming the active operation and its finishing tools; read-only tools remain available; any member may resolve or abort.
- **SC-930 A stash outlives its context** — remains listable/applicable after its source branch is deleted; a pop that conflicts retains the entry until resolved.

## 6. Functional Requirements

All ids kept verbatim from the legacy spec. EARS notation; universal rule FR-NEW-100 (authorization + path normalization) applies to every tool below and is not repeated per entry.

### New Requirements

**A. Branch lifecycle**
- **FR-NEW-100**: WHEN any tool from this spec is called THE server SHALL authorize on project membership for `mount_id` before anything else, and normalize every path. Evidence: `crates/core/src/tools/git.rs`.
- **FR-NEW-101**: `git.branch_create` creates `refs/heads/{name}` at `start_point`'s commit; HEAD moves only if `checkout:true`. Outputs `{branch, sha, checked_out}`.
- **FR-NEW-102**: duplicate `name` → `ERR_INVALID_ARGUMENT`, existing ref untouched.
- **FR-NEW-103**: unresolvable `start_point` → `ERR_NOT_FOUND`.
- **FR-NEW-104 / FR-NEW-116**: branch-name validation mirrors `git check-ref-format`; 255-byte (not character) UTF-8 ceiling, keyed to `git_refs.name`'s `TextKey(400)` SQL Server rendering.
- **FR-NEW-105**: `git.branch_switch` moves HEAD and rewrites the volume exactly to the target tree; atomic; quota-charged and audited. Outputs `{branch, sha, changed, files_changed}`.
- **FR-NEW-106**: dirty volume → switch refused, naming `git.commit`/`git.stash_save`.
- **FR-NEW-107**: switch to current branch is a no-op, `files_changed` both zero.
- **FR-NEW-108**: `git.branch_delete` removes `refs/heads/{name}`; commits become unreachable but not destroyed. Outputs `{branch, sha, forced}`.
- **FR-NEW-109**: delete checked-out branch → refused.
- **FR-NEW-110**: delete unmerged branch without `force` → refused, naming each orphaned commit.
- **FR-NEW-111 / FR-NEW-112**: `git.branch_reset` sets `refs/heads/{name}` to `target_commit`; rewrites volume only if that branch is checked out. Outputs `{branch, old_sha, new_sha, checked_out, files_changed}`.
- **FR-NEW-113**: non-fast-forward move without `force` → refused, naming orphaned commits.
- **FR-NEW-114**: unknown `target_commit` → `ERR_NOT_FOUND`, ref unmoved.
- **FR-NEW-115**: every branch-mutating tool holds the write lock for the whole mutation.

**B. Stash**
- **FR-NEW-120 / FR-NEW-131**: `git.stash_save` captures volume state as a commit under `refs/stash/{stash_id}`, reverts volume to HEAD. Outputs `{stash_id, sha, message, base_sha, branch, created_at, files_stashed}`; stored in existing `git_refs` table, no new schema.
- **FR-NEW-121**: clean volume → stash refused, no ref created.
- **FR-NEW-122**: `git.stash_list` returns every entry, newest first, empty list when none.
- **FR-NEW-123**: stash refs never listed in `git.branches` nor checkoutable via `branch_switch`.
- **FR-NEW-124 / FR-NEW-132**: `git.stash_apply` applies and retains the entry; `git.stash_pop` applies and deletes the entry only once fully succeeded.
- **FR-NEW-126**: a conflicting stash application enters the shared conflict model, retains the entry regardless of apply vs pop, records `op_type` `stash_apply`/`stash_pop`.
- **FR-NEW-127**: `git.stash_drop` deletes without touching the volume.
- **FR-NEW-128**: unknown `stash_id` → `ERR_NOT_FOUND` on apply/pop/drop.
- **FR-NEW-129**: stash entries outlive their source branch (anchored on `base_sha`, not a branch name).
- **FR-NEW-130**: `git.max_stash_entries` (default 100) bounds the pool.

**C. Remotes**
- **FR-NEW-140**: `git.remote_add` records a remote (HTTPS only, no embedded credentials, host declared). Outputs `{name, url, host, provider}`.
- **FR-NEW-141**: duplicate name → refused in the tool layer (storage is an upsert).
- **FR-NEW-142 / FR-NEW-143**: `git.remote_remove` deletes the remote and its tracking refs; unknown name → `ERR_NOT_FOUND` (storage delete is unconditional, check lives in the tool layer).
- **FR-NEW-144**: `git.remote_list` returns every remote with resolved host/provider.
- **FR-NEW-145**: no token value ever appears in remote tool output or errors.
- **FR-NEW-146**: unknown `remote` parameter on push/fetch/pull → `ERR_NOT_FOUND` before any connection.
- **FR-NEW-147**: a named fetch touches only that remote's `refs/remotes/{name}/*`.

**D. Force push with lease**
- **FR-NEW-155**: force push with a valid `expected_remote_sha` updates a diverged remote ref.
- **FR-NEW-156**: `force:true` without `expected_remote_sha` → refused, no network call (lease mandatory, DEC-903).
- **FR-NEW-157**: stale lease → refused naming both expected and actual sha, remote unchanged.
- **FR-NEW-158**: lease supplied with `force:false` → refused as contradictory.
- **FR-NEW-159**: a forced push audits the overwritten sha.
- **FR-NEW-160**: force never applies implicitly on a non-fast-forward push.

**E. Shared conflict model**
- **FR-NEW-170**: one conflict response shape (`status:"conflict"`, `operation`, `operation_id`, `conflicts[]` of `{path, ours, theirs, base}`) for `remote_pull`, `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply`, `stash_pop`.
- **FR-NEW-171**: a conflict applies nothing (no file write, no commit, no ref move) — volume byte-identical to pre-operation state.
- **FR-NEW-172**: conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`) never enter the volume.
- **FR-NEW-173**: disjoint-region divergence auto-completes, no caller decision.
- **FR-NEW-174**: resolve by per-file `ours`/`theirs`.
- **FR-NEW-175**: resolve by supplied `content`, used verbatim.
- **FR-NEW-176**: strategy and content mix freely across paths in one call.
- **FR-NEW-177**: resolving a non-conflicting path rejects the whole call.
- **FR-NEW-178**: partial resolution keeps the operation in progress, remaining paths returned.
- **FR-NEW-179**: invalid strategy string → `ERR_INVALID_ARGUMENT`.
- **FR-NEW-180**: delete/modify conflict surfaced (deleted side explicit null), not refused.
- **FR-NEW-181**: both-deleted path auto-resolves to deletion, not in the conflict set.
- **FR-NEW-182**: binary conflict marked `binary:true`, content omitted, only `ours`/`theirs` accepted.
- **FR-NEW-183**: file-vs-directory type change surfaced with `type_change:true`, only `ours`/`theirs` accepted.
- **FR-NEW-184**: applying a completed resolution is atomic: quota charged before write, ref advances only after every file write succeeds.
- **FR-NEW-185**: quota exhaustion mid-resolution changes nothing, operation retained for retry.

**F. Local merge and squash**
- **FR-NEW-190**: `git.merge` creates a two-parent commit, updates the volume. Outputs `{status, merge_commit, fast_forward, squashed, files_changed}`.
- **FR-NEW-191**: `squash:true` produces one single-parent commit; source commits absent from history.
- **FR-NEW-192**: already-merged source → `status:"already_up_to_date"`, `merge_commit:null`.
- **FR-NEW-193**: fast-forwardable merge fast-forwards, no merge commit, `fast_forward:true`.
- **FR-NEW-194**: dirty volume → merge refused before any merge work.
- **FR-NEW-195**: unknown `source_ref` → `ERR_NOT_FOUND`.
- **FR-NEW-196**: `git.merge_resolve` completes merge, pull and stash-apply conflicts; not rebase/cherry-pick (own continue tools).
- **FR-NEW-197**: `git.merge_abort` restores HEAD and volume exactly, clears the record.
- **FR-NEW-198**: resolve/abort with nothing in progress → `ERR_INVALID_ARGUMENT`.

**G. Interactive rebase**
- **FR-NEW-210**: `git.rebase` replays `todo` in order onto `onto`.
- **FR-NEW-211..214**: `pick` replays unchanged (committer = caller); `squash` folds into predecessor (concatenated message); `drop` omits; `reword` replaces message, tree identical to `pick`.
- **FR-NEW-215**: pre-flight validation (unknown/out-of-range/duplicate sha, omitted in-range commit, leading `squash`, unknown action, empty `reword` message) rejects before any work.
- **FR-NEW-216**: `git.max_rebase_todo` (default 200) bounds the todo length.
- **FR-NEW-217**: nothing-to-replay onto an ancestor → `status:"up_to_date"`.
- **FR-NEW-218**: dirty volume → refused before any replay.
- **FR-NEW-219 / FR-NEW-220**: a conflicting step pauses the rebase, recording `op_type:"rebase"`, step index, remaining todo, conflict set; can pause more than once.
- **FR-NEW-221**: `git.rebase_continue` resolves the paused entry, commits it, proceeds until exhausted or paused again.
- **FR-NEW-222**: continuing with unresolved paths → rejected, listing them, rebase stays paused at the same step.
- **FR-NEW-223**: `git.rebase_abort` restores the exact pre-rebase ref and volume.
- **FR-NEW-224**: continue/abort with nothing in progress → rejected.
- **FR-NEW-225**: continuation tools are not cross-wired between operation types (FR-NEW-241).
- **FR-NEW-226**: the rebase holds the write lock for its entire replay.

**H. Cherry-pick**
- **FR-NEW-235**: `git.cherry_pick` applies one commit as a new commit with a new sha (author preserved, committer = caller).
- **FR-NEW-236**: already-ancestor commit → `status:"already_present"`, no duplicate.
- **FR-NEW-237**: unknown `commit_sha` → `ERR_NOT_FOUND`.
- **FR-NEW-238 / FR-NEW-239**: conflict pauses (`op_type:"cherry_pick"`); `cherry_pick_continue`/`cherry_pick_abort` finish or undo it.
- **FR-NEW-240**: cherry-picking a merge commit without `mainline` → rejected.
- **FR-NEW-241**: exactly one completion pair per operation type: `merge`/`remote_pull`/`stash_apply` → `git.merge_resolve`/`git.merge_abort`; `rebase` → `git.rebase_continue`/`git.rebase_abort`; `cherry_pick` → `git.cherry_pick_continue`/`git.cherry_pick_abort`; `revert` → `git.revert_continue`/`git.revert_abort`.

**I. Reset**
- **FR-NEW-250 / FR-NEW-251**: soft reset moves the branch pointer only; hard reset also rewrites the volume, discarding uncommitted changes.
- **FR-NEW-252**: `mode` required, closed to `soft`/`hard` — deliberately no `mixed` (no index to distinguish it, DEC-905).
- **FR-NEW-253**: unknown `target_ref` → `ERR_NOT_FOUND`, nothing moved.
- **FR-NEW-254**: reset to current tip → no-op success.
- **FR-NEW-255**: reset orphans rather than destroys unreachable commits (recoverable via `branch_reset` to `old_sha`).

**J. Revert**
- **FR-NEW-260**: `git.revert` creates an inverse commit, leaves the original in history.
- **FR-NEW-261 / FR-NEW-262**: reverting a merge requires `mainline`; out-of-range or non-merge `mainline` rejected.
- **FR-NEW-263**: reverting the initial commit computes against the empty tree.
- **FR-NEW-264 / FR-NEW-266**: a conflicting revert enters the conflict model (`op_type:"revert"`); `git.revert_continue`/`git.revert_abort` finish or undo it (own pair, per DEC-904, not reusing cherry-pick tools).
- **FR-NEW-265**: reverting a revert restores the original tree.

**K. Operation state and the in-progress guard**
- **FR-NEW-275**: a new relational table `git_operations`, keyed by `volume_id`, holds `op_type`, `state`, `onto_sha`, `original_tip_sha`, `todo`, `current_step`, `total_steps`, `conflicts`, `resolutions`, timestamps.
- **FR-NEW-276 / FR-MOD-108**: the table is registered in `TABLES` (grows from 3 to 4) so project deletion purges it.
- **FR-NEW-277**: defined through the existing dialect abstraction; works identically on SQLite/PostgreSQL/SQL Server (`TextKey(n)` for keyed columns, unbounded `Text` for payload columns).
- **FR-NEW-278**: a paused operation survives a server restart with step/todo/conflicts unchanged.
- **FR-NEW-279 / FR-NEW-280**: in progress, every other ref-mutating tool is rejected naming the active operation and its finishing tools; read-only tools (`status`, `log`, `show`, `diff`, `branches`, `tags`, `blame`, `stash_list`, `remote_list`, `remote_fetch`) stay available.
- **FR-NEW-281 / FR-NEW-286 / FR-MOD-107**: `git.status` includes an `operation` object with exactly `op_type`, `source_ref`, `current_step`, `total_steps`, `remaining_conflicts`, `continue_with`, `abort_with` when one is in progress, omitted otherwise.
- **FR-NEW-282**: any project member may resolve/continue/abort another member's paused operation (no per-operation ownership).
- **FR-NEW-283**: at most one in-progress row per `volume_id`, every query scoped by it.
- **FR-NEW-284**: completion or abort deletes the row.
- **FR-NEW-285**: `op_type` is a closed six-value set: `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply`, `stash_pop` (`remote_pull` recorded as `merge`).

**L. Pull requests and merge requests**
- **FR-NEW-300**: provider and API base resolve from the remote's host via the existing `git.hosts` map and stored `instance_url`.
- **FR-NEW-301**: provider `generic`/`anonymous` → `ERR_NOT_SUPPORTED` on every `git.pr_*` tool.
- **FR-NEW-302**: no usable remote / unparsable URL → `ERR_INVALID_ARGUMENT` before any network call.
- **FR-NEW-303 / FR-NEW-317**: one normalized response shape across providers, exact key order `provider, host, number, title, body, state, draft, base, head, author, url, created_at, updated_at, commits, changed_files, additions, deletions, review_state, checks_state, mergeable, raw`. `draft` is a separate boolean, never folded into `state`.
- **FR-NEW-304 / FR-NEW-305**: `git.pr_create`; `head` absent from the remote → `ERR_NOT_FOUND` naming `git.remote_push`, before any create request.
- **FR-NEW-306**: `git.pr_list` filtered by `state` (`open`|`closed`|`merged`|`all`), empty result is an empty array.
- **FR-NEW-307 / FR-NEW-308**: `git.pr_get` enriched with commits, changed files, `review_state`, `checks_state`; a failed enrichment sub-call fails the whole tool rather than degrading silently.
- **FR-NEW-309**: `git.pr_diff` bounded by `git.max_pr_diff_mb` (default 12), truncated with `truncated:true` past the bound.
- **FR-NEW-310 / FR-NEW-311**: `git.pr_merge` with `strategy` (`merge`|`squash`|`rebase`); provider refusals (checks, reviews, protected branch, disabled strategy, already merged/closed) surfaced faithfully, never reported as success. Local/remote-tracking refs not updated.
- **FR-NEW-312**: `git.pr_review` with `verdict` (`approve`|`request_changes`|`comment`); `request_changes`/`comment` require a non-empty `body`.
- **FR-NEW-313**: unknown `pr_number` → `ERR_NOT_FOUND`.
- **FR-NEW-314**: no token value ever leaves through a PR tool, response, `raw` payload, error, log, or tracing span.
- **FR-NEW-315**: the provider client is injectable for offline testing (mirrors the device-flow client seam).
- **FR-NEW-316**: the provider client never follows a cross-host redirect.

**M. OAuth scope**
- **FR-NEW-330 / FR-MOD-109**: the GitLab device flow requests scope including `api` (was `read_repository write_repository`, which grants no REST API access).
- **FR-NEW-331**: requested scopes configurable (`git.github_scope` default `repo`, `git.gitlab_scope` default `api read_repository write_repository`).
- **FR-NEW-332**: required scope validated against stored `(person, host)` scopes before any network call, `ERR_FORBIDDEN` on insufficiency.
- **FR-NEW-333**: the scope error names the missing scope, host, and `git.auth`/`git.token_set` as remedies.
- **FR-NEW-334**: an empty/unrecorded scope set is attempted (benefit of the doubt), not pre-emptively refused — deliberately asymmetric with FR-NEW-332.
- **FR-NEW-335**: `git.auth_status` reports scopes held and PR-surface sufficiency.

**N. Contract, counts and documentation**
- **FR-NEW-345 / FR-NEW-346 / FR-NEW-349**: exactly these 31 tools registered and no others (enumerated in the spec); frozen contract 63→94, git-family 18→49, `git.*` registry 14→45; five count-assertion sites updated, two others (`all.rs:64` admin-only, `:99` git-disabled) correctly untouched.
- **FR-NEW-347**: all 31 added to the blanket non-member/non-member-admin authorization tests.
- **FR-NEW-348**: `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/git.md` updated with the new counts and tool families.

**O. Wire-shape requirements (added at implementability audit round 1, authoritative over any earlier Outputs line)**
- **FR-NEW-186**: the conflict response has exactly one serialized shape (`status, operation, operation_id, source_ref, current_step, total_steps, conflicts[], continue_with, abort_with`; each conflict `{path, ours, theirs, base, binary, type_change}`; each side `{exists, content}`).
- **FR-NEW-187**: `current_step` is zero-based; `total_steps` is a count.
- **FR-NEW-188**: `git_operations.state` is closed to `conflicted`/`running` — `paused` is not valid.
- **FR-NEW-189**: branch tool response fields fixed exactly as FR-NEW-101/105/108/111 state.
- **FR-NEW-131 / FR-NEW-132**: stash entry and stash-apply/pop key sets fixed exactly.
- **FR-NEW-285**: op_type six-value closed set (restated above for cohesion).
- **FR-NEW-199**: one table pins every tool's exact response key set and closed `status` values (see Section 8.3/8.4 for the shapes; AMENDED post-implementation: `git.remote_push` reports `remote_sha` not `new_sha`, and unconditionally reports `remote`).
- **FR-NEW-286**: `git.status`'s `operation` object key set fixed (restated above).
- **FR-NEW-116**: branch-name length measured in UTF-8 bytes, not characters.

### Modified Requirements

- **FR-MOD-101 / FR-MOD-102**: `git.remote_push` accepts `remote` (default `origin`) and `remote_branch` (default = local name).
- **FR-MOD-103**: `git.remote_fetch`/`git.remote_pull` accept `remote` (default `origin`).
- **FR-MOD-104**: a diverged pull merges automatically (no conflict) or returns the shared conflict response (conflict), instead of refusing outright for want of `on_conflict`.
- **FR-MOD-105**: a conflicted pull is completed by `git.merge_resolve`/`git.merge_abort` (no pull-specific resolver).
- **FR-MOD-106**: `git.branches` marks the current branch and reports ahead/behind counts against the remote-tracking ref (`null` for both when no tracking ref).
- **FR-MOD-107**: `git.status` includes the `operation` object when in progress (restated above).
- **FR-MOD-108**: the git index's `TABLES` grows from 3 to 4 (restated above).
- **FR-MOD-109**: the GitLab scope constant includes `api` (restated above).

### Removed Requirements

- **FR-DEL-101**: the `on_conflict` parameter of `git.remote_pull` (and its global `ours`/`theirs` strategy) is removed outright, superseded by the shared conflict model. Deliberate, user-approved breaking change (DEC-908).

## 7. Non-Functional Requirements

**Performance**: rebase bounded to O(N) three-way merges by `git.max_rebase_todo` (default 200); `branch_switch`/`reset --hard` rewrite only differing paths, not charged for identical ones; `git.pr_diff` bounded by `git.max_pr_diff_mb` (default 12 MiB, truncated with a marker); no git call blocks the async request thread (`on_git_thread`); provider REST calls carry `git.provider_api_timeout_secs` (default 30).

**Security**: no token value ever surfaces (response, `raw`, error, log, tracing span — verified by a test inspecting every captured tracing field); scope checked before the network; authorization unchanged (membership only); no cross-host redirect followed; force push leased and audited; transport rules unchanged (HTTPS only, no URL credentials, host must be declared); no new secret.

**Usability**: tool names mirror the git CLI vocabulary (LLM affordance, DEC-904); every refusal names its remedy; error messages name concrete values (both lease shas, unresolved paths, the offending todo entry).

**Reliability**: a paused operation survives a restart; every volume write is all-or-nothing; abort is exact (restores pre-operation tip and bytes, not approximate); refs never advance before their files land; orphaned commits retained, never pruned, recoverable through the reported `old_sha`.

**Observability**: every destructive operation audited (branch delete/reset, reset, force push with overwritten sha, stash drop, rebase, revert, cherry-pick, merge); remote/provider ops emit the existing `git.remote` tracing span field set; tokens/credentials/Authorization header contents never traced. OTLP export remains out of scope (BL-001).

**Deployment**: no new service, no new infra, no new REST route, no new outbound dependency beyond provider REST (reused `reqwest`). New config keys, all optional with defaults: `git.max_stash_entries` (100), `git.max_rebase_todo` (200), `git.max_pr_diff_mb` (12), `git.provider_api_timeout_secs` (30), `git.github_scope` (`repo`), `git.gitlab_scope` (`api read_repository write_repository`).

**Scalability**: explicitly unchanged — the per-project write lock stays an in-process `tokio::sync::Mutex`; single-replica assumption retained; `git_operations` is visible to every replica but does not fix multi-replica coordination (BL-014 remains open).

## 8. E2E Tests

The full catalog lives in the archived spec's Section 12 (~536 new `E2E-NEW-4xx..9xx` tests plus 9 modified/removed shipped tests, `E2E-MOD-401..409`, `E2E-DEL-401..402`) and in the shipped Rust test functions under `crates/core/src/tools/git.rs` and `crates/core/src/tools/git_pr.rs`. Key properties pinned by that suite:

- Every scenario of Section 5 carries at least one happy, one failure and one edge/other test (Section 11 traceability matrix of the archived spec — zero gaps).
- Six shipped tests were removed or rewritten for `FR-DEL-101`/`FR-MOD-104` (`e2e_new_117`, `126` removed; `118`, `125`, `128`, `129` rewritten onto `git.merge_resolve`); a further ten shipped tests were adapted beyond that declared set and one additional test (`e2e_new_135`) was deleted as its subject (`on_conflict` value validation) no longer exists — see drift entry `2026-09-22_11-59-56` in Section 10.
- Five hardcoded count assertions updated (`contract_golden.rs:131` 63→94, `all.rs:78/:122/:141`, `git.rs:2500` 14→45); two correctly left untouched (`all.rs:64`, `:99`).
- The blanket authorization test (`a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool`) extended to all 31 new tools.
- A resumed-stash defect (conflicted stash resolution wrongly creating a merge commit) was caught during implementation (US-012) and fixed with three new tests pinning branch-tip identity and commit-count invariance across a resolved apply and pop, plus a guard test for the ordinary merge path (see drift entry `2026-09-22_14-24-41`).
- A wire-shape gap (`git.remote_push` missing the `remote` key entirely) was caught post-implementation and closed with a mechanical key-set equality check between the amended spec row and the emitted keys (see drift entry `2026-09-22_23-33-21`).

## 9. Glossary

| Term | Definition |
|---|---|
| Volume | The simulated filesystem for one project, also the git working tree; no separate checkout. |
| Index | Git's staging area — this server has none. |
| Dirty | The volume differs from HEAD in any added/modified/deleted path. |
| Combine operation | Any operation that merges two histories and can conflict: pull, merge, rebase, cherry-pick, revert, stash apply. |
| Conflict set | The paths an in-progress operation could not auto-merge. |
| In-progress operation | A combine operation paused awaiting resolution, recorded in `git_operations`. |
| Resolution | A caller's answer for one conflicting path: `ours`, `theirs`, or literal content. |
| Ours / theirs | `ours` is the side being merged into; `theirs` the side being applied (for a rebase, `ours` is the new base). |
| Todo list | The ordered pick/squash/drop/reword plan a rebase executes. |
| Step | One todo entry; a rebase pauses at a step boundary only. |
| Stash entry | A commit under `refs/stash/*` with the `base_sha` it was taken against. |
| Lease | The `expected_remote_sha` a force push must supply. |
| Remote-tracking ref | A ref under `refs/remotes/{remote}/*`. |
| Refspec | A `local:remote` mapping, supported for a single branch via `remote_branch`. |
| Orphaned commit | A commit no ref reaches, retained rather than pruned. |
| Host map | `git.hosts`, mapping a hostname to a provider. |
| Instance URL | Base URL of a self-hosted provider (GitHub Enterprise / self-hosted GitLab). |
| Scope | The permission set a token carries, validated before the network for PR operations. |
| Normalized model | The provider-independent pull-request shape returned by every `git.pr_*` tool. |
| Write lock | The per-project mutex serializing ref-mutating operations. |
| Frozen tool contract | `TOOL_CONTRACT.txt` + `tool-contract-golden.json`. |

## 10. Confidence notes

- **FR list**: all 162 FR ids (New/Modified/Removed, including the wire-shape subsection O) reproduced with their ids verbatim and condensed EARS text; business-rule detail kept where load-bearing, trimmed where purely restating the rule header. Nothing renumbered.
- **Prose purity**: file:line citations were stripped from scenario and FR prose where feasible; a handful of load-bearing citations (storage ceiling rationale for FR-NEW-116, the five count-assertion sites for FR-NEW-346) were kept because the requirement's substance is the specific site, not a general statement. This is the one deliberate shortcut against "FUNCTIONAL ONLY" prose purity, taken to finish within the size budget of a ~9200-line source document.
- **E2E test catalog**: not reproduced test-by-test (approx. 536 entries); Section 8 here summarizes structural properties and defers to the archived spec's Section 12 and the shipped Rust test functions as the source of truth, per the task's explicit prioritization (FR list and decisions log over full prose/test reproduction).
- **Traceability matrix**: not reproduced here; it lives unchanged in the archived spec's Section 11 and was verified by the stories index as gap-free (0 unassigned FRs/tests, 0 uncovered scenarios).
- **4 external drift files**: all four (`2026-09-22_07-49-37`, `11-59-56`, `14-24-41`, `23-33-21`) were read in full and mapped into design.md's Legacy mapping as resolved — none required a FINDINGS FOR BACKLOG entry because each one either closed as "accepted as-is" (US-007 drift, a documentation note not a code change) or was independently verified resolved in the current codebase (US-012's `Completion::WorkingTree` enum at `crates/core/src/tools/git.rs:5248`, and US-022's unconditional `"remote"` key emission at `crates/core/src/tools/git.rs:3470`).
- **Decisions Log**: all 18 decisions (DEC-901 through DEC-918) renumbered into design.md's `DEC-NNN` sequence and kept in full; none dropped.
