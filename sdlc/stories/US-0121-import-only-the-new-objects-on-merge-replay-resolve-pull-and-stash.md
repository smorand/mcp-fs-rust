# US-0121: import only the new objects on merge, replay, resolve, pull and stash

> Spec: SPEC-0062 (sdlc/specs/SPEC-0062-import-reachable-objects/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0120
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Stop the six remaining bounded write paths (merge, rebase landing and pause, merge_resolve, the pull's merge commit, stash) from copying the whole repository into the blob store.

## Requirements covered
- SPEC-0062/DR-002 (partial: the six other sites): Each of the seven bounded write paths SHALL call `entry.objects.import_reachable(&repo, &[<tip>])` where it called `import_from_repo`, and SHALL NOT call `import_from_repo`. New calls (`crates/core/src/tools/git.rs`): `merge_inner` clean arm `:4262` `import_reachable(&repo, &[oid])`; `land_replay` `:4891` `import_reachable(repo, &[new_tip])`; `pause_replay` `:4928` `import_reachable(repo, &[replay_tip])`; `merge_resolve_inner` `:5632` `import_reachable(&repo, &[oid])`; `pull_branch_inner` merge commit `:6935` `entry_for_thread.objects.import_reachable(&repo, &[merge_oid])`; `stash_save` `:7178` `import_reachable(&repo, &[oid])`. The ordering rule of each site is unchanged: the objects are in the blob store before the ref row names them.
- SPEC-0062/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green. For each site the new test asserts the closure of the new tip is in the store (non mock: it walks libgit2 and probes the store) and, for five of them, that the store did not grow by the strays. Then restore `import_from_repo` at each site (one at a time) and see its test fail; for the pull site delete the call and see the closure assertion fail. Revert each.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0062): two repairs of the whole walk are dropped on purpose for the seven sites (objects no new tip reaches; a missing index row for an existing blob). Rebuilds of libgit2 come from the store, so such objects were already lost on rebuild.
- DEC-010: `assert_closure_in_store` (written in US-0120) is the one definition of "the whole tip is stored".
- Assumption (SPEC-0062): `pause_replay` needs nothing beyond what is reachable from `replay_tip`; the operation row stores ids of stored commits and conflict paths, not object ids (`tools/git.rs:4938-4955`).
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No tool schema or output changes for `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.merge_resolve`, `git.remote_pull`, `git.stash_save`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (production: six call sites; tests module: 6 tests) |
### Existing patterns
Current code at each site is the single line `entry.objects.import_from_repo(&repo).await?;` (`land_replay` and `pause_replay` pass `repo` already a `&Repository`: `entry.objects.import_from_repo(repo).await?;`; the pull site is `entry_for_thread.objects.import_from_repo(&repo).await?;` inside `on_git_thread`). Replace as in the requirement. Keep the pull's fetch step call at `:6803` and clone/fetch at `:3324`, `:3918` untouched. Test patterns to copy: merge clean `e2e_new_` merge tests with two branches changing different files (search `git.merge` clean tests in `tools/git.rs`); pull with a diverged branch `e2e_new_940_a_divergent_pull_with_disjoint_changes_merges` (`tools/git.rs:12151`); a paused rebase on a conflict (search the rebase conflict tests, around `:11665`); `git.stash_save` tests; `git.merge_resolve` completing tests. Strays: lock `entry.repo`, `hydrate`, then `repo.blob(format!("blob {i}").as_bytes())` for i in 0..2000 right before the tool call (for the pull test, no strays: its fetch step imports everything).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0062/DT-012: `git.merge` clean
- Level / Driver / Location: integration / `Env` harness / `tools/git.rs` `mod tests`
- Given two branches changing different files, 2 000 unimported strays and `before = entry.objects.list().await.unwrap().len()`
- When `git.merge` completes cleanly
- Then `assert_closure_in_store` holds for the merge commit and the store length is below `before + 200`
- Red before the fix because (mutation): restore `import_from_repo` at `:4262`; the length is above `before + 2000`.

### SPEC-0062/DT-013: a rebase of two commits landing
- Same Given/Then as DT-012 for `land_replay` (`:4891`): the closure of the new branch tip holds and the length is below `before + 200`. Mutation: restore `import_from_repo` at `:4891`.

### SPEC-0062/DT-014: a rebase pausing on a conflict
- Same Given/Then for `pause_replay` (`:4928`): the closure of the replay ref tip (`REPLAY_HEAD_REF`) holds and the length is below `before + 200`. Mutation: restore `import_from_repo` at `:4928`.

### SPEC-0062/DT-015: `git.merge_resolve` completing with a merge commit
- Same Given/Then for `merge_resolve_inner` (`:5632`): the closure of the merge commit holds and the length is below `before + 200`. Mutation: restore `import_from_repo` at `:5632`.

### SPEC-0062/DT-016: `git.remote_pull` producing a merge commit
- Level / Driver / Location: integration / `Env` harness / `tools/git.rs` `mod tests`, pattern `e2e_new_940` (`:12151`)
- Given a diverged branch with disjoint changes against a file remote
- When `git.remote_pull` merges
- Then `assert_closure_in_store` holds for the merge commit. No store length bound applies: the fetch step of the same pull imports every object (`:6803`, unchanged) and nothing can inject strays between the two steps.
- Red before the fix because (mutation): delete the import call at `:6935`; the closure assertion fails on the merge commit's tree or blob.

### SPEC-0062/DT-017: `git.stash_save`
- Same Given/Then as DT-012 for `stash_save` (`:7178`): the closure of the stash commit holds and the length is below `before + 200`. Mutation: restore `import_from_repo` at `:7178`.

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; the four pack sites (`git/http/mod.rs:572`, `tools/git.rs:3324,3918,6803`).
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion; changing a site's ordering (objects first, then the ref row).
- Scope boundary: the structural check and the measurement are US-0122.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(new objects) imported per call instead of O(repository objects)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted one by one: `git diff main -- crates/core/src/tools/git.rs` shows only the six real production lines and the tests.
- [ ] One commit `US-0121: import only the new objects on merge, replay, resolve, pull and stash`, body `Spec: SPEC-0062` and the requirement ids.
