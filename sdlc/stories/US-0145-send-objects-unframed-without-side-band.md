# US-0145: send the objects unframed without the side band

> Spec: SPEC-0069 (sdlc/specs/SPEC-0069-honour-the-options-the-client-chose/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0144
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
A client that does not ask for `side-band-64k` receives the objects framed on a side band it did not ask for. Record the choice from the first wanted line and write the objects raw, with nothing after them, when it was not asked for. Update the docs that described the options as open.

## Requirements covered
- SPEC-0069/FR-NEW-002: WHEN a client does not ask for `side-band-64k`, THE git service SHALL send the objects right after the acknowledgement lines, with no framing, no progress text and no closing byte.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e spec_0069` is green; E2E-003, 005 and 008 fail before the change (the answer carries channel bytes).

## Design excerpt
### Decisions
- DEC-002: the side band choice is read from the first wanted line's words after the id. Obliged by: SPEC-0069/FR-NEW-002. Rationale: git sends its options only on the first wanted line; `parse` already takes the id as the first word of that line. Alternatives considered: scanning every wanted line (rejected: not the protocol). Source: design. Evidence: `crates/core/src/git/http/negotiate.rs:45`.
- DEC-003: without the side band the objects are appended as they are and nothing follows: no closing byte, no flush. Obliged by: SPEC-0069/FR-NEW-002. Rationale: that is git's own server's output (checked with git 2.50.1 on 2026-11-01) and the client reads the pack to its checksum. Alternatives considered: keep the closing flush (rejected: bytes after a raw pack break clients that read to the end). Source: reference program. Evidence: spec 14.1 row 5.
### Data model
None.
### Contracts
`Request.side_band: bool` set by `parse` from the words after the id of the first `want` line. `finish(repo, wants, commons, out)` keeps its signature plus the flag (or reads it from `Request`); with the flag false it appends the pack bytes unchanged and returns, with no closing byte and no flush. The empty pack goes out raw too. Harness: an answer parser for the unframed form (read pkt-lines until the next bytes are `PACK`, the rest is the pack).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify (`Request`, `parse`, `finish`) |
| `crates/core/tests/git_negotiation_e2e.rs` | modify (parser, three tests) |
| `.agent_docs/git.md` | modify (Negotiation section: options, unframed form, known gaps without BL-0119) |
| `.agent_docs/git-divergences.md` | modify (D18: options list) |
### Existing patterns
`negotiate.rs:105-125` (`finish`) and `:33-60` (`parse`); `parse_answer` in the e2e file for the framed form.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Project `demo` has commits `c1` to `c6` in a line on `main` (`Env::with_history(6, false)`) unless a test says otherwise. The reference answer is the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data. The harness is `crates/core/tests/git_negotiation_e2e.rs` (see `.agent_docs/testing.md`).

### SPEC-0069/E2E-003: a client that does not ask for the side band
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c4` as held, asking for `multi_ack_detailed` and `ofs-delta` but not `side-band-64k`, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c4> common`, `ACK <c4>`, then the 6 objects of `c5` and `c6` unframed, the same lines and the same objects as git's own server gives for that request
- **And** the answer carries no channel byte, no progress text and no closing byte after the objects; the project is unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-005: an up to date client without the side band
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c6` as held, asking for `multi_ack_detailed` but not `side-band-64k`, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c6> common`, `ACK <c6>`, then an empty set of objects unframed, as git's own server gives
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-008: a clone larger than one packet without the side band
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a project holding one commit whose file is 400 KB of text, and a request wanting that commit, asking for `multi_ack_detailed` but not `side-band-64k`, with no held commit, ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` followed by all 3 objects of the commit unframed, the same objects as git's own server gives
- **And** the objects arrive in one piece with no length prefix every 65 515 bytes; the project is unchanged
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
- [ ] Docs updated: `.agent_docs/git.md`, `.agent_docs/git-divergences.md`.
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0069 cited by at least one story with status done or landed.
- [ ] One commit `US-0145: send the objects unframed without the side band`, body `Spec: SPEC-0069` and the requirement ids.
