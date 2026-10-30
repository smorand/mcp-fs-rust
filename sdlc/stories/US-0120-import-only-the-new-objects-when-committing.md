# US-0120: import only the new objects when committing

> Spec: SPEC-0062 (sdlc/specs/SPEC-0062-import-reachable-objects/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0119
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Stop `git.commit` from copying the whole repository into the blob store. Measured before: 265 to 273 ms at 5 000 objects and 1 099 to 1 173 ms at 20 000, against 32 to 39 ms at 500. This story also writes the closure helper the next story reuses.

## Requirements covered
- SPEC-0062/DR-002 (partial: the `commit` site): Each of the seven bounded write paths SHALL call `entry.objects.import_reachable(&repo, &[<tip>])` where it called `import_from_repo`, and SHALL NOT call `import_from_repo`. For `commit` (`crates/core/src/tools/git.rs:2323`): `entry.objects.import_reachable(&repo, &[oid]).await?;` where `oid` is the commit just created. The ordering rule of the site is unchanged: the objects are in the blob store before the ref row names them.
- SPEC-0062/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green. DT-011 is the non mock assertion: the store grows by exactly 3 (blob, tree, commit) although 2 000 unrelated objects sit in libgit2. Then restore `import_from_repo` at `git.rs:2323` (mutation), see DT-011 fail with `before + 2003`, and revert with `git checkout -- crates/core/src/tools/git.rs` after saving your real change.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0062): two repairs of the whole walk are dropped on purpose for the seven sites: storing objects libgit2 holds that no new tip reaches, and recording a missing index row for an object whose blob exists. The libgit2 repository is a cache rebuilt from the store (`hydrate`, `tools/git.rs:2069`), so an object the store lacks is lost at the next rebuild whichever tool created it.
- DEC-010: the closure helper `assert_closure_in_store(entry, tip)` is written here and reused by US-0121.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No tool schema or output changes. `git.commit` keeps returning its `commit_sha` and the other fields as today.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (production: `commit`, line 2323; tests module, starts at line 7655: the helper and 1 test) |
### Existing patterns
Current code (`crates/core/src/tools/git.rs:2315-2326`):
```rust
let oid = repo
    .commit(None, &sig, &sig, &pretty, &tree, &parents)
    .map_err(|e| git_err("create commit", e))?;
let sha = oid.to_string();
// Persist everything libgit2 just wrote (blobs, trees, the commit) into the
// blob store ... 
entry.objects.import_from_repo(&repo).await?;
```
Change that call to `entry.objects.import_reachable(&repo, &[oid]).await?;` and adjust the comment (only what the new commit reaches). Test pattern: `e2e_tag_028_annotated_tag_imports_only_its_own_object` (`tools/git.rs`, near `e2e_tag_026_delete_an_annotated_tag_reports_the_commit`): `Env::new()`, `seed_tags(&e)`, `e.entry().await`, lock `entry.repo`, `hydrate(&entry, &repo)`, write strays with `repo.blob(format!("blob {i}").as_bytes())`, `entry.objects.list().await.unwrap().len()`. Closure helper shape:
```rust
async fn assert_closure_in_store(entry: &GitRepoEntry, tip: git2::Oid) {
    let repo = entry.repo.lock().await;
    let mut walk = repo.revwalk().unwrap();
    walk.push(tip).unwrap();
    for c in walk {
        let commit = repo.find_commit(c.unwrap()).unwrap();
        assert!(entry.objects.exists(&commit.id().to_string()).await.unwrap(), "commit");
        let tree = commit.tree().unwrap();
        let mut ids = vec![tree.id()];
        tree.walk(git2::TreeWalkMode::PreOrder, |_, e| { ids.push(e.id()); 0 }).unwrap();
        for id in ids { assert!(entry.objects.exists(&id.to_string()).await.unwrap(), "{id}"); }
    }
}
```
(adapt: skip gitlink entries; do not hold the `tree.walk` closure across an await.)

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0062/DT-011: `git.commit` adds exactly three objects
- Level / Driver / Location: integration / `Env` harness, `git.commit` / `crates/core/src/tools/git.rs` `mod tests`
- Given a project with a commit (`seed_tags(&e)`), hydrated, then 2 000 blobs written into libgit2 and not imported, and `before = entry.objects.list().await.unwrap().len()`
- When a file `/c.txt` is written (`e.write("/c.txt", "c\n")`) and `git.commit` succeeds (`e.commit("c")`)
- Then the store length is `before + 3` (blob, tree, commit) and `assert_closure_in_store` holds for the new commit
- Red before the fix because (mutation): restore `import_from_repo` at `git.rs:2323`; the length is `before + 2003`, `assert_eq!` fails.
- Cleanup: none (fresh `Env`).

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; the other six `import_from_repo` sites and the four pack sites.
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion; holding the `entry.repo` lock across the helper's awaits more than needed.
- Scope boundary: the six other bounded sites are US-0121.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(new objects) imported instead of O(repository objects); space O(size of the new objects)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutation reverted: `git diff main -- crates/core/src/tools/git.rs` shows only the one real production line (plus its comment) and the tests.
- [ ] One commit `US-0120: import only the new objects when committing`, body `Spec: SPEC-0062` and the requirement ids.
