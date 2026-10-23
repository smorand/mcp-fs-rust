# Git divergence review (BL-0027)

> Generated: 2026-10-24
> Request: BL-0027, review the consolidated register of divergences from standard git
> Deliverable: non-code

## Summary
Build the register BL-0027 asks for: every way the server's git behaviour differs from ordinary git, each row cited to code and justified. Rank the gaps by user impact and decide with the user which to close. Each chosen gap becomes its own backlog entry for `/sdlc-spec`. No behaviour changes here.

## Actions
- [x] 1. Verify each of the 9 listed divergences plus the 2 asymmetries against the code and record `file:LINE` per row (no index and whole-volume commits; pull refuses a dirty tree, `tools/git.rs:732,834,936`; HTTPS only; credentials in URL rejected, `git/remote.rs:243,265`; unknown host error; token as password with `oauth2`, `git/remote.rs:388`; no refresh or rotation, `git/oauth`; no submodules, `tools/git.rs:7331`; clone tolerates failure and reports `skipped` while pull is atomic, `tools/git.rs:3165`); verify: every row has a citation opened and read, none reads ASSUMED.
- [x] 2. Check the BL's "closed since" list (reset, rebase, cherry-pick, revert, stash, force push with lease, remotes, pull requests) against the tool list, and look for divergences the entry missed (for example SSH, shallow clone, hooks, tags, LFS); verify: the 39 `git.*` tool names from `TOOL_CONTRACT.txt` are each accounted for in rows or in "closed".
- [x] 3. For each row, write: what standard git does, what this server does, why (cite the originating spec or design), and the cost to a user; verify: no row lacks a "why" or an origin reference.
- [x] 4. Write the register to `.agent_docs/git-divergences.md` (under 200 lines, no dashes as punctuation) and add one line to the index in `AGENTS.md` and a pointer in `.agent_docs/git.md` ("Divergences from the reference" section); verify: `wc -l` under 200, `rg git-divergences AGENTS.md .agent_docs/git.md` finds both pointers.
- [x] 5. Add a coherence section: group rows into themes (working tree model, transport and auth, atomicity), and name contradictions or gaps; verify: each row belongs to exactly one theme.
- [x] 6. Present the ranked gap list to the user in chat with a recommendation and default per gap (close or keep as designed); verify: the user has answered or the defaults are recorded in the register.
- [x] 7. For each gap to close, create one `sdlc/backlog/BL-NNNN-<slug>.md` (next id = max of disk and `git log --all --name-only -- sdlc/backlog`, plus 1) with `Nature`, `What`, `Why`, `Evidence`; verify: `ls sdlc/backlog` shows them, ids unique.
- [x] 8. Delete `sdlc/backlog/BL-0027-*.md` with `git rm` and commit register, new entries and deletion together on a branch, then merge; verify: `git status --porcelain` empty, `git log -1 --stat` shows all three kinds of change.

## Assumptions & defaults
- The register lives in `.agent_docs/` (project convention), not `sdlc/specs/`, since it specifies no behaviour.
- Divergences marked "justified" stay as designed unless the user says otherwise at step 6.
- Divergences in `.agent_docs/parity.md` (vs the C# lineage) are a different set and out of scope; only the BL's own ones count.

## Test plan / verification
Documentation only, no tests. `cargo` gates are untouched. Checks are the per step verifies above, plus a final reread that every citation still resolves (`sed -n LINEp file`).

## Open risks
- Line numbers drift; rows cite function names too.
- Step 6 depends on the user's time; if unanswered, defaults apply and no backlog entries are created for "keep".
