# US-0138: serve only wanted ids a branch or tag reaches

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0137
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A wanted id is served only when a branch or tag reaches it. This story drops unservable wanted ids (answering `NAK` and zero objects when none is left) and sends annotated tags with the objects their commit needs.

## Requirements covered
- SPEC-0067/FR-NEW-002: WHEN the client ends the negotiation, THE git service SHALL answer with the acknowledgement lines and the closing line of the reference answer, followed by the same set of objects as the reference answer.
- SPEC-0067/FR-NEW-008: IF a wanted id is malformed, is the id of no object of the project, or is the id of an object that no branch or tag reaches, THEN THE git service SHALL send no data for it and SHALL NOT report an error.
- SPEC-0067/FR-NEW-013: IF every wanted id of a request is malformed, is the id of no object of the project, or is the id of an object no branch or tag reaches, THEN THE git service SHALL answer as it does today: the single line `NAK`, then an answer with zero objects, whatever the held ids and whether or not the client ended the negotiation, and SHALL NOT report an error.

## Independent Test
The story's tests are green; E2E-030 shows an orphan commit answered with zero objects while the same commit reached by a branch is served.

## Design excerpt
### Decisions
- DEC-006: a wanted id is servable when it exists and either equals the value of a branch or tag, or is a commit reachable from one (`graph_descendant_of` over the ref values), or is a tree or file version contained in a commit reachable from one. Unservable wanted ids are dropped; if none is left the answer is `NAK` and an empty pack. A wanted annotated tag adds the tag object and continues with its target. Obliged by: SPEC-0067/FR-NEW-008, FR-NEW-013. Rationale: the reference refuses unreachable wants; the spec declares silence instead. Alternatives considered: serve any held object as today
### Data model
None.
### Contracts
`negotiate::answer`: wanted ids filtered by DEC-006 before anything else; an annotated tag want adds the tag object (`PackBuilder::insert_object`) and continues with its target.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
### Existing patterns
`crates/core/src/git/http/mod.rs:471-480` (today's skip of unknown wants and the non commit branch with `insert_recursive`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-008: annotated tag on a held commit
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a copy at `c4`; the project gains annotated tag `v1` (message `first release`) on `c4`
- **When** the developer runs `git fetch --tags`
- **Then** the copy has tag `v1` with message `first release` pointing at `c4`
- **And** the objects sent are the same set as the reference answer's (the tag only)
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-009: annotated tag on a new commit
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a copy at `c4`; the project gains `c5` (adds `d.txt`) and annotated tag `v2` on `c5`
- **When** the developer runs `git fetch --tags`
- **Then** the copy has `v2` pointing at `c5` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's (the tag, `c5`, its folder listing, `d.txt`)
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-030: wanted commit that no branch reaches
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** `demo` at `c4`, plus an orphan commit `o1` that no branch or tag reaches; a request wanting `o1`, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects and no error
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-047: unreachable wanted commit with a held id, no end
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-013
- **Given** `demo` at `c4` and an orphan commit `o1` that no branch or tag reaches; a request wanting `o1`, stating `c3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects, no `ACK` line for `c3` and no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-048: malformed wanted id with a held id
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-013
- **Given** a request wanting the text `xyz`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` with zero objects, no `ACK` line for `c3` and no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-049: one servable and one unservable wanted id
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-013
- **Given** a request wanting `c6` and the orphan commit `o1`, stating `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer for the request wanting only `c6`
- **And** nothing of `o1` is sent
- **Priority:** Medium
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

- [ ] One commit `US-0138: serve only wanted ids a branch or tag reaches`, body `Spec: SPEC-0067` and the requirement ids.
