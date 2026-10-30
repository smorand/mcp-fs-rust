# US-0114: prove a remote delete leaves a paused operation untouched

> Spec: SPEC-0060 (sdlc/specs/SPEC-0060-tag-test-gaps/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0113
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
The test only compares the operation row, so a remote delete that moved HEAD or rewrote a conflicted file would pass. This story adds the comparison (SPEC-0053 FR-NEW-010, DEC-009).

## Requirements covered
- SPEC-0060/DR-005 [EARS-U]: `e2e_tag_029_remote_delete_during_a_paused_operation` SHALL, for each of its four operation kinds, also assert equality before and after the deletion of: (a) the HEAD commit read with `e.ref_sha("refs/heads/main")`; (b) the full list `e.entry().await.db.list_refs()`; (c) the content, read with `e.read(path)`, of every path in the paused operation's conflict list, which is the JSON array of path strings held in the `conflicts` field of `e.op_row()` (for example `["/a.txt"]`), all in addition to the existing `op_row()` comparison. It SHALL NOT assert anything about index entries.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_029_remote_delete` is green; with `delete_remote_tag` changed to touch HEAD it fails (mutation), then the mutation is reverted.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0060): each story proves its test red by mutation.
- SPEC-0053 DEC-009: the remote delete takes the write lock and does not call the pause guard; it must not alter the paused state.
### Data model
None.
### Contracts
None. No production signature changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 1 existing test strengthened) |
### Existing patterns
The test is at `git.rs:27758`: loop over `["merge", "rebase", "cherry_pick", "revert"]`, `pause_operation(&e, kind)`, `let (op_before, before) = (e.op_row().await, bare_refs(&dir));`. Helpers: `e.ref_sha(name) -> Option<String>` (`git.rs:13619`), `e.entry().await.db.list_refs()` (rows derive `PartialEq`), `e.read(path) -> String` (`git.rs:7756`). The conflict list is in `op_row.conflicts: Option<String>`, a JSON array of path strings (`git/db.rs:565`, `tools/git.rs:4951`); parse with `serde_json::from_str::<Vec<String>>`. Snapshot before and after:
```rust
async fn paused_state(e: &Env) -> (Option<String>, Vec<GitRefRow>, Vec<(String, String)>) {
    let paths: Vec<String> = e.op_row().await.and_then(|r| r.conflicts)
        .map(|c| serde_json::from_str(&c).unwrap()).unwrap_or_default();
    let mut files = Vec::new();
    for p in paths { let c = e.read(&p).await; files.push((p, c)); }
    (e.ref_sha("refs/heads/main").await, e.entry().await.db.list_refs().await.unwrap(), files)
}
```
Use the actual row type name returned by `list_refs()` instead of `GitRefRow` if it differs.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Success criterion of the DEBT: the existing suite passes unmodified, tolerated exceptions: 1 (`e2e_tag_029_remote_delete_during_a_paused_operation` gains assertions only).

### SPEC-0060/DR-005: nothing moved while the remote tag was deleted
- Level / Driver / Location: integration / `call_delete_remote_tag` / `git.rs`, `e2e_tag_029_remote_delete_during_a_paused_operation`
- Given each of merge, rebase, cherry_pick, revert paused (`op_row()` is `Some`) and a remote tag `v1.0` at `f`
- When the owner deletes the remote tag `v1.0` with its short commit
- Then the result `deleted_commit` is `f` and the remote lost `v1.0` (existing assertions, kept)
- And `paused_state(&e)` after equals `paused_state(&e)` before: HEAD sha, the whole ref list, and the content of every conflicted path (each kind has at least one conflicted path; assert `!files.is_empty()` in the snapshot)
- And no assertion about index entries
- Red before the fix because (mutation): make `delete_remote_tag` move HEAD (for example call `reject_if_operation_in_progress`, which refuses, or write a ref); the snapshot differs.
- Cleanup: none.

## Constraints
- Files not to touch: any file other than `crates/core/src/tools/git.rs`; no production line.
- Dependencies not to add: none.
- Patterns to avoid: asserting index entries; relaxing an existing assertion of the test.
- Scope boundary: reasons on failure legs are US-0115.

## Definition of done
- [ ] RED evidence recorded for every new test (the mutation named in the story) before reverting it.
- [ ] Every acceptance test above green; `make check` green (or `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, tolerated exceptions: 1 (`e2e_tag_029_remote_delete_during_a_paused_operation` gains assertions only). `cargo test -p mcp-fs-core --lib e2e_tag` stays green.
- [ ] DR-009: no line outside `#[cfg(test)]` changed (`git diff main --stat` lists only `crates/core/src/tools/git.rs`).
- [ ] Docs: none to update (`.agent_docs/testing.md` lists suites, not tests).
- [ ] Self review: tests assert exact values, no sleep used to synchronize, mutation reverted with `git checkout`, no `unwrap` outside tests.
- [ ] Mutation reverted: `git diff main -- crates/core/src/tools/git_tags.rs` is empty.
- [ ] One commit `US-0114: prove a remote delete leaves a paused operation untouched`, body `Spec: SPEC-0060` and the requirement ids.
