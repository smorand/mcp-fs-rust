# US-0111: test the deletion of an annotated tag and its lock

> Spec: SPEC-0060 (sdlc/specs/SPEC-0060-tag-test-gaps/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Two regressions in local tag deletion pass the suite today: a wrong `sha` for an annotated tag, and an existence check moved outside the write lock. This story adds the two tests that catch them.

## Requirements covered
- SPEC-0060/DR-001 [EARS-U]: The tag test suite SHALL contain a test that creates an annotated tag with a message, deletes it through `git.tag_delete`, and asserts the result `sha` is the commit the tag points at (not the tag object id) and that the one `git.tag_delete` audit line reads `deleted tag '<name>' (was at <commit sha>)`.
- SPEC-0060/DR-002 [EARS-U]: The tag test suite SHALL contain a test that holds the repository write lock, starts a `git.tag_delete` of an existing tag, asserts after a wait of 150 ms (as at `git.rs:27790`) that the tag still exists and the call has not returned, releases the lock, and asserts the call then succeeds and the tag is gone. The project has no paused operation, otherwise the pause guard (`git_tags.rs:72`) refuses before the lock.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag` runs the two new tests green. Then apply the two mutations of the acceptance section to production code, run the two tests, see each fail with the quoted assertion, and `git checkout` the production files. The mutation is the non mock assertion.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0060): lock proofs hold the lock from the test and bound the wait, not a looped race.
- DEC-007 (SPEC-0060): each story proves its own tests red by mutation; the closing story re runs the table.
### Data model
None.
### Contracts
None. No production signature changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 2 new tests) |
### Existing patterns
Plain tag deletion test to copy the shape of (`git.rs:25218`, `e2e_tag_026_delete_a_local_tag`); `tag(&e, json!({"name": "v3.0", "message": "three"}))` makes an annotated tag (`git_tags.rs:143-146`); `untag_as(&e, OWNER, "v1.0")` deletes (`git.rs:25182`). The lock pattern to copy is `e2e_tag_030_remote_delete_two_members_at_once` (`git.rs:27780-27800`):

```rust
let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
let guard = entry.write_lock.lock().await;
let del = untag_as(&e, OWNER, "v1.0");
tokio::pin!(del);
tokio::select! {
    _ = &mut del => panic!("the deletion ran under the held lock"),
    () = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
}
// assert the tag still exists here, then:
drop(guard);
del.await.unwrap();
```
The audit reader for tag deletion is `deletion_audit(&e)` (used at `git.rs:25404-25420`), which returns `(person, detail)` pairs.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Success criterion of the DEBT: the existing suite passes unmodified, tolerated exceptions: none.

### SPEC-0060/DR-001: annotated tag deletion
- Level / Driver / Location: integration / `tag`, `untag_as` / `git.rs`, next to `e2e_tag_026_delete_a_local_tag`
- Given a project with commits (`seed_tags(&e)` gives commits `a`, `b`) and an annotated tag `v3.0` at `a` created with message `three`
- When the owner deletes `v3.0` with `untag_as(&e, OWNER, "v3.0")`
- Then the result `sha` equals `a` (the commit), which differs from the tag object id (assert both: `sha == a` and `sha != tag object id read from refs`)
- And `deletion_audit(&e)` equals `[(OWNER, "deleted tag 'v3.0' (was at <a>)")]`
- And `tag_names(&e)` no longer lists `v3.0`
- Red before the fix because (mutation): in `tag_delete` report the row target instead of the peeled commit (`git_tags.rs:210,225`); the `sha == a` assertion fails with the tag object id on the right side.
- Cleanup: none (fresh `Env`).

### SPEC-0060/DR-002: local deletion waits on the write lock
- Level / Driver / Location: integration / `untag_as` with held lock / `git.rs`, next to `e2e_tag_032_concurrent_deletion`
- Given a project with a plain tag `v1.0`, no paused operation, and the test holding `entry.write_lock`
- When the owner starts the deletion of `v1.0` and 150 ms pass
- Then the deletion future has not completed and `tag_names(&e)` still lists `v1.0`
- And after the guard is dropped the deletion returns `Ok` and `tag_names(&e)` no longer lists `v1.0`
- Red before the fix because (mutation): move the `get_ref` check and the delete before `write_lock.lock()` (`git_tags.rs:200`); the `select!` arm for the deletion fires and the test panics with `the deletion ran under the held lock`.
- Cleanup: none.

## Constraints
- Files not to touch: any file other than `crates/core/src/tools/git.rs`; no production line (DR-009).
- Dependencies not to add: none.
- Patterns to avoid: `sleep` as the only synchronization; repeating a race N times.
- Scope boundary: fetch, remote delete and publish all tests belong to later stories.

## Definition of done
- [ ] RED evidence recorded for every new test (the mutation named in the story) before reverting it.
- [ ] Every acceptance test above green; `make check` green (or `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, tolerated exceptions: none. `cargo test -p mcp-fs-core --lib e2e_tag` stays green.
- [ ] DR-009: no line outside `#[cfg(test)]` changed (`git diff main --stat` lists only `crates/core/src/tools/git.rs`).
- [ ] Docs: none to update (`.agent_docs/testing.md` lists suites, not tests).
- [ ] Self review: tests assert exact values, no sleep used to synchronize, mutation reverted with `git checkout`, no `unwrap` outside tests.
- [ ] Mutations reverted: `git diff main -- crates/core/src/tools/git_tags.rs` is empty.
- [ ] One commit `US-0111: test the deletion of an annotated tag and its lock`, body `Spec: SPEC-0060` and the requirement ids.
