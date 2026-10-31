# US-0137: answer the rounds that do not end the negotiation

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0136
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A round that does not end the negotiation gets acknowledgement lines only. This story adds `ready` and `NAK` for requests without the end marker, the way git's server does, including the case where every have is unusable.

## Requirements covered
- SPEC-0067/FR-NEW-001: WHEN a client states wanted and held commits without ending the negotiation, THE git service SHALL answer with the acknowledgement lines of the reference answer, in the same order, and no object data.
- SPEC-0067/FR-NEW-004: IF a held id is not a well formed object id or is the id of no object of the project, THEN THE git service SHALL treat it as not stated and SHALL NOT report an error.
- SPEC-0067/FR-NEW-007: IF a held id is a well formed id of an object the project holds and that object is not a commit, THEN THE git service SHALL NOT acknowledge it and SHALL treat it as not stated, without reporting an error.
- SPEC-0067/FR-NEW-011: WHEN a request wants at least one valid commit, contains at least one `have` line none of which is acknowledged, and does not end the negotiation, THE git service SHALL answer as the reference answer does: the closing acknowledgement line and no object data.

## Independent Test
The story's four tests are green; E2E-010 compares the three lines `ACK <c3> common`, `ACK <c4> common`, `NAK` (plus `ready` when the reference sends it) with the reference program's.

## Design excerpt
### Decisions
- DEC-004: acknowledgements follow git's `get_common_commits` (stateless, `multi_ack_detailed`): one `ACK <id> common` per have the project holds as a commit, repeats included; at the end of the haves, without `done`: `ACK <last> ready` when at least one held commit, no unknown well formed have, and every wanted commit reaches a held commit, then `NAK`; with `done`: `ACK <last>` or `NAK`, then the objects. Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-011. Rationale: verified on 2026-10-31 against git 2.50.1: duplicates are acknowledged twice; an unknown have suppresses `ready`; a want that does no
- DEC-005: declared differences are applied while reading the haves: a have that is not 40 hexadecimal characters is skipped; a well formed id of an object that is not a commit is skipped; neither acknowledges nor suppresses `ready`; a well formed id the project does not hold suppresses `ready` and is not acknowledged (as the reference). Obliged by: SPEC-0067/FR-NEW-004, FR-NEW-007. Rationale: matches "treat as not stated" for the two cases the spec declares, and the reference for the rest. Alternatives considered: treat unknown well formed ids as skipped too (rejected: differs from the refere
### Data model
None.
### Contracts
`negotiate::answer`: without `done`, after the common lines, `ACK <last> ready` when (DEC-004) then `NAK`, and nothing else.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
### Existing patterns
The `answer` function of US-0136 and its tests in the e2e file.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-010: negotiation stopped before the end
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6` and stating `c3` then `c4` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer line by line and carries no object data
- **And** the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-041: unusable held ids, no end of negotiation
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** a request wanting `c6` and stating 12 well formed ids the project never held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer: the closing line and no object data
- **And** the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-042: held id of a file version, no end of negotiation
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-007
- **Given** a request wanting `c6` and stating only the id of `a.txt` in `c4` as held, with no end of negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id; the answer equals the reference answer and carries no object data
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-043: malformed held id, no end of negotiation
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-011, FR-NEW-004
- **Given** a request wanting `c6` and stating the text `not-an-id` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer carries no object and no error
- **And** the id appears in no line of the answer
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

## Constraints
- Files not to touch: `crates/core/src/git/http/mod.rs` route and access code (`:96-260`), `crates/core/src/git/http/mod.rs` receive-pack code, any `spec.md`.
- Dependencies not to add: none (git2, axum, tokio, reqwest are already in the manifest).
- Patterns to avoid: running the `git` program in production code (DEC-002); dashes as punctuation in code, comments or docs.
- Scope boundary: behaviors of later stories (see Depends on and the story list in the design) stay as they are today.

## Definition of done
- [ ] RED evidence recorded. Write the story's tests first and record them failing for the stated reason before the production code (a failing assertion, not a compile error).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing tests of `crates/core/src/git/http/mod.rs` (`parses_wants_and_haves`, `parses_an_empty_upload_pack_request`, `upload_pack_without_wants_answers_a_bare_flush`, the `handle_upload_pack` round trips) and the three real clones of `crates/core/tests/full_stack_e2e.rs` keep passing unchanged.
- [ ] Docs updated: none in this story (US-0141).
- [ ] Self review: full (complexity of the critical path stated: the answer is O(included commits + objects); no nested loop over wants and haves beyond O(W x H) reachability checks, bounded by the number of wanted ids).

- [ ] One commit `US-0137: answer the rounds that do not end the negotiation`, body `Spec: SPEC-0067` and the requirement ids.
