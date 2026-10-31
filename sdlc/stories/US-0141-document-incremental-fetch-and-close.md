# US-0141: document incremental fetch and close the spec

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0140
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
Document the negotiation in the project docs and close the spec: the full acceptance set and the requirement coverage audit.

## Requirements covered
- SPEC-0067/FR-NEW-001 (partial: documentation of the acknowledgements): WHEN a client states wanted and held commits without ending the negotiation, THE git service SHALL answer with the acknowledgement lines of the reference answer, in the same order, and no object data.
- SPEC-0067/FR-NEW-002 (partial: documentation of the object set): WHEN the client ends the negotiation, THE git service SHALL answer with the acknowledgement lines and the closing line of the reference answer, followed by the same set of objects as the reference answer.

## Independent Test
`make check` is green; `rg -n 'no have/want negotiation' .agent_docs` prints nothing; the full acceptance set (E2E-001 to E2E-053) is green.

## Design excerpt
### Decisions
- DEC-001: implement the negotiation in a new module over libgit2 (revwalk `hide`, packbuilder). Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-002. Rationale: reuses what the service already uses to build packs. Alternatives considered: DEC-002, DEC-003. Source: design. Evidence: `crates/core/src/git/http/mod.rs:460-494`.
### Data model
None.
### Contracts
No code contract.
### Paths
| Path | Action |
|---|---|
| `.agent_docs/git.md` | modify |
| `.agent_docs/git-divergences.md` | modify |
| `AGENTS.md` | modify |
### Existing patterns
`.agent_docs/git.md:141-152` (the paragraph to replace), `.agent_docs/git-divergences.md` header (line 3 to 5).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

No new test. The full acceptance set of SPEC-0067 (E2E-001 to E2E-053) must be green through `make check`.

## Constraints
- Files not to touch: `crates/core/src/git/http/mod.rs` route and access code (`:96-260`), `crates/core/src/git/http/mod.rs` receive-pack code, any `spec.md`.
- Dependencies not to add: none (git2, axum, tokio, reqwest are already in the manifest).
- Patterns to avoid: running the `git` program in production code (DEC-002); dashes as punctuation in code, comments or docs.
- Scope boundary: behaviors of later stories (see Depends on and the story list in the design) stay as they are today.

## Definition of done
- [ ] RED evidence recorded. Write the story's tests first and record them failing for the stated reason before the production code (a failing assertion, not a compile error).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing tests of `crates/core/src/git/http/mod.rs` (`parses_wants_and_haves`, `parses_an_empty_upload_pack_request`, `upload_pack_without_wants_answers_a_bare_flush`, the `handle_upload_pack` round trips) and the three real clones of `crates/core/tests/full_stack_e2e.rs` keep passing unchanged.
- [ ] Docs updated: `.agent_docs/git.md`, `.agent_docs/git-divergences.md`, `AGENTS.md` (the paragraph "There is no have/want negotiation" is replaced by the negotiation description).
- [ ] Self review: full (complexity of the critical path stated: the answer is O(included commits + objects); no nested loop over wants and haves beyond O(W x H) reachability checks, bounded by the number of wanted ids).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001 to E2E-053) and every requirement of SPEC-0067 cited by at least one story with status done or landed.
- [ ] One commit `US-0141: document incremental fetch and close the spec`, body `Spec: SPEC-0067` and the requirement ids.
