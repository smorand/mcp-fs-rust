# US-0124: import only the new objects of a fetch and a pull

> Spec: SPEC-0063 (sdlc/specs/SPEC-0063-import-new-objects-of-packs/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0123
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Stop `git.remote_fetch` and the fetch step of `git.remote_pull` from copying the whole repository into the blob store. Measured before: 276 to 288 ms at 5 000 objects and 1 132 to 1 184 ms at 20 000, against 31 to 35 ms at 500.

## Requirements covered
- SPEC-0063/DR-002 (partial: fetch and pull): Push receive, fetch and the fetch step of pull SHALL call `import_tips` with the tips their pack brought, where they called `import_from_repo`.
  - `fetch_branch_inner` (`crates/core/src/tools/git.rs:3920`): `entry_for_thread.objects.import_tips(&repo, &fetch_tips(&outcome)).await?;`. The fetch step of `pull_branch_inner` (`:6805`): the same call. `fetch_tips(outcome: &FetchOutcome) -> Vec<Oid>` returns, in this order, the parsed `new_sha` of every `outcome.refs_updated` entry, then the `oid` of every `outcome.incoming_tags` entry; an entry whose `new_sha` does not parse as an object id is skipped.
  - The ordering rule of each site is unchanged: the objects are in the blob store before the ref rows or tag rows name them; `reconcile_incoming_tags` still runs after the import (`:3921-3929`, `:6806-6815`).
- SPEC-0063/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green. DT-005 is the non mock assertion: after a fetch of one new remote commit the store grew by under 200 although 2 000 unrelated objects sit in libgit2, and the new tip's closure is in the store. Then apply the mutations of the tests (restore `import_from_repo` at each site; leave the tag oids out of `fetch_tips`), see each fail, and revert with `git checkout -- crates/core/src/tools/git.rs` after saving your real change.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0063): the tips of a fetch are the updated refs' new ids plus the incoming tag oids, not every ref of the repository (one store probe per ref would make a project with 20 000 tags pay 20 000 probes per fetch); `reconcile_incoming_tags` calls `set_ref` for each incoming tag, so every tag oid must be in the store first (`tools/git_tags.rs:428-459`).
- DEC-004: a repair of the whole walk is dropped on purpose: objects a pack carried that no new tip reaches are no longer imported.
- DEC-006: one private helper `fetch_tips` shared by fetch and pull.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No tool schema or output changes for `git.remote_fetch` and `git.remote_pull`. Types used (crates/core/src/git/remote.rs): `FetchOutcome { refs_updated: Vec<FetchRefUpdate>, incoming_tags: Vec<IncomingTag>, .. }`, `FetchRefUpdate { ref_name, old_sha: Option<String>, new_sha: String }`, `IncomingTag { name, oid: git2::Oid, annotated }`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (production: helper `fetch_tips` and the calls at `:3920` and `:6805`; tests module, starts at line 7657: 3 tests) |
### Existing patterns
Current code at both sites is `entry_for_thread.objects.import_from_repo(&repo).await?;` right after `fetch_from_remote(...)` returned `outcome` and before `reconcile_incoming_tags(...)`. Helper to add near `fetch_branch_inner`:
```rust
/// The objects a fetch brought: the new tip of every updated remote ref and every incoming tag object.
fn fetch_tips(outcome: &crate::git::remote::FetchOutcome) -> Vec<Oid> {
    outcome.refs_updated.iter().filter_map(|u| Oid::from_str(&u.new_sha).ok())
        .chain(outcome.incoming_tags.iter().map(|t| t.oid)).collect()
}
```
(`Oid` is `git2::Oid`; check what `tools/git.rs` already imports.) Test helpers in `tools/git.rs` `mod tests`: `plant_strays(&entry) -> usize` (writes 2 000 unimported blobs into libgit2 and returns the store length), `assert_closure_in_store(&entry, oid)`, `cloned_remote(&e, "label") -> (dir, url, tip)`, `advance_bare_remote(&dir, "refs/heads/main", "payload\n") -> String`, `annotated_tag_bare_remote(&dir, "v1.0", &commit, "m")`, `call_fetch_branch(&e, &url, "anonymous")`, `seed_pull_disjoint(&e, "label")`, `call_pull_branch(...)` (see `e2e_new_940_a_divergent_pull_with_disjoint_changes_merges`, `tools/git.rs:12151`, and `e2e_tag_501_annotated_tags_span_two_batches` for the fetch setup). Place the new tests next to `e2e_pull_062_merge_commit_closure_is_stored` (SPEC-0062).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0063/DT-005: fetch of one new commit among strays
- Level / Driver / Location: integration / `Env` harness, `git.remote_fetch` / `tools/git.rs` `mod tests`
- Given a project cloned from a bare remote (`cloned_remote`), then `before = plant_strays(&entry).await`
- When the remote gains a commit (`advance_bare_remote`) and `call_fetch_branch` succeeds
- Then the store length is below `before + 200` and `assert_closure_in_store` holds for the remote's new tip
- Red before the fix because (mutation): restore `import_from_repo` at `tools/git.rs:3920`; the length is above `before + 2000`.

### SPEC-0063/DT-006: fetch with an annotated tag
- Level / Driver / Location: integration / `Env` harness / `tools/git.rs` `mod tests`
- Given the setup of DT-005 and an annotated tag `v1.0` (message `m`) on the new remote commit (`annotated_tag_bare_remote`)
- When `call_fetch_branch` succeeds
- Then `tags_added` lists `v1.0`, `entry.objects.exists` holds for the tag object (the id in the result or `repo.refname_to_id("refs/tags/v1.0")`), and the store length is below `before + 200`
- Red before the fix because (mutation): leave the incoming tag oids out of `fetch_tips`; the tag object is missing from the store (or the reconcile fails).

### SPEC-0063/DT-007: pull with a diverged branch
- Level / Driver / Location: integration / `Env` harness, `git.remote_pull` / `tools/git.rs` `mod tests`
- Given `seed_pull_disjoint` and `before = plant_strays(&entry).await` just before the call
- When `git.remote_pull` merges
- Then `assert_closure_in_store` holds for the merge commit and the store length is below `before + 200`
- Red before the fix because (mutation): restore `import_from_repo` at `tools/git.rs:6805`; the length is above `before + 2000`.

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; `git/http/mod.rs`; the clone site (`:3326`) and the seven `import_reachable` sites of SPEC-0062.
- Dependencies not to add: none.
- Patterns to avoid: passing every ref of the repository as a tip (O(refs) probes); a timing assertion; moving `reconcile_incoming_tags` before the import.
- Scope boundary: push receive is US-0125.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(new objects + updated refs + incoming tags) per call instead of O(repository objects)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted: `git diff main -- crates/core/src/tools/git.rs` shows only the helper, the two real production lines and the tests.
- [ ] One commit `US-0124: import only the new objects of a fetch and a pull`, body `Spec: SPEC-0063` and the requirement ids.
