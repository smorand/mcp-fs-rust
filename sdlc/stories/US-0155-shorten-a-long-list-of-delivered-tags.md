# US-0155: shorten a long list of delivered tags

> Spec: SPEC-0076 (sdlc/specs/SPEC-0076-failed-pull-reports-delivered-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0154
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
A failed pull that delivered thousands of tags would print thousands of names. Name the first 20 by name and end with ` and <N> more.`.

## Requirements covered
- SPEC-0076/FR-NEW-002: IF a failed pull delivered more than 20 tags THEN THE Remote exchange context SHALL name the first 20 by name order and end the sentence with ` and <N> more.`, where N is the number of tags not named.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including the five tests below; with 25 delivered tags the sentence names `t01` to `t20` and ends ` and 5 more.`.

## Design excerpt
### Decisions
- DEC-003: the cap is the constant `MAX_NAMED_DELIVERED_TAGS = 20`, added in the second story. Obliged by: SPEC-0076/FR-NEW-002. Source: spec business rule.
- DEC-005: two stories: US-0154 (the sentence without the cap, hook, ten tests), US-0155 (the cap, five tests, docs, closes the spec). Obliged by: n/a (no code impact): partitioning only. Source: design.
### Data model
None.
### Contracts
`const MAX_NAMED_DELIVERED_TAGS: usize = 20;` in `git.rs`; `delivered_tags_sentence` names `added[..20]` joined by `, ` then ` and <rest> more.` when `added.len() > 20`; exactly 20 names no suffix.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (constant, sentence function, five tests) |
| `.agent_docs/git.md` | modify (one sentence in the pull section) |
### Existing patterns
the function written in US-0154; tags built in a loop with `plain_tag_bare_remote` as in `e2e_tag_048_two_thousand_tags` (`git.rs:25901`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0076/E2E-006: 25 tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `t01` to `t25` and the merge fails. **When** Alice pulls. **Then** the refusal ends with the sentence of SC-002 naming `t01` to `t20` and ` and 5 more.`
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-007: exactly 20 tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `t01` to `t20`. **When** Alice pulls and the merge fails. **Then** the sentence names all 20 and has no `and`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-011: the shortened list is sorted
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `t25` and `t01` to `t24` in that order and the merge fails. **Then** the sentence names `t01` to `t20`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-012: a shortened list leaves the others delivered
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-006. **Then** all 25 tags are listed in the project.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-015: boundary of 21
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `t01` to `t21` and the merge fails. **Then** the sentence names `t01` to `t20` and ends ` and 1 more.`
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for E2E-006, 011, 015 (uncapped before the change); E2E-007 and E2E-012 guard the boundary and the delivered tags and are proven by the mutation "cap at 19" (E2E-007 fails).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] Docs updated: `.agent_docs/git.md` (the failed pull sentence and its cap).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0076 cited by at least one story with status done or landed.
- [ ] One commit `US-0155: shorten a long list of delivered tags`, body `Spec: SPEC-0076` and the requirement ids.
