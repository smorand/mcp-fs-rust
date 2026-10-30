# US-0115: assert the transport reason on connection failure tests and close the spec

> Spec: SPEC-0060 (sdlc/specs/SPEC-0060-tag-test-gaps/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0114
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
A connection failure test passes today whatever follows the prefix `fetch from host '<h>' failed: `. This story asserts the transport reason on seven tests, then closes the spec: it runs every mutation proof and the structural checks.

## Requirements covered
- SPEC-0060/DR-007 [EARS-U]: `e2e_tag_032`, `e2e_tag_033` and `e2e_tag_034` (remote delete) SHALL assert, after the prefix, the transport reason: `No such file or directory` for the missing `file://` path leg, and `unexpected eof while reading` for each `closing_remote` leg.
- SPEC-0060/DR-008 [EARS-U]: `assert_connection_refused` SHALL take the expected reason and assert it after the prefix, and `e2e_tag_020`, `e2e_tag_029` and `e2e_tag_050` (publish all) SHALL pass `No such file or directory`. `e2e_tag_021` already asserts `connection reset` and `e2e_tag_022` keeps its `closing_remote` leg with `unexpected eof while reading`.
- SPEC-0060/DR-009 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; no line outside `#[cfg(test)]` code is modified.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag` green; then the DT-001 mutation table below, row by row; then DT-002 and DT-003 commands.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0060): `contains` on the transport reason, not equality. The reasons were measured with the bundled OpenSSL libgit2 backend; re measure if the TLS backend changes.
- DEC-007 (SPEC-0060): the closing story re runs the whole mutation table.
- Honest limit: the `closing_remote` leg of `e2e_tag_032` is labelled rate limited and cannot tell a rate limit from a drop (BL-0098).
### Data model
None.
### Contracts
None. No production signature changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 7 tests, 1 helper signature) |
### Existing patterns
Current assertion in the remote delete tests (`git.rs:27474`, `27542`, `27564`), prefix only:
```rust
assert!(err.message.starts_with(&connection_prefix(&down)), "{}", err.message);
```
Target: add after it
```rust
assert!(err.message.contains("No such file or directory"), "{}", err.message);   // missing file:// leg
assert!(err.message.contains("unexpected eof while reading"), "{}", err.message); // closing_remote legs
```
Publish all helper today (`git.rs:29080`):
```rust
fn assert_connection_refused(err: &ToolError, url: &str) {
    assert_eq!(err.code, code::INTERNAL_ERROR, "{}", err.message);
    assert!(err.message.starts_with(&connection_prefix(url)), "{}", err.message);
}
```
Target signature `fn assert_connection_refused(err: &ToolError, url: &str, reason: &str)` adding `assert!(err.message.contains(reason), ...)`. Its 4 callers: `e2e_tag_020` (`git.rs:29096`, missing file, `No such file or directory`), `e2e_tag_022` (`git.rs:29177`, `closing_remote`, `unexpected eof while reading`), `e2e_tag_029` (`git.rs:29248`, missing file), `e2e_tag_050` (`git.rs:29449`, missing file). Change helper and the 4 callers in the same step so the build stays green. Measured messages: dropped connection `fetch from host '127.0.0.1' failed: SSL error: error:0A000126:SSL routines::unexpected eof while reading; class=Ssl (16)`; missing path `fetch from host 'file://<path>' failed: failed to resolve path '<path>': No such file or directory; class=Os (2)`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Success criterion of the DEBT: the existing suite passes unmodified, tolerated exceptions: 8 over the whole spec (029, 032, 033, 034 remote delete; 020, 022, 029, 050 publish all), assertions only added.

### SPEC-0060/DR-007: remote delete failure legs assert the reason
- Level / Driver / Location: integration / `call_delete_remote_tag` / `git.rs`, tests 032, 033, 034 (remote delete)
- Given a missing `file://` remote (032 down leg), and `closing_remote()` (032 refusing leg, 033, 034)
- When the owner deletes a remote tag
- Then the code is `INTERNAL_ERROR`, the message starts with the existing prefix and contains the reason: `No such file or directory` for the missing path leg, `unexpected eof while reading` for each `closing_remote` leg
- And all existing assertions of the three tests are kept
- Red before the fix because (mutation): replace the transport message after the prefix by a fixed string in `connection_error` (`git/remote.rs:1490`); the `contains` assertion fails.

### SPEC-0060/DR-008: publish all failure legs assert the reason
- Level / Driver / Location: integration / `call_publish_all_tags` / `git.rs`, tests 020, 022, 029, 050 (publish all) and helper `assert_connection_refused`
- Given a missing `file://` remote (`down_remote(&e)`) for 020, 029, 050; `closing_remote()` for 022
- When publish all runs
- Then the code is `INTERNAL_ERROR`, the message starts with the prefix and contains the reason passed to the helper
- And `e2e_tag_021` is unchanged (it already asserts `connection reset`)
- Red before the fix because (mutation): same mutation as DR-007.

### SPEC-0060/DR-009: behavior invariance
- Level / Driver / Location: structural / `git diff` / repository
- Given the branch of this story
- When `git diff main -- crates | rg '^[+-]'` is restricted to lines before the test module start (the line of `#[cfg(test)]` in `git.rs`)
- Then it returns nothing, and `git diff main --stat -- crates` lists only `crates/core/src/tools/git.rs`
- And the whole `cargo test -p mcp-fs-core --lib e2e_tag` and `cargo test --workspace` stay green

### SPEC-0060/DT-001: mutation proof, all rows
- Level / Driver / Location: mutation / manual mutation + `cargo test -p mcp-fs-core --lib e2e_tag` / production files, reverted with `git checkout`
- Given the tests of US-0111 to US-0115
- When each mutation is applied alone: DR-001 (report the row target instead of the peeled commit, `git_tags.rs:210,225`); DR-002 (check before `write_lock`, `git_tags.rs:200`); DR-003 (first chunk only, `remote.rs:1463`); DR-004 (blank `a.remote` treated as `origin`, `git_tags.rs:1272-1274`); DR-005 (remote delete calls `reject_if_operation_in_progress` or moves HEAD); DR-006 (publish all calls `reject_if_operation_in_progress`); DR-007 and DR-008 (fixed string after the prefix in `connection_error`, `remote.rs:1490`)
- Then each makes its named test fail with the quoted assertion; the report lists command and failing assertion per row
- And after the last `git checkout`, `git status --porcelain` shows only the test file

### SPEC-0060/DT-002: no production line changed
- Same check as DR-009's first Then; recorded once in the commit body.

### SPEC-0060/DT-003: residual backlog entries exist
- Level / Driver / Location: structural / shell / `sdlc/backlog`
- When `ls sdlc/backlog | rg 'BL-(009[89]|010[0-7])'` runs
- Then it prints 10 lines; and `ls sdlc/backlog | rg 'BL-00(81|82|83|86|87|94)'` prints nothing

## Constraints
- Files not to touch: any file other than `crates/core/src/tools/git.rs`; no production line.
- Dependencies not to add: none.
- Patterns to avoid: equality on the full message (the temp path and the OpenSSL code vary per machine); leaving a mutation applied.
- Scope boundary: a test double for 429 and hook rejection is BL-0098.

## Definition of done
- [ ] RED evidence recorded for every new test (the mutation named in the story) before reverting it.
- [ ] Every acceptance test above green; `make check` green (or `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, tolerated exceptions: 8 over the whole spec (029, 032, 033, 034 remote delete; 020, 022, 029, 050 publish all), assertions only added. `cargo test -p mcp-fs-core --lib e2e_tag` stays green.
- [ ] DR-009: no line outside `#[cfg(test)]` changed (`git diff main --stat` lists only `crates/core/src/tools/git.rs`).
- [ ] Docs: none to update (`.agent_docs/testing.md` lists suites, not tests).
- [ ] Self review: tests assert exact values, no sleep used to synchronize, mutation reverted with `git checkout`, no `unwrap` outside tests.
- [ ] Closing story: every requirement of SPEC-0060 (DR-001 to DR-009) is cited by a story with status done or landed, and the mutation table is in the commit body.
- [ ] One commit `US-0115: assert the transport reason on connection failure tests and close the spec`, body `Spec: SPEC-0060` and the requirement ids.
