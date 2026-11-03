# US-0156: route the other on disk ref writes through sync_disk_ref

> Spec: SPEC-0077 (sdlc/specs/SPEC-0077-route-the-other-disk-ref-writes/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
Ten sites still drop a failed on disk ref write silently. Route each through the SPEC-0071 helper so a failure is logged once, the call still succeeds, and the reflog text stays the same.

## Requirements covered
- SPEC-0077/DR-001: The git tools SHALL write an on disk ref after an index ref write only through `sync_disk_ref`.
- SPEC-0077/DR-002: WHEN the on disk ref write of any of the ten sites fails THE git tools SHALL log one warning `on disk ref write failed` carrying `tool`, `ref` and `error`, and SHALL return the same result as when the write succeeds.
- SPEC-0077/DR-003: The non test code of `crates/core/src/tools/git.rs` SHALL NOT contain `let _ = repo.reference(`, and SHALL contain `.reference(` only inside `sync_disk_ref` and at the checked write of `push_branch` (`:3716`), whose error already fails the push.
- SPEC-0077/DR-004: The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including DT-001 to DT-007, DT-010 and DT-011; with the helper made to warn always, DT-007 fails; with a `let _ = repo.reference(` put back in `commit`, DT-010 fails.

## Design excerpt
### Decisions
- DEC-001: every site calls the SPEC-0071 helper with the old message minus `mcp-fs `. Alternatives considered: a second helper for non branch refs (rejected: one writer is the point). Rationale: the reflog text stays identical at nine sites. Implemented by: DR-001. Evidence: `git.rs:2398-2403`.
- DEC-002: `land_replay` wrote its raw `tool` as reflog text (`git.rs:4937`); through the helper it reads `mcp-fs <tool>` like every other site. Alternatives considered: a helper variant taking the full message (rejected: one more form for a text a bare repository does not keep). Implemented by: DR-001. Evidence: `git.rs:4937`.
- DEC-003: `pause_replay` writes `refs/mcp-fs/rebase-head`, not a branch; it takes the same helper, so its failure is logged too. Implemented by: DR-001, DR-002. Evidence: `git.rs:4521`, `:4971`.
- DEC-004: one story: ten one line changes and nine tests in one file. Obliged by: n/a (no code impact): partitioning only.
- DEC-005: the checked write at `git.rs:3716` stays as is: its failure must fail `git.remote_push`, which names the branch by ref. Alternatives considered: routing it through `sync_disk_ref` (rejected: a refusal would become a warning, a behavior change). Implemented by: DR-003. Evidence: `git.rs:3711-3721`.
- DEC-006: one story: ten one line changes and nine tests in one file plus a doc line. Obliged by: n/a (no code impact): partitioning only. Alternatives considered: one story per function (rejected: each would add a site with no way to show a distinct behavior beyond the shared helper). Source: design.
- DEC-007: red evidence: DT-001 to DT-006, DT-010 and DT-011 fail before the change; DT-007 passes on arrival and is proven by the named mutation of Section 8.2. Obliged by: DR-002, DR-003. Source: design.
- DEC-008: the warnings are read through `DISK_REF_WARNINGS` and `disk_ref_warnings(ref_name)` of SPEC-0071, filtered by a ref name unique per test. Obliged by: DR-002. Source: existing code `git.rs:23840`.
### Data model
None.
### Contracts
Each site becomes `sync_disk_ref(&repo, <ref>, <oid>, "<tool>");` with `<tool>` the old reflog message minus `mcp-fs `: `commit` `git.commit`; `set_branch_ref` its `tool`; `merge_inner` `git.merge fast-forward` and `git.merge`; `land_replay` its `tool`; `pause_replay` `git.rebase pause`; `merge_resolve_inner` `git.merge_resolve`; `replay_abort` `replay abort`; `pull_branch_inner` `git.remote_pull merge` and `git.remote_pull`. `sync_disk_ref(repo: &Repository, ref_name: &str, oid: Oid, tool: &str)` is at `git.rs:2398`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (ten sites, nine tests) |
| `.agent_docs/git.md` | modify (one sentence) |
### Existing patterns
`sync_disk_ref` and `warn_disk_ref_skipped` at `crates/core/src/tools/git.rs:2398-2416`; the tests `dt_0071_001` to `dt_0071_005` at `:23832-23960` (`block_disk_ref`, `disk_ref_warnings`); `seed_a`, `branch_create` helpers; the merge, rebase and pull test builders.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> A blocked write is made by creating the empty file `<ref>.lock` inside the project's on disk repository before the call (`block_disk_ref`). Use a warning ref name unique per test.

### SPEC-0077/DT-001: commit survives a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a project and the file `refs/heads/main.lock` in its on disk repository, When `git.commit` runs, Then it returns `Ok` and one warning for `refs/heads/main` names `git.commit`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-002: merge survives a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a fast forward merge and a true merge (two cases), each with the target branch's lock file in place, Then both return `Ok` and each records one warning naming `git.merge fast-forward` and `git.merge`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-003: merge resolve survives a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a conflicted merge, the lock on the branch, When it is resolved, Then `Ok` and one warning naming `git.merge_resolve`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-004: pull survives a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a fast forward pull and a merging pull (two cases) with the branch lock in place, Then `Ok` and one warning each naming `git.remote_pull` and `git.remote_pull merge`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-005: rebase paths survive a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given a rebase that lands, one that pauses on a conflict and an abort, each with the matching lock in place (the branch, `refs/mcp-fs/rebase-head`, the branch), Then each returns as without the block and records one warning naming the tool.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-006: set_branch_ref survives a blocked write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. Given the lock on the branch, When a branch reset runs, Then `Ok` and one warning for that ref.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-007: healthy writes are silent
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Validates:** DR-002. The operations of DT-001 to DT-006 with no lock file record no warning for their refs.
- **Red evidence:** DT-001 to DT-006 fail before the change (no warning is recorded). DT-007 passes on arrival; named mutation: `sync_disk_ref` always warns, DT-007 must go red.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-010: no discarded write
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Asserts:** the non test part of `git.rs` (text before `#[cfg(test)]\nmod tests`) contains no `let _ = repo.reference(`. **Method:** `include_str!("git.rs")` sliced at the test module.
- Cleanup: temp directories are dropped with the test.

### SPEC-0077/DT-011: one writer
- Level / Driver / Location: integration / direct tool call on `Env` with `block_disk_ref` and `disk_ref_warnings` / `crates/core/src/tools/git.rs` tests; DT-010, DT-011 structural by `include_str!`
- **Asserts:** that same slice contains `.reference(` exactly twice: once inside `sync_disk_ref`, once in `push_branch` followed by `.map_err(`. **Method:** count and slice as DT-010.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md` or design document, `git_tags.rs`, the checked write of `push_branch` (`git.rs:3716`), `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for every new test that can fail before the change; tests that guard behavior that already exists are proven by the mutation named in the story.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] The existing suite passes unmodified: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`; exceptions: none (0 existing tests modified).
- [ ] Docs updated: `.agent_docs/git.md` (the family of silent on disk ref writes is closed; the checked write of `push_branch` stays).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0077 cited by at least one story with status done or landed.
- [ ] One commit `US-0156: route the other on disk ref writes through sync_disk_ref`, body `Spec: SPEC-0077` and the requirement ids.
