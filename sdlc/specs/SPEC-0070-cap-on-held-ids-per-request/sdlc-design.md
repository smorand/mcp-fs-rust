# SPEC-0070: Cap on held ids per request, design

> Spec: sdlc/specs/SPEC-0070-cap-on-held-ids-per-request/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-01
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust. Harness: SPEC-0067's.

## 2. Approaches
### Chosen: stop storing `have` lines after the 5 000th in `parse`
One constant, one comparison in the loop that already reads the lines. `acknowledge` and the `ready` rule then see at most 5 000 ids with no further change. Pros: one place, linear parse, no new error path. Cons: the lines beyond the cap are still parsed (cheap) so the end marker and option lines after them keep counting. Scope: `git/http/negotiate.rs`.
### Rejected: refuse an oversized request
Rejected in the spec (owner decision): the fetch has to keep working.

## 3. Architecture
`parse` (`negotiate.rs:33-60`) pushes held ids at `:46-47`; `acknowledge` (`:221-266`) does one object lookup per id. The cap bounds the lookups.

## 4. Data model
None.

## 5. Contracts and interfaces
`const MAX_HAVE_LINES: usize = 5_000;` in `negotiate.rs`; `parse` keeps the first `MAX_HAVE_LINES` held ids (in order of appearance in the whole request) and drops the others. No signature changes. The limit is not configurable here (spec 3.3).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/http/negotiate.rs` | modify (constant, one condition in `parse`, a unit test) | FR-NEW-001, FR-NEW-002 | US-0146 |
| `crates/core/tests/git_negotiation_e2e.rs` | modify (8 tests, a builder for long lists) | all | US-0146 |
| `.agent_docs/git.md` | modify (the sentence "No limit applies ... (BL-0117)" becomes the cap) | FR-NEW-001 | US-0146 |
Affected existing tests: keep all. Documentation: `.agent_docs/git.md`.

## 7. Compatibility and risks
Requests with 5 000 held ids or fewer are unchanged. Above it, held ids beyond the 5 000th stop counting: the client may receive more objects than needed, never fewer (an id not read is an id not held). Risk: a client that legitimately states more than 5 000 loses incremental transfer for the excess; the owner accepted it. Rollback: revert.

## 8. Security design
Not applicable (`Security: n/a`). The cap is the resource bound BL-0117 asked for.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-002 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-003 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-004 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-005 | functional e2e | real git client | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-006 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-007 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
| E2E-008 | functional e2e | hand written request over HTTP, reference program where git has no cap | `crates/core/tests/git_negotiation_e2e.rs` | US-0146 |
Test data: generated; a list builder makes the 5 000, 5 001, 6 000 and 20 000 line requests.

## 10. Decisions log
- **DEC-001:** the cap is a constant of 5 000 held ids, applied in `parse`. **Obliged by:** SPEC-0070/FR-NEW-001, SPEC-0070/FR-NEW-002. **Rationale:** the cheapest place; later stages need no change. **Alternatives considered:** cap inside `acknowledge` (rejected: the `ready` rule and the unknown-id counting would still see the full list); a setting (rejected: spec 3.3). **Source:** spec. **Evidence:** `crates/core/src/git/http/negotiate.rs:46-47`.
- **DEC-002:** the cap counts lines in order of appearance and does not deduplicate. **Obliged by:** SPEC-0070/FR-NEW-001. **Rationale:** the spec says 6 000 repeats give 5 000 acknowledgements; deduplicating would change SPEC-0067's behavior for repeats. **Alternatives considered:** count distinct ids (rejected by the spec). **Source:** spec. **Evidence:** spec FR-NEW-001 business rules.
- **DEC-003:** time bound by construction: parse is linear in lines, the lookups are at most 5 000; NFR-001 is checked by tests E2E-003, 007, 008 on a small project, one second in a development build. **Obliged by:** SPEC-0070/NFR-001. **Rationale:** the same measure as SPEC-0066 and SPEC-0067. **Alternatives considered:** a benchmark harness (rejected: overkill). **Source:** spec. **Evidence:** `.agent_docs/testing.md` (timing tests).
- **DEC-004:** one story: the production change is one line, the work is the tests. **Obliged by:** n/a (no code impact): partitioning only. **Rationale:** 8 tests and 3 files fit tier 2. **Alternatives considered:** none. **Source:** design.
- **DEC-005:** red evidence for E2E-002, 003, 004, 007, 008 is the change itself (they fail before the cap); E2E-001, 005, 006 guard unchanged behavior and are proven by mutation: set the cap to 4 999 and E2E-001 fails. **Obliged by:** SPEC-0070/FR-NEW-002. **Rationale:** same reasoning as SPEC-0069 DEC-005. **Alternatives considered:** none. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0146 | cap the held ids read from one request | P1 | FR-NEW-001, FR-NEW-002, NFR-001 | US-0145 | 2 | yes |
