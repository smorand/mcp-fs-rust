# US-0132: prove the store's entry takes the hydration fast path

> Spec: SPEC-0066 (sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Nothing today fails if `create_entry` stops binding its repository, which silently turns `hydrate_if_stale` back into a full export. This story adds the test that notices. No production change.

## Requirements covered
- SPEC-0066/DR-002: The entry created by `GitRepoStore` SHALL take the `hydrate_if_stale` fast path for its own repository.

## Independent Test
`cargo test -p mcp-fs-core --lib git::repo` passes the new test; with the line `objects.bind_repo(repo.path());` (`git/repo.rs:208`) removed it fails on its first assertion; the line is restored before the commit.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0066): DT-003 proves the binding by the declared limit itself: after a synchronisation, wiping the repository's loose objects is NOT repaired by the next `hydrate_if_stale` (the marker holds), whereas an unbound repository is re-exported.
### Contracts
`BlobObjectDb::hydrate_if_stale(&self, repo: &git2::Repository) -> Result<usize>` (`git/odb.rs:144`), `export_to_repo` (`git/odb.rs:407`). The helper `seed_commit(&entry.objects, path, bytes, message)` is the one `http/mod.rs` tests use; read its definition there and reuse the same shape.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/repo.rs` (`mod tests`, line 229) | add the test |
### Existing patterns
`forget_loose_objects` (`git/odb.rs:1211`) removes the two hex character directories under `objects/`; copy its three lines into the test as a local helper. Entries come from `GitRepoStore::init_repo` as in `repo.rs` tests (e.g. `entries_are_cached_per_project`, line 270).

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0066/DT-003: the store's entry takes the fast path
- Level / Driver / Location: integration / `GitRepoStore::init_repo` / `crates/core/src/git/repo.rs` `mod tests`
- Given an entry holding 3 seeded objects (a commit, its tree, its blob via `seed_commit`) and a first `entry.objects.hydrate_if_stale(&repo)` that returns 3
- When the repository's loose object directories are removed and `hydrate_if_stale` is called again
- Then it returns `0` and `repo.odb().unwrap().exists(commit_oid)` is false
- And a following `export_to_repo` returns `3` and the commit is present again
- Red before the fix because (mutation): with the `bind_repo` call removed the second call re-exports and returns `3`, so the first Then fails.
- Cleanup: temp dirs drop.

## Constraints
- Files not to touch: every production file; the mutation is reverted before the commit.
- Dependencies not to add: none.
- Scope boundary: the upload-pack switch belongs to US-0133.

## Definition of done
- [ ] RED evidence (the mutation run) recorded before the commit.
- [ ] DT-003 green; `make check` green; main stays green.
- [ ] Non regression: the existing suite passes unmodified (0 tolerated exceptions): `cargo test -p mcp-fs-core --lib git`, `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --all -- --check`.
- [ ] Docs updated: none in this story.
- [ ] Self review: only `repo.rs` tests changed (`git diff --stat`).
- [ ] One commit `US-0132: prove the store's entry takes the hydration fast path`, body `Spec: SPEC-0066` and DR-002.
