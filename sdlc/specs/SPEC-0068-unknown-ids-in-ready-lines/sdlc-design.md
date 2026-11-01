# SPEC-0068: Unknown held ids and the ready line, design

> Spec: sdlc/specs/SPEC-0068-unknown-ids-in-ready-lines/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-01
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
Existing project (Rust workspace, `Cargo.toml`); no library added. Skills loaded: rust (`make check` is the gate). Test harness: the one SPEC-0067 built (`crates/core/tests/git_negotiation_e2e.rs`: in process servers, `request`, `reference_answer`, `our_answer`, `assert_same_answer`).

## 2. Approaches
### Chosen: tests only
The service already answers as the spec now says (`negotiate.rs` `acknowledge` and the closing `ready` rule), so the increment is a set of guard tests that compare with git's own server, plus a documentation line. Pros: no risk to the wire behavior; the contract becomes executable. Cons: the tests pass on arrival, so their red signal is a mutation (spec 14.3). Affected scope: one test file, one test in `http/mod.rs`, one doc line.
### Rejected: hide the unknown id from `ready` lines
Rejected in the spec (Section 3.2, option B); it would add code and leave the reference rule.

## 3. Architecture
No component changes. Code under test: `acknowledge` at `crates/core/src/git/http/negotiate.rs:221-266` (one `ACK <id> ready` per unknown id after enough held commits, `:245-262`) and the closing `ready` guard `!got_unknown` at `:89`.

## 4. Data model
None.

## 5. Contracts and interfaces
No new contract. Test names carry the spec prefix because SPEC-0067 already owns `e2e_001_…` style names in the same file: `spec_0068_e2e_001_…` to `spec_0068_e2e_006_…`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/tests/git_negotiation_e2e.rs` | modify (6 tests) | FR-NEW-001, FR-MOD-001 | US-0143 |
| `crates/core/src/git/http/mod.rs` | modify (`e2e_038_…` gains the assertion that no line names `U1`) | FR-MOD-001 | US-0143 |
| `.agent_docs/git.md` | modify (the "Known gaps" sentence names BL-0118, now settled) | FR-NEW-001 | US-0143 |
Affected existing tests: 1 modified (`e2e_038_up_to_date_request_with_unknown_ids_besides_the_held_tip`: one added assertion, the expectation is unchanged); all others keep. Documentation: `.agent_docs/git.md` only.

## 7. Compatibility and risks
No consumer sees a change. Risk: a future change of git's rule (a newer git) turns the comparison tests red; that is the intended alarm. Rollback: revert the story commit.

## 8. Security design
Not applicable (`Security: n/a`).

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-002 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-003 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-004 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-005 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-006 | functional e2e | hand written request over HTTP, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0143 |
| E2E-MOD-001 | integration | direct call on 20 000 objects | `crates/core/src/git/http/mod.rs` (`mod tests`) | US-0143 |
Test data: generated (commits made with the `git` program in temp directories; ids `U1`, `U2`, `U3` are `0x1111...`, `0x2222...`, `0x3333...` written as 40 hexadecimal characters).

## 10. Decisions log
- **DEC-001:** no production change; the contract is made executable by tests. **Obliged by:** SPEC-0068/FR-NEW-001, SPEC-0068/FR-MOD-001. **Rationale:** the code already follows git; changing it would break the reference rule. **Alternatives considered:** hide the unknown id (rejected in the spec). **Source:** spec (owner chose A). **Evidence:** `crates/core/src/git/http/negotiate.rs:221-266`.
- **DEC-002:** the six new tests run on the SPEC-0067 wire harness and compare with `git upload-pack --stateless-rpc`; E2E-MOD-001 stays an integration test on the answer function. **Obliged by:** SPEC-0068/FR-NEW-001, SPEC-0068/FR-MOD-001. **Rationale:** same oracle, same helpers; the 20 000 object test cannot be pushed over the wire in a debug build (SPEC-0067 DEC-009). **Alternatives considered:** a new test file (rejected: duplicates the harness). **Source:** existing design. **Evidence:** `sdlc/specs/SPEC-0067-incremental-fetch-negotiation/sdlc-design.md` DEC-007, DEC-009.
- **DEC-003:** red evidence by mutation, as the spec's Section 14.3 lists: skip the `ready` line for unknown ids (E2E-001, 002, 005, 006 and MOD-001 fail) and send the closing `ready` after an unknown id (E2E-001, 004 fail). **Obliged by:** SPEC-0068/FR-NEW-001, SPEC-0068/FR-MOD-001. **Rationale:** the behavior exists, so a test cannot be red before a change. **Alternatives considered:** none. **Source:** spec 14.3. **Evidence:** `crates/core/src/git/http/negotiate.rs:89,259`.
- **DEC-004:** one story carries all seven tests and the doc line. **Obliged by:** n/a (no code impact): partitioning only. **Rationale:** one file, no production change, 7 tests within the tier 2 budget (5 to 15). **Alternatives considered:** two stories, new tests then the modified one (rejected: no independent value, main stays green either way). **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0143 | pin the ready line that names an unknown id | P1 | FR-NEW-001, FR-MOD-001 | main | 2 | yes |
