# US-0112: test a fetch of 501 annotated tags and a blank remote name

> Spec: SPEC-0060 (sdlc/specs/SPEC-0060-tag-test-gaps/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0111
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
The tag receive loop sends refspecs in batches of 500 and no test runs a second batch; the blank remote name rule of SPEC-0053 has no test. This story adds both.

## Requirements covered
- SPEC-0060/DR-003 [EARS-U]: The tag test suite SHALL contain a test where a remote holds 501 annotated tags on at least 3 distinct commits, a fetch is run, and every one of the 501 tags is received with its message and commit.
- SPEC-0060/DR-004 [EARS-U]: The tag test suite SHALL contain a test that calls `git.remote_delete_tag` with the `remote` argument `""`, and again with `"   "`, on a project that tracks `origin`, and asserts each call is refused with the remote name shown as given, no remote tag removed, and no audit line written.

## Independent Test
The two new tests are green under `cargo test -p mcp-fs-core --lib e2e_tag`. The mutations of the acceptance section make each fail; they are reverted afterwards.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0060): each story proves its own tests red by mutation.
- Governing: SPEC-0052 FR-NEW-009 (receive tags on fetch), SPEC-0053 FR-NEW-014 and DEC-011 (a blank text is a given name).
### Data model
None.
### Contracts
None. No production signature changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 2 new tests) |
### Existing patterns
Fetch of many tags: copy `e2e_tag_048_two_thousand_tags` (`git.rs:25582`); remote builders `annotated_tag_bare_remote(dir, tag, target, message)` (about `git.rs:25465`) and `advance_bare_remote`; fetch through `call_fetch_branch`. `TAG_REFSPEC_BATCH` is 500 (`git/remote.rs:1336`), applied by `chunks` at `remote.rs:1463`, and only annotated tags go through it, so 501 annotated tags make 2 batches.
Blank remote: copy `e2e_tag_044_remote_delete_named_remote_not_tracked` (`git.rs:27630`) and `assert_lookup_refused`; the helper that calls the registered tool is `delete_by_tool` (`git.rs:27325-27329`), the refusal text is `untracked_text(tag, remote)` (`git.rs:27350`), the lookup is the exact match `*n == a.remote` (`git_tags.rs:1272-1274`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Success criterion of the DEBT: the existing suite passes unmodified, tolerated exceptions: none.

### SPEC-0060/DR-003: fetch receives 501 annotated tags
- Level / Driver / Location: integration / `call_fetch_branch` / `git.rs`, next to `e2e_tag_048_two_thousand_tags`
- Given a bare remote with the project history and 501 annotated tags `t000` to `t500`, each with message `m<i>`, spread over at least 3 distinct commits (`commits[i % 3]`), 
- When the project fetches the branch
- Then the project holds exactly 501 tags (`tag_names(&e).len() == 501`), each named `t<i>` with the commit `commits[i % 3]`
- And `t500` (the 501st, the one in the second batch) is present with message `m500`
- Red before the fix because (mutation): make the `chunks` loop at `git/remote.rs:1463` process only the first chunk; fewer than 501 tags are received and `t500` is missing.
- Cleanup: none.
- Note: if the test takes more than 10 seconds, say so in the commit body (DEC assumption of SPEC-0060).

### SPEC-0060/DR-004: blank remote name is refused as given
- Level / Driver / Location: integration / `delete_by_tool` / `git.rs`, next to `e2e_tag_049_remote_delete_empty_or_blank_name_is_absent`
- Given a project tracking `origin` (a `file://` bare remote with tag `v1.0`; `deletion_remote(&e, "remote-blank")`)
- When `git.remote_delete_tag` is called with `remote` = `""`, then with `remote` = `"   "`, tag `v1.0` and the tag's short commit
- Then each call fails with the code and the text `untracked_text("v1.0", <name as given>)` (the name shown as given, spaces kept)
- And `bare_refs(&dir)` is unchanged and the remote delete audit is empty
- Red before the fix because (mutation): treat a blank `a.remote` as `origin` (`git_tags.rs:1272-1274`); the call succeeds and the remote tag is removed.
- Cleanup: none.

## Constraints
- Files not to touch: any file other than `crates/core/src/tools/git.rs`; no production line.
- Dependencies not to add: none.
- Patterns to avoid: a per test copy of the remote builders (reuse the existing ones).
- Scope boundary: paused and publish all tests are later stories.

## Definition of done
- [ ] RED evidence recorded for every new test (the mutation named in the story) before reverting it.
- [ ] Every acceptance test above green; `make check` green (or `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, tolerated exceptions: none. `cargo test -p mcp-fs-core --lib e2e_tag` stays green.
- [ ] DR-009: no line outside `#[cfg(test)]` changed (`git diff main --stat` lists only `crates/core/src/tools/git.rs`).
- [ ] Docs: none to update (`.agent_docs/testing.md` lists suites, not tests).
- [ ] Self review: tests assert exact values, no sleep used to synchronize, mutation reverted with `git checkout`, no `unwrap` outside tests.
- [ ] Mutations reverted: `git diff main -- crates/core/src/git/remote.rs crates/core/src/tools/git_tags.rs` is empty.
- [ ] One commit `US-0112: test a fetch of 501 annotated tags and a blank remote name`, body `Spec: SPEC-0060` and the requirement ids.
