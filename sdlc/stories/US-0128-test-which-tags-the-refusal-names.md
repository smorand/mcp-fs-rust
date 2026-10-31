# US-0128: test which tags the refusal names, the retry and the order of refusals

> Spec: SPEC-0064 (sdlc/specs/SPEC-0064-publish-all-tags-remote-keeps-changing/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0127
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
Pin the rest of the contract: a tag that became identical is not named, names come in byte order, the product sends again after a re-listing that removed a tag, a wrong number is refused first, and a member can retry after the refusal. Then close the spec.

## Requirements covered
- SPEC-0064/FR-NEW-001 [EARS-O]: IF, during a confirmed send, the remote lost the product's attempt because a tag to send changed, and after the product listed the remote again a tag to send is still a tag to publish, THEN THE Remote exchange context SHALL send nothing, report no tag as sent, and refuse with `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.`
  - **Business rules:** `<names>` are the tags still to publish at that moment, not only the tag that changed, in the order of the preview (byte by byte by name), separated by a comma and a space; a tag that the re-listing showed `up to date` or `differs` is not named and its outcome is not reported (the refusal replaces the report); no audit entry is recorded (SPEC-0054/FR-NEW-009: none for any refusal); the remote receives nothing from this action; the refusal comes after the number check of SPEC-0054/FR-NEW-003 and FR-NEW-019, so a wrong number is refused first and nothing is sent; a new preview afterwards shows the real state of the remote.
- SPEC-0064/FR-NEW-002 [EARS-UB]: The Remote exchange context SHALL NOT send again after a re-listing that removed no tag from the tags to publish, and SHALL send again after a re-listing that removed at least one.
  - **Business rules:** a tag leaves the tags to publish when the re-listing shows it `up to date` or `differs` (SPEC-0054/FR-NEW-020); the confirmed send therefore always ends, with the report of SPEC-0054/FR-NEW-002 or with the refusal of FR-NEW-001; a send that ended in the report of FR-NEW-002 after one or more re-listings sends each tag at most once.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag` is green. Then run the mutations named under each test. Finally the converge of the whole spec: all eight E2E tests (this story's five and the three of US-0127) pass.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0064): this is the second of two stories; the first brought the seam.
- DEC-005: red proof by mutation.
### Data model
None.
### Contracts
No change to production contracts. Seam as built in US-0127 (`bulk_seams::around_push`, `PushPhase`, `counts`, `reset`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (tests module: 5 tests) |
| `.agent_docs/git.md` | modify (two lines on the seam, see the Docs item) |
### Existing patterns
A flapping tag, as a closure, with `dir` the bare remote path (`two_due_previewed` returns `(dir, url, a, b, c)`; `plain_tag_bare_remote(dir, tag, target)` at `tools/git.rs:25741` creates a tag ref forced; delete with `git2::Repository::open_bare(&dir).unwrap().find_reference("refs/tags/v1.0").unwrap().delete().unwrap()`):
```rust
let (d, c) = (dir.clone(), commit_a.clone());
bulk_seams::around_push(&url, move |phase| match phase {
    PushPhase::Before(_) => plain_tag_bare_remote(&d, "v1.0", &c),   // the remote gains the tag, so the lease on "absent" fails
    PushPhase::After(_) => { /* delete refs/tags/v1.0 again, before the product re-lists */ }
});
```
Harness helpers in `tools/git.rs` `mod tests`: `two_due_previewed(e, label)` (two local tags `v1.0` and `v2.0`, preview done, `Tags to publish: 2`), `three_commit_remote`, `tag(e, json!({"name": .., "target": ..}))`, `preview_all(e)`, `confirm_all(e, json!(2))` (unwraps), `call_publish_all_tags(e, "origin", Some(json!(2)))` (returns the `Result`), `publish_audit(e, OWNER)`, `bare_refs(&dir)`, `track_remote`/`retrack`, `count_line(n)`, `published_line(tag, sha)`. Remember the seam state is per URL: call `bulk_seams::reset(&url)` at the start of each test.
For E2E-003 and E2E-007 the first sending attempt must find `a1` already on the remote, identical to the local tag: create it in the `Before(0)` phase with `plain_tag_bare_remote(&dir, "a1", &commit)` where `commit` is the local tag's commit (a lightweight tag on that commit is identical, SPEC-0054/FR-NEW-005 definition). For E2E-003 `b1` flaps on every attempt (create in `Before(_)`, delete in `After(_)`) and `c1` is never touched. Byte order: `Zeta` sorts before `alpha` (`b"Z" < b"a"`), the preview order of SPEC-0054/FR-NEW-002 is the same.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. The behavior pre exists, so the RED proof of each test is a mutation of production code (listed per test), reverted afterwards.

### SPEC-0064/E2E-003: A tag that became identical is not named
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** local `a1`, `b1`, `c1`, none on the remote; Alice previewed `Tags to publish: 3`; before the first send another source creates `a1` on the remote identical to the local tag, and the remote changes `b1` as in E2E-001 on every send; `c1` is never touched. **When** Alice confirms stating 3. **Then** the refusal is exactly `Remote tags kept changing while publishing: b1, c1. Review the list, then confirm again.` **And** the remote holds `a1` (from the other source) and neither `b1` nor `c1`, and the audit trail has no new entry.
- **Priority:** High
- Mutation: make the loop break unconditionally after the first re-listing (`git_tags.rs:1205-1209`, replace the condition by `true`); this test fails. Revert.

### SPEC-0064/E2E-004: Retry after the refusal
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** recovery. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the refusal of E2E-001 has just been given and the remote has stopped changing. **When** Alice previews, then confirms stating the number the preview gives. **Then** the preview says `v1.0 to publish` and `Tags to publish: 1. Confirm by stating 1 to send them.`, and the confirmation reports exactly `v1.0 published`. **And** the remote holds `v1.0` and the audit trail has one new entry (`v1.0`).
- **Priority:** High

### SPEC-0064/E2E-005: Names in byte order
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `alpha` and `Zeta`, neither on the remote, both to publish; the remote changes `alpha` as in E2E-001. **When** Alice confirms stating 2. **Then** the refusal is exactly `Remote tags kept changing while publishing: Zeta, alpha. Review the list, then confirm again.` (capital `Z` sorts before lower case `a`). **And** nothing was sent.
- **Priority:** Medium
- Mutation: sort `names` descending before joining (`git_tags.rs:1217`); this test fails. Revert.

### SPEC-0064/E2E-007: A re-listing that removes a tag allows another send
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** local `a1` and `b1`, neither on the remote; Alice previewed `Tags to publish: 2`; before the first send another source creates `a1` on the remote identical to the local tag. **When** Alice confirms stating 2. **Then** the report is exactly `a1 up to date`, `b1 published`. **And** the product made two sending attempts, the first lost and the second accepted, `b1` is on the remote once, and the audit trail has one new entry (`b1`).
- **Priority:** High
- Mutation: make the loop break unconditionally after the first re-listing (same as E2E-003); this test fails (`b1 published` never comes). Revert.

### SPEC-0064/E2E-008: A wrong number comes first
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0`, not on the remote, and a remote that changes `v1.0` as in E2E-001. **When** Alice confirms stating 5. **Then** the refusal is the wrong number refusal of SPEC-0054/FR-NEW-003, not the one of FR-NEW-001. **And** the product made no sending attempt and the audit trail has no new entry.
- **Priority:** Medium
- Mutation: move the number check of `parse_confirm`/FR-NEW-003 after the send (or delete it); this test fails. Revert.

## Constraints
- Files not to touch: every production file; `crates/core/src/git/remote.rs` (the seam is final after US-0127).
- Dependencies not to add: none.
- Patterns to avoid: asserting on timing; sharing a URL between tests.
- Scope boundary: nothing else; BL-0093 is consumed by SPEC-0064.

## Definition of done
- [ ] Each new test shown RED by the mutation named for it (`git checkout -- <file>` after each), recorded in the commit body.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified (`cargo test -p mcp-fs-core --lib e2e_tag`); the existing seams `reject` and `drop_after` behave as before; no production line outside `#[cfg(test)]` changes.
- [ ] Docs: `.agent_docs/git.md`: add two lines (timestamped 2026-10-30) where the publish all tag tests are described: the `bulk_seams::around_push` hook and `counts` seam, and that it lets a test change a bare remote between the listing and the sending.
- [ ] Self review: full (time and space of the seam: O(1) per push or listing; no `unwrap` in production code).
- [ ] One commit `US-0128: test which tags the refusal names, the retry and the order of refusals`, body `Spec: SPEC-0064` and the requirement ids.
