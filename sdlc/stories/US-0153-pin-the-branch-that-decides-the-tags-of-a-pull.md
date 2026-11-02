# US-0153: pin the branch that decides the tags of a pull

> Spec: SPEC-0075 (sdlc/specs/SPEC-0075-which-branch-tags-a-fetch-receives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0152
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
A pull brings in the tags of the pulled branch's history and a later fetch or pull of another branch brings in that branch's tags. Write the rule in tests and docs.

## Requirements covered
- SPEC-0075/FR-NEW-002: WHEN a member pulls a branch THE Remote exchange context SHALL bring in the remote tags that point into the history of that branch, and no others.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including the six tests below; forcing `TagScope::Off` in the pull makes E2E-005 fail.

## Design excerpt
### Decisions
- DEC-001: no production change planned. Obliged by: SPEC-0075/FR-NEW-001, SPEC-0075/FR-NEW-002. Rationale: the code follows SPEC-0052 FR-NEW-009. Alternatives considered: widening the rule (rejected by the owner). Evidence: `git.rs:3894-3918`.
- DEC-004: two stories: US-0152 (fetch, nine tests), US-0153 (pull and the rest, six tests, docs, closes the spec). Obliged by: n/a (no code impact): partitioning only. Rationale: SC-001 and SC-002 are fetch; SC-003 is pull plus the later fetch. Source: design.
- DEC-005: red evidence by mutation (tests pass on arrival): in the fetch path force `TagScope::RemoteDefault` always (E2E-002, 004 fail); force `TagScope::Off` (E2E-001 fails); in pull force `Off` (E2E-005 fails). Obliged by: SPEC-0075/FR-NEW-001, FR-NEW-002. Source: design.
### Data model
None.
### Contracts
No change.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (six tests) |
| `.agent_docs/git.md` | modify (one paragraph: which branch decides) |
### Existing patterns
`e2e_tag_043` at `crates/core/src/tools/git.rs:25826+`; `call_pull_branch` at `:11300`; the pull scope at `:6811`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0075/E2E-004: a later fetch of the other branch
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** E2E-002's end state, then Alice checks out `experimental`. **When** she fetches. **Then** `v9.0` and `v9.1` are added; `v1.0` stays and is not reported again.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-005: pull decides by the pulled branch
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** `main` checked out, the E2E-002 remote. **When** Alice pulls `experimental`. **Then** `v9.0` and `v9.1` are reported as added.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-006: pull of `main` does not bring in the other branch's tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the E2E-002 remote, Alice with nothing received yet. **When** she pulls `main`. **Then** `v1.0` is added and `v9.0` is not listed.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-007: a tag in both histories
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** tag `v0.5` on the commit where `experimental` branched off `main`. **When** Alice fetches on `main`, and on a fresh project pulls `experimental`. **Then** `v0.5` is added both times.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-013: pulling both branches
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** the E2E-002 remote. **When** Alice pulls `experimental`, then `main`, then `experimental` again. **Then** the project lists `v1.0`, `v9.0` and `v9.1` once each, and the third pull reports no tag added.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-014: a pull fails
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Failure.** **Given** Alice and a remote that does not exist. **When** she pulls `experimental`. **Then** the pull fails with the connection failure text and no tag is listed.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] Tests guard behavior that exists; each is proven by the mutation of DEC-005 (pull with `Off`: E2E-005 fails; pull always scoped to `main`: E2E-005 and E2E-013 fail).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] Docs updated: `.agent_docs/git.md` (which branch decides the tags of a fetch and of a pull).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0075 cited by at least one story with status done or landed.
- [ ] One commit `US-0153: pin the branch that decides the tags of a pull`, body `Spec: SPEC-0075` and the requirement ids.
