# US-0125: import only the new objects of a push

> Spec: SPEC-0063 (sdlc/specs/SPEC-0063-import-new-objects-of-packs/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0124
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Stop a push from copying the whole repository into the blob store, and prove a malformed push is still answered exactly as before.

## Requirements covered
- SPEC-0063/DR-002 (partial: push receive): Push receive, fetch and the fetch step of pull SHALL call `import_tips` with the tips their pack brought, where they called `import_from_repo`.
  - `receive_pack_locked` (`crates/core/src/git/http/mod.rs:572`): `entry.objects.import_tips(&repo, &push_tips(updates)).await?;` where `push_tips` collects `Oid::from_str(&u.new_sha)` of each update whose `new_sha` is not `ZERO_ID` and parses, in order. The push site keeps its gate: the import runs only when `pack.len() > 12` (`:569`).
- SPEC-0063/DR-005 [EARS-E]: WHEN a push names a tip that libgit2 does not hold, or whose closure is incomplete, THE server SHALL answer exactly as before this change: through the fallback of DR-001 the whole repository is imported and the per ref refusals of `process_ref_update` (`http/mod.rs:630-654`) are produced from the same state.
- SPEC-0063/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
`cargo test -p mcp-fs-core --lib git::http` is green. DT-008 is the non mock assertion: the pushed tip's closure is in the store and the store grew by under 200 although 2 000 unrelated objects sit in libgit2. DT-012 proves the fallback end to end (a ghost tip with a pack still gets `ng refs/heads/ghost object not found`). Then apply the mutations of the tests and revert.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0063): `import_tips` falls back to the whole walk on `ERR_NOT_FOUND` only, so a client controlled push naming an unknown tip together with a pack keeps getting `ng <ref> object not found` per ref (`http/mod.rs:635-637`). The existing test at `:985` sends no pack, so it never enters the gate; DR-005 is covered by DT-002 and DT-012.
- DEC-003: the push tips are the `new_sha` of the non delete commands.
- DEC-004: objects a pack carried that no new tip reaches are no longer imported.
- DEC-006: one local helper `push_tips` in the HTTP module.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No wire change: the report lines (`unpack ok`, `ok <ref>`, `ng <ref> object not found`, `ng <ref> non-fast-forward`) are byte identical. Type used: `RefUpdate { old_sha, new_sha, ref_name }` (`http/mod.rs:514-518`), `ZERO_ID` constant.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/mod.rs` | modify (production: helper `push_tips` and the call at `:572`; tests module, starts at line 663: 3 tests) |
### Existing patterns
Current code (`http/mod.rs:562-575`): `receive_pack_locked(entry, updates: &[RefUpdate], pack, side_band)`; inside `if pack.len() > 12 { let repo = entry.repo.lock().await; index_pack(&repo, pack)?; entry.objects.import_from_repo(&repo).await?; }`. Helper:
```rust
fn push_tips(updates: &[RefUpdate]) -> Vec<Oid> {
    updates.iter().filter(|u| u.new_sha != ZERO_ID).filter_map(|u| Oid::from_str(&u.new_sha).ok()).collect()
}
```
(`Oid` is already imported in this module: `object_present` uses `Oid::from_str`.) Test pattern to copy: `receive_pack_indexes_a_real_packfile_into_the_blob_store` (`http/mod.rs:1037`): `let (_s, e) = entry(d.path()).await;` builds a pack from a scratch repo with `packbuilder().insert_recursive(commit, None)` and `write_buf`, then a body of `pktline::encode_raw(format!("{ZERO_ID} {commit} refs/heads/main\0report-status\n"))` + `pktline::flush()` + the pack, and `handle_receive_pack(&e, &body)`; read the report with `read_pkt_lines`. Strays: `{ let repo = e.repo.lock().await; for i in 0..2000 { repo.blob(format!("stray {i}").as_bytes()).unwrap(); } }`, `e.objects.list().await.unwrap().len()`. For the ghost tip the existing test at `:985` uses `"b".repeat(40)`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0063/DT-008: push of a real pack among strays
- Level / Driver / Location: integration / `handle_receive_pack` / `git/http/mod.rs` `mod tests`
- Given the entry of the other http tests holding 2 000 unimported strays in `entry.repo`, `before` the store length, and a pack built like `:1037` (blob, tree, commit)
- When the push to `refs/heads/main` is received
- Then the report lines are `unpack ok` and `ok refs/heads/main`, the commit, its tree and its blob are in the store (`e.objects.exists`), and the store length is below `before + 200`
- Red before the fix because (mutation): restore `import_from_repo` at `http/mod.rs:572`; the length is above `before + 2000`.

### SPEC-0063/DT-009: push of an annotated tag object
- Level / Driver / Location: integration / `handle_receive_pack` / `git/http/mod.rs` `mod tests`
- Given a pack holding a commit and an annotated tag object `v1` on it (`src.tag("v1", &commit_obj, &sig, "m", false)`, `pb.insert_recursive(tag_oid, None)`), and the command `refs/tags/v1` at the tag object id
- When the push is received
- Then the report lines are `unpack ok` and `ok refs/tags/v1`, and the tag object, the commit, its tree and its blob are in the store

### SPEC-0063/DT-012: a push naming a ghost tip together with a pack
- Level / Driver / Location: integration / `handle_receive_pack` / `git/http/mod.rs` `mod tests`
- Given the entry, a real pack built as in `:1037` (so `pack.len() > 12`), and two commands in one request: `refs/heads/main` from `ZERO_ID` to the pushed commit, and `refs/heads/ghost` from `ZERO_ID` to `"b".repeat(40)`
- When the push is received
- Then the report lines are `unpack ok`, `ok refs/heads/main`, `ng refs/heads/ghost object not found`, the commit, its tree and its blob are in the store, and `e.db.get_ref("refs/heads/ghost")` is `None`
- Red before the fix because (mutation): remove the fallback arm from `import_tips`; the whole request fails with `ERR_NOT_FOUND` instead of producing the report.

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; `tools/git.rs`; `process_ref_update` and `object_present` must not change.
- Dependencies not to add: none.
- Patterns to avoid: filtering the tips by what libgit2 holds (the fallback is the decision, DEC-001); importing when the pack is 12 bytes or fewer.
- Scope boundary: the structural check and the measurement are US-0126.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (O(new objects + commands) per push instead of O(repository objects)), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted: `git diff main -- crates/core/src/git/http/mod.rs crates/core/src/git/odb.rs` shows only the helper, the one real production line and the tests.
- [ ] One commit `US-0125: import only the new objects of a push`, body `Spec: SPEC-0063` and the requirement ids.
