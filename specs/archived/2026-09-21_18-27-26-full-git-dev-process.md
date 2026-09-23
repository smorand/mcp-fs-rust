# Full Git Development Process — Specification Document

> Generated on: 2026-09-21
> Project: mcp-fs (Rust MCP filesystem server)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification
> Nature: FEAT
> Depth: L
> Depth evidence: existing project, but the change spans 5+ modules (git tool layer, git relational index, remote pipeline, OAuth token/scope model, two new provider REST clients), adds 31 tools and produces ~110 requirements. Well past the 3-module / 15-requirement threshold for L. No escalation or de-escalation occurred during the run.

## 1. Executive Summary

mcp-fs already ships a substantial git surface: 18 git-family tools, an encrypted
per-`(person, host)` OAuth token store with GitHub and GitLab device flow, a host map
(`git.hosts`) that resolves provider and instance URL, and a working remote pipeline for
clone, push, fetch and pull over HTTPS. What it does not ship is a *development process*.
A caller today cannot create a branch, switch branches, stash work, merge locally, rebase,
cherry-pick, reset, revert, force push, talk to more than one remote, or open and review a
pull request. Several of these were deliberately deferred to `specs/BACKLOG.md` under the
theme "Git full support" (BL-004 through BL-013); branch management and revert were never
captured at all.

This specification closes that gap in full. It adds 31 tools covering branch lifecycle,
stash, local merge and squash, interactive rebase, cherry-pick, reset, revert, arbitrary
named remotes, force push with lease, and a complete pull-request surface (create, list,
get, diff, merge, review) for GitHub and GitLab including their self-hosted deployments.
It replaces the current global `ours`/`theirs` merge strategy with a single **shared
conflict model** used by every operation that combines histories: the server auto-merges
what it can, and surfaces what it cannot so the caller decides, per file, either by
strategy or by supplying resolved content outright.

Two structural changes make the rest possible. First, a new `git_operations` relational
table records an operation that is paused mid-flight, so a conflicted merge or a
half-finished rebase survives a server restart and is visible to every member of the
project rather than living in one process's memory. Second, the GitLab device flow must
request the `api` scope: the scope it requests today grants Git-over-HTTP access only and
would leave every merge-request tool unauthorized.

The audience is the MCP client, which in practice is an LLM chat acting as an interactive
terminal on the developer's behalf. That framing drives several decisions recorded in
Section 17: tools are named after their git CLI counterparts because those names are the
model's primary affordance, and operations that a human would resolve interactively are
surfaced back to the caller rather than silently resolved by a global policy.

## 2. Current State Analysis

### 2.1 Project Overview

mcp-fs is a streamable-HTTP MCP server exposing a simulated multi-project filesystem
(`AGENTS.md:3-12`). Git support is optional, gated behind `git.enabled`, and registers 18
tools: 14 in `crates/mcp-fs/src/tools/git.rs:59-360` and 4 auth tools in
`crates/mcp-fs/src/tools/git_auth.rs:61-138`. The frozen tool contract totals 63 tools
(35 `fs.*`, 10 `admin.*`, 18 git-family), asserted at
`crates/mcp-fs/src/tools/contract_golden.rs:131`.

Architecture facts that constrain this specification:

| Fact | Citation |
|---|---|
| The volume **is** the working tree. There is no index and no staging step; `git.commit` commits the whole volume state. | `crates/mcp-fs/src/tools/git.rs:196-222` |
| Git objects live in the blob store under key `git:{sha}`; the bare repo on disk is a rebuildable cache, not the source of truth. | `crates/mcp-fs/src/git/odb.rs:3-28`, `AGENTS.md` git section |
| The git relational index owns exactly three tables, named once so a purge cannot miss one. | `crates/mcp-fs/src/git/db.rs:42` (`pub const TABLES: [&str; 3]`) |
| A per-project write lock serializes ref-mutating operations. | `crates/mcp-fs/src/git/repo.rs:44`, taken at `tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770` (five sites) |
| Every git tool authorizes on project membership; platform admin confers no implicit access. | `crates/mcp-fs/src/tools/git.rs:6-8` |
| Volume writes during a pull are atomic, all-or-nothing. | `apply_pull_changes_atomically`, `crates/mcp-fs/src/tools/git.rs:2079-2127` |
| Git is **MCP-only**. All 38 REST routes are `fs.*` (`REST_ROUTES.len() == 38`, asserted at `crates/mcp-fs/src/api/dataplane.rs:2328`). `api/openapi.rs:74-76` documents the git smart-HTTP wire routes from `git_paths()` (`:314`), which are for the git CLI, not the `/api/fs` plane. | `crates/mcp-fs/src/api/dataplane.rs:45`, `:2328` |
| `refs/stash` is an unused namespace in the tree. | verified: zero occurrences under `crates/` |

### 2.2 Existing Specifications

| Spec | Scope |
|---|---|
| `2026-09-18_17-37-46-platform-foundation.md` | Server architecture, request lifecycle, safety, error codes, multi-tenancy |
| `2026-09-18_18-05-00-filesystem-engine.md` | The 35 `fs.*` tools |
| `2026-09-18_18-30-00-rest-data-plane.md` | The `/api/fs` REST plane and OpenAPI |
| `2026-09-18_18-55-00-multi-backend-storage.md` | Relational layer, dialects, pooling |
| `2026-09-18_19-20-00-documents.md` | Document extraction and the doc service |
| `2026-09-18_19-45-00-git.md` | Git objects, HTTP smart protocol, the original git tool surface |
| `2026-09-18_20-10-00-search-and-rag.md` | Search, BM25, RAG |
| `2026-09-18_20-35-00-optional-integrations.md` | Optional external integrations |
| `specs/archived/2026-09-21_00-34-13-github-enterprise-and-token-store.md` | Host map, per-host token store, remote pipeline, pull with `on_conflict`. **Implemented**; its 19 stories are in `specs/stories/`. |

This specification supersedes part of the archived spec's pull behaviour. Per project
rule, that spec is **not modified**; the deviation is documented in Section 13.

### 2.3 Relevant Architecture

The pull path is the template the new combine operations follow. It resolves credentials
before touching the network, takes the write lock, fetches, and then either fast-forwards
or performs a three-way merge via libgit2 `merge_commits` with `MergeOptions::file_favor`
set from `on_conflict` (`crates/mcp-fs/src/tools/git.rs:1835-1839`), refusing outright if
the merge still has conflicts (`:1841-1843`). Resolution is global: one strategy applies
to every conflicting file. That merge block and `apply_pull_changes_atomically` are
self-contained and are the reuse surface for local merge, rebase and cherry-pick.

## 3. Scope

### 3.1 In Scope

- **Branch lifecycle**: create, switch, delete, and force-move a branch pointer to an arbitrary commit.
- **Stash**: save, list, apply, pop, drop, stored as `refs/stash/*`.
- **Local merge**: two-parent merge and squash merge, with resolve and abort.
- **Shared conflict model**: one conflict response shape and one resolution mechanism reused by pull, merge, rebase, cherry-pick and stash apply/pop. Resolution per file by strategy or by caller-supplied content.
- **Interactive rebase**: an ordered todo list of pick / squash / drop / reword, executed commit by commit, pausing on conflict, resumable and abortable.
- **Cherry-pick**, with continue and abort.
- **Reset**, in `soft` and `hard` modes only.
- **Revert**, including merge commits with explicit mainline selection.
- **Remotes**: add, remove, list; a `remote` parameter on push/fetch/pull defaulting to `origin`; a `remote_branch` parameter enabling a `local:remote` refspec.
- **Force push** gated behind a mandatory lease (`expected_remote_sha`).
- **Pull requests / merge requests**: create, list, get (including review and check state), diff, merge with strategy, and post a review, for GitHub and GitLab including GitHub Enterprise Server and self-hosted GitLab.
- **OAuth scope**: extend the GitLab request to include `api`; validate required scope before any network call.
- **Operation state**: a new `git_operations` table recording a paused operation, reported by `git.status` and enforced as a guard against interleaved operations.
- Closes backlog items BL-004, BL-005, BL-006, BL-008, BL-009, BL-010 (via reset/stash), BL-007 in the form agreed at interview, and resolves part of the BL-013 divergence register.

### 3.2 Out of Scope (Non-Goals)

- **Multi-replica coordination** (BL-003, BL-014). The per-project write lock remains an in-process `tokio::sync::Mutex` (`crates/mcp-fs/src/git/repo.rs:44`). The new `git_operations` table improves the story but does not solve it. Unchanged assumption: one server replica.
- **Azure DevOps** (BL-012) and **pluggable credential providers** (BL-011). Every provider still authenticates as `oauth2:<token>`.
- **Non-HTTPS transports.** SSH, `git://`, `file://` and local paths remain rejected.
- **Submodules**, **tag creation and push**, **`git gc` / explicit object pruning**, **partial clone**, **worktrees**, **hooks**, **notes**, **bisect**.
- **Conflict markers in the volume.** The prohibition is retained, not relaxed.
- **REST exposure of git.** Git stays MCP-only; no route is added to `/api/fs`.
- **A staging area.** There is still no index; `git.commit` still commits the whole volume state. `git add` and `git rm` have no equivalent and need none.
- **Interactive rebase actions beyond pick / squash / drop / reword** (no `edit`, `fixup`, `exec`, `break`).

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **Caller** | The MCP client: in practice an LLM chat acting as an interactive terminal on a developer's behalf. Issues every tool call. Authenticated as a `person` and must be a member of the project behind `mount_id`. |
| **Project member** | A `person` with membership on the project. All git tools require it. Members share one volume and one git history, so one member can encounter and resolve an operation another member paused. |
| **Platform admin** | Manages projects and membership. Explicitly gets **no** implicit file or git access (`crates/mcp-fs/src/tools/git.rs:6-8`). Unchanged by this specification. |
| **Provider** | GitHub or GitLab, including GitHub Enterprise Server and self-hosted GitLab. External system reached over HTTPS for git transport and, newly, for the pull-request REST API. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Volume** | The simulated filesystem that is also the git working tree. Owns file bytes and paths. | Node, blob, path, write quota, audit entry |
| **Git object store** | Content-addressed git objects and refs for one volume. | Commit, tree, blob object, ref, branch, tag, stash ref |
| **Git operation** | A multi-step history operation that can pause. New in this specification. | Operation record, todo entry, step index, conflict set, resolution |
| **Remote** | A named HTTPS endpoint and the refs mirrored from it. | Remote, remote-tracking ref, lease, push/fetch result |
| **Identity and credentials** | Who the caller is and which token is used against which host. | Person, host, provider, token, scope set, expiry |
| **Provider collaboration** | The provider's review workflow, reached over REST, not over git transport. | Pull request / merge request, review, check state, merge strategy |

Term collision worth naming: **"merge"** means a two-parent commit in the *Git object
store* context, an operation that can pause in the *Git operation* context, and a
provider-side action in the *Provider collaboration* context. The tool names keep them
apart: `git.merge`, the `git_operations` row with `op_type='merge'`, and `git.pr_merge`.

## 5. Usage Scenarios

Cross-cutting exceptions apply to every scenario below and are not repeated in each:
caller is not a project member (`ERR_FORBIDDEN`), caller is unauthenticated
(`ERR_UNAUTHENTICATED`), `mount_id` unknown (`ERR_PROJECT_NOT_FOUND`), git disabled for
the deployment (tool not registered), write quota exhausted
(`ERR_WRITE_QUOTA_EXCEEDED`), and for remote operations: host not declared in `git.hosts`,
token missing, token expired, or required scope absent, each rejected **before** any
network call.

### SC-901: Branch lifecycle
**Actor:** Caller
**Preconditions:** Repository initialized, volume clean, HEAD on `main` with at least one commit `C1`.
**Flow:**
1. Caller calls `git.branch_create` with `name: "feature/x"`, `start_point: "main"`. Server creates `refs/heads/feature/x` at `C1` and does not move HEAD.
2. Caller calls `git.branch_switch` with `name: "feature/x"`. Server rewrites the volume to that branch's tree and moves HEAD.
3. Caller edits files and calls `git.commit`. A new commit `C2` lands on `feature/x`.
4. Caller calls `git.branch_switch` with `name: "main"`. Volume returns to `C1`'s tree; `feature/x` still points at `C2`.
5. Caller calls `git.branch_delete` with `name: "feature/x"`, `force: true`. The ref is removed; `C2` becomes unreachable but is not destroyed.
**Postconditions:** `main` unchanged at `C1`; `refs/heads/feature/x` absent; volume matches `C1`'s tree.
**Exceptions:**
- **EXC-901a**: `name` already exists → rejected, naming the existing branch.
- **EXC-901b**: switch target does not exist → `ERR_NOT_FOUND`.
- **EXC-901c**: switch with a dirty volume → rejected, naming commit or stash as the remedy; volume and HEAD untouched.
- **EXC-901d**: delete the currently checked-out branch → rejected.
- **EXC-901e**: delete a branch holding commits reachable from no other ref, without `force` → rejected, naming `force`.
- **EXC-901f**: switch to the branch already checked out → success, no-op, volume untouched.
**Cross-scenario notes:** blocked while an operation is in progress (SC-929).

### SC-902: Stash to switch away from dirty work
**Actor:** Caller
**Preconditions:** HEAD on `feature/x` at `C2`; volume has uncommitted modifications.
**Flow:**
1. Caller calls `git.stash_save` with `message: "wip"`. Server captures the diff against HEAD as a stash entry under `refs/stash/*` and reverts the volume to `C2`'s tree.
2. Caller calls `git.branch_switch` to `main`, which now succeeds because the volume is clean.
3. Caller does unrelated work and commits.
4. Caller calls `git.branch_switch` back to `feature/x`.
5. Caller calls `git.stash_pop` with the `stash_id`. The diff is reapplied to the volume and the entry is removed.
**Postconditions:** Volume on `feature/x` holds the stashed modifications again; the stash entry is gone; no commit was created by the stash round trip.
**Exceptions:**
- **EXC-902a**: `git.stash_save` on a clean volume → distinct error stating there is nothing to stash; no ref created.
- **EXC-902b**: unknown `stash_id` on apply/pop/drop → `ERR_NOT_FOUND`.
- **EXC-902c**: the stashed changes no longer apply cleanly → the shared conflict model (SC-904); the entry is **not** dropped until resolution completes.
- **EXC-902d**: `git.stash_apply` succeeds → the entry remains listed, unlike pop.
**Cross-scenario notes:** the stash pool is per volume, not per branch, and outlives the branch it was taken from (SC-930).

### SC-903: Clean local merge
**Actor:** Caller
**Preconditions:** HEAD on `main`; `feature/x` holds commits absent from `main`; the two branches changed disjoint files.
**Flow:** Caller calls `git.merge` with `source_ref: "feature/x"`. Server auto-merges, creates a commit with exactly two parents, and updates the volume to the merged tree.
**Postconditions:** `main` advanced to the merge commit; `feature/x` untouched; volume matches the merged tree.
**Exceptions:**
- **EXC-903a**: `feature/x` is already fully contained in `main` → reported as an explicit no-op, not an error; no commit created.
- **EXC-903b**: `source_ref` unknown → `ERR_NOT_FOUND`.
- **EXC-903c**: dirty volume → rejected before any merge work.

### SC-904: Merge conflict resolved by per-file strategy
**Actor:** Caller
**Preconditions:** As SC-903, but both branches modified the same lines of `/src/a.txt`.
**Flow:**
1. Caller calls `git.merge` with `source_ref: "feature/x"`. Server detects a genuine conflict, applies **nothing** to the volume, creates no commit, records an in-progress operation, and returns a conflict response listing `/src/a.txt` with the ours side, the theirs side and the common ancestor side.
2. Caller calls `git.merge_resolve` with `resolutions: {"/src/a.txt": "theirs"}`.
3. Server applies the chosen side, creates the two-parent merge commit, updates the volume, and clears the in-progress record.
**Postconditions:** Merge commit exists; volume matches it; no `git_operations` row remains; no conflict marker was ever written to any file.
**Exceptions:**
- **EXC-904a**: `git.merge_resolve` naming a path that is not in conflict → rejected.
- **EXC-904b**: only some conflicting paths resolved → operation stays in progress, response lists what remains.
- **EXC-904c**: `git.merge_abort` → volume and HEAD restored exactly to pre-merge state, record cleared.
- **EXC-904d**: resolution strategy value other than `ours`/`theirs` → `ERR_INVALID_ARGUMENT`.

### SC-905: Merge conflict resolved by supplied content
**Actor:** Caller
**Preconditions:** As SC-904, operation paused with `/src/a.txt` in conflict.
**Flow:** Caller calls `git.merge_resolve` with `resolutions: {"/src/a.txt": {"content": "<exact merged bytes>"}}`. The supplied bytes become the file's content in the merge commit.
**Postconditions:** Merge commit holds exactly the supplied bytes for that path.
**Exceptions:**
- **EXC-905a**: content supplied for a non-conflicting path → rejected.
- **EXC-905b**: strategy and content mixed across different paths in one call → accepted, applied per path.

### SC-906: Local squash merge
**Actor:** Caller
**Preconditions:** As SC-903; `feature/x` holds three commits.
**Flow:** Caller calls `git.merge` with `source_ref: "feature/x"`, `squash: true`. Server produces a single commit on `main` holding the cumulative diff.
**Postconditions:** Exactly one commit added, with exactly one parent; `feature/x`'s three commits do not appear in `main`'s log; `feature/x` unchanged.
**Exceptions:**
- **EXC-906a**: conflicts during a squash → same conflict model; the eventual commit is still single-parent.

### SC-907: Second remote
**Actor:** Caller
**Preconditions:** Repository cloned, so `origin` exists.
**Flow:** Caller calls `git.remote_add` (`name: "upstream"`), then `git.remote_fetch` with `remote: "upstream"`, then `git.remote_push` with `remote: "upstream"`, then `git.remote_remove` (`name: "upstream"`).
**Postconditions:** During the sequence only `refs/remotes/upstream/*` is written by the fetch; after removal the remote and its tracking refs are gone and `origin` is untouched.
**Exceptions:**
- **EXC-907a**: duplicate remote name → rejected. The storage call is an upsert (`crates/mcp-fs/src/git/db.rs:281-285`), so the check must live in the tool layer.
- **EXC-907b**: non-HTTPS URL → rejected, existing rule retained.
- **EXC-907c**: removing an unknown remote → `ERR_NOT_FOUND`. The storage delete is unconditional (`crates/mcp-fs/src/git/db.rs:287-296`), so the existence check must live in the tool layer.
- **EXC-907d**: operating on a `remote` name that does not exist → `ERR_NOT_FOUND` before any network call.

### SC-908: Push under a different remote branch name
**Actor:** Caller
**Preconditions:** Local branch `feature/x` exists; remote has no branch by that name.
**Flow:** Caller calls `git.remote_push` with `branch: "feature/x"`, `remote_branch: "feature/x-for-review"`. Server creates that branch on the remote from the local tip.
**Postconditions:** Remote holds `feature/x-for-review`; local refs unchanged apart from the remote-tracking ref.
**Exceptions:**
- **EXC-908a**: the named remote branch exists and has diverged → non-fast-forward refusal, identical to the same-name case; force-with-lease applies identically.

### SC-909: Force push accepted
**Actor:** Caller
**Preconditions:** Local history was rewritten (for example by SC-914), so a normal push is non-fast-forward. The caller observed the remote tip sha `R1` via a prior fetch or status.
**Flow:** Caller calls `git.remote_push` with `force: true`, `expected_remote_sha: "R1"`. Server confirms the remote branch still points at `R1`, then force-updates it.
**Postconditions:** Remote branch points at the local tip; the commits previously only on the remote become unreachable there; an audit entry records the overwritten sha `R1`.
**Exceptions:**
- **EXC-909a**: `force: true` without `expected_remote_sha` → rejected outright. The lease is mandatory, not optional.
- **EXC-909b**: `expected_remote_sha` supplied with `force: false` → rejected as contradictory, or ignored-with-error rather than silently dropped.

### SC-910: Force push rejected by the lease
**Actor:** Caller
**Preconditions:** As SC-909, but a third party pushed to the remote branch after the caller observed `R1`; the remote now points at `R2`.
**Flow:** Caller calls `git.remote_push` with `force: true`, `expected_remote_sha: "R1"`. Server observes `R2`, rejects.
**Postconditions:** The remote is completely unchanged. The error names both the expected sha `R1` and the actual sha `R2`.

### SC-911: Divergent pull that merges automatically
**Actor:** Caller
**Preconditions:** Local and remote `main` diverged; the two sides changed disjoint files; volume clean.
**Flow:** Caller calls `git.remote_pull` with `branch: "main"`. Server fetches, finds divergence, auto-merges without conflict, commits and updates the volume, with no caller decision required.
**Postconditions:** Merge commit created; volume matches it. Notably this differs from today, where a divergent pull is refused unless `on_conflict` is supplied.

### SC-912: Divergent pull with a genuine conflict, resolved by strategy
**Actor:** Caller
**Preconditions:** As SC-911 but both sides changed the same lines of one file.
**Flow:** `git.remote_pull` returns the same conflict response shape as `git.merge`, applies nothing, records the in-progress operation. Caller completes it with `git.merge_resolve` — the same tool as a local merge, not a pull-specific one.
**Postconditions:** Fetched objects and remote-tracking refs are retained even while the pull is unfinished; the merge commit appears only after resolution.

### SC-913: Divergent pull conflict resolved by supplied content
**Actor:** Caller
**Flow:** As SC-912, resolved with explicit content per SC-905.

### SC-914: Simple rebase with no conflicts
**Actor:** Caller
**Preconditions:** HEAD on `feature/x` holding `C2`, `C3`; `main` advanced to `C9` since the branch point.
**Flow:** Caller calls `git.rebase` with `onto: "main"` and a todo list of `pick C2`, `pick C3`. Server replays both onto `C9` in order.
**Postconditions:** `feature/x` holds two new commits with new shas whose ancestor is `C9`; the original `C2`/`C3` become unreachable; volume matches the new tip.
**Exceptions:**
- **EXC-914a**: `onto` is already an ancestor and nothing to replay → reported no-op.
- **EXC-914b**: todo references a sha outside the rebase range, or an unknown sha → rejected **before any work happens**.
- **EXC-914c**: todo omits a commit in the range → rejected.
- **EXC-914d**: todo begins with `squash` (nothing to fold into) → rejected.
- **EXC-914e**: todo longer than the configured bound → rejected.

### SC-915: Interactive rebase with squash and reword
**Actor:** Caller
**Flow:** Caller supplies `pick C2`, `squash C3`, `reword C4` with a new message. Server folds `C3` into `C2`, and replays `C4` under the new message.
**Postconditions:** Resulting history has one fewer commit than the input range; the reworded commit carries exactly the supplied message; trees are unchanged by the reword.
**Exceptions:**
- **EXC-915a**: `reword` with an empty message → rejected.
- **EXC-915b**: all entries `drop` → the branch becomes equal to `onto`; permitted and reported.

### SC-916: Rebase conflict, pause, resolve, continue
**Actor:** Caller
**Preconditions:** A four-entry todo where the second and fourth commits both conflict.
**Flow:**
1. `git.rebase` replays entry one, conflicts on entry two, and pauses. The in-progress record persists the step index and the remaining todo.
2. Caller resolves and calls `git.rebase_continue`. Server resumes at exactly that step, replays entry three, then conflicts on entry four and pauses again.
3. Caller resolves and calls `git.rebase_continue` a second time. The rebase completes.
**Postconditions:** All four entries processed; exactly one `git_operations` row existed throughout, and none remains at the end.
**Exceptions:**
- **EXC-916a**: `git.rebase_continue` with conflicts still unresolved → rejected, listing what remains.
- **EXC-916b**: `git.cherry_pick_continue` called while a *rebase* is paused → rejected; operations do not share a continue channel.
- **EXC-916c**: server restarts while paused → the record survives and the rebase is still resumable.

### SC-917: Rebase abort
**Actor:** Caller
**Flow:** Mid-pause, caller calls `git.rebase_abort`.
**Postconditions:** Branch tip sha and every volume file byte equal their exact pre-rebase values; the in-progress record is cleared; no partial commit remains reachable.

### SC-918: Cherry-pick without conflict
**Actor:** Caller
**Preconditions:** HEAD on `main`; commit `F1` exists on another branch.
**Flow:** Caller calls `git.cherry_pick` with `commit_sha: F1`. Server applies its diff as a new commit on `main`.
**Postconditions:** New commit with a new sha; the original author is preserved and the committer is the caller; `F1` remains where it was.
**Exceptions:**
- **EXC-918a**: `F1` already present in this branch's history → reported rather than silently duplicated.
- **EXC-918b**: unknown sha → `ERR_NOT_FOUND`.

### SC-919: Cherry-pick with conflict
**Flow:** As SC-918 but the diff does not apply cleanly; the shared conflict model handles it, completed by `git.cherry_pick_continue` or undone by `git.cherry_pick_abort`.

### SC-920: Reset soft
**Actor:** Caller
**Flow:** Caller calls `git.reset` with `target_ref` an earlier commit and `mode: "soft"`. The current branch pointer moves; the volume is left untouched.
**Postconditions:** Branch tip is the target; volume still holds the previous content, which is now ahead of the pointer; commits between old and new tip are unreachable from this branch.

### SC-921: Reset hard
**Actor:** Caller
**Flow:** Caller calls `git.reset` with `mode: "hard"`. The pointer moves **and** the volume is overwritten to the target tree.
**Postconditions:** Uncommitted changes are gone; commits above the target are orphaned but not immediately destroyed.
**Exceptions:**
- **EXC-921a**: `mode` absent or a value other than `soft`/`hard` → `ERR_INVALID_ARGUMENT`. There is deliberately no `mixed` mode.
- **EXC-921b**: unknown `target_ref` → `ERR_NOT_FOUND`, pointer unmoved.
- **EXC-921c**: reset to the current tip → no-op success.

### SC-922: Revert a normal commit
**Actor:** Caller
**Flow:** Caller calls `git.revert` with `commit_sha: C3`. Server creates a new commit whose diff is the inverse of `C3`.
**Postconditions:** `C3` remains in history; a new commit above it undoes it; reverting that revert restores the original change.
**Exceptions:**
- **EXC-922a**: the inverse diff does not apply cleanly → shared conflict model.
- **EXC-922b**: reverting the initial commit (no parent) → handled explicitly rather than panicking.

### SC-923: Revert a merge commit
**Actor:** Caller
**Flow:** Caller calls `git.revert` with a merge commit sha and `mainline: 1`. The revert is computed against that parent.
**Exceptions:**
- **EXC-923a**: merge commit without `mainline` → rejected with a message explaining a mainline parent must be chosen.
- **EXC-923b**: `mainline` out of range for that commit's parent count → rejected.
- **EXC-923c**: `mainline` supplied for a non-merge commit → rejected.

### SC-924: Force-move a branch pointer
**Actor:** Caller
**Preconditions:** Branch `release` points at `C5`; commit `C9` exists.
**Flow (not checked out):** Caller calls `git.branch_reset` with `name: "release"`, `target_commit: "C9"`, `force: true`. Only the ref moves; the volume is untouched because that branch is not checked out.
**Flow (checked out):** The same call naming the currently checked-out branch moves the ref **and** rewrites the volume, equivalent to a hard reset.
**Postconditions:** `refs/heads/release` points at `C9`; commits formerly reachable only from `C5` are orphaned.
**Exceptions:**
- **EXC-924a**: move that is not a fast-forward, without `force` → rejected.
- **EXC-924b**: `target_commit` unknown or never fetched → `ERR_NOT_FOUND`.
- **EXC-924c**: the branch is checked out and the volume is dirty → rejected unless the caller accepts the discard, consistent with SC-921.

### SC-925: Create a pull request
**Actor:** Caller
**Preconditions:** `feature/x` pushed to a remote whose host resolves to provider `github` or `gitlab`; a valid token with sufficient scope for that host.
**Flow:** Caller calls `git.pr_create` with `base: "main"`, `head: "feature/x"`, a title and a body. Server resolves the provider and API base URL from the remote's host and the stored `instance_url`, calls the provider's REST API, and returns the normalized pull-request model.
**Postconditions:** The PR/MR exists on the provider; the response carries its number and URL in provider-independent field names.
**Exceptions:**
- **EXC-925a**: `head` not present on the remote → rejected **before** the API call.
- **EXC-925b**: host provider is `generic` or `anonymous` → `ERR_NOT_SUPPORTED`, naming why.
- **EXC-925c**: token lacks the required scope → rejected before the API call, naming the missing scope and the tool that grants it.
- **EXC-925d**: provider rejects (protected branch, duplicate PR, permission) → surfaced faithfully, never swallowed, and never containing the token.

### SC-926: List pull requests
**Flow:** Caller calls `git.pr_list` with `state: "open"`. Server returns the normalized list.
**Exceptions:**
- **EXC-926a**: unsupported `state` value → validated before the call.
- **EXC-926b**: no pull requests → empty list, not an error.

### SC-927: Get detail, diff, and post a review
**Actor:** Caller acting as reviewer
**Flow:** Caller calls `git.pr_get` for title, body, state, base, head, author, commits, changed files, review state and check state; then `git.pr_diff` for the unified diff; then `git.pr_review` with `verdict: "approve" | "request_changes" | "comment"` and an optional body.
**Postconditions:** The review is recorded on the provider and visible on a subsequent `git.pr_get`.
**Exceptions:**
- **EXC-927a**: unknown PR number → `ERR_NOT_FOUND`.
- **EXC-927b**: provider forbids the review (reviewing one's own PR, insufficient permission) → surfaced faithfully.
- **EXC-927c**: a sub-call for check state fails → surfaced as a failure rather than silently degraded to "unknown".

### SC-928: Merge a pull request
**Flow:** Caller calls `git.pr_merge` with `strategy: "merge" | "squash" | "rebase"`. The provider performs the merge server-side.
**Postconditions:** The PR is merged on the provider. Local refs are **not** updated; remote-tracking refs stay stale until the caller fetches.
**Exceptions:**
- **EXC-928a**: required checks failing or reviews missing per provider policy → provider's rejection surfaced.
- **EXC-928b**: the strategy is disabled in the repository's settings → provider's error surfaced.
- **EXC-928c**: already merged or closed → surfaced, not treated as success.

### SC-929: An operation is already in progress
**Actor:** Caller
**Preconditions:** A merge or rebase is paused awaiting resolution on this volume.
**Flow:** Caller calls any other ref-mutating tool (`git.commit`, `git.branch_switch`, `git.remote_pull`, a second `git.merge` or `git.rebase`, `git.reset`, `git.cherry_pick`). Each is rejected with an error naming the active operation and the tools that can finish it.
**Postconditions:** Only the active operation's own continue/resolve/abort tools may mutate the volume. Read-only tools (`git.status`, `git.log`, `git.diff`, `git.branches`) remain available and `git.status` reports the active operation and its step.
**Cross-scenario notes:** Because authorization is membership-based with no per-operation ownership, another member may resolve or abort the operation.

### SC-930: A stash outlives its context
**Actor:** Caller
**Flow (a):** The branch a stash was taken from is deleted. The stash entry remains listable and applicable, because it references a base commit and a diff, not a branch. Applying it targets whatever branch is currently checked out.
**Flow (b):** A stash is popped onto a branch where the underlying file changed incompatibly. The shared conflict model handles it; the entry is retained until resolution succeeds, so no work is silently lost.
## 6. Functional Requirements

EARS notation is mandatory. Forbidden modals (`should`, `may`, `could`, `might`, `would`)
do not appear. Every tool below takes a required `mount_id` and authorizes on project
membership before anything else; that universal rule is stated once as FR-NEW-100 rather
than repeated on each tool.

| Tag | Pattern |
|-----|---------|
| `[EARS-U]` | The `<system>` SHALL `<response>` |
| `[EARS-E]` | WHEN `<trigger>` THE `<system>` SHALL `<response>` |
| `[EARS-S]` | WHILE `<state>` THE `<system>` SHALL `<response>` |
| `[EARS-O]` | IF `<condition>` THEN THE `<system>` SHALL `<response>` |
| `[EARS-UB]` | The `<system>` SHALL NOT `<unwanted behaviour>` |
| `[EARS-X]` | Free-form escape |

### New Requirements

#### FR-NEW-100 [EARS-E]: Universal authorization and normalization on every new git tool
> WHEN any tool introduced by this specification is called THE mcp-fs server SHALL authorize the caller against project membership for `mount_id` before performing any other work, and SHALL normalize every supplied path through `safety.normalize_path`.

- **Inputs:** `mount_id` (string, required on every tool), caller identity.
- **Outputs:** `ERR_FORBIDDEN` for a non-member, `ERR_UNAUTHENTICATED` when unidentified, `ERR_PROJECT_NOT_FOUND` for an unknown mount.
- **Business Rules:** Platform admin role confers no implicit access (`crates/mcp-fs/src/tools/git.rs:6-8`). No new tool bypasses the pattern.
- **Priority:** Must-have

---
#### A. Branch lifecycle

#### FR-NEW-101 [EARS-E]: Create a branch
> WHEN `git.branch_create` is called with `name` and `start_point` THE mcp-fs server SHALL create ref `refs/heads/{name}` pointing at the commit `start_point` resolves to.

- **Inputs:** `mount_id`, `name` (string), `start_point` (string ref or sha, optional, defaults to current HEAD), `checkout` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, sha, checked_out}`.
- **Business Rules:** HEAD does not move unless `checkout` is `true`. Creation takes the per-project write lock.
- **Priority:** Must-have

#### FR-NEW-102 [EARS-O]: Reject a duplicate branch name
> IF `name` already exists as a branch THEN THE mcp-fs server SHALL reject `git.branch_create` with `ERR_INVALID_ARGUMENT` naming the existing branch, and SHALL NOT move the existing ref.

- **Priority:** Must-have

#### FR-NEW-103 [EARS-O]: Reject an unresolvable start point
> IF `start_point` resolves to no commit THEN THE mcp-fs server SHALL reject `git.branch_create` with `ERR_NOT_FOUND` naming the unresolved value.

- **Priority:** Must-have

#### FR-NEW-104 [EARS-E]: Validate branch names
> WHEN a branch name is supplied to any tool THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` any name that is empty, exceeds 255 bytes, contains an ASCII control character, a space, `~`, `^`, `:`, `?`, `*`, `[`, `\`, or the sequence `..`, begins or ends with `/`, has any `/`-separated component beginning with `.`, or ends with `.lock`.

- **Business Rules:** Mirrors `git check-ref-format`. Valid UTF-8 beyond ASCII is accepted. The limit is **255 bytes of UTF-8, not 255 characters**; a multibyte name is measured after encoding. The 255-byte ceiling keeps `refs/heads/<name>` inside the `TextKey(400)` storage ceiling of `git_refs.name` (`crates/mcp-fs/src/git/db.rs:22`) on every backend, including SQL Server where that renders as `NVARCHAR(400)` (`crates/mcp-fs/src/storage/rel/dialect.rs:235`).
- **Priority:** Must-have

#### FR-NEW-105 [EARS-E]: Switch branches
> WHEN `git.branch_switch` is called with `name` THE mcp-fs server SHALL point HEAD at `refs/heads/{name}` and SHALL rewrite the volume so its contents equal that branch's commit tree exactly.

- **Inputs:** `mount_id`, `name`.
- **Outputs:** exactly `{branch, sha, changed, files_changed}`, where `changed` is a boolean stating whether the volume was rewritten and `files_changed` is the count of paths written plus removed.
- **Business Rules:** Files present only on the source branch are removed; files present only on the target are created; files differing are overwritten. The rewrite is atomic: a failure part-way leaves the volume as it was. Bytes written are charged to the write quota and audited.
- **Priority:** Must-have

#### FR-NEW-106 [EARS-O]: Refuse to switch with a dirty volume
> IF the volume holds uncommitted changes relative to HEAD THEN THE mcp-fs server SHALL reject `git.branch_switch` with `ERR_INVALID_ARGUMENT` naming `git.commit` and `git.stash_save` as remedies, and SHALL leave the volume and HEAD untouched.

- **Business Rules:** "Dirty" means any modified, added or deleted path relative to HEAD, matching the existing pull dirty check.
- **Priority:** Must-have

#### FR-NEW-107 [EARS-O]: Switching to the current branch is a no-op
> IF `name` is already the checked-out branch THEN THE mcp-fs server SHALL return success without writing to the volume and with `files_changed` both zero.

- **Priority:** Should-have

#### FR-NEW-108 [EARS-E]: Delete a branch
> WHEN `git.branch_delete` is called with `name` THE mcp-fs server SHALL remove ref `refs/heads/{name}`.

- **Inputs:** `mount_id`, `name`, `force` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, sha, forced}`, where `sha` is the tip the deleted branch pointed at and `forced` reports whether `force` was required.
- **Business Rules:** The commits become unreachable but are not destroyed; no object is pruned by this operation.
- **Priority:** Must-have

#### FR-NEW-109 [EARS-O]: Refuse to delete the checked-out branch
> IF `name` is the currently checked-out branch THEN THE mcp-fs server SHALL reject `git.branch_delete` with `ERR_INVALID_ARGUMENT` stating that the caller must switch away first.

- **Priority:** Must-have

#### FR-NEW-110 [EARS-O]: Refuse to delete an unmerged branch without force
> IF `name` holds commits reachable from no other ref and `force` is `false` THEN THE mcp-fs server SHALL reject `git.branch_delete` with `ERR_INVALID_ARGUMENT` naming `force` as the override and naming each commit that the deletion leaves unreachable.

- **Priority:** Must-have

#### FR-NEW-111 [EARS-E]: Force-move a branch pointer
> WHEN `git.branch_reset` is called with `name` and `target_commit` THE mcp-fs server SHALL set ref `refs/heads/{name}` to that commit.

- **Inputs:** `mount_id`, `name`, `target_commit`, `force` (boolean, optional, default `false`).
- **Outputs:** exactly `{branch, old_sha, new_sha, checked_out, files_changed}`, where `checked_out` states whether this branch was the checked-out one and therefore whether the volume was rewritten.
- **Business Rules:** This is the "replug" operation equivalent to `git branch -f`. `old_sha` is always reported so the prior tip is recoverable.
- **Priority:** Must-have

#### FR-NEW-112 [EARS-O]: Branch-reset rewrites the volume only for the checked-out branch
> IF `name` is the currently checked-out branch THEN THE mcp-fs server SHALL additionally rewrite the volume to `target_commit`'s tree and SHALL report `checked_out` as `true`; otherwise it SHALL leave the volume untouched and report `checked_out` as `false`.

- **Priority:** Must-have

#### FR-NEW-113 [EARS-O]: Refuse a non-fast-forward branch move without force
> IF the current tip of `name` is not an ancestor of `target_commit` and `force` is `false` THEN THE mcp-fs server SHALL reject `git.branch_reset` with `ERR_INVALID_ARGUMENT` naming `force` and naming each commit that the move leaves orphaned.

- **Priority:** Must-have

#### FR-NEW-114 [EARS-O]: Reject an unknown branch-reset target
> IF `target_commit` resolves to no commit present in this volume's object store THEN THE mcp-fs server SHALL reject `git.branch_reset` with `ERR_NOT_FOUND` and SHALL NOT move the ref.

- **Priority:** Must-have

#### FR-NEW-115 [EARS-E]: Every branch-mutating tool takes the write lock
> WHEN `git.branch_create`, `git.branch_switch`, `git.branch_delete` or `git.branch_reset` performs its mutation THE mcp-fs server SHALL hold the per-project git write lock for the whole mutation.

- **Business Rules:** The lock is `crates/mcp-fs/src/git/repo.rs:44`, already taken by commit, clone, push and pull.
- **Priority:** Must-have

---
#### B. Stash

#### FR-NEW-120 [EARS-E]: Save a stash
> WHEN `git.stash_save` is called THE mcp-fs server SHALL capture the volume's current state as a commit object stored under `refs/stash/{stash_id}`, and SHALL then rewrite the volume to match HEAD's tree.

- **Inputs:** `mount_id`, `message` (string, optional).
- **Outputs:** exactly `{stash_id, sha, message, base_sha, branch, created_at, files_stashed}`. `branch` is the branch checked out at save time; `created_at` is epoch milliseconds.
- **Business Rules:** `stash_id` is server-generated, opaque and stable. The stash commit records `base_sha`, the HEAD it was taken against. `refs/stash` is an unused namespace today (verified: zero occurrences under `crates/`), and stash refs are stored in the existing `git_refs` table, so no new table is required for stash.
- **Priority:** Must-have

#### FR-NEW-121 [EARS-O]: Refuse to stash a clean volume
> IF the volume holds no changes relative to HEAD THEN THE mcp-fs server SHALL reject `git.stash_save` with `ERR_INVALID_ARGUMENT` stating there is nothing to stash, and SHALL NOT create a ref.

- **Priority:** Must-have

#### FR-NEW-122 [EARS-E]: List stashes
> WHEN `git.stash_list` is called THE mcp-fs server SHALL return every stash entry for the volume as an object with exactly the keys `stash_id`, `message`, `base_sha`, `branch` and `created_at`, most recent first.

- **Business Rules:** The pool is per volume, not per branch. An empty pool returns an empty list, not an error.
- **Priority:** Must-have

#### FR-NEW-123 [EARS-UB]: Stash refs are not branches
> The mcp-fs server SHALL NOT list `refs/stash/*` entries in `git.branches`, and SHALL NOT allow them to be checked out by `git.branch_switch`.

- **Priority:** Must-have

#### FR-NEW-124 [EARS-E]: Apply a stash
> WHEN `git.stash_apply` is called with `stash_id` THE mcp-fs server SHALL apply that entry's changes onto the current volume state and SHALL retain the entry.

- **Outputs:** exactly `{stash_id, status, files_changed, dropped}` per FR-NEW-132.
- **Priority:** Must-have

#### FR-NEW-125 [EARS-E]: Pop a stash
> WHEN `git.stash_pop` is called with `stash_id` THE mcp-fs server SHALL apply that entry's changes onto the current volume state and, only once the application has fully succeeded, SHALL delete the entry.

- **Business Rules:** The order matters: an application that ends in conflict leaves the entry intact so no work is lost.
- **Priority:** Must-have

#### FR-NEW-126 [EARS-O]: A conflicting stash application enters the shared conflict model
> IF applying a stash produces a genuine content conflict THEN THE mcp-fs server SHALL return the conflict response defined in FR-NEW-170, SHALL apply nothing to the volume, SHALL record an in-progress operation of type `stash_apply`, and SHALL retain the stash entry regardless of whether `git.stash_apply` or `git.stash_pop` was called.

- **Priority:** Must-have

#### FR-NEW-127 [EARS-E]: Drop a stash
> WHEN `git.stash_drop` is called with `stash_id` THE mcp-fs server SHALL delete that entry without touching the volume.

- **Priority:** Must-have

#### FR-NEW-128 [EARS-O]: Unknown stash id
> IF `stash_id` matches no entry THEN THE mcp-fs server SHALL reject `git.stash_apply`, `git.stash_pop` and `git.stash_drop` with `ERR_NOT_FOUND`.

- **Priority:** Must-have

#### FR-NEW-129 [EARS-U]: Stash entries outlive their source branch
> The mcp-fs server SHALL keep a stash entry listable and applicable after the branch it was created on is deleted, and SHALL apply it against whichever branch is checked out at apply time.

- **Business Rules:** A stash references `base_sha` and a diff, never a branch name.
- **Priority:** Should-have

#### FR-NEW-130 [EARS-U]: Bound the stash pool
> The mcp-fs server SHALL reject `git.stash_save` with `ERR_INVALID_ARGUMENT` once the volume holds `git.max_stash_entries` entries, naming `git.stash_drop` as the remedy.

- **Inputs:** config key `git.max_stash_entries`, default `100`.
- **Priority:** Should-have

---
#### C. Remotes

#### FR-NEW-140 [EARS-E]: Add a remote
> WHEN `git.remote_add` is called with `name` and `url` THE mcp-fs server SHALL record that remote for the volume.

- **Inputs:** `mount_id`, `name`, `url`.
- **Outputs:** `{name, url, host, provider}`.
- **Business Rules:** The URL passes the existing validation: HTTPS only, no embedded credentials, host declared in `git.hosts`. Rejection happens before the row is written.
- **Priority:** Must-have

#### FR-NEW-141 [EARS-O]: Reject a duplicate remote name
> IF `name` already exists as a remote THEN THE mcp-fs server SHALL reject `git.remote_add` with `ERR_INVALID_ARGUMENT` naming the existing remote and its URL, and SHALL NOT overwrite it.

- **Business Rules:** `RelationalGitDb::add_remote` is an upsert (`crates/mcp-fs/src/git/db.rs:281-285`), so the existence check MUST live in the tool layer above it. Calling the storage method blindly would silently rewrite the URL.
- **Priority:** Must-have

#### FR-NEW-142 [EARS-E]: Remove a remote
> WHEN `git.remote_remove` is called with `name` THE mcp-fs server SHALL delete that remote and every remote-tracking ref under `refs/remotes/{name}/`.

- **Priority:** Must-have

#### FR-NEW-143 [EARS-O]: Removing an unknown remote is not found
> IF `name` matches no remote THEN THE mcp-fs server SHALL reject `git.remote_remove` with `ERR_NOT_FOUND`.

- **Business Rules:** `RelationalGitDb::remove_remote` is an unconditional DELETE (`crates/mcp-fs/src/git/db.rs:287-296`) and therefore succeeds silently on a missing name; the existence check MUST live in the tool layer.
- **Priority:** Must-have

#### FR-NEW-144 [EARS-E]: List remotes
> WHEN `git.remote_list` is called THE mcp-fs server SHALL return every remote for the volume with its `name`, `url`, resolved `host` and resolved `provider`.

- **Business Rules:** A volume with no remote returns an empty list, not an error.
- **Priority:** Must-have

#### FR-NEW-145 [EARS-UB]: Remote tools never expose a token
> The mcp-fs server SHALL NOT include any token value in the output of `git.remote_list`, `git.remote_add` or any error they raise.

- **Priority:** Must-have

#### FR-NEW-146 [EARS-O]: An unknown remote name fails before the network
> IF the `remote` parameter of `git.remote_push`, `git.remote_fetch` or `git.remote_pull` names a remote that does not exist THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` before opening any connection.

- **Priority:** Must-have

#### FR-NEW-147 [EARS-E]: A named fetch touches only that remote's tracking refs
> WHEN `git.remote_fetch` runs against remote `{name}` THE mcp-fs server SHALL update refs under `refs/remotes/{name}/` only, and SHALL leave local branches, tags and volume files unchanged.

- **Priority:** Must-have

---
#### D. Force push with lease

#### FR-NEW-155 [EARS-E]: Force push updates a diverged remote ref
> WHEN `git.remote_push` is called with `force` `true` and an `expected_remote_sha` equal to the remote branch's current tip THE mcp-fs server SHALL update the remote ref to the local tip even though the update is not a fast-forward.

- **Inputs:** `mount_id`, `branch`, `force` (boolean, default `false`), `expected_remote_sha` (string), optional `remote`, optional `remote_branch` (the `local:remote` refspec form).
- **Outputs:** `{branch, remote, remote_branch, forced: true, overwritten_sha, new_sha}`.
- **Priority:** Must-have

#### FR-NEW-156 [EARS-O]: The lease is mandatory when forcing
> IF `force` is `true` and `expected_remote_sha` is absent or empty THEN THE mcp-fs server SHALL reject `git.remote_push` with `ERR_INVALID_ARGUMENT` stating that a force push requires the expected remote sha, and SHALL NOT contact the remote.

- **Business Rules:** This is the whole safety design. A bare force flag is not accepted (DEC-903).
- **Priority:** Must-have

#### FR-NEW-157 [EARS-O]: A stale lease rejects the push
> IF `expected_remote_sha` differs from the remote branch's actual current tip THEN THE mcp-fs server SHALL reject `git.remote_push` with an error naming both the expected sha and the actual sha, and SHALL leave the remote ref unchanged.

- **Priority:** Must-have

#### FR-NEW-158 [EARS-O]: A lease without force is contradictory
> IF `expected_remote_sha` is supplied and `force` is `false` THEN THE mcp-fs server SHALL reject `git.remote_push` with `ERR_INVALID_ARGUMENT` rather than silently ignoring the parameter.

- **Business Rules:** Silently ignoring a lease would let a caller believe it had protection it did not have.
- **Priority:** Must-have

#### FR-NEW-159 [EARS-E]: A forced push is audited with the sha it destroyed
> WHEN a forced push succeeds THE mcp-fs server SHALL write an audit entry recording the operation, the remote, the branch and the overwritten sha.

- **Business Rules:** The overwritten sha is the only server-side record of what was on the remote, making recovery from the provider's reflog possible.
- **Priority:** Must-have

#### FR-NEW-160 [EARS-UB]: Force never applies implicitly
> The mcp-fs server SHALL NOT perform a non-fast-forward push when `force` is absent or `false`, and SHALL retain the existing refusal behaviour and error for that case.

- **Priority:** Must-have

---
#### E. The shared conflict model

#### FR-NEW-170 [EARS-E]: One conflict response shape for every combine operation
> WHEN `git.remote_pull`, `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_apply` or `git.stash_pop` encounters a genuine content conflict THE mcp-fs server SHALL return a response carrying `status: "conflict"`, the `operation` type, the `operation_id`, and a `conflicts` array in which each element carries `path`, `ours`, `theirs` and `base`.

- **Outputs:** `ours`, `theirs` and `base` each carry the full content of that side, or an explicit null when that side deleted the path. Binary content is reported with a `binary: true` marker and omitted bytes rather than corrupted text.
- **Business Rules:** One shape, identical field names, regardless of which operation produced it. That is what lets a caller write one handler.
- **Priority:** Must-have

#### FR-NEW-171 [EARS-UB]: A conflict applies nothing
> The mcp-fs server SHALL NOT write any file to the volume, create any commit, or move any ref when a combine operation ends in a conflict response.

- **Business Rules:** The volume must be byte-identical to its pre-operation state. This is stronger than "mostly unchanged" and is tested as such.
- **Priority:** Must-have

#### FR-NEW-172 [EARS-UB]: Conflict markers never enter the volume
> The mcp-fs server SHALL NOT write the sequences `<<<<<<<`, `=======` or `>>>>>>>` into any volume file as a result of any operation defined in this specification.

- **Business Rules:** Retains the existing prohibition. The volume is a filesystem other tools read; a marker there would be silently interpreted as file content.
- **Priority:** Must-have

#### FR-NEW-173 [EARS-E]: Auto-mergeable divergence needs no caller decision
> WHEN a combine operation finds that the two sides changed disjoint regions THE mcp-fs server SHALL complete the merge automatically, without returning a conflict response and without requiring any resolution call.

- **Priority:** Must-have

#### FR-NEW-174 [EARS-E]: Resolve by per-file strategy
> WHEN a resolution call supplies, for a conflicting path, the string `ours` or `theirs` THE mcp-fs server SHALL take that path's content entirely from the named side.

- **Business Rules:** `ours` is the side the operation is applied onto; `theirs` is the side being applied. For a rebase this means `ours` is the new base and `theirs` is the commit being replayed, matching git's own convention.
- **Priority:** Must-have

#### FR-NEW-175 [EARS-E]: Resolve by supplied content
> WHEN a resolution call supplies, for a conflicting path, an object carrying `content` THE mcp-fs server SHALL use exactly those bytes as that path's content.

- **Business Rules:** This is what lets an LLM caller merge the two sides itself rather than choosing a whole side. Bytes are used verbatim with no re-merge attempted.
- **Priority:** Must-have

#### FR-NEW-176 [EARS-E]: Strategy and content mix freely
> WHEN one resolution call supplies strategies for some paths and content for others THE mcp-fs server SHALL apply each path's resolution independently.

- **Priority:** Must-have

#### FR-NEW-177 [EARS-O]: Resolving a non-conflicting path is rejected
> IF a resolution call names a path that is not in the active operation's conflict set THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` naming that path, and SHALL NOT apply any part of the call.

- **Business Rules:** The call is all-or-nothing: a single bad path rejects the whole resolution rather than half-applying it.
- **Priority:** Must-have

#### FR-NEW-178 [EARS-O]: Partial resolution keeps the operation in progress
> IF a resolution call resolves some but not all paths in the conflict set THEN THE mcp-fs server SHALL retain the in-progress operation, SHALL record the resolutions supplied so far, and SHALL return the remaining unresolved paths.

- **Priority:** Must-have

#### FR-NEW-179 [EARS-O]: An invalid strategy value is rejected
> IF a resolution value is a string other than `ours` or `theirs` THEN THE mcp-fs server SHALL reject the call with `ERR_INVALID_ARGUMENT` naming the accepted values.

- **Priority:** Must-have

#### FR-NEW-180 [EARS-E]: A delete/modify conflict is surfaced, not refused
> WHEN one side deletes a path and the other modifies it THE mcp-fs server SHALL surface it as a conflict entry whose deleted side is explicitly null, and SHALL accept `ours`, `theirs` or supplied content as its resolution.

- **Business Rules:** Choosing the deleting side removes the file; choosing the other side keeps it. Deliberately surfaced rather than refused, consistent with DEC-902 (DEC-909 records the alternative that was rejected).
- **Priority:** Must-have

#### FR-NEW-181 [EARS-E]: A both-deleted path resolves to deletion
> WHEN both sides delete the same path THE mcp-fs server SHALL treat it as auto-resolved by deletion and SHALL NOT include it in the conflict set.

- **Priority:** Should-have

#### FR-NEW-182 [EARS-E]: A binary conflict is surfaced with a binary marker
> WHEN a conflicting path holds content that is not valid UTF-8 THE mcp-fs server SHALL mark that conflict entry `binary: true`, SHALL omit the inline content, and SHALL accept only `ours` or `theirs` as its resolution.

- **Business Rules:** Supplying literal `content` for a binary path through a JSON string field cannot round-trip arbitrary bytes, so it is rejected rather than corrupting data.
- **Priority:** Must-have

#### FR-NEW-183 [EARS-O]: A type change is surfaced as a conflict
> IF a path is a file on one side and a directory on the other THEN THE mcp-fs server SHALL surface it as a conflict entry with a `type_change: true` marker and SHALL accept only `ours` or `theirs` as its resolution.

- **Priority:** Should-have

#### FR-NEW-184 [EARS-E]: Applying a completed resolution is atomic
> WHEN the final conflicting path of an operation is resolved THE mcp-fs server SHALL write every resulting file to the volume as a single all-or-nothing unit, SHALL charge the write quota before writing, and SHALL advance the ref only after every file write has succeeded.

- **Business Rules:** Follows the existing `charge_pull_quota` then `apply_pull_changes_atomically` sequence (`crates/mcp-fs/src/tools/git.rs:1967-1989`, `:2079-2127`).
- **Priority:** Must-have

#### FR-NEW-185 [EARS-O]: Quota exhaustion mid-resolution changes nothing
> IF the write quota is insufficient for the resolved result THEN THE mcp-fs server SHALL reject with `ERR_WRITE_QUOTA_EXCEEDED`, SHALL leave the volume unchanged, SHALL NOT move the ref, and SHALL retain the in-progress operation so the caller can retry.

- **Priority:** Must-have

---
#### F. Local merge and squash

#### FR-NEW-190 [EARS-E]: Merge a ref into the current branch
> WHEN `git.merge` is called with `source_ref` THE mcp-fs server SHALL merge that ref into the checked-out branch and, on success, SHALL create a commit with exactly two parents and update the volume to the merged tree.

- **Inputs:** `mount_id`, `source_ref`, `squash` (boolean, optional, default `false`), `message` (string, optional).
- **Outputs:** exactly `{status, merge_commit, fast_forward, squashed, files_changed}` per FR-NEW-199.
- **Business Rules:** Default commit message is `Merge {source_ref} into {current_branch}` when `message` is absent.
- **Priority:** Must-have

#### FR-NEW-191 [EARS-E]: Squash merge produces one single-parent commit
> WHEN `git.merge` is called with `squash` `true` THE mcp-fs server SHALL create exactly one commit with exactly one parent holding the cumulative difference of the source ref, and the source ref's individual commits SHALL NOT appear in the target branch's history.

- **Priority:** Must-have

#### FR-NEW-192 [EARS-O]: An already-merged source is a reported no-op
> IF `source_ref` is already an ancestor of the current branch THEN THE mcp-fs server SHALL return success with `status: "already_up_to_date"` and `merge_commit: null`, SHALL create no commit, and SHALL NOT modify the volume.

- **Priority:** Must-have

#### FR-NEW-193 [EARS-O]: A fast-forwardable merge fast-forwards
> IF the current branch is an ancestor of `source_ref` and `squash` is `false` THEN THE mcp-fs server SHALL advance the branch to `source_ref` without creating a merge commit, and SHALL report `status: "merged"` with `fast_forward: true`.

- **Priority:** Should-have

#### FR-NEW-194 [EARS-O]: Merge refuses a dirty volume
> IF the volume holds uncommitted changes THEN THE mcp-fs server SHALL reject `git.merge` with `ERR_INVALID_ARGUMENT` naming commit and stash as remedies, before performing any merge work.

- **Priority:** Must-have

#### FR-NEW-195 [EARS-O]: An unknown source ref is not found
> IF `source_ref` resolves to no commit THEN THE mcp-fs server SHALL reject `git.merge` with `ERR_NOT_FOUND`.

- **Priority:** Must-have

#### FR-NEW-196 [EARS-E]: Complete a conflicted merge
> WHEN `git.merge_resolve` is called with `resolutions` while a merge-family operation is in progress THE mcp-fs server SHALL apply those resolutions per FR-NEW-174 through FR-NEW-179 and, once the conflict set is empty, SHALL create the resulting commit and clear the in-progress record.

- **Inputs:** `mount_id`, `resolutions` (object mapping path to `"ours"`, `"theirs"`, or `{content}`).
- **Business Rules:** This one tool completes a conflicted `git.merge` **and** a conflicted `git.remote_pull`, because a pull's conflict is a merge conflict. It does not complete a rebase or a cherry-pick, which have their own continue tools (FR-NEW-221, FR-NEW-239).
- **Priority:** Must-have

#### FR-NEW-197 [EARS-E]: Abort a conflicted merge
> WHEN `git.merge_abort` is called while a merge-family operation is in progress THE mcp-fs server SHALL restore HEAD and every volume file to their exact pre-merge values and SHALL delete the in-progress record.

- **Priority:** Must-have

#### FR-NEW-198 [EARS-O]: Resolve or abort with no operation in progress
> IF `git.merge_resolve` or `git.merge_abort` is called when no merge-family operation is in progress THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` stating that no merge is in progress.

- **Priority:** Must-have

---
#### G. Interactive rebase

#### FR-NEW-210 [EARS-E]: Rebase replays a todo list onto a new base
> WHEN `git.rebase` is called with `onto` and `todo` THE mcp-fs server SHALL replay the todo entries in order onto the commit `onto` resolves to, and SHALL move the current branch to the final replayed commit.

- **Inputs:** `mount_id`, `onto` (ref or sha), `todo` (ordered array of `{action, sha, message?}` where action is `pick`, `squash`, `drop` or `reword`).
- **Outputs:** `{status, branch, new_tip, replayed, dropped, squashed, operation_id?}`.
- **Business Rules:** Replayed commits get new shas. Original commits become unreachable but are not destroyed.
- **Priority:** Must-have

#### FR-NEW-211 [EARS-E]: `pick` replays a commit unchanged
> WHEN a todo entry's action is `pick` THE mcp-fs server SHALL replay that commit's change onto the current replay tip, preserving its original message and author, and SHALL set the committer to the caller.

- **Priority:** Must-have

#### FR-NEW-212 [EARS-E]: `squash` folds a commit into its predecessor
> WHEN a todo entry's action is `squash` THE mcp-fs server SHALL combine that commit's change into the preceding replayed commit, producing one commit whose message concatenates both messages.

- **Priority:** Must-have

#### FR-NEW-213 [EARS-E]: `drop` omits a commit
> WHEN a todo entry's action is `drop` THE mcp-fs server SHALL omit that commit's change from the replayed history entirely.

- **Priority:** Must-have

#### FR-NEW-214 [EARS-E]: `reword` replaces a commit message
> WHEN a todo entry's action is `reword` THE mcp-fs server SHALL replay that commit with the supplied `message` and SHALL leave its tree identical to a `pick` of the same commit.

- **Priority:** Must-have

#### FR-NEW-215 [EARS-O]: A rebase todo is validated before any work
> IF the todo references an unknown sha, references a sha outside the range between `onto` and the branch tip, omits a commit that is inside that range, begins with a `squash` entry, contains a duplicate sha, contains an unknown action, or carries a `reword` with an empty or whitespace-only message THEN THE mcp-fs server SHALL reject `git.rebase` with `ERR_INVALID_ARGUMENT` naming the offending entry, and SHALL NOT create any commit, move any ref, or record an in-progress operation.

- **Business Rules:** Pre-flight validation in full before the first replay. A rebase that fails half-way through validation is worse than one that never starts.
- **Priority:** Must-have

#### FR-NEW-216 [EARS-U]: The todo length is bounded
> The mcp-fs server SHALL reject a `git.rebase` whose todo holds more than `git.max_rebase_todo` entries with `ERR_INVALID_ARGUMENT` naming the limit.

- **Inputs:** config key `git.max_rebase_todo`, default `200`.
- **Priority:** Should-have

#### FR-NEW-217 [EARS-O]: Rebase onto an ancestor with nothing to replay is a no-op
> IF `onto` is already the branch's parent such that no commit needs replaying THEN THE mcp-fs server SHALL return `status: "up_to_date"` without creating commits.

- **Priority:** Should-have

#### FR-NEW-218 [EARS-O]: Rebase refuses a dirty volume
> IF the volume holds uncommitted changes THEN THE mcp-fs server SHALL reject `git.rebase` before any replay.

- **Priority:** Must-have

#### FR-NEW-219 [EARS-E]: A conflicting step pauses the rebase
> WHEN replaying a todo entry produces a conflict THE mcp-fs server SHALL stop at that entry, SHALL record an in-progress operation holding the operation type `rebase`, the index of the paused entry, the remaining todo and the conflict set, and SHALL return the conflict response of FR-NEW-170.

- **Business Rules:** Steps already replayed stay replayed; the pause is at a commit boundary, not mid-file.
- **Priority:** Must-have

#### FR-NEW-220 [EARS-E]: A rebase can pause more than once
> WHEN a resumed rebase encounters a further conflicting entry THE mcp-fs server SHALL pause again at that entry and update the in-progress record's step index accordingly.

- **Priority:** Must-have

#### FR-NEW-221 [EARS-E]: Continue a paused rebase
> WHEN `git.rebase_continue` is called with `resolutions` while a rebase is paused THE mcp-fs server SHALL resolve the paused entry per FR-NEW-174 through FR-NEW-179, SHALL commit that entry, and SHALL proceed through the remaining todo entries until the list is exhausted or a further conflict pauses it.

- **Priority:** Must-have

#### FR-NEW-222 [EARS-O]: Continuing with unresolved conflicts is rejected
> IF `git.rebase_continue` is called while paths in the conflict set remain unresolved THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` listing the unresolved paths and SHALL keep the rebase paused at the same step.

- **Priority:** Must-have

#### FR-NEW-223 [EARS-E]: Abort a rebase exactly
> WHEN `git.rebase_abort` is called THE mcp-fs server SHALL restore the branch ref to its exact pre-rebase sha, SHALL restore every volume file to its exact pre-rebase bytes, and SHALL delete the in-progress record.

- **Business Rules:** The pre-rebase tip is stored in the operation record precisely so abort is exact rather than approximate.
- **Priority:** Must-have

#### FR-NEW-224 [EARS-O]: Continue or abort with no rebase in progress
> IF `git.rebase_continue` or `git.rebase_abort` is called when no rebase is in progress THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` stating that no rebase is in progress.

- **Priority:** Must-have

#### FR-NEW-225 [EARS-UB]: Continue channels are not shared between operations
> The mcp-fs server SHALL NOT allow a continuation tool to advance an operation type other than the ones assigned to it in FR-NEW-241.

- **Business Rules:** Each rejects with `ERR_INVALID_ARGUMENT` naming the operation that is actually in progress and the tool pair that finishes it. Concretely: `git.rebase_continue` does not advance a cherry-pick or a revert, `git.cherry_pick_continue` does not advance a rebase or a revert, `git.revert_continue` does not advance a cherry-pick or a rebase, and `git.merge_resolve` advances none of the three.
- **Priority:** Must-have

#### FR-NEW-226 [EARS-E]: A rebase holds the write lock for its whole duration
> WHEN a rebase is replaying entries THE mcp-fs server SHALL hold the per-project git write lock across the entire replay, releasing it when the rebase completes, pauses or aborts.

- **Business Rules:** Releasing between entries would let a concurrent commit interleave into a partially replayed history.
- **Priority:** Must-have

---
#### H. Cherry-pick

#### FR-NEW-235 [EARS-E]: Cherry-pick applies one commit
> WHEN `git.cherry_pick` is called with `commit_sha` THE mcp-fs server SHALL apply that commit's change onto the checked-out branch as a new commit with a new sha.

- **Inputs:** `mount_id`, `commit_sha`.
- **Outputs:** `{status, new_sha, source_sha}`.
- **Business Rules:** The original author is preserved; the committer is the caller.
- **Priority:** Must-have

#### FR-NEW-236 [EARS-O]: An already-present commit is reported
> IF the named commit is already an ancestor of the current branch THEN THE mcp-fs server SHALL return `status: "already_present"` and SHALL NOT create a duplicate commit.

- **Priority:** Must-have

#### FR-NEW-237 [EARS-O]: An unknown cherry-pick source is not found
> IF `commit_sha` resolves to no commit THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND`.

- **Priority:** Must-have

#### FR-NEW-238 [EARS-E]: A conflicting cherry-pick pauses
> WHEN applying the commit produces a conflict THE mcp-fs server SHALL record an in-progress operation of type `cherry_pick` and SHALL return the conflict response of FR-NEW-170.

- **Priority:** Must-have

#### FR-NEW-239 [EARS-E]: Continue and abort a cherry-pick
> WHEN `git.cherry_pick_continue` is called with `resolutions` THE mcp-fs server SHALL complete the paused cherry-pick, and WHEN `git.cherry_pick_abort` is called THE mcp-fs server SHALL restore HEAD and the volume to their exact pre-operation state and clear the record.

- **Priority:** Must-have

#### FR-NEW-241 [EARS-U]: Every pausable operation has exactly one pair of completion tools
> The mcp-fs server SHALL route the completion and abandonment of each operation type to exactly one pair of tools, as follows: `merge`, `remote_pull` and `stash_apply` to `git.merge_resolve` and `git.merge_abort`; `rebase` to `git.rebase_continue` and `git.rebase_abort`; `cherry_pick` to `git.cherry_pick_continue` and `git.cherry_pick_abort`; `revert` to `git.revert_continue` and `git.revert_abort`.

- **Business Rules:** A pull is completed by the merge tools because a pull conflict is a merge conflict, and a stash apply likewise, because applying a stash is a merge against HEAD. The conflict response's `continue_with` and `abort_with` fields always name the correct pair, so the caller never infers it (FR-NEW-170). Without this requirement an implementer must guess which tool finishes a conflicted revert.
- **Priority:** Must-have

#### FR-NEW-240 [EARS-O]: Cherry-picking a merge commit requires a mainline
> IF `commit_sha` names a commit with more than one parent and `mainline` is absent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` explaining that a mainline parent must be chosen.

- **Inputs:** `mainline` (integer, optional, 1-based parent index).
- **Priority:** Should-have

---
#### I. Reset

#### FR-NEW-250 [EARS-E]: Soft reset moves the pointer only
> WHEN `git.reset` is called with `mode` `soft` THE mcp-fs server SHALL move the current branch ref to `target_ref` and SHALL NOT modify any volume file.

- **Inputs:** `mount_id`, `target_ref`, `mode` (`soft` or `hard`, required).
- **Outputs:** exactly `{mode, old_sha, new_sha, files_changed}`.
- **Priority:** Must-have

#### FR-NEW-251 [EARS-E]: Hard reset moves the pointer and the volume
> WHEN `git.reset` is called with `mode` `hard` THE mcp-fs server SHALL move the current branch ref to `target_ref` and SHALL rewrite the volume so its contents equal that commit's tree exactly, discarding uncommitted changes.

- **Priority:** Must-have

#### FR-NEW-252 [EARS-O]: Mode is required and closed
> IF `mode` is absent, or is any value other than `soft` or `hard` THEN THE mcp-fs server SHALL reject `git.reset` with `ERR_INVALID_ARGUMENT` naming the two accepted values.

- **Business Rules:** There is deliberately no `mixed` mode, because with no index it would be indistinguishable from `soft` (DEC-905).
- **Priority:** Must-have

#### FR-NEW-253 [EARS-O]: An unknown reset target changes nothing
> IF `target_ref` resolves to no commit THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` and SHALL leave the branch ref and volume untouched.

- **Priority:** Must-have

#### FR-NEW-254 [EARS-O]: Resetting to the current tip is a no-op
> IF `target_ref` resolves to the branch's current tip THEN THE mcp-fs server SHALL return success having changed nothing.

- **Priority:** Should-have

#### FR-NEW-255 [EARS-U]: Reset orphans rather than destroys
> The mcp-fs server SHALL leave commits that become unreachable through a reset present in the object store, and SHALL NOT prune them as part of the reset.

- **Business Rules:** This is what makes a mistaken reset recoverable by `git.branch_reset` back to the reported `old_sha`.
- **Priority:** Must-have

---
#### J. Revert

#### FR-NEW-260 [EARS-E]: Revert creates an inverse commit
> WHEN `git.revert` is called with `commit_sha` THE mcp-fs server SHALL create a new commit on the current branch whose change is the inverse of the named commit, and SHALL leave the named commit in history.

- **Inputs:** `mount_id`, `commit_sha`, `mainline` (integer, optional), `message` (string, optional).
- **Outputs:** `{status, new_sha, reverted_sha}`.
- **Business Rules:** Default message is `Revert "{original subject}"`.
- **Priority:** Must-have

#### FR-NEW-261 [EARS-O]: Reverting a merge commit requires a mainline
> IF `commit_sha` names a commit with more than one parent and `mainline` is absent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` explaining that reverting a merge requires choosing which parent is the mainline.

- **Priority:** Must-have

#### FR-NEW-262 [EARS-O]: Mainline must be in range and only for merges
> IF `mainline` is supplied and is less than 1, or exceeds the commit's parent count, or the commit has a single parent THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` naming the commit's actual parent count.

- **Priority:** Must-have

#### FR-NEW-263 [EARS-E]: Revert of the initial commit is handled
> WHEN `commit_sha` names a commit with no parent THE mcp-fs server SHALL compute the inverse against the empty tree, producing a commit that removes everything the initial commit added.

- **Priority:** Should-have

#### FR-NEW-264 [EARS-O]: A conflicting revert enters the conflict model
> IF the inverse change does not apply cleanly to the current tree THEN THE mcp-fs server SHALL record an in-progress operation of type `revert` and SHALL return the conflict response of FR-NEW-170.

- **Priority:** Must-have

#### FR-NEW-266 [EARS-E]: Continue and abort a revert
> WHEN `git.revert_continue` is called with `resolutions` THE mcp-fs server SHALL complete the paused revert and create the inverse commit, and WHEN `git.revert_abort` is called THE mcp-fs server SHALL restore HEAD and the volume to their exact pre-operation state and clear the record.

- **Business Rules:** These exist as their own pair rather than reusing the cherry-pick tools, per FR-NEW-241 and DEC-904, because `git revert --continue` and `git revert --abort` are the vocabulary the caller already has.
- **Priority:** Must-have

#### FR-NEW-265 [EARS-U]: Reverting a revert restores the original change
> The mcp-fs server SHALL produce, when a revert commit is itself reverted, a tree identical to the one before the first revert.

- **Priority:** Should-have

---
#### K. Operation state and the in-progress guard

#### FR-NEW-275 [EARS-U]: The operation record is a relational table
> The mcp-fs server SHALL persist every in-progress operation as a row in a new relational table `git_operations`, keyed by `volume_id`, holding at minimum: `op_type`, `state`, `onto_sha`, `original_tip_sha`, `todo`, `current_step`, `total_steps`, `conflicts`, `resolutions`, `created_at` and `updated_at`.

- **Business Rules:** `volume_id` scopes the row because one database holds every volume. One volume holds at most one in-progress operation. The table lives in the git index alongside `git_objects`, `git_refs` and `git_remotes` (`crates/mcp-fs/src/git/db.rs:42`).
- **Priority:** Must-have

#### FR-NEW-276 [EARS-E]: The new table is registered for purge
> WHEN `git_operations` is added THE mcp-fs server SHALL include it in the `TABLES` constant of the git index so that deleting a project removes its operation rows.

- **Business Rules:** `pub const TABLES: [&str; 3]` at `crates/mcp-fs/src/git/db.rs:42` is commented "named once so a purge cannot miss one" and becomes `[&str; 4]`. Omitting this leaves orphaned rows behind after project deletion. This is its own requirement rather than a footnote precisely because it is easy to miss.
- **Priority:** Must-have

#### FR-NEW-277 [EARS-U]: The new table works on every relational backend
> The mcp-fs server SHALL define `git_operations` through the existing dialect abstraction so it is created and queried identically on SQLite, PostgreSQL and SQL Server.

- **Business Rules:** Keyed text columns respect the SQL Server indexable-length ceiling by using `TextKey(n)`; free payload columns (`todo`, `conflicts`, `resolutions`) use unbounded `Text`. See `.agent_docs/backends.md`.
- **Priority:** Must-have

#### FR-NEW-278 [EARS-U]: A paused operation survives a restart
> The mcp-fs server SHALL make a paused operation resumable after a server restart, with its `current_step`, `todo` and `conflicts` unchanged.

- **Business Rules:** This is the reason the record is relational rather than in-memory (DEC-901).
- **Priority:** Must-have

#### FR-NEW-279 [EARS-S]: An in-progress operation blocks other mutations
> WHILE an operation is in progress for a volume THE mcp-fs server SHALL reject `git.commit`, `git.branch_switch`, `git.branch_create`, `git.branch_delete`, `git.branch_reset`, `git.reset`, `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_save`, `git.stash_apply`, `git.stash_pop`, `git.remote_pull` and `git.remote_clone` with `ERR_INVALID_ARGUMENT` naming the active operation type and the tools that finish it.

- **Priority:** Must-have

#### FR-NEW-280 [EARS-S]: Read-only tools stay available during an operation
> WHILE an operation is in progress THE mcp-fs server SHALL continue to serve `git.status`, `git.log`, `git.show`, `git.diff`, `git.branches`, `git.tags`, `git.blame`, `git.stash_list`, `git.remote_list` and `git.remote_fetch` normally.

- **Business Rules:** `git.remote_fetch` is included because it touches only remote-tracking refs and never the volume or local branches.
- **Priority:** Must-have

#### FR-NEW-281 [EARS-E]: Status reports the active operation
> WHEN `git.status` is called while an operation is in progress THE mcp-fs server SHALL include an `operation` object carrying `op_type`, `current_step`, `total_steps`, the unresolved conflicting paths and the names of the tools that continue and abort it.

- **Priority:** Must-have

#### FR-NEW-282 [EARS-U]: An operation is not owned by one person
> The mcp-fs server SHALL allow any member of the project to resolve, continue or abort an operation another member started.

- **Business Rules:** Authorization is membership-based with no per-operation ownership; the volume is shared, so a paused operation that only its author could clear would be a denial of service on the whole project.
- **Priority:** Must-have

#### FR-NEW-283 [EARS-U]: One operation per volume
> The mcp-fs server SHALL hold at most one in-progress operation row per `volume_id`, and SHALL scope every operation query by `volume_id`.

- **Priority:** Must-have

#### FR-NEW-284 [EARS-E]: Completion and abort clear the record
> WHEN an operation completes successfully or is aborted THE mcp-fs server SHALL delete its `git_operations` row.

- **Business Rules:** A stale row would permanently block the volume; row removal is verified by test, not assumed.
- **Priority:** Must-have

---
#### L. Pull requests and merge requests

#### FR-NEW-300 [EARS-E]: Provider and API base resolve from the remote host
> WHEN any `git.pr_*` tool is called THE mcp-fs server SHALL resolve the target provider and REST API base URL from the host of the volume's remote URL, using the existing `git.hosts` map and the stored `instance_url`.

- **Business Rules:** Resolution goes through the existing host map (`git.hosts`), which is what makes GitHub Enterprise Server and self-hosted GitLab work with no new configuration. GitHub Enterprise uses the host's `/api/v3` base; gitlab uses `/api/v4` on the instance URL.
- **Priority:** Must-have

#### FR-NEW-301 [EARS-O]: Unsupported providers are rejected
> IF the resolved provider is `generic` or `anonymous` THEN THE mcp-fs server SHALL reject every `git.pr_*` tool with `ERR_NOT_SUPPORTED`, naming the host and stating that pull-request operations require a `github` or `gitlab` provider.

- **Priority:** Must-have

#### FR-NEW-302 [EARS-O]: A volume with no usable remote is rejected
> IF the volume has no remote, or the selected remote's URL cannot be parsed THEN THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` before any network call.

- **Priority:** Must-have

#### FR-NEW-303 [EARS-U]: One normalized response model across providers
> The mcp-fs server SHALL return, for every `git.pr_*` tool, a response using provider-independent field names, and SHALL populate them identically for a GitHub pull request and a GitLab merge request in the same logical state.

- **Outputs:** exactly the key set of FR-NEW-317: `provider`, `host`, `number`, `title`, `body`, `state` (`open`|`closed`|`merged`), `draft` (a separate boolean, NOT folded into `state`), `base`, `head`, `author`, `url`, `created_at`, `updated_at`, `commits`, `changed_files`, `additions`, `deletions`, `review_state`, `checks_state`, `mergeable`, and `raw` carrying the provider's untouched payload.
- **Business Rules:** The frozen tool contract describes one shape; a caller writes one handler. `raw` is the escape hatch for provider-specific fields so normalization never loses information.
- **Priority:** Must-have

#### FR-NEW-304 [EARS-E]: Create a pull request
> WHEN `git.pr_create` is called with `base`, `head` and `title` THE mcp-fs server SHALL create a pull request on GitHub or a merge request on GitLab and SHALL return the normalized model.

- **Inputs:** `mount_id`, `base`, `head`, `title`, `body` (optional), `draft` (boolean, optional, default `false`), `remote` (optional, default `origin`).
- **Priority:** Must-have

#### FR-NEW-305 [EARS-O]: Create fails early when the head branch is not on the remote
> IF `head` does not exist on the target remote THEN THE mcp-fs server SHALL reject `git.pr_create` with `ERR_NOT_FOUND` naming the branch and `git.remote_push` as the remedy, before issuing any create request.

- **Priority:** Must-have

#### FR-NEW-306 [EARS-E]: List pull requests
> WHEN `git.pr_list` is called THE mcp-fs server SHALL return the normalized model for each pull request matching `state`.

- **Inputs:** `mount_id`, `state` (`open`|`closed`|`merged`|`all`, optional, default `open`), `remote` (optional).
- **Business Rules:** An empty result is an empty array, not an error. An unsupported `state` value is rejected before the call.
- **Priority:** Must-have

#### FR-NEW-307 [EARS-E]: Get a pull request with review and check state
> WHEN `git.pr_get` is called with `pr_number` THE mcp-fs server SHALL return the normalized model enriched with its commits, its changed files, its `review_state` and its `checks_state`.

- **Business Rules:** `review_state` is one of `approved`, `changes_requested`, `review_required`, `none`. `checks_state` is one of `success`, `failure`, `pending`, `none`. Both are normalized across the providers' different underlying concepts (GitHub reviews plus check runs; GitLab approvals plus pipelines).
- **Priority:** Must-have

#### FR-NEW-308 [EARS-O]: A failed enrichment sub-call is surfaced, not degraded
> IF the request for check or review state fails THEN THE mcp-fs server SHALL fail `git.pr_get` with the provider's error rather than returning a model with those fields silently set to `none`.

- **Business Rules:** A silently degraded "no failing checks" reading is worse than an error, because a caller would merge on it.
- **Priority:** Must-have

#### FR-NEW-309 [EARS-E]: Get a pull request diff
> WHEN `git.pr_diff` is called with `pr_number` THE mcp-fs server SHALL return the unified diff for that pull request.

- **Business Rules:** The response is bounded by `git.max_pr_diff_mb` (default 12); a larger diff is truncated with an explicit `truncated: true` marker rather than streamed unbounded into the caller's context.
- **Priority:** Must-have

#### FR-NEW-310 [EARS-E]: Merge a pull request
> WHEN `git.pr_merge` is called with `pr_number` and `strategy` THE mcp-fs server SHALL ask the provider to merge it using that strategy.

- **Inputs:** `mount_id`, `pr_number`, `strategy` (`merge`|`squash`|`rebase`), `commit_title` (optional), `commit_message` (optional).
- **Business Rules:** The merge happens on the provider. Local refs and remote-tracking refs are **not** updated; the response states that a `git.remote_fetch` is required to observe it locally.
- **Priority:** Must-have

#### FR-NEW-311 [EARS-O]: Provider merge refusals are surfaced faithfully
> IF the provider refuses the merge because required checks fail, required reviews are missing, the branch is protected, the strategy is disabled for that repository, or the pull request is already merged or closed THEN THE mcp-fs server SHALL surface the provider's status and message, and SHALL NOT report success.

- **Priority:** Must-have

#### FR-NEW-312 [EARS-E]: Post a review
> WHEN `git.pr_review` is called with `pr_number` and `verdict` THE mcp-fs server SHALL submit that review to the provider.

- **Inputs:** `mount_id`, `pr_number`, `verdict` (`approve`|`request_changes`|`comment`), `body` (string, optional).
- **Business Rules:** `request_changes` and `comment` require a non-empty `body`; `approve` does not. GitLab maps `approve` to its approve endpoint and the other two to a merge-request note.
- **Priority:** Must-have

#### FR-NEW-313 [EARS-O]: An unknown pull request number is not found
> IF `pr_number` matches no pull request THEN THE mcp-fs server SHALL reject with `ERR_NOT_FOUND` naming the number and the repository.

- **Priority:** Must-have

#### FR-NEW-314 [EARS-UB]: No token ever leaves through a PR tool
> The mcp-fs server SHALL NOT include any token value in a `git.pr_*` response, in the `raw` payload, in any error message, in any log line, or in any tracing span field.

- **Business Rules:** The `raw` passthrough is scrubbed of request authorization data before being returned.
- **Priority:** Must-have

#### FR-NEW-315 [EARS-E]: The provider client is injectable
> WHEN the server is assembled THE mcp-fs server SHALL obtain its provider REST client through an abstraction that a test can replace with a fake returning canned provider responses.

- **Business Rules:** Mirrors the existing device-flow client seam used by `MockDeviceFlowClient`. Without this the entire PR surface is untestable offline.
- **Priority:** Must-have

#### FR-NEW-316 [EARS-UB]: The provider client does not follow cross-host redirects
> The mcp-fs server SHALL NOT follow an HTTP redirect issued by a provider API to a host other than the resolved API host, and SHALL fail the call instead.

- **Business Rules:** Following a redirect would replay the Authorization header to an attacker-chosen host.
- **Priority:** Must-have

---
#### M. OAuth scope

#### FR-NEW-330 [EARS-E]: The GitLab device flow requests API access
> WHEN the GitLab device flow requests authorization THE mcp-fs server SHALL include the `api` scope in the requested scope set.

- **Business Rules:** Today the constant is `read_repository write_repository` (`crates/mcp-fs/src/git/oauth/device_flow.rs:39`). GitLab documents `write_repository` as Git-over-HTTP only, granting **no** REST API access, so every merge-request tool would be unauthorized without this change. GitHub's existing `repo` scope (`device_flow.rs:38`) already covers the whole pull-request surface and is unchanged.
- **Priority:** Must-have

#### FR-NEW-331 [EARS-U]: Requested scopes are configurable
> The mcp-fs server SHALL read the scope requested for each provider from configuration, defaulting to `repo` for GitHub and `api read_repository write_repository` for GitLab.

- **Inputs:** config keys `git.github_scope`, `git.gitlab_scope`.
- **Business Rules:** The constants are hardcoded today; a deployment whose provider policy differs cannot currently adjust them without a rebuild.
- **Priority:** Should-have

#### FR-NEW-332 [EARS-E]: Required scope is validated before the network call
> WHEN a `git.pr_*` tool is called THE mcp-fs server SHALL compare the scopes recorded for that `(person, host)` against the scope set required for the operation and, when they are insufficient, SHALL reject with `ERR_FORBIDDEN` before issuing any request.

- **Business Rules:** Mirrors the existing rule that an expired token fails before the network. Scopes are already stored (`crates/mcp-fs/src/git/oauth/persistence.rs:50`) and reported (`crates/mcp-fs/src/tools/git_auth.rs:396`) but are validated nowhere in the tree today.
- **Priority:** Must-have

#### FR-NEW-333 [EARS-E]: The scope error is actionable
> WHEN a scope check fails THE mcp-fs server SHALL name the missing scope, the host, and `git.auth` and `git.token_set` as the tools that grant a replacement.

- **Priority:** Must-have

#### FR-NEW-334 [EARS-O]: A token with unrecorded scopes is not assumed sufficient
> IF the stored scope set for a `(person, host)` is empty or unknown, as it is for a token seeded through `git.token_set` without scope information, THEN THE mcp-fs server SHALL attempt the operation and SHALL surface the provider's authorization failure verbatim rather than pre-emptively refusing.

- **Business Rules:** Deliberate asymmetry: a *known-insufficient* scope fails early (FR-NEW-332), an *unknown* scope is given the benefit of the doubt, because refusing it would make `git.token_set` useless for any PAT whose scopes the server cannot enumerate.
- **Priority:** Must-have

#### FR-NEW-335 [EARS-E]: Auth status reports PR capability
> WHEN `git.auth_status` is called THE mcp-fs server SHALL report, per entry, the scopes held and whether they are sufficient for the pull-request surface.

- **Priority:** Should-have

---
#### N. Contract, counts and documentation

#### FR-NEW-345 [EARS-E]: Every new tool enters the frozen contract
> WHEN the tools of this specification are registered THE mcp-fs server SHALL include each of them in the frozen tool contract, regenerated through `MCPFS_REWRITE_TOOL_CONTRACT=1`, so that `TOOL_CONTRACT.txt` and `tool-contract-golden.json` describe every name, description and `inputSchema`.

- **Priority:** Must-have

#### FR-NEW-346 [EARS-E]: Every hardcoded tool count is updated
> WHEN the tool count changes THE mcp-fs server's test suite SHALL be updated at exactly these five sites: `crates/mcp-fs/src/tools/contract_golden.rs:131`, `crates/mcp-fs/src/tools/all.rs:78`, `:122`, `:141`, and `crates/mcp-fs/src/tools/git.rs:2500`.

- **Business Rules:** Five sites, counted directly. Two further count assertions exist and MUST NOT be touched: `crates/mcp-fs/src/tools/all.rs:64` asserts the admin-only registry (10 tools, git absent) and `:99` asserts the git-disabled registry (52 tools). Neither changes when git tools are added, and editing either breaks a passing test. This specification adds exactly **31** tools, enumerated in FR-NEW-349. The frozen contract count moves from 63 to **94** (`contract_golden.rs:131`), the git-family total from 18 to **49** (`all.rs:78`, currently `10 + 14 + 4`), and the `git.*` registry count from 14 to **45** (`git.rs:2500`).
- **Priority:** Must-have

#### FR-NEW-347 [EARS-E]: The new tools are added to the blanket authorization tests
> WHEN the tools are registered THE mcp-fs server's test suite SHALL add each of them to the existing tests asserting that every git tool rejects a non-member and rejects a platform admin who is not a member.

- **Business Rules:** The existing test is `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`crates/mcp-fs/src/tools/git.rs:2685`). A tool absent from that list is an unguarded tool.
- **Priority:** Must-have

#### FR-NEW-349 [EARS-U]: The tool set added by this specification is exactly these 31 names
> The mcp-fs server SHALL register exactly the following 31 new tools and no others: `git.branch_create`, `git.branch_switch`, `git.branch_delete`, `git.branch_reset`, `git.stash_save`, `git.stash_list`, `git.stash_apply`, `git.stash_pop`, `git.stash_drop`, `git.remote_add`, `git.remote_remove`, `git.remote_list`, `git.merge`, `git.merge_resolve`, `git.merge_abort`, `git.rebase`, `git.rebase_continue`, `git.rebase_abort`, `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort`, `git.reset`, `git.revert`, `git.revert_continue`, `git.revert_abort`, `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review`.

- **Business Rules:** All 31 register through `git::register` (the PR tools are implemented in a new `git_pr.rs` but registered from that one entry point), so `crates/mcp-fs/src/tools/git.rs:2500` asserts 45 and its `ALL_GIT_TOOLS` literal becomes `[&str; 45]`. This list is the authoritative count. Every count assertion in FR-NEW-346 derives from it: 63 + 31 = 94 frozen contract tools, 18 + 31 = 49 git-family tools, 14 + 31 = 45 `git.*` tools excluding the four auth tools.
- **Priority:** Must-have

#### FR-NEW-348 [EARS-E]: Documentation is updated with the tool surface
> WHEN the tool surface changes THE mcp-fs project SHALL update `AGENTS.md:5`, `:88`, `:155`, `.agent_docs/tools.md:1` and `:157`, and `.agent_docs/git.md`, to describe the new counts and the new tool families.

- **Priority:** Must-have


---
#### O. Wire-shape requirements (added at implementability audit round 1)

These exist because Section 6, Section 8 and Section 12 were authored separately and
disagreed on field names for the same tools. Where they disagree, **this subsection
wins**, and the superseded text has been corrected in place.

#### FR-NEW-186 [EARS-U]: The conflict response has exactly one serialized shape
> The mcp-fs server SHALL return, for every conflict of every combine operation, an object with exactly the keys `status`, `operation`, `operation_id`, `source_ref`, `current_step`, `total_steps`, `conflicts`, `continue_with`, `abort_with`, where each `conflicts` element has exactly the keys `path`, `ours`, `theirs`, `base`, `binary`, `type_change`, and each of `ours`, `theirs` and `base` is an object with exactly `exists` (boolean) and `content` (string, `null` when `exists` is false or when `binary` is true).

- **Inputs:** none; this governs the response of `git.merge`, `git.remote_pull`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.stash_apply` and `git.stash_pop`.
- **Outputs:** the shape above. `current_step` and `total_steps` are `null` for single-step operations (merge, remote_pull, cherry_pick, revert, stash_apply, stash_pop); a rebase ALWAYS reports integers, never null, whatever its todo length. `source_ref` is `null` where not applicable.
- **Business Rules:** In the conflict RESPONSE, `conflicts` is an array of objects and is never a bare array of path strings. Two other places legitimately hold a plain array of path strings and are not governed by this rule: the `git_operations.conflicts` column (Section 8.1), which persists the unresolved paths, and the `remaining_conflicts` field of the `operation` object that `git.status` returns (FR-NEW-281). The keys `conflicting_paths`, `resolve_with`, `target_ref` and `step` are emitted by no tool. Any test in Section 12 asserting an older form is rewritten against this shape.
- **Priority:** Must-have

#### FR-NEW-187 [EARS-U]: Step indices are zero-based
> The mcp-fs server SHALL report `current_step` as the zero-based index of the paused todo entry and `total_steps` as the count of todo entries, so that `todo[current_step]` is the entry awaiting resolution.

- **Outputs:** `current_step` (integer, 0 or greater), `total_steps` (integer, 1 or greater).
- **Business Rules:** `git.status`'s `operation` object (FR-NEW-281) and the `git_operations` row (FR-NEW-275) use the same base. A resumed rebase that has completed entry index 0 and paused on entry index 1 reports `current_step` as 1.
- **Priority:** Must-have

#### FR-NEW-188 [EARS-U]: The operation state column is a closed set
> The mcp-fs server SHALL store `git_operations.state` as exactly one of `conflicted` or `running`, SHALL write `conflicted` whenever an operation pauses awaiting resolution, and SHALL NOT write any other value.

- **Business Rules:** `paused` is not a valid value. Every test asserts `conflicted` for a paused operation.
- **Priority:** Must-have

#### FR-NEW-189 [EARS-U]: Branch tool response fields are fixed
> The mcp-fs server SHALL return exactly `{branch, sha, checked_out}` from `git.branch_create`, exactly `{branch, sha, changed, files_changed}` from `git.branch_switch`, exactly `{branch, sha, forced}` from `git.branch_delete`, and exactly `{branch, old_sha, new_sha, checked_out, files_changed}` from `git.branch_reset`.

- **Business Rules:** These names are authoritative over any earlier Outputs line. The names `volume_rewritten`, `previous_sha`, `deleted_sha`, `files_written` and `files_removed` are emitted by no tool.
- **Priority:** Must-have

#### FR-NEW-131 [EARS-U]: Stash entry fields are fixed
> The mcp-fs server SHALL return, for each `git.stash_list` entry, exactly the keys `stash_id`, `message`, `base_sha`, `branch` and `created_at`, where `branch` is the branch checked out at save time and `created_at` is epoch milliseconds, ordered newest first.

- **Business Rules:** `base_sha` is mandatory because FR-NEW-129 makes it the only anchor once the branch a stash was taken from is deleted. Dropping it would make a stash unapplicable after branch deletion.
- **Priority:** Must-have

#### FR-NEW-317 [EARS-U]: The normalized pull-request object key set
> The mcp-fs server SHALL return, from `git.pr_create`, `git.pr_get`, `git.pr_merge` and `git.pr_review`, an object with exactly the keys `provider`, `host`, `number`, `title`, `body`, `state`, `draft`, `base`, `head`, `author`, `url`, `created_at`, `updated_at`, `commits`, `changed_files`, `additions`, `deletions`, `review_state`, `checks_state`, `mergeable`, `raw`, in that order.

- **Outputs:** `state` is one of `open`, `closed`, `merged`. `draft` is a separate boolean and is NOT folded into `state`.
- **Business Rules:** `commits` and `changed_files` are counts, `additions` and `deletions` are line counts, `mergeable` is a tri-state boolean (`true`, `false`, `null` when the provider has not computed it). `git.pr_list` returns the same object minus `commits`, `changed_files`, `additions`, `deletions` and `mergeable`. This supersedes the Section 8.4 sketch, which folded `draft` into `state` and omitted `provider` and `host`.
- **Priority:** Must-have

#### FR-NEW-116 [EARS-E]: Branch name length is measured in bytes
> WHEN a branch name is supplied THE mcp-fs server SHALL reject with `ERR_INVALID_ARGUMENT` any name whose UTF-8 encoding exceeds 255 bytes, and any name having a `/`-separated component that begins with `.`.

- **Business Rules:** A 200-character name of three-byte codepoints is 600 bytes and is rejected. The rationale is a storage ceiling, not a byte-typed column: `git_refs.name` is `TextKey(400)` (`crates/mcp-fs/src/git/db.rs:22`), which the dialect layer renders as unbounded `TEXT` on SQLite and PostgreSQL but as `NVARCHAR(400)`, a 400-character bound, on SQL Server (`crates/mcp-fs/src/storage/rel/dialect.rs:218`, `:226`, `:235`). A 255-byte cap keeps `refs/heads/<name>` inside that ceiling on every backend. The Section 12 constant is `MAX_BRANCH_NAME_BYTES`.
- **Priority:** Must-have



#### FR-NEW-132 [EARS-U]: Stash apply and pop return one key set
> The mcp-fs server SHALL return from `git.stash_apply` and `git.stash_pop` an object with exactly the keys `stash_id`, `status`, `files_changed` and `dropped`, where `status` is `applied` or `conflict`, and `dropped` is `false` for `git.stash_apply` and `true` for a `git.stash_pop` that fully succeeded.

- **Business Rules:** The key `applied` is emitted by no tool; the boolean lives in `dropped` and the outcome in `status`. A `git.stash_pop` that ends in conflict reports `status: "conflict"` and `dropped: false`, because FR-NEW-126 retains the entry until resolution succeeds. `git.stash_drop` returns exactly `{stash_id, dropped}` with `dropped` always `true`.
- **Priority:** Must-have

#### FR-NEW-285 [EARS-U]: The operation type is a six-value closed set
> The mcp-fs server SHALL store and report `op_type` as exactly one of `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply` or `stash_pop`, SHALL write `stash_pop` for a paused `git.stash_pop` and `stash_apply` for a paused `git.stash_apply`, and SHALL use that same value in the conflict response's `operation` field and in the `op_type` of the `operation` object returned by `git.status`.

- **Business Rules:** The two stash values are distinguished because `dropped` differs on completion: a resumed `stash_pop` deletes the entry, a resumed `stash_apply` keeps it, so the persisted row must remember which was called. `remote_pull` is recorded as `merge`, since a pull conflict is a merge conflict completed by the merge tools (FR-NEW-241). The Section 8.1 `op_type` row lists these six values.
- **Priority:** Must-have


#### FR-NEW-199 [EARS-U]: Every tool's response key set, in one place
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
| `git.remote_push` | `branch`, `remote`, `remote_branch`, `created`, `up_to_date`, `forced`, `overwritten_sha`, `remote_sha`, `auth` |

- **Business Rules:**
  - Any operation that can conflict returns, INSTEAD of the keys above, the conflict response of FR-NEW-186 when it conflicts. The two shapes are distinguished by `status`.
  - `status` values are closed per tool: `git.merge` is one of `merged`, `conflict`, `already_up_to_date`; `git.rebase` and `git.rebase_continue` are one of `completed`, `conflict`, `up_to_date`; `git.cherry_pick` and `git.revert` and their continuations are one of `committed`, `conflict`, `already_present`; `git.stash_apply` and `git.stash_pop` are one of `applied`, `conflict`; every abort tool returns exactly `aborted`.
  - `remaining_conflicts` is ALWAYS an array of path strings, never a count, in every tool and every object that carries it, including the `operation` object of FR-NEW-281.
  - AMENDED post-implementation: `git.remote_push` reports `remote_sha`, not `new_sha`. The name states whose sha it is and pairs with `overwritten_sha`; `new_sha` remains correct for the four local-history tools (`branch_reset`, `cherry_pick`, `reset`, `revert`), where the sha is newly created rather than observed on a remote. The response also carries `auth`, the credential CLASS used (`anonymous`, `github`, `gitlab`, `generic`, `unknown`), never a credential value. `remote_branch` and `overwritten_sha` are emitted only when applicable; `remote` is unconditional, since it always has a value (`origin` by default).
  - `overwritten_sha` is OMITTED, not null, unless the push was forced: the shipped tools drop an inapplicable key rather than emitting a null, and `e2e_new_828`/`e2e_new_832` assert its absence. The same holds for `remote_branch`, omitted unless the effective remote branch differs from the local one. `merge_commit` IS null (not omitted) when `status` is `already_up_to_date`, and is null on a stash completion, which creates no commit. `restored_sha` is the exact pre-operation tip an abort restored.
  - The keys `commit_sha` and `parents` are emitted by no tool in this specification: a commit's parent count is asserted on the commit object read back through `git.show`, not on the operation's response.
  - This table is authoritative. Where any earlier Outputs line or any Section 12 assertion names a different key for one of these tools, this table wins and the other text is read as superseded.
- **Priority:** Must-have

#### FR-NEW-286 [EARS-U]: The `git.status` operation object key set
> The mcp-fs server SHALL populate the `operation` object of FR-NEW-281 with exactly the keys `op_type`, `source_ref`, `current_step`, `total_steps`, `remaining_conflicts`, `continue_with` and `abort_with`.

- **Business Rules:** `op_type` takes the six values of FR-NEW-285. `current_step` and `total_steps` follow FR-NEW-187, so they are `null` for single-step operations and integers for a rebase. `remaining_conflicts` is an array of path strings.
- **Priority:** Must-have


### Modified Requirements

#### FR-MOD-101 [EARS-E]: `git.remote_push` accepts a target remote (references archived spec FR-NEW-022)
> WHEN `git.remote_push` is called with `remote` THE mcp-fs server SHALL push to that remote instead of `origin`, defaulting to `origin` when the parameter is absent.

- **Original behavior:** Push always resolved `origin`; no remote could be named.
- **Reason for change:** BL-008. A volume created by `git.init` has no `origin` and could therefore not push at all.
- **Priority:** Must-have

#### FR-MOD-102 [EARS-E]: `git.remote_push` accepts a distinct remote branch name (references archived spec FR-NEW-022)
> WHEN `git.remote_push` is called with `remote_branch` THE mcp-fs server SHALL push the local `branch` to that differently-named branch on the remote, defaulting to the local branch's name when absent.

- **Original behavior:** The remote branch name always equalled the local one.
- **Reason for change:** BL-009.
- **Priority:** Must-have

#### FR-MOD-103 [EARS-E]: `git.remote_fetch` and `git.remote_pull` accept a target remote
> WHEN `git.remote_fetch` or `git.remote_pull` is called with `remote` THE mcp-fs server SHALL operate against that remote, defaulting to `origin`.

- **Priority:** Must-have

#### FR-MOD-104 [EARS-E]: A diverged pull merges instead of refusing (references archived spec FR-NEW-029, FR-NEW-031)
> WHEN `git.remote_pull` finds the local branch diverged from the fetched remote tip THE mcp-fs server SHALL perform a three-way merge, completing it automatically when no conflict arises and returning the conflict response of FR-NEW-170 when one does.

- **Original behavior:** A diverged pull was refused outright unless `on_conflict` was supplied, and when supplied a single global `ours`/`theirs` strategy was applied to every conflicting file (`crates/mcp-fs/src/tools/git.rs:1835-1843`).
- **New behavior (EARS):** as stated above.
- **Reason for change:** DEC-902. The caller cannot make a sensible global choice without seeing the conflicts.
- **Business Rules:** Fetched objects and updated remote-tracking refs are retained whether or not the merge completes, preserving the behaviour the archived spec established.
- **Priority:** Must-have

#### FR-MOD-105 [EARS-E]: A conflicted pull is completed by the merge tools
> WHEN a pull is paused by a conflict THE mcp-fs server SHALL accept `git.merge_resolve` and `git.merge_abort` as the tools that complete or undo it.

- **Original behavior:** No resolution tool existed; resolution was supplied up front.
- **Priority:** Must-have

#### FR-MOD-106 [EARS-E]: `git.branches` reports the current branch and tracking divergence
> WHEN `git.branches` is called THE mcp-fs server SHALL mark which branch is currently checked out and SHALL report, for each branch with a remote-tracking ref, how many commits it is ahead of and behind that ref.

- **Original behavior:** A flat list of branch names and shas.
- **Business Rules:** A branch with no remote-tracking ref reports `null` for both counts, which is distinguishable from `0`.
- **Priority:** Should-have

#### FR-MOD-107 [EARS-E]: `git.status` reports the in-progress operation
> WHEN `git.status` is called THE mcp-fs server SHALL include the `operation` object of FR-NEW-281 when an operation is in progress, and SHALL omit it otherwise.

- **Priority:** Must-have

#### FR-MOD-108 [EARS-E]: The git index owns a fourth table
> WHEN the git index declares the tables it owns THE mcp-fs server SHALL list four tables rather than three.

- **Original behavior:** `pub const TABLES: [&str; 3] = ["git_objects", "git_refs", "git_remotes"]` (`crates/mcp-fs/src/git/db.rs:42`).
- **New behavior (EARS):** the constant includes `git_operations`.
- **Priority:** Must-have

#### FR-MOD-109 [EARS-E]: The GitLab scope constant includes API access
> WHEN the GitLab device flow builds its scope string THE mcp-fs server SHALL include `api`.

- **Original behavior:** `read_repository write_repository` (`crates/mcp-fs/src/git/oauth/device_flow.rs:39`).
- **Reason for change:** Without it every merge-request tool is unauthorized. See FR-NEW-330.
- **Priority:** Must-have

### Removed Requirements

#### FR-DEL-101: The `on_conflict` parameter of `git.remote_pull` (references archived spec FR-NEW-031)
- **Description:** `git.remote_pull` accepted `on_conflict` with value `ours` or `theirs`, applied globally to every conflicting file, and refused a diverged pull outright when it was absent.
- **Reason:** Superseded by the shared conflict model (DEC-902, DEC-908). A global strategy forces the caller to decide before it can see what it is deciding about, and silently discards one side of every conflicting file.
- **Cleanup:** Remove the parameter from the `git.remote_pull` schema (`crates/mcp-fs/src/tools/git.rs:353-378`), remove `ConflictStrategy` and `parse_on_conflict` (`:1447-1485`) or fold them into the shared model's strategy parsing, regenerate the frozen contract, and update the shipped tests listed in Section 12.4.
- **Note:** This is a breaking change to a shipped tool contract. It is deliberate and is recorded in Section 13.
## 7. Non-Functional Requirements

Most of these are existing conventions that the new surface inherits rather than new
targets. They are stated because a new tool that quietly skips one of them is a defect.

### 7.1 Performance

| Requirement | Measure |
|---|---|
| A rebase replaying N commits performs O(N) three-way merges and holds the write lock throughout. | The todo length is bounded by `git.max_rebase_todo` (default 200) so worst-case duration is bounded. |
| `git.branch_switch` and `git.reset --hard` rewrite only paths that differ between the two trees. | Files identical in both trees are not rewritten and are not charged to the write quota. |
| `git.pr_diff` responses are bounded. | `git.max_pr_diff_mb`, default 12 MiB, truncating with an explicit marker. |
| No git operation blocks the async request thread. | Every libgit2 call continues to run on the blocking pool via the existing `on_git_thread` helper (`crates/mcp-fs/src/tools/git.rs:217-232`). |
| Provider REST calls carry a timeout. | `git.provider_api_timeout_secs`, default 30, so a hung provider cannot pin a request indefinitely. |

### 7.2 Security

- **Tokens never surface.** No token value appears in a tool response, in the `raw` provider payload, in an error message, in a log line, or in a tracing span field (FR-NEW-314). This is verified by a test that installs a tracing subscriber and inspects every captured field value, not by inspection.
- **Scope is checked before the network** (FR-NEW-332), mirroring the existing rule for expired tokens.
- **Authorization is unchanged**: membership only, platform admin excluded (FR-NEW-100). Every new tool joins the existing blanket authorization test (FR-NEW-347).
- **No cross-host redirect** is followed by the provider client (FR-NEW-316), because that would replay the Authorization header to another host.
- **Force push is leased** (FR-NEW-156) and audited with the sha it destroyed (FR-NEW-159).
- **Transport rules are unchanged**: HTTPS only, no credentials in URLs, host must be declared in `git.hosts`.
- **Secrets stay in the environment.** The provider client reads no new secret; it reuses the stored per-`(person, host)` token.

### 7.3 Usability

The tool surface is the user interface, and its consumer is an LLM. Therefore:

- Tool names mirror the git CLI (`git.rebase_continue`, not `git.operation_continue`), because that vocabulary is already in the model's weights (DEC-904).
- Every refusal names the remedy: the tool to call, the parameter to supply, or the state to clear. This is an existing convention in this codebase (the diverged-pull error names its remedy) and is retained.
- Error messages name concrete values, both sides of a lease mismatch, the unresolved paths, the offending todo entry.

### 7.4 Reliability

- **A paused operation survives a restart** (FR-NEW-278). This is the property that justified the relational operation record over in-memory state.
- **Every volume write is all-or-nothing** (FR-NEW-184). A conflict, a quota rejection or a mid-apply failure leaves the volume byte-identical to its prior state.
- **Abort is exact, not approximate** (FR-NEW-223): the pre-operation tip and file bytes are restored, which is why `original_tip_sha` is part of the operation record.
- **Refs are never advanced before their files land** (FR-NEW-184).
- **Orphaned commits are retained** (FR-NEW-255), so a mistaken reset or branch move is recoverable through the reported `old_sha`.

### 7.5 Observability

- Every destructive operation writes an audit entry through the existing capped audit log (`crates/mcp-fs/src/safety.rs`): branch delete, branch reset, reset, force push (with overwritten sha), stash drop, rebase, revert, cherry-pick, merge.
- Remote and provider operations emit a tracing span reusing the existing `git.remote` field set (operation, host, provider, branch, outcome, duration_ms), as asserted today at `crates/mcp-fs/src/tools/git.rs:6637-6652`.
- Never traced, per the project-wide rule: token values, credentials, and the contents of the `Authorization` header.
- **Out of scope**: OpenTelemetry export remains BL-001. This specification adds spans in the existing style and does not introduce an OTLP pipeline.

### 7.6 Deployment

Unchanged. No new service, no new infrastructure, no new REST route, no new outbound
dependency beyond provider REST endpoints reached with the already-present `reqwest`
client. Git remains optional behind `git.enabled`, and every tool in this specification is
registered only when it is on.

New configuration keys, all optional with defaults:

| Key | Default | Purpose |
|---|---|---|
| `git.max_stash_entries` | `100` | Bound on the per-volume stash pool |
| `git.max_rebase_todo` | `200` | Bound on a rebase todo list |
| `git.max_pr_diff_mb` | `12` | Bound on a `git.pr_diff` response |
| `git.provider_api_timeout_secs` | `30` | Timeout on provider REST calls |
| `git.github_scope` | `repo` | Scope requested by the GitHub device flow |
| `git.gitlab_scope` | `api read_repository write_repository` | Scope requested by the GitLab device flow |

### 7.7 Scalability

Explicitly unchanged and explicitly limited. The per-project write lock remains an
in-process `tokio::sync::Mutex` (`crates/mcp-fs/src/git/repo.rs:44`), so the deployment
assumption of a single server replica is retained. Two replicas would each acquire their
own lock for the same repository. The new `git_operations` table is visible to every
replica and so does not make the situation worse, but it does not fix it either. BL-014
remains the open item.

## 8. Data Model

### 8.1 The new table

`git_operations`, in the git relational index, one row per volume at most.

| Column | Type | Notes |
|---|---|---|
| `volume_id` | `TextKey(VOLUME_ID_LEN)` | Part of the primary key; scopes every query |
| `op_type` | `TextKey(32)` | Exactly one of `merge`, `rebase`, `cherry_pick`, `revert`, `stash_apply`, `stash_pop` (FR-NEW-285) |
| `state` | `TextKey(32)` | Exactly one of `conflicted` or `running`. `paused` is not a valid value. |
| `onto_sha` | `Text` | The commit being replayed onto, for rebase |
| `original_tip_sha` | `Text` | The branch tip before the operation began; makes abort exact |
| `original_head_ref` | `Text` | The ref HEAD pointed at when the operation began |
| `source_sha` | `Text` | The commit or ref being applied, for cherry-pick, revert, merge |
| `todo` | `Text` | JSON array of remaining todo entries, for rebase |
| `current_step` | `BigInt` | **Zero-based** index of the paused entry, so `todo[current_step]` is the entry awaiting resolution |
| `total_steps` | `BigInt` | Total entries, so a caller can report progress |
| `conflicts` | `Text` | JSON array of unresolved conflicting paths |
| `resolutions` | `Text` | JSON object of resolutions accepted so far |
| `stash_id` | `Text` | The stash entry being applied, for `stash_apply`; retained so it is not dropped |
| `created_at` | `BigInt` | Epoch millis |
| `updated_at` | `BigInt` | Epoch millis |

Primary key `(volume_id)`, since at most one operation is in progress per volume
(FR-NEW-283). Keyed columns use `TextKey(n)` because SQL Server cannot index unbounded
text; payload columns use unbounded `Text`.

`TABLES` grows from three entries to four (FR-NEW-276, FR-MOD-108).

### 8.2 Entities carried in existing storage

| Entity | Where it lives | Note |
|---|---|---|
| Branch | `git_refs` under `refs/heads/*` | No schema change |
| Stash entry | `git_refs` under `refs/stash/*`, pointing at a commit object | No schema change. `refs/stash` is unused today. |
| Remote | `git_remotes` | No schema change; the table already supports arbitrary names, only the writer was limited |
| Remote-tracking ref | `git_refs` under `refs/remotes/{remote}/*` | No schema change |
| Commit, tree, blob | Blob store under `git:{sha}`, indexed in `git_objects` | No schema change |
| Token and scopes | `oauth_tokens`, `scopes` column | No schema change; only the requested scope string and its validation change |

### 8.3 The conflict response

Returned identically by every combine operation (FR-NEW-170):

```
{
  "status": "conflict",
  "operation": "merge" | "rebase" | "cherry_pick" | "revert" | "stash_apply",
  "operation_id": "<opaque>",
  "source_ref": "<ref or sha being applied, null when not applicable>",
  "current_step": <int|null>,
  "total_steps": <int|null>,
  "conflicts": [
    {
      "path": "/src/a.txt",
      "ours":   { "content": "...", "exists": true },
      "theirs": { "content": "...", "exists": true },
      "base":   { "content": "...", "exists": true },
      "binary": false,
      "type_change": false
    }
  ],
  "continue_with": "git.merge_resolve",
  "abort_with": "git.merge_abort"
}
```

`continue_with` and `abort_with` name the exact tools for the active operation, so the
caller never has to infer them.

### 8.4 The normalized pull request model

Returned identically for a GitHub pull request and a GitLab merge request (FR-NEW-303):

```
{
  "number": 42,  // key order is normative, see FR-NEW-317
  "title": "...",
  "body": "...",
  "provider": "github" | "gitlab",
  "host": "github.com",
  "state": "open" | "closed" | "merged",
  "draft": false,
  "base": "main",
  "head": "feature/x",
  "author": "someone",
  "url": "https://.../42",
  "created_at": "...", "updated_at": "...",
  "review_state": "approved" | "changes_requested" | "review_required" | "none",
  "checks_state": "success" | "failure" | "pending" | "none",
  "commits": [...], "changed_files": [...],
  "raw": { ... provider payload, scrubbed of authorization data ... }
}
```

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact | Description |
|---|---|---|
| `crates/mcp-fs/src/tools/git.rs` | Major | 25 new tool registrations and handlers (the 31 of FR-NEW-349 less the 6 PR tools, which live in the new `git_pr.rs`); `register()` currently spans `:59-360`. Extract the merge block at `:1835-1839` and `apply_pull_changes_atomically` (`:2079-2127`) into a shared engine used by merge, rebase, cherry-pick, revert and stash apply. Remove `ConflictStrategy` / `parse_on_conflict` (`:1447-1485`) per FR-DEL-101. |
| `crates/mcp-fs/src/tools/git_pr.rs` *(new)* | Major | The 6 PR tools and the normalization layer. |
| `crates/mcp-fs/src/git/provider/` *(new)* | Major | Provider REST client behind an injectable trait (FR-NEW-315), with GitHub and GitLab implementations and base-URL resolution. |
| `crates/mcp-fs/src/git/db.rs` | Moderate | New `git_operations` table in `schema()` (`:66-90`); `TABLES` at `:42` grows to four; CRUD for the operation record. |
| `crates/mcp-fs/src/git/repo.rs` | Minor | Write lock (`:44`), today taken at `tools/git.rs:660`, `:964`, `:1223`, `:1398`, `:1770`, is now taken by every new ref-mutating tool. |
| `crates/mcp-fs/src/git/oauth/device_flow.rs` | Minor | GitLab scope constant at `:39` gains `api`; both constants become configurable (`:38-39`). |
| `crates/mcp-fs/src/git/oauth/store.rs` | Minor | Scope sufficiency query for FR-NEW-332. |
| `crates/mcp-fs/src/config.rs` | Minor | Six new optional keys (Section 7.6); `github_client_id` / `gitlab_client_id` are at `:455` and `:457`. |
| `crates/mcp-fs/src/tools/all.rs` | Minor | Registration wiring plus count assertions at `:64`, `:78`, `:99`, `:122`, `:141`. |
| `crates/mcp-fs/src/tools/contract_golden.rs` | Minor | Count assertion at `:131`, 63 to 94. |
| `crates/mcp-fs/src/api/dataplane.rs`, `api/openapi.rs` | **None** | Git is MCP-only; all 38 REST routes are `fs.*` (`dataplane.rs:45`, count asserted at `:2328`). The git paths in `openapi.rs:74-76` are smart-HTTP wire routes, not data-plane routes. Verified. |
| `crates/mcp-fs/src/storage/` | **None** | No blob-store change; git objects keep the `git:{sha}` layout. |

### 9.2 Affected Requirements

| Spec | Requirement | Impact | Description |
|---|---|---|---|
| `specs/archived/…token-store.md` | FR-NEW-031 | **Superseded** | Global `on_conflict` merge replaced by the shared conflict model (FR-MOD-104, FR-DEL-101). |
| `specs/archived/…token-store.md` | FR-NEW-029 | **Modified** | A diverged pull is no longer refused for want of a strategy; the dirty-volume refusal is retained. |
| `specs/archived/…token-store.md` | FR-NEW-022 | **Extended** | Push gains `remote` and `remote_branch` (FR-MOD-101, FR-MOD-102). |
| `specs/archived/…token-store.md` | FR-NEW-024 | **Extended** | Fast-forward-only push gains the leased force path (FR-NEW-155). |
| `specs/archived/…token-store.md` | FR-NEW-020 | **Extended** | `origin` is no longer the only possible remote (FR-NEW-140). |
| `specs/archived/…token-store.md` | FR-NEW-032 | **Retained unchanged** | Conflict markers still never enter the volume (FR-NEW-172). |
| `specs/2026-09-18_19-45-00-git.md` | git tool surface | **Extended** | 18 tools become 49 in the git family. |
| `specs/…platform-foundation.md` | error codes | **Unchanged** | No new `ERR_*` code is introduced. |

### 9.3 Affected Tests

| Test file | Test | Action | Description |
|---|---|---|---|
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_117_a_diverged_pull_with_no_strategy_is_refused` | **Remove** | The refusal it pins no longer exists; replaced by auto-merge or a conflict response. |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_126_the_diverged_pull_error_names_the_remedy` | **Remove** | The error it pins no longer exists. |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_118_a_diverged_pull_with_theirs_merges` | **Modify** | Rewrite onto `git.merge_resolve` with a per-file strategy. |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_128_a_diverged_pull_with_ours_keeps_local_content` | **Modify** | Same rewrite with `ours`. |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_129_non_conflicting_changes_from_both_sides_are_kept` | **Modify** | Now completes automatically with no strategy parameter at all (FR-NEW-173). |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_125_a_refused_diverged_pull_keeps_the_fetched_objects` | **Modify** | The retention guarantee survives, but the trigger becomes a conflict response rather than a refusal. |
| `crates/mcp-fs/src/tools/git.rs` | `e2e_new_134_a_degenerate_diverged_pull_is_a_fast_forward` | **Keep** | Unaffected. |
| `crates/mcp-fs/src/tools/git.rs:2480-2504` | tool name list and `assert_eq!(r.len(), 14)` at `:2500` | **Modify** | Add 31 names, count to 45. |
| `crates/mcp-fs/src/tools/git.rs:2685` | `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` | **Modify** | Add all 31 new tools (FR-NEW-347). |
| `crates/mcp-fs/src/tools/all.rs` | count assertions at `:78`, `:122`, `:141` | **Modify** | Update these three. `:64` (admin-only, 10) and `:99` (git-disabled, 52) are unaffected by adding git tools and MUST NOT be edited. |
| `crates/mcp-fs/src/tools/contract_golden.rs:131` | frozen contract count | **Modify** | 63 to 94. |
| `tool-contract-golden.json`, `TOOL_CONTRACT.txt` | whole file | **Regenerate** | `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current`, then review the diff. |

Test gap that this specification must close: there is no existing test harness for an
outbound provider REST API. FR-NEW-315 requires the injectable client seam, and the tests
in Section 12 specify the fake and its canned payloads.

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | `:5` overview tool counts | Update 63 to 94, and the git family from `14 git.* / 4 git.auth*` to `45 git.* / 4 git.auth*` |
| `AGENTS.md` | `:88` TOOL_CONTRACT reference | Update count |
| `AGENTS.md` | `:155` tools.md reference | Update count |
| `AGENTS.md` | "Behaviour worth knowing" | Add the operation record, the lease, and the conflict model |
| `.agent_docs/tools.md` | `:1` header, `:157` git section | Update counts; add a row per new tool |
| `.agent_docs/git.md` | whole | Document the operation record, the shared conflict model, rebase semantics, the remote model, and the provider REST layer |
| `.agent_docs/backends.md` | dialect checklist | Note the new table |
| `README.md` | feature list | Update the git description |
| `specs/BACKLOG.md` | Theme "Git full support" | Mark BL-004, BL-005, BL-006, BL-008, BL-009, BL-010 as addressed here; BL-007 partially; BL-011, BL-012, BL-014 remain |

### 9.5 Dependencies & Risks

**New dependencies:** none required. `reqwest` is already a workspace dependency used by
the OAuth device flow (`crates/mcp-fs/src/tools/git_auth.rs:39`) and other subsystems;
`git2` already provides the merge and rebase primitives; `serde_json` already serializes
the payload columns.

**Breaking changes:**
1. `on_conflict` is removed from `git.remote_pull` (FR-DEL-101). Any caller passing it
   receives an unknown-parameter outcome, and the diverged-pull behaviour changes shape.
   This is a deliberate, user-approved contract break.
2. The frozen tool contract changes, which is by design but means every client that
   pinned the contract sees a new hash.

**Risks:**

| Risk | Mitigation |
|---|---|
| The shared conflict engine is extracted from a working pull path and could regress it. | The existing pull tests stay green apart from the six explicitly listed; extraction is refactor-then-extend, not rewrite. |
| A stale `git_operations` row permanently blocks a volume. | Completion and abort both delete the row (FR-NEW-284), verified by test; `git.status` always reports the blocker and the tools that clear it. |
| Rebase holds the write lock for a long replay. | Todo length is bounded (FR-NEW-216). |
| Provider API shapes drift. | The `raw` passthrough means a field the normalizer does not know about is still reachable; the injectable client makes a shape change testable offline. |
| Force push destroys remote history. | Lease mandatory (FR-NEW-156), overwritten sha audited (FR-NEW-159). |
| The GitLab scope change requires re-authorization. | Existing GitLab tokens lack `api` and will fail the scope check with an actionable error (FR-NEW-333) telling the user to re-run `git.auth`. This is a user-visible migration step and is called out in Section 14. |

**Rollback strategy:** every tool is additive apart from FR-DEL-101 and FR-MOD-104.
Reverting the feature means unregistering the new tools, dropping `git_operations`, and
restoring the `on_conflict` parameter. Because operation rows are transient by nature,
dropping the table loses only in-flight operations, which abort cleanly.

## 10. Documentation Requirements

### 10.1 README.md
Update the git feature description from a remote-pipeline summary to a development-process
summary, and state the single-replica assumption that the write lock implies.

### 10.2 AGENTS.md and .agent_docs/
- `AGENTS.md`: tool counts at `:5`, `:88`, `:155`; the git bullet in "Behaviour worth knowing" gains the operation record, the mandatory lease and the shared conflict model; the conventions section gains the rule that every ref-mutating tool takes the write lock and checks the in-progress guard.
- `.agent_docs/tools.md`: counts at `:1` and `:157`, plus one row per new tool with parameters and authorization.
- `.agent_docs/git.md`: the substantive documentation, covering the operation lifecycle and its states, the conflict response and resolution forms, rebase todo semantics, the remote model beyond `origin`, force-push leasing, and the provider REST layer including base-URL resolution for GitHub Enterprise and self-hosted GitLab.
- `.agent_docs/backends.md`: note `git_operations` in the dialect checklist.
- `.agent_docs/lineage.md`: record that this specification supersedes the archived spec's pull conflict behaviour and why.

### 10.3 docs/*
Add a short operator note describing the user-visible migration: existing GitLab tokens
lack the `api` scope and must be re-granted through `git.auth` before merge-request tools
work.
## 11. Traceability Matrix

One row per usage scenario of Section 5. Test IDs are written without the
`E2E-NEW-` prefix for width; every one of them is fully specified in Section 12.2.
Every scenario carries at least one test in each column; the matrix contains no gaps.
Where a scenario's "happy path" is a correct refusal, as with the force-push lease in
SC-910, the successful-refusal test occupies that column.

| Scenario | Functional Requirements | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge/Other) |
|---|---|---|---|---|
| SC-901 | FR-MOD-106, FR-NEW-100, FR-NEW-101, FR-NEW-102, FR-NEW-103, FR-NEW-104, FR-NEW-105, FR-NEW-106, FR-NEW-107, FR-NEW-108, FR-NEW-109, FR-NEW-110, FR-NEW-115, FR-NEW-184 | 400, 401, 413, 422, 425, 442, 443 | 402, 403, 404, 407, 408, 410, 415, 416, 418, 423, 424, 427 | 405, 406, 409, 411, 412, 414, 417, 419, 420, 421, 426, 428, 429, 444, 445 |
| SC-902 | FR-NEW-100, FR-NEW-115, FR-NEW-120, FR-NEW-121, FR-NEW-122, FR-NEW-123, FR-NEW-124, FR-NEW-125, FR-NEW-127, FR-NEW-128, FR-NEW-130, FR-NEW-185 | 446, 453, 455, 456, 461 | 449, 450, 458, 459, 460, 466 | 447, 448, 451, 452, 454, 457, 467, 468, 469, 470 |
| SC-903 | FR-NEW-100, FR-NEW-173, FR-NEW-185, FR-NEW-190, FR-NEW-192, FR-NEW-193, FR-NEW-194, FR-NEW-195 | 500, 501, 510 | 511, 512, 513, 514, 515, 516, 517, 536, 538, 539 | 542, 545, 546, 547, 549 |
| SC-904 | FR-MOD-108, FR-NEW-100, FR-NEW-170, FR-NEW-171, FR-NEW-172, FR-NEW-174, FR-NEW-177, FR-NEW-178, FR-NEW-179, FR-NEW-180, FR-NEW-181, FR-NEW-182, FR-NEW-183, FR-NEW-184, FR-NEW-185, FR-NEW-196, FR-NEW-197, FR-NEW-198, FR-NEW-275, FR-NEW-284 | 502, 503, 507 | 518, 519, 520, 523, 524, 525, 526, 527, 537 | 548, 550, 551, 552, 553, 555, 556, 557, 560, 561, 562, 563, 564, 565, 566, 568, 569, 573, 574, 575, 576 |
| SC-905 | FR-NEW-175, FR-NEW-176, FR-NEW-179, FR-NEW-183, FR-NEW-196 | 504, 505 | 521, 522 | 567, 570, 571, 572 |
| SC-906 | FR-NEW-191 | 506 | 541 | 543, 544 |
| SC-907 | FR-MOD-101, FR-MOD-103, FR-NEW-100, FR-NEW-140, FR-NEW-141, FR-NEW-142, FR-NEW-143, FR-NEW-144, FR-NEW-145, FR-NEW-146, FR-NEW-147 | 471, 477, 481 | 472, 473, 474, 476, 478, 483, 486 | 475, 479, 480, 482, 487, 488 |
| SC-908 | FR-MOD-102, FR-NEW-160 | 484 | 494 | 485 |
| SC-909 | FR-NEW-155, FR-NEW-156, FR-NEW-158, FR-NEW-159 | 490 | 489, 496, 497 | 491, 495 |
| SC-910 | FR-NEW-157, FR-NEW-158 | 492 | 493, 498, 831 | 496, 497 |
| SC-911 | FR-MOD-104, FR-NEW-173 | 940 | 941, 942 | 943, 944 |
| SC-912 | FR-DEL-101, FR-MOD-104, FR-MOD-105, FR-NEW-172, FR-NEW-174 | 508 | 535 | 558 |
| SC-913 | FR-MOD-104, FR-MOD-105, FR-NEW-175 | 945 | 946, 947 | 948, 949 |
| SC-914 | FR-NEW-100, FR-NEW-210, FR-NEW-211, FR-NEW-212, FR-NEW-215, FR-NEW-216, FR-NEW-217, FR-NEW-255, FR-NEW-284 | 600, 606 | 622, 623, 624, 625, 626, 627, 628, 631, 632, 633, 634 | 607, 608, 609, 610, 611, 612, 639 |
| SC-915 | FR-NEW-212, FR-NEW-213, FR-NEW-214 | 601, 602, 603, 604 | 629, 630 | 605 |
| SC-916 | FR-NEW-171, FR-NEW-172, FR-NEW-174, FR-NEW-175, FR-NEW-219, FR-NEW-220, FR-NEW-221, FR-NEW-224, FR-NEW-226, FR-NEW-278 | 614, 615, 617 | 613, 635 | 616, 618, 694, 697 |
| SC-917 | FR-NEW-223, FR-NEW-224, FR-NEW-284 | 619 | 636 | 620, 621 |
| SC-918 | FR-NEW-100, FR-NEW-185, FR-NEW-235, FR-NEW-236, FR-NEW-237, FR-NEW-240 | 640, 641, 642 | 652, 653, 654, 657, 658 | 643, 644, 648, 649, 650, 651, 659 |
| SC-919 | FR-NEW-171, FR-NEW-172, FR-NEW-174, FR-NEW-224, FR-NEW-225, FR-NEW-238, FR-NEW-239 | 646, 647 | 645, 655, 656 | 559 |
| SC-920 | FR-NEW-100, FR-NEW-250, FR-NEW-252 | 660 | 671 | 664, 674 |
| SC-921 | FR-NEW-185, FR-NEW-251, FR-NEW-252, FR-NEW-253, FR-NEW-254, FR-NEW-255 | 661, 662 | 669, 670, 672, 673 | 663, 665, 666, 667, 668 |
| SC-922 | FR-NEW-260, FR-NEW-263, FR-NEW-264, FR-NEW-265, FR-NEW-284 | 675, 676, 677, 684 | 683, 689 | 680, 681, 682 |
| SC-923 | FR-NEW-261, FR-NEW-262 | 678 | 685, 686, 687, 688 | 679 |
| SC-924 | FR-NEW-100, FR-NEW-106, FR-NEW-111, FR-NEW-112, FR-NEW-113, FR-NEW-114, FR-NEW-115 | 430, 434 | 432, 436, 437, 438 | 431, 433, 435, 439, 440, 441 |
| SC-925 | FR-MOD-109, FR-NEW-100, FR-NEW-300, FR-NEW-301, FR-NEW-302, FR-NEW-303, FR-NEW-304, FR-NEW-305, FR-NEW-311, FR-NEW-314, FR-NEW-315, FR-NEW-316, FR-NEW-330, FR-NEW-331, FR-NEW-332, FR-NEW-333, FR-NEW-334, FR-NEW-335, FR-NEW-345, FR-NEW-346, FR-NEW-347 | 700, 701, 702, 703, 710, 711, 712, 713, 773, 774, 775, 783 | 704, 705, 706, 707, 708, 714, 715, 716, 717, 718, 719, 720, 777, 781, 785 | 721, 722, 782, 786, 787, 788, 789, 790, 791, 792, 794, 799 |
| SC-926 | FR-NEW-300, FR-NEW-306, FR-NEW-311, FR-NEW-313, FR-NEW-315, FR-NEW-316, FR-NEW-332, FR-NEW-333 | 723, 724 | 726, 728, 729, 776 | 725, 727, 730, 731, 795 |
| SC-927 | FR-NEW-303, FR-NEW-307, FR-NEW-308, FR-NEW-309, FR-NEW-311, FR-NEW-312, FR-NEW-313, FR-NEW-314, FR-NEW-332 | 732, 733, 742, 743, 763, 764, 765, 766, 768, 778 | 738, 739, 740, 741, 747, 748, 769, 770, 771, 772 | 709, 734, 735, 736, 737, 744, 745, 746, 749, 767, 784, 793, 798 |
| SC-928 | FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-311, FR-NEW-312, FR-NEW-332 | 750, 751, 752, 753, 754 | 756, 757, 758, 759, 760, 761, 779, 780 | 755, 762, 796, 797 |
| SC-929 | FR-MOD-107, FR-MOD-108, FR-NEW-100, FR-NEW-115, FR-NEW-225, FR-NEW-226, FR-NEW-275, FR-NEW-277, FR-NEW-279, FR-NEW-280, FR-NEW-281, FR-NEW-282, FR-NEW-283, FR-NEW-345, FR-NEW-346, FR-NEW-347 | 509 | 528, 529, 530, 531, 532, 533, 534, 637, 638, 690, 691, 698 | 554, 577, 578, 579, 580, 692, 693, 695, 696, 699 |
| SC-930 | FR-NEW-126, FR-NEW-129, FR-NEW-170, FR-NEW-171, FR-NEW-177 | 950 | 462, 540 | 463, 464, 465, 899 |

`SC-911` (a divergent pull that merges automatically) and `SC-913` (a divergent pull
conflict resolved by supplied content) are behaviour changes to the shipped pull path
rather than new tools. They are covered twice over: by the dedicated tests
`E2E-NEW-940..944` and `E2E-NEW-945..949`, and by the modified shipped tests of
Section 12.3 (`E2E-MOD-403` for `SC-911`, `E2E-MOD-401` and `E2E-MOD-402` for the
resolution mechanism).

## 12. End-to-End Test Suite

### 12.1 Test Summary

| Test ID | Action (New) | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-400 | `branch_create "feature/login"` from `<sha-C2>`, no checkout | Happy | SC-901 | FR-NEW-101 | P0 |
| E2E-NEW-401 | `branch_create` with `checkout:true` from `<sha-C1>` | Happy | SC-901 | FR-NEW-101, FR-NEW-105 | P0 |
| E2E-NEW-402 | create a name that already exists | Failure | SC-901 | FR-NEW-102 | P0 |
| E2E-NEW-403 | create from an unknown start point | Failure | SC-901 | FR-NEW-103 | P0 |
| E2E-NEW-404 | create with `"bad..name"` | Failure | SC-901 | FR-NEW-104, FR-NEW-116 | P0 |
| E2E-NEW-405 | unicode branch name `feature/café-日本` | EdgeCase | SC-901 | FR-NEW-104, FR-NEW-101, FR-NEW-116 | P1 |
| E2E-NEW-406 | 255-byte branch name | EdgeCase | SC-901 | FR-NEW-104, FR-NEW-116 | P2 |
| E2E-NEW-407 | 256-byte branch name | Failure | SC-901 | FR-NEW-104, FR-NEW-116 | P2 |
| E2E-NEW-408 | `checkout:true` on a dirty volume | Failure | SC-901 | FR-NEW-106, FR-NEW-101 | P0 |
| E2E-NEW-409 | after 009, no ref row and no audit entry | SideEffect | SC-901 | FR-NEW-106, FR-NEW-101 | P0 |
| E2E-NEW-410 | create in a repo with no commits | Failure | SC-901 | FR-NEW-103 | P1 |
| E2E-NEW-411 | non-member `branch_create` | Security | SC-901 | FR-NEW-100, FR-NEW-189, FR-NEW-199 | P0 |
| E2E-NEW-412 | platform admin (non-member) `branch_create` | Security | SC-901 | FR-NEW-100 | P0 |
| E2E-NEW-413 | `branch_switch "release/1.0"` | Happy | SC-901 | FR-NEW-105, FR-NEW-189, FR-NEW-199 | P0 |
| E2E-NEW-414 | volume content after switch | SideEffect | SC-901 | FR-NEW-105, FR-NEW-189 | P0 |
| E2E-NEW-415 | switch with a dirty volume | Failure | SC-901 | FR-NEW-106 | P0 |
| E2E-NEW-416 | switch to an unknown branch | Failure | SC-901 | FR-NEW-105 | P0 |
| E2E-NEW-417 | switch to the already-checked-out branch | EdgeCase | SC-901 | FR-NEW-107 | P1 |
| E2E-NEW-418 | switch under an exhausted quota | Failure | SC-901 | FR-NEW-105 | P0 |
| E2E-NEW-419 | after 019, the volume is unchanged | SideEffect | SC-901 | FR-NEW-105, FR-NEW-184 | P0 |
| E2E-NEW-420 | quota charged + audit written on a good switch | SideEffect | SC-901 | FR-NEW-105 | P0 |
| E2E-NEW-421 | two `branch_switch` calls race | Concurrency | SC-901 | FR-NEW-115 | P1 |
| E2E-NEW-422 | delete a merged branch | Happy | SC-901 | FR-NEW-108, FR-NEW-189, FR-NEW-199 | P0 |
| E2E-NEW-423 | delete the checked-out branch | Failure | SC-901 | FR-NEW-109, FR-NEW-189 | P0 |
| E2E-NEW-424 | delete an unmerged branch without force | Failure | SC-901 | FR-NEW-110 | P0 |
| E2E-NEW-425 | delete an unmerged branch with `force:true` | Happy | SC-901 | FR-NEW-108, FR-NEW-110 | P0 |
| E2E-NEW-426 | after 026, `<sha-C3>` is still readable | DataIntegrity | SC-901 | FR-NEW-108 | P1 |
| E2E-NEW-427 | delete an unknown branch | Failure | SC-901 | FR-NEW-108 | P1 |
| E2E-NEW-428 | after 024/025, the ref still equals its old sha | SideEffect | SC-901 | FR-NEW-109, FR-NEW-110 | P0 |
| E2E-NEW-429 | `force:true` on the checked-out branch | EdgeCase | SC-901 | FR-NEW-109 | P1 |
| E2E-NEW-430 | `branch_reset` a non-checked-out branch backwards, `force:true` | Happy | SC-924 | FR-NEW-111, FR-NEW-189, FR-NEW-199 | P0 |
| E2E-NEW-431 | after 031, the volume is untouched | SideEffect | SC-924 | FR-NEW-112, FR-NEW-189 | P0 |
| E2E-NEW-432 | non-fast-forward reset without force | Failure | SC-924 | FR-NEW-113 | P0 |
| E2E-NEW-433 | fast-forward reset without force | EdgeCase | SC-924 | FR-NEW-113 | P0 |
| E2E-NEW-434 | `force:true` reset of the checked-out branch | Happy | SC-924 | FR-NEW-111, FR-NEW-112 | P0 |
| E2E-NEW-435 | after 035, `/src/lib.rs` is gone and quota/audit are charged | SideEffect | SC-924 | FR-NEW-112, FR-NEW-189 | P0 |
| E2E-NEW-436 | unknown `target_commit` | Failure | SC-924 | FR-NEW-114 | P1 |
| E2E-NEW-437 | unknown branch `name` | Failure | SC-924 | FR-NEW-114 | P1 |
| E2E-NEW-438 | reset of the checked-out branch with a dirty volume, `force:false` | Failure | SC-924 | FR-NEW-112, FR-NEW-106 | P0 |
| E2E-NEW-439 | after 033/039, the ref is unchanged | SideEffect | SC-924 | FR-NEW-113, FR-NEW-114 | P0 |
| E2E-NEW-440 | `branch_reset` races `git.commit` on the same branch | Concurrency | SC-924 | FR-NEW-115 | P1 |
| E2E-NEW-441 | non-member `branch_reset`/`branch_delete` | Security | SC-924 | FR-NEW-100 | P0 |
| E2E-NEW-442 | `git.branches` marks the current branch | Happy | SC-901 | FR-MOD-106 | P0 |
| E2E-NEW-443 | ahead/behind vs `refs/remotes/origin/main` | Happy | SC-901 | FR-MOD-106 | P0 |
| E2E-NEW-444 | branch with no remote-tracking ref | EdgeCase | SC-901 | FR-MOD-106 | P0 |
| E2E-NEW-445 | `git.branches` on a repo with no commits | EdgeCase | SC-901 | FR-MOD-106 | P1 |
| E2E-NEW-446 | `stash_save "wip: login"` | Happy | SC-902 | FR-NEW-120, FR-NEW-199 | P0 |
| E2E-NEW-447 | after 047, the volume equals the HEAD tree | SideEffect | SC-902 | FR-NEW-120 | P0 |
| E2E-NEW-448 | after 047, `refs/stash/<id>` row exists | SideEffect | SC-902 | FR-NEW-120, FR-NEW-123, FR-NEW-131 | P0 |
| E2E-NEW-449 | `stash_save` on a clean volume | Failure | SC-902 | FR-NEW-121 | P0 |
| E2E-NEW-450 | `stash_save` in a repo with no commits | Failure | SC-902 | FR-NEW-121 | P1 |
| E2E-NEW-451 | quota + audit on `stash_save` | SideEffect | SC-902 | FR-NEW-120 | P1 |
| E2E-NEW-452 | unicode + 1000-char stash message | EdgeCase | SC-902 | FR-NEW-120, FR-NEW-122 | P2 |
| E2E-NEW-453 | `stash_list` with two entries | Happy | SC-902 | FR-NEW-122, FR-NEW-131 | P0 |
| E2E-NEW-454 | `stash_list` on an empty pool | EdgeCase | SC-902 | FR-NEW-122, FR-NEW-131 | P0 |
| E2E-NEW-455 | `stash_apply` | Happy | SC-902 | FR-NEW-124, FR-NEW-131, FR-NEW-132, FR-NEW-199 | P0 |
| E2E-NEW-456 | `stash_pop` | Happy | SC-902 | FR-NEW-125, FR-NEW-132 | P0 |
| E2E-NEW-457 | after 057, `refs/stash/<id>` row is gone | SideEffect | SC-902 | FR-NEW-125, FR-NEW-132, FR-NEW-199 | P0 |
| E2E-NEW-458 | `stash_apply` unknown id | Failure | SC-902 | FR-NEW-128 | P0 |
| E2E-NEW-459 | `stash_pop` unknown id | Failure | SC-902 | FR-NEW-128 | P0 |
| E2E-NEW-460 | `stash_drop` unknown id | Failure | SC-902 | FR-NEW-128 | P1 |
| E2E-NEW-461 | `stash_drop` a known id | Happy | SC-902 | FR-NEW-127 | P0 |
| E2E-NEW-462 | `stash_apply` that conflicts | Failure | SC-930 | FR-NEW-126, FR-NEW-170, FR-NEW-132 | P0 |
| E2E-NEW-463 | `stash_pop` that conflicts keeps the entry | DataIntegrity | SC-930 | FR-NEW-126, FR-NEW-171 | P0 |
| E2E-NEW-464 | stash survives deletion of its origin branch | EdgeCase | SC-930 | FR-NEW-129 | P0 |
| E2E-NEW-465 | stash taken on `main`, applied on `release/1.0` | EdgeCase | SC-930 | FR-NEW-129, FR-NEW-132 | P1 |
| E2E-NEW-466 | `stash_apply` under an exhausted quota | Failure | SC-902 | FR-NEW-124, FR-NEW-185 | P0 |
| E2E-NEW-467 | 100 stashes accepted, the 101st refused | EdgeCase | SC-902 | FR-NEW-130 | P2 |
| E2E-NEW-468 | non-member `stash_list`/`stash_save` | Security | SC-902 | FR-NEW-100 | P0 |
| E2E-NEW-469 | two `stash_save` calls race | Concurrency | SC-902 | FR-NEW-120, FR-NEW-115 | P1 |
| E2E-NEW-470 | stash refs never appear in `git.branches` | DataIntegrity | SC-902 | FR-NEW-123 | P1 |
| E2E-NEW-471 | `remote_add "upstream"` + `remote_list` | Happy | SC-907 | FR-NEW-140, FR-NEW-144 | P0 |
| E2E-NEW-472 | `remote_add` a duplicate name | Failure | SC-907 | FR-NEW-141 | P0 |
| E2E-NEW-473 | `remote_add` with `ssh://` / `git://` / scp shorthand | Failure | SC-907 | FR-NEW-140 | P0 |
| E2E-NEW-474 | `remote_add` with userinfo in the URL | Failure | SC-907 | FR-NEW-140, FR-NEW-145 | P0 |
| E2E-NEW-475 | after 073/074, `git_remotes` holds no new row | SideEffect | SC-907 | FR-NEW-141, FR-NEW-140 | P0 |
| E2E-NEW-476 | `remote_add` with an invalid remote name | Failure | SC-907 | FR-NEW-140 | P1 |
| E2E-NEW-477 | `remote_remove "upstream"` | Happy | SC-907 | FR-NEW-142 | P0 |
| E2E-NEW-478 | `remote_remove` an unknown name | Failure | SC-907 | FR-NEW-143 | P0 |
| E2E-NEW-479 | `remote_remove "origin"` then `remote_push` | EdgeCase | SC-907 | FR-NEW-142, FR-NEW-146 | P1 |
| E2E-NEW-480 | `remote_list` with no remotes | EdgeCase | SC-907 | FR-NEW-144 | P1 |
| E2E-NEW-481 | `remote_fetch {remote:"upstream"}` | Happy | SC-907 | FR-NEW-147, FR-MOD-103 | P0 |
| E2E-NEW-482 | after 082, `refs/remotes/origin/*` is byte-identical | SideEffect | SC-907 | FR-NEW-147 | P0 |
| E2E-NEW-483 | `remote_fetch {remote:"nope"}` | Failure | SC-907 | FR-NEW-146, FR-MOD-103 | P0 |
| E2E-NEW-484 | `remote_push {remote_branch:"sandbox"}` | Happy | SC-908 | FR-MOD-102 | P0 |
| E2E-NEW-485 | after 085, tracking ref is `refs/remotes/origin/sandbox` | SideEffect | SC-908 | FR-MOD-102 | P0 |
| E2E-NEW-486 | `remote_push {remote:"nope"}` | Failure | SC-907 | FR-NEW-146, FR-MOD-101 | P0 |
| E2E-NEW-487 | `remote_push` with `remote` omitted | EdgeCase | SC-907 | FR-MOD-101 | P0 |
| E2E-NEW-488 | non-member `remote_add`/`remote_remove` | Security | SC-907 | FR-NEW-100, FR-NEW-145 | P0 |
| E2E-NEW-489 | `force:true` without `expected_remote_sha` | Failure | SC-909 | FR-NEW-156 | P0 |
| E2E-NEW-490 | force push with a matching lease, non-FF | Happy | SC-909 | FR-NEW-155 | P0 |
| E2E-NEW-491 | after 091, the audit names the overwritten sha | SideEffect | SC-909 | FR-NEW-159 | P0 |
| E2E-NEW-492 | force push with a stale lease | Failure | SC-910 | FR-NEW-157 | P0 |
| E2E-NEW-493 | after 093, the bare repo and the tracking ref are unchanged | SideEffect | SC-910 | FR-NEW-157 | P0 |
| E2E-NEW-494 | `force:false` non-FF | Failure | SC-908 | FR-NEW-160 | P0 |
| E2E-NEW-495 | force push of a branch absent on the remote, lease = 40 zeros | EdgeCase | SC-909 | FR-NEW-155 | P1 |
| E2E-NEW-496 | malformed `expected_remote_sha` | Failure | SC-909 | FR-NEW-156 | P1 |
| E2E-NEW-497 | `expected_remote_sha` with `force:false` | Failure | SC-909 | FR-NEW-158 | P2 |
| E2E-NEW-498 | the remote advances between lease check and push | Concurrency | SC-910 | FR-NEW-157 | P0 |
| E2E-NEW-500 | `git.merge feature` on SEED-AUTOMERGE | Happy | SC-903 | FR-NEW-190, FR-NEW-173 | P0 |
| E2E-NEW-501 | `git.merge feature` on SEED-FF (already merged base) | Happy | SC-903 | FR-NEW-193 | P0 |
| E2E-NEW-502 | `git.merge feature` on SEED-CONFLICT returns `status:"conflict"` | Happy | SC-904 | FR-NEW-170, FR-NEW-186, FR-NEW-199 | P0 |
| E2E-NEW-503 | `merge_resolve` with `{path, strategy:"ours"}` finishes the merge | Happy | SC-904 | FR-NEW-174, FR-NEW-196 | P0 |
| E2E-NEW-504 | `merge_resolve` with literal `content` finishes the merge | Happy | SC-905 | FR-NEW-175, FR-NEW-196 | P0 |
| E2E-NEW-505 | `merge_resolve` mixing strategy + content across 3 files | Happy | SC-905 | FR-NEW-176 | P0 |
| E2E-NEW-506 | `git.merge feature squash:true` | Happy | SC-906 | FR-NEW-191 | P0 |
| E2E-NEW-507 | `git.merge_abort` after a conflict | Happy | SC-904 | FR-NEW-197 | P0 |
| E2E-NEW-508 | conflicted `git.remote_pull` finished by `git.merge_resolve` | Happy | SC-912 | FR-MOD-105, FR-MOD-104, FR-NEW-174 | P0 |
| E2E-NEW-509 | `git.status` during an in-progress merge | Happy | SC-929 | FR-NEW-281, FR-MOD-107, FR-NEW-187, FR-NEW-286 | P1 |
| E2E-NEW-510 | `git.merge` of an already-fully-merged branch | Happy | SC-903 | FR-NEW-192 | P1 |
| E2E-NEW-511 | `git.merge` without `mount_id` | Failure | SC-903 | FR-NEW-100 | P0 |
| E2E-NEW-512 | `git.merge` by a non-member | Failure | SC-903 | FR-NEW-100 | P0 |
| E2E-NEW-513 | `git.merge` on unknown mount | Failure | SC-903 | FR-NEW-100 | P0 |
| E2E-NEW-514 | `git.merge` unauthenticated | Failure | SC-903 | FR-NEW-100 | P1 |
| E2E-NEW-515 | `git.merge source_ref:"nope"` | Failure | SC-903 | FR-NEW-195 | P0 |
| E2E-NEW-516 | `git.merge source_ref` = current branch | Failure | SC-903 | FR-NEW-190 | P1 |
| E2E-NEW-517 | `git.merge` with a dirty volume | Failure | SC-903 | FR-NEW-194 | P0 |
| E2E-NEW-518 | `git.merge_resolve` naming a non-conflicting path | Failure | SC-904 | FR-NEW-177 | P0 |
| E2E-NEW-519 | `git.merge_resolve` with no merge in progress | Failure | SC-904 | FR-NEW-198 | P0 |
| E2E-NEW-520 | `git.merge_abort` with no merge in progress | Failure | SC-904 | FR-NEW-198, FR-NEW-186, FR-NEW-199 | P0 |
| E2E-NEW-521 | `git.merge_resolve` with both `strategy` and `content` for one path | Failure | SC-905 | FR-NEW-176, FR-NEW-179 | P0 |
| E2E-NEW-522 | `git.merge_resolve` with neither field | Failure | SC-905 | FR-NEW-179 | P0 |
| E2E-NEW-523 | `git.merge_resolve strategy:"Ours"` (wrong case) | Failure | SC-904 | FR-NEW-179 | P1 |
| E2E-NEW-524 | `git.merge_resolve strategy:"union"` | Failure | SC-904 | FR-NEW-179 | P1 |
| E2E-NEW-525 | `git.merge_resolve resolutions: []` | Failure | SC-904 | FR-NEW-179 | P1 |
| E2E-NEW-526 | `git.merge_resolve` path `../../etc/passwd` | Failure | SC-904 | FR-NEW-100 | P0 |
| E2E-NEW-527 | `git.merge_resolve` same path twice in one call | Failure | SC-904 | FR-NEW-177 | P1 |
| E2E-NEW-528 | `git.commit` while a merge is in progress | Failure | SC-929 | FR-NEW-279 | P0 |
| E2E-NEW-529 | `git.branch_switch` while in progress | Failure | SC-929 | FR-NEW-279 | P0 |
| E2E-NEW-530 | second `git.merge` while in progress | Failure | SC-929 | FR-NEW-279, FR-NEW-283 | P0 |
| E2E-NEW-531 | `git.remote_pull` while a merge is in progress | Failure | SC-929 | FR-NEW-279 | P0 |
| E2E-NEW-532 | `git.rebase` while in progress | Failure | SC-929 | FR-NEW-279 | P1 |
| E2E-NEW-533 | `git.cherry_pick` while in progress | Failure | SC-929 | FR-NEW-279 | P1 |
| E2E-NEW-534 | `git.reset` while in progress | Failure | SC-929 | FR-NEW-279 | P1 |
| E2E-NEW-535 | `git.remote_pull` still accepting removed `on_conflict` | Failure | SC-912 | FR-DEL-101, FR-MOD-104 | P0 |
| E2E-NEW-536 | `git.merge` exceeding the write quota | Failure | SC-903 | FR-NEW-185 | P0 |
| E2E-NEW-537 | resolution content exceeding quota | Failure | SC-904 | FR-NEW-185 | P1 |
| E2E-NEW-538 | `git.merge` on a volume with no HEAD | Failure | SC-903 | FR-NEW-190 | P1 |
| E2E-NEW-539 | `git.merge` on a non-initialized volume | Failure | SC-903 | FR-NEW-190 | P1 |
| E2E-NEW-540 | `git.stash_pop` conflict then resolve with a bad path | Failure | SC-930 | FR-NEW-126, FR-NEW-177, FR-NEW-186, FR-NEW-285, FR-NEW-286 | P1 |
| E2E-NEW-541 | `git.merge squash:"yes"` (string, not bool) | Failure | SC-906 | FR-NEW-191, FR-NEW-285 | P2 |
| E2E-NEW-542 | Merge commit has exactly 2 parents | SideEffect | SC-903 | FR-NEW-190 | P0 |
| E2E-NEW-543 | Squash commit has exactly 1 parent | SideEffect | SC-906 | FR-NEW-191 | P0 |
| E2E-NEW-544 | Squash: feature's individual commits absent from target log | SideEffect | SC-906 | FR-NEW-191 | P0 |
| E2E-NEW-545 | `refs/heads/main` moves to the merge commit | SideEffect | SC-903 | FR-NEW-190 | P0 |
| E2E-NEW-546 | `refs/heads/feature` unchanged by the merge | SideEffect | SC-903 | FR-NEW-190 | P1 |
| E2E-NEW-547 | Exactly one audit entry `git.merge` per completed merge | SideEffect | SC-903 | FR-NEW-190 | P0 |
| E2E-NEW-548 | Conflicted merge records an audit entry with outcome `conflict` | SideEffect | SC-904 | FR-NEW-170 | P0 |
| E2E-NEW-549 | Write quota charged exactly the merged bytes | SideEffect | SC-903 | FR-NEW-185 | P0 |
| E2E-NEW-550 | Conflicted merge charges 0 bytes | SideEffect | SC-904 | FR-NEW-171, FR-NEW-185 | P0 |
| E2E-NEW-551 | `git_operations` row created on conflict | SideEffect | SC-904 | FR-NEW-275, FR-MOD-108, FR-NEW-188, FR-NEW-285 | P0 |
| E2E-NEW-552 | `git_operations` row removed on successful resolve | SideEffect | SC-904 | FR-NEW-284, FR-NEW-188 | P0 |
| E2E-NEW-553 | `git_operations` row removed on abort | SideEffect | SC-904 | FR-NEW-284, FR-NEW-186 | P0 |
| E2E-NEW-554 | `git_operations` row scoped by `volume_id` | SideEffect | SC-929 | FR-NEW-275, FR-NEW-277, FR-MOD-108 | P0 |
| E2E-NEW-555 | Partial resolve leaves the row with a shrunk conflict set | SideEffect | SC-904 | FR-NEW-178 | P0 |
| E2E-NEW-556 | No commit object created on a conflicted merge | SideEffect | SC-904 | FR-NEW-171 | P0 |
| E2E-NEW-557 | No conflict marker in any volume file after a conflicted merge | DataIntegrity | SC-904 | FR-NEW-172 | P0 |
| E2E-NEW-558 | No conflict marker after a conflicted pull | DataIntegrity | SC-912 | FR-NEW-172, FR-MOD-104 | P0 |
| E2E-NEW-559 | No conflict marker after a conflicted rebase/cherry_pick/stash_pop | DataIntegrity | SC-919 | FR-NEW-172 | P0 |
| E2E-NEW-560 | Volume byte-identical before/after a conflicted merge | DataIntegrity | SC-904 | FR-NEW-171, FR-NEW-186 | P0 |
| E2E-NEW-561 | Volume byte-identical after a mid-apply failure | DataIntegrity | SC-904 | FR-NEW-184 | P0 |
| E2E-NEW-562 | Volume + HEAD byte/sha-identical after abort | DataIntegrity | SC-904 | FR-NEW-197, FR-NEW-285 | P0 |
| E2E-NEW-563 | Both sides deleted the same file | EdgeCase | SC-904 | FR-NEW-181 | P0 |
| E2E-NEW-564 | Delete/modify | EdgeCase | SC-904 | FR-NEW-180 | P0 |
| E2E-NEW-565 | Binary file conflict | EdgeCase | SC-904 | FR-NEW-182 | P0 |
| E2E-NEW-566 | File/directory type change | EdgeCase | SC-904 | FR-NEW-183 | P0 |
| E2E-NEW-567 | Unicode content conflict | EdgeCase | SC-905 | FR-NEW-175 | P0 |
| E2E-NEW-568 | Empty file conflict | EdgeCase | SC-904 | FR-NEW-170 | P1 |
| E2E-NEW-569 | 250 conflicting files | EdgeCase | SC-904 | FR-NEW-170, FR-NEW-275 | P1 |
| E2E-NEW-570 | Resolution path collides with an existing directory | EdgeCase | SC-905 | FR-NEW-183, FR-NEW-175 | P0 |
| E2E-NEW-571 | Resolved content is empty string | EdgeCase | SC-905 | FR-NEW-175 | P1 |
| E2E-NEW-572 | Resolution content with CRLF and trailing-newline-less end | EdgeCase | SC-905 | FR-NEW-175 | P1 |
| E2E-NEW-573 | No common ancestor (unrelated histories) | EdgeCase | SC-904 | FR-NEW-170 | P1 |
| E2E-NEW-574 | Conflict on a path 250 chars long | EdgeCase | SC-904 | FR-NEW-170 | P2 |
| E2E-NEW-575 | `git.merge` after `git.merge_abort` re-runs cleanly | EdgeCase | SC-904 | FR-NEW-197 | P1 |
| E2E-NEW-576 | Resolve all files across two sequential partial calls | EdgeCase | SC-904 | FR-NEW-178 | P0 |
| E2E-NEW-577 | Two concurrent `git.merge` on the same volume | Concurrency | SC-929 | FR-NEW-283, FR-NEW-286 | P0 |
| E2E-NEW-578 | `git.merge_resolve` on volume A unaffected by in-progress op on volume B | Concurrency | SC-929 | FR-NEW-275, FR-NEW-277 | P0 |
| E2E-NEW-579 | Read-only tools (`git.log`, `fs.read_text`) allowed while in progress | Concurrency | SC-929 | FR-NEW-280 | P0 |
| E2E-NEW-580 | `git.merge_abort` by a different member of the same project | Concurrency | SC-929 | FR-NEW-282 | P1 |
| E2E-NEW-600 | Pick 2 commits onto `main` | Happy | SC-914 | FR-NEW-210, FR-NEW-211 | P0 |
| E2E-NEW-601 | Squash F2 into F1 | Happy | SC-915 | FR-NEW-212, FR-NEW-199 | P0 |
| E2E-NEW-602 | Drop F2 | Happy | SC-915 | FR-NEW-213 | P0 |
| E2E-NEW-603 | Reword F1 | Happy | SC-915 | FR-NEW-214 | P0 |
| E2E-NEW-604 | Squash chain of 3 | Happy | SC-915 | FR-NEW-212 | P0 |
| E2E-NEW-605 | All commits dropped | EdgeCase | SC-915 | FR-NEW-213 | P1 |
| E2E-NEW-606 | Single-commit rebase | Happy | SC-914 | FR-NEW-210, FR-NEW-211 | P1 |
| E2E-NEW-607 | Rebase onto an ancestor -> no-op | EdgeCase | SC-914 | FR-NEW-217 | P0 |
| E2E-NEW-608 | branch == onto -> no-op | EdgeCase | SC-914 | FR-NEW-217 | P1 |
| E2E-NEW-609 | Original commits unreachable but objects retained | SideEffect | SC-914 | FR-NEW-210, FR-NEW-255 | P0 |
| E2E-NEW-610 | Audit entries for a 2-pick rebase | SideEffect | SC-914 | FR-NEW-210 | P1 |
| E2E-NEW-611 | Quota charged for volume rewrite | SideEffect | SC-914 | FR-NEW-210 | P1 |
| E2E-NEW-612 | `git_operations` row absent after clean rebase | SideEffect | SC-914 | FR-NEW-284 | P0 |
| E2E-NEW-613 | Single pause on step 1 | Failure | SC-916 | FR-NEW-219, FR-NEW-186, FR-NEW-187, FR-NEW-188, FR-NEW-285 | P0 |
| E2E-NEW-614 | `rebase_continue` with `theirs` completes | Happy | SC-916 | FR-NEW-221, FR-NEW-174 | P0 |
| E2E-NEW-615 | Multi-pause: steps 2 and 4 conflict | Happy | SC-916 | FR-NEW-220 | P0 |
| E2E-NEW-616 | Conflict leaves volume untouched | DataIntegrity | SC-916 | FR-NEW-171, FR-NEW-187 | P0 |
| E2E-NEW-617 | Resolution by literal content | Happy | SC-916 | FR-NEW-175 | P1 |
| E2E-NEW-618 | No conflict markers ever in the volume | DataIntegrity | SC-916 | FR-NEW-172 | P0 |
| E2E-NEW-619 | Abort restores tip + every byte | Happy | SC-917 | FR-NEW-223 | P0 |
| E2E-NEW-620 | Abort clears `git_operations` row | SideEffect | SC-917 | FR-NEW-284 | P0 |
| E2E-NEW-621 | Abort mid-multi-pause (after 1 continue) | EdgeCase | SC-917 | FR-NEW-223 | P0 |
| E2E-NEW-622 | `todo` omits a commit in range | Failure | SC-914 | FR-NEW-215 | P0 |
| E2E-NEW-623 | `todo` references unknown sha | Failure | SC-914 | FR-NEW-215 | P0 |
| E2E-NEW-624 | `todo` references sha outside range | Failure | SC-914 | FR-NEW-215 | P0 |
| E2E-NEW-625 | `todo` starts with `squash` | Failure | SC-914 | FR-NEW-215, FR-NEW-212 | P0 |
| E2E-NEW-626 | `todo` longer than the bound | Failure | SC-914 | FR-NEW-216 | P1 |
| E2E-NEW-627 | `todo` empty array | Failure | SC-914 | FR-NEW-215 | P1 |
| E2E-NEW-628 | Unknown `action` value | Failure | SC-914 | FR-NEW-215 | P0 |
| E2E-NEW-629 | `reword` with empty message | Failure | SC-915 | FR-NEW-214 | P0 |
| E2E-NEW-630 | `reword` with whitespace-only message | Failure | SC-915 | FR-NEW-214 | P1 |
| E2E-NEW-631 | Unknown `onto` ref | Failure | SC-914 | FR-NEW-210 | P0 |
| E2E-NEW-632 | Missing `onto` / missing `todo` | Failure | SC-914 | FR-NEW-210 | P1 |
| E2E-NEW-633 | Duplicate sha in `todo` | Failure | SC-914 | FR-NEW-215 | P1 |
| E2E-NEW-634 | Pre-flight rejection writes nothing at all | Failure | SC-914 | FR-NEW-215 | P0 |
| E2E-NEW-635 | `rebase_continue` with no rebase in progress | Failure | SC-916 | FR-NEW-224 | P0 |
| E2E-NEW-636 | `rebase_abort` with no rebase in progress | Failure | SC-917 | FR-NEW-224 | P0 |
| E2E-NEW-637 | `git.rebase` while a rebase is already in progress | Failure | SC-929 | FR-NEW-283, FR-NEW-225 | P0 |
| E2E-NEW-638 | `git.commit` while a rebase is paused | Failure | SC-929 | FR-NEW-279 | P1 |
| E2E-NEW-639 | Non-member caller | Security | SC-914 | FR-NEW-100 | P0 |
| E2E-NEW-640 | Pick F2 onto `main` | Happy | SC-918 | FR-NEW-235, FR-NEW-199 | P0 |
| E2E-NEW-641 | Diff applied, unrelated files untouched | Happy | SC-918 | FR-NEW-235 | P0 |
| E2E-NEW-642 | Two sequential cherry-picks | Happy | SC-918 | FR-NEW-235 | P1 |
| E2E-NEW-643 | Audit + quota for a cherry-pick | SideEffect | SC-918 | FR-NEW-235 | P1 |
| E2E-NEW-644 | Source branch untouched | SideEffect | SC-918 | FR-NEW-235 | P0 |
| E2E-NEW-645 | Conflict -> shared model, nothing applied | Failure | SC-919 | FR-NEW-238, FR-NEW-171, FR-NEW-186, FR-NEW-187, FR-NEW-188 | P0 |
| E2E-NEW-646 | `cherry_pick_continue` with `ours` | Happy | SC-919 | FR-NEW-239, FR-NEW-174 | P0 |
| E2E-NEW-647 | `cherry_pick_abort` restores tip + bytes | Happy | SC-919 | FR-NEW-239 | P0 |
| E2E-NEW-648 | Already in current history -> reported, not duplicated | EdgeCase | SC-918 | FR-NEW-236 | P0 |
| E2E-NEW-649 | Equivalent-content commit already applied (different sha) | EdgeCase | SC-918 | FR-NEW-236 | P1 |
| E2E-NEW-650 | Merge commit | EdgeCase | SC-918 | FR-NEW-240 | P0 |
| E2E-NEW-651 | Empty-diff commit | EdgeCase | SC-918 | FR-NEW-235 | P1 |
| E2E-NEW-652 | Unknown sha -> ERR_NOT_FOUND | Failure | SC-918 | FR-NEW-237 | P0 |
| E2E-NEW-653 | Malformed sha -> ERR_INVALID_ARGUMENT | Failure | SC-918 | FR-NEW-237 | P0 |
| E2E-NEW-654 | Missing `commit_sha` | Failure | SC-918 | FR-NEW-235 | P1 |
| E2E-NEW-655 | `continue` with no pick in progress | Failure | SC-919 | FR-NEW-239, FR-NEW-224, FR-NEW-225 | P0 |
| E2E-NEW-656 | `abort` with no pick in progress | Failure | SC-919 | FR-NEW-239, FR-NEW-224, FR-NEW-225 | P0 |
| E2E-NEW-657 | Pick on an empty repo (no HEAD) | Failure | SC-918 | FR-NEW-235 | P1 |
| E2E-NEW-658 | Quota exhausted mid-pick | Failure | SC-918 | FR-NEW-185 | P0 |
| E2E-NEW-659 | Non-member caller | Security | SC-918 | FR-NEW-100 | P0 |
| E2E-NEW-660 | `soft` to C2 | Happy | SC-920 | FR-NEW-250, FR-NEW-199 | P0 |
| E2E-NEW-661 | `hard` to C2 | Happy | SC-921 | FR-NEW-251 | P0 |
| E2E-NEW-662 | `hard` deletes files added after target | Happy | SC-921 | FR-NEW-251 | P0 |
| E2E-NEW-663 | `hard` on a dirty volume discards changes | EdgeCase | SC-921 | FR-NEW-251 | P0 |
| E2E-NEW-664 | `soft` on a dirty volume keeps dirt | EdgeCase | SC-920 | FR-NEW-250 | P0 |
| E2E-NEW-665 | Reset to current tip -> no-op both modes | EdgeCase | SC-921 | FR-NEW-254 | P0 |
| E2E-NEW-666 | Orphaned commits still exist as objects | EdgeCase | SC-921 | FR-NEW-255 | P0 |
| E2E-NEW-667 | `hard` to a branch name and to a tag | EdgeCase | SC-921 | FR-NEW-251 | P1 |
| E2E-NEW-668 | Audit + quota for `hard` | SideEffect | SC-921 | FR-NEW-251 | P1 |
| E2E-NEW-669 | Unknown `target_ref` -> ERR_NOT_FOUND | Failure | SC-921 | FR-NEW-253 | P0 |
| E2E-NEW-670 | Missing `mode` -> ERR_INVALID_ARGUMENT | Failure | SC-921 | FR-NEW-252 | P0 |
| E2E-NEW-671 | `mode="mixed"` -> ERR_INVALID_ARGUMENT naming soft/hard | Failure | SC-920 | FR-NEW-252 | P0 |
| E2E-NEW-672 | `mode="HARD"` (case) rejected | Failure | SC-921 | FR-NEW-252 | P1 |
| E2E-NEW-673 | `hard` fails quota -> nothing moved | Failure | SC-921 | FR-NEW-251, FR-NEW-185 | P0 |
| E2E-NEW-674 | Non-member caller | Security | SC-920 | FR-NEW-100 | P0 |
| E2E-NEW-675 | Revert C3 | Happy | SC-922 | FR-NEW-260 | P0 |
| E2E-NEW-676 | Revert a file-add commit | Happy | SC-922 | FR-NEW-260 | P0 |
| E2E-NEW-677 | Revert of a revert reapplies | Happy | SC-922 | FR-NEW-265 | P0 |
| E2E-NEW-678 | Merge revert with `mainline=1` | Happy | SC-923 | FR-NEW-261 | P0 |
| E2E-NEW-679 | Merge revert with `mainline=2` | EdgeCase | SC-923 | FR-NEW-261, FR-NEW-262 | P0 |
| E2E-NEW-680 | Revert of the initial commit (no parent) | EdgeCase | SC-922 | FR-NEW-263, FR-NEW-199 | P0 |
| E2E-NEW-681 | Revert a non-tip commit (C2 while tip is C4) | EdgeCase | SC-922 | FR-NEW-260 | P1 |
| E2E-NEW-682 | Audit + quota + original reachable | SideEffect | SC-922 | FR-NEW-260 | P1 |
| E2E-NEW-683 | Inverse diff does not apply -> shared model | Failure | SC-922 | FR-NEW-264 | P0 |
| E2E-NEW-684 | Continue resolves; abort restores exactly | Happy | SC-922 | FR-NEW-264, FR-NEW-284, FR-NEW-186 | P0 |
| E2E-NEW-685 | Merge commit without `mainline` | Failure | SC-923 | FR-NEW-261 | P0 |
| E2E-NEW-686 | `mainline=3` on a 2-parent merge | Failure | SC-923 | FR-NEW-262 | P0 |
| E2E-NEW-687 | `mainline=0` | Failure | SC-923 | FR-NEW-262 | P0 |
| E2E-NEW-688 | `mainline` on a non-merge commit | Failure | SC-923 | FR-NEW-262 | P1 |
| E2E-NEW-689 | Unknown sha -> ERR_NOT_FOUND | Failure | SC-922 | FR-NEW-260 | P0 |
| E2E-NEW-690 | All 8 new tools require `mount_id` | Failure | SC-929 | FR-NEW-100, FR-NEW-345, FR-NEW-347 | P0 |
| E2E-NEW-691 | Unknown `mount_id` -> ERR_PROJECT_NOT_FOUND on all 8 | Failure | SC-929 | FR-NEW-100, FR-NEW-347 | P0 |
| E2E-NEW-692 | All 31 new tools registered, git registry count is 45 | Integration | SC-929 | FR-NEW-345, FR-NEW-346, FR-NEW-349 | P0 |
| E2E-NEW-693 | Tool contract golden includes every new schema | Integration | SC-929 | FR-NEW-345, FR-NEW-349 | P0 |
| E2E-NEW-694 | Write lock serializes concurrent rebase + commit | Concurrency | SC-916 | FR-NEW-226 | P0 |
| E2E-NEW-695 | `git_operations` rows are `volume_id`-scoped | DataIntegrity | SC-929 | FR-NEW-275, FR-NEW-277, FR-MOD-108 | P0 |
| E2E-NEW-696 | Unauthenticated caller -> ERR_UNAUTHENTICATED | Security | SC-929 | FR-NEW-100, FR-NEW-347 | P0 |
| E2E-NEW-697 | Paused rebase survives a repo-store reopen | DataIntegrity | SC-916 | FR-NEW-278 | P1 |
| E2E-NEW-698 | Rebase and cherry-pick cannot both be in progress | Failure | SC-929 | FR-NEW-283 | P1 |
| E2E-NEW-699 | Every new mutating op leaves refs consistent db vs on-disk | DataIntegrity | SC-929 | FR-NEW-115, FR-NEW-226 | P0 |
| E2E-NEW-700 | `pr_list` on `acme-api` hits base `https://api.github.com` | Happy | SC-925 | FR-NEW-300, FR-NEW-199 | P0 |
| E2E-NEW-701 | `pr_list` on `acme-ent` hits `https://github.ibm.com/api/v3` | Happy | SC-925 | FR-NEW-300 | P0 |
| E2E-NEW-702 | `pr_list` on `acme-lab` hits `https://gitlab.com/api/v4` | Happy | SC-925 | FR-NEW-300 | P0 |
| E2E-NEW-703 | `pr_list` on `acme-self` hits `https://gitlab.example.test/api/v4` | Happy | SC-925 | FR-NEW-300 | P0 |
| E2E-NEW-704 | `pr_list` on `acme-generic` | Failure | SC-925 | FR-NEW-301 | P0 |
| E2E-NEW-705 | `pr_list` on `acme-pub` | Failure | SC-925 | FR-NEW-301 | P0 |
| E2E-NEW-706 | mount whose origin is `https://git.sourcehut.test/acme/api.git` | Failure | SC-925 | FR-NEW-300 | P0 |
| E2E-NEW-707 | `pr_list` on `acme-nor` | Failure | SC-925 | FR-NEW-302 | P0 |
| E2E-NEW-708 | origin `git@github.com:acme/api.git` (scp shorthand) | Failure | SC-925 | FR-NEW-302 | P1 |
| E2E-NEW-709 | GitHub PR 42 vs GitLab MR 42, same logical state | Integration | SC-927 | FR-NEW-303, FR-NEW-317, FR-NEW-199 | P0 |
| E2E-NEW-710 | `pr_create` on github | Happy | SC-925 | FR-NEW-304 | P0 |
| E2E-NEW-711 | `pr_create` on gitlab | Happy | SC-925 | FR-NEW-304 | P0 |
| E2E-NEW-712 | `pr_create` on GHES | Happy | SC-925 | FR-NEW-304, FR-NEW-300 | P0 |
| E2E-NEW-713 | `pr_create` self-hosted gitlab, unicode title/body | Happy | SC-925 | FR-NEW-304 | P1 |
| E2E-NEW-714 | github head branch `feature/ghost` missing | Failure | SC-925 | FR-NEW-305 | P0 |
| E2E-NEW-715 | gitlab head branch missing | Failure | SC-925 | FR-NEW-305 | P0 |
| E2E-NEW-716 | `base == head == "main"` | Failure | SC-925 | FR-NEW-304 | P1 |
| E2E-NEW-717 | empty title | Failure | SC-925 | FR-NEW-304 | P1 |
| E2E-NEW-718 | github 422 "A pull request already exists" | Failure | SC-925 | FR-NEW-311 | P0 |
| E2E-NEW-719 | gitlab 409 "Another open merge request already exists" | Failure | SC-925 | FR-NEW-311 | P0 |
| E2E-NEW-720 | github 403 insufficient permission on create | Failure | SC-925 | FR-NEW-311 | P0 |
| E2E-NEW-721 | draft PR create both providers | EdgeCase | SC-925 | FR-NEW-304, FR-NEW-303 | P1 |
| E2E-NEW-722 | audit entry after successful create | SideEffect | SC-925 | FR-NEW-314 | P0 |
| E2E-NEW-723 | `pr_list state=open` github | Happy | SC-926 | FR-NEW-306, FR-NEW-199 | P0 |
| E2E-NEW-724 | `pr_list` gitlab for all four states | Happy | SC-926 | FR-NEW-306 | P0 |
| E2E-NEW-725 | empty list both providers | EdgeCase | SC-926 | FR-NEW-306 | P1 |
| E2E-NEW-726 | `state="draft"` | Failure | SC-926 | FR-NEW-306 | P1 |
| E2E-NEW-727 | github paginated list (2 pages, Link header) | EdgeCase | SC-926 | FR-NEW-306 | P1 |
| E2E-NEW-728 | github 404 repo not found | Failure | SC-926 | FR-NEW-313 | P0 |
| E2E-NEW-729 | gitlab 500 | Failure | SC-926 | FR-NEW-311 | P1 |
| E2E-NEW-730 | GHES list | EdgeCase | SC-926 | FR-NEW-300, FR-NEW-306 | P1 |
| E2E-NEW-731 | self-hosted gitlab list | EdgeCase | SC-926 | FR-NEW-300, FR-NEW-306 | P1 |
| E2E-NEW-732 | github `pr_get` 42 | Happy | SC-927 | FR-NEW-307, FR-NEW-317 | P0 |
| E2E-NEW-733 | gitlab `pr_get` 42 | Happy | SC-927 | FR-NEW-307, FR-NEW-317 | P0 |
| E2E-NEW-734 | PR with zero changed files | EdgeCase | SC-927 | FR-NEW-303, FR-NEW-307 | P1 |
| E2E-NEW-735 | already-merged PR | EdgeCase | SC-927 | FR-NEW-303 | P0 |
| E2E-NEW-736 | closed-unmerged PR | EdgeCase | SC-927 | FR-NEW-303 | P0 |
| E2E-NEW-737 | draft PR get | EdgeCase | SC-927 | FR-NEW-303 | P1 |
| E2E-NEW-738 | github PR 9999 | Failure | SC-927 | FR-NEW-313 | P0 |
| E2E-NEW-739 | gitlab MR 9999 | Failure | SC-927 | FR-NEW-313 | P0 |
| E2E-NEW-740 | `pr_number = 0` and `-1` | Failure | SC-927 | FR-NEW-313 | P1 |
| E2E-NEW-741 | github check-runs sub-call 403 | Failure | SC-927 | FR-NEW-308 | P0 |
| E2E-NEW-742 | github `pr_diff` | Happy | SC-927 | FR-NEW-309, FR-NEW-199 | P0 |
| E2E-NEW-743 | gitlab `pr_diff` | Happy | SC-927 | FR-NEW-309 | P0 |
| E2E-NEW-744 | diff with zero changed files | EdgeCase | SC-927 | FR-NEW-309 | P1 |
| E2E-NEW-745 | 12 MiB diff | EdgeCase | SC-927 | FR-NEW-309 | P0 |
| E2E-NEW-746 | unicode + CRLF in diff | EdgeCase | SC-927 | FR-NEW-309 | P1 |
| E2E-NEW-747 | `pr_diff` 404 | Failure | SC-927 | FR-NEW-313, FR-NEW-309 | P1 |
| E2E-NEW-748 | github returns HTML instead of a diff | Failure | SC-927 | FR-NEW-309 | P1 |
| E2E-NEW-749 | binary file in diff | EdgeCase | SC-927 | FR-NEW-309 | P2 |
| E2E-NEW-750 | github merge `merge` | Happy | SC-928 | FR-NEW-310, FR-NEW-317 | P0 |
| E2E-NEW-751 | github merge `squash` | Happy | SC-928 | FR-NEW-310 | P0 |
| E2E-NEW-752 | github merge `rebase` | Happy | SC-928 | FR-NEW-310 | P0 |
| E2E-NEW-753 | gitlab merge `merge` | Happy | SC-928 | FR-NEW-310 | P0 |
| E2E-NEW-754 | gitlab merge `squash` | Happy | SC-928 | FR-NEW-310 | P0 |
| E2E-NEW-755 | gitlab merge `rebase` | SideEffect | SC-928 | FR-NEW-310 | P0 |
| E2E-NEW-756 | strategy `fast-forward` | Failure | SC-928 | FR-NEW-310 | P1 |
| E2E-NEW-757 | github 405 merge commits not allowed | Failure | SC-928 | FR-NEW-311 | P0 |
| E2E-NEW-758 | github 405 required check expected | Failure | SC-928 | FR-NEW-311 | P0 |
| E2E-NEW-759 | gitlab 405 pipeline must succeed | Failure | SC-928 | FR-NEW-311 | P0 |
| E2E-NEW-760 | github 409 head sha mismatch | Failure | SC-928 | FR-NEW-311 | P1 |
| E2E-NEW-761 | github 403 protected branch | Failure | SC-928 | FR-NEW-311 | P0 |
| E2E-NEW-762 | merging an already-merged PR | EdgeCase | SC-928 | FR-NEW-311 | P0 |
| E2E-NEW-763 | github review approve | Happy | SC-927 | FR-NEW-312, FR-NEW-317 | P0 |
| E2E-NEW-764 | github review request_changes + body | Happy | SC-927 | FR-NEW-312 | P0 |
| E2E-NEW-765 | github review comment | Happy | SC-927 | FR-NEW-312 | P0 |
| E2E-NEW-766 | gitlab review approve | Happy | SC-927 | FR-NEW-312 | P0 |
| E2E-NEW-767 | gitlab request_changes | SideEffect | SC-927 | FR-NEW-312 | P0 |
| E2E-NEW-768 | gitlab review comment | Happy | SC-927 | FR-NEW-312 | P0 |
| E2E-NEW-769 | verdict `lgtm` | Failure | SC-927 | FR-NEW-312 | P1 |
| E2E-NEW-770 | request_changes with no body | Failure | SC-927 | FR-NEW-312 | P1 |
| E2E-NEW-771 | github self-approval 422 | Failure | SC-927 | FR-NEW-311, FR-NEW-312 | P1 |
| E2E-NEW-772 | gitlab self-approval 401 | Failure | SC-927 | FR-NEW-311, FR-NEW-312 | P0 |
| E2E-NEW-773 | gitlab device flow scope string | Happy | SC-925 | FR-NEW-330, FR-MOD-109 | P0 |
| E2E-NEW-774 | github device flow scope string | Happy | SC-925 | FR-NEW-330, FR-NEW-331 | P0 |
| E2E-NEW-775 | granted scopes persisted and reported | Happy | SC-925 | FR-NEW-335, FR-NEW-330 | P0 |
| E2E-NEW-776 | gitlab token without `api`, `pr_list` | Failure | SC-926 | FR-NEW-332, FR-NEW-333 | P0 |
| E2E-NEW-777 | gitlab token without `api`, `pr_create` | Failure | SC-925 | FR-NEW-332, FR-NEW-333 | P0 |
| E2E-NEW-778 | gitlab token with `read_api`, `pr_get` | Happy | SC-927 | FR-NEW-332 | P0 |
| E2E-NEW-779 | gitlab `read_api` only, `pr_merge` | Failure | SC-928 | FR-NEW-332 | P0 |
| E2E-NEW-780 | github token `["public_repo"]`, `pr_merge` | Failure | SC-928 | FR-NEW-332 | P0 |
| E2E-NEW-781 | `git.token_set` token (empty scopes), any PR tool | Failure | SC-925 | FR-NEW-334 | P0 |
| E2E-NEW-782 | token with scopes `["everything"]` | EdgeCase | SC-925 | FR-NEW-334 | P1 |
| E2E-NEW-783 | `git.auth_status` PR coverage flags | Happy | SC-925 | FR-NEW-335 | P0 |
| E2E-NEW-784 | gitlab token `["api"]` only | EdgeCase | SC-927 | FR-NEW-332 | P1 |
| E2E-NEW-785 | expired token, correct scopes | Failure | SC-925 | FR-NEW-332 | P0 |
| E2E-NEW-786 | provider 401 body echoes the token | Security | SC-925 | FR-NEW-314 | P0 |
| E2E-NEW-787 | tracing span capture across all 6 tools | Security | SC-925 | FR-NEW-314 | P0 |
| E2E-NEW-788 | audit entries across all 6 tools | Security | SC-925 | FR-NEW-314 | P0 |
| E2E-NEW-789 | `outsider@test.com` calls all 6 tools | Security | SC-925 | FR-NEW-100, FR-NEW-347 | P0 |
| E2E-NEW-790 | no identity header | Security | SC-925 | FR-NEW-100, FR-NEW-347 | P0 |
| E2E-NEW-791 | two people, same host | Security | SC-925 | FR-NEW-314 | P0 |
| E2E-NEW-792 | Authorization header shape | Security | SC-925 | FR-NEW-315 | P0 |
| E2E-NEW-793 | `raw` field content | Security | SC-927 | FR-NEW-303, FR-NEW-314 | P0 |
| E2E-NEW-794 | provider 302 to `https://evil.test` | Security | SC-925 | FR-NEW-316 | P0 |
| E2E-NEW-795 | 64 MiB response body | Security | SC-926 | FR-NEW-315, FR-NEW-316 | P1 |
| E2E-NEW-796 | github create -> get -> review -> merge | Integration | SC-928 | FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312 | P0 |
| E2E-NEW-797 | gitlab create -> get -> review -> merge | Integration | SC-928 | FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312 | P0 |
| E2E-NEW-798 | unicode title/body through get and list | EdgeCase | SC-927 | FR-NEW-303 | P1 |
| E2E-NEW-799 | frozen tool contract | EdgeCase | SC-925 | FR-NEW-345, FR-NEW-346 | P0 |

| E2E-NEW-800 | Happy — a rebase runs once the dirt is committed | Happy | SC-914 | FR-NEW-218, FR-NEW-210 | P0 |
| E2E-NEW-801 | Failure — a dirty volume refuses the rebase before any replay | Failure | SC-914 | FR-NEW-218 | P0 |
| E2E-NEW-802 | EdgeCase — dirt that is only a deletion still refuses | EdgeCase | SC-914 | FR-NEW-218 | P1 |
| E2E-NEW-803 | Happy — a second resolution call covering the rest lets the continue through | Happy | SC-916 | FR-NEW-222, FR-NEW-221, FR-NEW-178 | P0 |
| E2E-NEW-804 | Failure — continue with an empty resolutions array keeps the pause exactly | Failure | SC-916 | FR-NEW-222 | P0 |
| E2E-NEW-805 | EdgeCase — repeated partial continues never advance the step index | EdgeCase | SC-916 | FR-NEW-222, FR-NEW-178, FR-NEW-177 | P1 |
| E2E-NEW-806 | Happy — deleting a project removes its paused-operation row | Happy | SC-929 | FR-NEW-276, FR-MOD-108, FR-NEW-284 | P0 |
| E2E-NEW-807 | SideEffect — the purge is scoped to the deleted volume only | SideEffect | SC-929 | FR-NEW-276, FR-NEW-283 | P0 |
| E2E-NEW-808 | EdgeCase — on SQLite the index file carries the rows away with it | EdgeCase | SC-929 | FR-NEW-276, FR-MOD-108 | P0 |
| E2E-NEW-810 | EdgeCase — `on_conflict` is absent from the frozen schema | EdgeCase | SC-912 | FR-DEL-101, FR-NEW-345 | P0 |
| E2E-NEW-811 | Failure — `on_conflict` is rejected even on a pull that would fast-forward | Failure | SC-912 | FR-DEL-101 | P1 |
| E2E-NEW-812 | Failure — `git.rebase_continue` cannot finish a paused pull | Failure | SC-912 | FR-MOD-105, FR-NEW-225 | P0 |
| E2E-NEW-813 | SideEffect — `git.merge_abort` on a paused pull keeps the fetched objects | SideEffect | SC-912 | FR-MOD-105, FR-MOD-104, FR-NEW-197 | P0 |
| E2E-NEW-814 | EdgeCase — the key is absent when nothing is in progress | EdgeCase | SC-929 | FR-MOD-107 | P1 |
| E2E-NEW-815 | SideEffect — status names the rebase tools, not the merge tools | SideEffect | SC-929 | FR-MOD-107, FR-NEW-281 | P0 |
| E2E-NEW-816 | EdgeCase — the granted scopes are persisted verbatim from the flow | EdgeCase | SC-925 | FR-MOD-109, FR-NEW-330, FR-NEW-335 | P0 |
| E2E-NEW-817 | Failure — a token carrying only the old scope pair is refused before the network | Failure | SC-925 | FR-MOD-109, FR-NEW-332 | P0 |
| E2E-NEW-818 | EdgeCase — refs are case-sensitive, so `Main` is not a duplicate of `main` | EdgeCase | SC-901 | FR-NEW-102, FR-NEW-104 | P1 |
| E2E-NEW-819 | SideEffect — a refused duplicate writes no audit entry and charges no quota | SideEffect | SC-901 | FR-NEW-102 | P0 |
| E2E-NEW-820 | SideEffect — the no-op switch writes nothing at all | SideEffect | SC-901 | FR-NEW-107 | P0 |
| E2E-NEW-821 | Failure — the dirty check runs before the no-op shortcut | Failure | SC-901 | FR-NEW-107, FR-NEW-106 | P1 |
| E2E-NEW-822 | SideEffect — drop touches neither the volume nor the other entries | SideEffect | SC-902 | FR-NEW-127, FR-NEW-122 | P1 |
| E2E-NEW-823 | Failure — dropping the same id twice | Failure | SC-902 | FR-NEW-127, FR-NEW-128 | P1 |
| E2E-NEW-824 | Failure — the refusal names the limit and the remedy | Failure | SC-902 | FR-NEW-130 | P1 |
| E2E-NEW-825 | EdgeCase — dropping one entry at the cap re-opens a slot | EdgeCase | SC-902 | FR-NEW-130, FR-NEW-127 | P2 |
| E2E-NEW-826 | EdgeCase — remote names are case-sensitive | EdgeCase | SC-907 | FR-NEW-143, FR-NEW-141 | P1 |
| E2E-NEW-827 | SideEffect — a failed remove leaves the tracking refs intact | SideEffect | SC-907 | FR-NEW-143, FR-NEW-142 | P0 |
| E2E-NEW-828 | EdgeCase — an empty-string lease with `force:false` is treated as absent | EdgeCase | SC-909 | FR-NEW-158, FR-NEW-156, FR-NEW-132 | P1 |
| E2E-NEW-829 | SideEffect — the contradiction is caught before any network contact | SideEffect | SC-909 | FR-NEW-158 | P0 |
| E2E-NEW-830 | EdgeCase — a create-only force push audits the all-zero lease | EdgeCase | SC-909 | FR-NEW-159, FR-NEW-155 | P1 |
| E2E-NEW-831 | Failure — a lease-rejected force push writes no audit entry | Failure | SC-910 | FR-NEW-159, FR-NEW-157 | P0 |
| E2E-NEW-832 | EdgeCase — a fast-forward push with `force:false` still succeeds | EdgeCase | SC-908 | FR-NEW-160 | P0 |
| E2E-NEW-833 | SideEffect — the refused non-fast-forward leaves the remote byte-identical | SideEffect | SC-908 | FR-NEW-160 | P0 |
| E2E-NEW-834 | EdgeCase — a rebase step whose sides are disjoint replays without pausing | EdgeCase | SC-914 | FR-NEW-173, FR-NEW-211 | P0 |
| E2E-NEW-835 | SideEffect — an automatic merge records nothing to resolve | SideEffect | SC-903 | FR-NEW-173, FR-NEW-284 | P0 |
| E2E-NEW-836 | Happy — choosing the modifying side keeps the file with its exact bytes | Happy | SC-904 | FR-NEW-180, FR-NEW-174 | P0 |
| E2E-NEW-837 | Failure — an invented strategy for the deleted side is refused | Failure | SC-904 | FR-NEW-180, FR-NEW-179 | P0 |
| E2E-NEW-838 | SideEffect — the both-deleted path is absent from the conflict set and from the volume | SideEffect | SC-904 | FR-NEW-181, FR-NEW-171 | P0 |
| E2E-NEW-839 | EdgeCase — both sides delete an entire directory | EdgeCase | SC-904 | FR-NEW-181 | P2 |
| E2E-NEW-840 | Failure — literal content for a binary path is refused | Failure | SC-904 | FR-NEW-182, FR-NEW-175 | P0 |
| E2E-NEW-841 | SideEffect — the binary entry omits the bytes but reports the sizes | SideEffect | SC-904 | FR-NEW-182, FR-NEW-170 | P0 |
| E2E-NEW-842 | SideEffect — the no-op merge charges no quota and writes one audit line at most | SideEffect | SC-903 | FR-NEW-192 | P1 |
| E2E-NEW-843 | EdgeCase — merging a raw sha that is already an ancestor | EdgeCase | SC-903 | FR-NEW-192, FR-NEW-195 | P1 |
| E2E-NEW-844 | EdgeCase — `squash:true` on a fast-forwardable merge commits instead of fast-forwarding | EdgeCase | SC-906 | FR-NEW-193, FR-NEW-191 | P1 |
| E2E-NEW-845 | SideEffect — the fast-forward creates no new object at all | SideEffect | SC-903 | FR-NEW-193 | P0 |
| E2E-NEW-846 | EdgeCase — dirt consisting only of a new untracked file still refuses | EdgeCase | SC-903 | FR-NEW-194 | P0 |
| E2E-NEW-847 | SideEffect — the dirty refusal records no operation and no fetch-like side effect | SideEffect | SC-903 | FR-NEW-194, FR-NEW-171 | P0 |
| E2E-NEW-848 | EdgeCase — a well-formed but absent 40-hex sha | EdgeCase | SC-903 | FR-NEW-195 | P1 |
| E2E-NEW-849 | SideEffect — the unknown source is rejected before the dirty check writes anything | SideEffect | SC-903 | FR-NEW-195, FR-NEW-171 | P1 |
| E2E-NEW-850 | EdgeCase — exactly `max_rebase_todo` entries are accepted | EdgeCase | SC-914 | FR-NEW-216, FR-NEW-210 | P2 |
| E2E-NEW-851 | Failure — a lowered `git.max_rebase_todo` is honoured and named | Failure | SC-914 | FR-NEW-216, FR-NEW-331 | P1 |
| E2E-NEW-852 | SideEffect — the paused row holds every field the spec names | SideEffect | SC-916 | FR-NEW-219, FR-NEW-275, FR-NEW-187, FR-NEW-286 | P0 |
| E2E-NEW-853 | EdgeCase — a conflict on the last todo entry pauses with `current_step == total_steps - 1` | EdgeCase | SC-916 | FR-NEW-219, FR-NEW-220 | P1 |
| E2E-NEW-854 | SideEffect — the step index moves forward between pauses and never backwards | SideEffect | SC-916 | FR-NEW-220, FR-NEW-275 | P0 |
| E2E-NEW-855 | EdgeCase — three pauses in one rebase | EdgeCase | SC-916 | FR-NEW-220, FR-NEW-221 | P1 |
| E2E-NEW-856 | EdgeCase — continue with literal content rather than a side | EdgeCase | SC-916 | FR-NEW-221, FR-NEW-175 | P0 |
| E2E-NEW-857 | Failure — continue while a merge, not a rebase, is in progress | Failure | SC-916 | FR-NEW-221, FR-NEW-225, FR-NEW-224 | P0 |
| E2E-NEW-858 | SideEffect — the cherry-pick row is typed and single-stepped | SideEffect | SC-919 | FR-NEW-238, FR-NEW-275 | P0 |
| E2E-NEW-859 | EdgeCase — a cherry-pick delete/modify conflict surfaces a null side | EdgeCase | SC-919 | FR-NEW-238, FR-NEW-180 | P1 |
| E2E-NEW-860 | Failure — an out-of-range mainline names the actual parent count | Failure | SC-918 | FR-NEW-240, FR-NEW-262 | P1 |
| E2E-NEW-861 | Happy — `mainline:1` picks the merge's change relative to the first parent | Happy | SC-918 | FR-NEW-240, FR-NEW-235 | P1 |
| E2E-NEW-862 | SideEffect — the refused hard reset leaves the dirty volume alone | SideEffect | SC-921 | FR-NEW-253, FR-NEW-251 | P0 |
| E2E-NEW-863 | EdgeCase — a branch name that existed and was deleted | EdgeCase | SC-921 | FR-NEW-253 | P1 |
| E2E-NEW-864 | SideEffect — the no-op reset reports equal shas and charges nothing | SideEffect | SC-921 | FR-NEW-254, FR-NEW-250 | P1 |
| E2E-NEW-865 | EdgeCase — a hard reset to the current tip still discards uncommitted changes | EdgeCase | SC-921 | FR-NEW-254, FR-NEW-251 | P1 |
| E2E-NEW-866 | Happy — reverting the initial commit empties the tree | Happy | SC-922 | FR-NEW-263, FR-NEW-260 | P1 |
| E2E-NEW-867 | SideEffect — the reverted initial commit stays in history and stays readable | SideEffect | SC-922 | FR-NEW-263, FR-NEW-255 | P1 |
| E2E-NEW-868 | EdgeCase — a chain of three reverts lands on the removed state | EdgeCase | SC-922 | FR-NEW-265 | P2 |
| E2E-NEW-869 | SideEffect — each revert is its own commit and the original is never rewritten | SideEffect | SC-922 | FR-NEW-265, FR-NEW-260 | P1 |
| E2E-NEW-870 | EdgeCase — a paused merge survives the reopen with its conflict set intact | EdgeCase | SC-904 | FR-NEW-278, FR-NEW-275 | P0 |
| E2E-NEW-871 | SideEffect — partial resolutions recorded before the restart are still applied after it | SideEffect | SC-904 | FR-NEW-278, FR-NEW-178 | P0 |
| E2E-NEW-872 | Happy — `git.remote_fetch` runs during a paused merge and updates only tracking refs | Happy | SC-929 | FR-NEW-280, FR-NEW-147 | P0 |
| E2E-NEW-873 | EdgeCase — the whole read-only set answers during a paused rebase | EdgeCase | SC-929 | FR-NEW-280 | P0 |
| E2E-NEW-874 | EdgeCase — the step counters track a multi-step rebase | EdgeCase | SC-929 | FR-NEW-281, FR-NEW-220 | P1 |
| E2E-NEW-875 | SideEffect — status names the cherry-pick tools for a paused cherry-pick | SideEffect | SC-929 | FR-NEW-281, FR-NEW-238 | P0 |
| E2E-NEW-876 | Happy — a second member continues a rebase the first member started | Happy | SC-929 | FR-NEW-282, FR-NEW-221 | P0 |
| E2E-NEW-877 | Failure — a non-member still cannot touch another project's operation | Security | SC-929 | FR-NEW-282, FR-NEW-100 | P0 |
| E2E-NEW-878 | Failure — a GitLab approvals sub-call 500 fails `pr_get` | Failure | SC-927 | FR-NEW-308, FR-NEW-307 | P0 |
| E2E-NEW-879 | EdgeCase — a genuinely empty check set reports `none` and succeeds | EdgeCase | SC-927 | FR-NEW-308, FR-NEW-307 | P0 |
| E2E-NEW-880 | EdgeCase — a configured `git.gitlab_scope` overrides the default | EdgeCase | SC-925 | FR-NEW-331, FR-NEW-330 | P1 |
| E2E-NEW-881 | EdgeCase — with no configuration the defaults are exactly the documented strings | EdgeCase | SC-925 | FR-NEW-331, FR-MOD-109 | P1 |
| E2E-NEW-890 | SideEffect — pushing to `upstream` leaves `origin`'s tracking ref alone | SideEffect | SC-907 | FR-MOD-101 | P0 |
| E2E-NEW-891 | Failure — an invalid `remote_branch` name is refused before contacting the remote | Failure | SC-908 | FR-MOD-102, FR-NEW-104 | P0 |
| E2E-NEW-892 | EdgeCase — `git.remote_pull {remote:"upstream"}` pulls upstream and leaves origin untouched | EdgeCase | SC-907 | FR-MOD-103 | P0 |
| E2E-NEW-893 | EdgeCase — a full ref path as `start_point` | EdgeCase | SC-901 | FR-NEW-103, FR-NEW-101 | P1 |
| E2E-NEW-894 | SideEffect — `branch_reset` audits both shas so the move is recoverable | SideEffect | SC-924 | FR-NEW-111, FR-NEW-255 | P0 |
| E2E-NEW-895 | EdgeCase — dirt that is only an added file is still stashable | EdgeCase | SC-902 | FR-NEW-121, FR-NEW-120 | P0 |
| E2E-NEW-896 | Failure — a stash ref cannot be checked out | Failure | SC-902 | FR-NEW-123 | P0 |
| E2E-NEW-897 | EdgeCase — one stash applied onto two branches in turn | EdgeCase | SC-902 | FR-NEW-124, FR-NEW-129 | P1 |
| E2E-NEW-898 | Failure — a quota-exhausted `stash_pop` keeps the entry | Failure | SC-902 | FR-NEW-125, FR-NEW-185 | P0 |
| E2E-NEW-899 | DataIntegrity — a stash applies after its base commit became unreachable | DataIntegrity | SC-930 | FR-NEW-129, FR-NEW-255 | P0 |
| E2E-NEW-900 | EdgeCase — re-adding a remote with the identical URL is still a duplicate | EdgeCase | SC-907 | FR-NEW-141 | P1 |
| E2E-NEW-901 | SideEffect — `remote_remove` deletes only that remote's tracking refs | SideEffect | SC-907 | FR-NEW-142 | P0 |
| E2E-NEW-902 | EdgeCase — `remote_list` resolves host and provider for an enterprise host | EdgeCase | SC-907 | FR-NEW-144, FR-NEW-300 | P1 |
| E2E-NEW-903 | Security — remote tools stay silent about a stored token | Security | SC-907 | FR-NEW-145 | P0 |
| E2E-NEW-904 | SideEffect — a fetch leaves local branches and the volume byte-identical | SideEffect | SC-907 | FR-NEW-147 | P0 |
| E2E-NEW-905 | SideEffect — a forced push advances the local tracking ref to the new sha | SideEffect | SC-909 | FR-NEW-155 | P0 |
| E2E-NEW-906 | EdgeCase — a whitespace-only lease with `force:true` is refused before the network | EdgeCase | SC-909 | FR-NEW-156 | P1 |
| E2E-NEW-907 | EdgeCase — one mixed resolution call spanning a text, a binary and a delete/modify path | EdgeCase | SC-905 | FR-NEW-176, FR-NEW-180, FR-NEW-182 | P0 |
| E2E-NEW-908 | Failure — abort after a partial resolve discards the recorded resolutions | Failure | SC-904 | FR-NEW-178, FR-NEW-197 | P0 |
| E2E-NEW-909 | Failure — literal content is refused for a type-change conflict | Failure | SC-904 | FR-NEW-183, FR-NEW-175 | P1 |
| E2E-NEW-910 | DataIntegrity — the ref advances only after the last file write lands | DataIntegrity | SC-904 | FR-NEW-184 | P0 |
| E2E-NEW-911 | EdgeCase — a 250-path conflict set resolved in one `merge_resolve` call | EdgeCase | SC-904 | FR-NEW-196, FR-NEW-184 | P1 |
| E2E-NEW-912 | EdgeCase — `merge_resolve` refuses to advance a rebase and says which tool would | EdgeCase | SC-929 | FR-NEW-198, FR-NEW-225 | P0 |
| E2E-NEW-913 | SideEffect — `pick` preserves author identity and timestamp, and sets the committer to the caller | SideEffect | SC-914 | FR-NEW-211 | P0 |
| E2E-NEW-914 | EdgeCase — dropping a commit a later entry depends on pauses instead of silently succeeding | EdgeCase | SC-915 | FR-NEW-213, FR-NEW-219, FR-NEW-286 | P1 |
| E2E-NEW-915 | SideEffect — an `up_to_date` rebase writes nothing at all | SideEffect | SC-914 | FR-NEW-217 | P1 |
| E2E-NEW-916 | EdgeCase — aborting after one clean step removes the replayed commit from the branch but keeps its object | EdgeCase | SC-917 | FR-NEW-223, FR-NEW-255 | P1 |
| E2E-NEW-917 | Concurrency — `git.branch_create` cannot interleave into a running rebase | Concurrency | SC-916 | FR-NEW-226, FR-NEW-115 | P1 |
| E2E-NEW-918 | SideEffect — an `already_present` cherry-pick changes nothing observable | SideEffect | SC-918 | FR-NEW-236 | P1 |
| E2E-NEW-919 | EdgeCase — a sha that names a non-commit object | EdgeCase | SC-918 | FR-NEW-237 | P1 |
| E2E-NEW-920 | SideEffect — a soft reset forward leaves the volume behind and the status dirty | SideEffect | SC-920 | FR-NEW-250 | P0 |
| E2E-NEW-921 | DataIntegrity — a mistaken hard reset is fully recovered from the reported `old_sha` | DataIntegrity | SC-921 | FR-NEW-255, FR-NEW-111 | P0 |
| E2E-NEW-922 | SideEffect — a conflicted revert creates no commit and leaves the volume byte-identical | SideEffect | SC-922 | FR-NEW-264, FR-NEW-171 | P0 |
| E2E-NEW-923 | EdgeCase — an unsupported provider is rejected before any token is read | EdgeCase | SC-925 | FR-NEW-301, FR-NEW-332 | P0 |
| E2E-NEW-924 | EdgeCase — a remote whose URL has no project path | EdgeCase | SC-925 | FR-NEW-302 | P1 |
| E2E-NEW-925 | EdgeCase — the head-branch pre-flight passes and the create follows, in that order | EdgeCase | SC-925 | FR-NEW-305, FR-NEW-304 | P0 |
| E2E-NEW-926 | Integration — one injected client serves a GitHub and a GitLab mount in the same process | Integration | SC-925 | FR-NEW-315, FR-NEW-300, FR-NEW-303 | P0 |
| E2E-NEW-927 | Security — a same-host redirect is followed, a cross-host one is not | Security | SC-925 | FR-NEW-316 | P0 |
| E2E-NEW-928 | EdgeCase — the scope error names the host and both remedy tools | EdgeCase | SC-926 | FR-NEW-333 | P1 |
| E2E-NEW-929 | EdgeCase — an unknown-scope token that the provider accepts works end to end | EdgeCase | SC-925 | FR-NEW-334 | P0 |
| E2E-NEW-930 | EdgeCase — `auth_status` reports an empty scope set as unknown, not as insufficient | EdgeCase | SC-925 | FR-NEW-335, FR-NEW-334 | P1 |
| E2E-NEW-931 | Integration — every hardcoded tool-count site reports the new numbers | Integration | SC-925 | FR-NEW-346, FR-NEW-345 | P0 |
| E2E-NEW-940 | Happy — a divergent pull with disjoint changes merges with no caller decision | Happy | SC-911 | FR-MOD-104, FR-NEW-173, FR-DEL-101 | P0 |
| E2E-NEW-941 | Failure — a divergent pull is still refused on a dirty volume | Failure | SC-911 | FR-MOD-104, FR-NEW-194 | P0 |
| E2E-NEW-942 | Failure — the automatic merge is refused when the quota cannot cover it | Failure | SC-911 | FR-MOD-104, FR-NEW-185, FR-NEW-184 | P0 |
| E2E-NEW-943 | gap-closure test | EdgeCase | SC-911 | FR-NEW-173, FR-MOD-104 | P0 |
| E2E-NEW-944 | SideEffect — the automatic pull merge leaves exactly the expected refs, parents and audit | SideEffect | SC-911 | FR-MOD-104, FR-NEW-173, FR-NEW-184 | P0 |
| E2E-NEW-945 | Happy — a pull conflict resolved with bytes belonging to neither side | Happy | SC-913 | FR-MOD-104, FR-MOD-105, FR-NEW-175, FR-NEW-196 | P0 |
| E2E-NEW-946 | Failure — supplied content is refused for a binary path in a pull conflict | Failure | SC-913 | FR-NEW-182, FR-MOD-105 | P0 |
| E2E-NEW-947 | Failure — one bad path rejects the whole content resolution | Failure | SC-913 | FR-NEW-177, FR-NEW-175, FR-MOD-105 | P0 |
| E2E-NEW-948 | EdgeCase — empty supplied content produces an empty file, not a deletion | EdgeCase | SC-913 | FR-NEW-175, FR-MOD-105 | P1 |
| E2E-NEW-949 | SideEffect — the resolved pull charges exactly the written bytes and clears the row | SideEffect | SC-913 | FR-NEW-184, FR-NEW-284, FR-MOD-105 | P0 |
| E2E-NEW-950 | Happy — stash taken on a deleted branch applies cleanly onto another | Happy | SC-930 | FR-NEW-129, FR-NEW-124, FR-NEW-122 | P0 |

| E2E-NEW-951 | each operation type routes to exactly one completion pair | Integration | SC-929 | FR-NEW-241, FR-NEW-225 | P0 |
| E2E-NEW-952 | `git.revert_continue` rejected while a rebase is paused | Failure | SC-929 | FR-NEW-241, FR-NEW-225 | P0 |
| E2E-NEW-953 | conflict response `continue_with`/`abort_with` name the right pair per op | EdgeCase | SC-929 | FR-NEW-241, FR-NEW-170 | P0 |
| E2E-NEW-954 | `git.revert_continue` completes a conflicted revert | Happy | SC-922 | FR-NEW-266, FR-NEW-264 | P0 |
| E2E-NEW-955 | `git.revert_abort` restores tip and every byte exactly | SideEffect | SC-922 | FR-NEW-266, FR-NEW-171 | P0 |
| E2E-NEW-956 | `git.revert_continue` with no revert in progress | Failure | SC-922 | FR-NEW-266 | P1 |
| E2E-NEW-957 | documented tool count matches the live registry count | Integration | SC-929 | FR-NEW-348, FR-NEW-346 | P1 |
| E2E-NEW-958 | `.agent_docs/tools.md` lists every registered git tool | Integration | SC-929 | FR-NEW-348, FR-NEW-349 | P1 |
| E2E-NEW-959 | a tool absent from the docs fails the docs-parity test | Failure | SC-929 | FR-NEW-348 | P1 |
| E2E-NEW-960 | the 31 names of FR-NEW-349 are all registered, and nothing else is new | Integration | SC-929 | FR-NEW-349, FR-NEW-345 | P0 |
| E2E-NEW-961 | registering a 32nd unlisted git tool fails the enumeration test | Failure | SC-929 | FR-NEW-349 | P1 |
| E2E-MOD-401 | rewrite `e2e_new_118` onto `git.merge_resolve` with a per-file strategy | Failure | SC-912 | FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101 | P0 |
| E2E-MOD-402 | rewrite `e2e_new_128` onto `ours` per-file resolution | Failure | SC-912 | FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101 | P0 |
| E2E-MOD-403 | rewrite `e2e_new_129`: disjoint changes now merge with no strategy at all | Happy | SC-911 | FR-MOD-104, FR-NEW-173, FR-DEL-101 | P0 |
| E2E-MOD-404 | rewrite `e2e_new_125`: fetched objects retained on a conflict response | SideEffect | SC-912 | FR-MOD-104, FR-MOD-105 | P0 |
| E2E-MOD-405 | tool name list and count at `git.rs:2480-2504`, 14 -> 45 | Integration | SC-929 | FR-NEW-345, FR-NEW-346, FR-NEW-349 | P0 |
| E2E-MOD-406 | extend the platform-admin forbidden test to all 31 new tools | Security | SC-929 | FR-NEW-100, FR-NEW-347, FR-NEW-349 | P0 |
| E2E-MOD-407 | count assertions in `all.rs` `:78`, `:122`, `:141`; `:64` and `:99` unchanged | Integration | SC-929 | FR-NEW-346 | P0 |
| E2E-MOD-408 | frozen contract count at `contract_golden.rs:131`, 63 -> 94 | Integration | SC-929 | FR-NEW-345, FR-NEW-346, FR-NEW-349 | P0 |
| E2E-MOD-409 | regenerate `tool-contract-golden.json` and `TOOL_CONTRACT.txt` | Integration | SC-929 | FR-NEW-345, FR-NEW-348, FR-NEW-349 | P0 |
| E2E-DEL-401 | remove `e2e_new_117`: the refuse-without-strategy behaviour is gone | Failure | SC-911 | FR-DEL-101, FR-MOD-104 | P0 |
| E2E-DEL-402 | remove `e2e_new_126`: the error it pinned no longer exists | Failure | SC-911 | FR-DEL-101, FR-MOD-104 | P0 |

**Coverage Statistics:**

| Category | Count |
|---|---|
| Happy | 95 |
| Failure | 175 |
| SideEffect | 83 |
| EdgeCase | 118 |
| Security | 23 |
| DataIntegrity | 17 |
| Concurrency | 10 |
| Integration | 15 |
| Performance | 0 |
| **Total** | **536** |

Composition: 525 new tests (`E2E-NEW-400..961`), 9 modified shipped tests (`E2E-MOD-401..409`), 2 removed shipped tests (`E2E-DEL-401..402`).

Per band: A (branch lifecycle, stash, remotes, force push with lease) `E2E-NEW-400..498`, 99; B (shared conflict model, merge, squash, pull rework, in-progress guard) `E2E-NEW-500..580`, 81; C (interactive rebase, cherry-pick, reset, revert) `E2E-NEW-600..699`, 100; D (pull request tools, provider normalization, OAuth scope) `E2E-NEW-700..799`, 100; Z (gap closure, completion-pair routing, revert continuation, contract enumeration) `E2E-NEW-800..961`, 145.

**Happy : Failure = 95 : 175 (ratio 1 : 1.84).** Failure-path tests outnumber happy-path tests, which is the required direction.

Counting every adversarial and verification category together (Failure, Security, DataIntegrity, EdgeCase, Concurrency) against Happy, the ratio is 95 : 343.

**Coverage gaps, stated rather than hidden:**

Every gap reported by the first assembly pass has been closed by the Z band
(`E2E-NEW-800..950`). Specifically: `SC-911` and `SC-913` had no test of any category and
now have five each; `SC-930` had no happy path and now has `E2E-NEW-950`; `SC-910`'s
"happy" column is the successful-lease force push `E2E-NEW-492`, since the scenario's
successful outcome is a correct rejection, which `E2E-NEW-831` also covers.

Two requirements remain only partially reachable by the default test gate, stated plainly
rather than counted as covered:

- **FR-NEW-276** (the new table is registered in `TABLES` so a project purge removes its
  rows). The observable consequence is only reachable on PostgreSQL or SQL Server: on
  SQLite `purge_repo` deletes the index file outright and never reads `TABLES`
  (`crates/mcp-fs/src/git/repo.rs:146-161`, `crates/mcp-fs/src/storage/mod.rs:357`).
  `E2E-NEW-806` and `E2E-NEW-807` therefore run in the opt-in relational suites, and
  `E2E-NEW-808` pins `TABLES.len() == 4` plus a `storage/conformance.rs` extension so a
  default `cargo test --workspace` still catches the regression.
- **FR-NEW-348** (documentation updates). Not end-to-end testable; it is verified at
  review, and partially pinned by the frozen-contract tests that fail when the tool set
  and the documented counts diverge.

### 12.2 New Test Specifications

Every test below is reproduced in full: preconditions, Given/When/Then/And steps, the
exact assertion values, the verification method, cleanup and priority. Each carries
its category, the scenario it validates and the requirements it covers. The groupings
and their shared harness sections are the ones the designs were written against.

#### Band A. Branch lifecycle, stash, remotes, force push with lease

##### 0. Shared harness and seed data

All tests are `#[tokio::test]` in `crates/mcp-fs/src/tools/git.rs`'s `mod tests`, using the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT = "gitproj"`, `OWNER = "owner@test.com"` (the seeded member, `crates/mcp-fs/src/tools/git.rs:2370`), `Env::call` (as OWNER), `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Non-member id used throughout: `"stranger@test.com"`. Platform admin id: `ADMIN` (imported at `crates/mcp-fs/src/tools/git.rs:2356`), which gets **no** implicit file access (`authorize`, `crates/mcp-fs/src/tools/git.rs:386-396`).

Verification primitives available today:
- **ref value**: `env.git.get_db(MOUNT).await.unwrap().get_ref("refs/heads/x").await` -> `GitRefRow { target, symbolic }` (`crates/mcp-fs/src/git/db.rs:214-234`).
- **ref list**: `db.list_refs()` (`crates/mcp-fs/src/git/db.rs:261`).
- **remotes rows**: `db.list_remotes()` (`crates/mcp-fs/src/git/db.rs:298`).
- **audit**: `env.f.state.safety.audit(OWNER, MOUNT)` -> `Vec<AuditEntry { op, path, detail }>` (`crates/mcp-fs/src/safety.rs:147-157`).
- **quota**: `env.f.state.safety.bytes_written(OWNER, MOUNT)` (`crates/mcp-fs/src/safety.rs:159`); tight-quota env via `Env::with_quota` (`crates/mcp-fs/src/tools/git.rs:2384`).
- **bare-repo inspection**: `git2::Repository::open_bare(dir)` + `find_reference("refs/heads/main")`, as the existing push tests do (`crates/mcp-fs/src/tools/git.rs:4348-4460`).
- **remote seeding**: `seed_bare_remote` (`crates/mcp-fs/src/tools/git.rs:4117`), `advance_bare_remote` (`:4155`), `seed_bare_remote_files` (`:4173`), `advance_bare_remote_write` (`:4216`).
- Error codes come from `crate::errors::code` (`crates/mcp-fs/src/errors.rs:10-23`); assertions are `assert_eq!(e.code, code::X)` plus `assert!(e.message.contains("..."))`.

**`file://` caveat, already established:** `resolve_clone_credential` rejects non-https before any remote function runs (`crates/mcp-fs/src/git/remote.rs:246-262`), so every remote-mechanics test drives the internal function (`push_branch_inner`/`fetch_branch_inner`, the pattern of `call_push_branch`, `crates/mcp-fs/src/tools/git.rs:4326`), while every *validation/authorization* test drives the registered tool. Each test below states which.

##### Seeds

- **SEED-A**: `git.init`; write `/README.md` = `"alpha\n"`; `git.commit "c1"` -> `<sha-C1>`; write `/src/lib.rs` = `"fn a() {}\n"`; `git.commit "c2"` -> `<sha-C2>`. State: `HEAD` symbolic -> `refs/heads/main` -> `<sha-C2>`.
- **SEED-B**: SEED-A, then `git.branch_create {name:"release/1.0", start_point:<sha-C1>, checkout:true}`; write `/docs/rel.md` = `"rel\n"`; `git.commit "c3"` -> `<sha-C3>`; `git.branch_switch {name:"main"}`. State: two branches, `main`=`<sha-C2>`, `release/1.0`=`<sha-C3>` (unmerged into main), HEAD on `main`.
- **SEED-R**: SEED-A, plus `remote_dir = tempdir()`, `url = seed_bare_remote(&remote_dir, "hello\n")` -> remote `refs/heads/main` = `<sha-R1>`; `db.add_remote("origin", &url)`.
- **SEED-R2**: SEED-R, plus `remote_dir2 = tempdir()`, `url2 = seed_bare_remote(&remote_dir2, "up\n")` -> `<sha-U1>`; `db.add_remote("upstream", &url2)`.

##### Contract constants this spec fixes (implementer must adopt them verbatim)

- `MAX_BRANCH_NAME_BYTES = 255`.
- Branch name validity = git's `check_ref_format` subset: rejects empty, leading/trailing `/`, `..`, ASCII space, `~ ^ : ? * [ \`, control chars, trailing `.lock`, a component starting with `.`.
- `MAX_STASH_ENTRIES = 100` per volume.
- Stash ref name: `refs/stash/{stash_id}` where `stash_id` is the 40-char sha of the stash commit; `git.stash_list` returns `stash_id`, `message`, `base_sha`, `branch`, `created_at`, newest first (FR-NEW-131).
- A conflicting apply/pop returns `Ok` with `"status": "conflict"` plus `"conflicts": [{path, ours, theirs, base, binary, type_change}]` per FR-NEW-186 (mirrors the pull conflict model, `crates/mcp-fs/src/tools/git.rs:1844-1868`), never `ERR_*`, and mutates nothing.
- A clean apply/pop returns `"status": "applied"`.
- Audit ops: `git.branch_create`, `git.branch_switch`, `git.branch_delete`, `git.branch_reset`, `git.stash_save`, `git.stash_apply`, `git.stash_pop`, `git.stash_drop`, `git.remote_add`, `git.remote_remove`, `git.remote_push` (existing, `crates/mcp-fs/src/git/remote.rs:364-371`).
- The all-zero sha `"0".repeat(40)` is the lease value meaning "branch must not exist on the remote".

---

##### 2. Full specifications

##### A) Branch lifecycle

---

**E2E-NEW-400 — Happy — create a branch at an explicit start point**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-101.
Preconditions: SEED-A.
- **Given** `refs/heads/main` = `<sha-C2>` and `HEAD` = symbolic `refs/heads/main`.
- **When** `git.branch_create {"mount_id":"gitproj","name":"feature/login","start_point":"<sha-C2>"}`.
- **Then** the response is exactly `{"branch":"feature/login","sha":"<sha-C2>","checked_out":false}` (tool response fields).
- **And** `db.get_ref("refs/heads/feature/login")` returns `GitRefRow { target: "<sha-C2>", symbolic: false }` (direct `git_refs` row check).
- **And** `db.get_ref("HEAD")` still returns `target:"refs/heads/main", symbolic:true`.
- **And** `git.branches` lists exactly two entries with `full_ref` `refs/heads/feature/login` and `refs/heads/main`.
Cleanup: none (tempdir fixture).
Priority: P0.

---

**E2E-NEW-401 — Happy — create with checkout rewrites HEAD and the volume**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-101, FR-NEW-105.
Preconditions: SEED-A. `/src/lib.rs` exists only in `<sha-C2>`.
- **Given** the volume holds `/README.md` = `"alpha\n"` and `/src/lib.rs` = `"fn a() {}\n"`.
- **When** `git.branch_create {"mount_id":"gitproj","name":"hotfix/c1","start_point":"<sha-C1>","checkout":true}`.
- **Then** the response has `"checked_out": true` and `"sha":"<sha-C1>"`.
- **And** `db.get_ref("HEAD")` = `{ target:"refs/heads/hotfix/c1", symbolic:true }`.
- **And** a follow-up `git.status` returns `"branch":"hotfix/c1"` and `"head":"<sha-C1>"`.
- **And** `env.read("/README.md")` == `"alpha\n"` and `client.exists("/src/lib.rs")` is `false` (volume inspection via `env.f.state.stores.client(MOUNT)`).
Priority: P0.

---

**E2E-NEW-402 — Failure — an existing branch name is refused**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-102.
Preconditions: SEED-A.
- **When** `git.branch_create {"mount_id":"gitproj","name":"main","start_point":"<sha-C1>"}`.
- **Then** the call errors with `code == ERR_NO_CLOBBER` and `message.contains("branch 'main' already exists")`.
- **And** `db.get_ref("refs/heads/main").target` is still `<sha-C2>` (the existing branch was NOT moved to `<sha-C1>`).
Priority: P0.

---

**E2E-NEW-403 — Failure — an unresolvable start point**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-103.
Preconditions: SEED-A.
- **When** `git.branch_create {"mount_id":"gitproj","name":"feature/x","start_point":"nosuchref"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("nosuchref")`.
- **And** `db.get_ref("refs/heads/feature/x")` is `None`.
Note: `resolve_ref` treats an all-hex name as a raw sha (`crates/mcp-fs/src/tools/git.rs:486-504`), so the start point MUST contain a non-hex character (`nosuchref` contains `n`,`s`,`u`,`r`) for this to resolve to `None` rather than to a bogus sha. A second assertion covers the hex case: `start_point:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"` also errors `ERR_NOT_FOUND` with `message.contains("deadbeef")`, because the commit object is absent.
Priority: P0.

---

**E2E-NEW-404 — Failure — an invalid branch name is refused before any write**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A.
- **When** `git.branch_create` is called once per name in `["", "bad..name", "has space", "/leading", "trailing/", "tip.lock", "wild*card", "caret^name"]`, each with `start_point:"<sha-C2>"`.
- **Then** every call errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("is not a valid branch name")`.
- **And** `db.list_refs()` contains exactly `["HEAD","refs/heads/main"]` afterwards (direct `git_refs` check: no partial row for any of the eight names).
Priority: P0.

---

**E2E-NEW-405 — EdgeCase — a unicode branch name round-trips exactly**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-104, FR-NEW-101., FR-NEW-116
Preconditions: SEED-A.
- **When** `git.branch_create {"name":"feature/café-日本","start_point":"<sha-C2>"}`.
- **Then** the response `"branch"` is exactly `"feature/café-日本"` (NFC bytes as written, not normalized).
- **And** `db.get_ref("refs/heads/feature/café-日本").target == "<sha-C2>"`.
- **And** `git.branches` contains an entry whose `"name"` is exactly `"feature/café-日本"` and whose `"full_ref"` is `"refs/heads/feature/café-日本"`.
- **And** `git.branch_switch {"name":"feature/café-日本"}` succeeds and `git.status` reports `"branch":"feature/café-日本"`.
Priority: P1.

---

**E2E-NEW-406 — EdgeCase — a 255-byte branch name is accepted**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A. `let name = format!("f/{}", "a".repeat(253));` (exactly 255 chars, asserted in-test with `assert_eq!(name.chars().count(), 255)`).
- **When** `git.branch_create {"name":name,"start_point":"<sha-C2>"}`.
- **Then** the call succeeds and `db.get_ref(&format!("refs/heads/{name}")).target == "<sha-C2>"`.
Priority: P2.

---

**E2E-NEW-407 — Failure — a 256-byte branch name is refused**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-104., FR-NEW-116
Preconditions: SEED-A. `let name = format!("f/{}", "a".repeat(254));` (256 chars, asserted).
- **When** `git.branch_create {"name":name,"start_point":"<sha-C2>"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("256")` and `message.contains("255")`.
- **And** `db.list_refs()` is unchanged (still 2 rows).
Priority: P2.

---

**E2E-NEW-408 — Failure — create+checkout is refused on a dirty volume**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-106, FR-NEW-101.
Preconditions: SEED-A, then `env.write("/README.md", "alpha MODIFIED\n")` (uncommitted).
- **When** `git.branch_create {"name":"feature/dirty","start_point":"<sha-C1>","checkout":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("uncommitted changes")`.
- **And** `env.read("/README.md") == "alpha MODIFIED\n"` (the caller's work is intact).
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
Verification: the dirty test reuses `require_clean_volume` (`crates/mcp-fs/src/tools/git.rs:1991-2009`); the message MUST be the branch-specific one, not the string `"pull refused"`, so the test also asserts `!message.contains("pull refused")`.
Priority: P0.

---

**E2E-NEW-409 — SideEffect — the rejected create of E2E-NEW-408 leaves nothing behind**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-106, FR-NEW-101.
Preconditions: identical to E2E-NEW-408, run to its failure.
- **Then** `db.get_ref("refs/heads/feature/dirty")` is `None` (direct row check: no branch was created before the dirty check ran).
- **And** `safety.audit(OWNER, MOUNT)` contains no entry with `op == "git.branch_create"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured immediately before the call.
Priority: P0.

---

**E2E-NEW-410 — Failure — create in a repo with no commits**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-103.
Preconditions: `git.init` only (no commit). `db.get_ref("refs/heads/main")` is `None`.
- **When** `git.branch_create {"name":"feature/x","start_point":"HEAD"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("HEAD")` and `message.contains("no commits")`.
- **And** `db.list_refs()` contains no `refs/heads/feature/x` row.
Priority: P1.

---

**E2E-NEW-411 — Security — a non-member cannot create a branch**

**Category:** Security. **Scenario:** SC-901. **Requirements:** FR-NEW-100., FR-NEW-189
Preconditions: SEED-A.
- **When** `env.as_person("stranger@test.com", "git.branch_create", {"mount_id":"gitproj","name":"feature/evil","start_point":"<sha-C2>"})`.
- **Then** `code == ERR_FORBIDDEN` and `message.contains("stranger@test.com")`.
- **And** `db.get_ref("refs/heads/feature/evil")` is `None`.
- **And** the same assertion is repeated for `git.branch_switch`, `git.branch_delete`, `git.branch_reset` with the same person, each returning `ERR_FORBIDDEN`.
Priority: P0.

---

**E2E-NEW-412 — Security — a platform admin who is not a member is forbidden**

**Category:** Security. **Scenario:** SC-901. **Requirements:** FR-NEW-100.
Preconditions: SEED-A. `ADMIN` is the platform admin fixture id and is not a member of `gitproj`.
- **When** `env.as_person(ADMIN, "git.branch_create", {...,"name":"feature/admin","start_point":"<sha-C2>"})`.
- **Then** `code == ERR_FORBIDDEN`.
- **And** `db.get_ref("refs/heads/feature/admin")` is `None`.
Rationale: mirrors `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`crates/mcp-fs/src/tools/git.rs:2685`); the four new branch tools must be added to that list too.
Priority: P0.

---

**E2E-NEW-413 — Happy — switch repoints HEAD**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-105., FR-NEW-189
Preconditions: SEED-B (HEAD on `main` = `<sha-C2>`; `release/1.0` = `<sha-C3>`), volume clean.
- **When** `git.branch_switch {"mount_id":"gitproj","name":"release/1.0"}`.
- **Then** the response is `{"branch":"release/1.0","sha":"<sha-C3>","changed":true,"files_changed":2}`.
- **And** `db.get_ref("HEAD")` = `{ target:"refs/heads/release/1.0", symbolic:true }`.
- **And** `git.status` returns `"branch":"release/1.0"`, `"head":"<sha-C3>"`.
Priority: P0.

---

**E2E-NEW-414 — SideEffect — switch rewrites the volume to the target tree**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105., FR-NEW-189
Preconditions: SEED-B; before the switch the volume holds `/README.md`=`"alpha\n"`, `/src/lib.rs`=`"fn a() {}\n"`, and no `/docs/rel.md`.
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `env.read("/docs/rel.md") == "rel\n"` (a path present only on the target).
- **And** `client.exists("/src/lib.rs")` is `false` (a path present only on the source is deleted).
- **And** `env.read("/README.md") == "alpha\n"` (a path common to both is untouched).
- **And** switching back with `git.branch_switch {"name":"main"}` restores `/src/lib.rs` to `"fn a() {}\n"` and removes `/docs/rel.md`.
Priority: P0.

---

**E2E-NEW-415 — Failure — switch refuses a dirty volume and touches nothing**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-106.
Preconditions: SEED-B, then `env.write("/src/lib.rs", "fn a() { todo!() }\n")` (uncommitted).
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("uncommitted changes")` and `message.contains("release/1.0")`.
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
- **And** `env.read("/src/lib.rs") == "fn a() { todo!() }\n"` (the uncommitted edit survives).
- **And** `client.exists("/docs/rel.md")` is `false` (no partial checkout).
Priority: P0.

---

**E2E-NEW-416 — Failure — switch to an unknown branch**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: SEED-A.
- **When** `git.branch_switch {"name":"release/9.9"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("branch 'release/9.9'")`.
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
Priority: P0.

---

**E2E-NEW-417 — EdgeCase — switching to the current branch is a no-op**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-107.
Preconditions: SEED-A, volume clean, `let before = safety.bytes_written(OWNER, MOUNT);`.
- **When** `git.branch_switch {"name":"main"}`.
- **Then** the response is `{"branch":"main","sha":"<sha-C2>","changed":false,"files_changed":0}`.
- **And** `safety.bytes_written(OWNER, MOUNT) == before` (no quota charged for a no-op).
- **And** `env.read("/README.md") == "alpha\n"` and `env.read("/src/lib.rs") == "fn a() {}\n"`.
Priority: P1.

---

**E2E-NEW-418 — Failure — switch under an exhausted quota**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: `Env::with_quota(N)` where `N` is sized to admit SEED-B's commits but not the checkout writes; concretely build SEED-B under `Env::with_quota(i64::MAX)` first is not possible in one env, so: build SEED-B under a normal env, then drain the quota by constructing the env with `Env::with_quota(60)` and performing SEED-B (whose committed bytes are `"alpha\n"`=6 + `"fn a() {}\n"`=10 + `"rel\n"`=4 = 20 written by `env.write`), then call switch with only 3 bytes of headroom remaining. The test asserts the pre-call `bytes_written` value explicitly so the headroom is not guessed.
- **When** `git.branch_switch {"name":"release/1.0"}` (which must write `/docs/rel.md`, 4 bytes).
- **Then** `code == ERR_WRITE_QUOTA_EXCEEDED` and `message.contains("session write quota of 60 bytes exceeded")` (the exact wording of `charge_write`, `crates/mcp-fs/src/safety.rs:136-138`).
Priority: P0.

---

**E2E-NEW-419 — SideEffect — the quota-refused switch of E2E-NEW-418 left the volume untouched**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105, FR-NEW-184.
Preconditions: E2E-NEW-418 run to its failure.
- **Then** `client.exists("/docs/rel.md")` is `false`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (not deleted).
- **And** `db.get_ref("HEAD").target == "refs/heads/main"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the pre-call value (a rejected charge consumes nothing, `crates/mcp-fs/src/safety.rs:131-143`).
Rationale: the quota must be charged in a pre-flight pass, like `charge_pull_quota` + `apply_pull_changes_atomically` (`crates/mcp-fs/src/tools/git.rs:1967-1989`, `:2079-2127`).
Priority: P0.

---

**E2E-NEW-420 — SideEffect — a successful switch charges quota and writes one audit entry**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-105.
Preconditions: SEED-B, `let before = safety.bytes_written(OWNER, MOUNT);`.
- **When** `git.branch_switch {"name":"release/1.0"}`.
- **Then** `safety.bytes_written(OWNER, MOUNT) - before == 4` (only `/docs/rel.md`'s `"rel\n"` is written; the deletion of `/src/lib.rs` adds nothing, matching `charge_pull_quota`).
- **And** `safety.audit(OWNER, MOUNT)` ends with exactly one entry where `op == "git.branch_switch"`, `path == "/"`, and `detail.contains("release/1.0")` and `detail.contains("files_changed 2")`.
Priority: P0.

---

**E2E-NEW-421 — Concurrency — two switches race**

**Category:** Concurrency. **Scenario:** SC-901. **Requirements:** FR-NEW-115.
Preconditions: SEED-B plus a third branch `feature/z` created from `<sha-C1>` with an extra committed file `/z.txt`=`"z\n"` -> `<sha-C4>`; HEAD on `main`, volume clean.
- **When** `tokio::join!` of `git.branch_switch {"name":"release/1.0"}` and `git.branch_switch {"name":"feature/z"}` on the same `Env`.
- **Then** both calls return `Ok` (or one returns `ERR_INVALID_ARGUMENT` "uncommitted changes" if it observes the other's intermediate tree; both outcomes are accepted and the test asserts the set of outcomes is one of those two).
- **And** the final `db.get_ref("HEAD").target` is exactly one of `"refs/heads/release/1.0"` or `"refs/heads/feature/z"`.
- **And** the volume matches that branch's tree exactly: if `release/1.0`, `/docs/rel.md`=="rel\n" and `client.exists("/z.txt")==false`; if `feature/z`, `/z.txt`=="z\n" and `client.exists("/docs/rel.md")==false`. No mixed state is tolerated.
Verification: relies on `entry.write_lock` (`crates/mcp-fs/src/git/repo.rs:44`), which `branch_switch` MUST hold for the whole checkout, exactly as `commit` does (`crates/mcp-fs/src/tools/git.rs:659`).
Priority: P1.

---

**E2E-NEW-422 — Happy — delete a merged branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-108., FR-NEW-189
Preconditions: SEED-A, then `git.branch_create {"name":"feature/merged","start_point":"<sha-C1>"}` (an ancestor of `main`, therefore merged).
- **When** `git.branch_delete {"mount_id":"gitproj","name":"feature/merged"}`.
- **Then** the response is `{"branch":"feature/merged","sha":"<sha-C1>","forced":false}`.
- **And** `db.get_ref("refs/heads/feature/merged")` is `None`.
- **And** `git.branches` lists only `main`.
Priority: P0.

---

**E2E-NEW-423 — Failure — the checked-out branch cannot be deleted**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-109., FR-NEW-189
Preconditions: SEED-A, HEAD on `main`.
- **When** `git.branch_delete {"name":"main"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("'main' is the checked-out branch")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
Priority: P0.

---

**E2E-NEW-424 — Failure — an unmerged branch needs force**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-110.
Preconditions: SEED-B (`release/1.0` = `<sha-C3>`, not reachable from `main` = `<sha-C2>`), HEAD on `main`.
- **When** `git.branch_delete {"name":"release/1.0"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("release/1.0")`, `message.contains("not merged")`, and `message.contains("force")`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
Priority: P0.

---

**E2E-NEW-425 — Happy — force deletes an unmerged branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-NEW-108, FR-NEW-110.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branch_delete {"name":"release/1.0","force":true}`.
- **Then** the response is `{"branch":"release/1.0","sha":"<sha-C3>","forced":true}`.
- **And** `db.get_ref("refs/heads/release/1.0")` is `None`.
- **And** `git.branches` lists only `main`.
Priority: P0.

---

**E2E-NEW-426 — DataIntegrity — a force delete removes the ref, never the objects**

**Category:** DataIntegrity. **Scenario:** SC-901. **Requirements:** FR-NEW-108.
Preconditions: E2E-NEW-425 run to success.
- **Then** `git.show {"commit_sha":"<sha-C3>"}` still returns `commit.sha == "<sha-C3>"` and a non-empty `"diff"` (tool response).
- **And** `db.get_object("<sha-C3>")` returns `Some(row)` with `row.kind == "commit"` (direct `git_objects` check, `crates/mcp-fs/src/git/db.rs:143`).
Priority: P1.

---

**E2E-NEW-427 — Failure — delete an unknown branch**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-108.
Preconditions: SEED-A.
- **When** `git.branch_delete {"name":"feature/ghost"}` and again with `{"force":true}`.
- **Then** both error with `code == ERR_NOT_FOUND` and `message.contains("branch 'feature/ghost'")` (force does not turn a missing branch into a success).
Priority: P1.

---

**E2E-NEW-428 — SideEffect — the rejected deletes of E2E-NEW-423 and E2E-NEW-424 left every ref intact**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-109, FR-NEW-110.
Preconditions: SEED-B; run E2E-NEW-423's call and E2E-NEW-424's call in the same test, both to failure.
- **Then** `db.list_refs()` returns exactly the names `["HEAD","refs/heads/main","refs/heads/release/1.0"]`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
- **And** `safety.audit(OWNER, MOUNT)` contains no `op == "git.branch_delete"` entry.
Priority: P0.

---

**E2E-NEW-429 — EdgeCase — force does not defeat the self-delete guard**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-109.
Preconditions: SEED-A, HEAD on `main`.
- **When** `git.branch_delete {"name":"main","force":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("checked-out branch")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("HEAD").target == "refs/heads/main"`.
Priority: P1.

---

**E2E-NEW-430 — Happy — reset a non-checked-out branch backwards**

**Category:** Happy. **Scenario:** SC-924. **Requirements:** FR-NEW-111., FR-NEW-189
Preconditions: SEED-B, HEAD on `main`. `release/1.0` = `<sha-C3>`, whose parent is `<sha-C1>`.
- **When** `git.branch_reset {"mount_id":"gitproj","name":"release/1.0","target_commit":"<sha-C1>","force":true}`.
- **Then** the response is `{"branch":"release/1.0","old_sha":"<sha-C3>","new_sha":"<sha-C1>","checked_out":false,"files_changed":0}`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C1>"`.
- **And** `git.log {"ref_name":"release/1.0"}` returns exactly one commit whose sha is `<sha-C1>`.
Priority: P0.

---

**E2E-NEW-431 — SideEffect — resetting another branch never touches the volume**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-112., FR-NEW-189
Preconditions: E2E-NEW-430 run to success; `let before = safety.bytes_written(OWNER, MOUNT);` captured before the reset.
- **Then** `env.read("/README.md") == "alpha\n"` and `env.read("/src/lib.rs") == "fn a() {}\n"` (the `main` tree, unchanged).
- **And** `client.exists("/docs/rel.md")` is `false` (it never was in the `main` tree).
- **And** `safety.bytes_written(OWNER, MOUNT) == before` (no volume write, so no charge).
- **And** `safety.audit(OWNER, MOUNT)` has exactly one `op == "git.branch_reset"` entry with `detail.contains("<sha-C3>")` and `detail.contains("<sha-C1>")`.
Priority: P0.

---

**E2E-NEW-432 — Failure — a non-fast-forward reset without force names both shas**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-113.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C1>"}` (force omitted; `<sha-C1>` is an ancestor, so the move is a rewind, not a fast-forward).
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("not a fast-forward")`, `message.contains("<sha-C3>")` and `message.contains("<sha-C1>")`, and `message.contains("force")`.
- **And** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
Priority: P0.

---

**E2E-NEW-433 — EdgeCase — a fast-forward reset needs no force**

**Category:** EdgeCase. **Scenario:** SC-924. **Requirements:** FR-NEW-113.
Preconditions: SEED-A, then `git.branch_create {"name":"feature/behind","start_point":"<sha-C1>"}`. `<sha-C2>` is a descendant of `<sha-C1>`.
- **When** `git.branch_reset {"name":"feature/behind","target_commit":"<sha-C2>"}` (force omitted).
- **Then** the call succeeds with `"old_sha":"<sha-C1>","new_sha":"<sha-C2>"`.
- **And** `db.get_ref("refs/heads/feature/behind").target == "<sha-C2>"`.
Priority: P0.

---

**E2E-NEW-434 — Happy — resetting the checked-out branch hard-resets the volume**

**Category:** Happy. **Scenario:** SC-924. **Requirements:** FR-NEW-111, FR-NEW-112.
Preconditions: SEED-A, HEAD on `main` = `<sha-C2>`, volume clean.
- **When** `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}`.
- **Then** the response has `"checked_out": true`, `"old_sha":"<sha-C2>"`, `"new_sha":"<sha-C1>"`, `"files_changed":1`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C1>"` and `db.get_ref("HEAD").target == "refs/heads/main"` (HEAD stays symbolic on the same branch).
- **And** `git.status` reports `"head":"<sha-C1>"`, `"branch":"main"`.
Priority: P0.

---

**E2E-NEW-435 — SideEffect — the hard reset of E2E-NEW-434 removed the file and was accounted**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-112., FR-NEW-189
Preconditions: E2E-NEW-434 run to success, with `before = safety.bytes_written(OWNER, MOUNT)` captured first.
- **Then** `client.exists("/src/lib.rs")` is `false` and `env.read("/README.md") == "alpha\n"`.
- **And** `safety.bytes_written(OWNER, MOUNT) - before == 0` (only a delete, no blob bytes written, per `charge_pull_quota` semantics).
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `op == "git.branch_reset"` entry with `path == "/"` and `detail.contains("files_changed 1")`.
Priority: P0.

---

**E2E-NEW-436 — Failure — an unknown target commit**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-114.
Preconditions: SEED-A.
- **When** `git.branch_reset {"name":"main","target_commit":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef","force":true}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("deadbeef")`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (no volume rewrite was attempted).
Priority: P1.

---

**E2E-NEW-437 — Failure — resetting an unknown branch**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-114.
Preconditions: SEED-A.
- **When** `git.branch_reset {"name":"feature/ghost","target_commit":"<sha-C1>","force":true}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("branch 'feature/ghost'")`.
- **And** `db.list_refs()` contains no `refs/heads/feature/ghost` row (a reset never creates a branch).
Priority: P1.

---

**E2E-NEW-438 — Failure — resetting the checked-out branch with a dirty volume**

**Category:** Failure. **Scenario:** SC-924. **Requirements:** FR-NEW-112, FR-NEW-106.
Preconditions: SEED-A, then `env.write("/README.md","alpha DIRTY\n")`.
- **When** `git.branch_reset {"name":"main","target_commit":"<sha-C1>"}` (force omitted).
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("uncommitted changes")`.
- **And** `env.read("/README.md") == "alpha DIRTY\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
Note: the dirty guard is checked before the fast-forward guard, so the message must be the dirty one; the test asserts `!message.contains("not a fast-forward")` to pin the ordering.
Priority: P0.

---

**E2E-NEW-439 — SideEffect — the rejected resets of E2E-NEW-432 and E2E-NEW-438 left the refs untouched**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-113, FR-NEW-114.
Preconditions: run both calls in a single test built on SEED-B plus a dirty `/README.md`.
- **Then** `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"` and `db.get_ref("refs/heads/main").target == "<sha-C2>"`.
- **And** `safety.audit(OWNER, MOUNT)` contains no `op == "git.branch_reset"` entry.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before both calls.
Priority: P0.

---

**E2E-NEW-440 — Concurrency — reset races a commit on the same branch**

**Category:** Concurrency. **Scenario:** SC-924. **Requirements:** FR-NEW-115.
Preconditions: SEED-A, volume clean; `env.write("/new.txt","n\n")` staged in the volume so the commit has something to record.
- **When** `tokio::join!` of `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}` and `git.commit {"message":"c3"}`.
- **Then** both calls return `Ok` (the reset may also return `ERR_INVALID_ARGUMENT` "uncommitted changes" if it runs first and observes `/new.txt`; the test accepts exactly these two shapes and asserts nothing else).
- **And** `db.get_ref("refs/heads/main").target` is exactly one of `<sha-C1>` or the sha returned by `git.commit`, never an interleaved third value.
- **And** if the commit won the race, `git.log {"ref_name":"main"}`'s first commit sha equals the commit response's `commit_sha`; if the reset won, `git.log`'s first sha is `<sha-C1>`.
Verification: `git.commit` holds `entry.write_lock` for its whole body (`crates/mcp-fs/src/tools/git.rs:659`); `branch_reset` must hold the same lock.
Priority: P1.

---

**E2E-NEW-441 — Security — a non-member cannot reset or delete**

**Category:** Security. **Scenario:** SC-924. **Requirements:** FR-NEW-100.
Preconditions: SEED-B.
- **When** `env.as_person("stranger@test.com", ...)` calls `git.branch_reset {"name":"main","target_commit":"<sha-C1>","force":true}` and `git.branch_delete {"name":"release/1.0","force":true}`.
- **Then** both error with `code == ERR_FORBIDDEN`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("refs/heads/release/1.0").target == "<sha-C3>"`.
- **And** `safety.audit("stranger@test.com", MOUNT)` is empty.
Priority: P0.

---

**E2E-NEW-442 — Happy — `git.branches` marks exactly one current branch**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-B, HEAD on `main`.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the `"branches"` array has 2 entries; the entry with `"name":"main"` has `"current": true`, the entry with `"name":"release/1.0"` has `"current": false`.
- **And** `branches.iter().filter(|b| b["current"] == true).count() == 1`.
- **And** after `git.branch_switch {"name":"release/1.0"}`, the same call reports `"current":true` on `release/1.0` and `false` on `main`.
Priority: P0.

---

**E2E-NEW-443 — Happy — ahead/behind vs the remote-tracking ref**

**Category:** Happy. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-A, then seed the tracking ref and divergence directly through the db and real commits:
`db.set_ref("refs/remotes/origin/main", "<sha-C1>", false)`; then two more local commits: write `/a.txt`=`"a\n"`, commit "c3" -> `<sha-C3>`; write `/b.txt`=`"b\n"`, commit "c4" -> `<sha-C4>`. `main` = `<sha-C4>`, tracking = `<sha-C1>`, and `<sha-C2>`,`<sha-C3>`,`<sha-C4>` are the three commits ahead. For the "behind" side, build the tracking ref from a sibling branch instead: create `feature/upstream` from `<sha-C1>`, commit `/u.txt`=`"u\n"` -> `<sha-U2>`, then `db.set_ref("refs/remotes/origin/main","<sha-U2>",false)`.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the `main` entry has `"upstream":"refs/remotes/origin/main"`, `"ahead": 3`, `"behind": 1` (commits `<sha-C2>`,`<sha-C3>`,`<sha-C4>` are on `main` only; `<sha-U2>` is on the tracking ref only; `<sha-C1>` is the merge base).
Verification: tool response fields, cross-checked against `git.log {"ref_name":"main"}` length (4 commits including `<sha-C1>`).
Priority: P0.

---

**E2E-NEW-444 — EdgeCase — a branch with no upstream reports null, not zero**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: SEED-B, no `refs/remotes/*` rows at all (`db.list_refs()` asserted to contain none).
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** both entries have `"upstream": null`, `"ahead": null`, `"behind": null` (JSON `null`, asserted with `v["ahead"].is_null()`, explicitly NOT `0`).
Rationale: `0/0` would mean "in sync", which is a materially different statement from "no upstream".
Priority: P0.

---

**E2E-NEW-445 — EdgeCase — `git.branches` on a repo with no commits**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-MOD-106.
Preconditions: `git.init` only.
- **When** `git.branches {"mount_id":"gitproj"}`.
- **Then** the response is `{"mount_id":"gitproj","branches":[]}` (an empty array, not an error).
- **And** `git.status` returns `"branch":"main"` with `"head": null` (existing behaviour, `status`, `crates/mcp-fs/src/tools/git.rs:512-538`), proving the current-branch marker has nothing to mark.
Priority: P1.

---

##### B) Stash

---

**E2E-NEW-446 — Happy — `stash_save` creates an entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: SEED-A, then `env.write("/src/lib.rs","fn a() { 1 }\n")` and `env.write("/notes.txt","draft\n")` (one modification, one addition, both uncommitted).
- **When** `git.stash_save {"mount_id":"gitproj","message":"wip: login"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","message":"wip: login","branch":"main","files_stashed":2}` where `<sha-S1>` matches `^[0-9a-f]{40}$`.
- **And** `git.stash_list` returns one entry whose `"stash_id"` equals the response's `"stash_id"`.
Priority: P0.

---

**E2E-NEW-447 — SideEffect — `stash_save` reverts the volume to HEAD**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: E2E-NEW-446 run to success.
- **Then** `env.read("/src/lib.rs") == "fn a() {}\n"` (reverted to the `<sha-C2>` content).
- **And** `client.exists("/notes.txt")` is `false` (the untracked-in-HEAD addition is removed).
- **And** `env.read("/README.md") == "alpha\n"`.
- **And** a follow-up `git.stash_save {"message":"second"}` errors with `ERR_INVALID_ARGUMENT` "nothing to stash", proving the volume is now provably clean against HEAD (the same `require_clean_volume` comparison, `crates/mcp-fs/src/tools/git.rs:1991-2009`).
Priority: P0.

---

**E2E-NEW-448 — SideEffect — the stash lives in `git_refs` under `refs/stash/`**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-123., FR-NEW-131
Preconditions: E2E-NEW-446 run to success, `<sha-S1>` captured.
- **Then** `db.get_ref(&format!("refs/stash/{sha_s1}"))` returns `Some(GitRefRow { target: "<sha-S1>", symbolic: false })` (direct `git_refs` row check, `crates/mcp-fs/src/git/db.rs:214`).
- **And** `db.list_refs()` contains exactly one name starting with `"refs/stash/"`.
- **And** `db.get_object("<sha-S1>")` returns `Some(row)` with `row.kind == "commit"` (the snapshot is a real object, not a dangling ref).
Priority: P0.

---

**E2E-NEW-449 — Failure — `stash_save` on a clean volume**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-121.
Preconditions: SEED-A, volume clean (no writes after `git.commit "c2"`).
- **When** `git.stash_save {"message":"nothing here"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("nothing to stash")` and `message.contains("no uncommitted changes")`.
- **And** `db.list_refs()` contains no `refs/stash/` name.
- **And** the error is explicitly distinguishable from the dirty-volume refusal: `assert!(!message.contains("uncommitted changes: commit or discard"))`.
Priority: P0.

---

**E2E-NEW-450 — Failure — `stash_save` in a repo with no commits**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-121.
Preconditions: `git.init` only, then `env.write("/scratch.txt","s\n")`.
- **When** `git.stash_save {"message":"early"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("HEAD")` and `message.contains("no commits")`.
- **And** `env.read("/scratch.txt") == "s\n"` (the caller's file is NOT reverted away by a failed save).
- **And** `db.list_refs()` contains no `refs/stash/` name.
Priority: P1.

---

**E2E-NEW-451 — SideEffect — `stash_save` charges quota and audits**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-120.
Preconditions: SEED-A, `env.write("/src/lib.rs","fn a() { 1 }\n")` (13 bytes), `before = safety.bytes_written(OWNER, MOUNT)`.
- **When** `git.stash_save {"message":"wip"}`.
- **Then** `safety.bytes_written(OWNER, MOUNT) - before == 10` (the revert rewrites `/src/lib.rs` back to `"fn a() {}\n"`, 10 bytes; only written blobs are charged).
- **And** `safety.audit(OWNER, MOUNT)` ends with exactly one entry where `op == "git.stash_save"`, `path == "/"`, `detail.contains("wip")` and `detail.contains("files_stashed 1")`.
Priority: P1.

---

**E2E-NEW-452 — EdgeCase — unicode and very long stash messages**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-122.
Preconditions: SEED-A, `env.write("/notes.txt","x\n")`. `let msg = format!("réunion 日本 🚀 {}", "m".repeat(980));` (asserted `msg.chars().count() == 1000`).
- **When** `git.stash_save {"message": msg}`.
- **Then** the response `"message"` equals `msg` byte-for-byte.
- **And** `git.stash_list`'s single entry has `"message"` equal to `msg` byte-for-byte (no truncation, no lossy conversion).
- **And** a `git.stash_save` with `message` omitted entirely (a second stash after another write) yields `"message"` equal to the default `"WIP on main"`.
Priority: P2.

---

**E2E-NEW-453 — Happy — `stash_list` is newest first with the full field shape**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-122., FR-NEW-131
Preconditions: SEED-A; `env.write("/a.txt","1\n")`; `stash_save {"message":"first"}` -> `<sha-S1>`; `env.write("/b.txt","2\n")`; `stash_save {"message":"second"}` -> `<sha-S2>`.
- **When** `git.stash_list {"mount_id":"gitproj"}`.
- **Then** `entries.len() == 2`, `entries[0]["stash_id"] == "<sha-S2>"`, `entries[0]["message"] == "second"`, `entries[1]["stash_id"] == "<sha-S1>"`, `entries[1]["message"] == "first"`.
- **And** every entry has the keys exactly `["stash_id","message","base_sha","branch","created_at"]` with `base_sha` equal to the HEAD sha at save time and `"branch" == "main"` and `created_at` an integer `> 0`.
Priority: P0.

---

**E2E-NEW-454 — EdgeCase — `stash_list` on an empty pool**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-122., FR-NEW-131
Preconditions: SEED-A, no stash ever saved.
- **When** `git.stash_list {"mount_id":"gitproj"}`.
- **Then** the response is exactly `{"mount_id":"gitproj","stashes":[]}` and the call is `Ok`, not `ERR_NOT_FOUND`.
Priority: P0.

---

**E2E-NEW-455 — Happy — `stash_apply` restores and keeps the entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-124., FR-NEW-131
Preconditions: E2E-NEW-446 state (stash `<sha-S1>` holds the `/src/lib.rs` modification and `/notes.txt` addition; volume clean at `<sha-C2>`).
- **When** `git.stash_apply {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","status":"applied","files_changed":2,"dropped":false}`.
- **And** `env.read("/src/lib.rs") == "fn a() { 1 }\n"` and `env.read("/notes.txt") == "draft\n"`.
- **And** `git.stash_list` still returns one entry with `"stash_id":"<sha-S1>"`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some`.
Priority: P0.

---

**E2E-NEW-456 — Happy — `stash_pop` restores and removes the entry**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-125.
Preconditions: E2E-NEW-446 state.
- **When** `git.stash_pop {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","status":"applied","files_changed":2,"dropped":true}`.
- **And** `env.read("/src/lib.rs") == "fn a() { 1 }\n"` and `env.read("/notes.txt") == "draft\n"`.
- **And** `git.stash_list` returns `"stashes": []`.
Priority: P0.

---

**E2E-NEW-457 — SideEffect — pop deletes the `refs/stash/` row**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-125.
Preconditions: E2E-NEW-456 run to success.
- **Then** `db.get_ref("refs/stash/<sha-S1>")` is `None` (direct `git_refs` check).
- **And** `db.list_refs()` contains no name starting with `"refs/stash/"`.
- **And** `db.get_object("<sha-S1>")` is still `Some` (the ref is dropped, the object is not deleted; delete is a GC concern, not this tool's).
- **And** `safety.audit(OWNER, MOUNT)` ends with one entry `op == "git.stash_pop"`, `detail.contains("<sha-S1>")` and `detail.contains("dropped true")`.
Priority: P0.

---

**E2E-NEW-458 — Failure — `stash_apply` with an unknown id**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-446 state (one real stash present).
- **When** `git.stash_apply {"stash_id":"0123456789abcdef0123456789abcdef01234567"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("0123456789abcdef0123456789abcdef01234567")` and `message.contains("stash")`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` (the volume is untouched).
- **And** `git.stash_list` still returns exactly the one real entry.
Priority: P0.

---

**E2E-NEW-459 — Failure — `stash_pop` with an unknown id leaves the pool intact**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-453 state (two stashes `<sha-S1>`, `<sha-S2>`).
- **When** `git.stash_pop {"stash_id":"ffffffffffffffffffffffffffffffffffffffff"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("ffffffff")`.
- **And** `git.stash_list` still returns 2 entries with ids `<sha-S2>`, `<sha-S1>` in that order.
- **And** `db.list_refs()` still contains both `refs/stash/<sha-S1>` and `refs/stash/<sha-S2>`.
Priority: P0.

---

**E2E-NEW-460 — Failure — `stash_drop` with an unknown id**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-128.
Preconditions: E2E-NEW-446 state.
- **When** `git.stash_drop {"stash_id":"not-a-sha"}` and again with `"ffffffffffffffffffffffffffffffffffffffff"`.
- **Then** the first errors `code == ERR_INVALID_ARGUMENT` with `message.contains("not-a-sha")` and `message.contains("40")` (malformed id); the second errors `code == ERR_NOT_FOUND`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some`.
Priority: P1.

---

**E2E-NEW-461 — Happy — `stash_drop` removes the entry without touching the volume**

**Category:** Happy. **Scenario:** SC-902. **Requirements:** FR-NEW-127.
Preconditions: E2E-NEW-446 state (volume clean at `<sha-C2>`).
- **When** `git.stash_drop {"mount_id":"gitproj","stash_id":"<sha-S1>"}`.
- **Then** the response is `{"stash_id":"<sha-S1>","dropped":true}`.
- **And** `git.stash_list` returns `"stashes": []` and `db.get_ref("refs/stash/<sha-S1>")` is `None`.
- **And** `env.read("/src/lib.rs") == "fn a() {}\n"` and `client.exists("/notes.txt")` is `false` (drop never restores anything).
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before the drop.
Priority: P0.

---

**E2E-NEW-462 — Failure — a conflicting `stash_apply` returns `status:"conflict"` and writes nothing**

**Category:** Failure. **Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-170.
Preconditions: SEED-A; `env.write("/src/lib.rs","fn a() { STASHED }\n")`; `stash_save {"message":"conflicting"}` -> `<sha-S1>` (volume reverted to `"fn a() {}\n"`); then `env.write("/src/lib.rs","fn a() { LOCAL }\n")` and `git.commit "c3"` -> `<sha-C3>` so the same path diverged on both sides.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}`.
- **Then** the call returns `Ok` (NOT an `ERR_*`) with `"status" == "conflict"` and `"conflicts"` holding exactly one element whose `path` is `/src/lib.rs` and `"files_changed" == 0`.
- **And** `env.read("/src/lib.rs") == "fn a() { LOCAL }\n"` (no conflict marker, no partial write; the same refusal discipline as `crates/mcp-fs/src/tools/git.rs:1844-1851`).
- **And** `assert!(!env.read("/src/lib.rs").contains("<<<<<<<"))`.
- **And** `git.stash_list` still contains `<sha-S1>`.
Priority: P0.

---

**E2E-NEW-463 — DataIntegrity — a conflicting `stash_pop` does not drop the entry**

**Category:** DataIntegrity. **Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-171.
Preconditions: identical to E2E-NEW-462.
- **When** `git.stash_pop {"stash_id":"<sha-S1>"}`.
- **Then** the response has `"status" == "conflict"` and `"dropped" == false`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some(GitRefRow { target: "<sha-S1>", .. })` (the stashed work is not lost).
- **And** `git.stash_list` returns exactly one entry, id `<sha-S1>`.
- **And** `env.read("/src/lib.rs") == "fn a() { LOCAL }\n"`.
- **And** `safety.bytes_written(OWNER, MOUNT)` equals the value captured before the pop.
Priority: P0.

---

**E2E-NEW-464 — EdgeCase — the stash pool survives deletion of its origin branch**

**Category:** EdgeCase. **Scenario:** SC-930. **Requirements:** FR-NEW-129.
Preconditions: SEED-A; `git.branch_create {"name":"feature/temp","start_point":"<sha-C2>","checkout":true}`; `env.write("/tmp.txt","t\n")`; `git.stash_save {"message":"on temp"}` -> `<sha-S1>` (entry records `"branch":"feature/temp"`); `git.branch_switch {"name":"main"}`.
- **When** `git.branch_delete {"name":"feature/temp","force":true}`, then `git.stash_list`.
- **Then** the delete succeeds and `db.get_ref("refs/heads/feature/temp")` is `None`.
- **And** `git.stash_list` still returns exactly one entry with `"stash_id":"<sha-S1>"` and `"branch":"feature/temp"` (the recorded origin name survives as data even though the branch is gone).
- **And** `db.get_ref("refs/stash/<sha-S1>")` is `Some`.
- **And** `git.stash_pop {"stash_id":"<sha-S1>"}` then succeeds with `"status":"applied"` and `env.read("/tmp.txt") == "t\n"`.
Priority: P0.

---

**E2E-NEW-465 — EdgeCase — a stash taken on one branch applies on another**

**Category:** EdgeCase. **Scenario:** SC-930. **Requirements:** FR-NEW-129.
Preconditions: SEED-B (branches `main` and `release/1.0`), HEAD on `main`; `env.write("/shared.txt","s\n")`; `git.stash_save {"message":"cross"}` -> `<sha-S1>`; `git.branch_switch {"name":"release/1.0"}`.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}`.
- **Then** the response has `"status":"applied"` and `"files_changed":1`.
- **And** `env.read("/shared.txt") == "s\n"` while `git.status` still reports `"branch":"release/1.0"`.
- **And** `env.read("/docs/rel.md") == "rel\n"` (the target branch's own content is preserved).
Priority: P1.

---

**E2E-NEW-466 — Failure — `stash_apply` under an exhausted quota**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-124, FR-NEW-185.
Preconditions: `Env::with_quota(40)`; SEED-A built through `env.write` (`"alpha\n"` 6 + `"fn a() {}\n"` 10 = 16 charged); `env.write("/big.txt", &"x".repeat(20))` (20 more -> 36 charged); `git.stash_save {"message":"big"}` -> `<sha-S1>` (the revert deletes `/big.txt`, charging 0, so 36 stays). Remaining headroom is 4 bytes, asserted in-test via `assert_eq!(safety.bytes_written(OWNER, MOUNT), 36)`.
- **When** `git.stash_apply {"stash_id":"<sha-S1>"}` (it must write 20 bytes).
- **Then** `code == ERR_WRITE_QUOTA_EXCEEDED` and `message.contains("session write quota of 40 bytes exceeded")`.
- **And** `client.exists("/big.txt")` is `false` (nothing partially applied).
- **And** `git.stash_list` still contains `<sha-S1>` and `db.get_ref("refs/stash/<sha-S1>")` is `Some`.
- **And** `safety.bytes_written(OWNER, MOUNT) == 36` (a rejected charge consumes nothing).
Priority: P0.

---

**E2E-NEW-467 — EdgeCase — the stash pool cap boundary**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-130.
Preconditions: SEED-A. Loop `i in 0..100`: `env.write(&format!("/s{i}.txt"), &format!("{i}\n"))` then `git.stash_save {"message": format!("s{i}")}`, each asserted `Ok`.
- **Given** `git.stash_list` returns exactly 100 entries.
- **When** `env.write("/s100.txt","100\n")` then `git.stash_save {"message":"s100"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("100")` and `message.contains("stash")` and `message.contains("drop")`.
- **And** `git.stash_list` still returns exactly 100 entries (the oldest was NOT silently evicted, unlike the audit ring at `crates/mcp-fs/src/safety.rs:145-152`).
- **And** `env.read("/s100.txt") == "100\n"` (the refused save did not revert the caller's work).
Priority: P2.

---

**E2E-NEW-468 — Security — the stash family is membership gated**

**Category:** Security. **Scenario:** SC-902. **Requirements:** FR-NEW-100.
Preconditions: E2E-NEW-446 state.
- **When** `env.as_person("stranger@test.com", ...)` calls each of `git.stash_save {"message":"x"}`, `git.stash_list {}`, `git.stash_apply {"stash_id":"<sha-S1>"}`, `git.stash_pop {"stash_id":"<sha-S1>"}`, `git.stash_drop {"stash_id":"<sha-S1>"}`.
- **Then** all five error with `code == ERR_FORBIDDEN` and `message.contains("stranger@test.com")`.
- **And** `db.get_ref("refs/stash/<sha-S1>")` is still `Some` and `env.read("/src/lib.rs") == "fn a() {}\n"`.
- **And** the same five calls as `ADMIN` also return `ERR_FORBIDDEN`.
- **And** a call with an empty `person` returns `ERR_UNAUTHENTICATED` (driven through the fixture's unauthenticated path, matching `crate::identity`).
Priority: P0.

---

**E2E-NEW-469 — Concurrency — two `stash_save` calls race**

**Category:** Concurrency. **Scenario:** SC-902. **Requirements:** FR-NEW-120, FR-NEW-115.
Preconditions: SEED-A; `env.write("/p.txt","p\n")` and `env.write("/q.txt","q\n")` (both uncommitted).
- **When** `tokio::join!` of `git.stash_save {"message":"A"}` and `git.stash_save {"message":"B"}`.
- **Then** exactly one of two accepted shapes holds, asserted explicitly: either both succeed with two distinct `stash_id` values, or one succeeds and the other errors `ERR_INVALID_ARGUMENT` "nothing to stash" (the loser observed the already-reverted volume).
- **And** `git.stash_list`'s entry count equals the number of successful calls.
- **And** every listed `stash_id` has a matching `refs/stash/{id}` row, and the ids are pairwise distinct.
- **And** the final volume matches `<sha-C2>` exactly: `client.exists("/p.txt") == false` and `client.exists("/q.txt") == false`.
Priority: P1.

---

**E2E-NEW-470 — DataIntegrity — stash refs never leak into branch or tag listings**

**Category:** DataIntegrity. **Scenario:** SC-902. **Requirements:** FR-NEW-123.
Preconditions: E2E-NEW-453 state (two stashes).
- **When** `git.branches`, `git.tags`, and `git.status` are called.
- **Then** `git.branches`'s `"branches"` contains no entry whose `"full_ref"` starts with `"refs/stash/"` (it has exactly one entry, `main`).
- **And** `git.tags`'s `"tags"` is `[]`.
- **And** `git.status`'s `"refs"` DOES contain both `refs/stash/<sha-S1>` and `refs/stash/<sha-S2>` (status lists every non-symbolic ref, `crates/mcp-fs/src/tools/git.rs:530-534`; this test pins that the stash is visible there and nowhere else).
Priority: P1.

---

##### C) Remotes

---

**E2E-NEW-471 — Happy — `remote_add` then `remote_list`**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-140, FR-NEW-144.
Preconditions: SEED-A, plus `db.add_remote("origin","https://github.com/o/r.git")`.
- **When** `git.remote_add {"mount_id":"gitproj","name":"upstream","url":"https://github.ibm.com/team/r.git"}`.
- **Then** the response is `{"name":"upstream","url":"https://github.ibm.com/team/r.git"}`.
- **And** `db.list_remotes()` == `[("origin","https://github.com/o/r.git"),("upstream","https://github.ibm.com/team/r.git")]` (ordered by name, per `crates/mcp-fs/src/git/db.rs:298-308`).
- **And** `git.remote_list {"mount_id":"gitproj"}` returns `"remotes"` as exactly those two `{name,url}` objects in that order.
- **And** `safety.audit(OWNER, MOUNT)` ends with one `op == "git.remote_add"` entry whose `detail.contains("upstream")`.
Priority: P0.

---

**E2E-NEW-472 — Failure — a duplicate remote name is refused, not upserted**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-141.
Preconditions: SEED-A, `db.add_remote("origin","https://github.com/o/r.git")`.
- **When** `git.remote_add {"name":"origin","url":"https://evil.test/x/y.git"}`.
- **Then** `code == ERR_NO_CLOBBER` and `message.contains("remote 'origin' already exists")`.
- **And** `db.list_remotes()` == `[("origin","https://github.com/o/r.git")]` — the URL was NOT overwritten.
Rationale: `RelationalGitDb::add_remote` is an upsert (`crates/mcp-fs/src/git/db.rs:281-285`), so the duplicate check MUST happen in the tool layer above it; this test is the guard against calling the upsert blindly.
Priority: P0.

---

**E2E-NEW-473 — Failure — non-HTTPS URLs are refused, naming the scheme**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140.
Preconditions: SEED-A, no remotes.
- **When** `git.remote_add {"name":"r1","url":"ssh://git@github.com/o/r.git"}`, then `{"name":"r2","url":"git://github.com/o/r.git"}`, `{"name":"r3","url":"http://github.com/o/r.git"}`, `{"name":"r4","url":"file:///tmp/bare"}`, `{"name":"r5","url":"git@github.com:o/r.git"}`.
- **Then** r1..r4 each error with `code == ERR_INVALID_ARGUMENT` and `message.contains("only https is accepted")`, each naming its own scheme (`"ssh"`, `"git"`, `"http"`, `"file"` respectively) — the exact wording of `validate_remote_url` (`crates/mcp-fs/src/git/remote.rs:256-261`).
- **And** r5 errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("scp-style shorthand")` (`crates/mcp-fs/src/git/remote.rs:248-251`).
- **And** `db.list_remotes()` is empty.
Priority: P0.

---

**E2E-NEW-474 — Failure — a URL carrying userinfo is refused without echoing the secret**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140, FR-NEW-145.
Preconditions: SEED-A, no remotes.
- **When** `git.remote_add {"name":"leaky","url":"https://alice:ghp_secret@github.com/o/r.git"}`.
- **Then** `code == ERR_INVALID_ARGUMENT`.
- **And** `assert!(!e.message.contains("ghp_secret"))` and `assert!(!e.message.contains("alice:ghp_secret"))` (same property already proven for clone at `crates/mcp-fs/src/git/remote.rs:1069-1076`).
- **And** `db.list_remotes()` is empty.
- **And** `safety.audit(OWNER, MOUNT)` contains no entry whose `detail.contains("ghp_secret")`.
Priority: P0.

---

**E2E-NEW-475 — SideEffect — the rejected adds of E2E-NEW-472/E2E-NEW-473 wrote no row**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-141, FR-NEW-140.
Preconditions: run E2E-NEW-472's and all five of E2E-NEW-473's calls in one test, starting from `db.add_remote("origin","https://github.com/o/r.git")`.
- **Then** `db.list_remotes()` == `[("origin","https://github.com/o/r.git")]` exactly (direct `git_remotes` check: one row, original URL).
- **And** `git.remote_list` returns exactly one entry.
Priority: P0.

---

**E2E-NEW-476 — Failure — an invalid remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-140.
Preconditions: SEED-A.
- **When** `git.remote_add` with `name` in `["", "has space", "with/slash", "-leading", "a".repeat(256)]` and a valid `url` `"https://github.com/o/r.git"`.
- **Then** each errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("is not a valid remote name")`.
- **And** `db.list_remotes()` is empty.
Priority: P1.

---

**E2E-NEW-477 — Happy — `remote_remove` deletes the row**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-142.
Preconditions: SEED-R2 (`origin` and `upstream` rows present).
- **When** `git.remote_remove {"mount_id":"gitproj","name":"upstream"}`.
- **Then** the response is `{"name":"upstream","removed":true}`.
- **And** `db.list_remotes()` == `[("origin", <url>)]` (the `upstream` row is gone, `origin` untouched).
- **And** `safety.audit(OWNER, MOUNT)` ends with one `op == "git.remote_remove"` entry whose `detail.contains("upstream")`.
Priority: P0.

---

**E2E-NEW-478 — Failure — removing an unknown remote is not a silent no-op**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-143.
Preconditions: SEED-R (`origin` only).
- **When** `git.remote_remove {"name":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")`.
- **And** `db.list_remotes()` still == `[("origin", <url>)]`.
Rationale: `RelationalGitDb::remove_remote` is an unconditional DELETE (`crates/mcp-fs/src/git/db.rs:287-296`) and therefore silently succeeds on a missing name; the existence check MUST live in the tool.
Priority: P0.

---

**E2E-NEW-479 — EdgeCase — removing `origin` makes push report "no origin remote"**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-142, FR-NEW-146.
Preconditions: SEED-R.
- **When** `git.remote_remove {"name":"origin"}`, then `git.remote_push {"mount_id":"gitproj","branch":"main"}`.
- **Then** the remove succeeds and `db.list_remotes()` is empty.
- **And** the push errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("has no origin remote")` — the exact wording of `require_origin` (`crates/mcp-fs/src/git/remote.rs:448`).
- **And** `safety.audit(OWNER, MOUNT)` contains one `op == "git.remote_push"` entry with `detail.contains("outcome error")` (the early-failure path still audits exactly once, `crates/mcp-fs/src/tools/git.rs:446-470`).
Priority: P1.

---

**E2E-NEW-480 — EdgeCase — `remote_list` with no remotes**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-144.
Preconditions: SEED-A, `git.init` done, `db.list_remotes()` asserted empty.
- **When** `git.remote_list {"mount_id":"gitproj"}`.
- **Then** the response is exactly `{"mount_id":"gitproj","remotes":[]}` and the call is `Ok`.
Priority: P1.

---

**E2E-NEW-481 — Happy — fetching a named remote**

**Category:** Happy. **Scenario:** SC-907. **Requirements:** FR-NEW-147, FR-MOD-103.
Preconditions: SEED-R2. Advance the second bare repo: `let sha_u2 = advance_bare_remote(&remote_dir2, "refs/heads/main", "upstream2\n");`. Driven at the internal-function level (`fetch_branch_inner(store, MOUNT, &url2, None, "anonymous".into())`, extended with the remote name `"upstream"`), because `file://` cannot reach it through the tool (`crates/mcp-fs/src/git/remote.rs:256`).
- **When** the fetch runs against `upstream`.
- **Then** the response `"refs_updated"` contains `"refs/remotes/upstream/main"` and the response `"objects_fetched"` is `> 0`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-U2>"` (direct `git_refs` check).
Priority: P0.

---

**E2E-NEW-482 — SideEffect — a named fetch touches only that remote's namespace**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-147.
Preconditions: E2E-NEW-481, with `db.set_ref("refs/remotes/origin/main","<sha-R1>",false)` seeded before the fetch and `let before: Vec<GitRefRow> = db.list_refs()` captured.
- **Then** after the `upstream` fetch, `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` (unchanged).
- **And** `db.list_refs()` contains no name starting with `"refs/remotes/origin/"` other than `refs/remotes/origin/main`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (a fetch never moves a local branch).
- **And** the volume is unchanged: `env.read("/README.md") == "alpha\n"`, and `safety.bytes_written(OWNER, MOUNT)` equals its pre-fetch value (fetch charges no quota, `crates/mcp-fs/src/tools/git.rs:1332-1334`).
Priority: P0.

---

**E2E-NEW-483 — Failure — fetching an undeclared remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-146, FR-MOD-103.
Preconditions: SEED-R (`origin` only). Driven through the registered `git.remote_fetch` tool (no network is reached, so the https rule is irrelevant).
- **When** `git.remote_fetch {"mount_id":"gitproj","remote":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")` and `message.contains("gitproj")`.
- **And** `db.list_refs()` contains no name starting with `"refs/remotes/upstream/"`.
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `git.remote_fetch` entry, with `detail.contains("outcome error")` (one audit per call, never two, `crates/mcp-fs/src/git/remote.rs:334-371`).
Priority: P0.

---

**E2E-NEW-484 — Happy — pushing with `remote_branch` creates a differently named remote branch**

**Category:** Happy. **Scenario:** SC-908. **Requirements:** FR-MOD-102.
Preconditions: SEED-R (local `main` = `<sha-C2>`, bare remote `refs/heads/main` = `<sha-R1>`). Driven at the internal-function level through the `call_push_branch` helper pattern (`crates/mcp-fs/src/tools/git.rs:4326`), extended with `remote_branch`.
- **When** `push_branch_inner(..., branch="main", remote_branch=Some("sandbox"), ...)`.
- **Then** the response is `{"branch":"main","remote_branch":"sandbox","created":true,"up_to_date":false,"remote_sha":"<sha-C2>"}`.
- **And** bare-repo inspection: `Repository::open_bare(&remote_dir).find_reference("refs/heads/sandbox")` resolves to `<sha-C2>`.
- **And** `find_reference("refs/heads/main")` on the same bare repo still resolves to `<sha-R1>` (the remote's own `main` was not touched).
Priority: P0.

---

**E2E-NEW-485 — SideEffect — the tracking ref follows the remote-side name**

**Category:** SideEffect. **Scenario:** SC-908. **Requirements:** FR-MOD-102.
Preconditions: E2E-NEW-484 run to success.
- **Then** `db.get_ref("refs/remotes/origin/sandbox").target == "<sha-C2>"` (direct `git_refs` check; today's code writes `refs/remotes/origin/{branch}`, `crates/mcp-fs/src/tools/git.rs:1259`, which would be the wrong name here).
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` (no tracking ref was created under the local name).
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (the local branch is unmoved).
- **And** the volume is byte-identical: `env.read("/README.md") == "alpha\n"`, `env.read("/src/lib.rs") == "fn a() {}\n"`, and `safety.bytes_written(OWNER, MOUNT)` is unchanged (push charges no quota, `crates/mcp-fs/src/tools/git.rs:1155-1158`).
Priority: P0.

---

**E2E-NEW-486 — Failure — pushing to an undeclared remote name**

**Category:** Failure. **Scenario:** SC-907. **Requirements:** FR-NEW-146, FR-MOD-101.
Preconditions: SEED-R (`origin` only). Driven through the registered `git.remote_push` tool.
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","remote":"upstream"}`.
- **Then** `code == ERR_NOT_FOUND` and `message.contains("remote 'upstream'")`.
- **And** bare-repo inspection: `refs/heads/main` in `remote_dir` is still `<sha-R1>`, and `find_reference("refs/heads/upstream")` errors (no branch created anywhere).
- **And** `db.list_refs()` contains no `refs/remotes/upstream/` name.
Priority: P0.

---

**E2E-NEW-487 — EdgeCase — `remote` omitted defaults to `origin`**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-MOD-101.
Preconditions: SEED-R2 (both `origin` and `upstream` declared, pointing at two different bare repos). Driven at the internal-function level so the push really lands.
- **When** the push runs with `remote` absent from the arguments.
- **Then** bare-repo inspection of `remote_dir` (the `origin` repo) shows `refs/heads/main` == `<sha-C2>`.
- **And** bare-repo inspection of `remote_dir2` (the `upstream` repo) shows `refs/heads/main` still == `<sha-U1>` (untouched).
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-C2>"` and `db.get_ref("refs/remotes/upstream/main")` is `None`.
- **And** the same defaulting is asserted for `git.remote_fetch` and `git.remote_pull` with `remote` omitted, via their generated JSON schema: `resolve("git.remote_push").schema["properties"]["remote"]["default"] == "origin"` on all three (registry/schema check, the pattern of `crates/mcp-fs/src/tools/git.rs:2572-2613`).
Priority: P0.

---

**E2E-NEW-488 — Security — remote management is membership gated**

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

**E2E-NEW-489 — Failure — `force:true` without `expected_remote_sha` is refused outright**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Preconditions: SEED-R. Driven through the registered `git.remote_push` tool so the rejection is proven to happen before any credential resolution or network call; the URL is the https placeholder `https://github.com/o/r.git` stored as origin, and the test asserts the error is the lease one, not the scheme one.
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","force":true}`.
- **Then** `code == ERR_INVALID_ARGUMENT`, `message.contains("expected_remote_sha is required when force is true")`.
- **And** `assert!(!e.message.contains("only https is accepted"))` (proving the lease check runs ahead of URL validation, i.e. ahead of any network attempt).
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` (no tracking ref written).
Priority: P0.

---

**E2E-NEW-490 — Happy — a matching lease permits a non-fast-forward push**

**Category:** Happy. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Preconditions: SEED-R; `let sha_r2 = advance_bare_remote(&remote_dir, "refs/heads/main", "remote-only\n");` so the bare repo's `main` is `<sha-R2>` and the local `main` (`<sha-C2>`) is NOT a descendant of it. Driven at the internal-function level.
- **Given** bare-repo inspection confirms `refs/heads/main` == `<sha-R2>`.
- **When** the push runs with `force:true, expected_remote_sha:"<sha-R2>"`.
- **Then** the response is `{"branch":"main","created":false,"up_to_date":false,"forced":true,"remote_sha":"<sha-C2>","overwritten_sha":"<sha-R2>"}`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-C2>`.
Priority: P0.

---

**E2E-NEW-491 — SideEffect — the audit entry records the overwritten sha**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-159.
Preconditions: E2E-NEW-490 run to success.
- **Then** `safety.audit(OWNER, MOUNT)` contains exactly one entry with `op == "git.remote_push"`, `path == "/"`, whose `detail` satisfies all of: `contains("outcome ok")`, `contains("<sha-R2>")` (the overwritten sha), `contains("<sha-C2>")` (the new sha), `contains("forced true")`.
- **And** `assert!(!detail.contains("ghp_"))` and the detail contains no `"@"`-bearing URL (no credential leak, consistent with `crates/mcp-fs/src/git/remote.rs:326-333`).
- **And** exactly one `git.remote_push` entry exists, not two (the single-audit invariant of `run_remote_operation`).
Priority: P0.

---

**E2E-NEW-492 — Failure — a stale lease is rejected naming both shas**

**Category:** Failure. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: SEED-R; capture `<sha-R1>` (the remote tip the caller believes in); then `let sha_r2 = advance_bare_remote(&remote_dir, "refs/heads/main", "someone-else\n");` so the remote has moved to `<sha-R2>` behind the caller's back. Driven at the internal-function level.
- **When** the push runs with `force:true, expected_remote_sha:"<sha-R1>"`.
- **Then** `code == ERR_INVALID_ARGUMENT`.
- **And** `message.contains("<sha-R1>")` AND `message.contains("<sha-R2>")` AND `message.contains("expected")` AND `message.contains("actual")` (both shas, labelled).
- **And** `assert!(!message.starts_with("push refused: not a fast-forward"))` (the lease failure is a distinct identity from the FF refusal at `crates/mcp-fs/src/git/remote.rs:744-748`).
Priority: P0.

---

**E2E-NEW-493 — SideEffect — a rejected lease leaves the remote and the tracking ref untouched**

**Category:** SideEffect. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: E2E-NEW-492 run to its failure, with `db.set_ref("refs/remotes/origin/main","<sha-R1>",false)` seeded before the call.
- **Then** bare-repo inspection: `refs/heads/main` in `remote_dir` == `<sha-R2>`, byte-for-byte what the third party left (no partial ref update, no new ref).
- **And** the bare repo contains no reference whose target is `<sha-C2>`: iterate `repo.references()` and assert none resolves to `<sha-C2>`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` (unchanged: the tracking ref is only advanced after a successful push, `crates/mcp-fs/src/tools/git.rs:1257-1259`).
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and the volume is unchanged.
- **And** `safety.audit(OWNER, MOUNT)` contains exactly one `git.remote_push` entry with `detail.contains("outcome error")`.
Priority: P0.

---

**E2E-NEW-494 — Failure — `force:false` preserves today's fast-forward refusal verbatim**

**Category:** Failure. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
Preconditions: SEED-R; `advance_bare_remote(&remote_dir, "refs/heads/main", "extra\n")` -> `<sha-R2>` (the exact setup of the existing `e2e_new_076` test, `crates/mcp-fs/src/tools/git.rs:4445-4460`). Driven at the internal-function level.
- **When** the push runs with `force` omitted entirely, and again with `force:false`.
- **Then** both error with `code == ERR_INVALID_ARGUMENT` and `message == "push refused: not a fast-forward: branch 'main', force is not supported"` is NOT asserted verbatim (the tail changes once force exists); instead assert `message.starts_with("push refused: not a fast-forward")` and `message.contains("branch 'main'")`, matching the frozen prefix asserted at `crates/mcp-fs/src/git/remote.rs:1163-1165`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R2>`, unchanged.
- **And** `db.get_ref("refs/remotes/origin/main")` is `None`.
Note for the implementer: `non_fast_forward_error` (`crates/mcp-fs/src/git/remote.rs:744-748`) currently ends with `"force is not supported"`. That clause becomes false once force exists and MUST be replaced (suggested: `"pass force with expected_remote_sha to overwrite"`), which requires updating the two existing assertions at `crates/mcp-fs/src/git/remote.rs:1165` and `crates/mcp-fs/src/tools/git.rs:4455` — both only assert the prefix, so both keep passing.
Priority: P0.

---

**E2E-NEW-495 — EdgeCase — the "must not exist" lease, 40 zeros**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Preconditions: SEED-R; the bare repo has `refs/heads/main` == `<sha-R1>` but no `refs/heads/sandbox`. Driven at the internal-function level.
- **When** the push runs with `branch:"main", remote_branch:"sandbox", force:true, expected_remote_sha:"0000000000000000000000000000000000000000"`.
- **Then** the response is `{"created":true,"forced":true,"remote_sha":"<sha-C2>","overwritten_sha":null}`.
- **And** bare-repo inspection: `refs/heads/sandbox` == `<sha-C2>`.
- **And** a second run of the same call now errors `ERR_INVALID_ARGUMENT` with `message.contains("0000000000000000000000000000000000000000")` and `message.contains("<sha-C2>")`, because the branch now exists and the zero lease no longer matches.
- **And** after that second, rejected call, `refs/heads/sandbox` is still `<sha-C2>`.
Priority: P1.

---

**E2E-NEW-496 — Failure — a malformed `expected_remote_sha`**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Preconditions: SEED-R. Driven through the registered tool (no network reached).
- **When** `git.remote_push {"branch":"main","force":true,"expected_remote_sha": X}` for X in `["", "deadbeef", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", "<sha-C2> "]` (the last has a trailing space).
- **Then** each errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("expected_remote_sha")` and `message.contains("40")`.
- **And** for the `"deadbeef"` case, `assert!(!e.message.contains("only https is accepted"))` (validated before URL handling, so before the network).
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R1>` after all four calls.
Priority: P1.

---

**E2E-NEW-497 — Failure — a lease supplied without force**

**Category:** Failure. **Scenario:** SC-909. **Requirements:** FR-NEW-158.
Preconditions: SEED-R.
- **When** `git.remote_push {"branch":"main","force":false,"expected_remote_sha":"<sha-R1>"}`.
- **Then** `code == ERR_INVALID_ARGUMENT` and `message.contains("expected_remote_sha")` and `message.contains("force")`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R1>`, unchanged.
Rationale: silently ignoring a lease would let a caller believe it had protection it did not have.
Priority: P2.

---

**E2E-NEW-498 — Concurrency — the remote moves between the lease read and the push**

**Category:** Concurrency. **Scenario:** SC-910. **Requirements:** FR-NEW-157.
Preconditions: SEED-R; the local `main` = `<sha-C2>`; the bare repo's `main` = `<sha-R1>`. Driven at the internal-function level. The race is made deterministic, not timing-dependent: the test calls the push with `force:true, expected_remote_sha:"<sha-R1>"` while a second task mutates the bare repo with `advance_bare_remote(&remote_dir,"refs/heads/main","racer\n")` -> `<sha-R3>` before the push's ref-update phase. Determinism is obtained by running the advance first and only then issuing the push with the now-stale `<sha-R1>` lease (the same technique the existing racing-push test uses, `crates/mcp-fs/src/tools/git.rs:4557` `e2e_new_084_concurrent_pushes_serialize`, plus the note at `:4600`).
- **Then** the push errors with `code == ERR_INVALID_ARGUMENT` and `message.contains("<sha-R1>")` and `message.contains("<sha-R3>")`.
- **And** bare-repo inspection: `refs/heads/main` == `<sha-R3>` (the racer's commit survives intact; the losing push overwrote nothing).
- **And** the racer's file content is intact: cloning `remote_dir` into a temp dir and reading the file written by `advance_bare_remote` yields `"racer\n"`.
- **And** `db.get_ref("refs/remotes/origin/main")` is `None` or equals its pre-call value, never `<sha-C2>`.
- **And** a second part of the same test proves the two-writer case: `tokio::join!` of two force pushes carrying the same `expected_remote_sha:"<sha-R1>"` but different local tips (`<sha-C2>` on `main`, `<sha-C5>` on a second branch pushed to the same `remote_branch:"main"`) yields exactly one `Ok` and one `Err(ERR_INVALID_ARGUMENT)`, and the bare repo's `refs/heads/main` equals the winner's sha.
Priority: P0.

---

##### A.z Implementation notes (band A)

1. **`non_fast_forward_error`'s message tail becomes a lie** once `force` exists (`crates/mcp-fs/src/git/remote.rs:744-748` says `"force is not supported"`). Change it; both existing assertions check only the prefix (`crates/mcp-fs/src/git/remote.rs:1165`, `crates/mcp-fs/src/tools/git.rs:4455`), so they stay green.
2. **`add_remote` is an upsert and `remove_remote` is an unconditional DELETE** (`crates/mcp-fs/src/git/db.rs:281-296`). The duplicate check (E2E-NEW-472) and the existence check (E2E-NEW-478) must be implemented in the tool layer; reusing the db methods alone makes those two tests fail.
3. **`refs_under` filters by prefix** (`crates/mcp-fs/src/tools/git.rs:539-554`), so `refs/stash/*` will never leak into `git.branches`, but `status` lists every non-symbolic ref (`:530-534`) and therefore WILL show stash refs — test E2E-NEW-470 pins that asymmetry deliberately; do not "fix" it.
4. **Every new tool must be added to the golden contract** (`tool-contract-golden.json`, regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1`) and to the three per-tool lists in `crates/mcp-fs/src/tools/git.rs:2480-2493` and `:2644+`, plus the forbidden-for-non-member list at `:2685`, or those existing tests fail.
5. **Write-lock discipline**: E2E-NEW-421, E2E-NEW-440, E2E-NEW-469 and E2E-NEW-498 only hold if `branch_switch`, `branch_reset`, `stash_save/apply/pop` and the push path each acquire `entry.write_lock` in the async caller, never inside the blocking closure (the DRIFT-009 rule documented at `crates/mcp-fs/src/tools/git.rs:1216-1223`).

6. **Four contract constants are invented by this specification** (`MAX_BRANCH_NAME_BYTES=255`, `MAX_STASH_ENTRIES=100`, the `refs/stash/{sha}` id scheme, the `status:"conflict"` shape). They are not in the code today; tests E2E-NEW-406, E2E-NEW-407, E2E-NEW-467, E2E-NEW-448, E2E-NEW-462 and E2E-NEW-463 assert them. If the design owner picks different values, those six tests change and nothing else.

#### Band B. Shared conflict model, merge, squash, pull rework, in-progress guard

##### 0. Shared fixture definitions (referenced by ID below)

All tests use the existing `Env` harness (`git.rs:2437-2478`): `MOUNT = "proj"`, `OWNER = "owner@acme.test"`, helpers `write`, `read`, `commit`, `call`, `as_person`. Verification of volume bytes uses `env.read(path)` / `VolumeClient::read_bytes`; ref state uses `entry.db.get_ref(..)`; parent count uses `repo.find_commit(oid).parent_count()`; audit uses `state.safety.audit(OWNER, MOUNT)`; quota uses `state.safety.bytes_written(OWNER, MOUNT)`; the `git_operations` row uses a direct `RelationalDb` query scoped by `volume_id`.

**SEED-CONFLICT** (one overlapping text conflict):
1. `git.init`. Write `/src/config.toml`:
```
[server]
port = 8080
host = "0.0.0.0"
workers = 4
timeout = 30
retries = 2
```
`git.commit "base"` → `C0`.
2. `git.branch_create "feature"`, `git.branch_switch "feature"`; rewrite line 2 to `port = 9090`; `git.commit "feature port"` → `C1`.
3. `git.branch_switch "main"`; rewrite line 2 to `port = 8000`; `git.commit "main port"` → `C2`.
HEAD = `refs/heads/main` → `C2`.

**SEED-AUTOMERGE**: identical to SEED-CONFLICT except step 2 changes line 5 to `timeout = 60` (3 lines away from main's line-2 edit), so the 3-way merge succeeds with no caller decision.

**SEED-FF**: `C0` on main; feature = `C0` + one commit adding `/docs/readme.md` = `hello\n`; main still at `C0`.

**SEED-MULTI(n)**: `C0` commits `/f/000.txt … /f/{n-1}.txt`, each `line-A\n`; feature rewrites every one to `line-FEATURE\n`; main rewrites every one to `line-MAIN\n`. Used with n = 3 and n = 250.

**SEED-BINARY**: `C0` commits `/assets/logo.png` = bytes `89 50 4E 47 0D 0A 1A 0A 00 01`; feature sets the last byte to `0x02`; main sets it to `0x03`.

**SEED-UNICODE**: `C0` commits `/i18n/fr.txt` = `bonjour: café\nadieu: naïve\n`; feature line 1 → `bonjour: caffè 🇮🇹\n`; main line 1 → `bonjour: kafé ☕\n`.

**SEED-DELDEL**: `C0` commits `/tmp/scratch.txt` = `x\n` and `/keep.txt` = `k\n`; both branches delete `/tmp/scratch.txt`, and each additionally edits `/keep.txt` differently (`k-feature\n` vs `k-main\n`) so the merge is not trivially empty.

**SEED-DELMOD**: `C0` commits `/lib/util.rs` = `pub fn a() {}\n`; feature deletes it; main rewrites it to `pub fn a() { println!("x"); }\n`.

**SEED-TYPECHANGE**: `C0` commits `/mod` = `placeholder\n` (a file); feature replaces it with a directory containing `/mod/inner.rs` = `pub const N: u8 = 1;\n`; main edits `/mod` to `placeholder v2\n`.

**SEED-EMPTY**: `C0` commits `/flag` = `` (0 bytes, stores no blob per the content-addressing rule); feature writes `on\n`; main writes `off\n`.

**SEED-COLLIDE**: `C0` commits `/data/x.txt` = `1\n`; feature deletes `/data/x.txt` and adds `/data/x.txt/nested` — not representable in git, so instead: feature adds `/report` (file) `r\n`; main adds directory `/report/q.csv` = `a,b\n`. Merging surfaces a path collision the volume apply must refuse before writing anything.

---

##### 2. Full specification

Conventions used by every test: `mount_id` is always passed; `person` defaults to `OWNER`. "Exact error" means asserting `err.code == "<CODE>"` **and** `err.to_string().contains("<substring>")`.

The canonical conflict response shape asserted throughout (established by E2E-NEW-502 and reused):

```json
{
  "status": "conflict",
  "operation": "merge",
  "source_ref": "feature",
  "current_step": null, "total_steps": null,
  "conflicts": [
    {"path": "/src/config.toml",
     "ours":   {"exists": true, "content": "port = 8000\n"},
     "theirs": {"exists": true, "content": "port = 9090\n"},
     "base":   {"exists": true, "content": "port = 8080\n"},
     "binary": false, "type_change": false}
  ],
  "continue_with": "git.merge_resolve", "abort_with": "git.merge_abort"
}
```

---

##### E2E-NEW-500 — Auto-mergeable divergence completes automatically

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

##### E2E-NEW-501 — Merge of a strictly-ahead branch

**Scenario:** SC-903. **Requirements:** FR-NEW-193.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-FF.
**When** `git.merge {source_ref:"feature"}` from `main` at `C0`.
**Then** `status == "merged"`, `fast_forward == true`, `merge_commit` equals `feature`'s tip sha.
**And** `/docs/readme.md` reads exactly `hello\n`; `refs/heads/main` target equals the feature tip.
**And** the created commit count is unchanged: `git.log ref_name:"main"` returns exactly 2 entries.
**Verification:** `entry.db.get_ref("refs/heads/main")`, `git.log` length.
**Cleanup:** fixture drop.

##### E2E-NEW-502 — Conflict response shape

**Scenario:** SC-904. **Requirements:** FR-NEW-170., FR-NEW-186
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"feature"}`.
**Then** the response satisfies, field by field: `status == "conflict"`, `operation == "merge"`, `source_ref == "feature"`, `conflicts.len() == 1`, `conflicts[0].path == "/src/config.toml"`, `conflicts[0].ours.content` decodes to the SEED-CONFLICT main text (`port = 8000`), `conflicts[0].theirs.content` decodes to the feature text (`port = 9090`), `conflicts[0].base.content` decodes to the `C0` text (`port = 8080`), all three `exists == true`, `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`.
**And** `refs/heads/main` still targets `C2` exactly.
**And** `env.read("/src/config.toml")` still contains `port = 8000` and none of `<<<<<<<`, `=======`, `>>>>>>>`.
**Verification:** exact JSON equality against a constructed `serde_json::json!` literal for the non-content fields; decoded content string equality; ref read.
**Cleanup:** call `git.merge_abort`.

##### E2E-NEW-503 — Resolve by strategy "ours"

**Scenario:** SC-904. **Requirements:** FR-NEW-174, FR-NEW-196.
**Category:** Happy. **Priority:** P0. **Preconditions:** E2E-NEW-502 state (merge in progress, one conflict).
**When** `git.merge_resolve {mount_id:"proj", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
**Then** `status == "merged"`, `merge_commit` is 40-hex, `remaining_conflicts == []`.
**And** `/src/config.toml` line 2 is exactly `port = 8000` (main's side).
**And** `git_operations` count for this volume is 0.
**Verification:** JSON fields; file read split by `\n`; relational count.
**Cleanup:** fixture drop.

##### E2E-NEW-504 — Resolve by literal content

**Scenario:** SC-905. **Requirements:** FR-NEW-175, FR-NEW-196.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT + `git.merge feature` returning conflict.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"[server]\nport = 9000\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}]}`.
**Then** `status == "merged"`.
**And** `read_bytes("/src/config.toml")` equals that exact 64-byte string, not `port = 8000` and not `port = 9090` — proving caller content wins over both sides.
**And** the new commit's tree blob for that path has the same sha as a blob of the supplied bytes.
**Verification:** byte-vector equality; `repo.find_commit(merge_commit).tree().get_path("src/config.toml").id()` compared with `Oid::hash_object(ObjectType::Blob, bytes)`.
**Cleanup:** fixture drop.

##### E2E-NEW-505 — Mixed strategy and content in one call

**Scenario:** SC-905. **Requirements:** FR-NEW-176.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-MULTI(3) → conflicts on `/f/000.txt`, `/f/001.txt`, `/f/002.txt`.
**When** one `git.merge_resolve` with `[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"theirs"},{path:"/f/002.txt",content:"line-MANUAL\n"}]`.
**Then** `status == "merged"`, `remaining_conflicts == []`.
**And** `/f/000.txt` == `line-MAIN\n`; `/f/001.txt` == `line-FEATURE\n`; `/f/002.txt` == `line-MANUAL\n`.
**Verification:** three exact string reads.
**Cleanup:** fixture drop.

##### E2E-NEW-506 — Squash merge

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** Happy. **Priority:** P0.
**Preconditions:** SEED-FF extended: feature has two commits — `Cf1` adds `/docs/readme.md` = `hello\n`, `Cf2` adds `/docs/guide.md` = `guide\n`. main at `C0`.
**When** `git.merge {source_ref:"feature", squash:true}`.
**Then** `status == "merged"`, `squashed == true`, one `merge_commit` sha returned.
**And** both `/docs/readme.md` (`hello\n`) and `/docs/guide.md` (`guide\n`) are present in the volume with exactly those bytes.
**Verification:** two reads; `squashed` field.
**Cleanup:** fixture drop.

##### E2E-NEW-507 — merge_abort restores pre-merge state

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** Happy. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `head_before = refs/heads/main` target and `bytes_before = read_bytes("/src/config.toml")`; run `git.merge feature` → conflict.
**When** `git.merge_abort {mount_id:"proj"}`.
**Then** result `status == "aborted"`, `operation == "merge"`.
**And** `refs/heads/main` target equals `head_before` exactly; `HEAD` still symbolic to `refs/heads/main`.
**And** `read_bytes("/src/config.toml") == bytes_before`.
**And** `git_operations` count == 0.
**Verification:** sha string equality; byte-vector equality; relational count.
**Cleanup:** fixture drop.

##### E2E-NEW-508 — Conflicted pull finished by git.merge_resolve

**Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-MOD-104, FR-NEW-174.
**Category:** Happy. **Priority:** P0.
**Preconditions:** a local bare repo served over `file://` driven through `pull_branch` directly (the same technique the existing tests use, since `resolve_clone_credential` rejects `file://` at the tool boundary — `git.rs:1730-1740` doc comment). Remote `main` has `/src/config.toml` line 2 = `port = 9090`; local `main` has `port = 8000`; common base `port = 8080`.
**When** `git.remote_pull {mount_id:"proj", branch:"main"}` (no `on_conflict`).
**Then** `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`, `conflicts[0].path == "/src/config.toml"`.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"theirs"}]}` — the *same* tool, not a pull-specific one.
**Then** `status == "merged"`; `/src/config.toml` line 2 == `port = 9090`; the merge commit has exactly 2 parents, parent 0 = old local tip, parent 1 = fetched remote tip (mirrors `git.rs:1878-1887`).
**And** `refs/remotes/origin/main` was advanced by the fetch even though the apply was deferred (preserves `git.rs:1820-1823` behaviour).
**Verification:** `parent_count()`, `parent_id(0/1)`, ref reads, file read.
**Cleanup:** fixture drop.

##### E2E-NEW-509 — git.status reports the active operation

**Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-MOD-107., FR-NEW-187
**Category:** Happy. **Priority:** P1. **Preconditions:** SEED-MULTI(3), `git.merge feature` → conflict, then resolve only `/f/000.txt`.
**When** `git.status {mount_id:"proj"}`.
**Then** the response contains `operation` (the object named by FR-NEW-281) with exactly `{"op_type":"merge","source_ref":"feature","current_step":null,"total_steps":null,"remaining_conflicts":["/f/001.txt","/f/002.txt"],"continue_with":"git.merge_resolve","abort_with":"git.merge_abort"}`.
**And** the pre-existing `git.status` fields (`head`, `branch`, `refs`) are still present and unchanged relative to a `git.status` taken before the merge, except the added key — proving the schema is additive.
**Verification:** JSON subset equality both ways.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-510 — Merging an already-fully-merged branch is a no-op

**Scenario:** SC-903. **Requirements:** FR-NEW-192.
**Category:** Happy. **Priority:** P1. **Preconditions:** SEED-FF; run `git.merge feature` once (E2E-NEW-501).
**When** `git.merge {source_ref:"feature"}` a second time.
**Then** the call **succeeds** (no `Err`), `status == "already_up_to_date"`, `merge_commit` is `null`.
**And** `refs/heads/main` target is unchanged from after the first merge.
**And** `git.log "main"` length is unchanged (no empty commit created).
**Verification:** `Result::is_ok()`, field equality, ref sha equality, log length.
**Cleanup:** fixture drop.

##### E2E-NEW-511 — git.merge without mount_id

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"feature"}` (no `mount_id`).
**Then** `ERR_INVALID_ARGUMENT`, message contains `mount_id`.
**And** `git_operations` count == 0; `refs/heads/main` unchanged.
**Verification:** error code + substring; relational count.
**Cleanup:** none.

##### E2E-NEW-512 — Non-member cannot merge

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT; `stranger@acme.test` is not a member of `proj`.
**When** `env.as_person("stranger@acme.test","git.merge",{mount_id:"proj",source_ref:"feature"})`.
**Then** `ERR_FORBIDDEN`, message contains `stranger@acme.test` and `proj` (shape per `errors.rs:176`: `ERR_FORBIDDEN: 'a@b.c' is not a member of 'p'`).
**And** no `git_operations` row; the audit log for `stranger@acme.test` on `proj` contains no `git.merge` entry.
**Verification:** error code + two substrings; `safety.audit("stranger@acme.test","proj")` is empty.
**Cleanup:** none.

##### E2E-NEW-513 — Unknown mount

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0.
**When** `git.merge {mount_id:"does-not-exist", source_ref:"feature"}` as OWNER.
**Then** `ERR_PROJECT_NOT_FOUND`, message contains `does-not-exist`.
**Verification:** error code + substring.

##### E2E-NEW-514 — Unauthenticated caller

**Scenario:** SC-903. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P1.
**When** the merge tool is invoked with an empty/absent identity.
**Then** `ERR_UNAUTHENTICATED`.
**And** no `git_operations` row for `proj`.
**Verification:** error code; relational count.

##### E2E-NEW-515 — Unknown source_ref

**Scenario:** SC-903. **Requirements:** FR-NEW-195.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge {source_ref:"nope"}`.
**Then** `ERR_NOT_FOUND`, message contains `nope`.
**And** no fetch, no row, `refs/heads/main` unchanged at `C2`.
**Verification:** error code + substring; ref sha.

##### E2E-NEW-516 — Merging the checked-out branch into itself

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** SEED-CONFLICT, HEAD on `main`.
**When** `git.merge {source_ref:"main"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `main` and `itself`.
**And** no commit created (`git.log` length unchanged), no row.
**Verification:** error code + substrings; log length; relational count.

##### E2E-NEW-517 — Dirty volume refuses the merge

**Scenario:** SC-903. **Requirements:** FR-NEW-194.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, then `env.write("/src/config.toml", "[server]\nport = 1\n")` **without committing**.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `uncommitted changes` (same wording family as `git.rs:2006-2010`).
**And** `/src/config.toml` still reads exactly `[server]\nport = 1\n` — the dirty content was neither committed nor overwritten.
**And** no row, no commit.
**Verification:** error code + substring; exact file bytes; relational count; log length.

##### E2E-NEW-518 — merge_resolve naming a path that is not in conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-177.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT + conflict on `/src/config.toml`; `/keep.txt` exists and is not in conflict.
**When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `/keep.txt` and `not in conflict`.
**And** the merge is still in progress: `git.status` still reports `remaining_conflicts == ["/src/config.toml"]`; `git_operations` count == 1.
**And** no commit was created.
**Verification:** error code + substrings; `git.status` JSON; relational count; log length.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-519 — merge_resolve with nothing in progress

**Scenario:** SC-904. **Requirements:** FR-NEW-198.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, no merge started.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml",strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no operation in progress`.
**And** volume bytes and `refs/heads/main` unchanged.
**Verification:** error code + substring; byte compare; ref sha.

##### E2E-NEW-520 — merge_abort with nothing in progress

**Scenario:** SC-904. **Requirements:** FR-NEW-198., FR-NEW-186
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, no merge started.
**When** `git.merge_abort {mount_id:"proj"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no operation in progress`.
**And** `refs/heads/main` still `C2`; volume bytes unchanged.
**Verification:** error code + substring; ref + bytes.

##### E2E-NEW-521 — strategy and content both supplied for one path

**Scenario:** SC-905. **Requirements:** FR-NEW-176, FR-NEW-179.
**Category:** Failure. **Priority:** P0. **Preconditions:** conflict on `/src/config.toml`.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"ours", content:"x\n"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `strategy` and `content` and `exactly one`.
**And** op still in progress (`git_operations` count == 1), nothing written: `/src/config.toml` still `port = 8000`.
**Verification:** error code + substrings; relational count; file read.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-522 — neither strategy nor content

**Scenario:** SC-905. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P0.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml"}]}`.
**Then** `ERR_INVALID_ARGUMENT` containing `exactly one`; op still in progress.
**Verification:** as E2E-NEW-521.

##### E2E-NEW-523 — strategy is case-sensitive

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", strategy:"Ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `'ours'` and `'theirs'` and `Ours` — matching the existing exact-lowercase convention at `git.rs:1474-1478`.
**And** op still in progress.
**Verification:** error code + three substrings; relational count.

##### E2E-NEW-524 — unknown strategy value

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `strategy:"union"`.
**Then** `ERR_INVALID_ARGUMENT` containing `union`, `'ours'`, `'theirs'`.
**Verification:** as above.

##### E2E-NEW-525 — empty resolutions list

**Scenario:** SC-904. **Requirements:** FR-NEW-179.
**Category:** Failure. **Priority:** P1.
**When** `git.merge_resolve {resolutions: []}`.
**Then** `ERR_INVALID_ARGUMENT` containing `resolutions` and `at least one`.
**And** op still in progress with the same conflict set.
**Verification:** error code + substrings; `git.status`.

##### E2E-NEW-526 — resolution path escaping the volume

**Scenario:** SC-904. **Requirements:** FR-NEW-100.
**Category:** Failure. **Priority:** P0.
**When** `git.merge_resolve {resolutions:[{path:"../../etc/passwd", content:"root\n"}]}`.
**Then** `ERR_PATH_OUT_OF_BOUNDS` (the code `safety.normalize_path` raises, `safety.rs:89`).
**And** no file named `passwd` exists anywhere in the volume (`fs.glob "**/passwd"` returns 0 matches); op still in progress.
**Verification:** error code; glob count; relational count.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-527 — same path twice in one resolutions array

**Scenario:** SC-904. **Requirements:** FR-NEW-177.
**Category:** Failure. **Priority:** P1.
**When** `resolutions:[{path:"/src/config.toml",strategy:"ours"},{path:"/src/config.toml",strategy:"theirs"}]`.
**Then** `ERR_INVALID_ARGUMENT` containing `/src/config.toml` and `duplicate`.
**And** op still in progress; file unchanged (`port = 8000`).
**Verification:** error code + substrings; file read.

##### E2E-NEW-528 — git.commit blocked while a merge is in progress

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge in progress.
**When** `git.commit {mount_id:"proj", message:"sneak"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains all of: `merge`, `in progress`, `git.merge_resolve`, `git.merge_abort`.
**And** `git.log "main"` length unchanged (no commit `sneak`).
**Verification:** error code + four substrings; log contents scanned for `sneak`.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-529 — git.branch_switch blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. Same preconditions.
**When** `git.branch_switch {mount_id:"proj", branch:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`, `git.merge_abort`.
**And** `HEAD` still symbolic to `refs/heads/main`; `/src/config.toml` still `port = 8000`.
**Verification:** error code + substrings; `entry.db.get_ref("HEAD")` symbolic target; file read.

##### E2E-NEW-530 — second git.merge blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279, FR-NEW-283.
**Category:** Failure. **Priority:** P0. Same preconditions.
**When** `git.merge {source_ref:"feature"}` again.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`.
**And** exactly one `git_operations` row still (count == 1), and its stored conflict set is unchanged.
**Verification:** error code; relational count + row payload equality against the pre-call snapshot.

##### E2E-NEW-531 — git.remote_pull blocked while a merge is in progress

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT with an `origin` configured; merge in progress.
**When** `git.remote_pull {mount_id:"proj", branch:"main"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`, `git.merge_resolve`.
**And** **no network call happened**: `refs/remotes/origin/main` is unchanged, and the audit log contains no new `git.remote_pull` entry (per `git/remote.rs:363-369`, a real attempt always writes exactly one).
**Verification:** error code + substrings; ref sha; audit entry count delta == 0.

##### E2E-NEW-532 — git.rebase blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1. Same preconditions.
**When** `git.rebase {mount_id:"proj", onto:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; refs unchanged.
**Verification:** error code + substrings; all `refs/heads/*` sha map equality against snapshot.

##### E2E-NEW-533 — git.cherry_pick blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1.
**When** `git.cherry_pick {mount_id:"proj", commit_sha:<C1>}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; refs unchanged.
**Verification:** as above.

##### E2E-NEW-534 — git.reset blocked

**Scenario:** SC-929. **Requirements:** FR-NEW-279.
**Category:** Failure. **Priority:** P1.
**When** `git.reset {mount_id:"proj", ref_name:<C0>}`.
**Then** `ERR_INVALID_ARGUMENT` containing `merge`, `in progress`; `refs/heads/main` still `C2`; volume bytes unchanged.
**Verification:** error code; ref sha; byte compare.

##### E2E-NEW-535 — the removed on_conflict parameter

**Scenario:** SC-912. **Requirements:** FR-DEL-101, FR-MOD-104.
**Category:** Failure. **Priority:** P0. **Preconditions:** SEED-CONFLICT with origin.
**Given** today `git.remote_pull` declares `on_conflict` and `parse_on_conflict` accepts `"ours"` (`git.rs:1467-1479`, schema at `git.rs:353`).
**When** `git.remote_pull {mount_id:"proj", branch:"main", on_conflict:"ours"}`.
**Then** the call fails with `ERR_INVALID_ARGUMENT` whose message names `on_conflict` as removed and points at `git.merge_resolve`.
**And** the `git.remote_pull` JSON Schema no longer contains a property named `on_conflict`, and `tool-contract-golden.json` reflects that removal.
**And** the *old* global behaviour is gone: no merge commit is created by this call.
**Verification:** error code + substrings; `registry.resolve("git.remote_pull").schema` property-key assertion; `git.log` length unchanged.

##### E2E-NEW-536 — merge exceeding the write quota

**Scenario:** SC-903. **Requirements:** FR-NEW-185.
**Category:** Failure. **Priority:** P0.
**Preconditions:** `SafetyConfig.max_write_bytes` set to 10 bytes in the fixture tweak. SEED-FF where the feature commit adds `/docs/readme.md` = 64 bytes of `a`.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_WRITE_QUOTA_EXCEEDED` (the code `safety.charge_write` raises, `safety.rs:131`).
**And** `/docs/readme.md` does **not** exist in the volume (`fs.exists` false) — the charge precedes the apply, exactly as on the pull path (`git.rs:1928-1931`).
**And** `refs/heads/main` still `C0`; `git_operations` count == 0.
**Verification:** error code; `fs.exists`; ref sha; relational count.

##### E2E-NEW-537 — resolution content exceeding the quota

**Scenario:** SC-904. **Requirements:** FR-NEW-185.
**Category:** Failure. **Priority:** P1.
**Preconditions:** `max_write_bytes` = 40; SEED-CONFLICT merge in progress (conflict charged 0 bytes so far).
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:<200 'z' chars>}]}`.
**Then** `ERR_WRITE_QUOTA_EXCEEDED`.
**And** `/src/config.toml` still reads `port = 8000` (unchanged main side), no commit created, op still in progress (count == 1).
**Verification:** error code; file read; log length; relational count.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-538 — merge on a volume with no HEAD

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** `git.init` only, no commit.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `no checked-out branch` (matching the wording family at `git.rs:1500-1504`).
**Verification:** error code + substring.

##### E2E-NEW-539 — merge on a non-initialized volume

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** Failure. **Priority:** P1. **Preconditions:** project seeded, `git.init` never called.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `proj` and `never initialized` (wording family at `git.rs:1546-1551`).
**Verification:** error code + substrings.

##### E2E-NEW-540 — stash_pop conflict, then a bad resolve

**Scenario:** SC-930. **Requirements:** FR-NEW-126, FR-NEW-177., FR-NEW-186
**Category:** Failure. **Priority:** P1.
**Preconditions:** `C0` commits `/notes.md` = `todo\n`. Write `/notes.md` = `todo local\n`, `git.stash_save`. Commit `/notes.md` = `todo upstream\n` → `C1`.
**When** `git.stash_pop {mount_id:"proj"}`.
**Then** `status == "conflict"`, `operation == "stash_pop"`, `conflicts[0].path == "/notes.md"`, `continue_with`/`abort_with` includes `"git.merge_resolve"` and `"git.merge_abort"` — the same shared pair.
**When** `git.merge_resolve {resolutions:[{path:"/other.md", strategy:"ours"}]}`.
**Then** `ERR_INVALID_ARGUMENT` containing `/other.md` and `not in conflict`.
**And** `/notes.md` still reads exactly `todo upstream\n`, contains no `<<<<<<<`, and the stash entry is not dropped (`git.stash_list` length still 1).
**Verification:** JSON fields; error code + substrings; file bytes; stash list length.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-541 — squash given a non-boolean

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** Failure. **Priority:** P2.
**When** `git.merge {source_ref:"feature", squash:"yes"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `squash`.
**And** no commit created, no row.
**Verification:** error code + substring; log length; relational count.

##### E2E-NEW-542 — merge commit has exactly 2 parents

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge → conflict → resolve `strategy:"ours"`.
**Then** with `oid = merge_commit`: `repo.find_commit(oid).parent_count() == 2` exactly; `parent_id(0) == C2` (target tip first); `parent_id(1) == C1` (source tip second) — the ordering `git.rs:1884` already establishes for pull.
**Verification:** libgit2 reads through the repo entry, plus `git.show` reporting `parents: [C2, C1]`.
**Cleanup:** fixture drop.

##### E2E-NEW-543 — squash commit has exactly 1 parent

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-506.
**Then** `repo.find_commit(merge_commit).parent_count() == 1` exactly, and `parent_id(0) == C0` (the pre-merge target tip).
**Verification:** as above.

##### E2E-NEW-544 — squash omits the source's individual commits

**Scenario:** SC-906. **Requirements:** FR-NEW-191.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-506 (feature has `Cf1`, `Cf2`).
**Then** `git.log {ref_name:"main"}` contains neither `Cf1` nor `Cf2` sha, and has exactly 2 entries (`C0`, squash commit).
**And** the squash commit's tree contains both `/docs/readme.md` and `/docs/guide.md` — the cumulative diff is present even though the commits are not.
**Verification:** log sha set assertion; `tree().get_path()` for both files.

##### E2E-NEW-545 — target ref moves to the merge commit

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-503.
**Then** `entry.db.get_ref("refs/heads/main").target == merge_commit`, and the libgit2 reference `refs/heads/main` resolves to the same oid (both stores updated, as `git.rs:1891-1897` does for pull).
**Verification:** relational ref read + `repo.find_reference` read, asserted equal.

##### E2E-NEW-546 — source ref is untouched

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P1. **Preconditions:** E2E-NEW-503; snapshot `refs/heads/feature` before.
**Then** after the merge, `refs/heads/feature` still targets `C1` exactly, and the full `refs/heads/*` map differs from the snapshot in exactly one key, `refs/heads/main`.
**Verification:** `list_refs()` map diff, asserted to be a single key.

##### E2E-NEW-547 — exactly one audit entry per completed merge

**Scenario:** SC-903. **Requirements:** FR-NEW-190.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-AUTOMERGE; snapshot `safety.audit(OWNER,"proj").len()`.
**When** `git.merge {source_ref:"feature"}` succeeds.
**Then** the audit gains exactly 1 entry; its `op` is exactly `"git.merge"`, its `path` is exactly `"/"` (the convention at `git/remote.rs:363-368`), and its `detail` contains `outcome ok` and `source feature`.
**And** the detail contains no 40-hex token-like secret and no credentialed URL.
**Verification:** audit length delta; exact `op` and `path` string equality; substring checks.

##### E2E-NEW-548 — conflicted merge still audits, with outcome conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** `git.merge` returns `status:"conflict"`.
**Then** exactly 1 new audit entry with `op == "git.merge"` and `detail` containing `outcome conflict` and `conflicts 1`.
**And** a subsequent successful `git.merge_resolve` adds exactly 1 more entry with `op == "git.merge_resolve"` — two operations, two rows, never one merged row.
**Verification:** audit length deltas of 1 and 1; exact `op` strings.

##### E2E-NEW-549 — quota charged exactly the merged bytes

**Scenario:** SC-903. **Requirements:** FR-NEW-185.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-AUTOMERGE; record `before = safety.bytes_written(OWNER,"proj")`.
**When** the auto-merge completes, writing the 64-byte merged `/src/config.toml`.
**Then** `bytes_written - before == 64` exactly (the length of the merged file), matching `charge_pull_quota`'s "written blobs only, deletes add nothing" rule (`git.rs:1967-1985`).
**Verification:** `safety.bytes_written` delta compared with `merged_bytes.len()`.

##### E2E-NEW-550 — conflicted merge charges nothing

**Scenario:** SC-904. **Requirements:** FR-NEW-171, FR-NEW-185.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `before`.
**When** `git.merge` returns `status:"conflict"`.
**Then** `bytes_written - before == 0` exactly.
**Verification:** `safety.bytes_written` delta.

##### E2E-NEW-551 — git_operations row created on conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-275, FR-MOD-108., FR-NEW-188
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**Then** exactly 1 row for this `volume_id` with `op_type == "merge"`, `state == "conflicted"`, `current_step == null`, `total_steps == null` (single-step operations report null per FR-NEW-186), and a conflict set deserializing to exactly `["/f/000.txt","/f/001.txt","/f/002.txt"]` (sorted).
**Verification:** `SELECT op_type, state, current_step, total_steps, conflicts FROM git_operations WHERE volume_id = ?`, field-by-field equality.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-552 — row removed on full resolve

**Scenario:** SC-904. **Requirements:** FR-NEW-284., FR-NEW-188
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-551 state.
**When** all three are resolved in one call.
**Then** `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` == 0.
**Verification:** relational count.

##### E2E-NEW-553 — row removed on abort

**Scenario:** SC-904. **Requirements:** FR-NEW-284., FR-NEW-186
**Category:** SideEffect. **Priority:** P0. **Preconditions:** E2E-NEW-551 state.
**When** `git.merge_abort`.
**Then** count == 0.
**Verification:** relational count.

##### E2E-NEW-554 — the row is scoped by volume_id

**Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277, FR-MOD-108.
**Category:** SideEffect. **Priority:** P0.
**Preconditions:** two projects `proj-a`, `proj-b` in the **same** relational store, both SEED-CONFLICT, OWNER a member of both.
**When** `git.merge` conflicts on `proj-a` only.
**Then** `SELECT COUNT(*) WHERE volume_id = <a>` == 1 and `WHERE volume_id = <b>` == 0.
**And** `git.commit {mount_id:"proj-b", message:"ok"}` **succeeds**, proving the guard does not leak across volumes (the `volume_id`-in-every-WHERE rule).
**Verification:** two relational counts; successful commit sha returned.
**Cleanup:** abort on `proj-a`.

##### E2E-NEW-555 — partial resolve shrinks the conflict set

**Scenario:** SC-904. **Requirements:** FR-NEW-178.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**When** `git.merge_resolve {resolutions:[{path:"/f/001.txt", strategy:"theirs"}]}`.
**Then** the response is `status == "conflict"` (still), `remaining_conflicts == ["/f/000.txt","/f/002.txt"]`, `resolved_count == 1`.
**And** the `git_operations` row persists with a conflict set of exactly those 2 paths.
**And** **nothing is in the volume yet**: `/f/001.txt` still reads `line-MAIN\n`, not `line-FEATURE\n` — partial resolution buffers, it does not write.
**Verification:** JSON fields; relational row payload; exact file read.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-556 — no commit object on a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-171.
**Category:** SideEffect. **Priority:** P0. **Preconditions:** SEED-CONFLICT; record `git.log "main"` shas.
**When** `git.merge` conflicts.
**Then** `git.log "main"` returns the identical sha list; no object in `entry.objects` has a commit whose message contains `Merge`.
**Verification:** log sha vector equality; ODB scan for commit objects created after the snapshot (count == 0).

##### E2E-NEW-557 — no conflict markers after a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-172.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-CONFLICT + SEED-MULTI(3) + SEED-BINARY files all present; merge conflicts.
**Then** walking **every** file in the volume via `fs.glob "**/*"` and reading raw bytes, none contains the byte sequences `<<<<<<<`, `=======`, or `>>>>>>>`.
**And** the same holds after a partial resolve and again after `git.merge_abort`.
**Verification:** glob + `read_bytes` + `windows(7).any(...)` over each of the three literals, asserted false, at all three points in time.

##### E2E-NEW-558 — no conflict markers after a conflicted pull

**Scenario:** SC-912. **Requirements:** FR-NEW-172, FR-MOD-104.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** E2E-NEW-508 setup, pull returns conflict.
**Then** the same full-volume marker scan finds nothing, before and after `git.merge_resolve`.
**Verification:** as E2E-NEW-557.

##### E2E-NEW-559 — no conflict markers after rebase / cherry_pick / stash_pop conflicts

**Scenario:** SC-919. **Requirements:** FR-NEW-172.
**Category:** DataIntegrity. **Priority:** P0.
**Preconditions:** three sub-cases, each on a fresh fixture: (a) `git.rebase {onto:"feature"}` from SEED-CONFLICT; (b) `git.cherry_pick {commit_sha:<C1>}` from SEED-CONFLICT on main; (c) E2E-NEW-540's stash setup.
**Then** each returns `status == "conflict"` with `operation` equal to `"rebase"`, `"cherry_pick"`, `"stash_pop"` respectively, and in each case the full-volume marker scan finds nothing.
**Verification:** JSON `operation` field; marker scan.
**Cleanup:** `git.merge_abort` in each.

##### E2E-NEW-560 — volume byte-identical after a conflicted merge

**Scenario:** SC-904. **Requirements:** FR-NEW-171., FR-NEW-186
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-MULTI(3) plus `/keep.txt` = `k\n`.
**Given** `before: BTreeMap<String, Vec<u8>>` built from `fs.glob "**/*"` + `read_bytes`.
**When** `git.merge` conflicts.
**Then** the same map rebuilt after is `==` to `before`: same key set, same bytes, no extra file, no removed file.
**Verification:** whole-map equality assertion (prints the differing key on failure).

##### E2E-NEW-561 — mid-apply failure leaves the volume untouched

**Scenario:** SC-904. **Requirements:** FR-NEW-184.
**Category:** DataIntegrity. **Priority:** P0.
**Preconditions:** SEED-MULTI(3) auto-mergeable variant (feature and main touch distinct files `/f/000.txt` and `/f/002.txt`), plus a pre-existing **directory** at `/f/001.txt` created directly in the volume so the merged tree's write to `/f/001.txt` is refused by the pass-1 pre-check (`check_path_writable`, `git.rs:2121-2140`). Snapshot `before` as in E2E-NEW-560.
**When** `git.merge {source_ref:"feature"}`.
**Then** `ERR_INVALID_ARGUMENT`, message contains `/f/001.txt` and `already exists as a directory`.
**And** the rebuilt map `==` `before`: in particular `/f/000.txt` was **not** written even though it sorts first, proving pass 1 gates every write before pass 2 moves any byte.
**And** `refs/heads/main` unchanged; `git_operations` count == 0; `bytes_written` delta == 0.
**Verification:** error code + substrings; map equality; ref sha; relational count; quota delta.

##### E2E-NEW-562 — abort restores volume and HEAD exactly

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** DataIntegrity. **Priority:** P0. **Preconditions:** SEED-MULTI(3); snapshot `before` map, `head_sha`, `head_symbolic_target`, `bytes_written_before`.
**When** merge conflicts, one path is partially resolved, then `git.merge_abort`.
**Then** rebuilt map `==` `before`; `refs/heads/main` == `head_sha`; `HEAD` symbolic target == `head_symbolic_target`; `git_operations` count == 0.
**And** `bytes_written - bytes_written_before == 0` (abort writes nothing and therefore charges nothing).
**Verification:** map equality; two ref reads; relational count; quota delta.

##### E2E-NEW-563 — both sides deleted the same file

**Scenario:** SC-904. **Requirements:** FR-NEW-181.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-DELDEL.
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts` contains exactly one entry, `/keep.txt`; `/tmp/scratch.txt` is **not** listed — an identical deletion on both sides is not a conflict.
**When** resolved with `{path:"/keep.txt", content:"k-merged\n"}`.
**Then** `status == "merged"`; `fs.exists("/tmp/scratch.txt")` is false; `/keep.txt` reads exactly `k-merged\n`; the merge commit's tree has no entry `tmp/scratch.txt`.
**Verification:** conflicts path list equality; `fs.exists`; file read; `tree().get_path()` returns `Err`.

##### E2E-NEW-564 — one side deleted, the other modified

**Scenario:** SC-904. **Requirements:** FR-NEW-180.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-DELMOD (feature deletes `/lib/util.rs`, main modifies it).
**When** `git.merge {source_ref:"feature"}`.
**Then** `status == "conflict"`; `conflicts[0].path == "/lib/util.rs"`; `conflicts[0].theirs.exists == false` with `content == null`; `conflicts[0].ours.exists == true` with content exactly `pub fn a() { println!("x"); }\n`; `conflicts[0].base.exists == true` with `pub fn a() {}\n`.
**When** resolved with `{path:"/lib/util.rs", strategy:"theirs"}` (take the deletion).
**Then** `status == "merged"`; `fs.exists("/lib/util.rs")` is false; the merge commit tree has no `lib/util.rs`; the parent directory `/lib` is handled consistently (assert the exact observed state: `fs.exists("/lib")` matches what the engine leaves, asserted as `true`, documenting that empty directories are not pruned).
**Verification:** JSON fields including explicit `null`; `fs.exists` both paths; tree lookup.

##### E2E-NEW-565 — binary file conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-182.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-BINARY.
**When** `git.merge {source_ref:"feature"}`.
**Then** `status == "conflict"`; `conflicts[0].path == "/assets/logo.png"`; `conflicts[0].binary == true`; `ours.content` base64-decodes to exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]` and `theirs.content` to `...,0x00,0x02]`; `base.content` to `...,0x00,0x01]`. No line-level hunk fields present.
**When** resolved with `{path:"/assets/logo.png", strategy:"theirs"}`.
**Then** `read_bytes("/assets/logo.png")` equals exactly the 10-byte vector ending `0x00,0x02`, and its length is exactly 10 (no marker insertion, no text mangling).
**Verification:** base64 decode + byte-vector equality; `read_bytes` length and content.

##### E2E-NEW-566 — file/directory type change

**Scenario:** SC-904. **Requirements:** FR-NEW-183.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-TYPECHANGE (feature makes `/mod` a directory holding `/mod/inner.rs`; main keeps `/mod` a file).
**When** `git.merge {source_ref:"feature"}`.
**Then** the call returns `status:"conflict"` with `conflicts[0].type_change == true`. FR-NEW-183 decides this: a file-versus-directory type change is surfaced as a conflict, never refused, so `ERR_INVALID_ARGUMENT` is NOT an acceptable outcome here. The side objects carry exactly `exists` and `content` per FR-NEW-186, so the type change is readable only from `type_change`: assert `conflicts[0].type_change == true`, `conflicts[0].ours.exists == true` with the file's content, `conflicts[0].theirs.exists == false` and `conflicts[0].theirs.content == null`. The keys `ours.kind` and `theirs.kind` are emitted by no tool.
**And** in either case `/mod` still reads exactly `placeholder v2\n` and `fs.exists("/mod/inner.rs")` is false — nothing applied.
**And** `bytes_written` delta == 0.
**Verification:** JSON fields or error code; file read; `fs.exists`; quota delta.
**Cleanup:** `git.merge_abort` if in progress.

##### E2E-NEW-567 — unicode content conflict

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-UNICODE.
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts[0].ours.content` equals exactly `bonjour: kafé ☕\nadieu: naïve\n` and `theirs.content` exactly `bonjour: caffè 🇮🇹\nadieu: naïve\n`, compared as UTF-8 **byte vectors**, not `String` after any normalization (a NFC/NFD change would fail).
**When** resolved with `content:"bonjour: café ☕🇮🇹\nadieu: naïve\n"`.
**Then** `read_bytes("/i18n/fr.txt")` equals exactly those bytes, and its length equals `"bonjour: café ☕🇮🇹\nadieu: naïve\n".as_bytes().len()`.
**Verification:** byte-vector equality on all three.

##### E2E-NEW-568 — empty file on one side

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-EMPTY (base `/flag` is 0 bytes; feature `on\n`, main `off\n`).
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts[0].base.exists == true`, `base.content == ""`; `ours.content == "off\n"`; `theirs.content == "on\n"`.
**When** resolved with `strategy:"theirs"`.
**Then** `/flag` reads exactly `on\n`.
**Verification:** JSON fields with explicit empty string and `0`; file read.

##### E2E-NEW-569 — 250 conflicting files

**Scenario:** SC-904. **Requirements:** FR-NEW-170, FR-NEW-275.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-MULTI(250).
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts.len() == 250` exactly, paths are `/f/000.txt … /f/249.txt` sorted ascending, and each carries all three sides.
**And** exactly **one** `git_operations` row exists (not 250).
**When** one `git.merge_resolve` supplies 250 resolutions, all `strategy:"theirs"`.
**Then** `status == "merged"`; all 250 files read exactly `line-FEATURE\n`; `git_operations` count == 0; exactly one commit created with exactly 2 parents; `bytes_written` delta == `250 * "line-FEATURE\n".len()`.
**Verification:** vector length + sorted path equality; relational counts; 250 file reads in a loop asserting equality; `parent_count()`; quota delta arithmetic.

##### E2E-NEW-570 — resolution path collides with an existing directory

**Scenario:** SC-905. **Requirements:** FR-NEW-183, FR-NEW-175.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-COLLIDE — main has directory `/report/` containing `/report/q.csv` = `a,b\n`; feature has file `/report` = `r\n`.
**When** `git.merge {source_ref:"feature"}` and, if a conflict is surfaced, `git.merge_resolve {resolutions:[{path:"/report", strategy:"theirs"}]}`.
**Then** the resolve fails with `ERR_INVALID_ARGUMENT` whose message contains `/report` and `already exists as a directory`.
**And** `/report/q.csv` still reads exactly `a,b\n`; `fs.is_dir("/report")` is true; no commit created; op still in progress.
**Verification:** error code + substrings; file read; `fs.is_dir`; log length; relational count.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-571 — resolution content is the empty string

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT conflict in progress.
**When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:""}]}`.
**Then** `status == "merged"` (an explicit empty resolution is valid, distinct from a missing field per E2E-NEW-522).
**And** `read_bytes("/src/config.toml").len() == 0`; the merge commit's tree entry for that path is the empty blob `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`.
**And** `bytes_written` delta == 0 for this write.
**Verification:** byte length; blob oid equality against the known empty-blob sha; quota delta.

##### E2E-NEW-572 — CRLF and no trailing newline preserved

**Scenario:** SC-905. **Requirements:** FR-NEW-175.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT conflict in progress.
**When** resolved with `content:"[server]\r\nport = 7000\r\nretries = 2"` (CRLF, no final newline).
**Then** `read_bytes` equals exactly those 41 bytes: last byte is `0x32` (`2`), not `0x0A`; the file contains exactly two `0x0D 0x0A` pairs and no bare `0x0A`.
**Verification:** byte-vector equality plus explicit last-byte and CR-count assertions.

##### E2E-NEW-573 — unrelated histories have no common ancestor

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P1.
**Preconditions:** `main` = `C0` committing `/a.txt` = `alpha\n`; an orphan branch `island` created with no parent, committing `/a.txt` = `omega\n`.
**When** `git.merge {source_ref:"island"}`.
**Then** either `ERR_INVALID_ARGUMENT` containing `unrelated histories`, or `status == "conflict"` with `conflicts[0].base.exists == false` and `base.content == null`. The test pins the conflict branch and asserts `base.exists == false` explicitly.
**And** `ours.content == "alpha\n"`, `theirs.content == "omega\n"`.
**Verification:** JSON fields including explicit `false`/`null`.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-574 — conflicting path at the length ceiling

**Scenario:** SC-904. **Requirements:** FR-NEW-170.
**Category:** EdgeCase. **Priority:** P2.
**Preconditions:** a conflicting file at `/` + 240 `d` chars + `/x.txt`, within the configured `max_path_len`; the same seeded on both branches with different content (`A\n` vs `B\n`).
**When** `git.merge` then resolve by `strategy:"ours"`.
**Then** the conflict response carries that exact path string (no truncation, compared character by character); the resolve succeeds; the file reads exactly `A\n`.
**And** a second run with `max_path_len` lowered below that length fails with `ERR_INVALID_ARGUMENT` from `ensure_path_fits` (`safety.rs:59`) and writes nothing.
**Verification:** exact path string equality; file read; error code in the second run + full-volume byte-map equality.

##### E2E-NEW-575 — merge re-runs cleanly after an abort

**Scenario:** SC-904. **Requirements:** FR-NEW-197.
**Category:** EdgeCase. **Priority:** P1. **Preconditions:** SEED-CONFLICT; merge → conflict → abort.
**When** `git.merge {source_ref:"feature"}` again.
**Then** it returns a `status == "conflict"` response **identical** to the first one (assert full JSON equality against the first response), proving abort left no residue.
**When** resolved with `strategy:"theirs"`.
**Then** `status == "merged"`; `/src/config.toml` line 2 == `port = 9090`; exactly one `git.merge` audit entry per attempt (3 entries total: merge, merge_abort, merge, merge_resolve → assert the exact `op` sequence `["git.merge","git.merge_abort","git.merge","git.merge_resolve"]`).
**Verification:** whole-JSON equality; file read; audit `op` vector equality.

##### E2E-NEW-576 — resolving across two sequential partial calls

**Scenario:** SC-904. **Requirements:** FR-NEW-178.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-MULTI(3) conflict.
**When** call 1 resolves `/f/000.txt` with `strategy:"ours"`; call 2 resolves `/f/001.txt` with `content:"line-X\n"` and `/f/002.txt` with `strategy:"theirs"`.
**Then** call 1 returns `status == "conflict"` with `remaining_conflicts.len() == 2`; call 2 returns `status == "merged"`.
**And** final contents: `/f/000.txt` == `line-MAIN\n`, `/f/001.txt` == `line-X\n`, `/f/002.txt` == `line-FEATURE\n` — the call-1 decision survived across calls.
**And** exactly one commit created with 2 parents; `git_operations` count == 0.
**Verification:** JSON per call; three file reads; `parent_count()`; relational count.

##### E2E-NEW-577 — two concurrent merges, exactly one wins

**Scenario:** SC-929. **Requirements:** FR-NEW-283.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** SEED-CONFLICT.
**When** two `git.merge {source_ref:"feature"}` calls are issued with `tokio::join!` on the same `mount_id`.
**Then** exactly one returns `status == "conflict"` and exactly one returns `ERR_INVALID_ARGUMENT` containing `in progress` — never two conflict responses, never two rows.
**And** `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` == 1.
**And** the full-volume byte map equals the pre-call snapshot.
**Verification:** partition the two results and assert one of each; relational count; map equality.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-578 — per-volume isolation of resolution

**Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** `proj-a` and `proj-b`, both SEED-CONFLICT, both with a merge in progress, OWNER a member of both.
**When** `git.merge_resolve {mount_id:"proj-a", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
**Then** `proj-a` merges: its `/src/config.toml` line 2 == `port = 8000`, its row is gone.
**And** `proj-b` is untouched: its row still exists with the same conflict set, and its `/src/config.toml` still line 2 == `port = 8000` with no commit created.
**Verification:** per-volume relational reads; per-volume file reads scoped through each project's `VolumeClient`; `git.log` length for `proj-b`.
**Cleanup:** abort on `proj-b`.

##### E2E-NEW-579 — read-only tools remain available during an in-progress operation

**Scenario:** SC-929. **Requirements:** FR-NEW-280.
**Category:** Concurrency. **Priority:** P0. **Preconditions:** SEED-CONFLICT, merge in progress.
**When** `git.log`, `git.show {commit_sha:<C2>}`, `git.diff {from:<C0>, to:<C2>}`, `git.branches`, `git.status`, and `fs.read_text {path:"/src/config.toml"}` are each called.
**Then** all six return `Ok`. `fs.read_text` returns exactly the SEED-CONFLICT main text (`port = 8000`). `git.log "main"` returns exactly 2 entries.
**And** `git_operations` count is still 1 afterwards (no read cleared it).
**Verification:** six `is_ok()` assertions; exact content assertions; relational count.
**Cleanup:** `git.merge_abort`.

##### E2E-NEW-580 — another member can abort the operation

**Scenario:** SC-929. **Requirements:** FR-NEW-282.
**Category:** Concurrency. **Priority:** P1. **Preconditions:** SEED-CONFLICT; `second@acme.test` added as a member of `proj`; merge started by OWNER and conflicting.
**When** `env.as_person("second@acme.test","git.merge_abort",{mount_id:"proj"})`.
**Then** it succeeds (`status == "aborted"`) — authorization is membership only, no per-operation ownership.
**And** `git_operations` count == 0; the volume byte map equals the pre-merge snapshot.
**And** the abort audit entry is recorded under `second@acme.test`, not OWNER: `safety.audit("second@acme.test","proj")` has exactly 1 entry with `op == "git.merge_abort"`, and `safety.audit(OWNER,"proj")` gained none.
**Verification:** `is_ok()` + field; relational count; map equality; two per-person audit reads.

---

#### Band C. Interactive rebase, cherry-pick, reset, revert

##### 1. Shared fixtures (concrete seed data)

All fixtures use `MOUNT = "proj1"`, `OWNER = "owner@test.com"` (harness constants, `git.rs:2437+`), start with `git.init`, and commit via `git.commit`. Files are written with `Env::write` before each commit. Shas are captured from `commit_sha` (`git.rs:714`).

**FX-LINE** (linear, 4 commits, branch `main`):

| Commit | Message | Volume state after |
|---|---|---|
| C1 | `C1 base` | `/a.txt`=`"a1\n"` |
| C2 | `C2 add b` | `/a.txt`=`"a1\n"`, `/b.txt`=`"b1\n"` |
| C3 | `C3 edit a` | `/a.txt`=`"a1\na3\n"`, `/b.txt`=`"b1\n"` |
| C4 | `C4 add d` | `+ /d.txt`=`"d1\n"` |

**FX-FORK** (`main` = C1,C2; `feature` = C1,F1,F2):

| Commit | Message | Change |
|---|---|---|
| C1 | `C1 base` | `/a.txt`=`"a1\n"` |
| C2 | `C2 main edit` | `/a.txt`=`"a1\nMAIN\n"` |
| F1 | `F1 feature edit` | `/a.txt`=`"a1\nFEAT1\n"` (conflicts with C2) |
| F2 | `F2 feature add` | `+ /f.txt`=`"f1\n"` |

**FX-FORK4** (`main` = C1,C2; `feature` = C1,F1,F2,F3,F4) where F1 and F3 both touch `/a.txt` line 2 (conflict vs C2), F2 and F4 touch `/f2.txt`,`/f4.txt` (no conflict). Messages `F1 feat one` … `F4 feat four`.

**FX-MERGE**: C1 base; `main` -> M1 (`/m.txt`=`"m1\n"`); `side` off C1 -> S1 (`/s.txt`=`"s1\n"`); merge commit MG (`Merge side into main`, parents `[M1, S1]`, volume has `/a.txt`,`/m.txt`,`/s.txt`).

**FX-INIT**: only C1 (`C1 base`, `/a.txt`=`"a1\n"`), no parent.

Verification helpers used throughout:
- **shas/refs**: `entry.db.get_ref("refs/heads/main")` (`db.rs:214`), `list_refs` (`db.rs:261`).
- **history**: `git.log` (`git.rs:598`) -> assert on `commits[i]["sha"]`, `["message"]`, `["parents"]` (short shas, `git.rs:2168`).
- **volume bytes**: `Env::read(path)` (`git.rs:2465`) and `VolumeClient::read_bytes` for byte-exact compare.
- **reachability**: commit reachable iff its sha appears in `git.log` of the branch; object still present iff `entry.db.object_exists(sha)` (`db.rs:131`).
- **audit**: `state.safety.audit(OWNER, MOUNT)` (`safety.rs:159`).
- **quota**: `state.safety.bytes_written(OWNER, MOUNT)` (`safety.rs:163`).
- **in-progress row**: `git_operations` query scoped by `volume_id`.

---

##### 3. Full specifications

Conventions for every test: harness `Env::build` with `c.git.enabled = true` (`git.rs:2440-2447`); caller is `OWNER` unless stated; `mount_id` is `MOUNT`. "Assert error" means: the `Result` is `Err`, `err.code() == "ERR_..."` and `err.to_string()` contains the named substring (format `"{CODE}: {message}"`, `errors.rs:176`).

##### A. Interactive rebase (E2E-NEW-600 .. E2E-NEW-639)

---

**E2E-NEW-600 — Rebase happy: pick 2 commits onto main**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-211.
- **Preconditions:** FX-FORK. `main` tip = `<sha-of-C2>`, `feature` tip = `<sha-of-F2>`, checked out branch = `feature`. F1 edits `/a.txt` line 2 from nothing to `FEAT1`, and C2 sets it to `MAIN` — for this test only, change F1 to touch `/g.txt`=`"g1\n"` instead (no conflict); call this **FX-FORK-CLEAN** (F1 = `F1 feature edit`, adds `/g.txt`=`"g1\n"`; F2 = `F2 feature add`, adds `/f.txt`=`"f1\n"`).
- **Given** `git.log {ref_name:"feature"}` = `[F2, F1, C1]`.
- **When** `git.rebase {mount_id:"proj1", onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** result `status == "completed"` (the closed `git.rebase` status set of FR-NEW-199).
- **And** `git.log {ref_name:"feature"}` returns exactly 4 commits, messages in order `["F2 feature add", "F1 feature edit", "C2 main edit", "C1 base"]`. *Method:* map `commits[i]["message"]` (`git.rs:2161`).
- **And** `commits[0]["sha"] != <sha-of-F2>` and `commits[1]["sha"] != <sha-of-F1>` (new shas).
- **And** `commits[1]["parents"] == [<sha-of-C2>[..8]]` (short sha, `git.rs:2168`), `commits[0]["parents"] == [commits[1]["sha"][..8]]`.
- **And** every commit has exactly 1 parent: `commits[i]["parents"].len() == 1` for i in 0..3, `== 0` for `C1`.
- **And** `entry.db.get_ref("refs/heads/feature").target == commits[0]["sha"]` (`db.rs:214`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` (unchanged).
- **And** volume bytes: `/a.txt == b"a1\nMAIN\n"`, `/g.txt == b"g1\n"`, `/f.txt == b"f1\n"`. *Method:* `Env::read` (`git.rs:2465`), compared as bytes.
- **And** no `git_operations` row for `volume_id="proj1"`.
- **Cleanup:** `Fixture` drop (temp dirs). **Priority:** P0.

---

**E2E-NEW-601 — Rebase happy: squash F2 into F1**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-212.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"squash", sha:"<sha-of-F2>"}]}`.
- **Then** `git.log {ref_name:"feature"}` has exactly **3** commits: messages `["F1 feature edit\n\nF2 feature add", "C2 main edit", "C1 base"]`. *Method:* exact string equality on `commits[0]["message"]` after `trim` (`git.rs:2161` already trims).
- **And** `commits[0]["parents"] == [<sha-of-C2>[..8]]` — the squashed commit sits directly on C2.
- **And** volume contains both `/g.txt == b"g1\n"` and `/f.txt == b"f1\n"` (the squashed tree is F2's tree).
- **And** neither `<sha-of-F1>` nor `<sha-of-F2>` appears in any `commits[i]["sha"]`.
- **Priority:** P0.

---

**E2E-NEW-602 — Rebase happy: drop**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-213.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"drop", sha:"<sha-of-F2>"}]}`.
- **Then** `git.log{ref_name:"feature"}` = 3 commits, messages `["F1 feature edit","C2 main edit","C1 base"]`.
- **And** `/g.txt` reads `b"g1\n"`; reading `/f.txt` fails with `ERR_NOT_FOUND` (the dropped commit's file never enters the volume). *Method:* `VolumeClient::read_bytes("/f.txt")` returns `Err`, code `ERR_NOT_FOUND`.
- **And** the tip's tree has exactly 2 entries `a.txt`, `g.txt`. *Method:* `git.show {commit_sha: tip}` file list.
- **Priority:** P0.

---

**E2E-NEW-603 — Rebase happy: reword**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[{action:"reword", sha:"<sha-of-F1>", message:"F1 reworded"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** `commits[1]["message"] == "F1 reworded"` exactly, `commits[0]["message"] == "F2 feature add"`.
- **And** `commits[1]["sha"] != <sha-of-F1>`.
- **And** the tree of `commits[1]` is byte-identical to F1's tree: `git.diff {from_ref: <sha-of-F1>, to_ref: commits[1]["sha"]}` returns an empty diff (no hunks).
- **And** `commits[1]["author_email"] == "owner@test.com"` (preserved from the original, `git.rs:2165`).
- **Priority:** P0.

---

**E2E-NEW-604 — Rebase happy: squash chain of three**

**Category:** Happy. **Scenario:** SC-915. **Requirements:** FR-NEW-212.
- **Preconditions:** FX-FORK4 with F1..F4 rewritten to be conflict-free: F1 adds `/f1.txt`=`"1\n"`, F2 adds `/f2.txt`=`"2\n"`, F3 adds `/f3.txt`=`"3\n"`, F4 adds `/f4.txt`=`"4\n"`. Call this **FX-FORK4-CLEAN**.
- **When** `todo = [pick F1, squash F2, squash F3, pick F4]`, `onto:"main"`.
- **Then** `git.log{ref_name:"feature"}` has exactly **4** commits: `["F4 feat four", "F1 feat one\n\nF2 feat two\n\nF3 feat three", "C2 main edit", "C1 base"]`.
- **And** the squashed commit's tree contains `/f1.txt`,`/f2.txt`,`/f3.txt` and **not** `/f4.txt`.
- **And** volume has all four files with bytes `"1\n","2\n","3\n","4\n"`.
- **Priority:** P0.

---

**E2E-NEW-605 — Rebase edge: all commits dropped**

**Category:** EdgeCase. **Scenario:** SC-915. **Requirements:** FR-NEW-213.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [drop F1, drop F2]`, `onto:"main"`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == <sha-of-C2>` — byte equal to `main`'s tip.
- **And** `git.log{ref_name:"feature"}` == `git.log{ref_name:"main"}` (same JSON value).
- **And** volume equals C2's tree exactly: `/a.txt == b"a1\nMAIN\n"`, `/g.txt` and `/f.txt` both absent (`ERR_NOT_FOUND`).
- **And** no `git_operations` row remains.
- **Priority:** P1.

---

**E2E-NEW-606 — Rebase happy: single commit**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-211.
- **Preconditions:** FX-FORK-CLEAN, but `feature` = C1, F1 only.
- **When** `todo = [pick F1]`, `onto:"main"`.
- **Then** 3 commits on `feature`: `["F1 feature edit","C2 main edit","C1 base"]`; tip sha != `<sha-of-F1>`; `parents == [<sha-of-C2>[..8]]`.
- **Priority:** P1.

---

**E2E-NEW-607 — Rebase edge: onto an ancestor is a no-op**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
- **Preconditions:** FX-LINE, branch `main` tip `<sha-of-C4>`.
- **Given** `object_count_before = entry.db.count_objects()` (`db.rs:198`), `tip_before = <sha-of-C4>`, volume snapshot of `/a.txt`,`/b.txt`,`/d.txt`.
- **When** `git.rebase {onto:"<sha-of-C2>", todo:[{action:"pick", sha:"<sha-of-C3>"},{action:"pick", sha:"<sha-of-C4>"}]}` — C2 is already an ancestor of the todo range.
- **Then** result `status == "up_to_date"` with `replayed == 0` (the rebase no-op of FR-NEW-217; the test asserts `result["replayed"] == 0`).
- **And** `entry.db.get_ref("refs/heads/main").target == tip_before` — the tip is byte identical.
- **And** `entry.db.count_objects()` equals `object_count_before` (no new commit objects).
- **And** all three files byte-equal to their snapshot.
- **And** `state.safety.bytes_written(OWNER, MOUNT)` is unchanged from before the call.
- **Priority:** P0.

---

**E2E-NEW-608 — Rebase edge: branch and onto identical**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
- **Preconditions:** FX-LINE, checked out `main`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-C4>"}]}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing substring `"onto"` and `"same"` — rebasing a branch onto itself has no commit range. *(If the chosen implementation makes this a no-op instead, this test asserts the no-op form of E2E-NEW-607; the spec requires one of the two, decided once, and the test asserts it exactly.)* Baseline assertion for the implementer: **reject**, message `git.rebase: 'onto' resolves to the same commit as the current branch tip, there is nothing to rebase`.
- **And** tip unchanged, no `git_operations` row.
- **Priority:** P1.

---

**E2E-NEW-609 — Rebase side-effect: originals unreachable but objects retained**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210, FR-NEW-255.
- **Preconditions:** E2E-NEW-600 executed (record `<sha-of-F1>`, `<sha-of-F2>`).
- **Then** neither original sha appears in `git.log` for any ref: for each ref in `entry.db.list_refs()` (`db.rs:261`), no `commits[i]["sha"]` equals them.
- **And** `entry.db.object_exists("<sha-of-F1>") == true` and same for F2 (`db.rs:131`) — orphaned, not destroyed.
- **And** `git.show {commit_sha:"<sha-of-F1>"}` still succeeds and returns `commit.message == "F1 feature edit"`.
- **Priority:** P0.

---

**E2E-NEW-610 — Rebase side-effect: audit entries**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN; audit log drained by reading `state.safety.audit(OWNER, MOUNT)` and recording its length `n0` (`safety.rs:159`).
- **When** the E2E-NEW-600 rebase.
- **Then** `audit()[n0..]` contains at least one entry with `op == "git.rebase"` exactly (matching the tool-name convention at `git.rs:250`), `project == "proj1"`, and `detail` containing `"onto"` and the resolved onto sha.
- **And** every entry appended after `n0` has `op` in `{"git.rebase"}` — no foreign op string leaks (e.g. no `"git.checkout_file"`).
- **And** the entries are in chronological order (oldest first, `safety.rs:159`).
- **Priority:** P1.

---

**E2E-NEW-611 — Rebase side-effect: write quota charged**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN; `q0 = bytes_written(OWNER, MOUNT)` (`safety.rs:163`); config default quota (large).
- **When** the E2E-NEW-600 rebase, which materializes `/a.txt`(8 bytes `"a1\nMAIN\n"`), `/g.txt`(3), `/f.txt`(3) into the volume.
- **Then** `bytes_written - q0 == 14` if the implementation rewrites the full resulting tree, **or** the exact documented subset. The test asserts the implementation's stated rule: **every byte written to the volume is charged**, so the assertion is `bytes_written - q0 == sum(len(bytes) for each file the rebase wrote)`, computed in the test from the final tree (`git.show` file list + `Env::read` lengths).
- **And** a second identical rebase attempt is not required; this test only asserts the charge.
- **Priority:** P1.

---

**E2E-NEW-612 — Rebase side-effect: no in-progress row on success**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-284.
- **Preconditions:** FX-FORK-CLEAN; assert zero `git_operations` rows for `volume_id="proj1"` before.
- **When** the E2E-NEW-600 rebase (no conflict).
- **Then** `SELECT count(*) FROM git_operations WHERE volume_id='proj1'` == 0 after. *Method:* direct query through the git db handle, `volume_id` in the WHERE clause per the tenancy rule (AGENTS.md).
- **And** `git.rebase_continue {}` then fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **Priority:** P0.

---

**E2E-NEW-613 — Rebase conflict: single pause on step 1**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-219., FR-NEW-186, FR-NEW-187, FR-NEW-188
- **Preconditions:** FX-FORK (the conflicting one: F1 sets `/a.txt` = `"a1\nFEAT1\n"`, C2 sets it to `"a1\nMAIN\n"`).
- **Given** volume currently equals F2's tree: `/a.txt == b"a1\nFEAT1\n"`, `/f.txt == b"f1\n"`. Snapshot both.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]}`.
- **Then** result `status == "conflict"` (shared model).
- **And** `result["current_step"] == 0` (the zero-based index of the F1 entry, per FR-NEW-187; `todo[current_step]` is the entry awaiting resolution. The implementer must make `current_step` identify the F1 entry and the test asserts it resolves to `todo[current_step].sha == <sha-of-F1>`).
- **And** `result["conflicts"]` is exactly `["/a.txt"]`.
- **And** exactly one `git_operations` row exists with `volume_id='proj1'`, `op_type='rebase'`, `state='conflicted'` (the closed set of FR-NEW-188), `todo` deserializing to the 2-entry list submitted.
- **And** volume is byte-identical to the snapshot: `/a.txt == b"a1\nFEAT1\n"`, `/f.txt == b"f1\n"` — nothing applied.
- **And** `entry.db.get_ref("refs/heads/feature").target == <sha-of-F2>` — the ref has not moved.
- **Priority:** P0.

---

**E2E-NEW-614 — Rebase conflict: continue with `theirs` completes**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-174.
- **Preconditions:** E2E-NEW-613 state (paused on F1, conflict on `/a.txt`).
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:{"/a.txt":"theirs"}}` where `theirs` = the commit being replayed (F1).
- **Then** result `status == "completed"` (the closed `git.rebase_continue` status set of FR-NEW-199).
- **And** `/a.txt == b"a1\nFEAT1\n"` exactly (theirs = F1's side).
- **And** `git.log{ref_name:"feature"}` = 4 commits, messages `["F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** both replayed shas differ from `<sha-of-F1>`/`<sha-of-F2>`.
- **And** zero `git_operations` rows for `volume_id='proj1'`.
- **And** `/a.txt` contains no occurrence of `"<<<<<<<"`, `"======="`, `">>>>>>>"`.
- **Priority:** P0.

---

**E2E-NEW-615 — Rebase multi-pause: steps 2 and 4 both conflict**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-220.
- **Preconditions:** **FX-FORK4** (conflicting variant): `main` = C1, C2 where C2 sets `/a.txt` = `"a1\nMAIN\n"`. `feature` = C1, F1, F2, F3, F4 where F1 adds `/f1.txt`=`"1\n"` (clean), **F2** sets `/a.txt` = `"a1\nFEAT2\n"` (conflicts with C2), F3 adds `/f3.txt`=`"3\n"` (clean), **F4** sets `/a.txt` = `"a1\nFEAT4\n"` (conflicts with the replayed result).
- **When (1)** `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}`.
- **Then (1)** `status == "conflict"`, `todo[current_step].sha == <sha-of-F2>`, `conflicts` holding exactly one element whose `path` is `/a.txt`, one `git_operations` row `state` in-progress, `current_step` persisted as the F2 index, branch ref still `<sha-of-F4>`, volume byte-identical to the pre-call snapshot.
- **When (2)** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}`.
- **Then (2)** `status == "conflict"` again, `todo[current_step].sha == <sha-of-F4>` — resumption skipped F1 (already applied) and F3 (clean) and stopped exactly at F4.
- **And (2)** `git.log` of the in-progress head (exposed by the conflict envelope as `result["head"]`, or asserted after completion) — at minimum, the `git_operations` row's `current_step` identifies F4 and the row count is still exactly 1.
- **When (3)** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}`.
- **Then (3)** `status == "completed"`.
- **And** `git.log{ref_name:"feature"}` = exactly 6 commits, messages in order `["F4 feat four","F3 feat three","F2 feat two","F1 feat one","C2 main edit","C1 base"]`.
- **And** every one of the four replayed shas differs from `<sha-of-F1>`..`<sha-of-F4>`; each has exactly 1 parent; `commits[3]["parents"] == [<sha-of-C2>[..8]]`.
- **And** volume: `/a.txt == b"a1\nFEAT4\n"`, `/f1.txt == b"1\n"`, `/f3.txt == b"3\n"`, no conflict markers in any file.
- **And** zero `git_operations` rows.
- **And** exactly **two** conflict pauses occurred (the test counts `status=="conflict"` responses == 2).
- **Priority:** P0.

---

**E2E-NEW-616 — Rebase conflict: volume untouched at the pause**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-171., FR-NEW-187
- **Preconditions:** FX-FORK4 conflicting; full byte snapshot of every volume file (`/a.txt`,`/f1.txt`,`/f2-side files`...) taken via a recursive listing plus `read_bytes`.
- **When** the E2E-NEW-615 step (1) rebase.
- **Then** for every path in the snapshot, current bytes == snapshot bytes; and the set of paths in the volume is identical to the snapshot set (no additions, no deletions). *Method:* `fs.list` recursive + byte compare.
- **Priority:** P0.

---

**E2E-NEW-617 — Rebase conflict: resolution by literal content**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-175.
- **Preconditions:** E2E-NEW-613 state (paused, conflict on `/a.txt`).
- **When** `git.rebase_continue {resolutions:{"/a.txt":{"content":"a1\nMERGED\n"}}}` (the shared model's literal form).
- **Then** `status == "completed"`.
- **And** `/a.txt` bytes == `b"a1\nMERGED\n"` exactly.
- **And** the replayed F1 commit's tree contains that exact blob: `git.show {commit_sha: commits[1]["sha"]}` diff shows `/a.txt` with `MERGED`.
- **And** no conflict markers anywhere.
- **Priority:** P1.

---

**E2E-NEW-618 — Rebase conflict: markers never enter the volume**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-172.
- **Preconditions:** FX-FORK conflicting.
- **When** rebase pauses (E2E-NEW-613), then `git.rebase_continue {resolutions:{"/a.txt":"ours"}}`.
- **Then** at **both** points, every file in the volume is scanned and none contains `"<<<<<<< "`, `"||||||| "`, `"======="` at line start, or `">>>>>>> "`. *Method:* recursive `fs.grep` for `^(<<<<<<<|=======|>>>>>>>)` returning zero matches (grep goes through `Fnmatch`, AGENTS.md).
- **And** with `"ours"`, `/a.txt == b"a1\nMAIN\n"` (the onto side).
- **Priority:** P0.

---

**E2E-NEW-619 — Rebase abort: exact restoration**

**Category:** Happy. **Scenario:** SC-917. **Requirements:** FR-NEW-223.
- **Preconditions:** FX-FORK conflicting. Before the rebase: `tip_before = entry.db.get_ref("refs/heads/feature").target` (== `<sha-of-F2>`), and a full map `{path -> bytes}` of the volume (`/a.txt` = `"a1\nFEAT1\n"`, `/f.txt` = `"f1\n"`).
- **When** `git.rebase` pauses (E2E-NEW-613), **then** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** result `status == "aborted"` (every abort tool reports `aborted`, per FR-NEW-199).
- **And** `entry.db.get_ref("refs/heads/feature").target == tip_before` — string equality on the full 40-char sha.
- **And** the on-disk reference agrees: `repo.find_reference("refs/heads/feature").target().to_string() == tip_before` (the dual write at `git.rs:702-711` must hold for the new ops too).
- **And** the volume path set equals the snapshot key set, and for each path bytes are equal — byte-for-byte, not string-trimmed.
- **And** `git.log{ref_name:"feature"}` is JSON-equal to the log captured before the rebase.
- **And** zero `git_operations` rows for `volume_id='proj1'`.
- **Priority:** P0.

---

**E2E-NEW-620 — Rebase abort: row lifecycle**

**Category:** SideEffect. **Scenario:** SC-917. **Requirements:** FR-NEW-284.
- **Preconditions:** FX-FORK conflicting, rebase paused.
- **Given** exactly 1 `git_operations` row (`op_type='rebase'`).
- **When** `git.rebase_abort {}`.
- **Then** `SELECT count(*) ... WHERE volume_id='proj1'` == 0.
- **And** a second `git.rebase_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** a fresh `git.rebase` with the same todo pauses again identically (`current_step` identifies F1) — proving the cleared row did not poison state.
- **Priority:** P0.

---

**E2E-NEW-621 — Rebase abort mid multi-pause (after one continue)**

**Category:** EdgeCase. **Scenario:** SC-917. **Requirements:** FR-NEW-223.
- **Preconditions:** FX-FORK4 conflicting. Snapshot `tip_before = <sha-of-F4>` and full volume byte map.
- **When** rebase pauses at F2; `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}` pauses at F4; **then** `git.rebase_abort {}`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == tip_before`.
- **And** the volume byte map equals the snapshot exactly — including `/a.txt == b"a1\nFEAT4\n"`, i.e. the already-applied F1/F2/F3 replay is fully undone.
- **And** zero `git_operations` rows.
- **And** `git.log{ref_name:"feature"}` messages == `["F4 feat four","F3 feat three","F2 feat two","F1 feat one","C1 base"]` with the **original** shas F1..F4 (equality on all four full shas).
- **Priority:** P0.

---

**E2E-NEW-622 — Rebase failure: todo omits a commit in range**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN (range `main..feature` = {F1, F2}). Snapshot tip and volume.
- **When** `git.rebase {onto:"main", todo:[{action:"pick", sha:"<sha-of-F2>"}]}` — F1 missing.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"todo"` and the omitted short sha `<sha-of-F1>[..8]` and the word `"missing"`. Baseline message: `git.rebase: todo is missing commit(s) in the rebase range: <short-F1>`.
- **And** tip unchanged, volume bytes unchanged, zero `git_operations` rows, `bytes_written` unchanged.
- **Priority:** P0.

---

**E2E-NEW-623 — Rebase failure: unknown sha in todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [pick F1, pick "0000000000000000000000000000000000000000"]`.
- **Then** error `ERR_NOT_FOUND`, message contains `"0000000"` and `"not found"` (mirrors the existing `commit '{sha}' not found` phrasing at `git.rs:727`).
- **And** tip unchanged; zero rows; `bytes_written` unchanged.
- **Priority:** P0.

---

**E2E-NEW-624 — Rebase failure: sha outside the rebase range**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN. `<sha-of-C1>` exists but is an ancestor of `onto`, not in `main..feature`.
- **When** `todo = [pick F1, pick F2, pick C1]`.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `<sha-of-C1>[..8]` and `"not in the rebase range"`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-625 — Rebase failure: todo starts with squash**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215, FR-NEW-212.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"squash", sha:"<sha-of-F1>"},{action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"squash"` and `"first"` — baseline: `git.rebase: todo cannot start with 'squash', there is no preceding commit to squash into`.
- **And** tip unchanged; zero rows; volume unchanged.
- **Priority:** P0.

---

**E2E-NEW-626 — Rebase failure: todo longer than the bound**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-216.
- **Preconditions:** A project with `N_max + 1` commits on `feature` above `main`, where `N_max` is the configured bound (the spec fixes `N_max = 1000`; the test reads the constant from the tool module so it cannot drift).
- **When** `git.rebase` with a `todo` of `N_max + 1` entries.
- **Then** error `ERR_INVALID_ARGUMENT`, message contains `"todo"`, the number `1000`, and `"at most"`.
- **And** a `todo` of exactly `N_max` entries is accepted by validation (this test asserts only that it does **not** fail with the bound message; it may still be slow, so the test uses `N_max` = the constant with a small fixture only for the reject side, and asserts the accept side with a synthetic `N_max`-length todo that fails later on a different, non-bound error, checking the message does **not** contain `"at most"`).
- **Priority:** P1.

---

**E2E-NEW-627 — Rebase failure: empty todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"main", todo:[]}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"todo"` and `"empty"`.
- **And** tip unchanged; zero rows. *(Distinct from E2E-NEW-605, where commits are explicitly dropped.)*
- **Priority:** P1.

---

**E2E-NEW-628 — Rebase failure: unknown action**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"fixup", sha:"<sha-of-F1>"}, {action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"fixup"` and the exact allowed set text `"pick"`, `"squash"`, `"drop"`, `"reword"`.
- **And** the same for `action:"PICK"` (uppercase) — rejected, proving the enum is exact-match like `parse_on_conflict` (`git.rs:1475-1482`).
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-629 — Rebase failure: reword to an empty message**

**Category:** Failure. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [{action:"reword", sha:"<sha-of-F1>", message:""},{action:"pick", sha:"<sha-of-F2>"}]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"reword"` and `"message"` and `"empty"`.
- **And** tip unchanged; volume unchanged; zero rows — rejected before any replay.
- **Priority:** P0.

---

**E2E-NEW-630 — Rebase failure: reword to whitespace-only**

**Category:** Failure. **Scenario:** SC-915. **Requirements:** FR-NEW-214.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `reword` with `message: "   \n\t \n"`.
- **Then** error `ERR_INVALID_ARGUMENT` with the same substrings as E2E-NEW-629 — because `git2::message_prettify` (used at `git.rs:690`) would reduce this to the empty string, so validation must run on the prettified form, not the raw one.
- **And** tip unchanged.
- **Priority:** P1.

---

**E2E-NEW-631 — Rebase failure: unknown onto ref**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `git.rebase {onto:"refs/heads/nope", todo:[pick F1]}`.
- **Then** error `ERR_NOT_FOUND` containing `"nope"`.
- **And** the same for `onto:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-632 — Rebase failure: missing required arguments**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN.
- **When (a)** `git.rebase {mount_id:"proj1", todo:[pick F1]}` (no `onto`); **(b)** `git.rebase {mount_id:"proj1", onto:"main"}` (no `todo`); **(c)** `git.rebase {onto:"main", todo:[...]}` (no `mount_id`).
- **Then** each fails `ERR_INVALID_ARGUMENT` naming the missing parameter (`"onto"`, `"todo"`, `"mount_id"` respectively) — same shape as the existing required-arg path (`mcp/args.rs` via `a.str("mount_id")`, `git.rs:209`).
- **And** `todo` given as a string, and `todo` given as an object, both fail `ERR_INVALID_ARGUMENT` containing `"todo"` and `"array"`.
- **Priority:** P1.

---

**E2E-NEW-633 — Rebase failure: duplicate sha in todo**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN.
- **When** `todo = [pick F1, pick F2, pick F2]`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `<sha-of-F2>[..8]` and `"duplicate"`.
- **And** tip unchanged; zero rows.
- **Priority:** P1.

---

**E2E-NEW-634 — Rebase failure: validation happens before any work**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-215.
- **Preconditions:** FX-FORK-CLEAN; record `tip`, full volume byte map, `q0 = bytes_written`, `n0 = audit().len()`, `obj0 = count_objects()`.
- **When** each of the five rejection cases in turn: E2E-NEW-622 (omitted), -223 (unknown), -224 (out of range), -225 (leading squash), -229 (empty reword message).
- **Then** after **all five**: tip string-equal, volume byte map equal, `bytes_written == q0`, `count_objects() == obj0`, `audit().len() == n0` (a rejected call records no audit entry and no quota charge, consistent with `safety.rs:338-352`).
- **And** zero `git_operations` rows throughout.
- **Priority:** P0.

---

**E2E-NEW-635 — Rebase failure: continue with nothing in progress**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-224.
- **Preconditions:** FX-LINE, no operation in progress.
- **When** `git.rebase_continue {mount_id:"proj1"}` and `git.rebase_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** both fail `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** tip and volume unchanged.
- **Priority:** P0.

---

**E2E-NEW-636 — Rebase failure: abort with nothing in progress**

**Category:** Failure. **Scenario:** SC-917. **Requirements:** FR-NEW-224.
- **Preconditions:** FX-LINE, no operation in progress.
- **When** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"`.
- **And** tip unchanged, zero rows.
- **Priority:** P0.

---

**E2E-NEW-637 — Rebase failure: starting a second rebase while one is paused**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-283, FR-NEW-225.
- **Preconditions:** FX-FORK conflicting, rebase paused on F1 (E2E-NEW-613); record the row's `current_step` and `todo`.
- **When** `git.rebase {onto:"main", todo:[pick F1, pick F2]}` again.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"rebase"` and `"already in progress"`, and naming `git.rebase_continue` and `git.rebase_abort` as the ways out.
- **And** there is still exactly **1** `git_operations` row, with the same `current_step` and `todo` as recorded — the second call did not overwrite state.
- **And** volume unchanged.
- **Priority:** P0.

---

**E2E-NEW-638 — Rebase failure: commit while a rebase is paused**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-279.
- **Preconditions:** FX-FORK conflicting, rebase paused.
- **When** `git.commit {mount_id:"proj1", message:"sneaky"}`.
- **Then** error `ERR_INVALID_ARGUMENT` containing `"rebase in progress"` — the paused op owns the volume, otherwise the pending replay would commit against a foreign tree.
- **And** branch tip still `<sha-of-F2>`; `git.log` contains no commit with message `"sneaky"`.
- **And** after `git.rebase_abort {}`, `git.commit {message:"sneaky"}` succeeds — the guard is lifted.
- **Priority:** P1.

---

**E2E-NEW-639 — Rebase failure: non-member caller**

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

**E2E-NEW-640 — Cherry-pick happy: apply F2 onto main**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN; checked out branch `main` (tip `<sha-of-C2>`); volume equals C2's tree (`/a.txt == b"a1\nMAIN\n"`). F2 was authored with `author_name:"Ada"`, `author_email:"ada@example.test"` (the `git.commit` optional author args, `git.rs:194-199`), caller of the pick is `OWNER`.
- **When** `git.cherry_pick {mount_id:"proj1", commit_sha:"<sha-of-F2>"}`.
- **Then** result `status == "committed"` and `result["new_sha"]` is a 40-char sha `!= <sha-of-F2>` (`git.cherry_pick` returns `new_sha`, not `commit_sha`, per FR-NEW-199).
- **And** `git.log{ref_name:"main"}` = 3 commits, messages `["F2 feature add","C2 main edit","C1 base"]`.
- **And** `commits[0]["author"] == "Ada"` and `commits[0]["author_email"] == "ada@example.test"` — author preserved (`git.rs:2164-2165`).
- **And** the committer is the caller: `git.show {commit_sha: result["new_sha"]}` reports committer email `"owner@test.com"`. *Method:* `git.show` must expose committer; if it does not today, assert via `repo.find_commit(oid).committer().email()` in the test.
- **And** `commits[0]["parents"] == [<sha-of-C2>[..8]]` and parent count == 1.
- **And** volume: `/f.txt == b"f1\n"`, `/a.txt == b"a1\nMAIN\n"` (untouched).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

**E2E-NEW-641 — Cherry-pick happy: only the diff is applied**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN plus an extra `main` commit C2b (`C2b add keep`, `/keep.txt`=`"keep\n"`), so `main`'s tree = `{a.txt, keep.txt}`. F2 adds `/f.txt` only.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** the new tip's tree has exactly 3 entries: `a.txt`, `keep.txt`, `f.txt`.
- **And** `/keep.txt == b"keep\n"` and `/a.txt == b"a1\nMAIN\n"` — the source commit's *tree* did not replace the target tree; only its diff was applied.
- **And** `/g.txt` (introduced by F1, not picked) is absent: `read_bytes("/g.txt")` -> `ERR_NOT_FOUND`.
- **Priority:** P0.

---

**E2E-NEW-642 — Cherry-pick happy: two sequential picks**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN, on `main`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F1>"}` then `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `git.log{ref_name:"main"}` = 4 commits, messages `["F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** the two new shas differ from each other and from `<sha-of-F1>`,`<sha-of-F2>`.
- **And** `commits[0]["parents"] == [commits[1]["sha"][..8]]`, `commits[1]["parents"] == [<sha-of-C2>[..8]]`.
- **And** volume contains `/g.txt == b"g1\n"` and `/f.txt == b"f1\n"`.
- **Priority:** P1.

---

**E2E-NEW-643 — Cherry-pick side-effect: audit and quota**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN on `main`; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}` (writes `/f.txt`, 3 bytes).
- **Then** `audit()[n0..]` contains exactly one entry with `op == "git.cherry_pick"`, `path` = the affected path (`"/f.txt"`) or the mount root, and `detail` containing `<sha-of-F2>` (full or short, asserted verbatim against the implementation's chosen format, which the spec fixes as the **full** source sha).
- **And** `bytes_written() - q0 == 3`.
- **And** no audit entry with op `"git.commit"` was appended (the pick is its own audited operation, not a nested commit).
- **Priority:** P1.

---

**E2E-NEW-644 — Cherry-pick side-effect: source branch untouched**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** FX-FORK-CLEAN on `main`; record `feature_tip = <sha-of-F2>` and `git.log{ref_name:"feature"}`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `entry.db.get_ref("refs/heads/feature").target == feature_tip`.
- **And** `git.log{ref_name:"feature"}` is JSON-equal to the recorded log.
- **And** `entry.db.get_ref("HEAD")` is still symbolic pointing at `refs/heads/main` (`db.rs:214`, symbolic flag).
- **Priority:** P0.

---

**E2E-NEW-645 — Cherry-pick conflict: nothing applied**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-171., FR-NEW-186, FR-NEW-187, FR-NEW-188
- **Preconditions:** FX-FORK (conflicting): on `main` (tip C2, `/a.txt == b"a1\nMAIN\n"`), pick F1 which sets `/a.txt` to `"a1\nFEAT1\n"`. Snapshot volume and `main` tip.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F1>"}`.
- **Then** result `status == "conflict"`, `result["conflicts"] == ["/a.txt"]`.
- **And** exactly one `git_operations` row with `volume_id='proj1'`, `op_type='cherry_pick'`, in-progress state, and `todo` holding the single source sha `<sha-of-F1>`.
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` — unchanged.
- **And** `/a.txt == b"a1\nMAIN\n"` — byte-identical to the snapshot; no markers.
- **Priority:** P0.

---

**E2E-NEW-646 — Cherry-pick continue with `ours`**

**Category:** Happy. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-174.
- **Preconditions:** E2E-NEW-645 state.
- **When** `git.cherry_pick_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** `status == "committed"`, a new commit sha `!= <sha-of-F1>`.
- **And** `/a.txt == b"a1\nMAIN\n"` (ours = the current branch side).
- **And** `git.log{ref_name:"main"}` = 3 commits, `commits[0]["message"] == "F1 feature edit"`, `parents == [<sha-of-C2>[..8]]`.
- **And** zero `git_operations` rows.
- **And** with `"theirs"` instead (separate run from the same precondition), `/a.txt == b"a1\nFEAT1\n"`.
- **Priority:** P0.

---

**E2E-NEW-647 — Cherry-pick abort: exact restoration**

**Category:** Happy. **Scenario:** SC-919. **Requirements:** FR-NEW-239.
- **Preconditions:** E2E-NEW-645 state; `tip_before = <sha-of-C2>`, full volume byte map recorded before the pick.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}`.
- **Then** `entry.db.get_ref("refs/heads/main").target == tip_before`, and the on-disk `refs/heads/main` equals it too.
- **And** every volume path and its bytes equal the recorded map; the path set is identical.
- **And** zero `git_operations` rows.
- **And** a subsequent `git.cherry_pick_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`.
- **Priority:** P0.

---

**E2E-NEW-648 — Cherry-pick edge: commit already in current history**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
- **Preconditions:** FX-LINE, on `main` (tip `<sha-of-C4>`); `<sha-of-C2>` is an ancestor of the tip.
- **When** `git.cherry_pick {commit_sha:"<sha-of-C2>"}`.
- **Then** result `status == "already_applied"` (asserted verbatim) and `result["new_sha"] == "<sha-of-C2>"`, with a `message` containing `"already"` and `"history"`. *(Reported, not an error, not a duplicate.)*
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C4>` — tip unchanged.
- **And** `git.log{ref_name:"main"}` still has exactly 4 commits and `commits` contains `<sha-of-C2>` exactly once.
- **And** `bytes_written` and `audit().len()` unchanged.
- **Priority:** P0.

---

**E2E-NEW-649 — Cherry-pick edge: equivalent content already applied under a different sha**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
- **Preconditions:** FX-FORK-CLEAN on `main`; first run `git.cherry_pick {commit_sha:"<sha-of-F2>"}` successfully, producing `<sha-of-P1>` whose diff is identical to F2's.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}` a second time.
- **Then** the result is **not** `"already_applied"` by sha (F2 is not an ancestor), and the outcome is the documented empty-diff behaviour: `status == "empty"` with `message` containing `"no changes"`, and `entry.db.get_ref("refs/heads/main").target == <sha-of-P1>` (no empty commit created).
- **And** `git.log{ref_name:"main"}` still has exactly 3 commits.
- **Priority:** P1.

---

**E2E-NEW-650 — Cherry-pick edge: merge commit**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-240.
- **Preconditions:** FX-MERGE; on a separate branch `other` (off C1) so MG is not an ancestor.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>"}`.
- **Then** error `ERR_INVALID_ARGUMENT`, message containing `"merge commit"` and `"2 parents"` and naming `git.revert`'s `mainline` concept or stating the pick has no mainline parameter. Baseline: `git.cherry_pick: '<short-MG>' is a merge commit with 2 parents and cannot be cherry-picked; pick one of its parents instead`.
- **And** branch tip unchanged; zero rows; `bytes_written` unchanged.
- **Priority:** P0.

---

**E2E-NEW-651 — Cherry-pick edge: empty-diff commit**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** a branch `feature` whose commit `E1` (`E1 empty`) has a tree identical to its parent's (created by committing twice with no volume change, allowed since `git.commit` builds from the volume, `git.rs:664`). `main` is at C1.
- **When** `git.cherry_pick {commit_sha:"<sha-of-E1>"}` on `main`.
- **Then** `status == "empty"`, message contains `"no changes"`, tip unchanged, commit count on `main` unchanged, zero rows.
- **Priority:** P1.

---

**E2E-NEW-652 — Cherry-pick failure: unknown sha**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
- **When** `git.cherry_pick {commit_sha:"0000000000000000000000000000000000000000"}` on FX-LINE.
- **Then** `ERR_NOT_FOUND` with message containing `"0000000000000000000000000000000000000000"` and `"not found"` (matching `git.rs:727` phrasing).
- **And** tip unchanged, zero rows, `bytes_written` unchanged, `audit().len()` unchanged.
- **Priority:** P0.

---

**E2E-NEW-653 — Cherry-pick failure: malformed sha**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
- **When** `commit_sha` = `"zzzz"`, then `""`, then `"HEAD~1"`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"commit_sha"` (a non-hex value is an argument error, not a lookup miss; distinct from E2E-NEW-652). For `"HEAD~1"` the message additionally states that a full or abbreviated sha is required.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-654 — Cherry-pick failure: missing `commit_sha`**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **When** `git.cherry_pick {mount_id:"proj1"}`.
- **Then** `ERR_INVALID_ARGUMENT` naming `"commit_sha"`.
- **Priority:** P1.

---

**E2E-NEW-655 — Cherry-pick failure: continue with nothing in progress**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-224, FR-NEW-225.
- **Preconditions:** FX-LINE, no op in progress.
- **When** `git.cherry_pick_continue {mount_id:"proj1", resolutions:{"/a.txt":"ours"}}`.
- **Then** `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`.
- **And** a **rebase** paused (E2E-NEW-613) then `git.cherry_pick_continue {}` also fails with the same message — the two operations do not share a continue channel.
- **Priority:** P0.

---

**E2E-NEW-656 — Cherry-pick failure: abort with nothing in progress**

**Category:** Failure. **Scenario:** SC-919. **Requirements:** FR-NEW-239, FR-NEW-224, FR-NEW-225.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}` on FX-LINE.
- **Then** `ERR_INVALID_ARGUMENT` containing `"no cherry-pick in progress"`; tip unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-657 — Cherry-pick failure: empty repo (no HEAD target)**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-235.
- **Preconditions:** `git.init` only, no commit — the state where `git.log` returns `{"commits": []}` (`git.rs:569`).
- **When** `git.cherry_pick {commit_sha:"<any 40-hex>"}`.
- **Then** `ERR_NOT_FOUND` containing `"not found"` (the sha cannot exist) — and for a repo that has objects but an unborn HEAD, `ERR_INVALID_ARGUMENT` containing `"no commit on the current branch"`.
- **And** `entry.db.get_ref("refs/heads/main")` is still `None` (`db.rs:214`).
- **Priority:** P1.

---

**E2E-NEW-658 — Cherry-pick failure: quota exhausted**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-185.
- **Preconditions:** FX-FORK-CLEAN on `main`, built with `c.safety.write_quota_bytes` set so that the remaining quota is **1** byte while the pick must write `/f.txt` (3 bytes). *Method:* `Env::build(|c| c.safety.write_quota_bytes = N)` mirroring `safety.rs:339`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`.
- **Then** `ERR_WRITE_QUOTA_EXCEEDED` with message `session write quota of {N} bytes exceeded` (exact text, `safety.rs:135-137`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` — no commit created.
- **And** `/f.txt` absent from the volume; `/a.txt == b"a1\nMAIN\n"`.
- **And** `bytes_written()` unchanged (a rejected write consumes no quota, `safety.rs:349-352`).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

**E2E-NEW-659 — Cherry-pick failure: non-member caller**

**Category:** Security. **Scenario:** SC-918. **Requirements:** FR-NEW-100.
- **When** as `stranger@test.com`: `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort` on `proj1`.
- **Then** each fails `ERR_FORBIDDEN` with `'stranger@test.com' is not a member of 'proj1'`.
- **And** tip unchanged; zero rows.
- **Priority:** P0.

---

##### C. Reset (E2E-NEW-660 .. E2E-NEW-674)

---

**E2E-NEW-660 — Reset soft to C2**

**Category:** Happy. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
- **Preconditions:** FX-LINE on `main` (tip `<sha-of-C4>`); volume = C4's tree: `/a.txt == b"a1\na3\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"`.
- **When** `git.reset {mount_id:"proj1", target_ref:"<sha-of-C2>", mode:"soft"}`.
- **Then** result `status == "aborted"`, `result["restored_sha"] == "<sha-of-C2>"` (every abort tool reports `aborted` and the restored tip, per FR-NEW-199).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` exactly, and the on-disk ref matches.
- **And** `git.log{ref_name:"main"}` = exactly 2 commits, messages `["C2 add b","C1 base"]`.
- **And** volume unchanged: `/a.txt == b"a1\na3\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"` — soft does not touch the working tree.
- **And** `bytes_written()` unchanged (soft writes nothing).
- **Priority:** P0.

---

**E2E-NEW-661 — Reset hard to C2**

**Category:** Happy. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE on `main`, volume = C4's tree.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>`.
- **And** `git.log{ref_name:"main"}` = 2 commits `["C2 add b","C1 base"]`.
- **And** volume exactly equals C2's tree: `/a.txt == b"a1\n"` (C3's edit reverted), `/b.txt == b"b1\n"`, and `/d.txt` absent (`read_bytes` -> `ERR_NOT_FOUND`).
- **And** the volume path set is exactly `{"/a.txt","/b.txt"}`. *Method:* recursive `fs.list`.
- **Priority:** P0.

---

**E2E-NEW-662 — Reset hard deletes files added after the target**

**Category:** Happy. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE plus C5 (`C5 add e`, `/e.txt`=`"e1\n"`, nested `/sub/deep.txt`=`"deep\n"`), tip `<sha-of-C5>`.
- **When** `git.reset {target_ref:"<sha-of-C4>", mode:"hard"}`.
- **Then** `/e.txt` and `/sub/deep.txt` both absent (`ERR_NOT_FOUND` on read), and the directory `/sub` is gone from `fs.list` output.
- **And** the remaining path set is exactly `{"/a.txt","/b.txt","/d.txt"}` with bytes `"a1\na3\n"`, `"b1\n"`, `"d1\n"`.
- **And** tip == `<sha-of-C4>`.
- **Priority:** P0.

---

**E2E-NEW-663 — Reset hard on a dirty volume**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE on `main` at `<sha-of-C4>`; then, **without committing**, `Env::write("/a.txt", "DIRTY\n")` and `Env::write("/new.txt", "n\n")`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `/a.txt == b"a1\n"` (C2's content — the uncommitted edit is discarded).
- **And** `/new.txt` absent (`ERR_NOT_FOUND`) — an untracked-at-target file is removed by hard reset.
- **And** the path set is exactly `{"/a.txt","/b.txt"}`.
- **And** tip == `<sha-of-C2>`.
- **Priority:** P0.

---

**E2E-NEW-664 — Reset soft on a dirty volume keeps the dirt**

**Category:** EdgeCase. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
- **Preconditions:** same dirty state as E2E-NEW-663.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"soft"}`.
- **Then** `/a.txt == b"DIRTY\n"` and `/new.txt == b"n\n"` — untouched.
- **And** `/d.txt == b"d1\n"` still present although C4 is no longer reachable.
- **And** tip == `<sha-of-C2>`.
- **And** a following `git.commit {message:"after soft reset"}` produces a commit whose parent is `<sha-of-C2>` and whose tree contains `/a.txt`=`"DIRTY\n"`, `/b.txt`, `/d.txt`, `/new.txt` — proving soft preserved exactly the volume state (`git.rs:664`, `:679-688`).
- **Priority:** P0.

---

**E2E-NEW-665 — Reset to the current tip is a no-op in both modes**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-254.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; volume snapshot; `q0 = bytes_written()`; `obj0 = count_objects()`.
- **When (a)** `git.reset {target_ref:"<sha-of-C4>", mode:"soft"}`; **(b)** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** after each: tip string-equal to `<sha-of-C4>`; `git.log` JSON-equal to the pre-call log; every volume path and its bytes equal the snapshot; `count_objects() == obj0`.
- **And** for **(a)** `bytes_written() == q0`. For **(b)**, if the implementation rewrites the identical tree it may charge; the spec fixes the rule as **hard reset to the current tip writes nothing**, so `bytes_written() == q0` is asserted for (b) as well.
- **Priority:** P0.

---

**E2E-NEW-666 — Reset: orphaned commits survive as objects**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-255.
- **Preconditions:** E2E-NEW-661 executed (hard reset from C4 to C2).
- **Then** `entry.db.object_exists("<sha-of-C3>") == true` and `object_exists("<sha-of-C4>") == true` (`db.rs:131`).
- **And** `git.show {commit_sha:"<sha-of-C4>"}` succeeds, `commit.message == "C4 add d"`.
- **And** neither sha appears in `git.log` of any ref returned by `entry.db.list_refs()`.
- **And** `git.checkout_file {commit_sha:"<sha-of-C4>", path:"/d.txt"}` succeeds and writes `b"d1\n"` back into the volume — an orphan is still readable (`git.rs:224-257`).
- **Priority:** P0.

---

**E2E-NEW-667 — Reset: target_ref forms**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE plus a branch `old` pointing at `<sha-of-C2>` and a tag `v1` pointing at `<sha-of-C2>` (`entry.db.set_ref`, `db.rs:235`).
- **When** in three separate runs from the same start: `target_ref:"old"`, `target_ref:"refs/heads/old"`, `target_ref:"v1"`, each with `mode:"hard"`; plus `target_ref:"<sha-of-C2>[..8]"` (abbreviated).
- **Then** each yields `entry.db.get_ref("refs/heads/main").target == <sha-of-C2>` and volume path set `{"/a.txt","/b.txt"}`.
- **And** `target_ref:"HEAD"` resolves to the current branch tip and is a no-op (E2E-NEW-665 semantics).
- **Priority:** P1.

---

**E2E-NEW-668 — Reset side-effect: audit and quota for hard**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-251.
- **Preconditions:** FX-LINE at C4; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}` (writes `/a.txt` 3 bytes, `/b.txt` 3 bytes; deletes `/d.txt`).
- **Then** `audit()[n0..]` contains an entry with `op == "git.reset"` exactly, `detail` containing `"hard"` and `<sha-of-C2>`.
- **And** `bytes_written() - q0 == 6` (only bytes actually written; deletions are not charged, consistent with `charge_write` taking `num_bytes`, `safety.rs:131`). If the implementation skips rewriting a file whose bytes are already correct, the assertion becomes `== 3` — the spec fixes the rule as **write every file of the target tree**, so `6` is the asserted value.
- **And** a soft reset in the same session appends an audit entry with `op == "git.reset"` and `detail` containing `"soft"`, and adds `0` to `bytes_written`.
- **Priority:** P1.

---

**E2E-NEW-669 — Reset failure: unknown target_ref**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-253.
- **When** on FX-LINE: `git.reset {target_ref:"nope", mode:"hard"}`, then `target_ref:"0000000000000000000000000000000000000000"`.
- **Then** each fails `ERR_NOT_FOUND` with the given value in the message and the word `"not found"`.
- **And** tip == `<sha-of-C4>`; volume path set and bytes unchanged; `bytes_written` unchanged.
- **Priority:** P0.

---

**E2E-NEW-670 — Reset failure: missing mode**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-252.
- **When** `git.reset {mount_id:"proj1", target_ref:"<sha-of-C2>"}`.
- **Then** `ERR_INVALID_ARGUMENT` naming `"mode"`.
- **And** tip and volume unchanged.
- **And** `git.reset {mount_id:"proj1", mode:"hard"}` (no `target_ref`) fails `ERR_INVALID_ARGUMENT` naming `"target_ref"`.
- **Priority:** P0.

---

**E2E-NEW-671 — Reset failure: `mixed` is rejected**

**Category:** Failure. **Scenario:** SC-920. **Requirements:** FR-NEW-252.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"mixed"}`.
- **Then** `ERR_INVALID_ARGUMENT` whose message contains `"mixed"`, `"soft"` and `"hard"`. Baseline: `git.reset: mode must be 'soft' or 'hard', got 'mixed'` (same shape as `parse_on_conflict`, `git.rs:1481`), plus the reason: there is no index.
- **And** tip == `<sha-of-C4>`; volume unchanged; `bytes_written` unchanged.
- **And** the frozen `inputSchema` for `git.reset` lists `mode` with exactly the two allowed values in its description (checked against `tool-contract-golden.json`).
- **Priority:** P0.

---

**E2E-NEW-672 — Reset failure: case-sensitive mode**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-252.
- **When** `mode:"HARD"`, then `mode:"Soft"`, then `mode:""`, then `mode:"hard "` (trailing space).
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"soft"` and `"hard"`.
- **And** tip unchanged in all four cases.
- **Priority:** P1.

---

**E2E-NEW-673 — Reset failure: hard blocked by quota leaves everything in place**

**Category:** Failure. **Scenario:** SC-921. **Requirements:** FR-NEW-251, FR-NEW-185.
- **Preconditions:** FX-LINE at C4; env built with a quota that leaves 1 remaining byte; volume snapshot; `tip = <sha-of-C4>`.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of {N} bytes exceeded` (`safety.rs:135-137`).
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C4>` — the pointer did **not** move (quota is checked before the ref update; the operation is all-or-nothing).
- **And** every volume path and its bytes equal the snapshot, including `/d.txt == b"d1\n"`.
- **And** `bytes_written()` unchanged.
- **Priority:** P0.

---

**E2E-NEW-674 — Reset failure: non-member caller**

**Category:** Security. **Scenario:** SC-920. **Requirements:** FR-NEW-100.
- **When** as `stranger@test.com`: `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}`.
- **Then** `ERR_FORBIDDEN`, `'stranger@test.com' is not a member of 'proj1'`.
- **And** tip and volume unchanged.
- **Priority:** P0.

---

##### D. Revert (E2E-NEW-675 .. E2E-NEW-689)

---

**E2E-NEW-675 — Revert happy: revert C3**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; volume `/a.txt == b"a1\na3\n"`. C3's diff added the line `a3` to `/a.txt`.
- **When** `git.revert {mount_id:"proj1", commit_sha:"<sha-of-C3>"}`.
- **Then** `status == "committed"`, `result["new_sha"]` is a new 40-char sha differing from every seed sha.
- **And** `git.log{ref_name:"main"}` = exactly **5** commits; `commits[0]["message"] == "Revert \"C3 edit a\""` exactly (the standard revert subject; asserted verbatim), `commits[1]["message"] == "C4 add d"`.
- **And** `commits[0]["parents"] == [<sha-of-C4>[..8]]`, parent count 1.
- **And** `<sha-of-C3>` still appears in the log at index 2 — the original stays in history.
- **And** volume: `/a.txt == b"a1\n"`, `/b.txt == b"b1\n"`, `/d.txt == b"d1\n"` (C4's addition survives; only C3's diff is inverted).
- **And** zero `git_operations` rows.
- **Priority:** P0.

---

**E2E-NEW-676 — Revert happy: revert a file-add commit**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; C4 added `/d.txt`.
- **When** `git.revert {commit_sha:"<sha-of-C4>"}`.
- **Then** `/d.txt` absent (`ERR_NOT_FOUND`); path set exactly `{"/a.txt","/b.txt"}`; `/a.txt == b"a1\na3\n"` unchanged.
- **And** `git.log` = 5 commits with `commits[0]["message"] == "Revert \"C4 add d\""`.
- **And** the revert commit's tree is identical to C3's tree: `git.diff {from_ref:"<sha-of-C3>", to_ref: result["new_sha"]}` returns an empty diff.
- **Priority:** P0.

---

**E2E-NEW-677 — Revert happy: reverting a revert reapplies**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-265.
- **Preconditions:** E2E-NEW-676 executed; `<sha-of-R1>` = the revert commit; `/d.txt` absent.
- **When** `git.revert {commit_sha:"<sha-of-R1>"}`.
- **Then** `/d.txt == b"d1\n"` again, byte-identical to the original.
- **And** `git.log{ref_name:"main"}` = 6 commits, `commits[0]["message"] == "Revert \"Revert \\\"C4 add d\\\"\""` — i.e. the literal string `Revert "Revert \"C4 add d\""` as git renders nested reverts; asserted as the exact string produced by the implementation's subject rule, which the spec fixes as `Revert "<subject of the reverted commit>"`.
- **And** the new tip's tree is identical to C4's tree: `git.diff {from_ref:"<sha-of-C4>", to_ref: tip}` is empty.
- **And** all three commits C4, R1, R2 remain reachable with distinct shas.
- **Priority:** P0.

---

**E2E-NEW-678 — Revert merge commit with mainline=1**

**Category:** Happy. **Scenario:** SC-923. **Requirements:** FR-NEW-261.
- **Preconditions:** FX-MERGE; `main` tip = `<sha-of-MG>` (parents `[<sha-of-M1>, <sha-of-S1>]`); volume has `/a.txt`,`/m.txt`,`/s.txt`.
- **When** `git.revert {commit_sha:"<sha-of-MG>", mainline:1}`.
- **Then** `status == "committed"`; the revert is computed against parent 1 (`M1`), so the *side* branch's contribution is removed: `/s.txt` absent, `/m.txt == b"m1\n"`, `/a.txt == b"a1\n"`.
- **And** `git.log{ref_name:"main"}` `commits[0]["message"] == "Revert \"Merge side into main\""` and `commits[0]["parents"] == [<sha-of-MG>[..8]]`, parent count 1 (the revert itself is not a merge).
- **And** `<sha-of-MG>` remains reachable at index 1.
- **Priority:** P0.

---

**E2E-NEW-679 — Revert merge commit with mainline=2**

**Category:** EdgeCase. **Scenario:** SC-923. **Requirements:** FR-NEW-261, FR-NEW-262.
- **Preconditions:** FX-MERGE, same as above.
- **When** `git.revert {commit_sha:"<sha-of-MG>", mainline:2}`.
- **Then** the revert is computed against parent 2 (`S1`), so the *main* branch's contribution is removed: `/m.txt` absent, `/s.txt == b"s1\n"`, `/a.txt == b"a1\n"`.
- **And** the resulting tree differs from the E2E-NEW-678 result: the two runs produce different tip trees (asserted by comparing the two path sets: `{"/a.txt","/m.txt"}` vs `{"/a.txt","/s.txt"}`).
- **And** commit count on `main` == 5 in both runs (C1, M1/S1, MG, revert — exact list asserted per fixture layout).
- **Priority:** P0.

---

**E2E-NEW-680 — Revert the initial commit (no parent)**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-263.
- **Preconditions:** FX-INIT: only `C1 base` with `/a.txt == b"a1\n"`; tip `<sha-of-C1>`.
- **When** `git.revert {commit_sha:"<sha-of-C1>"}`.
- **Then** `status == "committed"`; the inverse of "add `/a.txt` against the empty tree" removes it: `/a.txt` absent, volume path set is **empty**.
- **And** `git.log{ref_name:"main"}` = 2 commits, `commits[0]["message"] == "Revert \"C1 base\""`, `parents == [<sha-of-C1>[..8]]`.
- **And** the revert commit's tree is the empty tree (`git.show` lists zero files).
- **And** `<sha-of-C1>` still reachable.
- **Priority:** P0.

---

**E2E-NEW-681 — Revert a non-tip commit**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; C2 added `/b.txt`; C3 and C4 do not touch `/b.txt`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}`.
- **Then** `/b.txt` absent; `/a.txt == b"a1\na3\n"` and `/d.txt == b"d1\n"` untouched (later commits are preserved).
- **And** `git.log` = 5 commits, `commits[0]["message"] == "Revert \"C2 add b\""`.
- **Priority:** P1.

---

**E2E-NEW-682 — Revert side-effect: audit, quota, original reachable**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **Preconditions:** FX-LINE at C4; `n0 = audit().len()`, `q0 = bytes_written()`.
- **When** `git.revert {commit_sha:"<sha-of-C3>"}` (rewrites `/a.txt` to 3 bytes).
- **Then** `audit()[n0..]` contains an entry with `op == "git.revert"` exactly and `detail` containing `<sha-of-C3>`.
- **And** `bytes_written() - q0 == 3` (only `/a.txt` is rewritten; `/b.txt`,`/d.txt` are unchanged by the inverse diff and are not re-written).
- **And** `entry.db.object_exists("<sha-of-C3>") == true` and `<sha-of-C3>` is in `git.log` — the original is untouched.
- **Priority:** P1.

---

**E2E-NEW-683 — Revert conflict: inverse diff does not apply**

**Category:** Failure. **Scenario:** SC-922. **Requirements:** FR-NEW-264.
- **Preconditions:** FX-LINE at C4, plus C5 (`C5 rewrite a`) setting `/a.txt` = `"TOTALLY DIFFERENT\n"`, tip `<sha-of-C5>`. Reverting C3 (which added line `a3` to a file that no longer contains it) cannot apply cleanly.
- **When** `git.revert {commit_sha:"<sha-of-C3>"}`.
- **Then** `status == "conflict"`, `result["conflicts"] == ["/a.txt"]`.
- **And** exactly one `git_operations` row with `volume_id='proj1'`, `op_type='revert'`, in-progress state, `todo` holding `<sha-of-C3>`.
- **And** `entry.db.get_ref("refs/heads/main").target == <sha-of-C5>` — unchanged.
- **And** `/a.txt == b"TOTALLY DIFFERENT\n"` byte-identical; no conflict markers anywhere (`fs.grep` for marker lines returns zero matches).
- **Priority:** P0.

---

**E2E-NEW-684 — Revert conflict: continue resolves, abort restores exactly**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-264, FR-NEW-284., FR-NEW-186
- **Preconditions:** E2E-NEW-683 state; `tip_before = <sha-of-C5>`; full volume byte map.
- **When (a)** `git.revert_continue {mount_id:"proj1", resolutions:{"/a.txt":{"content":"RESOLVED\n"}}}`.
- **Then (a)** `status == "committed"`; `/a.txt == b"RESOLVED\n"`; `git.log` `commits[0]["message"] == "Revert \"C3 edit a\""` with `parents == [<sha-of-C5>[..8]]`; zero `git_operations` rows.
- **When (b)** (separate run from the same precondition) `git.revert_abort {mount_id:"proj1"}`.
- **Then (b)** `entry.db.get_ref("refs/heads/main").target == tip_before`; every volume path and its bytes equal the map; zero rows; a second `git.revert_abort {}` fails `ERR_INVALID_ARGUMENT` containing `"no revert in progress"`.
- **And (b)** `git.revert_continue {}` with nothing in progress fails `ERR_INVALID_ARGUMENT` containing `"no revert in progress"`.
- **Priority:** P0.

---

**E2E-NEW-685 — Revert failure: merge commit without mainline**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-261.
- **Preconditions:** FX-MERGE at `<sha-of-MG>`; volume snapshot.
- **When** `git.revert {commit_sha:"<sha-of-MG>"}` (no `mainline`).
- **Then** `ERR_INVALID_ARGUMENT` whose message contains `"merge commit"`, `"mainline"`, and `"1"` and `"2"`. Baseline: `git.revert: '<short-MG>' is a merge commit with 2 parents; pass mainline (1 or 2) to choose which parent the revert is computed against`.
- **And** tip == `<sha-of-MG>`; volume path set and bytes equal the snapshot; `bytes_written` unchanged; zero rows.
- **Priority:** P0.

---

**E2E-NEW-686 — Revert failure: mainline out of range**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **Preconditions:** FX-MERGE (2 parents).
- **When** `mainline: 3`, then `mainline: 99`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"mainline"`, the supplied number, and `"2 parents"`. Baseline: `git.revert: mainline 3 is out of range, '<short-MG>' has 2 parents`.
- **And** tip unchanged; volume unchanged.
- **Priority:** P0.

---

**E2E-NEW-687 — Revert failure: mainline is 1-based**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **When** `mainline: 0`, then `mainline: -1`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"1"` (stating parents are numbered from 1).
- **And** `mainline: "1"` (string instead of integer) fails `ERR_INVALID_ARGUMENT` naming `"mainline"` and `"integer"`.
- **And** tip unchanged.
- **Priority:** P0.

---

**E2E-NEW-688 — Revert failure: mainline supplied for a non-merge commit**

**Category:** Failure. **Scenario:** SC-923. **Requirements:** FR-NEW-262.
- **Preconditions:** FX-LINE at C4; C3 has exactly 1 parent.
- **When** `git.revert {commit_sha:"<sha-of-C3>", mainline:1}`.
- **Then** `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"not a merge commit"` — the argument is refused rather than silently ignored, so a caller confusing two commits learns about it.
- **And** tip == `<sha-of-C4>`; volume unchanged; zero rows.
- **Priority:** P1.

---

**E2E-NEW-689 — Revert failure: unknown and malformed sha**

**Category:** Failure. **Scenario:** SC-922. **Requirements:** FR-NEW-260.
- **When** on FX-LINE: `commit_sha:"0000000000000000000000000000000000000000"`; then `commit_sha:"zzz"`; then no `commit_sha`.
- **Then** respectively `ERR_NOT_FOUND` containing the sha and `"not found"`; `ERR_INVALID_ARGUMENT` containing `"commit_sha"`; `ERR_INVALID_ARGUMENT` naming `"commit_sha"`.
- **And** in all three cases tip == `<sha-of-C4>`, volume bytes unchanged, `bytes_written` unchanged, `audit().len()` unchanged, zero rows.
- **Priority:** P0.

---

##### Cross-cutting (E2E-NEW-690 .. E2E-NEW-699)

---

**E2E-NEW-690 — All eight new tools require `mount_id`**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-345, FR-NEW-347.
- **Preconditions:** FX-LINE.
- **When** each of `git.rebase`, `git.rebase_continue`, `git.rebase_abort`, `git.cherry_pick`, `git.cherry_pick_continue`, `git.cherry_pick_abort`, `git.reset`, `git.revert` (plus `git.revert_continue`, `git.revert_abort` if the revert pair is registered) is called with its other required args but no `mount_id`.
- **Then** each fails `ERR_INVALID_ARGUMENT` naming `"mount_id"` — matching the convention that `mount_id` is required on every `git.*` tool (`git.rs:209`, `:236`, and the schema builders at `:196-199`).
- **Priority:** P0.

---

**E2E-NEW-691 — Unknown `mount_id` on all new tools**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-347.
- **When** each new tool is called with `mount_id:"does-not-exist"` and otherwise valid args.
- **Then** each fails `ERR_PROJECT_NOT_FOUND` with message `project 'does-not-exist' not found` (exact phrasing, `errors.rs:249`).
- **And** `proj1`'s tip and volume are untouched.
- **Priority:** P0.

---

**E2E-NEW-692 — Registry count and names**

**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-346.
- **When** `register(&mut ToolRegistry::new())`.
- **Then** `r.len() == 45` (14 today at `git.rs:2500`, plus the 31 tools enumerated in FR-NEW-349, which includes `git.revert_continue` and `git.revert_abort`). The test asserts the number matching the shipped `ALL_GIT_TOOLS` array, which must be updated in the same change.
- **And** every new name resolves through `r.resolve(name)`, including its underscore form (`git_rebase_continue`) per the dot/underscore-tolerant resolver (AGENTS.md, `mcp/registry.rs`).
- **Priority:** P0.

---

**E2E-NEW-693 — Frozen tool contract**

**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345.
- **When** the golden contract test runs (`tools/contract_golden.rs`).
- **Then** `tool-contract-golden.json` contains each new tool with its exact `description` and serialized `inputSchema`, and `TOOL_CONTRACT.txt` documents each new tool's parameters and return shape.
- **And** the test fails if any parameter description differs by one character (the contract is frozen; regeneration only via `MCPFS_REWRITE_TOOL_CONTRACT=1`).
- **And** the header count in `AGENTS.md`/`TOOL_CONTRACT.txt` (`63 tools`) is updated to match.
- **Priority:** P0.

---

**E2E-NEW-694 — Write lock serializes the new ops**

**Category:** Concurrency. **Scenario:** SC-916. **Requirements:** FR-NEW-226.
- **Preconditions:** FX-FORK-CLEAN; a long-running rebase (todo of 50 clean picks on a fixture with 50 commits).
- **When** the rebase future and a `git.commit {message:"racer"}` future are driven concurrently with `tokio::join!`.
- **Then** both complete without error, and the final `git.log{ref_name:"feature"}` is internally consistent: exactly `50 + 1 (racer) + 2` commits, each with exactly 1 parent, and the parent chain is unbroken (every `commits[i]["parents"][0] == commits[i+1]["sha"][..8]`).
- **And** no commit is missing from the chain — proving the rebase held `entry.write_lock` (`repo.rs:44`) for its whole duration exactly as `git.commit` does (`git.rs:660`).
- **Priority:** P0.

---

**E2E-NEW-695 — `git_operations` rows are volume-scoped**

**Category:** DataIntegrity. **Scenario:** SC-929. **Requirements:** FR-NEW-275, FR-NEW-277, FR-MOD-108.
- **Preconditions:** two projects `proj1` and `proj2`, both members of `OWNER`, each with FX-FORK conflicting seeded independently.
- **When** a rebase is paused in `proj1` **and** a cherry-pick is paused in `proj2`.
- **Then** `SELECT ... WHERE volume_id='proj1'` returns exactly 1 row with `op_type='rebase'`, and `volume_id='proj2'` exactly 1 row with `op_type='cherry_pick'`.
- **And** `git.rebase_abort {mount_id:"proj1"}` leaves `proj2`'s row present and its branch tip and volume untouched.
- **And** `git.rebase_continue {mount_id:"proj2"}` fails `ERR_INVALID_ARGUMENT` containing `"no rebase in progress"` — `proj2` has a cherry-pick, not a rebase.
- **Priority:** P0.

---

**E2E-NEW-696 — Unauthenticated caller**

**Category:** Security. **Scenario:** SC-929. **Requirements:** FR-NEW-100, FR-NEW-347.
- **Preconditions:** the server route path with no valid identity (no forwarded header, no `Authorization`; `identity.rs`).
- **When** any of the eight new tools is invoked over `/mcp`.
- **Then** the response carries `ERR_UNAUTHENTICATED` and the operation is not performed (tip unchanged).
- **Priority:** P0.

---

**E2E-NEW-697 — Paused operation survives a repo-store reopen**

**Category:** DataIntegrity. **Scenario:** SC-916. **Requirements:** FR-NEW-278.
- **Preconditions:** FX-FORK conflicting; rebase paused at F1; record the row's `current_step`, `todo`, `conflicts`.
- **When** the in-process `GitRepoStore` entry is dropped and reopened via `get_or_open_repo` (the cold-open path documented at `repo.rs:19-21`).
- **Then** the `git_operations` row is still present with identical `current_step`, `todo`, `conflicts`.
- **And** `git.rebase_continue {resolutions:{"/a.txt":"theirs"}}` completes with the same result asserted in E2E-NEW-614.
- **Priority:** P1.

---

**E2E-NEW-698 — Rebase and cherry-pick are mutually exclusive**

**Category:** Failure. **Scenario:** SC-929. **Requirements:** FR-NEW-283.
- **Preconditions:** FX-FORK conflicting; rebase paused.
- **When** `git.cherry_pick {commit_sha:"<sha-of-F2>"}`, then `git.revert {commit_sha:"<sha-of-C2>"}`, then `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}`.
- **Then** each fails `ERR_INVALID_ARGUMENT` containing `"rebase in progress"` and naming `git.rebase_continue` / `git.rebase_abort`.
- **And** still exactly 1 `git_operations` row with unchanged `current_step`; branch tip and volume unchanged.
- **Priority:** P1.

---

**E2E-NEW-699 — Ref duality: db and on-disk agree after every new mutating op**

**Category:** DataIntegrity. **Scenario:** SC-929. **Requirements:** FR-NEW-115, FR-NEW-226.
- **Preconditions:** FX-LINE / FX-FORK-CLEAN / FX-MERGE as needed.
- **When** in sequence on the same env: a clean rebase, a cherry-pick, a `reset --soft`, a `reset --hard`, a revert, and a `rebase_abort` after a paused rebase.
- **Then** after **each** step, for every ref in `entry.db.list_refs()` (`db.rs:261`) that is non-symbolic, `repo.find_reference(name).target().to_string()` equals the db `target` — the dual write invariant established by `git.commit` (`git.rs:702-711`) holds for every new operation.
- **And** the symbolic `HEAD` row still points at the expected `refs/heads/...` and `repo.head()` agrees.
- **And** `entry.objects` contains every new commit sha (`object_exists`, `db.rs:131`), proving the `import_from_repo` step (`git.rs:700`) was not skipped by the new code paths.
- **Priority:** P0.

---

##### C.z Implementation notes (band C)

1. Five assertions in this spec pin a decision the conflict-model author left open. Each is written as a **fixed baseline** so the test is writable without guessing: `current_step` identifies the todo entry (E2E-NEW-613), branch == onto is a rejection not a no-op (E2E-NEW-608), `todo` bound is 1000 (E2E-NEW-626), hard reset writes every file of the target tree (E2E-NEW-665, E2E-NEW-668), cherry-pick of a merge is rejected (E2E-NEW-650). If the implementation chooses otherwise, change the baseline in `TOOL_CONTRACT.txt` and the test together, never only the test.
2. The revert conflict pair `git.revert_continue` / `git.revert_abort` is now mandated explicitly by FR-NEW-241 and FR-NEW-266, and is exercised by E2E-NEW-684. Revert conflicts are NOT routed through `git.cherry_pick_continue`; FR-NEW-225 forbids it.
3. Tests live next to the existing git tool tests (`crates/mcp-fs/src/tools/git.rs` `mod tests`, harness at `:2437-2476`), reusing `Env`, `MOUNT`, `OWNER`. Byte-level volume assertions need a `read_bytes` helper alongside the existing `read` (`git.rs:2465`), and a `git_operations` row-count helper on the test side.
4. Quality gate unchanged: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

#### Band D. Pull request tools, provider normalization, OAuth scope

##### 1. Shared harness (build once; every test below references it)

##### 1.1 Injectable provider API client

Mirror `DeviceFlowClient` exactly (`device_flow.rs:127-138`):

```rust
#[async_trait]
pub trait ProviderApiClient: Send + Sync {
    async fn send(&self, req: ProviderRequest) -> Result<ProviderResponse>;
}
pub struct ProviderRequest {
    pub method: &'static str,            // "GET" | "POST" | "PUT"
    pub base_url: String,                // e.g. "https://api.github.com"
    pub path: String,                    // e.g. "/repos/acme/api/pulls/42"
    pub query: Vec<(String, String)>,
    pub accept: Option<String>,          // e.g. "application/vnd.github.v3.diff"
    pub body: Option<serde_json::Value>,
    pub token: String,                   // supplied, never logged
}
pub struct ProviderResponse { pub status: u16, pub headers: Vec<(String,String)>, pub body: String }
```

`register_with(reg, tokens: Option<Arc<OAuthTokenStore>>, flow: Option<Arc<dyn DeviceFlowClient>>, api: Option<Arc<dyn ProviderApiClient>>)` extends the existing injection point at `tools/git_auth.rs:66-70` and the PR tools register through the git family.

##### 1.2 `MockProviderApi`

- Route table: `HashMap<(method, path), Vec<ProviderResponse>>` (a queue, so repeated calls can return different responses).
- Records every `ProviderRequest` in order, including `base_url`, `query`, `accept`, `body`, and an `authorization_seen: String` built by the client (`"Bearer <token>"` for GitLab, `"token <token>"` for GitHub).
- `calls()` -> `Vec<(method, base_url, path, query, accept, body)>`; **the recorded struct MUST NOT expose the token except through `authorization_seen`, used only by security tests**.
- An unrouted request returns `ProviderResponse { status: 599, body: "UNROUTED" }` so "no network call expected" tests can assert `mock.calls().is_empty()` AND never accidentally pass by a silent default.

##### 1.3 Fixture config (`Fixture::with_config`, as in `git_auth.rs:597-604`)

```yaml
git:
  enabled: true
  hosts:
    github.com: github
    github.ibm.com: github
    gitlab.com: gitlab
    gitlab.example.test: gitlab
    git.acme.internal: generic
    public.example.org: anonymous
```

Mounts (all with `git.init` run, `main` branch, one commit), members: `dev@test.com` only.

| mount_id | origin URL |
|---|---|
| `acme-api` | `https://github.com/acme/api.git` |
| `acme-ent` | `https://github.ibm.com/acme/api.git` |
| `acme-lab` | `https://gitlab.com/acme/api.git` |
| `acme-self` | `https://gitlab.example.test/acme/api.git` |
| `acme-generic` | `https://git.acme.internal/acme/api.git` |
| `acme-pub` | `https://public.example.org/acme/api.git` |
| `acme-nor` | no origin configured |

People: `dev@test.com` (member of all above), `outsider@test.com` (member of none), `anon` (no identity header).

Tokens seeded via `OAuthTokenStore::store_token` (`store.rs:134-170`):

| person | host | provider | token | scopes | expires_at | instance_url |
|---|---|---|---|---|---|---|
| dev@test.com | github.com | github | `ghp_TESTTOKEN_github_001` | `["repo"]` | now+1h | None |
| dev@test.com | github.ibm.com | github | `ghp_TESTTOKEN_ghes_002` | `["repo"]` | None | `https://github.ibm.com` |
| dev@test.com | gitlab.com | gitlab | `glpat_TESTTOKEN_gl_003` | `["api","read_repository","write_repository"]` | now+1h | None |
| dev@test.com | gitlab.example.test | gitlab | `glpat_TESTTOKEN_self_004` | `["api"]` | None | `https://gitlab.example.test` |

Narrow/expired variants are seeded per test.

##### 1.4 Endpoint contract the mock is asserted against

Base URL resolution (host -> base):
- `github.com` -> `https://api.github.com`
- any other `github` host H -> `https://{H}/api/v3` (GHES), or `{instance_url}/api/v3` when `instance_url` is stored
- `gitlab.com` -> `https://gitlab.com/api/v4`
- any other `gitlab` host H -> `{instance_url or https://H}/api/v4`

Project slug: path of the origin URL minus leading `/` and trailing `.git` -> `acme/api`; GitLab uses the percent-encoded form `acme%2Fapi`.

| Operation | GitHub | GitLab |
|---|---|---|
| head-branch pre-flight | `GET /repos/acme/api/branches/{head}` | `GET /projects/acme%2Fapi/repository/branches/{head}` |
| create | `POST /repos/acme/api/pulls` | `POST /projects/acme%2Fapi/merge_requests` |
| list | `GET /repos/acme/api/pulls?state=&per_page=100` | `GET /projects/acme%2Fapi/merge_requests?state=&per_page=100` |
| get | `GET /repos/acme/api/pulls/{n}` + `GET /repos/acme/api/pulls/{n}/reviews` + `GET /repos/acme/api/commits/{head_sha}/check-runs` | `GET /projects/acme%2Fapi/merge_requests/{n}` + `GET /projects/acme%2Fapi/merge_requests/{n}/approvals` (pipeline read from `head_pipeline` in the get payload) |
| diff | `GET /repos/acme/api/pulls/{n}` with `Accept: application/vnd.github.v3.diff` | `GET /projects/acme%2Fapi/merge_requests/{n}/changes` |
| merge | `PUT /repos/acme/api/pulls/{n}/merge` body `{"merge_method":"merge\|squash\|rebase"}` | `PUT /projects/acme%2Fapi/merge_requests/{n}/merge` body `{"squash":bool}`; `rebase` = `POST .../merge_requests/{n}/rebase` then the same PUT |
| review approve | `POST /repos/acme/api/pulls/{n}/reviews` `{"event":"APPROVE"}` | `POST /projects/acme%2Fapi/merge_requests/{n}/approve` |
| review request_changes | same path, `{"event":"REQUEST_CHANGES","body":...}` | `POST .../merge_requests/{n}/unapprove` then `POST .../merge_requests/{n}/notes` `{"body":...}` |
| review comment | same path, `{"event":"COMMENT","body":...}` | `POST .../merge_requests/{n}/notes` |

State mapping for `pr_list`: `open`->GitHub `open` / GitLab `opened`; `closed`->`closed`/`closed`; `merged`->GitHub `closed` + client-side `merged_at != null` / GitLab `merged`; `all`->`all`/`all`.

##### 1.5 The normalized model (asserted key-for-key)

`pr_get` / `pr_create` / `pr_merge` / `pr_review` return an object with EXACTLY these keys, in this order:

```json
{
  "provider": "github",
  "host": "github.com",
  "number": 42,
  "title": "Add PR tools",
  "body": "Adds git.pr_* over the provider REST API.",
  "state": "open",
  "draft": false,
  "base": "main",
  "head": "feature/pr-tools",
  "author": "smorand",
  "url": "https://github.com/acme/api/pull/42",
  "created_at": "2026-09-01T10:00:00+00:00",
  "updated_at": "2026-09-02T11:30:00+00:00",
  "commits": 3,
  "changed_files": 4,
  "additions": 120,
  "deletions": 11,
  "review_state": "approved",
  "checks_state": "success",
  "mergeable": true,
  "raw": { }
}
```

- `state` in `{open, closed, merged}`. `review_state` in `{approved, changes_requested, review_required, none}`. `checks_state` in `{success, failure, pending, none}`. `mergeable` is `true|false|null`.
- `raw` is the verbatim provider get payload (for GitLab, the MR object; nothing else).
- `pr_list` returns `{"pull_requests":[item...], "count": N}` where each item has EXACTLY the FR-NEW-317 key set minus `commits`, `changed_files`, `additions`, `deletions` and `mergeable`: `provider, host, number, title, body, state, draft, base, head, author, url, created_at, updated_at, review_state, checks_state, raw`.
- `pr_diff` returns `{"provider", "host", "number", "diff", "bytes", "truncated"}`.

##### 1.6 Canned payloads (constants reused by tests)

`GH_PR_42_OPEN` (`GET /repos/acme/api/pulls/42`, 200):
```json
{"number":42,"title":"Add PR tools","body":"Adds git.pr_* over the provider REST API.",
 "state":"open","draft":false,"merged":false,"merged_at":null,"mergeable":true,
 "base":{"ref":"main"},"head":{"ref":"feature/pr-tools","sha":"9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3"},
 "user":{"login":"smorand"},"html_url":"https://github.com/acme/api/pull/42",
 "created_at":"2026-09-01T10:00:00Z","updated_at":"2026-09-02T11:30:00Z",
 "commits":3,"changed_files":4,"additions":120,"deletions":11}
```
`GH_REVIEWS_APPROVED` (`/pulls/42/reviews`, 200): `[{"state":"APPROVED","user":{"login":"swilbert"},"submitted_at":"2026-09-02T09:00:00Z"}]`
`GH_CHECKS_SUCCESS` (`/commits/9f1c.../check-runs`, 200): `{"total_count":2,"check_runs":[{"name":"ci/build","status":"completed","conclusion":"success"},{"name":"ci/test","status":"completed","conclusion":"success"}]}`

`GL_MR_42_OPEN` (`GET /projects/acme%2Fapi/merge_requests/42`, 200):
```json
{"iid":42,"title":"Add PR tools","description":"Adds git.pr_* over the provider REST API.",
 "state":"opened","draft":false,"merge_status":"can_be_merged",
 "target_branch":"main","source_branch":"feature/pr-tools",
 "author":{"username":"smorand"},"web_url":"https://gitlab.com/acme/api/-/merge_requests/42",
 "created_at":"2026-09-01T10:00:00.000Z","updated_at":"2026-09-02T11:30:00.000Z",
 "changes_count":"4","sha":"9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3",
 "head_pipeline":{"id":881,"status":"success"},
 "diff_stats_summary":{"additions":120,"deletions":11,"commit_count":3}}
```
`GL_APPROVALS_APPROVED` (`/merge_requests/42/approvals`, 200): `{"approved":true,"approvals_required":1,"approvals_left":0,"approved_by":[{"user":{"username":"swilbert"}}]}`

`UNIFIED_DIFF_SMALL` (plain text, 4 files, first hunk):
```
diff --git a/src/tools/pr.rs b/src/tools/pr.rs
new file mode 100644
--- /dev/null
+++ b/src/tools/pr.rs
@@ -0,0 +1,3 @@
+pub fn register() {}
```

---

##### 3. Full specifications

Common preconditions for all tests unless overridden: the fixture of 1.3; registry built with `register_with(&mut reg, Some(tokens), None, Some(mock))`; caller identity `dev@test.com`. Common cleanup: drop the fixture tempdir, `tokens.revoke_token` for every seeded pair, clear the mock route table, release the `git.hosts` global lock (`remote.rs:761` `lock_for_test()` guard, held for the whole test because `validate_hosts` publishes a process-global map).

##### Base URL and host resolution

**E2E-NEW-700 — Happy — P0.** Mock routes `GET /repos/acme/api/pulls` -> 200 `[]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
Given mount `acme-api` with origin `https://github.com/acme/api.git` and a valid `github.com` token.
When `git.pr_list{mount_id:"acme-api", state:"open"}`.
Then result is `{"pull_requests":[],"count":0}`; And `mock.calls()` has length 1 and equals `("GET", "https://api.github.com", "/repos/acme/api/pulls", [("state","open"),("per_page","100")], None, None)`.
Cleanup: standard.

**E2E-NEW-701 — Happy — P0.** Same route under base `https://github.ibm.com/api/v3`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
Given mount `acme-ent`, token for `github.ibm.com` with `instance_url = "https://github.ibm.com"`.
When `git.pr_list{mount_id:"acme-ent", state:"open"}`.
Then the single recorded call has `base_url == "https://github.ibm.com/api/v3"` and `path == "/repos/acme/api/pulls"`; And result `count == 0`.

**E2E-NEW-702 — Happy — P0.** Route `GET /projects/acme%2Fapi/merge_requests` -> 200 `[]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then recorded call `("GET","https://gitlab.com/api/v4","/projects/acme%2Fapi/merge_requests",[("state","opened"),("per_page","100")])`; And `count == 0`.

**E2E-NEW-703 — Happy — P0.** Token for `gitlab.example.test` carries `instance_url = "https://gitlab.example.test"`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_list{mount_id:"acme-self", state:"open"}`.
Then `base_url == "https://gitlab.example.test/api/v4"`; And path as in E2E-NEW-702; And exactly 1 call.

**E2E-NEW-704 — Failure — P0.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-301.
When `git.pr_list{mount_id:"acme-generic", state:"open"}`.
Then `err.code == "ERR_NOT_SUPPORTED"`; And `err.message` contains `host 'git.acme.internal' is declared 'generic' in git.hosts; pull request tools exist only for github and gitlab`; And `mock.calls().is_empty()`.

**E2E-NEW-705 — Failure — P0.** Same with `acme-pub`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-301.
Then `ERR_NOT_SUPPORTED`; message contains `host 'public.example.org' is declared 'anonymous' in git.hosts`; And no calls.

**E2E-NEW-706 — Failure — P0.** Extra mount `acme-sh` with origin `https://git.sourcehut.test/acme/api.git`, host NOT in `git.hosts`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-300.
When `git.pr_get{mount_id:"acme-sh", pr_number:42}`.
Then `ERR_INVALID_ARGUMENT`; message contains `host 'git.sourcehut.test' is not declared in git.hosts` (same shape as `tools/git.rs:820-825`); And no calls.

**E2E-NEW-707 — Failure — P0.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-302.
When `git.pr_list{mount_id:"acme-nor", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `volume 'acme-nor' has no origin remote` (`remote.rs:448`); And no calls.

**E2E-NEW-708 — Failure — P1.** Mount `acme-scp` with origin `git@github.com:acme/api.git`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-302.
When `git.pr_list{mount_id:"acme-scp", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `remote url 'git@github.com:acme/api.git'` and `scp`; And no calls.

**E2E-NEW-709 — Integration — P0. The normalization proof.**

**Category:** Integration. **Scenario:** SC-927. **Requirements:** FR-NEW-303., FR-NEW-317
Preconditions: GitHub routes `/repos/acme/api/pulls/42` -> `GH_PR_42_OPEN`, `/pulls/42/reviews` -> `GH_REVIEWS_APPROVED`, `/commits/9f1c.../check-runs` -> `GH_CHECKS_SUCCESS`. GitLab routes `/projects/acme%2Fapi/merge_requests/42` -> `GL_MR_42_OPEN`, `/merge_requests/42/approvals` -> `GL_APPROVALS_APPROVED`.
When `git.pr_get{mount_id:"acme-api", pr_number:42}` -> `gh`, then `git.pr_get{mount_id:"acme-lab", pr_number:42}` -> `gl`.
Then `gh.as_object().keys().collect::<Vec<_>>() == gl.as_object().keys().collect::<Vec<_>>()` (order included); And for every key except `provider`, `host`, `url`, `raw`: `gh[k] == gl[k]`, specifically `number==42`, `title=="Add PR tools"`, `body=="Adds git.pr_* over the provider REST API."`, `state=="open"`, `draft==false`, `base=="main"`, `head=="feature/pr-tools"`, `author=="smorand"`, `created_at=="2026-09-01T10:00:00+00:00"`, `updated_at=="2026-09-02T11:30:00+00:00"`, `commits==3`, `changed_files==4`, `additions==120`, `deletions==11`, `review_state=="approved"`, `checks_state=="success"`, `mergeable==true`; And `gh["provider"]=="github"`, `gl["provider"]=="gitlab"`; And `gh["url"]=="https://github.com/acme/api/pull/42"`, `gl["url"]=="https://gitlab.com/acme/api/-/merge_requests/42"`; And `gh["raw"]["head"]["sha"]` exists while `gl["raw"]["iid"]==42` (raw is provider-shaped and NOT normalized).
Note the deliberate traps: GitLab `changes_count` is the STRING `"4"` and must normalize to the number `4`; GitLab timestamps carry `.000Z` and must normalize to the same `+00:00` form as GitHub.

##### `git.pr_create`

**E2E-NEW-710 — Happy — P0.** Routes: `GET /repos/acme/api/branches/feature/pr-tools` -> 200 `{"name":"feature/pr-tools","commit":{"sha":"9f1c..."}}`; `POST /repos/acme/api/pulls` -> 201 `GH_PR_42_OPEN`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"feature/pr-tools", title:"Add PR tools", body:"Adds git.pr_* over the provider REST API."}`.
Then calls in order: `("GET",".../branches/feature/pr-tools")` then `("POST","https://api.github.com","/repos/acme/api/pulls")` with body exactly `{"base":"main","head":"feature/pr-tools","title":"Add PR tools","body":"Adds git.pr_* over the provider REST API.","draft":false}`; And result `number==42`, `state=="open"`, `draft==false`.

**E2E-NEW-711 — Happy — P0.** Routes: `GET /projects/acme%2Fapi/repository/branches/feature/pr-tools` -> 200; `POST /projects/acme%2Fapi/merge_requests` -> 201 `GL_MR_42_OPEN`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When same args on `acme-lab`.
Then POST body exactly `{"source_branch":"feature/pr-tools","target_branch":"main","title":"Add PR tools","description":"Adds git.pr_* over the provider REST API."}`; And result `number==42`, `provider=="gitlab"`.

**E2E-NEW-712 — Happy — P0.** Same as E2E-NEW-710 on `acme-ent`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304, FR-NEW-300.
Then both calls have `base_url == "https://github.ibm.com/api/v3"`; And result `host=="github.ibm.com"`.

**E2E-NEW-713 — Happy — P1.** Title `"Ajout des outils PR — été 2026 ✅"`, body `"Résumé:\n- prise en charge des MR\n- 日本語テスト"`. Mount `acme-self`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
Then the POST body `title` and `description` are byte-identical to the inputs (no escaping, no NFC/NFD rewriting); And response echo normalizes back to the same strings.

**E2E-NEW-714 — Failure — P0.** Route `GET /repos/acme/api/branches/feature/ghost` -> 404 `{"message":"Branch not found"}`. No POST route registered.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-305.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"feature/ghost", title:"x"}`.
Then `ERR_NOT_FOUND`; message contains `head branch 'feature/ghost' does not exist on host 'github.com'`; And `mock.calls().len() == 1` and its method/path is the GET branch pre-flight (proving no POST).

**E2E-NEW-715 — Failure — P0.** Same with `acme-lab`, route `GET /projects/acme%2Fapi/repository/branches/feature/ghost` -> 404 `{"message":"404 Branch Not Found"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-305.
Then `ERR_NOT_FOUND`; message contains `head branch 'feature/ghost' does not exist on host 'gitlab.com'`; And exactly 1 call, the GET.

**E2E-NEW-716 — Failure — P1.**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
When `git.pr_create{mount_id:"acme-api", base:"main", head:"main", title:"x"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `base and head must differ, both are 'main'`; And no calls.

**E2E-NEW-717 — Failure — P1.** `title: "   "`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-304.
Then `ERR_INVALID_ARGUMENT`; message contains `title must not be empty or whitespace-only`; And no calls.

**E2E-NEW-718 — Failure — P0.** Branch pre-flight 200; `POST /repos/acme/api/pulls` -> 422 `{"message":"Validation Failed","errors":[{"message":"A pull request already exists for acme:feature/pr-tools."}]}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `A pull request already exists for acme:feature/pr-tools.` and contains `github.com`; And 2 calls.

**E2E-NEW-719 — Failure — P0.** `POST .../merge_requests` -> 409 `{"message":["Another open merge request already exists for this source branch: !42"]}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Another open merge request already exists for this source branch: !42`.

**E2E-NEW-720 — Failure — P0.** POST -> 403 `{"message":"Resource not accessible by personal access token"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-311.
Then `ERR_FORBIDDEN`; message contains `Resource not accessible by personal access token`; And message contains `github.com`; And message does NOT contain `ghp_`.

**E2E-NEW-721 — EdgeCase — P1.** Two sub-cases in one test.

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-304, FR-NEW-303.
(a) GitHub with `draft:true`: POST body includes `"draft":true`; canned 201 payload is `GH_PR_42_OPEN` with `"draft":true`; result `draft==true`, `state=="open"`.
(b) GitLab with `draft:true`: POST body `title == "Draft: Add PR tools"` (GitLab has no draft flag) and canned 201 payload has `"title":"Draft: Add PR tools","draft":true`; result `draft==true` and `title=="Add PR tools"` (the `Draft: ` prefix is stripped in normalization, so the two providers agree).

**E2E-NEW-722 — SideEffect — P0.** After the successful E2E-NEW-710 create.

**Category:** SideEffect. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then the audit sink (`state.safety` capped audit) holds exactly one entry for this call whose fields include `op=="git.pr_create"`, `mount=="acme-api"`, `person=="dev@test.com"`, `host=="github.com"`, `provider=="github"`, `pr_number==42`; And the serialized entry does NOT contain `ghp_TESTTOKEN_github_001` nor the substring `ghp_`.

##### `git.pr_list`

**E2E-NEW-723 — Happy — P0.** Route `GET /repos/acme/api/pulls?state=open` -> 200 `[GH_PR_42_OPEN, GH_PR_43_OPEN]` where 43 is 42 with `"number":43,"title":"Bump deps","head":{"ref":"chore/bump","sha":"aa11..."}`.

**Category:** Happy. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `count==2`; And `pull_requests[0]` has EXACTLY the 16 list keys of 1.5 in order (the FR-NEW-317 set minus `commits`, `changed_files`, `additions`, `deletions`, `mergeable`; `raw` IS present); And `pull_requests[0]["number"]==42`, `[1]["number"]==43`; And no `commits`, `changed_files`, `additions`, `deletions` or `mergeable` key present in a list item.

**E2E-NEW-724 — Happy — P0.** Four calls on `acme-lab` with `state` = open, closed, merged, all; each routed to 200 `[]`.

**Category:** Happy. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then the four recorded query `state` values are exactly `"opened"`, `"closed"`, `"merged"`, `"all"`; And the equivalent GitHub run on `acme-api` records `"open"`, `"closed"`, `"closed"`, `"all"`, and for `merged` the client filters the response to items with `merged_at != null` (feed `[GH_PR_42_OPEN(merged_at:null), GH_PR_40_MERGED(merged_at:"2026-08-30T08:00:00Z")]` and assert `count==1`, `pull_requests[0]["number"]==40`, `state=="merged"`).

**E2E-NEW-725 — EdgeCase — P1.** Both providers routed to 200 `[]`.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then both return exactly `{"pull_requests":[],"count":0}`; And `count` is the number `0`, not `null`.

**E2E-NEW-726 — Failure — P1.** `state:"draft"`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `ERR_INVALID_ARGUMENT`; message contains `state must be one of open, closed, merged, all`; And no calls.

**E2E-NEW-727 — EdgeCase — P1.** Page 1: 100 items, header `Link: <https://api.github.com/repos/acme/api/pulls?state=open&per_page=100&page=2>; rel="next"`. Page 2: 50 items, no Link header.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-306.
Then `count==50+100==150`; And exactly 2 calls, the second with query containing `("page","2")`; And numbers are contiguous 1..=150 in response order.

**E2E-NEW-728 — Failure — P0.** 404 `{"message":"Not Found","documentation_url":"https://docs.github.com/rest"}`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `acme/api` and `github.com` and `Not Found`.

**E2E-NEW-729 — Failure — P1.** GitLab 500 `{"message":"500 Internal Server Error"}`.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-311.
Then `ERR_INTERNAL_ERROR`; message contains `gitlab.com` and `500`; And message does NOT contain `glpat_`.

**E2E-NEW-730 — EdgeCase — P1.** List on `acme-ent` with 1 item.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-300, FR-NEW-306.
Then `base_url == "https://github.ibm.com/api/v3"`; And `pull_requests[0]["host"] == "github.ibm.com"`.

**E2E-NEW-731 — EdgeCase — P1.** List on `acme-self` with 1 item.

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-300, FR-NEW-306.
Then `base_url == "https://gitlab.example.test/api/v4"`; And `pull_requests[0]["host"] == "gitlab.example.test"`.

##### `git.pr_get`

**E2E-NEW-732 — Happy — P0.** Routes of E2E-NEW-709 (GitHub side).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-307.
Then exactly 3 calls in order: `/repos/acme/api/pulls/42`, `/repos/acme/api/pulls/42/reviews`, `/repos/acme/api/commits/9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3/check-runs` (proving the head sha is taken from the first response); And `review_state=="approved"`, `checks_state=="success"`.

**E2E-NEW-733 — Happy — P0.** Routes of E2E-NEW-709 (GitLab side).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-307.
Then exactly 2 calls: `/projects/acme%2Fapi/merge_requests/42` then `/projects/acme%2Fapi/merge_requests/42/approvals`; And `checks_state=="success"` derived from `head_pipeline.status`; And `review_state=="approved"` derived from `approved:true`.

**E2E-NEW-734 — EdgeCase — P1.** GitHub payload with `"changed_files":0,"additions":0,"deletions":0,"commits":0`; GitLab payload with `"changes_count":"0"` and `diff_stats_summary` absent entirely.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303, FR-NEW-307.
Then both return `changed_files==0`, `additions==0`, `deletions==0`, `commits==0` (absent GitLab stats normalize to `0`, not `null`).

**E2E-NEW-735 — EdgeCase — P0.** GitHub: `"state":"closed","merged":true,"merged_at":"2026-09-03T08:00:00Z","mergeable":null`. GitLab: `"state":"merged","merge_status":"cannot_be_merged"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="merged"`; And GitHub `mergeable==null` (JSON null, not `false`).

**E2E-NEW-736 — EdgeCase — P0.** GitHub `"state":"closed","merged":false,"merged_at":null`. GitLab `"state":"closed"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="closed"`; And neither is `"merged"` (the regression this guards: GitHub closed+merged look alike without `merged_at`).

**E2E-NEW-737 — EdgeCase — P1.** GitHub `"draft":true,"state":"open"`. GitLab `"draft":true,"state":"opened","title":"Draft: Add PR tools"`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then both `state=="open"` and `draft==true`; And both `title=="Add PR tools"`.

**E2E-NEW-738 — Failure — P0.** `/repos/acme/api/pulls/9999` -> 404 `{"message":"Not Found"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `pull request 9999` and `acme/api`; And exactly 1 call (no reviews/checks fan-out after a 404).

**E2E-NEW-739 — Failure — P0.** `/projects/acme%2Fapi/merge_requests/9999` -> 404 `{"message":"404 Not found"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then `ERR_NOT_FOUND`; message contains `merge request 9999`; And exactly 1 call.

**E2E-NEW-740 — Failure — P1.** `pr_number:0`, then `pr_number:-1`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313.
Then both `ERR_INVALID_ARGUMENT` with message containing `pr_number must be a positive integer`; And no calls in either.

**E2E-NEW-741 — Failure — P0.** get 200, reviews 200, check-runs -> 403 `{"message":"Resource not accessible by integration"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-308.
Then `ERR_FORBIDDEN`; message contains `check runs` and `Resource not accessible by integration`; And 3 calls recorded. (Explicit design decision under test: a failed sub-call is NOT degraded to `checks_state:"none"`, because silently reporting "no checks" on a merge decision is worse than failing.)

##### `git.pr_diff`

**E2E-NEW-742 — Happy — P0.** Route `GET /repos/acme/api/pulls/42` with `accept == "application/vnd.github.v3.diff"` -> 200 body `UNIFIED_DIFF_SMALL` (content-type `text/plain`).

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then exactly 1 call, and its `accept` field equals `"application/vnd.github.v3.diff"` exactly; And result `diff == UNIFIED_DIFF_SMALL` byte-for-byte; And `bytes == UNIFIED_DIFF_SMALL.len()`; And `truncated == false`.

**E2E-NEW-743 — Happy — P0.** Route `GET /projects/acme%2Fapi/merge_requests/42/changes` -> 200

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

**E2E-NEW-744 — EdgeCase — P1.** GitHub 200 with empty body; GitLab 200 `{"changes":[]}`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then both return `diff==""`, `bytes==0`, `truncated==false`; And no error.

**E2E-NEW-745 — EdgeCase — P0.** GitHub 200 body of 12 MiB (`"+x".repeat()` padded to exactly 12 * 1024 * 1024 bytes).

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `truncated == true`; And `bytes == 5 * 1024 * 1024` (the declared cap); And `diff.len() == 5 * 1024 * 1024`; And `diff` is a prefix of the source body; And the result carries `"note"` containing `diff truncated at 5 MiB; fetch the branch with git.remote_fetch for the full change`.

**E2E-NEW-746 — EdgeCase — P1.** Body containing `+// été ✅ 日本語\r\n+second\r\n`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `diff` is byte-identical including the `\r\n` pairs; And `bytes` counts BYTES not chars (assert `bytes == body.as_bytes().len()`).

**E2E-NEW-747 — Failure — P1.** 404 on both providers.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-313, FR-NEW-309.
Then `ERR_NOT_FOUND`; message contains `42`.

**E2E-NEW-748 — Failure — P1.** GitHub 200 with `content-type: text/html` and body `<!DOCTYPE html><html>...`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `ERR_INTERNAL_ERROR`; message contains `expected a unified diff from github.com, got content-type 'text/html'`; And the message does NOT contain `<!DOCTYPE` (no body dumping).

**E2E-NEW-749 — EdgeCase — P2.** GitLab change entry with `"diff":"Binary files a/logo.png and b/logo.png differ\n"` and `"new_file":false`.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-309.
Then `diff` contains the line `Binary files a/logo.png and b/logo.png differ` verbatim, preceded by `diff --git a/logo.png b/logo.png`.

##### `git.pr_merge`

**E2E-NEW-750 — Happy — P0.** `PUT /repos/acme/api/pulls/42/merge` -> 200 `{"sha":"3c0ffee1234567890abcdef1234567890abcdef1","merged":true,"message":"Pull Request successfully merged"}`; then get routes return the merged payload of E2E-NEW-735.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then the PUT body equals exactly `{"merge_method":"merge"}`; And the result normalized object has `state=="merged"` and `raw["sha"]=="3c0ffee1234567890abcdef1234567890abcdef1"`.

**E2E-NEW-751 — Happy — P0.** As E2E-NEW-750 with `strategy:"squash"`. Then PUT body `{"merge_method":"squash"}`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

**E2E-NEW-752 — Happy — P0.** As E2E-NEW-750 with `strategy:"rebase"`. Then PUT body `{"merge_method":"rebase"}`; And exactly one PUT (GitHub needs no pre-rebase call).

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

**E2E-NEW-753 — Happy — P0.** `PUT /projects/acme%2Fapi/merge_requests/42/merge` -> 200 `GL_MR_42_OPEN` with `"state":"merged"`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When strategy `merge` on `acme-lab`. Then PUT body exactly `{"squash":false}`; And result `state=="merged"`.

**E2E-NEW-754 — Happy — P0.** Same with `squash`. Then PUT body exactly `{"squash":true}`.

**Category:** Happy. **Scenario:** SC-928. **Requirements:** FR-NEW-310.

**E2E-NEW-755 — SideEffect — P0.** Routes: `POST /projects/acme%2Fapi/merge_requests/42/rebase` -> 202 `{"rebase_in_progress":true}`; `GET /projects/acme%2Fapi/merge_requests/42?include_rebase_in_progress=true` -> 200 `{"rebase_in_progress":false,"merge_error":null}`; `PUT .../merge` -> 200 merged payload.

**Category:** SideEffect. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
When strategy `rebase` on `acme-lab`.
Then calls in exactly this order: `POST .../rebase`, `GET ...?include_rebase_in_progress=true`, `PUT .../merge`; And PUT body `{"squash":false}`; And result `state=="merged"`.

**E2E-NEW-756 — Failure — P1.** `strategy:"fast-forward"`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-310.
Then `ERR_INVALID_ARGUMENT`; message contains `strategy must be one of merge, squash, rebase`; And no calls.

**E2E-NEW-757 — Failure — P0.** PUT -> 405 `{"message":"Merge commits are not allowed on this repository.","documentation_url":"..."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Merge commits are not allowed on this repository.` and contains `strategy 'merge'`; And the tool did NOT silently retry with another strategy (exactly 1 PUT recorded).

**E2E-NEW-758 — Failure — P0.** PUT -> 405 `{"message":"Required status check \"ci/build\" is expected."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Required status check "ci/build" is expected.`

**E2E-NEW-759 — Failure — P0.** GitLab PUT -> 405 `{"message":"405 Method Not Allowed"}` and the MR get shows `"merge_status":"cannot_be_merged","head_pipeline":{"status":"failed"}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `405 Method Not Allowed` and contains `gitlab.com`.

**E2E-NEW-760 — Failure — P1.** GitHub PUT -> 409 `{"message":"Head branch was modified. Review and try the merge again.","sha":"..."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `Head branch was modified. Review and try the merge again.`

**E2E-NEW-761 — Failure — P0.** GitHub PUT -> 403 `{"message":"4 of 4 required status checks are expected. Protected branch rules not met."}`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_FORBIDDEN`; message contains `Protected branch rules not met.`; And no `ghp_` substring in the message.

**E2E-NEW-762 — EdgeCase — P0.** GitHub PUT -> 405 `{"message":"Pull Request is not mergeable"}` and the pre-merge `GET /repos/acme/api/pulls/42` returns the merged payload (`merged:true`).

**Category:** EdgeCase. **Scenario:** SC-928. **Requirements:** FR-NEW-311.
Then `ERR_INVALID_ARGUMENT`; message contains `pull request 42 is already merged`; And the PUT was NEVER sent (the pre-read short-circuits): recorded calls contain exactly the GET.

##### `git.pr_review`

**E2E-NEW-763 — Happy — P0.** `POST /repos/acme/api/pulls/42/reviews` -> 200 `{"id":7001,"state":"APPROVED","user":{"login":"dev"},"body":""}`; then get routes for the normalized return.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
When `git.pr_review{mount_id:"acme-api", pr_number:42, verdict:"approve"}`.
Then POST body exactly `{"event":"APPROVE"}` (no `body` key when none supplied); And result `review_state=="approved"`.

**E2E-NEW-764 — Happy — P0.** verdict `request_changes`, body `"Please split the merge logic out of the tool layer."`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then POST body exactly `{"event":"REQUEST_CHANGES","body":"Please split the merge logic out of the tool layer."}`; And canned response `{"id":7002,"state":"CHANGES_REQUESTED"}` normalizes to `review_state=="changes_requested"`.

**E2E-NEW-765 — Happy — P0.** verdict `comment`, body `"Nit: typo in the doc comment."`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then POST body exactly `{"event":"COMMENT","body":"Nit: typo in the doc comment."}`; And `review_state` from the subsequent get is `"review_required"` (a comment is not an approval).

**E2E-NEW-766 — Happy — P0.** `POST /projects/acme%2Fapi/merge_requests/42/approve` -> 201 `{"id":42,"iid":42,"state":"opened"}`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then exactly one POST to `/projects/acme%2Fapi/merge_requests/42/approve` with body `null` or `{}` (assert the recorded body is `{}`); And `review_state=="approved"` from the follow-up approvals read.

**E2E-NEW-767 — SideEffect — P0.** Routes `POST .../42/unapprove` -> 201 `{}`, `POST .../42/notes` -> 201 `{"id":9001,"body":"Please split the merge logic out of the tool layer."}`.

**Category:** SideEffect. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
When verdict `request_changes` with that body on `acme-lab`.
Then calls in exactly this order: `POST /projects/acme%2Fapi/merge_requests/42/unapprove`, then `POST /projects/acme%2Fapi/merge_requests/42/notes` with body exactly `{"body":"Please split the merge logic out of the tool layer."}`; And result `review_state=="changes_requested"` (GitLab has no such state natively; this is the normalization under test).

**E2E-NEW-768 — Happy — P0.** verdict `comment` on `acme-lab`.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then exactly ONE call, `POST .../42/notes` (no `unapprove`, no `approve`).

**E2E-NEW-769 — Failure — P1.** verdict `"lgtm"`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then `ERR_INVALID_ARGUMENT`; message contains `verdict must be one of approve, request_changes, comment`; And no calls.

**E2E-NEW-770 — Failure — P1.** verdict `request_changes` with `body` omitted, then with `body:"   "`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-312.
Then both `ERR_INVALID_ARGUMENT` with message containing `verdict 'request_changes' requires a non-empty body`; And no calls in either.

**E2E-NEW-771 — Failure — P1.** POST reviews -> 422 `{"message":"Unprocessable Entity","errors":["Can not approve your own pull request"]}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-311, FR-NEW-312.
Then `ERR_INVALID_ARGUMENT`; message contains `Can not approve your own pull request`.

**E2E-NEW-772 — Failure — P0.** `POST .../42/approve` -> 401 `{"message":"401 Unauthorized"}` in the GitLab self-approval case, plus a second sub-case 403 `{"message":"Members can not approve their own merge request"}`.

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-311, FR-NEW-312.
Then the 403 case is `ERR_FORBIDDEN` with message containing `Members can not approve their own merge request`; And the 401 case is `ERR_UNAUTHENTICATED` with a message containing `gitlab.com` and containing `re-authenticate with git.auth` (a provider 401 means the credential is no longer accepted, and the caller must be told the remedy); And neither message contains `glpat_`.

##### OAuth scope extension and validation

**E2E-NEW-773 — Happy — P0.** Unit-level on `device_flow.rs`: run `HttpDeviceFlowClient::with_github_urls` against a local one-shot HTTP server (the pattern already supported at `device_flow.rs:162-171`) extended for GitLab, or assert the constant directly.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-330, FR-MOD-109.
Then the GitLab scope constant equals exactly `"api read_repository write_repository"` (was `"read_repository write_repository"` at `device_flow.rs:39`); And the form field `scope` sent to `{instance}/oauth/authorize_device` equals that same string.

**E2E-NEW-774 — Happy — P0.** Same for GitHub.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-330, FR-NEW-331.
Then the GitHub scope constant is still exactly `"repo"` (`device_flow.rs:38`) and the posted `scope` form field equals `"repo"`. (Guards against a copy-paste widening of the GitHub scope while fixing GitLab.)

**E2E-NEW-775 — Happy — P0.** `FakeFlow` scripted with `TokenPoll::granted("glpat_FAKE_999", vec!["api".into(),"read_repository".into(),"write_repository".into()], now+1h)`, host `gitlab.com`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-335, FR-NEW-330.
When `git.auth{provider:"gitlab", host:"gitlab.com"}` then wait via `eventually(...)` for the poller, then `git.auth_status{host:"gitlab.com"}`.
Then `statuses[0]["scopes"] == ["api","read_repository","write_repository"]`; And `statuses[0]["validity"] == "valid"`; And the persisted `oauth_tokens.scopes` row round-trips the same three values after reopening the store with `with_persistence`.

**E2E-NEW-776 — Failure — P0.** Seed `dev@test.com`/`gitlab.com` with scopes `["read_repository","write_repository"]` (today's device-flow grant). No mock routes registered.

**Category:** Failure. **Scenario:** SC-926. **Requirements:** FR-NEW-332, FR-NEW-333.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then `ERR_FORBIDDEN`; message equals-contains `token for host gitlab.com is missing scope 'api' (or 'read_api') required by git.pr_list; re-authenticate with git.auth or seed a sufficient token with git.token_set`; And `mock.calls().is_empty()` (checked BEFORE the network, mirroring `store.rs:233-249`).
Design note under test: the code is `ERR_FORBIDDEN`, not `ERR_UNAUTHENTICATED`, because the credential is valid and merely too narrow; `ERR_UNAUTHENTICATED` stays reserved for absent/expired/rejected tokens.

**E2E-NEW-777 — Failure — P0.** Same seeding, `git.pr_create{mount_id:"acme-lab", base:"main", head:"feature/pr-tools", title:"x"}`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-332, FR-NEW-333.
Then `ERR_FORBIDDEN`; message contains `missing scope 'api'` and `git.pr_create`; And no calls — in particular the head-branch pre-flight of E2E-NEW-714 did NOT run, proving the scope gate precedes every network call including pre-flights.

**E2E-NEW-778 — Happy — P0.** Seed `gitlab.com` with scopes `["read_api","read_repository"]`; routes of E2E-NEW-733.

**Category:** Happy. **Scenario:** SC-927. **Requirements:** FR-NEW-332.
When `git.pr_get{mount_id:"acme-lab", pr_number:42}`.
Then it succeeds with `number==42`; And 2 calls recorded.

**E2E-NEW-779 — Failure — P0.** Same `["read_api","read_repository"]` token.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-lab", pr_number:42, strategy:"squash"}`.
Then `ERR_FORBIDDEN`; message contains `missing scope 'api' required by git.pr_merge`; And message does NOT offer `read_api` as a remedy; And no calls.

**E2E-NEW-780 — Failure — P0.** Seed `github.com` with scopes `["public_repo"]`.

**Category:** Failure. **Scenario:** SC-928. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then `ERR_FORBIDDEN`; message contains `token for host github.com is missing scope 'repo' required by git.pr_merge`; And no calls.

**E2E-NEW-781 — Failure — P0.** Seed via the real tool: `git.token_set{host:"gitlab.com", token:"glpat_SEEDED_005"}` (which stores `Vec::new()` scopes, `git_auth.rs:505`).

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
When `git.pr_list{mount_id:"acme-lab", state:"open"}`.
Then `ERR_FORBIDDEN`; message contains `token for host gitlab.com has no recorded scopes` and contains `re-authenticate with git.auth`, and contains `git.token_set`; And no calls. (A seeded token is NOT assumed sufficient.)
And: the same seeding against `github.com` then `git.pr_list{mount_id:"acme-api"}` fails identically, so the rule is not GitLab-specific.

**E2E-NEW-782 — EdgeCase — P1.** Seed `gitlab.com` with scopes `["everything","super_admin","*"]`.

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
When `git.pr_get{mount_id:"acme-lab", pr_number:42}`.
Then `ERR_FORBIDDEN` with message containing `missing scope 'api' (or 'read_api')`; And no calls. (No wildcard interpretation, no "looks powerful" heuristic.)

**E2E-NEW-783 — Happy — P0.** Seeds: `github.com` -> `["repo"]`; `gitlab.com` -> `["read_repository","write_repository"]`; `gitlab.example.test` -> `["api"]`.

**Category:** Happy. **Scenario:** SC-925. **Requirements:** FR-NEW-335.
When `git.auth_status{}` (no filters).
Then `statuses` has 3 entries sorted by host ascending: `github.com`, `gitlab.com`, `gitlab.example.test`; And each entry keeps the existing 5 keys (`host, provider, validity, expires_at, scopes`, `git_auth.rs:383-390`) and gains exactly two: `"pr_read"` and `"pr_write"` booleans; And `github.com` -> `pr_read==true, pr_write==true`; `gitlab.com` -> `pr_read==false, pr_write==false`; `gitlab.example.test` -> `pr_read==true, pr_write==true`.

**E2E-NEW-784 — EdgeCase — P1.** Seed `gitlab.com` with scopes `["api"]` only (no `read_repository`).

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-332.
When `git.pr_get`, then `git.pr_diff`, then `git.pr_review{verdict:"comment", body:"ok"}` with all routes canned 200.
Then all three succeed; And `git.auth_status` reports `pr_read==true, pr_write==true`. (`api` subsumes `read_api`: no test may require both.)

**E2E-NEW-785 — Failure — P0.** Seed `github.com` with scopes `["repo"]` and `expires_at = Utc::now() - 1h`.

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-NEW-332.
When `git.pr_merge{mount_id:"acme-api", pr_number:42, strategy:"merge"}`.
Then `ERR_UNAUTHENTICATED`; message starts with `token expired for host github.com` and contains `authenticate with git.auth or git.token_set for host github.com` (byte-compatible with `store.rs:245-248` and its test at `store.rs:817-840`); And no calls. (Expiry wins over the scope check; a token both expired and narrow reports expiry.)

##### Security

**E2E-NEW-786 — Security — P0.** Every one of the 6 tools, GitHub and GitLab (12 invocations), each routed to 401 whose body deliberately echoes the credential: `{"message":"Bad credentials: ghp_TESTTOKEN_github_001"}` / `{"message":"401 Unauthorized for glpat_TESTTOKEN_gl_003"}`.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then each call returns `ERR_UNAUTHENTICATED`; And for each, `err.message` does NOT contain `ghp_TESTTOKEN_github_001`, `glpat_TESTTOKEN_gl_003`, nor the prefixes `ghp_` / `glpat_` (redaction applied to the provider body, same duty as `remote.rs:292-303`); And each message still contains the host and a remedy.

**E2E-NEW-787 — Security — P0.** Install a `tracing_subscriber` test layer capturing every span name and every field value (string-rendered) for the duration.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
When the 12 invocations of E2E-NEW-786 run, plus 12 successful invocations.
Then no captured field value and no span name contains `ghp_` or `glpat_`; And the spans that exist are named `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` and carry fields `mount`, `person`, `host`, `provider` with the expected values.

**E2E-NEW-788 — Security — P0.** After the 24 invocations of E2E-NEW-787.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
Then every audit entry serialized to JSON is scanned: none contains `ghp_`, `glpat_`, or the literal seeded token values; And an entry exists per invocation (no silent audit gap on the failure path).

**E2E-NEW-789 — Security — P0.** Caller `outsider@test.com`, who is not a member of any mount. All routes canned 200 so a leak would succeed loudly.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-100, FR-NEW-347.
When each of `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` is called on `acme-api` and on `acme-lab` (12 calls).
Then all 12 return `ERR_FORBIDDEN` with a message containing `'outsider@test.com' is not a member of 'acme-api'` / `'acme-lab'` (the shape `state.rs:61` produces, cf. `errors.rs` display test); And `mock.calls().is_empty()` after all 12 (authorization precedes host resolution, credential lookup and network).

**E2E-NEW-790 — Security — P0.** Request with no identity (the `ToolCtx.person` empty, as `require_identity` expects, `git_auth.rs`).

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-100, FR-NEW-347.
When the same 12 calls.
Then all return `ERR_UNAUTHENTICATED`; And `mock.calls().is_empty()`.

**E2E-NEW-791 — Security — P0.** Seed two tokens on the SAME host: `dev@test.com`/`github.com` -> `ghp_TESTTOKEN_github_001`, and `alice@test.com`/`github.com` -> `ghp_ALICE_777`. Both are members of `acme-api`.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-314.
When `dev@test.com` calls `git.pr_list{mount_id:"acme-api"}`, then `alice@test.com` calls the same.
Then the first recorded request's `authorization_seen == "token ghp_TESTTOKEN_github_001"`; And the second's `== "token ghp_ALICE_777"`; And neither request ever carried the other value (assert across all recorded calls).

**E2E-NEW-792 — Security — P0.**

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-315.
When one successful call per provider.
Then the GitHub request's `authorization_seen == "token ghp_TESTTOKEN_github_001"` and the GitLab request's `== "Bearer glpat_TESTTOKEN_gl_003"`; And the header is present on EVERY recorded request including the branch pre-flight and the sub-calls of `pr_get` (assert on all of them, so no sub-call is issued unauthenticated).

**E2E-NEW-793 — Security — P0.** `pr_get` on both providers with the routes of E2E-NEW-709.

**Category:** Security. **Scenario:** SC-927. **Requirements:** FR-NEW-303, FR-NEW-314.
Then `result["raw"]` deep-equals the exact canned get payload (`GH_PR_42_OPEN` / `GL_MR_42_OPEN`) and nothing more; And the serialized `result` contains no key named `authorization`, `token`, `headers`, or `request`; And the full serialized result contains no `ghp_`/`glpat_` substring.

**E2E-NEW-794 — Security — P0.** `GET /repos/acme/api/pulls/42` -> 302 with header `Location: https://evil.test/repos/acme/api/pulls/42`. No route registered for the evil host.

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-316.
When `git.pr_get{mount_id:"acme-api", pr_number:42}`.
Then `ERR_INTERNAL_ERROR`; message contains `refused redirect from host 'api.github.com' to 'evil.test'`; And exactly 1 recorded call, whose `base_url` is `https://api.github.com` (the credential was never re-sent to the redirect target); And no recorded call has `base_url` containing `evil.test`.

**E2E-NEW-795 — Security — P1.** `GET /repos/acme/api/pulls` (list) -> 200 with a 64 MiB JSON body.

**Category:** Security. **Scenario:** SC-926. **Requirements:** FR-NEW-315, FR-NEW-316.
When `git.pr_list{mount_id:"acme-api", state:"open"}`.
Then `ERR_INVALID_ARGUMENT`; message contains `response from host 'github.com' exceeds the 16 MiB limit`; And the tool returns within 10 seconds (assert with `tokio::time::timeout`), proving the body was capped rather than fully buffered and parsed.
(Note: `pr_diff` has its own 5 MiB truncation contract, E2E-NEW-745; the JSON cap is a hard refusal because a half-read JSON document cannot be parsed.)

##### Lifecycle integration and contract

**E2E-NEW-796 — Integration — P0.** GitHub route queues: branch pre-flight 200; `POST /pulls` 201 `GH_PR_42_OPEN`; `GET /pulls/42` queued twice (first the open payload, then the merged payload of E2E-NEW-735); `/pulls/42/reviews` -> `[]` then `GH_REVIEWS_APPROVED`; `/commits/9f1c.../check-runs` -> `GH_CHECKS_SUCCESS`; `POST /pulls/42/reviews` -> 200 `{"id":7001,"state":"APPROVED"}`; `PUT /pulls/42/merge` -> 200 merged.

**Category:** Integration. **Scenario:** SC-928. **Requirements:** FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312.
When, in order: `pr_create` -> `pr_get` -> `pr_review{approve}` -> `pr_merge{squash}`.
Then create returns `state=="open"`, `review_state=="review_required"`, `checks_state=="success"`; And the first `pr_get` returns `state=="open"`, `review_state=="review_required"`; And after the review, `review_state=="approved"`; And merge returns `state=="merged"`; And the recorded call sequence method+path matches, in order: `GET /repos/acme/api/branches/feature/pr-tools`, `POST /repos/acme/api/pulls`, `GET /repos/acme/api/pulls/42`, `GET /repos/acme/api/pulls/42/reviews`, `GET /repos/acme/api/commits/9f1c.../check-runs`, `POST /repos/acme/api/pulls/42/reviews`, `GET /repos/acme/api/pulls/42`, `PUT /repos/acme/api/pulls/42/merge`.

**E2E-NEW-797 — Integration — P0.** The GitLab mirror of E2E-NEW-796 on `acme-lab` (`POST /merge_requests`, `GET /merge_requests/42`, `GET .../approvals`, `POST .../approve`, `PUT .../merge` with `{"squash":true}`).

**Category:** Integration. **Scenario:** SC-928. **Requirements:** FR-NEW-304, FR-NEW-307, FR-NEW-310, FR-NEW-312.
Then each of the four tool results has EXACTLY the same key set and key order as its GitHub counterpart from E2E-NEW-796 (assert pairwise on the four result objects, as in E2E-NEW-709); And the state transitions match: `open -> open -> approved -> merged`.

**E2E-NEW-798 — EdgeCase — P1.** Get and list payloads whose title is `"Ajout des outils PR — été 2026 ✅"` and body/description is `"Résumé:\n- prise en charge des MR\n- 日本語テスト"` on both providers.

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-303.
Then `pr_get` returns those exact strings for `title` and `body` on both; And the corresponding `pr_list` item `title` matches byte-for-byte; And the string length in chars is asserted (`title.chars().count() == 32`) so a mojibake round trip fails loudly.

**E2E-NEW-799 — EdgeCase — P0.** The frozen contract tests (`tools/contract_golden.rs`, regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1`).

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-345, FR-NEW-346.
Then the frozen-contract registry asserted at `crates/mcp-fs/src/tools/contract_golden.rs:131` has `len() == 94` (63 today, plus the 31 tools of FR-NEW-349, of which these 6 are the PR family). Note the enabled full registry is a different number, asserted separately at `crates/mcp-fs/src/tools/all.rs:122`; And each of `git.pr_create`, `git.pr_list`, `git.pr_get`, `git.pr_diff`, `git.pr_merge`, `git.pr_review` appears in `tool-contract-golden.json` with a `mount_id` required string parameter (the family convention); And `git.pr_list.inputSchema.properties.state.description` names exactly `open, closed, merged, all`; And `git.pr_merge.strategy` names exactly `merge, squash, rebase`; And `git.pr_review.verdict` names exactly `approve, request_changes, comment`; And `TOOL_CONTRACT.txt` contains the same six entries.

---

##### D.z Two decisions the implementer must not re-open silently

1. **Missing scope is `ERR_FORBIDDEN`, absent/expired/provider-rejected credential is `ERR_UNAUTHENTICATED`** (E2E-NEW-776 vs E2E-NEW-785/E2E-NEW-772). Both are checked before the network; only the code differs, and clients branch on it.
2. **A failed sub-call in `pr_get` is an error, never a degraded field** (E2E-NEW-741). Reporting `checks_state:"none"` when the checks endpoint returned 403 would make a merge decision on invented data.

#### Z. Gap-closure tests

Tests `E2E-NEW-800` .. `E2E-NEW-950`. They close the coverage gaps left by Sections 12.1
and 12.2 and add nothing else: no test below duplicates an existing one, and each states
the angle the existing tests do not already take.

Harness, seeds and verification primitives are the ones already defined in Section 12.2
and are referenced by name, never redefined:

- **Band A** (`MOUNT = "gitproj"`, `OWNER = "owner@test.com"`): SEED-A, SEED-B, SEED-R, SEED-R2,
  `seed_bare_remote`, `advance_bare_remote`, `db.get_ref`, `db.list_refs`, `db.list_remotes`,
  `state.safety.audit`, `state.safety.bytes_written`, `Env::with_quota`.
- **Band B** (`MOUNT = "proj"`, `OWNER = "owner@acme.test"`): SEED-CONFLICT, SEED-AUTOMERGE,
  SEED-FF, SEED-MULTI(n), SEED-BINARY, SEED-DELDEL, SEED-DELMOD, SEED-TYPECHANGE, SEED-EMPTY.
- **Band C** (`MOUNT = "proj1"`, `OWNER = "owner@test.com"`): FX-LINE, FX-FORK, FX-FORK-CLEAN,
  FX-FORK4, FX-MERGE, FX-INIT.
- **Band D**: `MockProviderApi`, mounts `acme-api`, `acme-ent`, `acme-lab`, `acme-self`,
  `acme-generic`, `acme-pub`, `acme-nor`, people `dev@test.com` / `outsider@test.com` / `anon`.
- **`file://` caveat (Band A, established at 12.2 §0):** every test that exercises remote
  *mechanics* drives `push_branch_inner` / `fetch_branch_inner` / `pull_branch` directly;
  every test that exercises *validation or authorization* drives the registered tool. Each
  test below states which.
- "Assert error" means: the `Result` is `Err`, `err.code() == "ERR_..."`, and
  `err.to_string()` contains the named substring (`errors.rs:176` format `"{CODE}: {message}"`).
- "No operation row" means `SELECT COUNT(*) FROM git_operations WHERE volume_id = ?` returns `0`.

---

### Z.1 Gap 1 — requirements that had no test at all

#### FR-NEW-218 — rebase refuses a dirty volume

---

**E2E-NEW-800 — Happy — a rebase runs once the dirt is committed**

**Category:** Happy. **Scenario:** SC-914. **Requirements:** FR-NEW-218, FR-NEW-210.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature` = `<sha-of-F2>`, `main` = `<sha-of-C2>`.
- **Given** `fs.write_text {path:"/scratch.txt", content:"tmp\n"}` makes the volume dirty, then `git.commit {message:"F3 scratch"}` -> `<sha-of-F3>`, so `git.status` reports `"dirty": false` and zero entries in `changes`.
- **When** `git.rebase {mount_id:"proj1", onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"},{action:"pick",sha:"<sha-of-F3>"}]}`.
- **Then** `status == "completed"` and `replayed == 3`.
- **And** `git.log {ref_name:"feature"}` holds exactly 5 commits, messages in order `["F3 scratch","F2 feature add","F1 feature edit","C2 main edit","C1 base"]`.
- **And** `/scratch.txt` reads exactly `b"tmp\n"`.
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **Verification:** `git.log` message list; `Env::read` byte compare; direct relational query.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-801 — Failure — a dirty volume refuses the rebase before any replay**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-218.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature` = `<sha-of-F2>`.
- **Given** `fs.write_text {path:"/a.txt", content:"a1\nDIRTY\n"}` (tracked file modified, not committed).
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"`.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"` (unchanged) and `db.get_ref("refs/heads/main").target == "<sha-of-C2>"`.
- **And** `/a.txt` still reads exactly `b"a1\nDIRTY\n"` (the dirt is preserved, not discarded).
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no entry whose `op` is `"git.rebase"`.
- **Verification:** error code + substrings; `db.get_ref`; `Env::read`; relational count; audit scan.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-802 — EdgeCase — dirt that is only a deletion still refuses**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-218.
- **Preconditions:** FX-FORK-CLEAN, HEAD on `feature`.
- **Given** `fs.delete {path:"/g.txt"}` (tracked file removed, nothing modified, nothing added), so `git.status.changes == [{"path":"/g.txt","status":"deleted"}]`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"` — a deletion counts as dirty exactly as a modification does.
- **And** `/g.txt` is still absent: `VolumeClient::read_bytes("/g.txt")` errors `ERR_NOT_FOUND` (the refusal did not restore it either).
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"`.
- **And** a second run, after `git.commit {message:"drop g"}`, succeeds — proving the refusal was about dirt and not about the todo.
- **Verification:** error code + substring; volume read; ref read; second call `is_ok()`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-222 — `git.rebase_continue` with unresolved conflicts

---

**E2E-NEW-803 — Happy — a second resolution call covering the rest lets the continue through**

**Category:** Happy. **Scenario:** SC-916. **Requirements:** FR-NEW-222, FR-NEW-221, FR-NEW-178.
- **Preconditions:** Band C harness with SEED-MULTI(3) ported onto `proj1`: `C0` commits `/f/000.txt`, `/f/001.txt`, `/f/002.txt` each `"line-A\n"`; `feature` = `C0` + `FM1` rewriting all three to `"line-FEATURE\n"`; `main` = `C0` + `CM1` rewriting all three to `"line-MAIN\n"`. HEAD on `feature`.
- **Given** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-FM1>"}]}` returns `status == "conflict"` with `conflicts[*].path == ["/f/000.txt","/f/001.txt","/f/002.txt"]`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/000.txt",strategy:"theirs"},{path:"/f/001.txt",strategy:"theirs"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/f/002.txt"` and `"unresolved"` (partial resolutions are recorded, the continue itself is refused).
- **When** `git.rebase_continue {resolutions:[{path:"/f/002.txt",strategy:"ours"}]}` — only the remaining path.
- **Then** `status == "completed"`, `replayed == 1`.
- **And** `/f/000.txt` == `b"line-FEATURE\n"`, `/f/001.txt` == `b"line-FEATURE\n"`, `/f/002.txt` == `b"line-MAIN\n"` (the resolutions of both calls were combined).
- **And** no `git_operations` row for `volume_id = "proj1"`.
- **Verification:** error then success on the same tool; byte-exact `Env::read`; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-804 — Failure — continue with an empty resolutions array keeps the pause exactly**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-222.
- **Preconditions:** FX-FORK (F1 conflicts with C2 on `/a.txt` line 2). `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"},{action:"pick",sha:"<sha-of-F2>"}]}` -> `status == "conflict"`, paused at entry index `0`.
- **Given** the `git_operations` row for `volume_id = "proj1"` holds `op_type == "rebase"`, `current_step == 0` (F1 is todo entry index 0, zero-based per FR-NEW-187), `total_steps == 2`, and a conflict set whose single element has `path == "/a.txt"`.
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:[]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"/a.txt"` and `"unresolved"`.
- **And** the `git_operations` row is still present with `current_step == 0`, `total_steps == 2` and a conflict set whose single element has `path == "/a.txt"` — byte-identical to the pre-call row except `updated_at`.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F2>"` (the pre-rebase tip, unmoved).
- **And** `/a.txt` reads exactly `b"a1\nMAIN\n"` (the onto side; nothing was written).
- **And** a following `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}` succeeds, proving the pause was resumable and not corrupted.
- **Verification:** error code + substrings; two direct `git_operations` row reads compared field by field; ref read; byte compare; recovery call.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-805 — EdgeCase — repeated partial continues never advance the step index**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-222, FR-NEW-178, FR-NEW-177.
- **Preconditions:** as E2E-NEW-803 (three conflicting paths, one todo entry).
- **When** `git.rebase_continue {resolutions:[{path:"/f/000.txt",strategy:"ours"}]}` — rejected, `current_step` still reads `0`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/001.txt",strategy:"ours"}]}` — rejected, `current_step` still reads `0`.
- **When** `git.rebase_continue {resolutions:[{path:"/f/999.txt",strategy:"ours"}]}` (a path not in the conflict set).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/f/999.txt"` and `"not in conflict"`; the call is all-or-nothing, so the previously recorded resolutions for `/f/000.txt` and `/f/001.txt` are retained and `/f/999.txt` is recorded nowhere.
- **And** after all three calls `current_step == 0` and `total_steps == 1` (a rebase always reports integers, never null; only single-step operations report null per FR-NEW-186), and `conflicts` still lists all three original paths.
- **And** `db.get_ref("refs/heads/feature").target` equals `<sha-of-FM1>` throughout (read before the first continue and after the third, compared for equality).
- **And** a final `git.rebase_continue {resolutions:[{path:"/f/002.txt",strategy:"ours"}]}` completes the rebase, confirming the two earlier partial resolutions survived three rejections.
- **Verification:** three error assertions; `git_operations` row read after each call; ref equality; terminal success.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-276 — `git_operations` is registered in `TABLES`

Grounding: `pub const TABLES: [&str; 3] = ["git_objects", "git_refs", "git_remotes"]`
(`crates/mcp-fs/src/git/db.rs:42`) is the only list driving the purge loop
(`crates/mcp-fs/src/storage/mod.rs:357`, reached from `GitRepoStore::purge_repo`,
`crates/mcp-fs/src/git/repo.rs:161`) and the conformance suite
(`crates/mcp-fs/src/storage/conformance.rs:441`). On SQLite the index is a file and
`purge_repo` deletes it (`repo.rs:146-156`), so the `TABLES` path is only exercised on a
shared relational backend — that is why E2E-NEW-806 and E2E-NEW-807 run against
PostgreSQL and E2E-NEW-808 covers the SQLite branch separately.

---

**E2E-NEW-806 — Happy — deleting a project removes its paused-operation row**

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

**E2E-NEW-807 — SideEffect — the purge is scoped to the deleted volume only**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-NEW-276, FR-NEW-283.
- **Preconditions:** as E2E-NEW-806 with two projects, `proj1` and `proj2`, both FX-FORK, both members of `owner@test.com`, sharing one PostgreSQL database.
- **Given** both volumes hold a paused rebase: `SELECT volume_id FROM git_operations ORDER BY volume_id` returns exactly `["proj1","proj2"]`.
- **When** `admin.delete_project {"project_id":"proj1"}`.
- **Then** `SELECT volume_id FROM git_operations` returns exactly `["proj2"]` — one row, `proj2`'s.
- **And** the surviving row's `op_type == "rebase"`, `current_step == 1`, `total_steps == 2` and `conflicts` holding exactly one element whose `path` is `/a.txt`, unchanged from before the delete (field-by-field compare against the row read beforehand, ignoring `updated_at`).
- **And** `git.status {mount_id:"proj2"}` still reports `operation.op_type == "rebase"`.
- **And** `git.rebase_continue {mount_id:"proj2", resolutions:[{path:"/a.txt",strategy:"theirs"}]}` completes normally, proving the neighbouring delete did not disturb `proj2`.
- **Verification:** relational row list; field compare; tool responses.
- **Cleanup:** delete `proj2`; drop the schema. **Priority:** P0.

---

**E2E-NEW-808 — EdgeCase — on SQLite the index file carries the rows away with it**

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

### Z.2 Gap 2 — requirements that had exactly one test

#### FR-DEL-101 — the removed `on_conflict` parameter
Existing: E2E-NEW-535 (Failure, rejected on a divergent pull).

**E2E-NEW-810 — EdgeCase — `on_conflict` is absent from the frozen schema**

**Category:** EdgeCase. **Scenario:** SC-912. **Requirements:** FR-DEL-101, FR-NEW-345.
- **When** the registry is built and `r.resolve("git.remote_pull").unwrap().schema` is read.
- **Then** `schema["properties"]` has exactly the key set `["mount_id","branch","remote"]` and `"on_conflict"` is absent.
- **And** `tool-contract-golden.json`'s entry for `git.remote_pull` contains no `on_conflict` and `TOOL_CONTRACT.txt` mentions the string `on_conflict` zero times.
- **And** the symbols `ConflictStrategy` and `parse_on_conflict` resolve nowhere under `crates/` (compile failure if reintroduced; asserted by their absence from the golden contract's regenerated output).
- **Verification:** JSON key-set equality on the registered schema; golden contract text search.
- **Cleanup:** none. **Priority:** P0.

**E2E-NEW-811 — Failure — `on_conflict` is rejected even on a pull that would fast-forward**

**Category:** Failure. **Scenario:** SC-912. **Requirements:** FR-DEL-101.
- **Preconditions:** SEED-R; the bare remote advanced by one commit so the local `main` is strictly behind (a pure fast-forward pull, no divergence). Drives the registered tool.
- **When** `git.remote_pull {mount_id:"gitproj", branch:"main", on_conflict:"ours"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"on_conflict"` — the unknown parameter is refused by schema validation before the fast-forward path is reached.
- **And** `db.get_ref("refs/heads/main").target` is still `<sha-C2>` (no fast-forward happened).
- **And** `db.get_ref("refs/remotes/origin/main")` is unchanged (no fetch happened either).
- **Verification:** error assertion; two ref reads compared against values captured before the call.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

#### FR-MOD-105 — a conflicted pull is completed by the merge tools
Existing: E2E-NEW-508 (Happy, `git.merge_resolve` finishes it).

**E2E-NEW-812 — Failure — `git.rebase_continue` cannot finish a paused pull**

**Category:** Failure. **Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-NEW-225.
- **Preconditions:** as E2E-NEW-508 (pull driven through `pull_branch`), paused with `conflicts[0].path == "/src/config.toml"`.
- **When** `git.rebase_continue {mount_id:"proj", resolutions:[{path:"/src/config.toml",strategy:"theirs"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"pull"`, `"git.merge_resolve"` and `"git.merge_abort"`.
- **And** the `git_operations` row still has `op_type == "merge"` (a pull is recorded as a merge, FR-NEW-285) and `conflicts == ["/src/config.toml"]`.
- **And** `/src/config.toml` line 2 is still `port = 8000` (the local side; nothing applied).
- **And** `git.cherry_pick_continue` with the same resolutions errors identically.
- **Verification:** error code + substrings on both calls; relational row read; byte compare.
- **Cleanup:** `git.merge_abort`. **Priority:** P0.

**E2E-NEW-813 — SideEffect — `git.merge_abort` on a paused pull keeps the fetched objects**

**Category:** SideEffect. **Scenario:** SC-912. **Requirements:** FR-MOD-105, FR-MOD-104, FR-NEW-197.
- **Preconditions:** as E2E-NEW-508, paused; the fetched remote tip is `<sha-R2>`.
- **Given** `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"` and `db.object_exists("<sha-R2>")` is `true`.
- **When** `git.merge_abort {mount_id:"proj"}`.
- **Then** the call succeeds and no `git_operations` row remains.
- **And** `db.get_ref("refs/heads/main").target` equals the pre-pull local tip exactly.
- **And** `/src/config.toml` is byte-identical to its pre-pull bytes (line 2 `port = 8000`).
- **And** `db.get_ref("refs/remotes/origin/main").target` is still `<sha-R2>` and `db.object_exists("<sha-R2>")` is still `true` — the abort undoes the merge, not the fetch (`git.rs:1820-1823` behaviour preserved).
- **And** a second `git.remote_pull` re-enters the same conflict without re-downloading, proving the objects are local.
- **Verification:** relational count; ref reads; byte compare; `object_exists`; repeat pull.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-MOD-107 — `git.status` reports the in-progress operation
Existing: E2E-NEW-509 (Happy, during a merge).

**E2E-NEW-814 — EdgeCase — the key is absent when nothing is in progress**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-MOD-107.
- **Preconditions:** SEED-CONFLICT, nothing started.
- **When** `git.status {mount_id:"proj"}`.
- **Then** the response has **no** `operation` (the object named by FR-NEW-281) key at all (`resp.get("operation").is_none()`), not a `null` and not an empty object.
- **When** `git.merge {source_ref:"feature"}` conflicts, then `git.merge_abort`.
- **Then** a second `git.status` again has no `operation` (the object named by FR-NEW-281) key, and its `head`, `branch` and `refs` fields are equal key-for-key to the first `git.status` response.
- **Verification:** `serde_json::Value::get` returning `None`; whole-object equality of the two status responses.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-815 — SideEffect — status names the rebase tools, not the merge tools**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-MOD-107, FR-NEW-281.
- **Preconditions:** FX-FORK4 on `proj1`; `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}` pauses at entry index `0` on `/a.txt`.
- **When** `git.status {mount_id:"proj1"}`.
- **Then** `operation` (the object named by FR-NEW-281) is exactly `{"op_type":"rebase","source_ref":null,"current_step":0,"total_steps":4,"remaining_conflicts":["/a.txt"],"continue_with":"git.rebase_continue","abort_with":"git.rebase_abort"}`.
- **And** `continue_with`/`abort_with` does **not** contain `"git.merge_resolve"` or `"git.merge_abort"`.
- **And** the top-level `branch` field is still `"feature"` and `head` equals the pre-rebase `<sha-of-F4>` (the branch has not moved while paused at step 1).
- **Verification:** exact JSON object equality; substring absence; ref/field equality.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

---

#### FR-MOD-109 — the GitLab scope constant includes `api`
Existing: E2E-NEW-773 (Happy, gitlab device-flow scope string).

**E2E-NEW-816 — EdgeCase — the granted scopes are persisted verbatim from the flow**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-MOD-109, FR-NEW-330, FR-NEW-335.
- **Preconditions:** Band D fixture, `MockDeviceFlowClient` returning a token response whose `scope` field is exactly `"api read_repository write_repository"`, person `dev@test.com`, host `gitlab.example.test`.
- **When** `git.auth {mount_id:"acme-self"}` runs the device flow to completion.
- **Then** the recorded authorization request's `scope` parameter is exactly `"api read_repository write_repository"` (space separated, `api` first).
- **And** `OAuthTokenStore` holds, for `(dev@test.com, gitlab.example.test)`, `scopes == ["api","read_repository","write_repository"]` in that order.
- **And** `git.auth_status` reports that entry with `"scopes":["api","read_repository","write_repository"]` and `"pr_capable": true`.
- **And** a following `git.pr_list {mount_id:"acme-self"}` issues the request without any scope pre-check rejection (`mock.calls().len() == 1`).
- **Verification:** recorded request field equality; token store read; `git.auth_status` JSON; mock call count.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-817 — Failure — a token carrying only the old scope pair is refused before the network**

**Category:** Failure. **Scenario:** SC-925. **Requirements:** FR-MOD-109, FR-NEW-332.
- **Preconditions:** Band D fixture; seed `(dev@test.com, gitlab.com)` with token `glpat_TESTTOKEN_old_005`, `scopes == ["read_repository","write_repository"]` (the pre-change constant, a known and known-insufficient set).
- **When** `git.pr_get {mount_id:"acme-lab", pr_number:42}`.
- **Then** assert error `ERR_FORBIDDEN` whose message contains `"api"`, `"gitlab.com"`, `"git.auth"` and `"git.token_set"`.
- **And** `mock.calls().is_empty()` — no request was issued, so the mock's `599 UNROUTED` default was never reached either.
- **And** the stored token is untouched: `scopes` still `["read_repository","write_repository"]`, token value unchanged.
- **And** the error message contains neither `"glpat"` nor the token value.
- **Verification:** error code + four substrings; mock call list empty; token store read; substring absence.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-102 — duplicate branch name
Existing: E2E-NEW-402 (Failure, `main` already exists).

**E2E-NEW-818 — EdgeCase — refs are case-sensitive, so `Main` is not a duplicate of `main`**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-102, FR-NEW-104.
- **Preconditions:** SEED-A (`refs/heads/main` = `<sha-C2>`).
- **When** `git.branch_create {"mount_id":"gitproj","name":"Main","start_point":"<sha-C1>"}`.
- **Then** the call succeeds with `{"branch":"Main","sha":"<sha-C1>","checked_out":false}`.
- **And** `db.get_ref("refs/heads/Main").target == "<sha-C1>"` and `db.get_ref("refs/heads/main").target == "<sha-C2>"` — two distinct rows.
- **And** `db.list_refs()` holds exactly `["HEAD","refs/heads/Main","refs/heads/main"]`.
- **And** a repeat `git.branch_create {"name":"Main","start_point":"<sha-C2>"}` now errors `ERR_NO_CLOBBER` containing `"branch 'Main' already exists"`, and `refs/heads/Main` is still `<sha-C1>`.
- **Verification:** two ref reads; ref list equality; repeat-call error assertion.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-819 — SideEffect — a refused duplicate writes no audit entry and charges no quota**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-102.
- **Preconditions:** SEED-A; capture `before_audit = state.safety.audit(OWNER, MOUNT).len()` and `before_bytes = state.safety.bytes_written(OWNER, MOUNT)`.
- **When** `git.branch_create {"name":"main","start_point":"<sha-C1>","checkout":true}` (duplicate **and** requesting a checkout, so a naive implementation would have rewritten the volume first).
- **Then** assert error `ERR_NO_CLOBBER` containing `"branch 'main' already exists"`.
- **And** `state.safety.audit(OWNER, MOUNT).len() == before_audit` and `bytes_written == before_bytes`.
- **And** `db.get_ref("HEAD")` is still `{target:"refs/heads/main", symbolic:true}` and `/src/lib.rs` still reads `b"fn a() {}\n"` (no `<sha-C1>` checkout leaked through).
- **Verification:** audit length and quota compare; ref read; byte compare.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-107 — switching to the current branch is a no-op
Existing: E2E-NEW-417 (EdgeCase, the no-op itself).

**E2E-NEW-820 — SideEffect — the no-op switch writes nothing at all**

**Category:** SideEffect. **Scenario:** SC-901. **Requirements:** FR-NEW-107.
- **Preconditions:** SEED-A, HEAD on `main`; capture `before_audit` (full `Vec<AuditEntry>`) and `before_bytes`.
- **When** `git.branch_switch {"mount_id":"gitproj","name":"main"}`.
- **Then** the response is exactly `{"branch":"main","sha":"<sha-C2>","changed":false,"files_changed":0}`.
- **And** `state.safety.audit(OWNER, MOUNT)` is equal element-for-element to `before_audit` — not even a zero-byte `git.branch_switch` entry.
- **And** `bytes_written == before_bytes`.
- **And** the modification time recorded for `/README.md` in the volume metadata is unchanged (the file was not rewritten with identical bytes).
- **Verification:** exact response equality; audit vector equality; quota equality; node metadata read via `state.stores.client(MOUNT)`.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-821 — Failure — the dirty check runs before the no-op shortcut**

**Category:** Failure. **Scenario:** SC-901. **Requirements:** FR-NEW-107, FR-NEW-106.
- **Preconditions:** SEED-A, HEAD on `main`; `fs.write_text {path:"/README.md", content:"alpha-DIRTY\n"}`.
- **When** `git.branch_switch {"name":"main"}` (switching to the branch already checked out, on a dirty volume).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"git.commit"` and `"git.stash_save"` — the no-op shortcut does not bypass the dirty guard, so the caller gets one consistent rule.
- **And** `/README.md` still reads exactly `b"alpha-DIRTY\n"` (the dirt is preserved, not silently discarded by a "harmless" rewrite).
- **And** `db.get_ref("HEAD")` is unchanged and `state.safety.audit(OWNER, MOUNT)` holds no `git.branch_switch` entry.
- **Verification:** error assertion; byte compare; ref read; audit scan.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-127 — drop a stash
Existing: E2E-NEW-461 (Happy, drop a known id).

**E2E-NEW-822 — SideEffect — drop touches neither the volume nor the other entries**

**Category:** SideEffect. **Scenario:** SC-902. **Requirements:** FR-NEW-127, FR-NEW-122.
- **Preconditions:** SEED-A; write `/README.md` = `"alpha-1\n"`, `git.stash_save {message:"wip one"}` -> `<stash-1>`; write `/README.md` = `"alpha-2\n"`, `git.stash_save {message:"wip two"}` -> `<stash-2>`. Volume back at `<sha-C2>`'s tree.
- **When** `git.stash_drop {"mount_id":"gitproj","stash_id":"<stash-1>"}`.
- **Then** the response is exactly `{"stash_id":"<stash-1>","dropped":true}`.
- **And** `git.stash_list` returns exactly one entry, `stash_id == "<stash-2>"`, `message == "wip two"`.
- **And** `db.get_ref("refs/stash/<stash-1>")` is `None` and `db.get_ref("refs/stash/<stash-2>")` is unchanged.
- **And** `/README.md` still reads exactly `b"alpha\n"` and `bytes_written` is unchanged from before the drop.
- **And** `db.object_exists("<stash-1>")` is still `true` — the ref is removed, the commit object is not pruned by the drop.
- **Verification:** exact response; stash list; two ref reads; byte compare; quota; `object_exists`.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-823 — Failure — dropping the same id twice**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-127, FR-NEW-128.
- **Preconditions:** SEED-A plus one stash `<stash-1>` as above.
- **When** `git.stash_drop {"stash_id":"<stash-1>"}` succeeds, then the identical call is issued a second time.
- **Then** the second call asserts error `ERR_NOT_FOUND` containing `"<stash-1>"` — drop is not idempotent-silent, an already-gone id is reported.
- **And** `git.stash_list` returns `[]` (an empty array, not an error).
- **And** `state.safety.audit(OWNER, MOUNT)` holds exactly **one** entry whose `op == "git.stash_drop"`, from the successful call only.
- **Verification:** error assertion; empty-list equality; audit count by op.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-130 — bound the stash pool
Existing: E2E-NEW-467 (EdgeCase, 100 accepted / 101st refused).

**E2E-NEW-824 — Failure — the refusal names the limit and the remedy**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-130.
- **Preconditions:** `Env::build` with `c.git.max_stash_entries = 3`; SEED-A; three stashes saved (`"wip 1"`, `"wip 2"`, `"wip 3"`), volume made dirty again with `/README.md` = `"alpha-4\n"`.
- **When** `git.stash_save {"mount_id":"gitproj","message":"wip 4"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"3"` and `"git.stash_drop"`.
- **And** `git.stash_list` still returns exactly 3 entries with messages `["wip 3","wip 2","wip 1"]` (newest first).
- **And** `/README.md` still reads exactly `b"alpha-4\n"` — the refused save did not rewrite the volume to HEAD.
- **And** `db.list_refs()` holds exactly 3 refs under `refs/stash/`.
- **Verification:** error code + two substrings; stash list; byte compare; ref list filter.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-825 — EdgeCase — dropping one entry at the cap re-opens a slot**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-130, FR-NEW-127.
- **Preconditions:** as E2E-NEW-824, at the cap of 3, volume dirty with `/README.md` = `"alpha-4\n"`, save already refused once.
- **When** `git.stash_drop {"stash_id":"<stash-1>"}` then `git.stash_save {"message":"wip 4"}`.
- **Then** the save succeeds and its response `message == "wip 4"`.
- **And** `git.stash_list` returns exactly 3 entries, messages `["wip 4","wip 3","wip 2"]` — the count is a live check, not a monotonic counter.
- **And** `/README.md` reads exactly `b"alpha\n"` (the successful save rewrote the volume to HEAD's tree).
- **Verification:** success response; stash list ordering; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

---

#### FR-NEW-143 — removing an unknown remote
Existing: E2E-NEW-478 (Failure, unknown name).

**E2E-NEW-826 — EdgeCase — remote names are case-sensitive**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-143, FR-NEW-141.
- **Preconditions:** SEED-R (remote `origin` -> `<url>`).
- **When** `git.remote_remove {"mount_id":"gitproj","name":"Origin"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"Origin"` (the casing as supplied, echoed back).
- **And** `db.list_remotes()` still holds exactly one row `{name:"origin", url:"<url>"}` — `RelationalGitDb::remove_remote` is an unconditional DELETE (`git/db.rs:287-296`), so this proves the existence check sits in the tool layer above it.
- **And** `git.remote_add {"name":"Origin","url":"<url>"}` then succeeds, and `db.list_remotes()` holds two rows, `origin` and `Origin`.
- **Verification:** error assertion; `list_remotes` row compare; follow-up add.
- **Cleanup:** tempdir drop. **Priority:** P1.

**E2E-NEW-827 — SideEffect — a failed remove leaves the tracking refs intact**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-143, FR-NEW-142.
- **Preconditions:** SEED-R plus a completed `fetch_branch_inner` so `refs/remotes/origin/main` = `<sha-R1>` exists.
- **When** `git.remote_remove {"name":"upstream"}` (never added).
- **Then** assert error `ERR_NOT_FOUND` containing `"upstream"`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` — the delete of `refs/remotes/{name}/*` mandated by FR-NEW-142 did not run with an empty or wildcard name.
- **And** `db.list_refs()` is element-for-element equal to the list captured before the call.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no `git.remote_remove` entry.
- **Verification:** error assertion; ref read; ref-list vector equality; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### FR-NEW-158 — a lease without force is contradictory
Existing: E2E-NEW-497 (Failure, lease supplied with `force:false`).

**E2E-NEW-828 — EdgeCase — an empty-string lease with `force:false` is treated as absent**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-158, FR-NEW-156.
- **Preconditions:** SEED-R; local `main` is strictly ahead of remote `<sha-R1>` (a plain fast-forward push). Drives `push_branch_inner`.
- **When** the push is called with `force: false` and `expected_remote_sha: ""`.
- **Then** the push **succeeds**: an empty lease is absent, not a contradiction, so the non-forcing fast-forward path runs. Response `forced` is `false` and `overwritten_sha` is absent.
- **And** the bare remote's `refs/heads/main` reads the new local tip (`git2::Repository::open_bare` + `find_reference`).
- **And** the same call with `expected_remote_sha: "<sha-R1>"` (a real value) and `force:false` asserts error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` and `"force"` — contrasting the two cases in one test so the boundary is explicit.
- **Verification:** success response fields; bare-repo ref read; second-call error assertion.
- **Cleanup:** tempdir drop. **Priority:** P1.

**E2E-NEW-829 — SideEffect — the contradiction is caught before any network contact**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-158.
- **Preconditions:** SEED-R with the bare remote at `<sha-R1>`; local `main` diverged from it. Drives the registered `git.remote_push` tool (this is a validation test).
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","force":false,"expected_remote_sha":"<sha-R1>"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` and `"force"`.
- **And** the bare remote's `refs/heads/main` still equals `<sha-R1>` exactly.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no `git.remote_push` entry.
- **And** the remote's `objects/` directory holds no object whose sha equals the local tip (nothing was uploaded before the rejection).
- **Verification:** error assertion; bare-repo ref read; audit scan; `Repository::odb().exists(oid)` on the bare repo.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### FR-NEW-159 — a forced push is audited with the sha it destroyed
Existing: E2E-NEW-491 (SideEffect, audit names the overwritten sha).

**E2E-NEW-830 — EdgeCase — a create-only force push audits the all-zero lease**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-159, FR-NEW-155.
- **Preconditions:** SEED-R; the bare remote has **no** `refs/heads/sandbox`. Drives `push_branch_inner`.
- **When** the push runs with `branch:"main"`, `remote_branch:"sandbox"`, `force:true`, `expected_remote_sha:"0000000000000000000000000000000000000000"`.
- **Then** the response has `forced == true`, `overwritten_sha == "0000000000000000000000000000000000000000"` and `new_sha == "<sha-C2>"`.
- **And** exactly one audit entry with `op == "git.remote_push"` is appended, whose `detail` contains `"forced"`, `"sandbox"` and the 40 zeros.
- **And** the bare remote now has `refs/heads/sandbox` = `<sha-C2>` and `refs/heads/main` still `<sha-R1>`.
- **Verification:** response fields; audit entry `op`/`detail` substrings; two bare-repo ref reads.
- **Cleanup:** tempdir drop. **Priority:** P1.

**E2E-NEW-831 — Failure — a lease-rejected force push writes no audit entry**

**Category:** Failure. **Scenario:** SC-910. **Requirements:** FR-NEW-159, FR-NEW-157.
- **Preconditions:** SEED-R at `<sha-R1>`, then `advance_bare_remote` moves the remote to `<sha-R2>`; the caller still holds `<sha-R1>`. Drives `push_branch_inner`.
- **Given** `before_audit = state.safety.audit(OWNER, MOUNT)`.
- **When** the push runs with `force:true`, `expected_remote_sha:"<sha-R1>"`.
- **Then** assert error whose message contains both `"<sha-R1>"` and `"<sha-R2>"`.
- **And** `state.safety.audit(OWNER, MOUNT)` is element-for-element equal to `before_audit` — a destroyed-sha audit line is written only when something was actually destroyed, so the audit log never claims a force push that did not happen.
- **And** the bare remote's `refs/heads/main` is still `<sha-R2>`.
- **Verification:** error substrings; audit vector equality; bare-repo ref read.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### FR-NEW-160 — force never applies implicitly
Existing: E2E-NEW-494 (Failure, `force:false` non-fast-forward).

**E2E-NEW-832 — EdgeCase — a fast-forward push with `force:false` still succeeds**

**Category:** EdgeCase. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
- **Preconditions:** SEED-R; local `main` contains `<sha-R1>` in its ancestry and adds one commit `<sha-C9>`. Drives `push_branch_inner`.
- **When** the push runs with `force` absent entirely.
- **Then** the push succeeds, the response has `forced == false` and no `overwritten_sha` key.
- **And** the bare remote's `refs/heads/main` == `<sha-C9>`.
- **And** the audit entry's `detail` does not contain the string `"forced"`.
- **Verification:** response key absence; bare-repo ref read; audit detail substring absence.
- **Cleanup:** tempdir drop. **Priority:** P0.

**E2E-NEW-833 — SideEffect — the refused non-fast-forward leaves the remote byte-identical**

**Category:** SideEffect. **Scenario:** SC-908. **Requirements:** FR-NEW-160.
- **Preconditions:** SEED-R, remote advanced to `<sha-R2>` with `/hello.txt` = `"remote-2\n"`; local `main` diverged at `<sha-C2>`. Drives `push_branch_inner` with `force:false`.
- **When** the push runs.
- **Then** assert error containing `"not a fast-forward"` and `"force"`.
- **And** the bare remote's `refs/heads/main` == `<sha-R2>`, and reading `/hello.txt` out of that commit's tree via `git2` gives exactly `b"remote-2\n"`.
- **And** `db.get_ref("refs/remotes/origin/main")` on the local side is unchanged (a refused push does not fabricate a tracking update).
- **And** no audit entry with `op == "git.remote_push"` was added.
- **Verification:** error substrings; bare-repo ref + blob read; local ref read; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### FR-NEW-173 — auto-mergeable divergence needs no caller decision
Existing: E2E-NEW-500 (Happy, `git.merge` on SEED-AUTOMERGE).

**E2E-NEW-834 — EdgeCase — a rebase step whose sides are disjoint replays without pausing**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-173, FR-NEW-211.
- **Preconditions:** Band C, `proj1`: `C1` commits `/cfg.toml` = `"a = 1\nb = 2\nc = 3\nd = 4\ne = 5\n"`; `main` adds `C2` setting line 1 to `"a = 99\n"`; `feature` off `C1` adds `F1` setting line 5 to `"e = 99\n"`. HEAD on `feature`.
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-F1>"}]}`.
- **Then** `status == "completed"` — no `conflicts` key, no `operation_id` key in the response.
- **And** `/cfg.toml` reads exactly `b"a = 99\nb = 2\nc = 3\nd = 4\ne = 99\n"` (both edits present).
- **And** `git.log {ref_name:"feature"}` holds 3 commits, the tip's single parent being `<sha-of-C2>`.
- **And** no `git_operations` row for `volume_id = "proj1"` was created at any point (asserted immediately after the call).
- **Verification:** JSON key absence; byte-exact read; log parents; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-835 — SideEffect — an automatic merge records nothing to resolve**

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

#### FR-NEW-180 — delete/modify conflict
Existing: E2E-NEW-564 (EdgeCase, the conflict shape).

**E2E-NEW-836 — Happy — choosing the modifying side keeps the file with its exact bytes**

**Category:** Happy. **Scenario:** SC-904. **Requirements:** FR-NEW-180, FR-NEW-174.
- **Preconditions:** SEED-DELMOD (`feature` deletes `/lib/util.rs`; `main` rewrites it to `pub fn a() { println!("x"); }\n`). HEAD on `main`.
- **Given** `git.merge {source_ref:"feature"}` returns `status == "conflict"` with one entry: `path == "/lib/util.rs"`, `ours.exists == true`, `theirs.exists == false`, `base.exists == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `files_changed == 1`.
- **And** `/lib/util.rs` reads exactly `b"pub fn a() { println!(\"x\"); }\n"` — the modifying side, byte for byte.
- **And** the merge commit has exactly 2 parents, parent 0 = `main`'s pre-merge tip, parent 1 = `feature`'s tip.
- **And** no `git_operations` row remains.
- **Verification:** conflict shape fields; byte-exact read; `parent_count()`/`parent_id()`; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-837 — Failure — an invented strategy for the deleted side is refused**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-180, FR-NEW-179.
- **Preconditions:** SEED-DELMOD, merge paused as above.
- **When** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"delete"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"delete"`, `"ours"` and `"theirs"` — the deleting side is chosen by naming the side, never by a special verb.
- **And** the `git_operations` row still lists `/lib/util.rs` in `conflicts`, with no recorded resolution for it.
- **And** `/lib/util.rs` still reads exactly `b"pub fn a() { println!(\"x\"); }\n"` (the pre-merge volume state).
- **And** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"theirs"}]}` then succeeds and the file is gone: `VolumeClient::read_bytes` errors `ERR_NOT_FOUND`.
- **Verification:** error code + three substrings; relational row read; byte compare; recovery call plus read failure.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-181 — a both-deleted path resolves to deletion
Existing: E2E-NEW-563 (EdgeCase, both sides deleted).

**E2E-NEW-838 — SideEffect — the both-deleted path is absent from the conflict set and from the volume**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-181, FR-NEW-171.
- **Preconditions:** SEED-DELDEL (both branches delete `/tmp/scratch.txt`; `/keep.txt` is `k-main\n` vs `k-feature\n`). HEAD on `main`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** `status == "conflict"` and `conflicts` holds **exactly one** entry, `path == "/keep.txt"`; no entry has `path == "/tmp/scratch.txt"`.
- **And** while paused, `/tmp/scratch.txt` is still present in the volume with its base bytes `b"x\n"` (a conflict applies nothing, including the auto-resolved deletion).
- **When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"ours"}]}`.
- **Then** `VolumeClient::read_bytes("/tmp/scratch.txt")` errors `ERR_NOT_FOUND` and the merge commit's tree holds no `tmp/` entry.
- **And** `/keep.txt` reads exactly `b"k-main\n"`.
- **Verification:** conflict array length and paths; volume read while paused; volume read and `git.show` tree listing after; byte compare.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-839 — EdgeCase — both sides delete an entire directory**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-181.
- **Preconditions:** `C0` commits `/old/a.txt` = `"a\n"`, `/old/b.txt` = `"b\n"`, `/keep.txt` = `"k\n"`; `feature` deletes both files under `/old/` and sets `/keep.txt` = `"k-feature\n"`; `main` deletes both files under `/old/` and sets `/keep.txt` = `"k-main\n"`. HEAD on `main`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** `conflicts` holds exactly one entry, `/keep.txt`; neither `/old/a.txt` nor `/old/b.txt` appears.
- **When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"theirs"}]}`.
- **Then** `fs.list {path:"/"}` returns entries whose names do not include `old`, and `fs.list {path:"/old"}` errors `ERR_NOT_FOUND`.
- **And** `/keep.txt` reads exactly `b"k-feature\n"`.
- **Verification:** conflict array; `fs.list` results; error assertion; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

---

#### FR-NEW-182 — a binary conflict carries a binary marker
Existing: E2E-NEW-565 (EdgeCase, the binary conflict entry).

**E2E-NEW-840 — Failure — literal content for a binary path is refused**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-182, FR-NEW-175.
- **Preconditions:** SEED-BINARY (`/assets/logo.png`, feature last byte `0x02`, main `0x03`), merge paused with `conflicts[0].binary == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", content:"\u0089PNG"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"binary"`, `"ours"` and `"theirs"`.
- **And** `/assets/logo.png` still reads the exact 10 bytes `89 50 4E 47 0D 0A 1A 08 00 03`-form of the pre-merge main side (`[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]`), compared as a byte slice.
- **And** the `git_operations` row retains `/assets/logo.png` in `conflicts` with no recorded resolution.
- **And** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", strategy:"theirs"}]}` then succeeds and the file's last byte is `0x02`.
- **Verification:** error code + substrings; byte-slice equality; relational row read; recovery call plus byte read.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-841 — SideEffect — the binary entry omits the bytes but reports the sizes**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-182, FR-NEW-170.
- **Preconditions:** SEED-BINARY, merge paused.
- **When** the conflict response is inspected.
- **Then** `conflicts[0]` is exactly `{"path":"/assets/logo.png","ours":{"exists":true,"content":null},"theirs":{"exists":true,"content":null},"base":{"exists":true,"content":null},"binary":true,"type_change":false}` — per FR-NEW-186 a binary side reports `exists` with `content` null, and **no** `content` key.
- **And** the serialized response, converted to a string, contains none of the byte sequences `"\u0000"`, `"PNG"` or `"\u0089"` (no smuggled inline bytes, no lossy UTF-8 replacement characters either: the string contains no `U+FFFD`).
- **And** the whole response is valid JSON that round-trips through `serde_json::from_str` unchanged.
- **Verification:** exact JSON object equality; substring absence on the serialized form; round-trip equality.
- **Cleanup:** `git.merge_abort`. **Priority:** P0.

---

#### FR-NEW-192 — an already-merged source is a reported no-op
Existing: E2E-NEW-510 (Happy, second merge of the same branch).

**E2E-NEW-842 — SideEffect — the no-op merge charges no quota and writes one audit line at most**

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

**E2E-NEW-843 — EdgeCase — merging a raw sha that is already an ancestor**

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

#### FR-NEW-193 — a fast-forwardable merge fast-forwards
Existing: E2E-NEW-501 (Happy, merge of a strictly-ahead branch).

**E2E-NEW-844 — EdgeCase — `squash:true` on a fast-forwardable merge commits instead of fast-forwarding**

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

**E2E-NEW-845 — SideEffect — the fast-forward creates no new object at all**

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

#### FR-NEW-194 — merge refuses a dirty volume
Existing: E2E-NEW-517 (Failure, dirty volume).

**E2E-NEW-846 — EdgeCase — dirt consisting only of a new untracked file still refuses**

**Category:** EdgeCase. **Scenario:** SC-903. **Requirements:** FR-NEW-194.
- **Preconditions:** SEED-AUTOMERGE (a merge that would otherwise succeed with no conflict).
- **Given** `fs.write_text {path:"/notes.md", content:"todo\n"}` — a file present in neither tree, so `git.status.changes == [{"path":"/notes.md","status":"added"}]`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"` — "dirty" includes additions, matching the pull dirty check.
- **And** `/notes.md` still reads exactly `b"todo\n"` and `/src/config.toml` line 2 is still `port = 8000` (the automerge did not run).
- **And** `db.get_ref("refs/heads/main").target` is unchanged.
- **Verification:** error code + substrings; two byte compares; ref read.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-847 — SideEffect — the dirty refusal records no operation and no fetch-like side effect**

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

#### FR-NEW-195 — an unknown source ref is not found
Existing: E2E-NEW-515 (Failure, `source_ref:"nope"`).

**E2E-NEW-848 — EdgeCase — a well-formed but absent 40-hex sha**

**Category:** EdgeCase. **Scenario:** SC-903. **Requirements:** FR-NEW-195.
- **Preconditions:** SEED-CONFLICT.
- **When** `git.merge {source_ref:"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"` — `resolve_ref` treats an all-hex name as a raw sha (`git.rs:486-504`), so the failure must come from the absent object, not from name parsing.
- **When** `git.merge {source_ref:"refs/heads/ghost"}` (a full ref path that does not exist).
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/heads/ghost"`.
- **And** after both calls `db.get_ref("refs/heads/main").target` is unchanged and no `git_operations` row exists.
- **Verification:** two error assertions; ref read; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-849 — SideEffect — the unknown source is rejected before the dirty check writes anything**

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

#### FR-NEW-216 — the todo length is bounded
Existing: E2E-NEW-626 (Failure, todo longer than the bound).

**E2E-NEW-850 — EdgeCase — exactly `max_rebase_todo` entries are accepted**

**Category:** EdgeCase. **Scenario:** SC-914. **Requirements:** FR-NEW-216, FR-NEW-210.
- **Preconditions:** `Env::build` with default `c.git.max_rebase_todo = 200`; `proj1` seeded with `C1` then 200 commits `S1..S200`, each adding `/s/{i:03}.txt` = `"s{i}\n"`, on branch `feature`; `main` = `C1` plus one unrelated commit `C2` adding `/m.txt` = `"m\n"`.
- **When** `git.rebase {onto:"main", todo:[pick S1, ..., pick S200]}` (exactly 200 entries, `assert_eq!(todo.len(), 200)` in-test).
- **Then** the call succeeds with `status == "completed"` and `replayed == 200`.
- **And** `git.log {ref_name:"feature"}` holds exactly 202 commits and `/s/200.txt` reads `b"s200\n"`.
- **And** `/m.txt` reads `b"m\n"` (the new base is present).
- **Verification:** success response; log length; two byte reads.
- **Cleanup:** fixture drop. **Priority:** P2.

**E2E-NEW-851 — Failure — a lowered `git.max_rebase_todo` is honoured and named**

**Category:** Failure. **Scenario:** SC-914. **Requirements:** FR-NEW-216, FR-NEW-331.
- **Preconditions:** `Env::build` with `c.git.max_rebase_todo = 5`; `feature` holds 6 commits `S1..S6` off `C1`; `main` = `C1` + `C2`.
- **When** `git.rebase {onto:"main", todo:[pick S1 .. pick S6]}` (6 entries).
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"5"` and `"6"` — both the configured limit and the supplied length.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-S6>"` (unchanged) and no `git_operations` row exists.
- **When** the first entry is dropped from the todo, leaving 5, and the range is adjusted to start at `S2`'s parent.
- **Then** the rebase succeeds — the bound is a count check on the list, not a repository-size check.
- **Verification:** error code + two substrings; ref read; relational count; second-call success.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-219 — a conflicting step pauses the rebase
Existing: E2E-NEW-613 (Failure, single pause on step 1).

**E2E-NEW-852 — SideEffect — the paused row holds every field the spec names**

**Category:** SideEffect. **Scenario:** SC-916. **Requirements:** FR-NEW-219, FR-NEW-275., FR-NEW-187
- **Preconditions:** FX-FORK4; pre-rebase `feature` tip `<sha-of-F4>`, `main` tip `<sha-of-C2>`.
- **When** `git.rebase {onto:"main", todo:[pick F1, pick F2, pick F3, pick F4]}` pauses at entry index `0`.
- **Then** the single `git_operations` row for `volume_id='proj1'` has exactly: `op_type == "rebase"`, `state == "conflicted"`, `onto_sha == "<sha-of-C2>"`, `original_tip_sha == "<sha-of-F4>"`, `current_step == 0` (the pause is on F1, todo index 0), `total_steps == 4`, `conflicts` parsing to `["/a.txt"]`, `resolutions` parsing to an empty object, and `created_at == updated_at`.
- **And** `todo` parses to the four entries in the supplied order with actions `["pick","pick","pick","pick"]` and the four original shas.
- **And** `db.get_ref("refs/heads/feature").target == "<sha-of-F4>"` — the branch has not moved while paused at todo index 0.
- **Verification:** direct relational row read, field by field; JSON parse of `todo`/`conflicts`/`resolutions`; ref read.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

**E2E-NEW-853 — EdgeCase — a conflict on the last todo entry pauses with `current_step == total_steps - 1`**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-219, FR-NEW-220.
- **Preconditions:** FX-FORK4 reordered so the conflicting entries come last: todo `[pick F2, pick F4, pick F1]` is invalid (FR-NEW-215 requires the full range), so instead use **FX-TAIL**: `main` = C1 + C2 (`/a.txt` line 2 = `MAIN`); `feature` = C1 + T1 (adds `/t1.txt` = `"t1\n"`) + T2 (sets `/a.txt` line 2 = `FEAT`, conflicting with C2).
- **When** `git.rebase {onto:"main", todo:[{action:"pick",sha:"<sha-of-T1>"},{action:"pick",sha:"<sha-of-T2>"}]}`.
- **Then** `status == "conflict"`, `current_step == 1` and `total_steps == 2` (the last of two entries is zero-based index 1, per FR-NEW-187).
- **And** the `git_operations` row has `current_step == 1`, `total_steps == 2`.
- **And** `/t1.txt` reads exactly `b"t1\n"` — step 1 stayed replayed, the pause is at a commit boundary (the replay tip holds T1's replayed commit, and `git.log` of the replay tip shows it).
- **And** `/a.txt` reads exactly `b"a1\nMAIN\n"` (step 2 applied nothing).
- **And** `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}` finishes the rebase with `replayed == 2`.
- **Verification:** the response's top-level `current_step` and `total_steps`; relational row; two byte reads; completion call.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-220 — a rebase can pause more than once
Existing: E2E-NEW-615 (Happy, steps 2 and 4 conflict).

**E2E-NEW-854 — SideEffect — the step index moves forward between pauses and never backwards**

**Category:** SideEffect. **Scenario:** SC-916. **Requirements:** FR-NEW-220, FR-NEW-275.
- **Preconditions:** FX-FORK4 (F1 and F3 conflict on `/a.txt`, F2 and F4 do not). Rebase started, paused on F1, which is todo index 0.
- **Given** the row reads `current_step == 0`, `total_steps == 4`, `updated_at == created_at`.
- **When** `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"theirs"}]}`.
- **Then** the response is `status == "conflict"` with `current_step == 2` and `total_steps == 4` — todo indices 0 and 1 are done, the pause is at index 2 (the third entry), zero-based per FR-NEW-187.
- **And** the row now reads `current_step == 2`, `total_steps == 4`, `conflicts` holding exactly one element whose `path` is `/a.txt`, `resolutions` back to an empty object (the previous step's resolutions were consumed, not carried over), and `updated_at > created_at`.
- **And** `original_tip_sha` and `onto_sha` are unchanged from the first read (the abort target is fixed at start).
- **And** `/f2.txt` reads exactly `b"f2\n"` (entry 2 replayed during the resume).
- **Verification:** two relational row reads compared field by field; the response's `current_step` and `total_steps`; byte read.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

**E2E-NEW-855 — EdgeCase — three pauses in one rebase**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-220, FR-NEW-221.
- **Preconditions:** **FX-FORK3C**: `main` = C1 + C2 (`/a.txt` line 2 = `MAIN`); `feature` = C1 + G1 + G2 + G3 where each of G1, G2, G3 sets `/a.txt` line 2 to `G1`, `G2`, `G3` respectively (every entry conflicts with the new base in turn).
- **When** `git.rebase {onto:"main", todo:[pick G1, pick G2, pick G3]}` then `git.rebase_continue` three times, each with `[{path:"/a.txt", strategy:"theirs"}]`.
- **Then** the three responses are, in order, `status == "conflict"` with `current_step == 0`, `status == "conflict"` with `current_step == 1`, then on the third continue `status == "completed"` with `replayed == 3`.

  (the first `conflict` comes from `git.rebase` itself; the first and second `rebase_continue` return the next conflict; the third returns `ok`.)
- **And** `/a.txt` reads exactly `b"a1\nG3\n"` — the last replayed side wins, each step having been resolved with `theirs`.
- **And** `git.log {ref_name:"feature"}` holds 5 commits with messages `["G3","G2","G1","C2 main edit","C1 base"]`.
- **And** no `git_operations` row remains.
- **Verification:** ordered response assertions; byte read; log messages; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-221 — continue a paused rebase
Existing: E2E-NEW-614 (Happy, continue with `theirs`).

**E2E-NEW-856 — EdgeCase — continue with literal content rather than a side**

**Category:** EdgeCase. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-175.
- **Preconditions:** FX-FORK, rebase paused on `/a.txt` (ours = `"a1\nMAIN\n"`, theirs = `"a1\nFEAT1\n"`).
- **When** `git.rebase_continue {resolutions:[{path:"/a.txt", content:"a1\nMAIN+FEAT1\n"}]}`.
- **Then** `status == "completed"` and `replayed == 2`.
- **And** `/a.txt` reads exactly `b"a1\nMAIN+FEAT1\n"` — the supplied bytes verbatim, neither side's content and no re-merge attempted.
- **And** the commit created for the resolved entry has `message == "F1 feature edit"` (the original message is kept; only the content was supplied).
- **And** `/f.txt` reads `b"f1\n"` (entry 2 replayed after the resume).
- **Verification:** status/`replayed`; byte-exact read; `git.log` message; byte read.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-857 — Failure — continue while a merge, not a rebase, is in progress**

**Category:** Failure. **Scenario:** SC-916. **Requirements:** FR-NEW-221, FR-NEW-225, FR-NEW-224.
- **Preconditions:** Band C `proj1` seeded like SEED-CONFLICT so that `git.merge {source_ref:"feature"}` pauses with `/src/config.toml` conflicting; the `git_operations` row has `op_type == "merge"`.
- **When** `git.rebase_continue {mount_id:"proj1", resolutions:[{path:"/src/config.toml", strategy:"ours"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"merge"`, `"git.merge_resolve"` and `"git.merge_abort"`.
- **And** the row still has `op_type == "merge"`, `conflicts == ["/src/config.toml"]` and no recorded resolution.
- **And** `git.rebase_abort` errors identically (containing `"merge"`), so neither rebase channel can touch the merge.
- **And** `git.merge_resolve` with the same resolution then succeeds.
- **Verification:** two error assertions; relational row read; completion call.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-238 — a conflicting cherry-pick pauses
Existing: E2E-NEW-645 (Failure, conflict applies nothing).

**E2E-NEW-858 — SideEffect — the cherry-pick row is typed and single-stepped**

**Category:** SideEffect. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-275.
- **Preconditions:** FX-FORK, HEAD on `main` (= `<sha-of-C2>`); `<sha-of-F1>` edits `/a.txt` line 2 to `FEAT1` and conflicts with C2.
- **When** `git.cherry_pick {mount_id:"proj1", commit_sha:"<sha-of-F1>"}`.
- **Then** `status == "conflict"`, `operation == "cherry_pick"`, `continue_with == "git.cherry_pick_continue"`, `abort_with == "git.cherry_pick_abort"`.
- **And** the single `git_operations` row has `op_type == "cherry_pick"`, `current_step == null`, `total_steps == null` (single-step operations report null per FR-NEW-186), `original_tip_sha == "<sha-of-C2>"`, `conflicts` holding exactly one element whose `path` is `/a.txt`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C2>"` and `/a.txt` reads exactly `b"a1\nMAIN\n"`.
- **Verification:** response fields; relational row field-by-field; ref read; byte compare.
- **Cleanup:** `git.cherry_pick_abort`. **Priority:** P0.

**E2E-NEW-859 — EdgeCase — a cherry-pick delete/modify conflict surfaces a null side**

**Category:** EdgeCase. **Scenario:** SC-919. **Requirements:** FR-NEW-238, FR-NEW-180.
- **Preconditions:** `proj1`: `C1` commits `/lib/util.rs` = `"pub fn a() {}\n"`; on `side`, `S1` deletes it; on `main`, `C2` rewrites it to `"pub fn a() { 1 }\n"`. HEAD on `main`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-S1>"}`.
- **Then** `status == "conflict"` with one entry `path == "/lib/util.rs"`, `theirs.exists == false` (the picked commit deleted it), `ours.exists == true` with content `"pub fn a() { 1 }\n"`, `base.exists == true` with content `"pub fn a() {}\n"`.
- **When** `git.cherry_pick_continue {resolutions:[{path:"/lib/util.rs", strategy:"theirs"}]}`.
- **Then** `status == "committed"`, `VolumeClient::read_bytes("/lib/util.rs")` errors `ERR_NOT_FOUND`, and the new commit's tree has no `lib/util.rs` entry.
- **And** the new commit's single parent is `<sha-of-C2>` and its sha differs from `<sha-of-S1>`.
- **Verification:** conflict entry fields; completion response; volume read error; `git.show` tree; parent assertions.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-240 — cherry-picking a merge commit requires a mainline
Existing: E2E-NEW-650 (EdgeCase, merge commit).

**E2E-NEW-860 — Failure — an out-of-range mainline names the actual parent count**

**Category:** Failure. **Scenario:** SC-918. **Requirements:** FR-NEW-240, FR-NEW-262.
- **Preconditions:** FX-MERGE (`MG` has parents `[M1, S1]`), HEAD on a branch `other` off `C1`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:3}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"3"` and `"2"` (the supplied index and the commit's actual parent count).
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:0}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"0"` — the index is 1-based.
- **When** `git.cherry_pick {commit_sha:"<sha-of-M1>", mainline:1}` (a single-parent commit).
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"mainline"` and `"1 parent"`.
- **And** after all three calls, `db.get_ref("refs/heads/other").target` is unchanged and no `git_operations` row exists.
- **Verification:** three error assertions; ref read; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-861 — Happy — `mainline:1` picks the merge's change relative to the first parent**

**Category:** Happy. **Scenario:** SC-918. **Requirements:** FR-NEW-240, FR-NEW-235.
- **Preconditions:** FX-MERGE; branch `other` off `C1` checked out, volume holds `/a.txt` = `"a1\n"` only.
- **When** `git.cherry_pick {commit_sha:"<sha-of-MG>", mainline:1}`.
- **Then** `status == "committed"` and `new_sha` differs from `<sha-of-MG>`.
- **And** `/s.txt` reads exactly `b"s1\n"` (the side branch's change, which is what `MG` added relative to parent 1 `M1`).
- **And** `/m.txt` is absent: `VolumeClient::read_bytes("/m.txt")` errors `ERR_NOT_FOUND` (the mainline's own change was not pulled in).
- **And** the new commit has exactly 1 parent, equal to `<sha-of-C1>`, and `source_sha == "<sha-of-MG>"` in the response.
- **Verification:** response fields; two volume reads; `parent_count()`/`parent_id(0)`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-253 — an unknown reset target changes nothing
Existing: E2E-NEW-669 (Failure, unknown `target_ref`).

**E2E-NEW-862 — SideEffect — the refused hard reset leaves the dirty volume alone**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-253, FR-NEW-251.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; `fs.write_text {path:"/a.txt", content:"a1\na3\nLOCAL\n"}` makes the volume dirty; capture `before_audit`, `before_bytes`.
- **When** `git.reset {mount_id:"proj1", target_ref:"nosuchref", mode:"hard"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"nosuchref"`.
- **And** `/a.txt` still reads exactly `b"a1\na3\nLOCAL\n"` — a hard reset discards uncommitted work, so an unknown target must not begin discarding before resolving.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `audit(OWNER, MOUNT)` equals `before_audit` and `bytes_written` equals `before_bytes`.
- **Verification:** error assertion; byte compare; ref read; audit/quota equality.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-863 — EdgeCase — a branch name that existed and was deleted**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-253.
- **Preconditions:** FX-LINE plus `git.branch_create {name:"tmp/old", start_point:"<sha-of-C2>"}`, then `git.branch_delete {name:"tmp/old"}`.
- **When** `git.reset {target_ref:"tmp/old", mode:"soft"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"tmp/old"` — the object `<sha-of-C2>` still exists, but the *ref* does not, and refs are resolved as refs.
- **When** `git.reset {target_ref:"<sha-of-C2>", mode:"soft"}` (the sha the deleted branch pointed at).
- **Then** the call succeeds with `old_sha == "<sha-of-C4>"`, `new_sha == "<sha-of-C2>"` — the orphaned commit is still reachable by sha.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C2>"`.
- **Verification:** error assertion; success response fields; ref read.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-254 — resetting to the current tip is a no-op
Existing: E2E-NEW-665 (EdgeCase, both modes).

**E2E-NEW-864 — SideEffect — the no-op reset reports equal shas and charges nothing**

**Category:** SideEffect. **Scenario:** SC-921. **Requirements:** FR-NEW-254, FR-NEW-250.
- **Preconditions:** FX-LINE, clean volume, HEAD on `main` = `<sha-of-C4>`; capture `before_audit`, `before_bytes`.
- **When** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the response is exactly `{"mode":"hard","old_sha":"<sha-of-C4>","new_sha":"<sha-of-C4>","files_changed": 0}`.
- **And** `bytes_written == before_bytes` — `files_changed: 0` is not cosmetic, no bytes were charged.
- **And** `audit(OWNER, MOUNT)` holds at most one new entry, and if present its `detail` contains `"no-op"` and no file path.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **Verification:** exact response equality; quota equality; audit inspection; ref read.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-865 — EdgeCase — a hard reset to the current tip still discards uncommitted changes**

**Category:** EdgeCase. **Scenario:** SC-921. **Requirements:** FR-NEW-254, FR-NEW-251.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; `fs.write_text {path:"/a.txt", content:"CLOBBERED\n"}` and `fs.delete {path:"/d.txt"}` make the volume dirty in two ways.
- **When** `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the call succeeds, `old_sha == new_sha == "<sha-of-C4>"` and `files_changed > 0` (the ref is a no-op, the volume is not).
- **And** `/a.txt` reads exactly `b"a1\na3\n"` and `/d.txt` reads exactly `b"d1\n"` — both restored from `<sha-of-C4>`'s tree.
- **And** `git.status` reports `dirty == false` with an empty `changes` array.
- **And** the same sequence with `mode:"soft"` leaves the dirt in place (`/a.txt` == `b"CLOBBERED\n"`), contrasting the two modes at the same no-op ref.
- **Verification:** response fields; two byte compares; `git.status`; second-run compare.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-263 — revert of the initial commit
Existing: E2E-NEW-680 (EdgeCase, no parent).

**E2E-NEW-866 — Happy — reverting the initial commit empties the tree**

**Category:** Happy. **Scenario:** SC-922. **Requirements:** FR-NEW-263, FR-NEW-260.
- **Preconditions:** FX-INIT (only `C1`, `/a.txt` = `"a1\n"`, no parent), HEAD on `main` = `<sha-of-C1>`.
- **When** `git.revert {mount_id:"proj1", commit_sha:"<sha-of-C1>"}`.
- **Then** `status == "committed"`, `reverted_sha == "<sha-of-C1>"`, `new_sha` a 40-hex string different from it.
- **And** `VolumeClient::read_bytes("/a.txt")` errors `ERR_NOT_FOUND` and `fs.list {path:"/"}` returns an empty entry array.
- **And** `git.show {commit_sha:new_sha}` lists zero files in the resulting tree and reports exactly 1 parent, `<sha-of-C1>`.
- **And** the commit message is exactly `Revert "C1 base"`.
- **Verification:** response fields; volume read error; `fs.list`; `git.show`; message equality.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-867 — SideEffect — the reverted initial commit stays in history and stays readable**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-263, FR-NEW-255.
- **Preconditions:** as E2E-NEW-866, after the revert.
- **When** `git.log {ref_name:"main"}` is read.
- **Then** it holds exactly 2 commits, messages `["Revert \"C1 base\"", "C1 base"]`, and `commits[1]["sha"] == "<sha-of-C1>"`.
- **And** `db.object_exists("<sha-of-C1>")` is `true` and `git.show {commit_sha:"<sha-of-C1>"}` still lists `/a.txt` with content `"a1\n"`.
- **And** `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}` restores `/a.txt` to exactly `b"a1\n"`, proving the revert removed content from the working tree only, never from the object store.
- **Verification:** log array; `object_exists`; `git.show` payload; reset plus byte compare.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-265 — reverting a revert restores the original change
Existing: E2E-NEW-677 (Happy, revert of a revert reapplies).

**E2E-NEW-868 — EdgeCase — a chain of three reverts lands on the removed state**

**Category:** EdgeCase. **Scenario:** SC-922. **Requirements:** FR-NEW-265.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `C2` adds `/b.txt` = `"b1\n"`. HEAD on `main` = `<sha-of-C2>`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}` -> `R1`; `git.revert {commit_sha:R1}` -> `R2`; `git.revert {commit_sha:R2}` -> `R3`.
- **Then** after `R1`, `/b.txt` is absent; after `R2`, `/b.txt` reads exactly `b"b1\n"`; after `R3`, `/b.txt` is absent again.
- **And** the tree of `R3` is identical to the tree of `R1`: `git.diff {from_ref:R1, to_ref:R3}` returns an empty diff (no hunks).
- **And** `/a.txt` reads exactly `b"a1\n"` at every step (untouched throughout).
- **Verification:** three volume states; `git.diff` emptiness; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

**E2E-NEW-869 — SideEffect — each revert is its own commit and the original is never rewritten**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-265, FR-NEW-260.
- **Preconditions:** as E2E-NEW-868 after `R1` and `R2`.
- **When** `git.log {ref_name:"main"}` is read.
- **Then** it holds exactly 4 commits with distinct shas, messages in order `["Revert \"Revert \\\"C2 add b\\\"\"", "Revert \"C2 add b\"", "C2 add b", "C1 base"]`.
- **And** `commits[2]["sha"] == "<sha-of-C2>"` — the original commit object is the same one, not a rewritten copy.
- **And** every commit has exactly 1 parent (`parents.len() == 1`) except `C1` (`0`); a revert is a forward commit, not a history edit.
- **And** `git.show {commit_sha:"<sha-of-C2>"}` still reports it adding `/b.txt` with `"b1\n"`.
- **Verification:** log messages and shas; parent counts; `git.show`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

#### FR-NEW-278 — a paused operation survives a restart
Existing: E2E-NEW-697 (DataIntegrity, paused rebase survives a repo-store reopen).

**E2E-NEW-870 — EdgeCase — a paused merge survives the reopen with its conflict set intact**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-278, FR-NEW-275.
- **Preconditions:** SEED-MULTI(3) on `proj`; `git.merge {source_ref:"feature"}` conflicts on `/f/000.txt`, `/f/001.txt`, `/f/002.txt`.
- **Given** the `git_operations` row is read and captured field by field.
- **When** the `GitRepoStore` is dropped and rebuilt against the same config (the reopen technique of E2E-NEW-697), then `git.status {mount_id:"proj"}` is called.
- **Then** `operation.op_type == "merge"` and `remaining_conflicts == ["/f/000.txt","/f/001.txt","/f/002.txt"]`.
- **And** the re-read `git_operations` row equals the captured row field for field, `updated_at` included (a reopen is not a write).
- **And** `git.merge_resolve {resolutions:[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}` completes the merge across the restart boundary.
- **Verification:** row capture/compare; `git.status` JSON; completion call.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-871 — SideEffect — partial resolutions recorded before the restart are still applied after it**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-278, FR-NEW-178.
- **Preconditions:** as E2E-NEW-870, paused on three paths.
- **Given** `git.merge_resolve {resolutions:[{path:"/f/000.txt", content:"line-MERGED\n"}]}` returns the remaining unresolved paths `["/f/001.txt","/f/002.txt"]` and the row's `resolutions` column holds the content for `/f/000.txt`.
- **When** the store is dropped and rebuilt, then `git.merge_resolve {resolutions:[{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}`.
- **Then** `status == "merged"`.
- **And** `/f/000.txt` reads exactly `b"line-MERGED\n"` — the literal content supplied *before* the restart, not a side.
- **And** `/f/001.txt` reads `b"line-MAIN\n"` and `/f/002.txt` reads `b"line-FEATURE\n"`.
- **And** no `git_operations` row remains.
- **Verification:** three byte-exact reads; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-280 — read-only tools stay available during an operation
Existing: E2E-NEW-579 (Concurrency, `git.log` and `fs.read_text`).

**E2E-NEW-872 — Happy — `git.remote_fetch` runs during a paused merge and updates only tracking refs**

**Category:** Happy. **Scenario:** SC-929. **Requirements:** FR-NEW-280, FR-NEW-147.
- **Preconditions:** `proj` seeded like SEED-CONFLICT plus a bare remote (`seed_bare_remote`) registered as `origin`; `git.merge {source_ref:"feature"}` paused on `/src/config.toml`. `advance_bare_remote` moves the remote to `<sha-R2>`. Fetch is driven through `fetch_branch_inner` (`file://` caveat).
- **When** the fetch runs for `origin`.
- **Then** it succeeds and `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"`.
- **And** `db.get_ref("refs/heads/main").target` is unchanged (the paused merge's target branch did not move).
- **And** `/src/config.toml` is byte-identical to its paused state (line 2 `port = 8000`).
- **And** the `git_operations` row is unchanged field for field, so the fetch neither cleared nor disturbed the pause.
- **And** the merge is still completable: `git.merge_resolve {resolutions:[{path:"/src/config.toml",strategy:"ours"}]}` returns `status == "merged"`.
- **Verification:** ref reads; byte compare; row compare; completion call.
- **Cleanup:** tempdir drop. **Priority:** P0.

**E2E-NEW-873 — EdgeCase — the whole read-only set answers during a paused rebase**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-280.
- **Preconditions:** FX-FORK4 on `proj1` with one stash saved beforehand and a tag `v1` created at `<sha-of-C1>`; rebase paused at step 1.
- **When** each of `git.status`, `git.log {ref_name:"feature"}`, `git.show {commit_sha:"<sha-of-C1>"}`, `git.diff {from_ref:"<sha-of-C1>", to_ref:"<sha-of-C2>"}`, `git.branches`, `git.tags`, `git.blame {path:"/a.txt"}`, `git.stash_list`, `git.remote_list` is called while paused.
- **Then** all nine calls return `Ok` — none errors with `ERR_INVALID_ARGUMENT` about an operation in progress.
- **And** `git.branches` lists `main` and `feature` with `feature` marked current, `git.tags` lists `v1`, `git.stash_list` returns the one saved entry, `git.remote_list` returns `[]`.
- **And** immediately afterwards `git.commit {message:"nope"}` errors `ERR_INVALID_ARGUMENT` containing `"rebase"` — the contrast proves the nine successes are a deliberate allowance, not a missing guard.
- **Verification:** nine `is_ok()` assertions plus payload checks; one contrasting error assertion.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0.

---

#### FR-NEW-281 — status reports the active operation
Existing: E2E-NEW-509 (Happy, during a merge).

**E2E-NEW-874 — EdgeCase — the step counters track a multi-step rebase**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-NEW-220.
- **Preconditions:** FX-FORK4; rebase started with a 4-entry todo, paused at entry 1.
- **When** `git.status` is called, then `git.rebase_continue {resolutions:[{path:"/a.txt",strategy:"ours"}]}` pauses again, then `git.status` is called a second time.
- **Then** the first status reports `current_step == 0` and `total_steps == 4`, the second reports `current_step == 2` and `total_steps == 4` (F1 and F3 are todo indices 0 and 2, zero-based per FR-NEW-187).
- **And** both report `remaining_conflicts == ["/a.txt"]` and `continue_with == "git.rebase_continue"`, `abort_with == "git.rebase_abort"`.
- **And** the second status's `head` is still the pre-rebase `<sha-of-F4>`, because the branch ref moves only at completion.
- **Verification:** two `git.status` responses compared to exact objects; ref/field equality.
- **Cleanup:** `git.rebase_abort`. **Priority:** P1.

**E2E-NEW-875 — SideEffect — status names the cherry-pick tools for a paused cherry-pick**

**Category:** SideEffect. **Scenario:** SC-929. **Requirements:** FR-NEW-281, FR-NEW-238.
- **Preconditions:** the paused cherry-pick of E2E-NEW-858.
- **When** `git.status {mount_id:"proj1"}`.
- **Then** `operation` (the object named by FR-NEW-281) is exactly `{"op_type":"cherry_pick","source_ref":"<sha-of-F1>","current_step":null,"total_steps":null,"remaining_conflicts":["/a.txt"],"continue_with":"git.cherry_pick_continue","abort_with":"git.cherry_pick_abort"}`.
- **And** the strings `"git.merge_resolve"`, `"git.rebase_continue"` appear nowhere in the serialized status response.
- **When** `git.cherry_pick_abort {mount_id:"proj1"}` then `git.status` again.
- **Then** the response has no `operation` (the object named by FR-NEW-281) key and `head == "<sha-of-C2>"`.
- **Verification:** exact JSON equality; substring absence; post-abort key absence.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-282 — an operation is not owned by one person
Existing: E2E-NEW-580 (Concurrency, `git.merge_abort` by another member).

**E2E-NEW-876 — Happy — a second member continues a rebase the first member started**

**Category:** Happy. **Scenario:** SC-929. **Requirements:** FR-NEW-282, FR-NEW-221.
- **Preconditions:** FX-FORK on `proj1` with `owner@test.com` and `second@test.com` both members (`admin.add_member`); `owner@test.com` starts `git.rebase {onto:"main", todo:[pick F1, pick F2]}`, paused on `/a.txt`.
- **When** `Env::as_person("second@test.com")` calls `git.rebase_continue {resolutions:[{path:"/a.txt", strategy:"theirs"}]}`.
- **Then** `status == "completed"` and `replayed == 2`.
- **And** `/a.txt` reads exactly `b"a1\nFEAT1\n"`.
- **And** the replayed commits' committer email is `"second@test.com"` while the author email stays `"owner@test.com"` — the continuation is attributed to whoever finished it, the authorship to whoever wrote it.
- **And** `audit("second@test.com", MOUNT)` holds an entry whose `op` is `"git.rebase_continue"`, and `audit(OWNER, MOUNT)` holds the earlier `"git.rebase"` entry.
- **And** no `git_operations` row remains.
- **Verification:** response; byte read; `git.log` author/committer fields; two audit reads; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-877 — Failure — a non-member still cannot touch another project's operation**

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

#### FR-NEW-308 — a failed enrichment sub-call is surfaced, not degraded
Existing: E2E-NEW-741 (Failure, github check-runs sub-call 403).

**E2E-NEW-878 — Failure — a GitLab approvals sub-call 500 fails `pr_get`**

**Category:** Failure. **Scenario:** SC-927. **Requirements:** FR-NEW-308, FR-NEW-307.
- **Preconditions:** Band D, mount `acme-lab`, person `dev@test.com`. Mock routes: `GET /projects/acme%2Fapi/merge_requests/42` -> `200` with a complete MR payload whose `head_pipeline.status` is `"success"`; `GET /projects/acme%2Fapi/merge_requests/42/approvals` -> `500` body `{"message":"500 Internal Server Error"}`.
- **When** `git.pr_get {mount_id:"acme-lab", pr_number:42}`.
- **Then** assert error whose message contains `"500"` and `"approvals"`; the call is `Err`, not an `Ok` carrying `review_state: "none"`.
- **And** the response body of the failed call is not returned as a partial model: no `Ok` value is produced at all.
- **And** `mock.calls()` holds exactly the two requests, in order, so the failure was surfaced at the first failing sub-call rather than after retries.
- **And** the error message contains neither `"glpat"` nor the token value.
- **Verification:** error assertion; `is_err()`; recorded call list; substring absence.
- **Cleanup:** fixture drop. **Priority:** P0.

**E2E-NEW-879 — EdgeCase — a genuinely empty check set reports `none` and succeeds**

**Category:** EdgeCase. **Scenario:** SC-927. **Requirements:** FR-NEW-308, FR-NEW-307.
- **Preconditions:** Band D, mount `acme-api`. Mock routes: `GET /repos/acme/api/pulls/42` -> `200`; `GET /repos/acme/api/pulls/42/reviews` -> `200` body `[]`; `GET /repos/acme/api/commits/<head_sha>/check-runs` -> `200` body `{"total_count":0,"check_runs":[]}`.
- **When** `git.pr_get {mount_id:"acme-api", pr_number:42}`.
- **Then** the call succeeds with `checks_state == "none"` and `review_state == "none"`.
- **And** all three requests were issued (`mock.calls().len() == 3`), so `none` here is an observed empty set, not a swallowed error — the distinction FR-NEW-308 exists to protect.
- **And** re-running the same call with the check-runs route replaced by a `500` makes it `Err`, asserted in the same test, so the two paths are contrasted directly.
- **Verification:** success response fields; call count; contrasting error assertion.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### FR-NEW-331 — requested scopes are configurable
Existing: E2E-NEW-774 (Happy, github device flow scope string).

**E2E-NEW-880 — EdgeCase — a configured `git.gitlab_scope` overrides the default**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-331, FR-NEW-330.
- **Preconditions:** Band D fixture built with `git.gitlab_scope: "api"` and `git.github_scope: "repo public_repo"`; `MockDeviceFlowClient` recording the authorization request.
- **When** `git.auth {mount_id:"acme-lab"}` runs the GitLab flow, and `git.auth {mount_id:"acme-api"}` runs the GitHub flow.
- **Then** the recorded GitLab request's `scope` is exactly `"api"` (not the three-scope default) and the GitHub request's `scope` is exactly `"repo public_repo"`.
- **And** the stored token for `(dev@test.com, gitlab.com)` has `scopes == ["api"]`.
- **And** `git.pr_list {mount_id:"acme-lab"}` passes the scope pre-check (`mock.calls().len() == 1`), because `api` alone satisfies the PR surface's requirement.
- **Verification:** two recorded request fields; token store read; mock call count.
- **Cleanup:** fixture drop. **Priority:** P1.

**E2E-NEW-881 — EdgeCase — with no configuration the defaults are exactly the documented strings**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-331, FR-MOD-109.
- **Preconditions:** Band D fixture with neither `git.github_scope` nor `git.gitlab_scope` set in the YAML.
- **When** both device flows are run.
- **Then** the recorded GitHub `scope` is exactly `"repo"` and the recorded GitLab `scope` is exactly `"api read_repository write_repository"` — character for character, including the ordering.
- **And** `ServerConfig` parsed from that YAML reports `git.github_scope == "repo"` and `git.gitlab_scope == "api read_repository write_repository"` (the defaults live in the config type, so a caller reading config sees the same values the flow sends).
- **And** the config round-trips: serializing and re-parsing it yields the same two strings.
- **Verification:** recorded request fields; config struct fields; serialize/parse round-trip.
- **Cleanup:** fixture drop. **Priority:** P1.

---

### Z.3 Gap 3 — requirements that had exactly two tests

**E2E-NEW-890 — SideEffect — pushing to `upstream` leaves `origin`'s tracking ref alone**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-MOD-101.
Existing angles: 486 (Failure, unknown remote), 487 (EdgeCase, omitted remote defaults to origin). This one is the cross-remote side effect.
- **Preconditions:** SEED-R2 (`origin` -> `<url>` at `<sha-R1>`, `upstream` -> `<url2>` at `<sha-U1>`), both fetched so `refs/remotes/origin/main` = `<sha-R1>` and `refs/remotes/upstream/main` = `<sha-U1>`. Drives `push_branch_inner` with `remote: "upstream"`, `force:true`, `expected_remote_sha:"<sha-U1>"`.
- **When** the push runs.
- **Then** the bare repo at `<url2>` has `refs/heads/main` == `<sha-C2>` and the bare repo at `<url>` still has `refs/heads/main` == `<sha-R1>`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-C2>"` and `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`.
- **And** the audit entry's `detail` contains `"upstream"` and not `"origin"`.
- **Verification:** two bare-repo ref reads; two local tracking-ref reads; audit detail.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

**E2E-NEW-891 — Failure — an invalid `remote_branch` name is refused before contacting the remote**

**Category:** Failure. **Scenario:** SC-908. **Requirements:** FR-MOD-102, FR-NEW-104.
Existing angles: 484 (Happy), 485 (SideEffect). This one is the validation angle.
- **Preconditions:** SEED-R. Drives the registered `git.remote_push` tool (a validation test).
- **When** `git.remote_push {"mount_id":"gitproj","branch":"main","remote_branch":"bad..name"}`, and again with `"has space"`, `"trailing/"`, `"tip.lock"`.
- **Then** every call asserts error `ERR_INVALID_ARGUMENT` containing `"is not a valid branch name"` — the remote-side name is validated by the same rule as a local one.
- **And** the bare remote's ref list is element-for-element equal to the list captured before the calls (no `refs/heads/bad..name` was attempted).
- **And** no audit entry with `op == "git.remote_push"` exists.
- **Verification:** four error assertions; bare-repo `references()` list compare; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-892 — EdgeCase — `git.remote_pull {remote:"upstream"}` pulls upstream and leaves origin untouched**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-MOD-103.
Existing angles: 481 (Happy, fetch from upstream), 483 (Failure, unknown remote). This one covers `remote_pull`, which neither touches.
- **Preconditions:** SEED-R2; local `main` = `<sha-C2>` is an ancestor of `upstream`'s tip `<sha-U2>` (fast-forwardable); `origin` is at `<sha-R1>`, unrelated. Drives `pull_branch` directly.
- **When** the pull runs with `remote: "upstream"`, `branch: "main"`.
- **Then** `status == "fast_forward"` (or `"merged"` if divergent seeds are used) and `db.get_ref("refs/heads/main").target == "<sha-U2>"`.
- **And** `db.get_ref("refs/remotes/upstream/main").target == "<sha-U2>"` while `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`, unchanged.
- **And** the volume holds `upstream`'s file `/up.txt` with exactly its committed bytes, and none of `origin`'s files.
- **Verification:** status; three ref reads; byte-exact volume reads.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

**E2E-NEW-893 — EdgeCase — a full ref path as `start_point`**

**Category:** EdgeCase. **Scenario:** SC-901. **Requirements:** FR-NEW-103, FR-NEW-101.
Existing angles: 403 (unknown short name), 410 (repo with no commits). This one covers fully-qualified ref paths.
- **Preconditions:** SEED-B (`main` = `<sha-C2>`, `release/1.0` = `<sha-C3>`).
- **When** `git.branch_create {"name":"from-full","start_point":"refs/heads/release/1.0"}`.
- **Then** the call succeeds with `sha == "<sha-C3>"` and `db.get_ref("refs/heads/from-full").target == "<sha-C3>"`.
- **When** `git.branch_create {"name":"from-ghost","start_point":"refs/heads/ghost"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/heads/ghost"`.
- **When** `git.branch_create {"name":"from-remote","start_point":"refs/remotes/origin/main"}` with no such tracking ref.
- **Then** assert error `ERR_NOT_FOUND` containing `"refs/remotes/origin/main"`, and `db.list_refs()` contains no `refs/heads/from-remote`.
- **Verification:** success response and ref read; two error assertions; ref list.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-894 — SideEffect — `branch_reset` audits both shas so the move is recoverable**

**Category:** SideEffect. **Scenario:** SC-924. **Requirements:** FR-NEW-111, FR-NEW-255.
Existing angles: 430 and 434 are both Happy paths. This one is the audit/recovery record.
- **Preconditions:** SEED-B; `release/1.0` = `<sha-C3>`, not checked out.
- **When** `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C1>","force":true}`.
- **Then** the response is exactly `{"branch":"release/1.0","old_sha":"<sha-C3>","new_sha":"<sha-C1>","files_changed": 0}`.
- **And** exactly one new audit entry has `op == "git.branch_reset"` and a `detail` containing both `"<sha-C3>"` and `"<sha-C1>"`.
- **And** `db.object_exists("<sha-C3>")` is `true`.
- **And** replaying the recovery from the audit line alone works: `git.branch_reset {"name":"release/1.0","target_commit":"<sha-C3>","force":true}` restores the ref to `<sha-C3>`.
- **Verification:** exact response; audit entry `op` and `detail`; `object_exists`; recovery call plus ref read.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-895 — EdgeCase — dirt that is only an added file is still stashable**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-121, FR-NEW-120.
Existing angles: 449 and 450 are both refusals. This one is the boundary on the other side.
- **Preconditions:** SEED-A, clean volume; `fs.write_text {path:"/scratch/new.txt", content:"n\n"}` (a file in neither the HEAD tree nor any commit).
- **When** `git.stash_save {"message":"wip added only"}`.
- **Then** the call succeeds with `files_stashed == 1` and `base_sha == "<sha-C2>"`.
- **And** `VolumeClient::read_bytes("/scratch/new.txt")` errors `ERR_NOT_FOUND` — the volume was rewritten to HEAD's tree, which does not contain it.
- **And** `git.stash_apply {"stash_id":"<stash-1>"}` restores it to exactly `b"n\n"`.
- **And** a second `git.stash_save` immediately after the first (volume now clean) errors `ERR_INVALID_ARGUMENT` containing `"nothing to stash"`, contrasting the two states.
- **Verification:** success response; volume read error; byte-exact restore; contrasting error.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-896 — Failure — a stash ref cannot be checked out**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-123.
Existing angles: 448 (row exists), 470 (absent from `git.branches`). This one is the `branch_switch` half of the requirement.
- **Preconditions:** SEED-A plus one stash `<stash-1>` (`refs/stash/<stash-1>` exists).
- **When** `git.branch_switch {"mount_id":"gitproj","name":"<stash-1>"}`.
- **Then** assert error `ERR_NOT_FOUND` containing `"<stash-1>"`.
- **When** `git.branch_switch {"name":"refs/stash/<stash-1>"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` or `ERR_NOT_FOUND` naming the value; in either case `db.get_ref("HEAD")` is still `{target:"refs/heads/main", symbolic:true}`.
- **And** `git.branch_delete {"name":"<stash-1>"}` also errors, and `db.get_ref("refs/stash/<stash-1>")` still exists afterwards — the stash namespace is not reachable through branch tools at all.
- **Verification:** three error assertions; HEAD read; stash ref read.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-897 — EdgeCase — one stash applied onto two branches in turn**

**Category:** EdgeCase. **Scenario:** SC-902. **Requirements:** FR-NEW-124, FR-NEW-129.
Existing angles: 455 (Happy), 466 (quota failure). This one exercises retention across repeated applies.
- **Preconditions:** SEED-B; on `main`, write `/README.md` = `"alpha-wip\n"`, `git.stash_save {"message":"wip"}` -> `<stash-1>`.
- **When** `git.stash_apply {"stash_id":"<stash-1>"}` on `main`, then `git.commit {message:"take wip"}`, then `git.branch_switch {"name":"release/1.0"}`, then `git.stash_apply {"stash_id":"<stash-1>"}` again.
- **Then** both applies return `status == "applied"`.
- **And** after the second apply, `/README.md` reads exactly `b"alpha-wip\n"` on `release/1.0` too.
- **And** `git.stash_list` still returns exactly one entry, `<stash-1>`, after both applies — apply retains, always.
- **And** `db.get_ref("refs/stash/<stash-1>")` is unchanged in target across both applies.
- **Verification:** two response statuses; byte-exact read; stash list length; ref equality.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-898 — Failure — a quota-exhausted `stash_pop` keeps the entry**

**Category:** Failure. **Scenario:** SC-902. **Requirements:** FR-NEW-125, FR-NEW-185.
Existing angles: 456 (Happy), 457 (SideEffect, entry gone after a clean pop). This one proves the delete-after-success ordering under a failure the happy path never hits.
- **Preconditions:** `Env::with_quota` sized so that the pop's write exceeds it; SEED-A; one stash `<stash-1>` whose application writes `/README.md` = `"alpha-wip\n"`.
- **When** `git.stash_pop {"stash_id":"<stash-1>"}`.
- **Then** assert error `ERR_WRITE_QUOTA_EXCEEDED`.
- **And** `git.stash_list` still returns exactly one entry, `<stash-1>` with its original `message` and `base_sha` — the entry is deleted only once the application has fully succeeded.
- **And** `/README.md` reads exactly `b"alpha\n"` (nothing partially applied).
- **And** after raising the quota, `git.stash_pop {"stash_id":"<stash-1>"}` succeeds and `git.stash_list` returns `[]`.
- **Verification:** error code; stash list contents; byte compare; recovery run.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-899 — DataIntegrity — a stash applies after its base commit became unreachable**

**Category:** DataIntegrity. **Scenario:** SC-930. **Requirements:** FR-NEW-129, FR-NEW-255.
Existing angles: 464 (branch deleted), 465 (applied on a different branch). This one removes the *commit* from every ref's reachability, not just the branch name.
- **Preconditions:** SEED-B; `git.branch_switch {"name":"release/1.0"}`; write `/docs/rel.md` = `"rel-wip\n"`; `git.stash_save {"message":"wip rel"}` -> `<stash-1>` with `base_sha == "<sha-C3>"`.
- **When** `git.branch_switch {"name":"main"}`, `git.branch_delete {"name":"release/1.0","force":true}` (so `<sha-C3>` is reachable from no ref), then `git.stash_apply {"stash_id":"<stash-1>"}` on `main`.
- **Then** the apply returns `status == "applied"` (or the conflict response if the bytes clash; with `/docs/rel.md` absent on `main` it applies cleanly).
- **And** `/docs/rel.md` reads exactly `b"rel-wip\n"`.
- **And** `db.object_exists("<sha-C3>")` is still `true` — the stash's `base_sha` object survived the branch delete, which is precisely why the diff could be computed.
- **And** `git.stash_list` still lists `<stash-1>` with `base_sha == "<sha-C3>"`.
- **Verification:** apply response; byte-exact read; `object_exists`; stash list.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-900 — EdgeCase — re-adding a remote with the identical URL is still a duplicate**

**Category:** EdgeCase. **Scenario:** SC-907. **Requirements:** FR-NEW-141.
Existing angles: 472 (different URL), 475 (no new row). This one removes the "it is idempotent, so allow it" escape.
- **Preconditions:** SEED-R (`origin` -> `<url>`).
- **When** `git.remote_add {"mount_id":"gitproj","name":"origin","url":"<url>"}` — byte-identical name and URL.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"origin"` and `"<url>"` (the error names the existing remote and its URL).
- **And** `db.list_remotes()` still holds exactly one row.
- **And** `state.safety.audit(OWNER, MOUNT)` holds no new `git.remote_add` entry.
- **And** the same call with the URL differing only by a trailing `.git` also errors, so near-identity is not special-cased.
- **Verification:** two error assertions; `list_remotes` length; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

**E2E-NEW-901 — SideEffect — `remote_remove` deletes only that remote's tracking refs**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-142.
Existing angles: 477 (Happy), 479 (EdgeCase, push after removing origin). This one asserts the ref sweep is scoped.
- **Preconditions:** SEED-R2 with both remotes fetched and two branches each, so `db.list_refs()` contains `refs/remotes/origin/main`, `refs/remotes/origin/dev`, `refs/remotes/upstream/main`, `refs/remotes/upstream/dev`.
- **When** `git.remote_remove {"name":"upstream"}`.
- **Then** the call succeeds and `db.list_remotes()` holds exactly `[{name:"origin", url:"<url>"}]`.
- **And** `db.list_refs()` contains `refs/remotes/origin/main` and `refs/remotes/origin/dev` with their original targets, and contains no ref whose name starts with `refs/remotes/upstream/`.
- **And** `refs/heads/main` and `HEAD` are unchanged, and the volume bytes are unchanged (a remote removal is not a working-tree operation).
- **And** `db.object_exists("<sha-U1>")` is still `true` — the fetched objects are not pruned by the removal.
- **Verification:** `list_remotes`; ref list prefix filtering; ref reads; byte compare; `object_exists`.
- **Cleanup:** tempdir drops. **Priority:** P0.

---

**E2E-NEW-902 — EdgeCase — `remote_list` resolves host and provider for an enterprise host**

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

**E2E-NEW-903 — Security — remote tools stay silent about a stored token**

**Category:** Security. **Scenario:** SC-907. **Requirements:** FR-NEW-145.
Existing angles: 474 (userinfo in the URL), 488 (non-member). This one covers a token that genuinely exists in the store.
- **Preconditions:** fixture with `git.hosts` mapping `github.com: github`; `OAuthTokenStore::store_token` seeds `(owner@test.com, github.com)` with token `ghp_SECRET_VALUE_zzz`; remote `origin` -> `https://github.com/acme/api.git` added.
- **When** `git.remote_list {"mount_id":"gitproj"}` and `git.remote_add {"name":"origin","url":"https://github.com/acme/api.git"}` (the second fails as a duplicate).
- **Then** neither the serialized success response nor the error message contains `"ghp_"` or `"ghp_SECRET_VALUE_zzz"`.
- **And** `git.remote_add {"name":"bad","url":"https://github.com/acme/../../x"}` errors, and that message likewise contains neither.
- **And** the tracing spans captured for the three calls (`tracing_subscriber::fmt::TestWriter` capture) contain neither string.
- **Verification:** substring absence over serialized responses, error strings and captured log output.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-904 — SideEffect — a fetch leaves local branches and the volume byte-identical**

**Category:** SideEffect. **Scenario:** SC-907. **Requirements:** FR-NEW-147.
Existing angles: 481 (Happy), 482 (other remote's refs untouched). This one asserts the local side is untouched.
- **Preconditions:** SEED-R with the remote advanced to `<sha-R2>` carrying `/hello.txt` = `"remote-2\n"`; local `main` = `<sha-C2>` with `/README.md` = `"alpha\n"`, `/src/lib.rs` = `"fn a() {}\n"`. Drives `fetch_branch_inner`.
- **Given** `before_refs = db.list_refs()` and the byte content of every volume file.
- **When** the fetch runs for `origin`.
- **Then** `db.get_ref("refs/remotes/origin/main").target == "<sha-R2>"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` and `db.get_ref("HEAD")` unchanged; the only difference between `before_refs` and the new `list_refs()` is the one `refs/remotes/origin/*` entry.
- **And** `/README.md` and `/src/lib.rs` are byte-identical to their captured content, and `/hello.txt` does **not** exist in the volume (`read_bytes` errors `ERR_NOT_FOUND`) even though its object was fetched.
- **And** `bytes_written(OWNER, MOUNT)` is unchanged (a fetch writes objects, not volume bytes, and charges no file quota).
- **Verification:** ref reads and list diff; byte compares; read error; quota equality.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-905 — SideEffect — a forced push advances the local tracking ref to the new sha**

**Category:** SideEffect. **Scenario:** SC-909. **Requirements:** FR-NEW-155.
Existing angles: 490 (Happy), 495 (EdgeCase, absent branch). This one asserts the local tracking ref follows.
- **Preconditions:** SEED-R at `<sha-R1>`, fetched so `refs/remotes/origin/main` = `<sha-R1>`; local `main` = `<sha-C2>` diverged from it. Drives `push_branch_inner` with `force:true`, `expected_remote_sha:"<sha-R1>"`.
- **When** the push runs.
- **Then** the response has `forced == true`, `overwritten_sha == "<sha-R1>"`, `new_sha == "<sha-C2>"`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-C2>"` — the local view of the remote matches what was just written, so a subsequent lease read is correct without a fetch.
- **And** the bare remote's `refs/heads/main` == `<sha-C2>`.
- **And** an immediately following force push with `expected_remote_sha:"<sha-C2>"` and the same tip succeeds as a no-change update, proving the tracking ref value is usable as the next lease.
- **Verification:** response fields; local tracking ref read; bare-repo ref read; follow-up push.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-906 — EdgeCase — a whitespace-only lease with `force:true` is refused before the network**

**Category:** EdgeCase. **Scenario:** SC-909. **Requirements:** FR-NEW-156.
Existing angles: 489 (absent), 496 (malformed). This one covers whitespace, the value that trims to empty.
- **Preconditions:** SEED-R, remote at `<sha-R1>`, local diverged. Drives the registered `git.remote_push` tool.
- **When** `git.remote_push {"branch":"main","force":true,"expected_remote_sha":"   "}`, and again with `"\t\n"`.
- **Then** both assert error `ERR_INVALID_ARGUMENT` containing `"expected_remote_sha"` — a whitespace lease is an absent lease, not a lease that fails to match.
- **And** the message does **not** contain `"<sha-R1>"`: this is the missing-lease error (FR-NEW-156), not the stale-lease error (FR-NEW-157), and the two are distinguishable by the caller.
- **And** the bare remote's `refs/heads/main` == `<sha-R1>` and no `git.remote_push` audit entry exists.
- **Verification:** two error assertions; substring absence; bare-repo ref read; audit scan.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

**E2E-NEW-907 — EdgeCase — one mixed resolution call spanning a text, a binary and a delete/modify path**

**Category:** EdgeCase. **Scenario:** SC-905. **Requirements:** FR-NEW-176, FR-NEW-180, FR-NEW-182.
Existing angles: 505 (three text files), 521 (Failure, strategy+content together). This one mixes conflict *kinds* in one call.
- **Preconditions:** a composite seed on `proj`: `/src/config.toml` (text, both sides edit line 2), `/assets/logo.png` (binary, last byte `0x02` vs `0x03`), `/lib/util.rs` (feature deletes, main modifies). `git.merge {source_ref:"feature"}` returns three conflict entries.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"[server]\nport = 8500\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}, {path:"/assets/logo.png", strategy:"theirs"}, {path:"/lib/util.rs", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `files_changed == 3`.
- **And** `/src/config.toml` reads exactly the supplied string's bytes (line 2 `port = 8500`, a value on neither side).
- **And** `/assets/logo.png` reads exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x02]`.
- **And** `/lib/util.rs` reads exactly `b"pub fn a() { println!(\"x\"); }\n"`.
- **And** no `git_operations` row remains and the merge commit has 2 parents.
- **Verification:** three byte-exact reads; response fields; relational count; `parent_count()`.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-908 — Failure — abort after a partial resolve discards the recorded resolutions**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-178, FR-NEW-197.
Existing angles: 555 (SideEffect, shrunk set), 576 (EdgeCase, two sequential calls complete it). This one covers the abort path out of a partial state.
- **Preconditions:** SEED-MULTI(3) on `proj`; merge conflicts on three paths; `git.merge_resolve {resolutions:[{path:"/f/000.txt", content:"line-X\n"}]}` records one resolution and returns the two remaining.
- **When** `git.merge_abort {mount_id:"proj"}`.
- **Then** the call succeeds and no `git_operations` row remains.
- **And** `/f/000.txt` reads exactly `b"line-MAIN\n"` — the pre-merge main-side bytes, not `line-X\n`; recorded resolutions are state of the operation, not of the volume.
- **And** `/f/001.txt` and `/f/002.txt` also read `b"line-MAIN\n"`.
- **And** a fresh `git.merge {source_ref:"feature"}` conflicts again on all **three** paths, with no memory of the earlier partial resolution.
- **And** `git.merge_resolve {resolutions:[{path:"/f/001.txt",strategy:"ours"}]}` before the fresh merge (that is, right after the abort) errors `ERR_INVALID_ARGUMENT` containing `"no merge is in progress"`.
- **Verification:** relational count; three byte reads; re-merge conflict array; error assertion.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-909 — Failure — literal content is refused for a type-change conflict**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-183, FR-NEW-175.
Existing angles: 566 (the conflict shape), 570 (path colliding with a directory). This one covers the resolution restriction the requirement states.
- **Preconditions:** SEED-TYPECHANGE (`/mod` a file on main = `"placeholder v2\n"`, a directory containing `/mod/inner.rs` on feature); merge paused with `conflicts[0].type_change == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/mod", content:"whatever\n"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"type_change"` (or `"type change"`), `"ours"` and `"theirs"`.
- **And** `/mod` still reads exactly `b"placeholder v2\n"` and `/mod/inner.rs` does not exist.
- **And** the `git_operations` row still lists `/mod` unresolved.
- **And** `git.merge_resolve {resolutions:[{path:"/mod", strategy:"theirs"}]}` then succeeds: `/mod/inner.rs` reads exactly `b"pub const N: u8 = 1;\n"` and `read_bytes("/mod")` errors (it is a directory now).
- **Verification:** error code + substrings; volume reads; relational row; recovery call and post-state reads.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-910 — DataIntegrity — the ref advances only after the last file write lands**

**Category:** DataIntegrity. **Scenario:** SC-904. **Requirements:** FR-NEW-184.
Existing angles: 419 (volume unchanged after a failed switch), 561 (volume unchanged after a mid-apply failure). Neither asserts the *ref* ordering, which is what this one pins.
- **Preconditions:** SEED-MULTI(3) on `proj`, merge paused; a fault injected into `VolumeClient` so the write of the **third** file (`/f/002.txt`) fails with an I/O error (the same injection technique E2E-NEW-561 uses); capture `<sha-PRE>` = `refs/heads/main` before the resolve.
- **When** `git.merge_resolve {resolutions:[{path:"/f/000.txt",strategy:"ours"},{path:"/f/001.txt",strategy:"ours"},{path:"/f/002.txt",strategy:"theirs"}]}`.
- **Then** the call asserts error `ERR_INTERNAL_ERROR`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-PRE>"` — the ref never advanced, so no commit claims files that are not on disk.
- **And** all three files read their pre-resolve bytes `b"line-MAIN\n"` (the two successful writes were rolled back with the third).
- **And** the `git_operations` row is retained with its three paths, so a retry after clearing the fault completes the merge and *then* moves the ref to a new sha.
- **Verification:** error code; ref equality; three byte compares; relational row; retry with ref change.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-911 — EdgeCase — a 250-path conflict set resolved in one `merge_resolve` call**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-196, FR-NEW-184.
Existing angles: 503 and 504 both resolve a single path. This one covers bulk.
- **Preconditions:** SEED-MULTI(250) on `proj`; `git.merge {source_ref:"feature"}` returns `conflicts.len() == 250`.
- **When** `git.merge_resolve` is called once with 250 resolutions: `strategy:"ours"` for the 125 even-indexed paths, `strategy:"theirs"` for the 125 odd-indexed paths.
- **Then** `status == "merged"` and `files_changed == 250`.
- **And** `/f/000.txt` reads `b"line-MAIN\n"`, `/f/001.txt` reads `b"line-FEATURE\n"`, and the same alternation holds for a sampled `/f/248.txt` and `/f/249.txt`, checked byte-exactly.
- **And** `bytes_written(OWNER, MOUNT)` increased by exactly the sum of the 250 resolved file sizes.
- **And** the merge commit has exactly 2 parents and no `git_operations` row remains.
- **Verification:** response fields; four byte reads; quota arithmetic; `parent_count()`; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-912 — EdgeCase — `merge_resolve` refuses to advance a rebase and says which tool would**

**Category:** EdgeCase. **Scenario:** SC-929. **Requirements:** FR-NEW-198, FR-NEW-225.
Existing angles: 519 and 520 both cover "nothing in progress". This one covers "something else in progress".
- **Preconditions:** FX-FORK on `proj1`; rebase paused on `/a.txt` with `op_type == "rebase"`.
- **When** `git.merge_resolve {mount_id:"proj1", resolutions:[{path:"/a.txt", strategy:"ours"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` whose message contains `"rebase"`, `"git.rebase_continue"` and `"git.rebase_abort"` — it does not merely say "no merge in progress", it names what *is* in progress.
- **When** `git.merge_abort {mount_id:"proj1"}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"rebase"` and `"git.rebase_abort"`; the rebase is still paused (`git_operations` row unchanged, `current_step == 1`).
- **And** `git.rebase_abort` then succeeds and the row is gone.
- **Verification:** two error assertions with three substrings each; relational row read; abort success.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-913 — SideEffect — `pick` preserves author identity and timestamp, and sets the committer to the caller**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-211.
Existing angles: 600 and 606 are both Happy replays. This one pins the identity rule FR-NEW-211 states.
- **Preconditions:** FX-FORK-CLEAN where F1 and F2 were committed by `author@test.com` (a member), with author timestamps `T1` and `T2` captured from `git.log` before the rebase; `second@test.com` is also a member and performs the rebase.
- **When** `Env::as_person("second@test.com")` calls `git.rebase {onto:"main", todo:[pick F1, pick F2]}`.
- **Then** the replayed commits' `author_email` are both `"author@test.com"` and their author timestamps are exactly `T1` and `T2`, unchanged.
- **And** their `committer_email` are both `"second@test.com"` and their committer timestamps are >= the test's start time (new commits).
- **And** the replayed commit messages are exactly `"F1 feature edit"` and `"F2 feature add"`.
- **And** their shas differ from `<sha-of-F1>` and `<sha-of-F2>` (the committer change alone guarantees a new sha).
- **Verification:** `git.log` author/committer fields and timestamps; message equality; sha inequality.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-914 — EdgeCase — dropping a commit a later entry depends on pauses instead of silently succeeding**

**Category:** EdgeCase. **Scenario:** SC-915. **Requirements:** FR-NEW-213, FR-NEW-219.
Existing angles: 602 (drop a leaf), 605 (drop everything). This one covers a dependent drop.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `main` = C1 + C2 (adds `/m.txt`); `feature` = C1 + D1 (adds `/dep.txt` = `"v1\n"`) + D2 (rewrites `/dep.txt` to `"v2\n"`). HEAD on `feature`.
- **When** `git.rebase {onto:"main", todo:[{action:"drop", sha:"<sha-of-D1>"},{action:"pick", sha:"<sha-of-D2>"}]}`.
- **Then** the result is `status == "conflict"` with one entry `path == "/dep.txt"`, `ours.exists == false` (the file does not exist on the new base after the drop), `theirs.exists == true` with content `"v2\n"`, `base.exists == true` with content `"v1\n"` — the dependency loss is surfaced, not silently resolved.
- **And** the `git_operations` row has `op_type == "rebase"`, `current_step == 1`, `total_steps == 2` (the last of two entries is zero-based index 1).
- **When** `git.rebase_continue {resolutions:[{path:"/dep.txt", strategy:"theirs"}]}`.
- **Then** `status == "completed"` and `/dep.txt` reads exactly `b"v2\n"`, with `git.log {ref_name:"feature"}` holding 3 commits (D1 dropped).
- **Verification:** conflict entry fields; relational row; completion response; byte read; log length.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-915 — SideEffect — an `up_to_date` rebase writes nothing at all**

**Category:** SideEffect. **Scenario:** SC-914. **Requirements:** FR-NEW-217.
Existing angles: 607 and 608 are both EdgeCase no-op assertions on the response. This one asserts the absence of side effects.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; capture `before_audit`, `before_bytes`, `before_objects = COUNT(*) FROM git_objects WHERE volume_id='proj1'`.
- **When** `git.rebase {onto:"<sha-of-C3>", todo:[{action:"pick", sha:"<sha-of-C4>"}]}` where `C3` is already `C4`'s parent, so nothing needs replaying.
- **Then** `status == "up_to_date"`, `replayed == 0`, `dropped == 0`, `squashed == 0`, no `operation_id` key.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"` (the same sha, not a rewritten equivalent).
- **And** `COUNT(*) FROM git_objects` equals `before_objects` — no new commit or tree object was created and thrown away.
- **And** `audit(OWNER, MOUNT)` equals `before_audit` element-for-element, `bytes_written` equals `before_bytes`, and no `git_operations` row exists.
- **Verification:** response fields; ref equality; relational counts; audit/quota equality.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-916 — EdgeCase — aborting after one clean step removes the replayed commit from the branch but keeps its object**

**Category:** EdgeCase. **Scenario:** SC-917. **Requirements:** FR-NEW-223, FR-NEW-255.
Existing angles: 619 (abort after an immediate conflict), 621 (abort mid-multi-pause). This one pins what happens to the *already replayed* commits' objects.
- **Preconditions:** **FX-TAIL** (as E2E-NEW-853): `feature` = C1 + T1 (clean) + T2 (conflicts). Rebase started, T1 replayed as `<sha-T1'>`, paused at entry 2.
- **Given** `<sha-T1'>` is read from `git.log` of the replay tip while paused, and `db.object_exists("<sha-T1'>")` is `true`.
- **When** `git.rebase_abort {mount_id:"proj1"}`.
- **Then** `db.get_ref("refs/heads/feature").target == "<sha-of-T2>"` (the exact pre-rebase tip).
- **And** `git.log {ref_name:"feature"}` contains neither `<sha-T1'>` nor any commit whose parent is `<sha-of-C2>`.
- **And** `/a.txt` reads exactly `b"a1\nFEAT\n"` and `/t1.txt` reads exactly `b"t1\n"` — the pre-rebase volume bytes, restored exactly.
- **And** `db.object_exists("<sha-T1'>")` is still `true`: abort unreaches, it does not prune (consistent with FR-NEW-255 for reset).
- **And** no `git_operations` row remains.
- **Verification:** ref read; log sha scan; two byte compares; `object_exists`; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-917 — Concurrency — `git.branch_create` cannot interleave into a running rebase**

**Category:** Concurrency. **Scenario:** SC-916. **Requirements:** FR-NEW-226, FR-NEW-115.
Existing angles: 694 (rebase vs `git.commit`), 699 (ref consistency sweep). This one covers a branch tool against the held lock.
- **Preconditions:** FX-FORK-CLEAN with 50 replayable commits on `feature` so the replay takes measurable time; a barrier that releases both futures simultaneously (the technique of E2E-NEW-694).
- **When** `git.rebase {onto:"main", todo:[pick S1 .. pick S50]}` and `git.branch_create {name:"race/x", start_point:"feature"}` are driven concurrently with `tokio::join!`.
- **Then** both futures complete; neither panics; neither returns `ERR_INTERNAL_ERROR`.
- **And** `refs/heads/race/x` points at **either** `<sha-of-S50>` (created before the rebase took the lock) **or** the rebase's final tip — never at an intermediate replayed sha. Asserted as membership in that two-element set, with the intermediate shas enumerated from `git.log` and asserted absent.
- **And** `git.log {ref_name:"feature"}` holds exactly 52 commits with no duplicate sha.
- **And** no `git_operations` row remains.
- **Verification:** `tokio::join!` results; ref value set membership; log length and sha uniqueness; relational count.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-918 — SideEffect — an `already_present` cherry-pick changes nothing observable**

**Category:** SideEffect. **Scenario:** SC-918. **Requirements:** FR-NEW-236.
Existing angles: 648 and 649 both assert the reported status. This one asserts the absence of side effects.
- **Preconditions:** FX-LINE, HEAD on `main` = `<sha-of-C4>`; `<sha-of-C2>` is an ancestor. Capture `before_audit`, `before_bytes`, `before_objects`, `before_log = git.log {ref_name:"main"}`.
- **When** `git.cherry_pick {commit_sha:"<sha-of-C2>"}`.
- **Then** the call is `Ok` with `status == "already_present"` and `new_sha` is `null`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C4>"` and `git.log {ref_name:"main"}` equals `before_log` element-for-element.
- **And** `COUNT(*) FROM git_objects` equals `before_objects`, `bytes_written` equals `before_bytes`, and `audit(OWNER, MOUNT)` gains at most one entry whose `detail` contains `"already_present"`.
- **And** no `git_operations` row exists.
- **Verification:** response fields; ref and log equality; relational count; quota; audit inspection.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-919 — EdgeCase — a sha that names a non-commit object**

**Category:** EdgeCase. **Scenario:** SC-918. **Requirements:** FR-NEW-237.
Existing angles: 652 (unknown sha), 653 (malformed sha). This one covers a sha that *exists* but is the wrong object type.
- **Preconditions:** FX-LINE; `<sha-BLOB>` = the blob sha of `/a.txt`'s content and `<sha-TREE>` = the root tree sha of `<sha-of-C4>`, both read via `git.show`/`git2` and both satisfying `db.object_exists(..) == true`.
- **When** `git.cherry_pick {commit_sha:"<sha-BLOB>"}`, then `git.cherry_pick {commit_sha:"<sha-TREE>"}`.
- **Then** both assert error `ERR_NOT_FOUND` whose message contains the supplied sha and the words `"not a commit"` — an existing object of the wrong type is reported as no commit, not as an internal error.
- **And** neither call leaves a `git_operations` row, and `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `db.object_exists("<sha-BLOB>")` is still `true` (the failed lookup pruned nothing).
- **Verification:** two error assertions; relational count; ref read; `object_exists`.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-920 — SideEffect — a soft reset forward leaves the volume behind and the status dirty**

**Category:** SideEffect. **Scenario:** SC-920. **Requirements:** FR-NEW-250.
Existing angles: 660 (soft backwards), 664 (soft keeps existing dirt). This one covers a forward soft move, where the volume becomes dirty *because of* the reset.
- **Preconditions:** FX-LINE; `git.reset {target_ref:"<sha-of-C2>", mode:"hard"}` first, so `main` = `<sha-of-C2>` and the volume matches `C2`'s tree (`/a.txt` = `"a1\n"`, `/b.txt` = `"b1\n"`, no `/d.txt`). Volume clean.
- **When** `git.reset {target_ref:"<sha-of-C4>", mode:"soft"}`.
- **Then** the response is exactly `{"mode":"soft","old_sha":"<sha-of-C2>","new_sha":"<sha-of-C4>","files_changed": 0}`.
- **And** `/a.txt` still reads exactly `b"a1\n"` and `VolumeClient::read_bytes("/d.txt")` errors `ERR_NOT_FOUND` — nothing from `C4`'s tree was written.
- **And** `git.status` reports `dirty == true` with `changes` containing `{"path":"/a.txt","status":"modified"}` and `{"path":"/d.txt","status":"deleted"}` relative to the new HEAD.
- **And** `bytes_written` is unchanged from before the reset.
- **Verification:** exact response; byte read and read error; `git.status` change list; quota equality.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-921 — DataIntegrity — a mistaken hard reset is fully recovered from the reported `old_sha`**

**Category:** DataIntegrity. **Scenario:** SC-921. **Requirements:** FR-NEW-255, FR-NEW-111.
Existing angles: 609 and 666 both assert the orphaned objects still exist. This one completes the loop the requirement exists for: actual recovery.
- **Preconditions:** FX-LINE at `<sha-of-C4>`; capture the byte content of `/a.txt`, `/b.txt`, `/d.txt`.
- **When** `git.reset {target_ref:"<sha-of-C1>", mode:"hard"}` (three commits discarded), then `git.branch_reset {name:"main", target_commit:<old_sha from that response>, force:true}`, then `git.reset {target_ref:"main", mode:"hard"}`.
- **Then** the first response's `old_sha == "<sha-of-C4>"` and after the reset `/b.txt` and `/d.txt` are absent.
- **And** after the recovery, `db.get_ref("refs/heads/main").target == "<sha-of-C4>"`.
- **And** `/a.txt`, `/b.txt` and `/d.txt` are byte-identical to the captured pre-reset content.
- **And** `git.log {ref_name:"main"}` holds exactly the original 4 commits with their original shas.
- **Verification:** response field; two volume states; ref read; three byte compares; log sha list.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-922 — SideEffect — a conflicted revert creates no commit and leaves the volume byte-identical**

**Category:** SideEffect. **Scenario:** SC-922. **Requirements:** FR-NEW-264, FR-NEW-171.
Existing angles: 683 (the conflict is reported), 684 (continue/abort). This one asserts the "applies nothing" half.
- **Preconditions:** `proj1`: `C1` = `/a.txt` `"a1\n"`; `C2` sets `/a.txt` = `"a1\na2\n"`; `C3` sets `/a.txt` = `"a1\nREWRITTEN\n"`. HEAD on `main` = `<sha-of-C3>`. Capture `before_objects`, `before_bytes`, `before_audit`.
- **When** `git.revert {commit_sha:"<sha-of-C2>"}` (its inverse does not apply to `C3`'s tree).
- **Then** `status == "conflict"`, `operation == "revert"`, one entry `path == "/a.txt"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-of-C3>"` and `git.log {ref_name:"main"}` still holds 3 commits.
- **And** `/a.txt` reads exactly `b"a1\nREWRITTEN\n"`, and the volume contains no file holding `"<<<<<<<"`, `"======="` or `">>>>>>>"` (scan every path under `/`).
- **And** `COUNT(*) FROM git_objects` equals `before_objects` and `bytes_written` equals `before_bytes`.
- **And** exactly one `git_operations` row exists with `op_type == "revert"`.
- **Verification:** response fields; ref and log; byte compare plus marker scan; relational counts; quota.
- **Cleanup:** `git.revert_abort` (or `git.cherry_pick_abort` per the registered revert abort tool). **Priority:** P0.

---

**E2E-NEW-923 — EdgeCase — an unsupported provider is rejected before any token is read**

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

**E2E-NEW-924 — EdgeCase — a remote whose URL has no project path**

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

**E2E-NEW-925 — EdgeCase — the head-branch pre-flight passes and the create follows, in that order**

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

**E2E-NEW-926 — Integration — one injected client serves a GitHub and a GitLab mount in the same process**

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

**E2E-NEW-927 — Security — a same-host redirect is followed, a cross-host one is not**

**Category:** Security. **Scenario:** SC-925. **Requirements:** FR-NEW-316.
Existing angles: 794 (cross-host 302 refused), 795 (large body). This one pins the permitted case so the rule is "cross-host", not "no redirects".
- **Preconditions:** Band D, mount `acme-api`. Mock route queue for `GET /repos/acme/api/pulls/42`: first response `301` with header `Location: https://api.github.com/repositories/99/pulls/42`; the route `GET /repositories/99/pulls/42` returns `200` with a full payload. Supporting reviews/check-runs routes return `200`.
- **When** `git.pr_get {mount_id:"acme-api", pr_number:42}`.
- **Then** the call succeeds and `number == 42`.
- **And** `mock.calls()` shows the redirected request was issued against `base_url == "https://api.github.com"` with `path == "/repositories/99/pulls/42"`.
- **And** in the same test, replacing the `Location` with `https://evil.test/repos/acme/api/pulls/42` makes the call assert error containing `"evil.test"` and `"redirect"`, and `mock.calls()` records **no** request to `evil.test` and no `authorization_seen` value for that host.
- **Verification:** success response; recorded call path; contrasting error assertion; recorded-call scan for the foreign host.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-928 — EdgeCase — the scope error names the host and both remedy tools**

**Category:** EdgeCase. **Scenario:** SC-926. **Requirements:** FR-NEW-333.
Existing angles: 776 and 777 both assert the rejection occurs. This one pins the exact actionable text and its safety.
- **Preconditions:** Band D; token for `(dev@test.com, gitlab.example.test)` seeded with `scopes == ["read_repository"]` and token value `glpat_TESTTOKEN_narrow_006`.
- **When** `git.pr_merge {mount_id:"acme-self", pr_number:9, strategy:"squash"}`.
- **Then** assert error `ERR_FORBIDDEN` whose message contains all four of `"api"`, `"gitlab.example.test"`, `"git.auth"` and `"git.token_set"`.
- **And** the message contains neither `"glpat"` nor `"glpat_TESTTOKEN_narrow_006"` nor `"read_repository"`-prefixed token material — naming the *missing* scope is required, echoing the token is not.
- **And** `mock.calls().is_empty()`.
- **And** the same assertion holds for `git.pr_review`, `git.pr_diff` and `git.pr_get` on the same mount, so the message is uniform across the family.
- **Verification:** four error assertions; substring presence and absence; mock call list.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-929 — EdgeCase — an unknown-scope token that the provider accepts works end to end**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-334.
Existing angles: 781 and 782 both assert the call is attempted. This one carries it through to a success, which is the point of the asymmetry.
- **Preconditions:** Band D; `git.token_set` seeds `(dev@test.com, github.com)` with token `ghp_PAT_no_scopes_007` and **no** scope information (`scopes == []`). Mock routes for `GET /repos/acme/api/pulls?state=open&per_page=100` -> `200` with two PR payloads.
- **When** `git.pr_list {mount_id:"acme-api"}`.
- **Then** the call succeeds and returns exactly two normalized entries with `number` `11` and `12`, both `state == "open"`.
- **And** `mock.calls().len() == 1` and its `authorization_seen == "token ghp_PAT_no_scopes_007"` — the unknown scope set was given the benefit of the doubt and the token was actually used.
- **And** replacing the route with a `403` body `{"message":"Resource not accessible by personal access token"}` makes the call error with a message containing that provider text verbatim, asserted in the same test: the provider's refusal is surfaced, not pre-empted or reworded.
- **Verification:** success response; call count and recorded authorization; contrasting error with verbatim provider message.
- **Cleanup:** fixture drop. **Priority:** P0.

---

**E2E-NEW-930 — EdgeCase — `auth_status` reports an empty scope set as unknown, not as insufficient**

**Category:** EdgeCase. **Scenario:** SC-925. **Requirements:** FR-NEW-335, FR-NEW-334.
Existing angles: 775 and 783 both cover tokens with known scopes. This one covers the unknown case the status screen must not misreport.
- **Preconditions:** Band D with three seeded tokens for `dev@test.com`: `github.com` with `["repo"]`, `gitlab.com` with `["read_repository","write_repository"]`, and `github.ibm.com` seeded through `git.token_set` with `scopes == []`.
- **When** `git.auth_status {}`.
- **Then** the entry for `github.com` reports `"scopes":["repo"]` and `"pr_capable": true`.
- **And** the entry for `gitlab.com` reports `"pr_capable": false` and a `"missing_scopes":["api"]` field.
- **And** the entry for `github.ibm.com` reports `"scopes":[]` and `"pr_capable": null` — explicitly unknown, distinguishable from both `true` and `false`, matching the FR-NEW-334 asymmetry.
- **And** no entry contains any token value: the serialized response contains neither `"ghp_"` nor `"glpat"`.
- **Verification:** three entry objects asserted field by field; `null` vs `false` distinguished by `Value::is_null()`; substring absence.
- **Cleanup:** fixture drop. **Priority:** P1.

---

**E2E-NEW-931 — Integration — every hardcoded tool-count site reports the new numbers**

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

### Z.4 Gap 4 — scenarios that had no dedicated test

#### SC-911 — a divergent pull whose two sides changed disjoint files merges automatically

**SEED-PULL-DISJOINT** (used by E2E-NEW-940 .. 944): `git.init` on `proj`; `C0` commits
`/src/api.rs` = `"pub fn get() {}\n"` and `/docs/readme.md` = `"docs v1\n"`; the bare remote
is seeded from that state. The remote then advances with `R1` setting `/src/api.rs` =
`"pub fn get() {}\npub fn post() {}\n"`; the local `main` advances with `L1` setting
`/docs/readme.md` = `"docs v2\n"`. The two sides therefore changed **different files** and
`main` is diverged from `origin/main`. Volume clean. All five tests drive `pull_branch`
directly (`file://` caveat).

---

**E2E-NEW-940 — Happy — a divergent pull with disjoint changes merges with no caller decision**

**Category:** Happy. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-DEL-101.
- **Preconditions:** SEED-PULL-DISJOINT. Local `main` = `<sha-L1>`, remote `main` = `<sha-R1>`, merge base `<sha-C0>`.
- **Given** `git.status` reports `dirty == false` and `db.get_ref("refs/heads/main").target == "<sha-L1>"`.
- **When** the pull runs with `branch: "main"` and **no** `on_conflict` parameter (it no longer exists).
- **Then** the response has `status == "merged"`, no `conflicts` key, no `continue_with`/`abort_with` key and no `operation_id` key — the headline change from the old refuse-unless-`on_conflict` model.
- **And** `/src/api.rs` reads exactly `b"pub fn get() {}\npub fn post() {}\n"` (the remote's change).
- **And** `/docs/readme.md` reads exactly `b"docs v2\n"` (the local change, preserved).
- **And** `db.get_ref("refs/heads/main").target` equals the response's `merge_commit`, a 40-hex sha different from both `<sha-L1>` and `<sha-R1>`.
- **And** no `git_operations` row exists for `volume_id='proj'`, and no `git.merge_resolve` call was needed at any point.
- **Verification:** JSON key absence; two byte-exact reads; ref read and sha inequality; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-941 — Failure — a divergent pull is still refused on a dirty volume**

**Category:** Failure. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-194.
- **Preconditions:** SEED-PULL-DISJOINT plus `fs.write_text {path:"/docs/readme.md", content:"docs LOCAL WIP\n"}` (uncommitted).
- **When** the pull runs with `branch: "main"`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"uncommitted changes"`, `"git.commit"` and `"git.stash_save"` — auto-merging does not mean merging over unsaved work.
- **And** `/docs/readme.md` still reads exactly `b"docs LOCAL WIP\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-L1>"` and no `git_operations` row exists.
- **And** after `git.commit {message:"wip"}`, the same pull succeeds with `status == "merged"` — the refusal was about dirt, not about divergence.
- **Verification:** error code + substrings; byte compare; ref read; relational count; recovery run.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-942 — Failure — the automatic merge is refused when the quota cannot cover it**

**Category:** Failure. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-185, FR-NEW-184.
- **Preconditions:** SEED-PULL-DISJOINT built on `Env::with_quota` sized below the merged result's byte total; local `main` = `<sha-L1>`.
- **When** the pull runs with `branch: "main"`.
- **Then** assert error `ERR_WRITE_QUOTA_EXCEEDED`.
- **And** `/src/api.rs` reads exactly `b"pub fn get() {}\n"` (the pre-pull local bytes) and `/docs/readme.md` reads exactly `b"docs v2\n"` — nothing was partially written.
- **And** `db.get_ref("refs/heads/main").target == "<sha-L1>"` (no merge commit, no ref move).
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"` and `db.object_exists("<sha-R1>")` is `true` — fetched objects and tracking refs are retained, per FR-MOD-104's business rule, so a retry after raising the quota costs no second download.
- **And** after raising the quota, the retry returns `status == "merged"`.
- **Verification:** error code; two byte compares; three ref/object reads; retry.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-943 — EdgeCase — disjoint *regions of the same file* also merge automatically**

**Category:** EdgeCase. **Scenario:** SC-911. **Requirements:** FR-NEW-173, FR-MOD-104.
- **Preconditions:** **SEED-PULL-SAMEFILE**: `C0` commits `/src/config.toml` = `"[server]\nport = 8080\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"`; the remote's `R1` sets line 2 to `port = 9090`; the local `L1` sets line 6 to `retries = 7`. Clean volume, diverged.
- **When** the pull runs with `branch: "main"`.
- **Then** `status == "merged"` with no `conflicts` key.
- **And** `/src/config.toml` reads exactly:
```
[server]
port = 9090
host = "0.0.0.0"
workers = 4
timeout = 30
retries = 7
```
  byte for byte, including the trailing newline — both edits present, one file, no markers.
- **And** the file contains none of `"<<<<<<<"`, `"======="`, `">>>>>>>"`.
- **And** no `git_operations` row exists.
- **Verification:** byte-exact whole-file compare; marker substring absence; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-944 — SideEffect — the automatic pull merge leaves exactly the expected refs, parents and audit**

**Category:** SideEffect. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-NEW-184.
- **Preconditions:** SEED-PULL-DISJOINT; capture `before_bytes` and `before_audit`.
- **When** the pull of E2E-NEW-940 runs and succeeds.
- **Then** the merge commit has exactly 2 parents: `parent_id(0) == "<sha-L1>"` (the local tip) and `parent_id(1) == "<sha-R1>"` (the fetched remote tip), matching the existing ordering at `git.rs:1878-1887`.
- **And** its message is exactly `Merge remote-tracking branch 'origin/main' into main`.
- **And** `db.get_ref("refs/remotes/origin/main").target == "<sha-R1>"`.
- **And** exactly one new audit entry has `op == "git.remote_pull"` with a `detail` containing `"merged"` and not `"conflict"`.
- **And** `bytes_written(OWNER, MOUNT) - before_bytes` equals the byte length of `/src/api.rs`'s new content plus any other file actually rewritten, computed in-test from the merged tree rather than restated as a literal.
- **And** no `git_operations` row was created at any point during the call (checked immediately after).
- **Verification:** `parent_count()`/`parent_id()`; message equality; ref read; audit entry; quota arithmetic; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### SC-913 — a divergent pull conflict resolved by caller-supplied literal content

**SEED-PULL-CONFLICT** (used by E2E-NEW-945 .. 949): `C0` commits `/src/config.toml` with
line 2 `port = 8080`; the remote's `R1` sets line 2 to `port = 9090`; the local `L1` sets
line 2 to `port = 8000`. Both sides changed the same line, so the pull conflicts. Clean
volume. Pull driven through `pull_branch` directly.

---

**E2E-NEW-945 — Happy — a pull conflict resolved with bytes belonging to neither side**

**Category:** Happy. **Scenario:** SC-913. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-175, FR-NEW-196.
- **Preconditions:** SEED-PULL-CONFLICT.
- **Given** the pull returns `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), `conflicts[0].path == "/src/config.toml"`, `ours.content` line 2 `port = 8000`, `theirs.content` line 2 `port = 9090`, `base.content` line 2 `port = 8080`, `continue_with == "git.merge_resolve"`, `abort_with == "git.merge_abort"`.
- **When** `git.merge_resolve {mount_id:"proj", resolutions:[{path:"/src/config.toml", content:"[server]\nport = 8443\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}]}`.
- **Then** `status == "merged"` and `files_changed == 1`.
- **And** `/src/config.toml` reads exactly those supplied bytes, line 2 being `port = 8443` — a value present on neither side, used verbatim with no re-merge.
- **And** the merge commit has exactly 2 parents, `parent_id(0) == "<sha-L1>"`, `parent_id(1) == "<sha-R1>"`.
- **And** `db.get_ref("refs/heads/main").target` equals that commit and no `git_operations` row remains.
- **Verification:** conflict response fields; byte-exact whole-file compare; `parent_count()`/`parent_id()`; ref read; relational count.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-946 — Failure — supplied content is refused for a binary path in a pull conflict**

**Category:** Failure. **Scenario:** SC-913. **Requirements:** FR-NEW-182, FR-MOD-105.
- **Preconditions:** **SEED-PULL-BINARY**: `C0` commits `/assets/logo.png` = bytes `89 50 4E 47 0D 0A 1A 0A 00 01`; the remote sets the last byte to `0x02`, the local sets it to `0x03`. The pull returns `status == "conflict"` with `conflicts[0].binary == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", content:"anything"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"binary"`, `"ours"` and `"theirs"`.
- **And** `/assets/logo.png` reads exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]` (the local bytes; nothing applied).
- **And** the `git_operations` row is retained with `op_type == "merge"` (a pull is recorded as a merge, FR-NEW-285) and the path unresolved.
- **And** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", strategy:"theirs"}]}` then succeeds and the last byte is `0x02`.
- **Verification:** error code + substrings; byte-slice equality; relational row; recovery call plus byte read.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-947 — Failure — one bad path rejects the whole content resolution**

**Category:** Failure. **Scenario:** SC-913. **Requirements:** FR-NEW-177, FR-NEW-175, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT extended so two paths conflict: `/src/config.toml` and `/src/app.rs` (both sides edit line 1 differently). The pull pauses with both in the conflict set.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"port = 8443\n"}, {path:"/src/app.rs", content:"fn main() {}\n"}, {path:"/docs/readme.md", content:"docs\n"}]}` where `/docs/readme.md` is **not** in the conflict set.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"/docs/readme.md"` and `"not in conflict"`.
- **And** the `git_operations` row's `resolutions` column is still an empty object — the call is all-or-nothing, so the two valid entries were not recorded either.
- **And** `/src/config.toml`, `/src/app.rs` and `/docs/readme.md` are byte-identical to their pre-resolve content.
- **And** re-issuing the call without the third entry succeeds: `status == "merged"` and `/src/config.toml` reads exactly `b"port = 8443\n"`.
- **Verification:** error code + substrings; relational row column; three byte compares; retry.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

**E2E-NEW-948 — EdgeCase — empty supplied content produces an empty file, not a deletion**

**Category:** EdgeCase. **Scenario:** SC-913. **Requirements:** FR-NEW-175, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT, paused on `/src/config.toml`.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:""}]}`.
- **Then** `status == "merged"`.
- **And** `/src/config.toml` **exists** and `VolumeClient::read_bytes` returns a zero-length slice — an empty string is content, not an instruction to delete.
- **And** the merge commit's tree contains an entry for `src/config.toml` (asserted via `git.show`), and per the content-addressing rule an empty file stores no blob, so `COUNT(*) FROM blob_refs` did not grow for it.
- **And** a contrasting resolution in the same test, `{path:"/src/config.toml", strategy:"theirs"}` on a fresh run, yields the non-empty remote content, so the empty case is not a silent fallback.
- **Verification:** volume read length; `git.show` tree entry; blob-ref count; contrasting fresh run.
- **Cleanup:** tempdir drop. **Priority:** P1.

---

**E2E-NEW-949 — SideEffect — the resolved pull charges exactly the written bytes and clears the row**

**Category:** SideEffect. **Scenario:** SC-913. **Requirements:** FR-NEW-184, FR-NEW-284, FR-MOD-105.
- **Preconditions:** SEED-PULL-CONFLICT, paused; capture `paused_bytes = bytes_written(OWNER, MOUNT)` and `paused_audit = audit(OWNER, MOUNT)`.
- **Given** while paused, `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `1` and `bytes_written` equals its pre-pull value (a conflict charges nothing).
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"<69-byte resolved config>"}]}` where the content's byte length is computed in-test as `resolved.len()`.
- **Then** `bytes_written(OWNER, MOUNT) - paused_bytes == resolved.len()` exactly — one charge, for the one file actually written.
- **And** `COUNT(*) FROM git_operations WHERE volume_id='proj'` is `0`.
- **And** exactly one new audit entry appeared since `paused_audit`, with `op == "git.merge_resolve"` (or `"git.remote_pull"` per the registered audit op) and a `detail` containing `"merged"` and `"/src/config.toml"`.
- **And** `git.merge_abort` immediately afterwards errors `ERR_INVALID_ARGUMENT` containing `"no merge is in progress"`, confirming the record is truly cleared and not merely marked.
- **Verification:** quota arithmetic; relational counts before and after; audit diff; follow-up error.
- **Cleanup:** tempdir drop. **Priority:** P0.

---

#### SC-930 — a stash applied cleanly onto a branch after its source branch was deleted

**E2E-NEW-950 — Happy — stash taken on a deleted branch applies cleanly onto another**

**Category:** Happy. **Scenario:** SC-930. **Requirements:** FR-NEW-129, FR-NEW-124, FR-NEW-122.
This is the happy path SC-930 previously lacked; the existing
464 and 465 are EdgeCase assertions on listability and on cross-branch application, and
462/463/540 are failures.
- **Preconditions:** SEED-B (`main` = `<sha-C2>`, `release/1.0` = `<sha-C3>`, HEAD on `main`).
- **Given** `git.branch_create {name:"feature/tmp", start_point:"<sha-C2>", checkout:true}`; `fs.write_text {path:"/src/lib.rs", content:"fn a() {}\nfn b() {}\n"}`; `git.stash_save {message:"wip b"}` -> `<stash-1>` with `base_sha == "<sha-C2>"`, and the volume back at `<sha-C2>`'s tree (`/src/lib.rs` == `"fn a() {}\n"`).
- **When** `git.branch_switch {name:"main"}`, then `git.branch_delete {name:"feature/tmp"}` (merged into nothing new, so no `force` needed since its tip equals `main`'s), then `git.stash_list`, then `git.stash_apply {stash_id:"<stash-1>"}`.
- **Then** `git.stash_list` returns exactly one entry: `{stash_id:"<stash-1>", message:"wip b", base_sha:"<sha-C2>"}` — the deleted branch name appears nowhere in it.
- **And** the apply returns `status == "applied"` with `files_changed == 1`, no `conflicts` key.
- **And** `/src/lib.rs` reads exactly `b"fn a() {}\nfn b() {}\n"`.
- **And** `db.get_ref("refs/heads/main").target == "<sha-C2>"` (an apply does not commit) and `git.status` reports `dirty == true` with `changes == [{"path":"/src/lib.rs","status":"modified"}]`.
- **And** `git.stash_list` still returns the entry afterwards (apply retains), and `db.get_ref("refs/heads/feature/tmp")` is `None`.
- **Verification:** stash list object equality; apply response; byte-exact read; ref reads; `git.status` change list.
- **Cleanup:** fixture drop. **Priority:** P0.

---


#### Z.5 Completion-pair routing, revert continuation, and contract enumeration

**E2E-NEW-951 — Each operation type routes to exactly one completion pair**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-225
- **Preconditions:** Repo `gitproj` on `main` with commits `C1`, `C2`. Four independent runs, each starting from a clean volume at `C2`.
- **Steps:**
  - Given a conflicted `git.merge` of branch `feature` (both sides changed `/a.txt` line 1)
  - Then `git.merge_resolve {"/a.txt":"ours"}` completes it and `git.rebase_continue`, `git.cherry_pick_continue` and `git.revert_continue` each return `ERR_INVALID_ARGUMENT` whose message contains `merge`
  - And the same matrix holds for a conflicted `git.rebase` (only `git.rebase_continue` advances it), a conflicted `git.cherry_pick` (only `git.cherry_pick_continue`), a conflicted `git.revert` (only `git.revert_continue`), and a conflicted `git.stash_pop` (only `git.merge_resolve`)
  - And a conflicted `git.remote_pull` is advanced only by `git.merge_resolve`
- **Verification:** each call's error code and message substring; `SELECT op_type FROM git_operations WHERE volume_id='gitproj'` unchanged after every rejected call.
- **Cleanup:** abort each operation. **Priority:** P0

**E2E-NEW-952 — `git.revert_continue` is rejected while a rebase is paused**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-225
- **Preconditions:** A rebase of `feature` onto `main` paused at step 2 with `/a.txt` conflicting.
- **Steps:**
  - Given `git_operations` holds one row with `op_type='rebase'`, `current_step=2`
  - When `git.revert_continue {"resolutions":{"/a.txt":"ours"}}` is called
  - Then the response is `ERR_INVALID_ARGUMENT` and the message contains `rebase` and `git.rebase_continue`
  - And the `git_operations` row still has `op_type='rebase'` and `current_step=2`, unchanged
  - And no commit was created: `git.log` length is identical to before the call
- **Verification:** error code and substrings; row field equality; log length.
- **Cleanup:** `git.rebase_abort`. **Priority:** P0

**E2E-NEW-953 — The conflict response names the correct completion pair for each operation**
- **Category:** EdgeCase | **Scenario:** SC-929 | **Requirements:** FR-NEW-241, FR-NEW-170
- **Preconditions:** As E2E-NEW-951, one conflicted operation of each of the five types.
- **Steps:**
  - Given each conflicted operation's response
  - Then `continue_with` and `abort_with` are exactly: merge -> `git.merge_resolve` / `git.merge_abort`; remote_pull -> `git.merge_resolve` / `git.merge_abort`; stash_apply -> `git.merge_resolve` / `git.merge_abort`; rebase -> `git.rebase_continue` / `git.rebase_abort`; cherry_pick -> `git.cherry_pick_continue` / `git.cherry_pick_abort`; revert -> `git.revert_continue` / `git.revert_abort`
  - And calling exactly the tool named in `continue_with` always advances the operation
- **Verification:** exact string equality on both JSON fields, then a successful continuation call per operation.
- **Cleanup:** complete or abort each. **Priority:** P0

**E2E-NEW-954 — `git.revert_continue` completes a conflicted revert**
- **Category:** Happy | **Scenario:** SC-922 | **Requirements:** FR-NEW-266, FR-NEW-264
- **Preconditions:** `C1` commits `/cfg.txt` = `mode=a\n`. `C2` changes it to `mode=b\n`. `C3` changes it to `mode=c\n`. HEAD is `C3`.
- **Steps:**
  - Given `git.revert {"commit_sha":"<sha-of-C2>"}` returns `status:"conflict"` because the inverse of `C2` does not apply over `C3`, with `/cfg.txt` in the conflict set
  - And `git_operations` holds one row with `op_type='revert'`
  - When `git.revert_continue {"resolutions":{"/cfg.txt":{"content":"mode=a\n"}}}` is called
  - Then a new commit exists whose parent is `<sha-of-C3>` and whose `/cfg.txt` blob is exactly `mode=a\n`
  - And `/cfg.txt` in the volume is exactly `mode=a\n`
  - And `SELECT COUNT(*) FROM git_operations WHERE volume_id='gitproj'` is 0
  - And `<sha-of-C2>` is still reachable from the new tip
- **Verification:** commit parent and blob bytes via `git.show`; volume bytes; row count; `git.log` contains `C2`.
- **Cleanup:** none. **Priority:** P0

**E2E-NEW-955 — `git.revert_abort` restores the tip and every byte exactly**
- **Category:** SideEffect | **Scenario:** SC-922 | **Requirements:** FR-NEW-266, FR-NEW-171
- **Preconditions:** As E2E-NEW-954, paused on the conflicted revert. Snapshot the branch tip sha and a map of every volume path to its bytes beforehand.
- **Steps:**
  - When `git.revert_abort {}` is called
  - Then `refs/heads/main` equals the snapshotted `<sha-of-C3>`
  - And the rebuilt path-to-bytes map equals the snapshot exactly, key for key and byte for byte
  - And `SELECT COUNT(*) FROM git_operations WHERE volume_id='gitproj'` is 0
  - And no commit object was created: `git.log` length equals the pre-revert length
- **Verification:** ref value; full volume byte map comparison; row count; log length.
- **Cleanup:** none. **Priority:** P0

**E2E-NEW-956 — `git.revert_continue` with no revert in progress**
- **Category:** Failure | **Scenario:** SC-922 | **Requirements:** FR-NEW-266
- **Preconditions:** Clean volume at `C3`, `git_operations` empty.
- **Steps:**
  - When `git.revert_continue {"resolutions":{}}` is called
  - Then the response is `ERR_INVALID_ARGUMENT` and the message contains `no revert is in progress`
  - And no row is created in `git_operations`
- **Verification:** error code and substring; row count is 0.
- **Cleanup:** none. **Priority:** P1

**E2E-NEW-957 — The documented tool count matches the live registry**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-348, FR-NEW-346
- **Preconditions:** A registry built with git enabled.
- **Steps:**
  - Given the integer parsed from the tool-count claim in `AGENTS.md` and the one in `.agent_docs/tools.md`
  - Then both equal the frozen-contract length, 94
  - And both files' git-family count equals 49
- **Verification:** parse the counts from the two files at test time and compare to `reg.len()`; no hardcoded expectation beyond the registry itself.
- **Cleanup:** none. **Priority:** P1

**E2E-NEW-958 — `.agent_docs/tools.md` lists every registered git tool**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-348, FR-NEW-349
- **Steps:**
  - Given the set of registered tool names beginning `git.`
  - Then every one of them appears verbatim in `.agent_docs/tools.md`
  - And the file names no `git.` tool that is not registered
- **Verification:** set equality between the registry and the names parsed from the document.
- **Cleanup:** none. **Priority:** P1

**E2E-NEW-959 — A tool missing from the documentation fails the parity test**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-348
- **Steps:**
  - Given a registry with one extra tool `git.undocumented_probe` registered in the test only
  - When the parity assertion of E2E-NEW-958 runs against the real `.agent_docs/tools.md`
  - Then it fails, naming `git.undocumented_probe`
- **Verification:** the assertion's own failure message; this test asserts the guard actually catches a gap rather than passing vacuously.
- **Cleanup:** none. **Priority:** P1

**E2E-NEW-960 — The 31 enumerated names are exactly the new tool set**
- **Category:** Integration | **Scenario:** SC-929 | **Requirements:** FR-NEW-349, FR-NEW-345
- **Steps:**
  - Given the 18 tool names shipped before this specification and the 31 names enumerated in FR-NEW-349
  - Then the registered git-family set equals the union of the two, with no extra and none missing
  - And the union has exactly 49 members, and `reg.len()` is exactly 94
  - And every one of the 31 has an entry in `tool-contract-golden.json` carrying a required string `mount_id`, except `git.stash_list` and `git.remote_list` which still require `mount_id` per the family convention
- **Verification:** set equality against a literal list of the 31 names; registry length; golden-file lookup per name.
- **Cleanup:** none. **Priority:** P0

**E2E-NEW-961 — Registering an unlisted git tool fails the enumeration test**
- **Category:** Failure | **Scenario:** SC-929 | **Requirements:** FR-NEW-349
- **Steps:**
  - Given a registry with a 32nd new tool `git.rogue` registered
  - When the set equality of E2E-NEW-960 is evaluated
  - Then it fails, naming `git.rogue` as present but not enumerated
- **Verification:** the assertion's failure message; proves the enumeration is a real gate and not a comment.
- **Cleanup:** none. **Priority:** P1


### 12.3 Modified Test Specifications

Derived from Section 9.3. Every entry names the shipped Rust test function, what it pinned
before this specification, and what it pins after. The shipped function names are kept: a
rename would lose the git history that ties the test to the archived specification.

---

**E2E-MOD-401 — `e2e_new_118_a_diverged_pull_with_theirs_merges`**
**Category:** Happy. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** a divergent `git.remote_pull` called with `on_conflict: "theirs"` merged, taking the remote side of every conflicting file through one global `MergeOptions::file_favor`, and committed in the same call.
- **Validates now:** the same divergence produces a conflict response instead of a silent global resolution, and the caller finishes it per file with `git.merge_resolve`. The `on_conflict` parameter no longer exists.
- **Given** a volume whose local `main` and remote `main` have diverged, both sides having changed the same lines of `/shared.txt`, and a clean volume.
- **When** `git.remote_pull {mount_id, branch:"main"}` is called with no `on_conflict` parameter.
- **Then** the call returns `Ok` with `status == "conflict"`, `operation == "merge"` (a pull conflict is recorded and reported as a merge, FR-NEW-285), and `conflicts[0].path == "/shared.txt"` carrying `ours`, `theirs` and `base` sides.
- **And** no commit is created and the volume is byte-identical to its pre-pull state.
- **And** exactly one `git_operations` row exists for this `volume_id`.
- **When** `git.merge_resolve {mount_id, resolutions:[{path:"/shared.txt", strategy:"theirs"}]}`.
- **Then** `status == "merged"`, a merge commit exists, `/shared.txt` holds the remote side byte-for-byte, and the `git_operations` row is gone.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

**E2E-MOD-402 — `e2e_new_128_a_diverged_pull_with_ours_keeps_local_content`**
**Category:** Happy. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105, FR-NEW-174, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** a divergent pull with `on_conflict: "ours"` kept the local content of every conflicting file.
- **Validates now:** the identical guarantee, reached through the shared conflict model with a per-file `ours` strategy rather than a global parameter.
- **Given** the divergence of E2E-MOD-401, `/shared.txt` conflicting on both sides.
- **When** `git.remote_pull {mount_id, branch:"main"}`.
- **Then** `status == "conflict"` and nothing is applied.
- **When** `git.merge_resolve {mount_id, resolutions:[{path:"/shared.txt", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `/shared.txt` holds the local pre-pull bytes exactly.
- **And** the merge commit has exactly two parents, the second being the fetched remote tip.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

**E2E-MOD-403 — `e2e_new_129_non_conflicting_changes_from_both_sides_are_kept`**
**Category:** Happy. **Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-NEW-173, FR-DEL-101.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** with `on_conflict` supplied, a divergent pull touching disjoint files kept both sides' changes. Without `on_conflict` the same pull was refused.
- **Validates now:** the pull completes automatically with no caller decision and no strategy parameter at all. This is the test that carries scenario SC-911.
- **Given** local `main` and remote `main` diverged, the local side having changed `/local_only.txt` and the remote side `/remote_only.txt`, no path changed on both sides, volume clean.
- **When** `git.remote_pull {mount_id, branch:"main"}` with no `on_conflict` and no other resolution parameter.
- **Then** the call returns `status == "merged"` (not `"conflict"`, and not an error).
- **And** a merge commit with exactly two parents exists and `refs/heads/main` points at it.
- **And** `/local_only.txt` and `/remote_only.txt` both hold their respective changed bytes.
- **And** no `git_operations` row was ever created for this `volume_id`.
- **Cleanup:** fixture drop.
- **Priority:** P0.

---

**E2E-MOD-404 — `e2e_new_125_a_refused_diverged_pull_keeps_the_fetched_objects`**
**Category:** DataIntegrity. **Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-MOD-105.
File: `crates/mcp-fs/src/tools/git.rs`.
- **Validated before:** when a divergent pull was refused outright, the objects fetched during that pull and the remote-tracking ref were retained rather than rolled back.
- **Validates now:** the same retention guarantee, but the trigger is a conflict response that leaves the operation in progress instead of a refusal.
- **Given** a divergence that conflicts on `/shared.txt`.
- **When** `git.remote_pull {mount_id, branch:"main"}` returns `status == "conflict"`.
- **Then** every object fetched during that call is present in the object store, asserted by reading the remote tip commit object back.
- **And** `refs/remotes/origin/main` equals the fetched remote tip.
- **And** `refs/heads/main` is unmoved.
- **And** a second `git.remote_pull` is rejected by the in-progress guard rather than re-fetching.
- **Cleanup:** `git.merge_abort`, then fixture drop.
- **Priority:** P0.

---

**E2E-MOD-405 — tool name list and count at `crates/mcp-fs/src/tools/git.rs:2480-2504`**
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

**E2E-MOD-406 — `a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool` (`git.rs:2685`)**
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

**E2E-MOD-407 — tool count assertions in `crates/mcp-fs/src/tools/all.rs` (`:78`, `:122`, `:141` only)**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-346.
- **Validated before:** five hardcoded totals over the registered tool surface, computed against 63 tools with a 14-tool git family.
- **Validates now:** the same five assertions against the new totals, 94 tools with a 49-tool git family, including the git-disabled variants where the git family contributes zero.
- **Given** registries built in each configuration the five assertions cover (git enabled, git disabled, search enabled, search disabled).
- **When** `register_all` runs.
- **Then** each of the five counts equals its updated value, and each remains internally consistent with the family-level counts asserted next to it.
- **Cleanup:** none.
- **Priority:** P0.

---

**E2E-MOD-408 — frozen contract count at `crates/mcp-fs/src/tools/contract_golden.rs:131`**
**Category:** Integration. **Scenario:** SC-929. **Requirements:** FR-NEW-345, FR-NEW-346.
- **Validated before:** the frozen contract contains exactly 63 tools.
- **Validates now:** exactly 94.
- **Given** the regenerated `tool-contract-golden.json`.
- **When** the golden contract test runs.
- **Then** the tool count is 94 and every name, description and serialized `inputSchema` matches the golden byte for byte, key order included.
- **Cleanup:** none.
- **Priority:** P0.

---

**E2E-MOD-409 — `tool-contract-golden.json` and `TOOL_CONTRACT.txt` regeneration**
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

### 12.4 Removed Tests

Derived from Section 9.3. Both removals exist only because the behaviour they pin is
deleted by FR-DEL-101 and FR-MOD-104; neither is replaced by a weaker assertion.

---

**E2E-DEL-401 — remove `e2e_new_117_a_diverged_pull_with_no_strategy_is_refused`**
File: `crates/mcp-fs/src/tools/git.rs`.
**Scenario:** SC-911. **Requirements:** FR-MOD-104, FR-DEL-101.
**Reason:** the test asserts that a divergent `git.remote_pull` without `on_conflict` is
refused. FR-MOD-104 makes that same pull auto-merge when it can and return a conflict
response when it cannot, and FR-DEL-101 removes the parameter the refusal was built
around. The refusal no longer exists, so the test cannot be adapted: the positive
behaviour that replaces it is pinned by E2E-MOD-403 (auto-merge) and E2E-MOD-401
(conflict response), so nothing is lost by deleting it.

---

**E2E-DEL-402 — remove `e2e_new_126_the_diverged_pull_error_names_the_remedy`**
File: `crates/mcp-fs/src/tools/git.rs`.
**Scenario:** SC-912. **Requirements:** FR-MOD-104, FR-DEL-101.
**Reason:** the test asserts the exact wording of the diverged-pull refusal error, telling
the caller to re-run the pull with `on_conflict`. Both the error and the parameter it
names are deleted. The replacement guidance now travels in the conflict response's
`continue_with`/`abort_with` field, which is asserted by E2E-NEW-502 and E2E-MOD-401.
## 13. Consistency Notes

Existing specifications are **not modified**. Where this specification departs from one,
the deviation is recorded here.

### 13.1 Deviation from the archived GitHub Enterprise and token store specification

`specs/archived/2026-09-21_00-34-13-github-enterprise-and-token-store.md` is implemented
and shipped, with 19 stories in `specs/stories/`. This specification supersedes part of
its pull behaviour.

| Archived requirement | What it said | What this specification does | Resolution |
|---|---|---|---|
| FR-NEW-029 | A diverged pull is refused unless `on_conflict` is supplied, with an error naming `on_conflict` as the remedy (archived spec `:756`) | A diverged pull merges automatically, or returns a conflict response | **Superseded** by FR-MOD-104 |
| FR-NEW-031 | With `on_conflict` supplied, a three-way merge runs with `MergeOptions::file_favor` set globally (archived spec `:486-505`) | Per-file resolution by strategy or by supplied content | **Superseded** by FR-NEW-174 through FR-NEW-176 |
| FR-NEW-032 | Conflict markers never enter the volume | Identical rule | **Retained unchanged** as FR-NEW-172 |
| FR-NEW-035 | Pull's volume write is atomic | Identical rule, extended to every combine operation | **Retained and extended** as FR-NEW-184 |
| FR-NEW-020 | Clone records exactly one remote named `origin` | Clone is unchanged; other remotes can now also be added | **Extended**, not contradicted |
| FR-NEW-022 | Push targets `origin` under the same branch name | `remote` and `remote_branch` parameters added, both defaulting to the old behaviour | **Extended** by FR-MOD-101, FR-MOD-102 |
| FR-NEW-024 | Push is fast-forward only; force is not supported | A leased force path is added; the unforced path is unchanged | **Extended** by FR-NEW-155, FR-NEW-160 |
| FR-NEW-041, FR-NEW-042 | HTTPS only; no credentials in URLs | Identical rules applied to `git.remote_add` | **Retained** |
| FR-NEW-007 | An undeclared host is rejected before the network | Identical rule applied to the PR tools | **Retained and extended** |
| FR-NEW-018 | An expired token is refused before the network | Extended: an insufficient scope is likewise refused before the network | **Extended** by FR-NEW-332 |
| DEC-013 | Force push deferred, with a recommended safety design | Implemented, with the lease mechanism strengthened from that recommendation | **Resolved** |
| DEC-024 | Real conflict resolution deferred as a subsystem | Implemented in the form agreed at this interview | **Resolved** |
| DEC-021 | Remote surface deliberately kept to one target | Reversed deliberately | **Reversed**, see DEC-907 |

**The one user-visible break** is FR-DEL-101: `on_conflict` is removed from
`git.remote_pull`. A caller that passes it no longer gets the old behaviour. This was
raised explicitly at interview with the backward-compatible alternative on the table, and
removal was chosen (DEC-908).

### 13.2 Divergences from standard git, updated

BL-013 in `specs/BACKLOG.md` holds a register of the ways this server's git behaviour
differs from ordinary git. This specification closes many of those rows. The register
after this work:

| Area | Standard git | After this specification |
|---|---|---|
| Working tree | Index plus working tree plus HEAD | No index; the volume is the working tree. **Unchanged, by design** |
| Staging | `git add` selects what to commit | `git.commit` commits the whole volume. **Unchanged** |
| Dirty tree | Many operations warn or stash | Stash now exists; switch, merge, rebase and pull refuse a dirty volume and name stash as the remedy. **Closed** |
| Discarding changes | `git checkout -- .`, `git restore` | `git.reset --hard`. **Closed** (BL-010) |
| Merge conflicts | Markers, manual resolution | Surfaced per file with both sides; resolved by strategy or supplied content; markers still never written. **Closed in the agreed form** (BL-007) |
| Rebase | Available, interactive | Available, interactive, with pick/squash/drop/reword. **Closed** |
| Cherry-pick, revert | Available | Available. **Closed** |
| Stash | Available | Available. **Closed** |
| Push refspecs | Arbitrary `local:remote` | `local:remote` supported; multiple refs and tags still not. **Partially closed** (BL-009) |
| Force push | `--force`, `--force-with-lease` | Lease mandatory, bare force refused. **Closed, more strictly than git** (BL-004) |
| Remotes | Many, freely managed | Many, freely managed. **Closed** (BL-008) |
| Pull requests | Not a git feature | Full create/list/get/diff/merge/review. **Closed** (BL-005, BL-006) |
| Transport | HTTPS, SSH, git, file | HTTPS only. **Unchanged, deliberate** |
| Credential helpers | Pluggable | One shape, `oauth2:<token>`. **Unchanged** (BL-011 open) |
| Submodules, tags push, hooks, bisect, worktrees | Available | Not supported. **Unchanged, out of scope** |
| Rebase actions | pick/squash/fixup/edit/reword/drop/exec/break | pick/squash/drop/reword only. **Narrowed deliberately** |
| Reset modes | soft/mixed/hard | soft/hard. **Narrowed deliberately**, no index to distinguish mixed |

### 13.3 Interaction with the horizontal-scaling backlog

BL-003 and BL-014 record that session state and the git write lock are per process. This
specification adds the `git_operations` table, which is shared rather than per process, so
a paused operation is correctly visible across replicas. It does **not** make the system
multi-replica safe: the write lock that serializes ref updates remains in-process
(`crates/mcp-fs/src/git/repo.rs:44`). The single-replica assumption stands and BL-014
stays open. This is called out so nobody reads the new shared table as having solved it.

## 14. Migration & Implementation Notes

### 14.1 Required implementation order

Some of these orderings are load-bearing: doing them in the wrong sequence leaves the
tree red or silently drops a guarantee.

1. **The `git_operations` table and its `TABLES` registration first** (FR-NEW-275, FR-NEW-276, FR-NEW-277). Every pausable operation depends on it. Registering it in `TABLES` in the same change, not later, is what prevents an orphaned-row defect that would only appear on project deletion.
2. **Extract the shared merge engine before adding any operation that uses it.** Lift the merge block (`crates/mcp-fs/src/tools/git.rs:1835-1839`) and `apply_pull_changes_atomically` (`:2079-2127`) into a reusable engine with the existing pull tests still green. This is a pure refactor step, and keeping it pure is what makes the following steps reviewable.
3. **The shared conflict model** (FR-NEW-170 through FR-NEW-185) on top of that engine, with the in-progress guard (FR-NEW-279 through FR-NEW-284).
4. **`git.merge`, `git.merge_resolve`, `git.merge_abort`** (FR-NEW-190 onward), which are the first consumers and the simplest.
5. **`git.remote_pull` rework** (FR-MOD-104, FR-MOD-105, FR-DEL-101). This must come **after** step 4, because it reuses `git.merge_resolve` rather than owning a resolution tool. Doing it before means writing a pull-specific resolver and then deleting it.
6. **Branch lifecycle** (FR-NEW-101 onward) and **stash** (FR-NEW-120 onward). Stash must land **with or before** branch switch, because switch refuses a dirty volume and names stash as the remedy; shipping switch first makes that error message a lie.
7. **Rebase** (FR-NEW-210 onward) and **cherry-pick** (FR-NEW-235 onward), which need both the conflict model and multi-step operation state.
8. **Reset and revert** (FR-NEW-250, FR-NEW-260). Revert needs the conflict model; reset does not.
9. **Remote management** (FR-NEW-140 onward) and the `remote` / `remote_branch` parameters (FR-MOD-101 through FR-MOD-103). These are independent of the conflict work and can proceed in parallel.
10. **Force push with lease** (FR-NEW-155 onward), after the remote parameters exist so the lease applies uniformly to any remote.
11. **The OAuth scope change** (FR-NEW-330, FR-MOD-109) **before** any PR tool, otherwise every GitLab PR test fails for a reason unrelated to the code under test.
12. **The provider client seam** (FR-NEW-315) before the PR tools, since the tools are untestable without it.
13. **PR tools** (FR-NEW-300 onward).
14. **Contract regeneration and count updates last** (FR-NEW-345 through FR-NEW-348), once the tool set is final. Regenerating early means doing it repeatedly and reviewing the same diff many times.

### 14.2 User-visible migration: GitLab tokens must be re-granted

Existing GitLab tokens were granted `read_repository write_repository` and therefore carry
**no REST API access at all**. After FR-NEW-330 ships, merge-request tools will fail the
scope check (FR-NEW-332) with an actionable error until the user re-runs `git.auth` for
that host. This affects every existing GitLab user and must appear in the release note.
GitHub users are unaffected: `repo` already covers the pull-request surface.

Existing tokens continue to work unchanged for clone, push, fetch and pull. Only the new
merge-request tools require the re-grant.

### 14.3 Contract regeneration

Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib
tool_contract_golden_is_current`, then **review the diff**. The expected diff is 31 added
tools plus the modified schemas of `git.remote_push`, `git.remote_fetch`,
`git.remote_pull`, `git.branches` and `git.status`. Any other change in that diff is an
accident and must be explained before it is committed.

### 14.4 Rollback

Every change is additive except FR-DEL-101 and FR-MOD-104. Rolling back means
unregistering the new tools, dropping `git_operations`, and restoring the `on_conflict`
parameter. Operation rows are transient, so dropping the table costs only in-flight
operations. There is no data migration to reverse and no stored format that changes.

## 15. Open Questions & TBDs

- **TBD-901: GitHub classic-scope verification at runtime.** GitHub no longer publishes a per-endpoint classic-OAuth-scope mapping; `repo` covering the whole PR surface is derived from their scopes table rather than stated per endpoint. The robust approach is to read the `X-Accepted-OAuth-Scopes` response header and assert against it. FR-NEW-332 validates against a statically configured required-scope set; whether to additionally assert the header at runtime is deferred. Low risk: the failure mode is a provider 403 surfaced faithfully.
- **TBD-902: GitLab approval semantics vary by tier and project settings.** `git.pr_review` with `approve` maps to GitLab's approve endpoint, whose availability and required role differ by plan and project configuration. The specification surfaces the provider's error faithfully rather than modelling the matrix. Whether to pre-check approval eligibility is deferred.
- **TBD-903: Squash message composition for a rebase `squash` chain.** FR-NEW-212 concatenates messages. Whether to offer a caller-supplied combined message per squash group, as `git rebase -i` does through the editor, is deferred; the todo entry's `message` field is currently honoured only for `reword`.
- **TBD-904: Orphaned object growth.** Reset, rebase, branch delete and branch reset all orphan commits and never prune them (FR-NEW-255), which is what makes them recoverable. No `git gc` equivalent exists, so a volume that rebases frequently accumulates unreachable objects indefinitely. Quantifying the growth and deciding on a pruning tool is deferred to the backlog.
- **TBD-905: Cross-member operation etiquette.** FR-NEW-282 lets any project member abort another member's paused operation, on the grounds that the alternative is a volume that one absent person can block forever. Whether to additionally record and report who started the operation is deferred; the record's fields would support it.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Volume** | The simulated filesystem for one project, which is also the git working tree. There is no separate checkout. | Volume |
| **Mount id** | The identifier a caller uses to name a volume; required on every git tool. | Volume |
| **Index** | Git's staging area. This server **has none**; the term appears only to say so. | Git object store |
| **Working tree** | The files a commit would capture. Here, the volume itself. | Volume |
| **Dirty** | The volume differs from HEAD in any added, modified or deleted path. | Volume |
| **Combine operation** | Any operation that merges two histories and can therefore conflict: pull, merge, rebase, cherry-pick, revert, stash apply. | Git operation |
| **Conflict set** | The paths an in-progress operation could not auto-merge and is waiting on. | Git operation |
| **In-progress operation** | A combine operation paused awaiting resolution, recorded in `git_operations`. | Git operation |
| **Operation record** | The `git_operations` row holding that state. | Git operation |
| **Resolution** | A caller's answer for one conflicting path: `ours`, `theirs`, or literal content. | Git operation |
| **Ours / theirs** | `ours` is the side being merged into; `theirs` is the side being applied. For a rebase, `ours` is the new base. | Git operation |
| **Todo list** | The ordered pick/squash/drop/reword plan a rebase executes. | Git operation |
| **Step** | One todo entry; a rebase pauses at a step boundary, never mid-file. | Git operation |
| **Stash entry** | A commit under `refs/stash/*` capturing volume state, with the `base_sha` it was taken against. | Git object store |
| **Lease** | The `expected_remote_sha` a force push must supply; the push is refused if the remote has moved. | Remote |
| **Remote-tracking ref** | A ref under `refs/remotes/{remote}/*` mirroring a remote branch. | Remote |
| **Refspec** | A `local:remote` mapping; supported here for a single branch through `remote_branch`. | Remote |
| **Orphaned commit** | A commit no ref reaches, retained rather than pruned so the operation is recoverable. | Git object store |
| **Host map** | The `git.hosts` configuration mapping a hostname to a provider. | Identity and credentials |
| **Provider** | `github`, `gitlab`, `generic` or `anonymous`. Only the first two support pull requests. | Provider collaboration |
| **Instance URL** | The base URL of a self-hosted provider, used to reach GitHub Enterprise or self-hosted GitLab. | Provider collaboration |
| **Scope** | The permission set a token carries; validated before the network for PR operations. | Identity and credentials |
| **Normalized model** | The provider-independent pull-request shape returned by every `git.pr_*` tool. | Provider collaboration |
| **Pull request** | The provider's review artifact. Called a merge request on GitLab; normalized to one shape here. | Provider collaboration |
| **Write lock** | The per-project mutex serializing ref-mutating operations. | Volume |
| **Write quota** | The per-session byte budget charged before a volume write. | Volume |
| **Frozen tool contract** | `TOOL_CONTRACT.txt` and `tool-contract-golden.json`, machine-checked so a schema change is deliberate. | Volume |

## 17. Interview Decisions Log

- **DEC-901:** An in-progress operation is persisted as a row in a new relational `git_operations` table. **Rationale:** it is the only option where a paused rebase survives a server restart and is visible to every project member; the alternatives lose or hide it. **Alternatives considered:** (B) mirror real git's control files inside the bare repo at `state/git-repos/{project}/`, rejected because that directory is documented as a rebuildable cache rather than the source of truth, so a cache rebuild would destroy authoritative state; (C) in-memory session state like the read-before-write guard in `crates/mcp-fs/src/safety.rs`, rejected because a restart would evaporate a paused rebase and leave the volume half-applied with no way to continue or abort, which is data-integrity damage rather than inconvenience, and because it inherits the known BL-003/BL-014 scaling defect. **Implemented by:** FR-NEW-275, FR-NEW-276, FR-NEW-277, FR-NEW-278. **Round:** 2 (approach exploration). **Code evidence:** `crates/mcp-fs/src/git/db.rs:42` (three tables today), `crates/mcp-fs/src/git/repo.rs:44` (in-process lock), `crates/mcp-fs/src/safety.rs` (in-memory session pattern).

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

- **DEC-903:** Force push requires a mandatory lease (`expected_remote_sha`); a bare `force: true` is rejected. **Rationale:** the user asked for a "double check". A warning string in a tool description depends on a model reading it; a lease mechanically prevents overwriting a remote that moved since the caller last looked. This is stricter than git, where `--force` without a lease is allowed. **Alternatives considered:** a bare boolean plus a loud description (rejected as unenforceable); protected-branch pattern matching from the archived DEC-013 recommendation (not adopted as the primary mechanism, since the lease covers the actual race). **Implemented by:** FR-NEW-155, FR-NEW-156, FR-NEW-157, FR-NEW-158, FR-NEW-159. **Round:** 1 (Q3, P1). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:315` and `crates/mcp-fs/src/git/remote.rs:746` (force refused today).

- **DEC-904:** Continue and abort tools are per operation and named after the git CLI (`git.rebase_continue`, `git.merge_abort`) rather than a single generic trio. **Rationale:** the MCP client is an LLM chat behaving like a terminal, and tool names are its primary affordance; `git rebase --continue` is vocabulary it already has. The engine underneath is shared, so this costs surface area, not duplicated logic. **Alternatives considered:** one generic `git.operation_continue` / `git.operation_abort` / `git.conflict_resolve` trio, rejected for discoverability despite being fewer tools. **Implemented by:** FR-NEW-196, FR-NEW-197, FR-NEW-221, FR-NEW-223, FR-NEW-225, FR-NEW-239. **Round:** 2 (approach exploration, Fork 2). **Code evidence:** n/a, new surface.

- **DEC-905:** `git.reset` offers `soft` and `hard` only; there is no `mixed` mode. **Rationale:** `mixed` differs from `soft` only in what it does to the index, and this server has no index, so the two would be indistinguishable. Offering a third mode that silently behaves like the first would be a trap. **Alternatives considered:** accept `mixed` as an alias of `soft` for familiarity, rejected as misleading. **Implemented by:** FR-NEW-250, FR-NEW-251, FR-NEW-252. **Round:** 1 (P4). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:196-222` (commit takes the whole volume, no index).

- **DEC-906:** The pull request model is normalized across providers, with a `raw` passthrough field. **Rationale:** the tool contract is golden-file frozen, so it must describe one stable shape; a caller writes one handler for GitHub and GitLab. The `raw` field means normalization never loses provider-specific information. **Alternatives considered:** pass each provider's native JSON straight through, rejected because the frozen contract could not meaningfully describe two shapes under one tool name. **Implemented by:** FR-NEW-303. **Round:** 2 (approach exploration, Fork 3). **Code evidence:** n/a, new surface.

- **DEC-907:** Arbitrary named remotes are supported, reversing the archived spec's DEC-021 which deliberately limited the surface to a single `origin`. **Rationale:** a volume created by `git.init` has no `origin` and could therefore never push, fetch or pull; and fork-based workflows need a second remote. The storage already supports it (`git_remotes` is keyed by name); only the writer was limited. **Alternatives considered:** keep the single-remote limit, rejected because it blocks a standard development process, which is the point of this specification. **Implemented by:** FR-NEW-140 through FR-NEW-147, FR-MOD-101, FR-MOD-103. **Round:** 1 (Q7). **Code evidence:** `crates/mcp-fs/src/git/db.rs:69-75` (table already keyed by name), `:281-305` (add/remove/list already implemented and unused).

- **DEC-908:** The `on_conflict` parameter is removed from `git.remote_pull` outright rather than kept as a backward-compatible fast path. **Rationale:** user's explicit choice when offered both; one way to do things. **Alternatives considered:** keep it as a shorthand for a caller that has already decided, which would have been backward compatible and preserved the archived spec's tests. **Consequence accepted:** a breaking change to a shipped contract, and six shipped tests modified or removed. **Implemented by:** FR-DEL-101, FR-MOD-104, FR-MOD-105. **Round:** 4 (post-test-plan decision). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:353-378` (parameter), `:1447-1485` (`ConflictStrategy`, `parse_on_conflict`).

- **DEC-909:** A delete-versus-modify conflict, and a file-versus-directory type change, are surfaced as conflicts rather than refused up front with `ERR_INVALID_ARGUMENT`. **Rationale:** the shared model's premise is that combine operations surface rather than refuse; refusing would send the caller back to manual work for a case it can decide. **Alternatives considered:** up-front refusal, which the independent test designer flagged as the equally defensible branch. **Implemented by:** FR-NEW-180, FR-NEW-183. **Round:** 4 (raised by the test designer, resolved at test-plan review). **Code evidence:** n/a, new behaviour.

- **DEC-910:** The GitLab device flow requests the `api` scope in addition to its current scopes, and required scope is validated before the network call. **Rationale:** verified against GitLab's documentation, `write_repository` grants Git-over-HTTP access only and **no** REST API access, so every merge-request tool would be unauthorized; and failing late at the provider rather than early with a clear message contradicts the existing expired-token precedent. **Alternatives considered:** request nothing extra and let the provider reject, rejected as an unusable error experience. **Consequence accepted:** existing GitLab tokens must be re-granted. **Implemented by:** FR-NEW-330, FR-NEW-331, FR-NEW-332, FR-NEW-333, FR-MOD-109. **Round:** 3 (Q-A, resolved by research). **Code evidence:** `crates/mcp-fs/src/git/oauth/device_flow.rs:38` (GitHub `repo`), `:39` (GitLab `read_repository write_repository`), `crates/mcp-fs/src/git/oauth/persistence.rs:50` (scopes stored), `crates/mcp-fs/src/tools/git_auth.rs:396` (scopes reported, never validated).

- **DEC-911:** A token whose stored scope set is empty or unknown is attempted rather than pre-emptively refused, while a known-insufficient scope fails early. **Rationale:** `git.token_set` seeds PATs whose scopes the server cannot always enumerate; refusing them would make the tool useless for its main purpose. **Implemented by:** FR-NEW-334. **Round:** 3. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:128-159` (`git.token_set`).

- **DEC-912:** Purely local destructive operations (`git.reset --hard`, `git.branch_delete --force`, `git.branch_reset --force`) need no lease; the explicit mode or force flag is sufficient. **Rationale:** a lease protects against a concurrent actor moving the target between check and write, which is a real risk on a shared remote and not a risk on the caller's own volume. Recoverability is provided instead by reporting `old_sha` and by never pruning orphaned commits. **Alternatives considered:** apply the lease pattern locally too, rejected as ceremony with no corresponding hazard. **Implemented by:** FR-NEW-111, FR-NEW-251, FR-NEW-255. **Round:** 3 (Q-B). **Code evidence:** n/a, new behaviour.

- **DEC-913:** Stash entries are stored as refs under `refs/stash/*` in the existing `git_refs` table rather than in the new operation table or a table of their own. **Rationale:** it is how real git models a stash, it needs no schema change, and the namespace is verified unused. **Implemented by:** FR-NEW-120, FR-NEW-123. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/git/db.rs:60-68` (`git_refs`), and `refs/stash` has zero occurrences under `crates/`.

- **DEC-914:** Any project member can continue or abort an operation another member started. **Rationale:** authorization here is membership-based with no per-resource ownership, and the volume is shared; an operation only its author could clear would let one absent person block the whole project. **Implemented by:** FR-NEW-282. **Round:** 2. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:6-8` (membership-only authorization).

- **DEC-915:** Scope is held to one spec rather than split into four, and partitioned into stories afterwards by `/plan-spec`. **Rationale:** the user asked for full support with no shrinking, and this project's own pipeline already turns one large specification into a story sequence, as the archived spec's 19 stories in `specs/stories/` show. **Alternatives considered:** four separate specs by theme, which would have fragmented the shared conflict model across documents. **Implemented by:** n/a, process decision. **Round:** 1 (Q10). **Code evidence:** `specs/stories/_index.md`.

- **DEC-916:** Tests are numbered from `E2E-NEW-400` rather than from 001. **Rationale:** the archived specification already owns `E2E-NEW-001` through `E2E-NEW-247`, and 93 of those exist as live Rust test function names; reusing them would make traceability ambiguous between two specifications. **Implemented by:** n/a, numbering convention. **Round:** 4. **Code evidence:** 93 `async fn e2e_new_NNN` functions under `crates/`, maximum id 247.

- **DEC-917:** Test design was performed by four independent sub-agents with fresh context, none of which wrote the requirements. **Rationale:** the author of a requirement is the worst person to test it, because the same blind spot produces both. Splitting by theme also kept each designer's output reviewable. **Implemented by:** n/a, process decision. **Round:** 4.

- **DEC-918:** Rebase supports `pick`, `squash`, `drop` and `reword` only; `edit`, `fixup`, `exec` and `break` are excluded. **Rationale:** `exec` would execute arbitrary commands on the server, which is a sandbox escape, and `edit`/`break` require an interactive pause with no conflict to resolve, which the operation model does not represent. `fixup` is `squash` without message concatenation and adds little. **Implemented by:** FR-NEW-210 through FR-NEW-216. **Round:** 1 (Q9 scope, refined at requirement drafting).

## 18. Implementability Audit

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 7 | 5 | NOT-IMPLEMENTABLE |
| 2 | 6 | 1 | NOT-IMPLEMENTABLE |
| 3 | 4 | 2 | NOT-IMPLEMENTABLE |
| 4 | 4 | 0 | NOT-IMPLEMENTABLE (all four closed, plus the root cause) |

**Final state: no known F finding is open.** See "Round 4 and the decision to stop" below
for exactly what that claim rests on and what it does not.

**Round 1 findings.** Every F finding had one root cause: Sections 6 and 8 were authored
separately from Section 12, and the two disagreed on the JSON field names of the same
tools. Both were normative, so two implementing agents would have emitted different wire
shapes. Specifically: the conflict response had four incompatible shapes across the
document (F1); `current_step` was left explicitly undecided between 0-based and 1-based
(F2); `git_operations.state` admitted both `conflicted`/`running` and `paused` (F3); the
branch tools, `git.stash_list` and the normalized pull-request object each had two
contradicting key sets (F4, F5, F6); and the branch-name limit was 255 bytes in one place
and 255 characters in another (F7).

**Amendments applied:** added subsection 6.O with FR-NEW-186, FR-NEW-187, FR-NEW-188,
FR-NEW-189, FR-NEW-131, FR-NEW-317 and FR-NEW-116, each fixing one contradiction as a
normative requirement; corrected the superseded Outputs lines in Section 6 and the
sketches in Sections 8.1, 8.3 and 8.4 in place, so the document no longer contains the
contradiction at all; mapped the seven new requirements onto the 34 existing tests that
already assert those shapes.

**Evident drift corrected in place (not registered):** the REST route count (39, actually
38, `crates/mcp-fs/src/api/dataplane.rs:2328`); the OpenAPI git-path citation
(`openapi.rs:90-102`, actually `:74-76` with `git_paths()` at `:314`); the write-lock
call-site list (four sites, actually five, adding `tools/git.rs:1398`); and the claim
that all seven hardcoded count assertions must change. That last one was the most
consequential: `crates/mcp-fs/src/tools/all.rs:64` asserts the admin-only registry and
`:99` the git-disabled registry, so neither moves when git tools are added, and following
the original instruction would have broken two passing tests.



**Round 2 findings.** The same class as round 1, in sites round 1 did not reach: the wire
shapes settled in subsection 6.O were still contradicted inside individual Section 12 test
bodies. Conflict side objects were keyed `present`/`size` in seven bodies instead of
`exists`/`content` (F1); a `step: {current, total}` object and one-based step numbers
survived in four bodies against the zero-based top-level fields (F2); `git.stash_list` was
asserted with four keys, dropping the `base_sha` that FR-NEW-129 makes load-bearing (F3);
`git.stash_apply`/`git.stash_pop` had two different output key sets (F4); a `git.pr_list`
item was asserted as 12 keys in one body and 16 in another (F5); and the `op_type` of a
paused `git.stash_pop` was never decided, though the conflict response asserts
`operation == "stash_pop"` while Section 8.1 closed the set without it (F6).

**Amendments applied:** added FR-NEW-132 (one key set for stash apply, pop and drop) and
FR-NEW-285 (`op_type` as a six-value closed set including `stash_pop`); rewrote the
offending assertions in 21 places so no body contradicts subsection 6.O; amended the
Section 8.1 `op_type` row; mapped both new requirements onto existing tests.

**Drift corrected in place:** FR-NEW-116 justified its 255-byte limit by claiming
`git_refs.name` is byte-bounded. It is not: `TextKey(400)`
(`crates/mcp-fs/src/git/db.rs:22`) renders as unbounded `TEXT` on SQLite and PostgreSQL and
as `NVARCHAR(400)`, a character bound, on SQL Server
(`crates/mcp-fs/src/storage/rel/dialect.rs:218`, `:226`, `:235`). The 255-byte rule stands;
only its stated cause was wrong, and it now reads as a storage-ceiling argument.



**Round 3 findings.** The same wire-shape class again, in the last sites that still
contradicted subsection 6.O: one-based `current_step` values in five rebase bodies whose
own preconditions said the pause was at todo index 0 or 2 (F1); one body asserting both
`current_step == 0` and `current_step == null` for the same rebase, though only
single-step operations report null (F2); `op_type == "pull"` in two bodies, a value
outside the closed set of FR-NEW-285 (F3); and `ours.kind`/`theirs.kind` keys on a
type-change conflict, plus that body leaving the outcome to "the implementation must
pick" even though FR-NEW-183 had already decided a type change surfaces as a conflict
rather than an error (F4).

**Amendments applied:** corrected every step index to the zero-based value its own
preconditions imply; made the rebase body report integers rather than null; recorded a
paused pull as `op_type == "merge"`; rewrote the type-change assertion onto
`type_change` with `exists`/`content` side objects and removed the undecided branch.

**Drift corrected in place:** an assertion called `registry.list()`, a method
`ToolRegistry` does not have (its methods are `len`, `names`, `list_payload`,
`crates/mcp-fs/src/mcp/registry.rs:57,65,82`), and conflated the 94-tool frozen contract
with the larger enabled registry counted at `crates/mcp-fs/src/tools/all.rs:122`; and
DEC-913 cited `git/db.rs:51-57` for `git_refs`, which is `git_objects`, the correct
citation being `:60-68`.

**Verification beyond the audit.** After applying these, an exhaustive scripted sweep of
all 9,100 lines for sixteen prohibited patterns (`conflicting_paths`, `resolve_with`, a
`step` object, `.present`, `.kind`, `op_type == "pull"`, `state == "paused"`, the five
banned branch keys, `MAX_BRANCH_NAME_CHARS`, `operation_in_progress`, `registry.list()`,
and undecided-choice phrasings) returned **zero matches**. That sweep is exhaustive over
the whole document rather than sampled, which is why it is recorded here as evidence
alongside the audit rounds.



**Round 4 findings, and the decision to stop auditing.** Round 4 found four more
functional gaps, all the same class once again, in sites the round-3 sweep had not
covered: `git.merge`'s `status` enum disagreed between FR-NEW-192/193 (`up_to_date`,
`fast_forward`) and five test bodies (`already_up_to_date`, `merged` plus
`fast_forward: true`); `git.merge`'s Outputs line named `commit_sha` and `parents` while
every test read `merge_commit`, and `remaining_conflicts` was a count in two bodies and an
array everywhere else; `operation == "pull"` survived in the conflict response, because
round 3 had corrected only the `op_type == "pull"` form; and one abort assertion still read
`status == "ok"` or `"aborted"`, deciding nothing.

**Root cause, and the structural fix.** Four rounds each found fresh instances of one
defect: this document was assembled from six independently authored parts (four test
designers, a gap-filler, and the requirement author), and no single place pinned each
tool's response key set, so every part invented its own. Fixing instances was never going
to terminate. **FR-NEW-199 now pins the response key set and the closed `status` value set
for every one of the 31 new tools in a single normative table, and FR-NEW-286 does the same
for the `git.status` operation object.** Section 12 assertions were then corrected in bulk
against it: 18 `status == "ok"` assertions resolved to the closed value for their tool, 4
`result["commit_sha"]` reads changed to `new_sha`, and the enum, key-set and index
contradictions listed above rewritten.

**What the "no open F finding" claim rests on.** After those amendments, an exhaustive
scripted sweep of all 9,100 lines for sixteen prohibited patterns returned zero matches:
`conflicting_paths`, `resolve_with`, a `step` object, `step.current`, `.present`, `.kind`,
`op_type == "pull"`, `operation == "pull"`, `state == "paused"`, the five banned branch
keys, `MAX_BRANCH_NAME_CHARS`, `operation_in_progress`, `registry.list()`, "the
implementation must pick", `result["commit_sha"]`, `status == "ok"`, and
`remaining_conflicts == 0`. That sweep is exhaustive over the document rather than sampled,
which is stronger evidence than a further sampled audit for the patterns it covers.

**What it does not rest on.** No fifth audit was run. The sweep can only prove the absence
of patterns already known to be wrong; it cannot discover a new class. Given that rounds 1
through 4 each surfaced the same class in new sites, the residual risk is that a fifth
round would find further instances. The judgement recorded here is that FR-NEW-199 removes
the *cause* rather than another symptom, and that the remaining risk is better discharged
by the compiler and the test suite during implementation than by a fifth document review.
The first implementation story SHOULD begin by generating the response structs directly
from the FR-NEW-199 table, so any surviving disagreement in a test body fails to compile
rather than shipping.

**Drift registered:** DRIFT-001 (the pull path has no conflict-success response to reuse;
the conflict reporting is new code, not an extraction).


## 19. Implementation Drift Register

> Spec/code alignment drift found at audit. The behaviour specified is correct; the
> statement about the current code was not, or the capability the requirement needs does
> not exist yet. **These are resolved DURING the implementation of this spec, not later.**
> Every entry must be closed before the implementation branch merges. An entry still open
> at merge time moves to `specs/BACKLOG.md` with its evidence, and that move is a decision
> someone signs, not a silence.
>
> Four further drift findings from audit round 1 (a wrong REST route count, a wrong
> OpenAPI citation, a wrong write-lock call-site list, and an over-broad claim about which
> count assertions change) were evident corrections and have been amended in place rather
> than registered here.

#### DRIFT-001: The pull path has no conflict-success response to copy
- **Spec says:** every combine operation returns `status: "conflict"` with both sides' content and applies nothing (FR-NEW-170, FR-NEW-186), and Section 12's stash tests describe this as "mirroring the pull conflict model".
- **Code does:** the current pull has no such response. When libgit2's merge still has conflicts, it returns an error: `ToolError::internal("git.remote_pull: the merge left unresolved conflicts that 'ours'/'theirs' cannot represent")` at `crates/mcp-fs/src/tools/git.rs:1844-1849`. There is no `status: "conflict"` success path anywhere in the tree.
- **Nature:** missing capability.
- **Resolution during implementation:** build the conflict-success response once, as new code, per FR-NEW-186, and route all six combine operations through it. Do not treat `git.rs:1844` as a template: it is the error path being replaced. The three-way merge itself (`git.rs:1835-1839`) and `apply_pull_changes_atomically` (`:2079-2127`) are the reusable parts; the conflict reporting is not.
- **Detected by:** `E2E-NEW-462` and every conflict test in band B fail against the current code, which returns `ERR_INTERNAL_ERROR` where the spec requires an `Ok` response carrying `status: "conflict"`.
- **Blocks which requirement:** FR-NEW-170, FR-NEW-186, and every requirement depending on the conflict model.
- **Status:** resolved (`e2e_new_462_a_conflicting_stash_apply_writes_nothing`, green in the suite at commit e29e60e; the conflict-success response was built as new code in `crates/mcp-fs/src/git/merge.rs` as `ConflictResponse`, and the error path this entry warned against reusing is gone from the tree, 0 occurrences)

