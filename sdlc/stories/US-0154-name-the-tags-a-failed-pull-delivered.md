# US-0154: name the tags a failed pull delivered

> Spec: SPEC-0076 (sdlc/specs/SPEC-0076-failed-pull-reports-delivered-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0153
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A pull that fails after it brought in tags says nothing about them. Append `Tags received before the failure: <names>.` to the refusal when at least one tag was added, and nothing otherwise.

## Requirements covered
- SPEC-0076/FR-NEW-001: WHEN a pull fails after it brought in at least one tag THE Remote exchange context SHALL end the refusal text with `Tags received before the failure: <names>.`
- SPEC-0076/FR-NEW-003: The Remote exchange context SHALL NOT add the sentence to a refusal when the pull delivered no tag, and SHALL NOT add it to the answer of a pull that succeeds or pauses on a conflict.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including the ten tests below; a quota refusal after two delivered tags ends with `Tags received before the failure: v1.0, v2.0.`.

## Design excerpt
### Decisions
- DEC-001: the sentence is appended to the message, a space after the usual text. Obliged by: SPEC-0076/FR-NEW-001. Rationale: the error type has no data field. Alternatives considered: a new error field (rejected by the spec). Evidence: `errors.rs:30-40`.
- DEC-002: the slot is read with `mem::take` on both paths; on success the code of `git.rs:6997` is unchanged. Obliged by: SPEC-0076/FR-NEW-003. Source: design.
- DEC-004: fault injection is a `#[cfg(test)]` hook keyed by URL, as `after_listing` is. Obliged by: SPEC-0076/FR-NEW-001 (E2E-003, E2E-010). Rationale: a merge error that is not a conflict and a deadline cannot be made from outside. Alternatives considered: a corrupted object (rejected: unreliable across libgit2 versions). Source: existing code `git_tags.rs:757-775`.
- DEC-005: two stories: US-0154 (the sentence without the cap, hook, ten tests), US-0155 (the cap, five tests, docs, closes the spec). Obliged by: n/a (no code impact): partitioning only. Source: design.
- DEC-006: red evidence: E2E-002, 003, 010 fail before the change (no sentence); E2E-001, 004, 005, 013, 014 guard unchanged text and are proven by the mutation "always append the sentence". Obliged by: SPEC-0076/FR-NEW-003. Source: design.
### Data model
None.
### Contracts
`fn delivered_tags_sentence(added: &[String]) -> Option<String>` in `git.rs` (this story: all names, no cap). The pull matches the result of `with_remote_deadline(..).await` (`git.rs:6990`): on `Err(mut e)` it takes `tags_slot`, and when the sentence exists appends `" "` and it to `e.message`. Test hook `#[cfg(test)] pub(crate) mod after_reconcile` with `PullFault::{Fail(ToolError), Stall}`, fired in the pull closure after `tags_slot` is stored (`git.rs:6823`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (sentence function, error path, hook, ten tests) |
### Existing patterns
`tags_slot` at `crates/core/src/tools/git.rs:6784-6823`, `:6995-7021`; `after_listing` hook at `git_tags.rs:757-775`; `e2e_new_141` (quota refusal after a pull) at `git.rs:12631`; `Env::with_hosts_and_timeout`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0076/E2E-001: a pull that succeeds
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice and the remote tag `v1.0`. **When** she pulls and it succeeds. **Then** `tags_added` is `["v1.0"]` and no sentence appears anywhere in the answer.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-002: quota refusal after the tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `v1.0` and `v2.0`; the project cannot take the merged files within its quota. **When** Alice pulls. **Then** the refusal is its usual text followed by ` Tags received before the failure: v1.0, v2.0.` **And** both tags are listed in the project.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-003: merge error after the tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `v1.0` and a merge that fails with an error (not a conflict). **When** Alice pulls. **Then** the refusal ends ` Tags received before the failure: v1.0.`
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-004: the remote is down
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** a remote that does not exist. **When** Alice pulls. **Then** the refusal is the connection failure text with no sentence.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-005: no tag delivered, merge fails
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers no tag and the merge fails. **When** Alice pulls. **Then** the refusal has no sentence.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-008: retry reports nothing new
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** E2E-002 and the cause removed. **When** Alice pulls again. **Then** it succeeds, `tags_added` is `[]`, `tags_kept_local` is `[]`, and both tags are still listed.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-009: nothing else changes
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-002. **Then** the remote is unchanged, and the project's tags are exactly `v1.0` and `v2.0`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-010: a deadline after the tags
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `v1.0`, and the merge step does not finish before the time allowed for a remote operation. **When** Alice pulls. **Then** the refusal is the usual deadline text; it ends with ` Tags received before the failure: v1.0.` when the tag was delivered before the deadline, and has no sentence otherwise.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-013: a pause on conflict is not a failure
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the remote delivers `v1.0` and the merge conflicts. **When** Alice pulls. **Then** the answer reports `v1.0` as added and the conflict, with no sentence.
- Cleanup: temp directories are dropped with the test.

### SPEC-0076/E2E-014: no duplicate sentence on a second failure
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-002 and the merge fails again on a retry. **When** Alice pulls. **Then** the refusal has no sentence, since no tag is added this time.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for E2E-002, 003, 008, 009, 010 (no sentence before the change); E2E-001, 004, 005, 013, 014 guard unchanged text and are proven by the mutation "always append the sentence".
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged.
- [ ] Docs updated: none in this story (docs in US-0155).
- [ ] Self review: full.
- [ ] One commit `US-0154: name the tags a failed pull delivered`, body `Spec: SPEC-0076` and the requirement ids.
