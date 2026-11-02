# US-0147: warn when an on disk ref write fails

> Spec: SPEC-0071 (sdlc/specs/SPEC-0071-warn-when-an-on-disk-ref-write-fails/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
Three sites write the on disk ref and drop a libgit2 failure silently. One helper writes it and logs one warning on failure, and the reconcile no longer lets libgit2 rewrite a name like `a//b`. The call still succeeds.

## Requirements covered
- SPEC-0071/DR-001: The git tools SHALL write an on disk ref after an index ref write through `sync_disk_ref` at the three sites of Section 2.
- SPEC-0071/DR-002: WHEN the on disk ref write fails THE git tools SHALL log one warning that carries the fields `tool`, `ref` and `error`, with the message `on disk ref write failed`, and SHALL return the same result as when the write succeeds.
- SPEC-0071/DR-003: IF `Reference::is_valid_name(<ref>)` is false for a ref that `reconcile_incoming_tags` is about to write THEN THE git tools SHALL skip the on disk write, keep the index row, and log the warning of DR-002 with `error` set to `invalid reference name`.
- SPEC-0071/DR-004: The git tools SHALL NOT discard the result of an on disk ref write at the three sites of Section 2.
- SPEC-0071/DR-005: The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including DT-001 to DT-004, DT-010 and DT-011; with the helper made to warn always, DT-004 fails; with the old `let _ =` restored in `tag_create`, DT-010 fails.

## Design excerpt
### Decisions
- DEC-001: a failed on disk write does not fail the call. Alternatives considered: fail the call and roll the index row back (rejected: the index is the source of truth, a rollback of a committed row plus a new error contract is a behavior change outside a debt; decided by the owner on 2026-11-02). Rationale: the index is authoritative; the on disk repository is a cache. Implemented by: DR-002, DR-004. Evidence: `git_tags.rs:154-158` (existing comment: "The index row is the source of truth").
- DEC-002: scope is the 3 sites of BL-0095 and BL-0105. Alternatives considered: all 13 sites (rejected by the owner's default; the other 10 become BL-0121). Implemented by: n/a, no code impact.
- DEC-003: BL-0105's `tag_create` half is already done: the name is refused before any write (`git_tags.rs:46`, `git.rs:2372-2395`). Only `reconcile_incoming_tags` remains. Implemented by: DR-003.
- DEC-004: the helper is `pub(crate) fn sync_disk_ref(repo: &git2::Repository, ref_name: &str, oid: git2::Oid, tool: &str)`, in `crates/core/src/tools/git.rs`, returning nothing. It calls `repo.reference(ref_name, oid, true, &format!("mcp-fs {tool}"))` and on `Err(e)` emits `tracing::warn!(tool, r#ref = ref_name, error = %e, "on disk ref write failed")`. Obliged by: DR-001, DR-002. Rationale: a unit return keeps the three call sites one line each; the reflog message stays `mcp-fs <tool>`. Alternatives considered: returning a `Result` the caller ignores (rejected: recreates the silent discard); a method on `BlobObjectDb` (rejected: the repository handle is not owned there). Source: design. Evidence: `git.rs:2504`, `git_tags.rs:158`, `:449`.
- DEC-005: `reconcile_incoming_tags` checks `git2::Reference::is_valid_name(&tag_ref)` before calling the helper; a rejected name logs the same warning with `error = "invalid reference name"` through a second function `warn_disk_ref_skipped(tool, ref_name, why)` that the helper also uses. Obliged by: DR-003. Rationale: one warning shape. Alternatives considered: let `reference` fail on `a//b` (rejected: libgit2 normalises it to `a/b` instead of failing, `git_tags.rs:699-704`). Source: existing code.
- DEC-006: one story: three call sites, one helper, six tests, three files, tier 2. Obliged by: n/a (no code impact): partitioning only. Alternatives considered: one story per site (rejected: each would add a helper use with no way to show the new warning). Source: design.
- DEC-007: red evidence: DT-001 to DT-003 fail before the change (no warning); DT-004 passes on arrival and is proven by the named mutation of the spec (the helper always warns). DT-010 and DT-011 fail before the change (the discarded writes exist). Obliged by: DR-002. Source: design.
### Data model
None.
### Contracts
`pub(crate) fn sync_disk_ref(repo: &git2::Repository, ref_name: &str, oid: git2::Oid, tool: &str)` in `crates/core/src/tools/git.rs`, returns `()`, warns `on disk ref write failed` with fields `tool`, `ref`, `error`. Call sites: `branch_create` (`tool = "git.branch_create"`), `tag_create` (`"git.tag_create"`), `reconcile_incoming_tags` (`"git.remote_fetch"`). The reconcile checks `git2::Reference::is_valid_name` first and warns with `error = "invalid reference name"` without writing.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (helper, `branch_create`, tests DT-001, 003, 004, 010, 011) |
| `crates/core/src/tools/git_tags.rs` | modify (`tag_create`, `reconcile_incoming_tags`) |
| `.agent_docs/git.md` | modify (sentence at the BL-0095 mention) |
### Existing patterns
`CapturedLog` and `tracing::subscriber::set_default` at `crates/core/src/tools/git.rs:23637-23700` (E2E-NEW-903); the `after_listing` style of test seam at `git_tags.rs:757+` is not needed.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> A blocked write is made by creating the empty file `refs/heads/feat.lock` (`refs/tags/v1.lock`) inside the project's on disk repository before the call.

### SPEC-0071/DT-001: branch_create survives a blocked on disk write
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002, DR-004, DR-005
- Given a project with one commit and a file `refs/heads/feat.lock` created in its on disk repository, When `git.branch_create` is called with `feat`, Then the call returns `Ok`, the index holds `refs/heads/feat`, and the captured log holds one line with `on disk ref write failed`, `git.branch_create` and `refs/heads/feat`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0071/DT-002: tag_create survives a blocked on disk write
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002, DR-004. Same, with `refs/tags/v1.lock`, tool `git.tag_create`, tag `v1`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0071/DT-003: the reconcile skips a rewritable name
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-003. Given an incoming tag named `a//b`, When `reconcile_incoming_tags` runs, Then the index holds `refs/tags/a//b`, no `refs/tags/a/b` exists in the on disk repository, and the log holds `invalid reference name`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0071/DT-004: a healthy write is silent
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a normal `branch_create`, `tag_create` and reconcile, Then the captured log holds no `on disk ref write failed`.
- **Red evidence:** DT-001 to DT-003 fail first (no warning exists today). DT-004 passes on arrival; named mutation: `sync_disk_ref` always warns, DT-004 must go red.
- Cleanup: temp directories are dropped with the test.

### SPEC-0071/DT-010: no discarded ref write at the three sites
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Asserts:** `git_tags.rs` contains no `let _ = repo.reference(`; the body of `branch_create` has none.
- **Method:** a unit test reading the two source files with `include_str!` and asserting on the extracted function bodies.
- Cleanup: temp directories are dropped with the test.

### SPEC-0071/DT-011: one writer
- Level / Driver / Location: integration / direct tool call on `Env` with `CapturedLog` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Asserts:** outside `sync_disk_ref`, the three sites contain no `.reference(` call. **Method:** same as DT-010.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for every new test that can fail before the change; tests that guard behavior that already exists are proven by the mutation named in the story.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] The existing suite passes unmodified: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`; exceptions: none (0 existing tests modified).
- [ ] Docs updated: `.agent_docs/git.md` (the BL-0095 sentence now states the warning).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0071 cited by at least one story with status done or landed.
- [ ] One commit `US-0147: warn when an on disk ref write fails`, body `Spec: SPEC-0071` and the requirement ids.
