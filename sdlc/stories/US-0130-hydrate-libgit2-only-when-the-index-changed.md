# US-0130: hydrate libgit2 only when the object index changed

> Spec: SPEC-0065 (sdlc/specs/SPEC-0065-hydrate-only-when-the-index-changed/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0129
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Bind each entry's `BlobObjectDb` to its libgit2 repository and make `hydrate` use the check, so a git tool call no longer re-lists and re-probes every object. Measured before: 135 to 157 ms per `hydrate` at 20 000 objects.

## Requirements covered
- SPEC-0065/DR-004 [EARS-U]: `create_entry` SHALL bind the path of the repository it opened, and `hydrate` SHALL call `hydrate_if_stale`.
  - **Old form:** `hydrate` is `entry.objects.export_to_repo(repo).await?; Ok(())` (`tools/git.rs:2069-2072`); `create_entry` builds `BlobObjectDb::new(blobs.clone(), db.clone())` (`repo.rs:206`) and nothing else.
  - **New form:** `objects.bind_repo(repo.path())` right after `create_entry` opened or initialised the repository (`repo.rs:192-198`, before the entry is built at `:215`); `hydrate` calls `entry.objects.hydrate_if_stale(repo).await?; Ok(())`.
  - **Applies to:** `repo.rs`, `tools/git.rs:2069-2072`.
- SPEC-0065/DR-006 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object `export_to_repo` copies into libgit2 is the same object as before.


## Independent Test
`cargo test -p mcp-fs-core --lib git` is green. DT-007 is the non mock assertion for correctness: a commit written to the store by another handle is visible through the first entry's `git.log` and `git.show`. Then restore the unconditional skip described in the test (mutation), see DT-007 fail, and revert.

## Design excerpt
### Decisions
- DEC-001 and DEC-005 (SPEC-0065): in-memory count marker; `BlobObjectDb::new` unchanged, the repository is bound by a separate call.
- DEC-006: declared limits (index only grows; a wiped on-disk cache is repaired at restart only; several replicas supported through the count); documented in US-0131.
- DEC-007: the upload-pack path stays on `export_to_repo` (BL-0113).
### Data model
None. The object index schema is not touched; the marker is process memory only.
### Contracts
No tool schema or output changes. `hydrate(entry, repo)` keeps its signature (`pub(crate) async fn hydrate(entry: &GitRepoEntry, repo: &Repository) -> Result<()>`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/repo.rs` | modify (`create_entry`: `objects.bind_repo(repo.path())` after the repository is opened, before the entry is built; `:192-215`) |
| `crates/core/src/tools/git.rs` | modify (production: `hydrate`, lines 2069-2072; tests module, starts at line 7665: DT-007 and DT-008) |
### Existing patterns
Current code: `hydrate` is `entry.objects.export_to_repo(repo).await?; Ok(())` (`tools/git.rs:2069-2072`); `create_entry` builds `let objects = Arc::new(BlobObjectDb::new(blobs.clone(), db.clone()));` at `repo.rs:206` and the repository `repo` exists from `:192-198`, so call `objects.bind_repo(repo.path());` once both exist (before `Ok(Arc::new(GitRepoEntry {..}))` at `:215`). `git2::Repository::path()` of a bare repository is its directory. Do NOT touch `git/http/mod.rs:434`. Test patterns: `Env::new()`, `seed_tags(&e)`, `e.entry().await`, `e.call("git.log", ..)`, `e.call("git.show", ..)`; `seed_commit(&objects, "f.txt", b"data", "seeded elsewhere") -> (commit, tree, blob)` is the helper `odb.rs` tests use (`git/odb.rs`, `pub async fn seed_commit`, check its visibility, add `pub(crate)` only if needed and say so); a second handle: `BlobObjectDb::new(entry.objects.blobs().clone(), entry.objects.index().clone())`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0065/DT-007: a commit written by another replica is visible
- **Validates:** DR-001, DR-004, DR-006. Given `Env` with a project that has a commit, and a second `BlobObjectDb` over the same blob store and index writing a commit with `seed_commit` and a branch ref `refs/heads/other` set through `entry.db.set_ref`; When `git.log` for `other` and `git.show` of the commit run through the first entry; Then both succeed and show the commit message `seeded elsewhere`.
- Red before the fix because (mutation): make `hydrate_if_stale` return early whenever a marker exists regardless of the count; `git.log` fails with a missing object.

### SPEC-0065/DT-008: the entry's own commit is read back consistently
- **Validates:** DR-002, DR-004, DR-006. Given `Env` with `seed_tags(&e)` then `git.commit` of `/c.txt`; When `git.log` and `git.show` of the new commit run; Then both show the commit, and a second `git.commit` and `git.log` also succeed (the marker moved with the import).

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs` beyond what US-0129 built; `git/http/mod.rs`; the 33 other `hydrate(` callers (they go through `hydrate`).
- Dependencies not to add: none.
- Patterns to avoid: calling `export_to_repo` from `hydrate`; a timing assertion.
- Scope boundary: documentation, the structural check and the measurement are US-0131.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a stub, or the mutation named for it where the behavior pre exists); mutations reverted with `git checkout -- <file>`.
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-006): every object `export_to_repo` copies into libgit2 is the same object as before.
- [ ] Docs: none in this story (US-0131)
- [ ] Self review: time and space of the critical path stated (a git tool call pays one `COUNT(*)` instead of O(repository objects)), no lock held across an await, no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutation reverted: `git diff main -- crates/core/src/tools/git.rs crates/core/src/git/repo.rs` shows only the real changes and the tests.
- [ ] One commit `US-0130: hydrate libgit2 only when the object index changed`, body `Spec: SPEC-0065` and the requirement ids.
