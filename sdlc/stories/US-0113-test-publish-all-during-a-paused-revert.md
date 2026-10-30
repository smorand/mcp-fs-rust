# US-0113: test publish all during a paused revert

> Spec: SPEC-0060 (sdlc/specs/SPEC-0060-tag-test-gaps/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0112
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
SPEC-0054 FR-NEW-014 names merge, rebase, cherry pick and revert; only the first three are tested for publish all. This story adds the revert arm and its test.

## Requirements covered
- SPEC-0060/DR-006 [EARS-U]: `pause_with_due_tag` SHALL accept the kind `revert` (a paused revert with a conflict and one due tag), and the test suite SHALL contain `assert_publish_during("revert")` asserting what the three existing kinds assert: line `v1.0 published`, the remote holds the tag, the operation row unchanged, one audit line.

## Independent Test
The new test is green; with publish all made to call `reject_if_operation_in_progress` it fails (mutation), then the mutation is reverted.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0060): each story proves its own tests red by mutation.
- Governing: SPEC-0054 FR-NEW-014 (a paused operation does not block publish all).
### Data model
None.
### Contracts
None. No production signature changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 1 helper arm, 1 new test) |
### Existing patterns
`pause_with_due_tag` (`git.rs:29320`) matches `kind`: `"merge"`, `"rebase"`, and the default `_` arm is cherry pick, so the `"revert"` arm goes BEFORE `_`. `assert_publish_during` is at `git.rs:29356` and is called for the three kinds at `git.rs:29367-29379`. A paused revert is built in `pause_operation` (`git.rs:27747-27750`):
```rust
_ => {
    let c = seed_fx_line_rewritten(e).await;
    revert(e, &c[2]).await.unwrap()
}
```
In `pause_with_due_tag` the revert arm must also seed (`let c = seed_fx_line_rewritten(e).await;`), build the remote with `remote_from_project(e, ...)`, create the due tag `v1.0` on `main`, then run `revert(e, &c[2])` and assert `paused["status"] == "conflict"`, in the same order as the other arms.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Success criterion of the DEBT: the existing suite passes unmodified, tolerated exceptions: none.

### SPEC-0060/DR-006: publish all during a paused revert
- Level / Driver / Location: integration / `confirm_all` / `git.rs`, after the publish all cherry pick test (`e2e_tag_046`)
- Given a project with a paused revert (conflict), `origin` tracked, and tag `v1.0` due on `main`
- When the owner confirms publish all with the number 1
- Then the lines are `["v1.0 published"]` and the remote refs equal the earlier ones plus `v1.0` at `main`
- And `e.op_row()` equals the row before (the revert is still paused) and `publish_audit(&e, OWNER)` equals `[published_line("v1.0", &main)]`
- Red before the fix because (mutation): make publish all call `reject_if_operation_in_progress`; the call is refused instead of returning `v1.0 published`.
- Cleanup: none.

## Constraints
- Files not to touch: any file other than `crates/core/src/tools/git.rs`; no production line.
- Dependencies not to add: none.
- Patterns to avoid: changing the behavior of the existing three arms.
- Scope boundary: transport reasons are US-0115.

## Definition of done
- [ ] RED evidence recorded for every new test (the mutation named in the story) before reverting it.
- [ ] Every acceptance test above green; `make check` green (or `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, tolerated exceptions: none. `cargo test -p mcp-fs-core --lib e2e_tag` stays green.
- [ ] DR-009: no line outside `#[cfg(test)]` changed (`git diff main --stat` lists only `crates/core/src/tools/git.rs`).
- [ ] Docs: none to update (`.agent_docs/testing.md` lists suites, not tests).
- [ ] Self review: tests assert exact values, no sleep used to synchronize, mutation reverted with `git checkout`, no `unwrap` outside tests.
- [ ] Mutation reverted: `git diff main -- crates/core/src/tools/git_tags.rs` is empty.
- [ ] One commit `US-0113: test publish all during a paused revert`, body `Spec: SPEC-0060` and the requirement ids.
