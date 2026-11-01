# US-0144: announce only the options the service honours

> Spec: SPEC-0069 (sdlc/specs/SPEC-0069-honour-the-options-the-client-chose/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0143
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The service announces `multi_ack`, which it does not honour. Announce exactly `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and the agent text, and pin by tests that real clients, the announced list, an empty project and unoffered options (plain `multi_ack`, `thin-pack`, `include-tag`) behave as the spec says.

## Requirements covered
- SPEC-0069/FR-MOD-001: IF the first wanted line of a request carries an option other than `multi_ack_detailed`, `side-band-64k`, `ofs-delta` or `agent`, THEN THE git service SHALL ignore that option and answer the request as for the same request without it.
- SPEC-0069/FR-NEW-001: THE git service SHALL announce to a client exactly the options `multi_ack_detailed`, `side-band-64k` and `ofs-delta`, followed by its agent text `agent=mcp-fs/0.1.0`, for a project with commits and for a project without.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e spec_0069` is green; with the constant put back to the old text, E2E-002 and E2E-006 fail.

## Design excerpt
### Decisions
- DEC-001: announce `multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0` and honour exactly those; plain `multi_ack` is dropped, not implemented. Obliged by: SPEC-0069/FR-NEW-001, SPEC-0069/FR-MOD-001. Rationale: git requires the detailed form over HTTP, so the plain form is dead weight. Alternatives considered: implement `ACK <id> continue` (rejected in the spec, owner decision). Source: spec. Evidence: `crates/core/src/git/http/mod.rs:61-68` (the comment already says detailed is required).
- DEC-005: the tests that guard behavior that already exists (E2E-001, 004, 007) get their red evidence by mutation: ask for the plain form in the answer, or announce `multi_ack` again. Obliged by: SPEC-0069/FR-MOD-001. Rationale: a test cannot be red before a change that is not needed. Alternatives considered: none. Source: design.
### Data model
None.
### Contracts
`UPLOAD_PACK_CAPABILITIES` becomes `"multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0"`; its doc comment (`mod.rs:61-66`) and the module header (`:19-20`) stop saying `multi_ack` is announced. Harness additions: a reader of the options announced with the first ref (GET `/git/<project>/info/refs?service=git-upload-pack`, the text after the NUL byte of the first pkt-line, up to the newline) and a request builder with free options. Tests are named `spec_0069_e2e_NNN_...`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/mod.rs` | modify (constant and comments) |
| `crates/core/tests/git_negotiation_e2e.rs` | modify (helpers and five tests) |
### Existing patterns
`crates/core/src/git/http/mod.rs:61-68`; harness helpers `request`, `request_with_option`, `reference_answer`, `our_answer`, `assert_same_answer` in `crates/core/tests/git_negotiation_e2e.rs`; the refs advertisement is `advertisement` at `mod.rs:359`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Project `demo` has commits `c1` to `c6` in a line on `main` (`Env::with_history(6, false)`) unless a test says otherwise. The reference answer is the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data. The harness is `crates/core/tests/git_negotiation_e2e.rs` (see `.agent_docs/testing.md`).

### SPEC-0069/E2E-001: a real client still clones and fetches
- Level / Driver / Location: functional e2e / real git client or hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a developer with a real git client
- **When** the developer runs `git clone` on `demo`, then the project gains `c7` and the developer runs `git fetch`
- **Then** the copy holds 7 commits, `origin/main` is at `c7` and the integrity check reports no problem
- **And** the client, which also offers options the service does not (such as `thin-pack` and `no-done`), gets no error; the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-002: the options the service offers
- Level / Driver / Location: functional e2e / real git client or hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** requirement; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`
- **When** a client reads the project's list of branches and tags before fetching
- **Then** the options announced with the first branch are exactly `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and the text `agent=mcp-fs/0.1.0`, and `multi_ack` is not among them
- **And** the branches and tags listed are unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-004: a client that asks only for the plain multi_ack
- Level / Driver / Location: functional e2e / real git client or hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c4` as held, asking for `multi_ack` and `side-band-64k` (not `multi_ack_detailed`), ending the negotiation
- **When** the request is sent
- **Then** the answer equals the answer to the same request asking for `multi_ack_detailed`: `ACK <c4> common`, `ACK <c4>`, then the 6 objects of `c5` and `c6`
- **And** no line is of the form `ACK <id> continue`; no error is reported; the project is unchanged
- **Priority:** High
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-006: the options of an empty project
- Level / Driver / Location: functional e2e / real git client or hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a project with no commit
- **When** a client reads the project's list of branches and tags
- **Then** the options announced are exactly `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and `agent=mcp-fs/0.1.0`
- **And** no branch is listed
- **Priority:** Medium
- Cleanup: temp directories are dropped with the test.

### SPEC-0069/E2E-007: several options the service does not offer
- Level / Driver / Location: functional e2e / real git client or hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` asking for `multi_ack`, `multi_ack_detailed`, `side-band-64k`, `thin-pack` and `include-tag`, stating `c4` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer equals the answer to the same request asking for `multi_ack_detailed` and `side-band-64k` only
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
- [ ] Docs updated: none in this story (US-0145).
- [ ] Self review: full.

- [ ] One commit `US-0144: announce only the options the service honours`, body `Spec: SPEC-0069` and the requirement ids.
