# Git divergences from standard git

Created 2026-10-24 (BL-0027). Last updated 2026-10-24: D15 closed by SPEC-0052. Register of the ways `git.*` differs from ordinary git, so the set
can be judged as a whole. Not to be confused with `lineage.md`, which covers the C#
lineage. Citations are `file:LINE` at the date above; line numbers drift, the symbol is the anchor.
Paths are under `crates/core/src/` unless stated.

## Model: the volume is the working tree

| # | Standard git | This server | Why | Cost to a user | Anchor |
|---|---|---|---|---|---|
| D1 | A staging index; `git add` picks what is committed | No index. The volume is the working tree | The volume is the single source of truth, there is no second copy to stage from | Cannot commit a subset of changes | `tools/git.rs:2121` `commit`, `tools/git.rs:7287` |
| D2 | A commit holds the staged paths | A commit snapshots every file of the volume | Follows from D1 | Unrelated edits ride along in a commit | `tools/git.rs:7287` snapshot, `build_tree_from_volume` |
| D3 | `reset --mixed` exists | `git.reset` offers `soft` and `hard` only | With no index `mixed` equals `soft` (SPEC-0012 DEC-005) | None | `sdlc/specs/SPEC-0012-full-git-dev-process/design.md:90` |
| D4 | File modes are kept | Every file committed as a non executable regular file | The volume stores no mode bits | Executable bit and symlinks are lost on a round trip | `tools/git.rs:43` |
| D5 | `.git/hooks` run | The repository is a bare libgit2 directory, hooks never run | Server side, no process to run them | No pre commit or pre push checks | `git/repo.rs:8` |

## Working tree safety

| # | Standard git | This server | Why | Cost | Anchor |
|---|---|---|---|---|---|
| D6 | Pull, merge, rebase, cherry pick, revert can run on a dirty tree when no path overlaps | Each refuses a dirty volume ("commit or stash first") | One atomic apply over a clean base keeps quota and rollback simple | Must commit or stash before every integration | `tools/git.rs:732,834,936` |
| D7 | Pull and clone write file by file | Pull is atomic (pre check of every target, all or nothing); clone tolerates a per file failure and reports `skipped` | Pull has a base to protect; clone fills an empty volume and partial progress beats none | The same word "sync" has two failure contracts | `git/merge.rs:729` `apply_changes_atomically`, `tools/git.rs:3165` |
| D8 | A conflict is an error state in the tree | A conflict is an `Ok` response with `status: "conflict"` | Lets a client branch on data, not on an error code | Callers must check the field | `git/merge.rs` response types |
| D9 | Submodules are first class | Commit entries are skipped, no bytes to copy | No nested repo store | Submodule content never appears in the volume | `tools/git.rs:7331` |

## Transport and authentication

| # | Standard git | This server | Why | Cost | Anchor |
|---|---|---|---|---|---|
| D10 | https, ssh, git, file | HTTPS only; scp shorthand and other schemes are rejected | One credential path, no key storage | Hosts reachable only by SSH are out | `git/remote.rs:242,256` |
| D11 | `https://user:token@host` works | A URL carrying userinfo is rejected, the token comes from the store | A secret in a URL leaks into logs and config | Must run `git.auth` or `git.token_set` first | `git/remote.rs:243,265` |
| D12 | Any host | A host must be declared in `git.hosts`, else an error (never anonymous downgrade) | A silent downgrade hides a missing credential | Operator edit needed for each new host | `git/remote.rs:181,186` |
| D13 | Many credential helpers and shapes | One shape: the token as password, username `oauth2` | Works on GitHub, GitHub Enterprise and GitLab (SPEC-0011 DEC-109) | A host wanting another shape is unsupported | `git/remote.rs:388` |
| D14 | Helpers can refresh | No token refresh or rotation; `expires_at` is stored, not acted on | Non goal of SPEC-0011 | An expired token needs a manual re auth | `sdlc/specs/SPEC-0011-github-enterprise-and-token-store/design.md:154`, `git/oauth/persistence.rs:47` |

## Refs and history

| # | Standard git | This server | Why | Cost | Anchor |
|---|---|---|---|---|---|
| D15 | Tags are created, deleted, pushed and fetched | **Closed by SPEC-0052 (2026-10-24)** except: remote tag deletion was delivered by SPEC-0053 (`git.remote_delete_tag`, BL-0075, 2026-10-25) and publishing every tag was delivered by SPEC-0054 (`git.remote_push_tags`, BL-0076, 2026-10-27). Fetch and pull receive only the tags in the fetched branch history, clone those of the cloned branch; a differing local tag is kept, never replaced | Auto tag following stays off (`AutotagOption::None`); candidates come from the advertised refs, so fetch still touches no ref outside the asked branch and its history tags | None left for tags | `tools/git_tags.rs`, `git/remote.rs` `history_tags` |
| D16 | Fetch with any refspec | Fetch works on one branch, tag following off, pruning forced off | Bounded, predictable ref updates | Cannot mirror a remote | `git/remote.rs:686` |
| D17 | `stash` can include untracked files | Stash saves "uncommitted changes" of the volume and refuses when there are none | Follows D1 | No untracked versus tracked distinction | `tools/git.rs:605` |

## Closed since BL-0027 was written (SPEC-0012)

Per file conflict resolution, rebase, cherry pick, revert, stash, `git.reset` (hard covers discard),
force push with lease, `remote_branch` on push, remote add, list and remove, remotes beyond
`origin`, and pull requests. The tools registered in `mcp/server.rs` under `git.*` were listed and none adds a divergence
beyond the rows above. Since SPEC-0052 there are 52 (42 `git.*`, 4 `git.auth*`, 6 `git.pr_*`);
the tag tools close D15 rather than add a row.

## Coherence

Three themes.
1. **Working tree model (D1 to D5, D17).** D1 explains D2, D3 and D17: one decision, consistent.
   D4 and D5 are independent losses.
2. **Safety and atomicity (D6 to D9).** D6 and D7 disagree in spirit: integration is strict and
   atomic, bootstrap is lenient. Coherent only if clone is treated as "fill an empty volume".
3. **Transport and credentials (D10 to D16).** Consistent: HTTPS plus one token. D15 was the
   outlier (a missing capability), closed by SPEC-0052.

## Gaps ranked (recommendation, default if not answered)

| Rank | Gap | Recommendation | Default |
|---|---|---|---|
| 1 | D14 no token refresh | Close only if a supported host issues expiring tokens; confirm first | Keep |
| 2 | D15 fully closed: remote tag delete by SPEC-0053, publish all tags by SPEC-0054 (2026-10-27) | Nothing left | Closed |
| 3 | D6 dirty tree refusal | Keep: the atomic apply depends on it | Keep |
| 4 | D4 file modes | Keep until a user reports an executable bit loss | Keep |
| 5 | D10 SSH | Keep: key storage is a security surface | Keep |
| 6 | D1, D2 no index | Keep: it is the model | Keep |
| 7 | D7 clone versus pull asymmetry | Keep, but document `skipped` in the clone contract text | Keep |
| 8 | D5, D9 hooks, submodules | Keep | Keep |

D15 tags (create, delete, follow on fetch), formerly rank 1, closed by SPEC-0052 (2026-10-24).
