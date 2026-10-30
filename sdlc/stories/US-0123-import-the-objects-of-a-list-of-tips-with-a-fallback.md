# US-0123: import the objects of a list of tips with a whole walk fallback

> Spec: SPEC-0063 (sdlc/specs/SPEC-0063-import-new-objects-of-packs/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Add the entry point the three pack sites will use: strict import of the closure of the given tips, falling back to the whole walk only when a tip or parent is missing from libgit2. Nothing calls it yet, so no behavior changes.

## Requirements covered
- SPEC-0063/DR-001 [EARS-U]: The object store SHALL provide `BlobObjectDb::import_tips(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` that calls `import_reachable(repo, tips)` and, only when that fails with the code `ERR_NOT_FOUND`, calls `import_from_repo(repo)` and returns its result.
  - An error other than `ERR_NOT_FOUND` from `import_reachable` (a failed blob write, an index error) is returned as is and triggers no fallback. Empty `tips` returns `Ok(0)` with no store probe and no fallback. `import_reachable` stores nothing when it fails (all reads precede the first write, SPEC-0062 DR-001 step 7), so the fallback starts from the state the call started in. `import_from_repo`, `import_object` and `import_reachable` are not modified.

## Independent Test
`cargo test -p mcp-fs-core --lib git::odb` runs DT-001 to DT-004 green. The non mock assertions are DT-001 (store grows by 3 among 2 000 strays) and DT-002 (the fallback stores all 2 003). Then apply the three mutations named in the tests, see each fail, and restore your real implementation (keep a copy first).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0063): a thin wrapper `import_tips`: strict import, whole walk on `ERR_NOT_FOUND` only. A client controlled push naming an unknown tip together with a pack must keep getting the per ref `ng <ref> object not found` answer instead of a failed request; filtering the tips would keep the incomplete closure case that the strict walk rejects.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
`pub async fn import_tips(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` on `BlobObjectDb` (crates/core/src/git/odb.rs). Internal: only this workspace calls it.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/odb.rs` | modify (new method after `import_reachable`, which starts at line 289; 4 unit tests and 1 wrapper in `mod tests`, which starts at line 455) |
### Existing patterns
Implementation shape (the whole method):
```rust
pub async fn import_tips(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize> {
    match self.import_reachable(repo, tips).await {
        Err(e) if e.code == code::NOT_FOUND => self.import_from_repo(repo).await,
        other => other,
    }
}
```
(check the exact field and constant names: `ToolError` has a `code` and `code::NOT_FOUND` is imported at the top of `odb.rs`; see the DT-007 test of SPEC-0062 `dt007_an_unknown_tip` for how the tests read them). Add a doc comment saying WHY: a pack names its tips; the whole walk stays as the answer for a tip libgit2 lacks, which a client can cause.
Test helpers in `mod tests`: `odb()` and `counting_odb()` (`CountingBlobs` at `odb.rs:464` delegates to a `LocalBlobStore` and counts `exists`); write `FailingPutBlobs` the same way: a struct holding a `LocalBlobStore` and an `AtomicUsize`, `#[async_trait] impl BlobBackend` delegating everything, except `put` which increments the counter and returns `Err(ToolError::internal("disk full"))`. Build repositories as the SPEC-0062 tests do (`dt001_a_commit_tip_imports_exactly_its_three_objects`): `git2::Repository::init_bare`, `repo.blob`, `repo.treebuilder`, `repo.commit`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0063/DT-001: one new commit among strays
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given a bare repository with 2 000 unimported blobs and one commit (one blob, its tree, the commit)
- When `o.import_tips(&repo, &[commit]).await`
- Then it returns `3` and `o.list().await.unwrap().len()` equals `3`
- Red before the fix because (mutation): make `import_tips` call `self.import_from_repo(repo).await` only; the length is `2003`.

### SPEC-0063/DT-002: an unreadable tip falls back to the whole walk
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given the repository of DT-001 and the tip list `[Oid::from_str("1111111111111111111111111111111111111111").unwrap()]`
- When `import_tips`
- Then it returns `Ok(2003)` (every object `import_from_repo` stores) and the store holds all 2 003
- Red before the fix because (mutation): remove the fallback arm; the call fails with `ERR_NOT_FOUND`.

### SPEC-0063/DT-003: a write failure is not retried by the whole walk
- Level / Driver / Location: unit / `FailingPutBlobs` / `odb.rs` `mod tests`
- Given a `FailingPutBlobs` store and the repository of DT-001
- When `import_tips(&repo, &[commit])`
- Then it fails with the injected error (message `disk full`) and exactly `1` `put` was attempted (the blob, first in post order)
- Red before the fix because (mutation): fall back on any error; more than `1` `put` is attempted.

### SPEC-0063/DT-004: empty tips
- Level / Driver / Location: unit / counting wrapper / `odb.rs` `mod tests`
- Given the counting wrapper and a repository holding objects
- When `import_tips(&repo, &[])`
- Then it returns `0` after `0` `exists` calls and the store is empty

## Constraints
- Files not to touch: any file other than `crates/core/src/git/odb.rs`; `import_from_repo`, `import_object` and `import_reachable` must not change.
- Dependencies not to add: none.
- Patterns to avoid: falling back on any error; calling `import_from_repo` first.
- Scope boundary: switching the call sites is US-0124 and US-0125.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(new objects) on the normal path (as `import_reachable`); O(repository objects) only on the fallback), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted: `git diff main -- crates/core/src/git/odb.rs` shows only the real implementation and tests.
- [ ] One commit `US-0123: import the objects of a list of tips with a whole walk fallback`, body `Spec: SPEC-0063` and the requirement ids.
