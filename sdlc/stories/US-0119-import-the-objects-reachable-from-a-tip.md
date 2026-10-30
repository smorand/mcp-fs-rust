# US-0119: import the objects reachable from a tip

> Spec: SPEC-0062 (sdlc/specs/SPEC-0062-import-reachable-objects/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Add the import that stores only what a new tip brings. Nothing calls it yet, so no behavior changes; ten unit tests prove the algorithm, its probe bound and its write order.

## Requirements covered
- SPEC-0062/DR-001 [EARS-U]: The object store SHALL provide `BlobObjectDb::import_reachable(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` that stores, in the blob store and the object index, every object reachable from `tips` that the blob store lacks, and returns how many it stored.
  - Normative algorithm:
  1. An object the blob store already holds (`self.exists(&sha)`) is not read, stored or descended into: a stored object implies its closure is stored.
  2. A tip that is a commit: store it; visit each parent as an object; visit its tree with `base` = the tree id of its first parent when that parent commit is readable in `repo`, else none. A parent id `repo` does not hold is an error under step 7 and the call stores nothing. A parent the store holds but `repo` does not hold is not descended into (step 1), so the check of step 7 does not apply to it, and `base` is then none.
  3. A tip that is an annotated tag: store it and visit its target. A tip that is a tree: visit it with no base. A tip that is a blob: store it.
  4. Visiting a tree with `base`: skip it if its id equals `base`; skip it if the store holds it; otherwise store it and, for each entry: skip a gitlink (mode `0o160000`, a submodule commit never stored); skip an entry whose name and id equal the same named entry of the base tree; otherwise visit the entry (a blob as an object, a tree with `base` = the same named base entry when it is a tree, else none).
  5. An id is processed at most once per call.
  6. **Order:** an object is written only after every object it references that must be stored has been written (post order: blobs, then trees, then commits, parents before children), so that a crash never leaves a stored object whose closure is missing.
  7. An id `repo` does not hold is `ToolError::not_found(format!("git object '{oid}' not found in the repository"))` and nothing is stored from that call (all reads happen before the first write).
  8. An object of an unstorable type (`object_type_name` fails) is skipped like `import_from_repo` does (the `object_type_name(obj.kind()).is_err()` check, about `odb.rs:208-210`).
  Objects are written with `write_with_sha`, which also records the index row, exactly like `import_from_repo` and `import_object`.

## Independent Test
`cargo test -p mcp-fs-core --lib git::odb` runs DT-001 to DT-010 green. The non mock assertions are DT-001 (store grows by 3 with 2 000 strays present), DT-003 (counted store probes) and DT-010 (recorded write order). Then apply the three mutations named in the tests, see each fail, and restore your real implementation (keep a copy before mutating).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0062): a new method that walks from the new tip, rather than tracking the ids each writer creates.
- DEC-002: compare each tree with its first parent's tree and probe the store only for changed entries (a probe per entry is a network request per entry on the S3 backend, `storage/blob/s3.rs:128`).
- DEC-003: "the store holds it" stops the walk.
- DEC-004: post order writes, so a crash never leaves a stored object whose closure is missing.
- DEC-006: a gitlink entry (mode 0o160000) is skipped.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
`pub async fn import_reachable(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` on `BlobObjectDb` (crates/core/src/git/odb.rs). Internal: only this workspace calls it. `import_from_repo` and `import_object` are not modified.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/odb.rs` | modify (new method; 10 unit tests and 2 test wrappers in `mod tests`, which starts at line 330) |
### Existing patterns
Mirror `import_from_repo` (`odb.rs:188-228`) and `import_object` (`odb.rs:240-266`): copy payloads out of the `git2::Odb` before any await, then write with `self.write_with_sha(&sha, kind, &payload).await?`, probing with `self.exists(&sha).await?`. Plan: an iterative depth first traversal using an explicit stack of enter/exit frames and a `HashSet<Oid>` of ids already processed; all reads (and `exists` probes) happen during the traversal, which appends `(sha, kind, payload)` to an `order: Vec` on exit (post order); only after the traversal finishes, write `order` front to back. Tree entries come from `repo.find_tree(oid)?.iter()` (`entry.kind()`, `entry.filemode()`, `entry.name_bytes()`, `entry.id()`); the base tree is `repo.find_commit(first_parent).ok().map(|c| c.tree_id())` then `repo.find_tree`.
Test helpers in `mod tests`: `async fn odb() -> (tempfile::TempDir, BlobObjectDb)` (`odb.rs:334`) builds `BlobObjectDb::new(Arc<dyn BlobBackend>, index)`; for the two wrappers write a struct holding `Arc<dyn BlobBackend>` (a `LocalBlobStore`) and implement `BlobBackend` (`crates/core/src/storage/traits.rs:264`, `#[async_trait]`): delegate `put`, `get`, `exists`, `delete`, `ensure_bucket`, `remove_bucket` (the other methods have defaults); the counting one increments an `AtomicUsize` in `exists`, the recording one pushes the key into a `Mutex<Vec<String>>` in `put`. Existing test to copy the shape of: `import_object_stores_exactly_one_object` and `import_reindexes_a_blob_missing_from_the_index` (`odb.rs`, `mod tests`). Build commits with `git2`: `repo.blob`, `repo.treebuilder(None)` / `repo.treebuilder(Some(&tree))`, `tb.insert(name, oid, 0o100644)`, `repo.commit(None, &sig, &sig, "m", &tree, &parents)` with `git2::Signature::now("t", "t@t")`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0062/DT-001: a commit tip imports exactly its three objects
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given a bare repository with 2 000 blobs (`format!("blob {i}")`) not imported and one commit (one blob `a.txt`, its tree, the commit)
- When `o.import_reachable(&repo, &[commit]).await`
- Then it returns `3` and `o.list().await.unwrap().len()` equals `3`
- Red before the fix because (mutation): make the body call `self.import_from_repo(repo).await`; the length is `2003`.

