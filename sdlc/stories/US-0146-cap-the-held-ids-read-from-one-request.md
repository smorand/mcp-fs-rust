# US-0146: cap the held ids read from one request

> Spec: SPEC-0070 (sdlc/specs/SPEC-0070-cap-on-held-ids-per-request/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0145
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
A request can state any number of held ids and the service reads all of them. Read at most 5 000, in order of appearance, treat the rest as not stated, and prove the bound, the unchanged small requests and the one second limit.

## Requirements covered
- SPEC-0070/FR-NEW-001: WHEN a request states more than 5 000 held ids, THE git service SHALL read the first 5 000 in the order they appear and SHALL treat the others as not stated, without reporting an error.
- SPEC-0070/FR-NEW-002: WHEN a request states 5 000 held ids or fewer, THE git service SHALL read all of them and answer as before this change.
- SPEC-0070/NFR-001: WHEN a request states 20 000 held ids, THE git service SHALL answer in under 1 second.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e spec_0070` is green; with the cap set to 4 999, E2E-001 fails; without the cap, E2E-002 and 004 fail.

## Design excerpt
### Decisions
- DEC-001: the cap is a constant of 5 000 held ids, applied in `parse`. Obliged by: SPEC-0070/FR-NEW-001, SPEC-0070/FR-NEW-002. Rationale: the cheapest place; later stages need no change. Alternatives considered: cap inside `acknowledge` (rejected: the `ready` rule and the unknown-id counting would still see the full list); a setting (rejected: spec 3.3). Source: spec. Evidence: `crates/core/src/git/http/negotiate.rs:46-47`.
- DEC-002: the cap counts lines in order of appearance and does not deduplicate. Obliged by: SPEC-0070/FR-NEW-001. Rationale: the spec says 6 000 repeats give 5 000 acknowledgements; deduplicating would change SPEC-0067's behavior for repeats. Alternatives considered: count distinct ids (rejected by the spec). Source: spec. Evidence: spec FR-NEW-001 business rules.
- DEC-003: time bound by construction: parse is linear in lines, the lookups are at most 5 000; NFR-001 is checked by tests E2E-003, 007, 008 on a small project, one second in a development build. Obliged by: SPEC-0070/NFR-001. Rationale: the same measure as SPEC-0066 and SPEC-0067. Alternatives considered: a benchmark harness (rejected: overkill). Source: spec. Evidence: `.agent_docs/testing.md` (timing tests).
### Data model
None.
### Contracts
`const MAX_HAVE_LINES: usize = 5_000;` in `negotiate.rs`; in `parse`, `if req.haves.len() < MAX_HAVE_LINES { req.haves.push(..) }` and nothing else changes. Harness: a request builder from a list of ids of any length (the existing `request(wants, haves, done)` already takes a slice). Tests are named `spec_0070_e2e_NNN_...`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify (constant, `parse`, one unit test) |
| `crates/core/tests/git_negotiation_e2e.rs` | modify (eight tests) |
| `.agent_docs/git.md` | modify (the cap replaces the "no limit" sentence) |
### Existing patterns
`negotiate.rs:33-60` (`parse`); the SPEC-0067 test `e2e_011_twelve_unknown_held_commits` for a long list of unknown ids.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Project `demo` has commits `c1` to `c6` in a line on `main` (`Env::with_history(6, false)`) unless a test says otherwise. The reference answer is the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data. The harness is `crates/core/tests/git_negotiation_e2e.rs` (see `.agent_docs/testing.md`).

### SPEC-0070/E2E-001: exactly the limit, the held commit last
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 4 999 well formed ids the project never held, then `c4`, as 5 000 held ids in all, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c4> common`, `ACK <c4>`, then exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** no error is reported; the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-002: one over the limit, the held commit last
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 5 000 well formed ids the project never held, then `c4`, as 5 001 held ids in all, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, as if `c4` had not been stated
- **And** no error is reported; no line names `c4`; the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-003: 20 000 unknown held ids
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 20 000 well formed ids the project never held, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, and arrives in under 1 second
- **And** no error is reported; the project is unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-004: the held commit repeated 6 000 times
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c4` 6 000 times as held, ending the negotiation
- **When** the request is sent
- **Then** the answer holds exactly 5 000 lines `ACK <c4> common`, then the line `ACK <c4>`, then exactly the 6 objects of `c5` and `c6`
- **And** no error is reported; the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-005: a real fetch is unaffected
- Level / Driver / Location: functional e2e / real git client / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a copy made by a real git client that then made 40 local commits; the project gains `c7`
- **When** the developer runs `git fetch`
- **Then** the copy holds `c7` on `origin/main` and keeps its 40 local commits; the integrity check reports no problem
- **And** the objects received are exactly those of `c7`; the project is unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-006: a small request is read in full
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c3`, `c4` and an id the project never held as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the answer of git's own server for the same request, line for line (the held ids are `c3`, `c4` and the unknown id, in that order)
- **And** no error is reported; the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-007: the held commit repeated 20 000 times, round not ended
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c4` 20 000 times as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is 5 000 lines `ACK <c4> common`, then `ACK <c4> ready`, then `NAK`, and arrives in under 1 second
- **And** no object is sent; no error is reported; the project is unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0070/E2E-008: 20 000 ids of a folder listing
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program where git has no cap / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating the id of the top folder listing of `c4` 20 000 times as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, and arrives in under 1 second
- **And** no error is reported; the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: route and access code of `crates/core/src/git/http/mod.rs` (`:96-260`), receive-pack code, any `spec.md`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for every new test that can fail before the change; tests that guard behavior that already exists are proven by a mutation named in the story.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all of `git_negotiation_e2e` and the tests of `crates/core/src/git/http/` keep passing unchanged, except where the story names a test it adapts.
- [ ] Docs updated: `.agent_docs/git.md`.
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0070 cited by at least one story with status done or landed.
- [ ] One commit `US-0146: cap the held ids read from one request`, body `Spec: SPEC-0070` and the requirement ids.
