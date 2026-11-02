# US-0152: pin the branch that decides the tags of a fetch

> Spec: SPEC-0075 (sdlc/specs/SPEC-0075-which-branch-tags-a-fetch-receives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0151
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A fetch brings in the tags of one branch's history: the checked-out branch, else the remote's default branch. Write that rule in tests (default branch, detached position, no default branch) and fix only what a test shows.

## Requirements covered
- SPEC-0075/FR-NEW-001: WHEN a member fetches THE Remote exchange context SHALL bring in the remote tags that point into the history of the checked-out branch, or into the history of the remote's default branch when no branch is checked out.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including the nine tests below; forcing `TagScope::RemoteDefault` in the fetch makes E2E-002 and E2E-001 differ from expectation.

## Design excerpt
### Decisions
- DEC-001: no production change planned. Obliged by: SPEC-0075/FR-NEW-001, SPEC-0075/FR-NEW-002. Rationale: the code follows SPEC-0052 FR-NEW-009. Alternatives considered: widening the rule (rejected by the owner). Evidence: `git.rs:3894-3918`.
- DEC-002: a detached position is simulated by writing the `HEAD` row non symbolic, not through a tool. Obliged by: SPEC-0075/FR-NEW-001 (E2E-011). Rationale: the product never leaves HEAD non symbolic itself (ASSUMED: no tool does; the story confirms with `rg 'set_ref\("HEAD"'`). Source: existing code `git.rs:3894`.
- DEC-003: if E2E-011 or E2E-015 fails on arrival, the fix goes in the same story and is the smallest that satisfies the spec. Obliged by: SPEC-0075/FR-NEW-001. Source: design.
- DEC-005: red evidence by mutation (tests pass on arrival): in the fetch path force `TagScope::RemoteDefault` always (E2E-002, 004 fail); force `TagScope::Off` (E2E-001 fails); in pull force `Off` (E2E-005 fails). Obliged by: SPEC-0075/FR-NEW-001, FR-NEW-002. Source: design.
### Data model
None.
### Contracts
No change planned. Helpers: `detach_head(e, sha)` (writes the `HEAD` row non symbolic); a bare remote with an unborn default branch (`Repository::set_head("refs/heads/nope")`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (nine tests, two helpers; a fix in the fetch path only if E2E-011 or E2E-015 fails) |
| `.agent_docs/git.md` | none in this story |
### Existing patterns
`e2e_tag_041`, `042` at `crates/core/src/tools/git.rs:25779-25817`; `call_fetch_branch` at `:10892`; `create_branch_on_bare_remote` `:10865`; fetch scope at `:3894-3918`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0075/E2E-001: fetch on the checked-out branch
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice with `main` checked out; remote tag `v1.0` on `main`'s history. **When** she fetches. **Then** `v1.0` is reported as added and listed in the project.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-002: tag on another branch is ignored
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the E2E-001 remote plus `experimental` holding tag `v9.0` (plain) and `v9.1` (annotated, message `outside`) only. **When** Alice, on `main`, fetches. **Then** neither is listed and neither name appears in the result.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-003: no checked-out branch (happy)
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** no branch checked out; the remote's default branch is `main`. **When** Alice fetches. **Then** `v1.0` is added and `v9.0` is not.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-008: nothing else changes
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-002's set up. **When** Alice fetches. **Then** the remote's tags are unchanged, the project's tag list is exactly `v1.0`, and the result lists no kept-local tag.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-009: remote down
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice with `main` checked out and a remote that does not exist. **When** she fetches. **Then** the fetch fails with the connection failure text and no tag is listed.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-010: no checked-out branch, other branch's tag
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Failure.** **Given** no branch checked out, the remote's default branch `main`, and `v9.0` only on `experimental`. **When** Alice fetches. **Then** `v9.0` is not listed and not mentioned in the result.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-011: a detached position behaves like no branch
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Edge.** **Given** Alice has checked out the commit `a1b2c3d` rather than a branch, and the remote's default branch is `main`. **When** she fetches. **Then** `v1.0` is added and `v9.0` is not.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-012: a fetch with no checked-out branch changes nothing else
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-010's set up. **When** Alice fetches. **Then** the project's tag list is exactly `v1.0` and the remote's tags are unchanged.
- Cleanup: temp directories are dropped with the test.

### SPEC-0075/E2E-015: the remote has no default branch
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Failure.** **Given** no branch checked out in the project and a remote whose default position names no branch. **When** Alice fetches. **Then** the fetch succeeds, the remote's branches are updated, and no tag is added, with no tag error.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] Tests guard behavior that exists; each is proven by the mutation of DEC-005 (always `RemoteDefault`: E2E-002 fails when the checked-out branch differs; `Off`: E2E-001 fails). E2E-011 and E2E-015 are unverified assumptions: if one fails on arrival, fix it here.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] Docs updated: none in this story (docs in US-0153).
- [ ] Self review: full.
- [ ] One commit `US-0152: pin the branch that decides the tags of a fetch`, body `Spec: SPEC-0075` and the requirement ids.
