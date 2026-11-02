# SPEC-0075: Which branch's tags a fetch receives, design

> Spec: sdlc/specs/SPEC-0075-which-branch-tags-a-fetch-receives/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-02
> Target tier: 2 (default)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust.

## 2. Approaches
### Chosen: keep the rule, test it, fix only what a test shows
The rule exists (`git.rs:3894-3918`, `:6811`). Two cases are unverified assumptions: a detached HEAD (E2E-011) and a remote with no default branch (E2E-015). Tests first; a failing one is fixed in the same story by the smallest change (DEC-003). Pros: no speculative code. Cons: the story may grow by one small fix.
### Rejected: receive tags of every remote branch
Rejected by the owner (BL-0099 decision).

## 3. Architecture
Fetch reads the symbolic `HEAD` row: a symbolic ref to `refs/heads/<b>` gives `TagScope::Branch(b)`, anything else `TagScope::RemoteDefault` (`git.rs:3894-3918`); pull passes `TagScope::Branch(<pulled branch>)` (`:6811`). `fetch_from_remote` takes the tip of that branch from `refs/remotes/<remote>/<b>` (`remote.rs:1370-1381`) and keeps the tags whose commit is in its history (`history_tags`).

## 4. Data model
None.

## 5. Contracts and interfaces
No change. Helpers: `detach_head(e, sha)` sets the project's `HEAD` row non symbolic (`entry.db.set_ref("HEAD", sha, false)`); a bare remote with an unborn default branch is made with `Repository::set_head("refs/heads/nope")` on the bare repository.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | modify (fifteen tests, two helpers; possibly one fix in the fetch path if E2E-011 or E2E-015 fails) | all | US-0152, US-0153 |
| `.agent_docs/git.md` | modify (one paragraph: which branch decides) | FR-NEW-001, FR-NEW-002 | US-0153 |
Affected existing tests: keep all (`e2e_tag_041`, `042`, `043` stay).

## 7. Compatibility and risks
No planned behavior change. Risk: E2E-011 or E2E-015 fails on arrival (signal); then the fix is limited to making the fetch behave as FR-NEW-001 says.

## 8. Security design
Not applicable.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, 002, 003, 008, 009, 010, 011, 012, 015 | functional e2e | direct tool call (`call_fetch_branch`, `create_branch_on_bare_remote`, `plain_tag_bare_remote`, `annotated_tag_bare_remote`, `tag_list`) | `crates/core/src/tools/git.rs` tests | US-0152 |
| E2E-004, 005, 006, 007, 013, 014 | functional e2e | direct tool call (`call_pull_branch`, same helpers) | `crates/core/src/tools/git.rs` tests | US-0153 |
Test data: generated bare remotes with `main` and `experimental`.

## 10. Decisions log
- **DEC-001:** no production change planned. **Obliged by:** SPEC-0075/FR-NEW-001, SPEC-0075/FR-NEW-002. **Rationale:** the code follows SPEC-0052 FR-NEW-009. **Alternatives considered:** widening the rule (rejected by the owner). **Evidence:** `git.rs:3894-3918`.
- **DEC-002:** a detached position is simulated by writing the `HEAD` row non symbolic, not through a tool. **Obliged by:** SPEC-0075/FR-NEW-001 (E2E-011). **Rationale:** the product never leaves HEAD non symbolic itself (ASSUMED: no tool does; the story confirms with `rg 'set_ref\("HEAD"'`). **Source:** existing code `git.rs:3894`.
- **DEC-003:** if E2E-011 or E2E-015 fails on arrival, the fix goes in the same story and is the smallest that satisfies the spec. **Obliged by:** SPEC-0075/FR-NEW-001. **Source:** design.
- **DEC-004:** two stories: US-0152 (fetch, nine tests), US-0153 (pull and the rest, six tests, docs, closes the spec). **Obliged by:** n/a (no code impact): partitioning only. **Rationale:** SC-001 and SC-002 are fetch; SC-003 is pull plus the later fetch. **Source:** design.
- **DEC-005:** red evidence by mutation (tests pass on arrival): in the fetch path force `TagScope::RemoteDefault` always (E2E-002, 004 fail); force `TagScope::Off` (E2E-001 fails); in pull force `Off` (E2E-005 fails). **Obliged by:** SPEC-0075/FR-NEW-001, FR-NEW-002. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0152 | pin the branch that decides the tags of a fetch | P1 | FR-NEW-001 | US-0151 | 2 | no |
| US-0153 | pin the branch that decides the tags of a pull | P1 | FR-NEW-002 | US-0152 | 2 | yes |
