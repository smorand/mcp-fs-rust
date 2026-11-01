# US-0143: pin the ready line that names an unknown id

> Spec: SPEC-0068 (sdlc/specs/SPEC-0068-unknown-ids-in-ready-lines/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
SPEC-0067 contradicted git's own server about unknown held ids. SPEC-0068 settles it on git's side and the service already behaves so. This story makes that contract executable: six tests that compare the answer with git's own server, the one 20 000 object test restated, and the documentation line that called the point open.

## Requirements covered
- SPEC-0068/FR-NEW-001: WHEN a stated held id that is well formed and names no object of the project follows held commits that already satisfy every wanted commit, THE git service SHALL answer, at that point, the line `ACK <that id> ready`.
- SPEC-0068/FR-MOD-001: IF a stated held id is well formed and names no object of the project, THEN THE git service SHALL NOT send an `ACK <id> common` line for it and SHALL NOT send `ACK <last held commit> ready` at the end of the exchange.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e spec_0068` is green and compares with git's own server; with `acknowledge` mutated to skip the `ready` line for unknown ids, E2E-001, 002, 005 and 006 fail.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0068): no production change; the contract is made executable by tests.
- DEC-002 (SPEC-0068): the six new tests run on the SPEC-0067 wire harness and compare with `git upload-pack --stateless-rpc`; E2E-MOD-001 stays an integration test on the answer function.
- DEC-003 (SPEC-0068): red evidence by mutation.
### Data model
None.
### Contracts
No new contract. Helpers already in the test file: `Env::with_history(n, anonymous)`, `request(wants, haves, done)`, `bare_copy()`, `reference_answer(bare, &req)`, `our_answer(&env, &req)`, `assert_same_answer(ours, reference)`, `refs()`. Unknown ids are 40 hexadecimal characters such as `format!("{:040x}", 0x1111)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/tests/git_negotiation_e2e.rs` | add the six tests |
| `crates/core/src/git/http/mod.rs` | in `e2e_038_up_to_date_request_with_unknown_ids_besides_the_held_tip`, add the assertion that the first unknown id (`0x1111`) appears in no line |
| `.agent_docs/git.md` | the "Known gaps" sentence: BL-0118 is settled by SPEC-0068 (the unknown id may be named by a `ready` line, never by a `common` line) |
### Existing patterns
`ready_is_withheld_like_git_does` in `crates/core/tests/git_negotiation_e2e.rs` (compares `ready` cases with the reference) and `e2e_038_…` in `crates/core/src/git/http/mod.rs`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Project `demo` has commits `c1` to `c6` in a line on `main` (`Env::with_history(6, false)`); `U1`, `U2`, `U3` are well formed ids the project never held. The reference answer is the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data.

### SPEC-0068/E2E-001: unknown ids after a satisfying held commit, round not ended
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_001_…`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** a request wanting `c6`, stating `c4`, then `U1`, then `U2` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c4> common`, `ACK <U1> ready`, `ACK <U2> ready`, `NAK`, and equals the answer of git's own server
- **And** no object is sent; no `common` line names `U1` or `U2`; the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-002: an unknown id before any held commit
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_002_…`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-001
- **Given** a request wanting `c6`, stating `U1`, then `c4`, then `U2` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c4> common`, `ACK <U2> ready`, `ACK <c4>`, followed by exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** no line names `U1`; the project is unchanged
- **Priority:** High
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-003: the held commit is newer than the wanted one
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_003_…`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001
- **Given** a request wanting `c4`, stating `c6`, then `U1` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c6> common` then `NAK`, with no `ready` line, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** High
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-004: only unknown ids
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_004_…`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001
- **Given** a request wanting `c6`, stating `U1` then `U2` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK`, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** High
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-005: an unknown id between two held commits, round ended
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_005_…`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6`, stating `c3`, then `U1`, then `c4` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c3> common`, `ACK <U1> ready`, `ACK <c4> common`, `ACK <c4>`, followed by exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-006: three unknown ids after a satisfying held commit
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`, function `spec_0068_e2e_006_…`
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6`, stating `c4`, then `U1`, `U2`, `U3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer has one `ready` line per unknown id, in the order stated (`U1`, `U2`, `U3`), between `ACK <c4> common` and the final `NAK`, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and servers are dropped with the test.

### SPEC-0068/E2E-MOD-001: up to date request with unknown ids, on 20 000 objects (was SPEC-0067/E2E-038)
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs` (`mod tests`, the existing `e2e_038_…`)
- **Validated before:** the unknown ids "appear in no line" (SPEC-0067 E2E-038 Then/And), while the test itself expects a `ready` line naming `U2`.
- **Validates now:** unknown ids are never named by a `common` line, and an unknown id met after the held tip is named by a `ready` line.
- **Given** a project of 20 000 objects with tip `T`; a request wanting `T`, stating `U1`, then `T`, then `U2` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <T> common`, `ACK <U2> ready`, `ACK <T>`, carries 0 objects, and arrives in under 1 second
- **And** no line names `U1`
- **Priority:** High
- Cleanup: temp directories and servers are dropped with the test.

## Constraints
- Files not to touch: `crates/core/src/git/http/negotiate.rs` and any other production file, any `spec.md`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: BL-0117 and BL-0119 stay open.

## Definition of done
- [ ] RED evidence recorded: the mutations of DEC-003 turn the listed tests red; revert each mutation.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all of `git_negotiation_e2e` and the existing tests of `crates/core/src/git/http/mod.rs` keep passing unchanged (the one assertion added to `e2e_038_…` excepted).
- [ ] Docs updated: `.agent_docs/git.md`.
- [ ] Self review: full (test code only).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001 to E2E-006, E2E-MOD-001) and every requirement of SPEC-0068 cited by at least one story with status done or landed.
- [ ] One commit `US-0143: pin the ready line that names an unknown id`, body `Spec: SPEC-0068` and the requirement ids.
