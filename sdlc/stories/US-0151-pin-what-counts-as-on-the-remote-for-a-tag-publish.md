# US-0151: pin what counts as on the remote for a tag publish

> Spec: SPEC-0074 (sdlc/specs/SPEC-0074-what-counts-as-on-the-remote/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0150
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
A tag publish accepts a commit the remote advertises under any reference (branch, pull request, tag, default position). The code already does; write the rule in tests and docs so it stays.

## Requirements covered
- SPEC-0074/FR-MOD-001: The Remote exchange context SHALL treat a commit as on the remote when it equals a position the remote advertises under any reference (branch, pull request, tag or default position), or is an ancestor of such a position whose history the project already holds.
- SPEC-0074/FR-NEW-001: IF the commit of the tag being published is not on the remote under FR-MOD-001, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag '<tag>': commit <short id> is not on the remote. Publish its branch first.` and leave the remote and the audit log untouched.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including E2E-001 to E2E-010; restricting `commit_on_remote` to `refs/heads/*` tips makes E2E-002, 004 and 007 fail.

## Design excerpt
### Decisions
- DEC-001: no production change. Obliged by: SPEC-0074/FR-MOD-001. Rationale: the code already implements the wide rule. Alternatives considered: a filter on `refs/heads/*` (rejected by the owner). Evidence: `git_tags.rs:1474-1477`.
- DEC-002: one story: ten tests, three files, no production code. Obliged by: n/a (no code impact): partitioning only. Source: design.
- DEC-003: red evidence by mutation: restricting `commit_on_remote` to tips of `refs/heads/*` makes E2E-002, 004 and 007 fail; ignoring the project's held history (accept only equality) makes E2E-005 fail; fetching unheld tips makes E2E-006 fail. E2E-003, 008, 009, 010 guard the refusal and are proven by the mutation "accept every commit". Obliged by: SPEC-0074/FR-MOD-001, FR-NEW-001. Source: design.
- DEC-004: the doc comment of `commit_on_remote` is rewritten to name "any advertised position" instead of "an advertised commit the project already holds". Obliged by: SPEC-0074/FR-MOD-001. Source: design.
### Data model
None.
### Contracts
No change. Helper to add in the tests: `detach_bare_remote_head(dir, commit)`. A pull request reference is made with `create_branch_on_bare_remote(dir, "refs/pull/7/head", from_ref)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (ten tests, one helper) |
| `crates/core/src/tools/git_tags.rs` | modify (doc comment of `commit_on_remote`) |
| `.agent_docs/git.md` | modify (the sentence on what counts as on the remote) |
### Existing patterns
`e2e_tag_071` to `073` and `084` at `crates/core/src/tools/git.rs:26573-26720`; `commit_on_remote` at `git_tags.rs:1474-1486`; `create_branch_on_bare_remote` at `git.rs:10865`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0074/E2E-001: commit at a branch tip
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice, local tag `v1.0` on `a1b2c3d`, and the remote's branch `main` at `a1b2c3d`. **When** she publishes `v1.0`. **Then** it is published and the remote holds `v1.0` at `a1b2c3d`. **And** the remote's branches are unchanged.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-002: commit only under a pull request reference
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d`; the remote advertises `a1b2c3d` only as `pull/7/head`; the project holds the commit. **When** Alice publishes `v1.0`. **Then** it is published. **And** the remote holds `v1.0` at `a1b2c3d`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-003: commit the remote does not hold
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d`; the remote has branch `main` at `9e8d7c6`, unrelated to `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` **And** the remote has no `v1.0`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-004: commit pointed at only by a remote tag
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d`; the remote has no branch holding it but its tag `old` points at `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** it is published.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-005: ancestor of a pull request tip
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d`, a parent of `b2c3d4e`; the remote advertises `b2c3d4e` only as `pull/7/head`; the project holds both. **When** Alice publishes `v1.0`. **Then** it is published.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-006: tip the project does not hold
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d`, a parent of `b2c3d4e`, which the project does not hold; the remote advertises `b2c3d4e` as `pull/7/head`. **When** Alice publishes `v1.0`. **Then** the refusal is the E2E-003 text. **And** nothing was fetched: the project still lacks `b2c3d4e`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-007: the default position only
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote's default position is `a1b2c3d` and no branch carries it. **When** Alice publishes `v1.0` on `a1b2c3d`. **Then** it is published.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-008: a refusal changes nothing
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** the E2E-003 set up. **When** Alice publishes `v1.0`. **Then** the remote's references and the audit log equal their state before. **And** local tag `v1.0` still exists.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-009: the commit refusal comes first
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `v1.0` on `a1b2c3d` (not on the remote) and a different remote tag `v1.0` at `f6a7b8c`. **When** Alice publishes `v1.0` stating `f6a7b8c`. **Then** the refusal is the E2E-003 text, not a refusal about the remote tag.
- Cleanup: temp directories are dropped with the test.

### SPEC-0074/E2E-010: a remote that advertises nothing
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** an empty remote. **When** Alice publishes `v1.0` on `a1b2c3d`. **Then** the refusal is the E2E-003 text.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] Tests guard behavior that already exists; each is proven by the named mutation of DEC-003 (restrict to `refs/heads/*`: E2E-002, 004, 007 fail; equality only: E2E-005 fails; fetch unheld tips: E2E-006 fails; accept every commit: E2E-003, 008, 009, 010 fail). Any test that fails on arrival is a finding fixed in this story.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] Docs updated: `.agent_docs/git.md` (the wide rule).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0074 cited by at least one story with status done or landed.
- [ ] One commit `US-0151: pin what counts as on the remote for a tag publish`, body `Spec: SPEC-0074` and the requirement ids.
