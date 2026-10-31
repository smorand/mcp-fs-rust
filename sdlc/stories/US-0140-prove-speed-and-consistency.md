# US-0140: prove speed and consistency of fetches

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0139
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The speed and consistency promises are proven: up to date answers and one commit fetches on a project of 20 000 objects take under 1 second, and fetches during pushes never see half a push.

## Requirements covered
- SPEC-0067/FR-NEW-002: WHEN the client ends the negotiation, THE git service SHALL answer with the acknowledgement lines and the closing line of the reference answer, followed by the same set of objects as the reference answer.
- SPEC-0067/NFR-001: WHEN a client asks for a commit it holds on a project of 20 000 objects, THE git service SHALL answer in under 1 second and send no object (baseline: 1.6 to 1.8 seconds before SPEC-0066, development build; not re-measured since).
- SPEC-0067/NFR-002: WHILE a push updating one or several branches is being received, THE git service SHALL answer a fetch with the state of each branch either entirely before or entirely after the push, never with a commit missing its parents.
- SPEC-0067/NFR-003: WHEN a client fetches one new commit changing 1 file on a project of 20 000 objects, THE git service SHALL answer in under 1 second and send exactly the 3 objects of that commit.

## Independent Test
`cargo test -p mcp-fs-core --lib git::http` and the e2e file are green; E2E-023 measures under 1 second on 20 000 objects with 0 objects sent.

## Design excerpt
### Decisions
- DEC-009: the five 20 000 object tests run as integration tests inside `mod.rs` tests, seeded through the object store as SPEC-0066 did. Obliged by: SPEC-0067/NFR-001, NFR-003. Rationale: pushing 20 000 objects through the wire in a debug build costs minutes; the answer function is the thing measured. Alternatives considered: wire push (rejected for cost). Source: SPEC-0066 design. Evidence: `sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/sdlc-design.md:38-44`.
- DEC-010: the fetch keeps taking only the repository lock; if E2E-025 to 027 are red, the answer is built under the project write lock instead. Obliged by: SPEC-0067/NFR-002. Rationale: measure first. Alternatives considered: take the write lock now (rejected: serialises fetches behind pushes without evidence). Source: design. Evidence: `crates/core/src/git/http/mod.rs:433,580-582,623`.
### Data model
None.
### Contracts
No new production contract. If E2E-025 to 027 are red, `handle_upload_pack` takes `entry.write_lock` around `answer` (DEC-010).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/mod.rs` | modify (tests; production only if a test is red) |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
### Existing patterns
SPEC-0066 measurement code (seed 20 000 objects through `import_reachable`, time the call) in the `mod.rs` tests of that spec's history: `crates/core/src/git/http/mod.rs` tests `seed_commit` use at `:851,870`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-023: up to date request on a large project
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs (mod tests)`
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001
- **Given** a project of 20 000 objects with tip `c20000`; a request wanting `c20000` and stating `c20000` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer arrives in under 1 second, equals the reference answer and carries 0 objects
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-024: one new commit on a large project
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs (mod tests)`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003, FR-NEW-002
- **Given** a project of 20 000 objects, a copy that holds all of it, one new commit changing 1 file
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 3 (the commit, its top folder listing, the file)
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-025: fetch during a push
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** a copy at `c4` and a push of `c5` to `c8` in progress
- **When** the developer runs `git fetch` at the same time
- **Then** the copy ends at `c4` or at `c8`, never in between (no `c6` without `c5`)
- **And** the integrity check reports no problem
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-026: a push that fails during a fetch
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** a copy at `c4` and a push of `c5` that the service rejects (the pusher is not a member)
- **When** the developer runs `git fetch` while the push is rejected
- **Then** the copy ends at `c4` with no integrity problem
- **And** the project still holds `c4` as its tip
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-027: two fetches during a push moving two branches
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-002
- **Given** two copies at `c4` (`main`) and `f1` (`feature`), and a push moving `main` to `c8` and `feature` to `f3` in progress
- **When** both developers run `git fetch` at the same time
- **Then** each branch in each copy ends at its old or its new tip, with no integrity problem
- **And** it is not asserted that the two branches move together
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-028: five up to date requests in a row
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs (mod tests)`
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001
- **Given** the project and the request of E2E-023
- **When** the request is sent 5 times one after the other
- **Then** each answer arrives in under 1 second and carries 0 objects
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-039: two new commits on a large project
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs (mod tests)`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003
- **Given** a project of 20 000 objects, a copy that holds all of it, and 2 new commits each changing 1 file in the top folder
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 6
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-040: new commit adding a file in a new folder on a large project
- Level / Driver / Location: integration / direct call of the answer function on a project seeded through the object store / `crates/core/src/git/http/mod.rs (mod tests)`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** NFR-003
- **Given** a project of 20 000 objects, a copy that holds all of it, and one new commit adding `docs/readme.txt`
- **When** the developer runs `git fetch`
- **Then** the fetch ends in under 1 second
- **And** the objects sent are exactly 4 (the commit, the top folder listing, the listing of `docs`, the file)
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

- [ ] One commit `US-0140: prove speed and consistency of fetches`, body `Spec: SPEC-0067` and the requirement ids.