### SPEC-0062/DT-002: a second commit stores only what changed
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given the first commit already imported, holding a directory `d` of 200 files `f000` to `f199` (distinct contents) and a file `top`; and a second commit (parent the first) changing only `d/f007`
- When `import_reachable(&repo, &[second])`
- Then it returns `4` (commit, root tree, tree `d`, the new blob) and the store length grew by `4`

### SPEC-0062/DT-003: the store probes are bounded
- Level / Driver / Location: unit / counting wrapper / `odb.rs` `mod tests`
- Given the repository of DT-002 and the counting wrapper whose counter is reset after the first import
- When the second import runs
- Then the number of `exists` calls is at most `8` (the tip, its parent, the root tree, `d`, `f007`, plus slack), far under `200`
- Red before the fix because (mutation): probe every entry of every new tree; the count is above `200`.

### SPEC-0062/DT-004: a merge commit sharing its second parent's tree
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given two imported commits `p1`, `p2` with different trees and a merge commit with parents `[p1, p2]` and the tree of `p2`
- When `import_reachable(&repo, &[merge])`
- Then it returns `1` (the commit only) because the tree is held by the store

### SPEC-0062/DT-005: a root commit imports all its entries
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given a commit with no parent holding `x.txt`, `y.txt` and a directory `s` with `z.txt` (three distinct contents)
- When imported into an empty store
- Then it returns `6` (1 commit, 2 trees, 3 blobs)

### SPEC-0062/DT-006: a gitlink entry is skipped
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given a tree whose entry `sub` has mode `0o160000` and an id libgit2 does not hold (`TreeBuilder::insert("sub", Oid::from_str("2222222222222222222222222222222222222222").unwrap(), 0o160000)`) and a commit on it
- When imported
- Then it succeeds, returns `2` (commit and tree) and `o.exists("2222222222222222222222222222222222222222")` is false

### SPEC-0062/DT-007: an unknown tip
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given an empty bare repository and an empty store
- When `import_reachable(&repo, &[Oid::from_str("1111111111111111111111111111111111111111").unwrap()])`
- Then it fails with code `ERR_NOT_FOUND` and message `git object '1111111111111111111111111111111111111111' not found in the repository`, and `o.list()` is empty

### SPEC-0062/DT-008: an annotated tag tip
- Level / Driver / Location: unit / direct call / `odb.rs` `mod tests`
- Given an annotated tag object (`repo.tag("v1", commit_obj, &sig, "m", false)`) on an unimported commit with a tree and a blob
- When imported with the tag as tip
- Then the tag, the commit, its tree and its blob are stored (returns `4`)

### SPEC-0062/DT-009: a tip the store already holds
- Level / Driver / Location: unit / counting wrapper / `odb.rs` `mod tests`
- Given a commit fully imported and the counting wrapper reset
- When imported again
- Then it returns `0` after exactly `1` `exists` call

### SPEC-0062/DT-010: dependencies are written first
- Level / Driver / Location: unit / recording wrapper / `odb.rs` `mod tests`
- Given the recording wrapper and a commit with a tree and a blob
- When imported
- Then the recorded `put` keys are in the order blob, tree, commit (`git:<sha>` keys)
- Red before the fix because (mutation): write in discovery order (commit first); the commit key is recorded first.

## Constraints
- Files not to touch: any file other than `crates/core/src/git/odb.rs`; `import_from_repo` and `import_object` must not change.
- Dependencies not to add: none.
- Patterns to avoid: holding the `git2::Odb` or a `git2::Tree` across an await; recursion (use an explicit stack: history can be deep); writing before all reads are done.
- Scope boundary: switching the call sites is US-0120 and US-0121.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(n) objects read and O(c) store probes where n is the number of new objects plus the entries compared and c the number of changed entries; space O(n) for the buffered payloads), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted: `git diff main -- crates/core/src/git/odb.rs` shows only the real implementation and tests.
- [ ] One commit `US-0119: import the objects reachable from a tip`, body `Spec: SPEC-0062` and the requirement ids.
