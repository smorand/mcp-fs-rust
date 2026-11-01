# US-0142: read the lines of a request in any order

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0141
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
The converge audit of SPEC-0067 found a gap in FR-NEW-014: lines in an unexpected order are not all read (a held id before the wanted lines, a `shallow` or `deepen` line after the closing flush), because the parser stops at the closing flush. Read every line of the request, in whatever order, up to the end marker or the end of the input.

## Requirements covered
- SPEC-0067/FR-NEW-014: IF the reference program rejects or stops on a request for a reason that no other declared difference covers (a missing flush after the wanted lines, a request that ends between two lines, lines in an unexpected order), THEN THE git service SHALL treat the lines present as complete, read a missing flush as present, end the negotiation only if the request contains the end marker, answer by every other requirement, and SHALL NOT report an error.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e` is green; the held id placed before the wanted line is acknowledged like the same request in the usual order.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0067): implement the negotiation in a new module over libgit2.
- DEC-005 (SPEC-0067): declared differences are applied while reading the haves.
### Data model
None.
### Contracts
`negotiate::parse(body: &[u8]) -> Request`: flushes no longer close the request; every packet is read until the end marker or the end of the input, and a line cut by the end of the input is dropped. Nothing else changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify (`parse`) |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
| `.agent_docs/git.md` | modify (the sentence "Lines after the closing flush or the end marker are not read") |
| `.agent_docs/git-divergences.md` | modify (D18 row, same sentence if present) |
### Existing patterns
`crates/core/src/git/http/negotiate.rs` `parse` and the tests of US-0139 (`request_with`, `pkt`, `reference_answer`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0067/FR-NEW-014 (unexpected order): a held id before the wanted line
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- Given project `demo` with `c1` to `c6`; a request stating `c4` as held first, then the wanted line for `c6`, then a flush, then the end marker
- When the request is sent
- Then the answer equals the reference answer for the usual order (wanted line, flush, held id, end marker): `ACK <c4> common`, `ACK <c4>` and exactly the objects of `c5` and `c6`
- And no error is reported and the project is unchanged

### SPEC-0067/FR-NEW-014 (unexpected order): a deepen line after the held ids
- Level / Driver / Location: functional e2e / hand written request over HTTP, reference program / `crates/core/tests/git_negotiation_e2e.rs`
- Given project `demo` with `c1` to `c4`; a request wanting `c4`, a flush, the held id `c2`, a flush, then the line `deepen 1`, then the end marker
- When the request is sent
- Then the answer's only acknowledgement line is `NAK` and it carries every object reachable from `c4` (the option still wins over the held ids, FR-NEW-009)
- And no error is reported and the project is unchanged

## Constraints
- Files not to touch: route and access code of `crates/core/src/git/http/mod.rs`, any `spec.md`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: nothing else in the answer changes; BL-0118 and BL-0119 stay open.

## Definition of done
- [ ] RED evidence recorded: both tests fail before the change (the first answers a bare flush, the second honours the held id).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all of `git_negotiation_e2e` and the existing tests of `crates/core/src/git/http/mod.rs` keep passing unchanged.
- [ ] Docs updated: `.agent_docs/git.md` (lines after the closing flush are read), `.agent_docs/git-divergences.md` if it repeats the sentence.
- [ ] Self review: full (the parse loop is O(lines)).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001 to E2E-053) and every requirement of SPEC-0067 cited by at least one story with status done or landed.
- [ ] One commit `US-0142: read the lines of a request in any order`, body `Spec: SPEC-0067` and the requirement ids.
