# US-0116: import a single git object into the blob store

> Spec: SPEC-0061 (sdlc/specs/SPEC-0061-tag-object-import/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Add the targeted import the tag creation will use. Nothing calls it yet, so no behavior changes; it is proven by four unit tests.

## Requirements covered
- SPEC-0061/DR-001 [EARS-U]: The object store SHALL provide `BlobObjectDb::import_object(&self, repo: &git2::Repository, oid: git2::Oid) -> Result<usize>` that copies exactly the object `oid` from the libgit2 object store of `repo` into the blob store and the object index.
  - New form: returns 1 when it stored the object (blob put through `write_with_sha`, which also records the index row), 0 when the blob store already held it (after recording the index row if it was missing, the same repair as `import_from_repo`, `odb.rs:215-222`), and `ToolError::not_found(format!("git object '{oid}' not found in the repository"))` with nothing stored when libgit2 does not hold the object. It reads only that one object. A libgit2 object of an unstorable type (`object_type_name` fails) is skipped like `import_from_repo` does (`odb.rs:206`): returns 0. `import_from_repo` is not modified.

## Independent Test
`cargo test -p mcp-fs-core --lib git::odb` runs DT-001 to DT-004 green. Then replace the body of `import_object` by a call to `import_from_repo(repo)` (mutation) and see DT-002 fail with `left: 2001 right: 1`-style output, then `git checkout -- crates/core/src/git/odb.rs` and re apply your real implementation (keep it in a scratch copy first).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0061): a new targeted method rather than changing `import_from_repo`; 11 other callers rely on the whole walk.
- DEC-002: returns `Result<usize>` (1 or 0), for parity with `import_from_repo`.
- DEC-003: an object libgit2 does not hold is `ERR_NOT_FOUND`, not a silent 0, because a ref row must never name an object the store lacks.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
`pub async fn import_object(&self, repo: &git2::Repository, oid: git2::Oid) -> Result<usize>` on `BlobObjectDb` (crates/core/src/git/odb.rs). Internal: only this workspace calls it.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/odb.rs` | modify (new method + 4 unit tests in `mod tests`, which starts at line 296) |
### Existing patterns
The code to mirror is `import_from_repo` (`crates/core/src/git/odb.rs:188-228`). Read one object, then store or repair:
```rust
let odb = repo.odb().map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
let obj = odb.read(oid).map_err(|_| ToolError::not_found(format!("git object '{oid}' not found in the repository")))?;
if object_type_name(obj.kind()).is_err() { return Ok(0); }
let (sha, kind, payload) = (oid.to_string(), obj.kind(), obj.data().to_vec());
// the git2 Odb borrow cannot be held across an await: copy out, drop `odb`, then:
if self.exists(&sha).await? {
    if !self.index.object_exists(&sha).await? {
        self.index.record_object(&sha, object_type_name(kind)?, payload.len() as i64).await?;
    }
    return Ok(0);
}
self.write_with_sha(&sha, kind, &payload).await?;
Ok(1)
```
Test helpers in `mod tests`: `async fn odb() -> (tempfile::TempDir, BlobObjectDb)` (`odb.rs:300`); `git2::Repository::init_bare(dir.path())`; accessors `o.blobs()`, `o.index()`, `blob_key`, `serialize`; the closest existing test is `import_reindexes_a_blob_missing_from_the_index` (`odb.rs:487-505`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0061/DT-001: import_object stores exactly one object
- Level / Driver / Location: unit / direct call / `crates/core/src/git/odb.rs` `mod tests`
- Given a bare libgit2 repository holding one blob written with `repo.blob(b"one")`, and an empty store (`odb()`)
- When `o.import_object(&repo, oid)` is called twice
- Then the first call returns `1`, the second `0`; `o.read(&oid.to_string())` returns `(ObjectType::Blob, b"one")`; `o.list().await.unwrap().len()` equals `1`

### SPEC-0061/DT-002: import_object leaves other objects alone
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given a bare repository holding 2 000 blobs (`format!("blob {i}")`, i in 0..2000) plus one target blob, none imported
- When `o.import_object(&repo, target)` is called
- Then it returns `1` and `o.list().await.unwrap().len()` equals `1`
- Red before the fix because (mutation): replace the body of `import_object` by `self.import_from_repo(repo).await`; the list length is `2001`, not `1`.

### SPEC-0061/DT-003: import_object repairs a missing index row
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given the blob already put in the blob store with `o.blobs().put(&blob_key(&sha), &serialize(ObjectType::Blob, b"orphan").unwrap())` and no index row (`!o.index().object_exists(&sha)`)
- When `o.import_object(&repo, oid)` is called
- Then it returns `0` and `o.index().object_exists(&sha).await.unwrap()` is true

### SPEC-0061/DT-004: import_object on an unknown id
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given an empty bare repository and an empty store
- When `o.import_object(&repo, Oid::from_str("1111111111111111111111111111111111111111").unwrap())` is called
- Then it fails with code `ERR_NOT_FOUND` and message `git object '1111111111111111111111111111111111111111' not found in the repository`; `o.list().await.unwrap()` is empty

## Constraints
- Files not to touch: any file other than `crates/core/src/git/odb.rs`; `import_from_repo` must not change.
- Dependencies not to add: none.
- Patterns to avoid: holding the `git2::Odb` across an await; reading every object.
- Scope boundary: switching `tag_create` is US-0117.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib e2e_tag` stays green (212 tests today).
- [ ] DEBT: observable behavior unchanged (DR-004): the stored annotated tag object has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(1) objects read, one blob existence probe, one blob put (O(size of the object)); space O(size of the object)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutation reverted: `git diff main -- crates/core/src/git/odb.rs` shows only the real implementation and tests.
- [ ] One commit `US-0116: import a single git object into the blob store`, body `Spec: SPEC-0061` and the requirement ids.
