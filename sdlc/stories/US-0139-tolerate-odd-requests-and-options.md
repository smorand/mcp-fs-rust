# US-0139: tolerate odd requests and options the service does not offer

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0138
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Odd requests and options the service does not offer get today's tolerant answers, and a request that ends early or lacks a flush is read as git's server would read it complete.

## Requirements covered
- SPEC-0067/FR-NEW-003: WHEN the request contains no `have` line, THE git service SHALL send every object reachable from the wanted commits, whether or not the client has ended the negotiation.
- SPEC-0067/FR-NEW-009: IF a request carries a shallow, deepen or filter line, THEN THE git service SHALL answer the whole request as it does today: the single acknowledgement line `NAK`, no `ACK` line, no `shallow` line, and every object reachable from the wanted commits, whatever the held ids and whether or not the client ended the negotiation.
- SPEC-0067/FR-NEW-010: IF a request is empty, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request ends in the middle of a line, THEN THE git service SHALL discard the cut line and everything after it and answer the lines before it by every other requirement, with no error. IF a request, once cut, contains no wanted line, or contains none at all, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request carries a shallow, deepen or filter line and contains no wanted line, THEN THE git service SHALL answer as it does today for a request that wants nothing (a bare flush, no `NAK`), this requirement taking precedence over FR-NEW-009.
- SPEC-0067/FR-NEW-012: IF the first wanted line of a request carries an option other than `multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta` or `agent`, THEN THE git service SHALL ignore that option and answer the request as it would without it.
- SPEC-0067/FR-NEW-014: IF the reference program rejects or stops on a request for a reason that no other declared difference covers (a missing flush after the wanted lines, a request that ends between two lines, lines in an unexpected order), THEN THE git service SHALL treat the lines present as complete, read a missing flush as present, end the negotiation only if the request contains the end marker, answer by every other requirement, and SHALL NOT report an error.

## Independent Test
The story's tests are green; E2E-053 shows a request with no flush answered like the same request completed.

## Design excerpt
### Decisions
- DEC-001: implement the negotiation in a new module over libgit2 (revwalk `hide`, packbuilder). Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-002. Rationale: reuses what the service already uses to build packs. Alternatives considered: DEC-002, DEC-003. Source: design. Evidence: `crates/core/src/git/http/mod.rs:460-494`.
- DEC-005: declared differences are applied while reading the haves: a have that is not 40 hexadecimal characters is skipped; a well formed id of an object that is not a commit is skipped; neither acknowledges nor suppresses `ready`; a well formed id the project does not hold suppresses `ready` and is not acknowledged (as the reference). Obliged by: SPEC-0067/FR-NEW-004, FR-NEW-007. Rationale: matches "treat as not stated" for the two cases the spec declares, and the reference for the rest. Alternatives considered: treat unknown well formed ids as skipped too (rejected: differs from the refere
### Data model
None.
### Contracts
`negotiate::parse`: shallow, deepen (any `deepen*` form) and filter lines set `option_line`; the wanted line's capabilities after the first space are ignored; a cut last line (`last_was_cut`) is dropped; a missing flush is read as present. `negotiate::answer`: `option_line` with wanted lines gives `NAK` plus the full set of objects reachable from the wanted commits; a request with no wanted line gives a bare flush.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify |
| `crates/core/src/git/http/pktline.rs` | modify |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
### Existing patterns
`crates/core/src/git/http/pktline.rs:77-113` (`read_packet`: a short or non hex packet returns an end, a cut packet returns what arrived).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-018: request cut in the middle of a line
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request wanting `c6`, stating `c4` as held, then a second held line interrupted partway; the request ends there
- **When** the request is sent
- **Then** the answer equals the answer to the same request cut before the cut line (here: the reference answer for wanting `c6` and holding `c4` with no end of negotiation), with no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-031: deepen option with a held id
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, adding the line `deepen 1`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line and no `shallow` line
- **And** it carries every object reachable from `c4`; the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-035: filter option
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, adding the line `filter blob:none`, stating `c3` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line
- **And** it carries every object reachable from `c4`
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-036: shallow line from the client
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009
- **Given** `demo` with `c1` to `c4`; a request wanting `c4`, stating a line `shallow` followed by the id of `c3`, then ending the negotiation
- **When** the request is sent
- **Then** the answer's only acknowledgement line is `NAK`; it has no `ACK` line and no `shallow` line
- **And** it carries every object reachable from `c4`
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-037: held ids but nothing wanted
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request stating `c4` as held and ending the negotiation, with no wanted line
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing, with no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-044: option `no-done` that the service does not offer
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** a request wanting `c4`, carrying the option `no-done` and stating `c3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer has the acknowledgement lines of the same request without `no-done`, and no object data
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-045: option `include-tag` that the service does not offer
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** annotated tag `v1` on `c3`; a request wanting `c4` and carrying the option `include-tag`, then ending the negotiation
- **When** the request is sent
- **Then** the objects sent are exactly those the same request without `include-tag` sends
- **And** the tag object is not among them
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-046: an unknown option
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012
- **Given** a request wanting `c4` and carrying the option `x-foo`, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the same as for the request without `x-foo`
- **And** no error is reported
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-050: deepen line with no wanted line
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-009, FR-NEW-010
- **Given** a request carrying the line `deepen 1` and stating `c3` as held, with no wanted line, then ending the negotiation
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing, with no `NAK` line and no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-051: no flush after the wanted line
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-014
- **Given** a request wanting `c6` and stating `c4` as held, with no flush after the wanted line and no end marker
- **When** the request is sent
- **Then** the answer equals the reference answer for the same request completed with the flush after the wanted line, with no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-052: end marker without a flush after the wanted line
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-014
- **Given** a request wanting `c6` and stating `c4` as held, then the end marker, with no flush after the wanted line
- **When** the request is sent
- **Then** the answer equals the reference answer for the same request completed with the flush after the wanted line, and carries every object of `c5` and `c6`
- **And** no error is reported
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-053: wanted line only, no flush
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-014, FR-NEW-003
- **Given** a request wanting `c6` only, with no flush and no held id
- **When** the request is sent
- **Then** the answer equals the answer to the same request completed with the flush after the wanted line: every object reachable from `c6`
- **And** no error is reported
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

- [ ] One commit `US-0139: tolerate odd requests and options the service does not offer`, body `Spec: SPEC-0067` and the requirement ids.
