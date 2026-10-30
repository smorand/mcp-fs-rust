# US-0117: import only the tag object when creating an annotated tag

> Spec: SPEC-0061 (sdlc/specs/SPEC-0061-tag-object-import/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0116
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Stop `git.tag_create` from copying the whole repository into the blob store when it adds one annotated tag. Measured before: 364 to 709 ms at 5 000 objects and 1.6 to 2.9 s at 20 000, against 23 to 35 ms and about 100 ms for a lightweight tag.

## Requirements covered
- SPEC-0061/DR-002 [EARS-U]: `tag_create` SHALL import only the tag object it wrote, by calling `entry.objects.import_object(&repo, tag_oid)` before the ref row is written, and SHALL NOT call `import_from_repo`.
  - Old form: `entry.objects.import_from_repo(&repo).await?;` at `crates/core/src/tools/git_tags.rs:149`. New form: `entry.objects.import_object(&repo, tag_oid).await?;`. Annotated branch of `tag_create` only; the ordering rule of the existing comment stays (the object is in the blob store before the ref row names it).
- SPEC-0061/DR-004 (partial: the stored tag object): The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; the object stored for an annotated tag has the same id, type, payload, blob key and index row as before.

## Independent Test
Run the two new tests (DT-005, DT-006) green with `cargo test -p mcp-fs-core --lib e2e_tag`. DT-005 is the assertion that is not a mock of the story's own output: the store grows by exactly one object although 2 000 unrelated objects sit in libgit2. Then restore `import_from_repo` at `git_tags.rs:149` (mutation) and see DT-005 fail with `before + 2001`, revert it with `git checkout -- crates/core/src/tools/git_tags.rs` and re apply your real change.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0061): a side effect is dropped on purpose. Today `import_from_repo` in `tag_create` also stores any object libgit2 holds that the store lacks. After the change only the tag object is imported. Every other writer imports what it creates: the 11 production sites (`git/http/mod.rs:572`; `tools/git.rs:2323,3324,3918,4262,4891,4928,5632,6803,6935,7178`).
- DEC-003: an unknown object is `ERR_NOT_FOUND`; `tag_create` just wrote the object so `import_object` cannot fail on it.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No tool schema, route or stored format changes. `git.tag_create` and `git.tags` outputs are unchanged byte for byte.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`tag_create`, line 149) |
| `crates/core/src/tools/git.rs` | modify (tests module, starts at line 7655: 2 new tests) |
### Existing patterns
Current code (`crates/core/src/tools/git_tags.rs:142-151`):
```rust
let (kind, object) = if message.is_empty() {
    ("lightweight", commit)
} else {
    let tag_oid = write_tag_object(&repo, person, name, commit, message)?;
    // The tag object must be in the authoritative blob store before the
    // ref row names it, or a reader could find a ref to nothing.
    entry.objects.import_from_repo(&repo).await?;
    ("annotated", tag_oid)
};
```
Change that one call to `entry.objects.import_object(&repo, tag_oid).await?;`. Test harness (`crates/core/src/tools/git.rs`): `Env::new()`, `seed_tags(&e)` (commits), `e.entry().await` (the `GitRepoEntry`), `hydrate(&entry, &repo)`, `tag(&e, json!({"name": "v1.0", "message": "m"}))`, `tag_list(&e)`, `untag`; the pattern to copy for writing straight into libgit2 is `e2e_tag_022_five_thousand_tags` (`tools/git.rs`, search the name) which locks `entry.repo`, hydrates, writes objects and calls `import_from_repo`. New tests go next to `e2e_tag_026_delete_an_annotated_tag_reports_the_commit`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0061/DT-005: annotated tag creation adds exactly one object
- Level / Driver / Location: integration / `tag`, `tag_list` / `crates/core/src/tools/git.rs` `mod tests`
- Given a project with commits (`seed_tags(&e)`), hydrated, then 2 000 extra blobs written straight into libgit2's object store (`repo.blob(format!("blob {i}").as_bytes())`, i in 0..2000, NOT imported), and `before = entry.objects.list().await.unwrap().len()`
- When `tag(&e, json!({"name": "v1.0", "message": "m"}))` succeeds
- Then `entry.objects.list().await.unwrap().len() == before + 1`; the new object is the tag object (`out["object"]` is in the store with type `tag`: `entry.objects.read(..)` returns `ObjectType::Tag`); `tag_list(&e)` lists `v1.0` as `annotated` with message `m`
- Red before the fix because (mutation): restore `import_from_repo` at `git_tags.rs:149`; the length is `before + 2001`, the assertion `left == right` fails.
- Cleanup: none (fresh `Env`).

### SPEC-0061/DT-006: the stored tag object round trips
- Level / Driver / Location: integration / `tag`, `export_to_repo` / `tools/git.rs` `mod tests`
- Given an annotated tag `v2.0` with message `second` created through `git.tag_create` by `OWNER`
- When a fresh `git2::Repository::init_bare` is filled with `entry.objects.export_to_repo(&fresh)` and the tag object (`out["object"]`) is read there with `fresh.find_tag(oid)`
- Then its target is the tag's commit, its message is `second`, its tagger name is the name part of `OWNER` (before `@`), `entry.objects.exists(&sha)` is true and `entry.objects.index().object_exists(&sha)` is true

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; `import_from_repo` and its other 11 callers.
- Dependencies not to add: none.
- Patterns to avoid: asserting timing in a test (flaky); touching the lightweight branch of `tag_create`.
- Scope boundary: the listing lookups and the measurement are US-0118.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib e2e_tag` stays green (212 tests today).
- [ ] DEBT: observable behavior unchanged (DR-004): the stored annotated tag object has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(1) objects imported (the tag object) instead of O(repository objects); space O(size of the tag object)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutation reverted: `git diff main -- crates/core/src/tools/git_tags.rs` shows only the one real change.
- [ ] One commit `US-0117: import only the tag object when creating an annotated tag`, body `Spec: SPEC-0061` and the requirement ids.
