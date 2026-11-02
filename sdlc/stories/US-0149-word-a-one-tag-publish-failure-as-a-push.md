# US-0149: word a one tag publish failure as a push and a vanished tag as gone

> Spec: SPEC-0072 (sdlc/specs/SPEC-0072-tag-publish-error-wording/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0148
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Publishing one tag reads `fetch from host` when the remote cannot be reached, and `has moved to 0000000` when the remote tag vanished. Name the push and say the tag does not exist.

## Requirements covered
- SPEC-0072/FR-NEW-001: WHEN the connection to the remote fails while a member publishes one tag THE Remote exchange context SHALL refuse with `push of tag '<tag>' to host '<host>' failed: <cause>`.
- SPEC-0072/FR-NEW-003: WHEN the remote tag a member confirmed replacing no longer exists at push time THE Remote exchange context SHALL refuse with `Remote tag '<tag>' does not exist.`
- SPEC-0072/FR-MOD-001 (partial: one tag publish; publish all is US-0150): WHEN a connection fails during a tag publish THE Remote exchange context SHALL use FR-NEW-001 or FR-NEW-002 instead of `fetch from host '<host>' failed: <cause>`.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including the eight tests below; publishing `v1.0` to a missing remote reads `push of tag 'v1.0' to host '<host>' failed: ` and a remote tag deleted between the listing and the push reads `Remote tag 'v1.0' does not exist.`.

## Design excerpt
### Decisions
- DEC-001: wording is chosen by an enum argument `ConnectionWording`, default `Fetch`. Obliged by: SPEC-0072/FR-NEW-001, SPEC-0072/FR-NEW-002, SPEC-0072/FR-MOD-001. Rationale: FR-MOD-001 and E2E-006 need the same callee to word differently per caller. Alternatives considered: per function copies of `connection_error` (rejected: 11 call sites). Source: design. Evidence: `remote.rs:1568-1571`.
- DEC-002: no string rewriting in the tools layer (see Approaches). Obliged by: SPEC-0072/FR-MOD-001. Source: design.
- DEC-003: `probe_remote_tag` always words as `PublishTag`: its only caller is the annotated message check of a publish. Obliged by: SPEC-0072/FR-NEW-001 (E2E-009). Source: existing code. Evidence: `git_tags.rs:739`.
- DEC-004: a vanished remote tag reuses `remote_tag_absent` verbatim. Obliged by: SPEC-0072/FR-NEW-003. Rationale: one condition, one message (owner decision, spec 3.3). Evidence: `git_tags.rs:1452-1454`.
### Data model
None.
### Contracts
`remote.rs`: `pub enum ConnectionWording<'a> { Fetch, PublishTag(&'a str) }` (US-0150 adds `PublishTags`); `fn connection_error_for(w, origin_url, e, token)`; `connection_error` = `connection_error_for(Fetch, ..)`. `list_advertised_refs(repo, origin_url, token, deadline, wording)`; `probe_remote_tag` words as `PublishTag(tag)`; `tag_push_error` words as `PublishTag(tag)`; the delete callers (`git_tags.rs:1408`, `:1442`) and `fetch_from_remote`'s `advertise` pass `Fetch`. `publish_on_repo` (`git_tags.rs:715-726`): vanished remote tag returns `remote_tag_absent(tag)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (enum, `connection_error_for`, wording parameter) |
| `crates/core/src/tools/git_tags.rs` | modify (callers pass a wording, vanished tag) |
| `crates/core/src/tools/git.rs` | modify (tests, helper `publish_tag_prefix(tag, url)`) |
### Existing patterns
`connection_error` at `remote.rs:1568-1571`; `after_listing` seam at `git_tags.rs:757+`; `connection_prefix` helper at `git.rs:27746` (kept for the delete tests); `call_publish_tag` at `git.rs:26287`, `closing_remote`, `down_remote`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0072/E2E-002: one tag, remote down
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice, local tag `v1.0`, a remote that does not exist. **When** she publishes `v1.0`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: ` and its failure is `internal error`. **And** the text contains no `fetch`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-004: remote tag vanished
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** remote tag `v1.0` at `f6a7b8c`, Alice confirmed replacing it with a local `v1.0` at `a1b2c3d`. **When** the remote tag is deleted before her push goes out and her publish continues. **Then** the refusal is `Remote tag 'v1.0' does not exist.` and its failure is `not found`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-005: remote tag moved, not vanished
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Edge.** **Given** the same, the remote tag is moved to `9e8d7c6` before the push. **Then** the refusal is `Remote tag 'v1.0' has moved to 9e8d7c6 since you last saw it. Fetch, then confirm again.`
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-006: other operations keep the fetch wording
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Edge.** **Given** a remote that does not exist. **When** Alice fetches, pulls and deletes the remote tag `v1.0`. **Then** each refusal starts `fetch from host '<host>' failed: `.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-007: the connection drops mid push
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** a remote that closes the connection after the listing. **When** Alice publishes `v1.0`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: ` and its text names the dropped connection.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-008: nothing changes on refusal
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Side effect.** **Given** E2E-002 and E2E-004 set ups. **Then** the remote refs and the audit log equal their state before the call. **And** the local tag `v1.0` still exists.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-009: annotated tag message check fails to connect
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** an annotated local tag `v1.0` with the message `Release 1.0`, a remote with `v1.0` that then goes down at the message check. **When** Alice publishes `v1.0` stating `f6a7b8c`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: `.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-010: vanished tag, publish confirmed twice
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** the vanish of E2E-004. **When** Alice repeats the call. **Then** the repeat is not a refusal about a move; the tag publishes as a creation, answering as before (the remote holds no `v1.0`, so no confirmation is needed).
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for E2E-002, 004, 005, 007, 009, 010 (they fail before the change); E2E-006 and E2E-008 guard behavior that exists and are proven by the mutation "every connection error reads `push of tag`" (E2E-006 fails).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing unchanged, including the delete tests that assert `fetch from host` and the publish all tests (their wording changes in US-0150, not here).
- [ ] Docs updated: none in this story (docs in US-0150).
- [ ] Self review: full.
- [ ] One commit `US-0149: word a one tag publish failure as a push and a vanished tag as gone`, body `Spec: SPEC-0072` and the requirement ids.
