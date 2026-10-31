# US-0127: reproduce a remote that keeps changing and test the refusal

> Spec: SPEC-0064 (sdlc/specs/SPEC-0064-publish-all-tags-remote-keeps-changing/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Make a remote that changes between the listing and the send reproducible in tests, then pin the refusal `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.` for one flapping tag, for a stable tag sent along, and the single attempt rule.

## Requirements covered
- SPEC-0064/FR-NEW-001 [EARS-O]: IF, during a confirmed send, the remote lost the product's attempt because a tag to send changed, and after the product listed the remote again a tag to send is still a tag to publish, THEN THE Remote exchange context SHALL send nothing, report no tag as sent, and refuse with `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.`
  - **Business rules:** `<names>` are the tags still to publish at that moment, not only the tag that changed, in the order of the preview (byte by byte by name), separated by a comma and a space; a tag that the re-listing showed `up to date` or `differs` is not named and its outcome is not reported (the refusal replaces the report); no audit entry is recorded (SPEC-0054/FR-NEW-009: none for any refusal); the remote receives nothing from this action; the refusal comes after the number check of SPEC-0054/FR-NEW-003 and FR-NEW-019, so a wrong number is refused first and nothing is sent; a new preview afterwards shows the real state of the remote.
- SPEC-0064/FR-NEW-002 [EARS-UB]: The Remote exchange context SHALL NOT send again after a re-listing that removed no tag from the tags to publish, and SHALL send again after a re-listing that removed at least one.
  - (partial: the part of the business rules about when to give up, tested here by E2E-006; the send-again half is tested by E2E-007 in the next story)

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_publish_all_flapping` runs the three tests green (names chosen by you, one common prefix). The non mock assertions are the exact refusal text, the remote's refs read straight from the bare repository (`bare_refs`), and the push and listing counters of the seam. Then run the mutations named under each test.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0064): no new dependency, no production line changed.
- DEC-002: the seam is `#[cfg(test)]` only and lives in `bulk_seams`; hooks run around the real push so libgit2's lease check fires for real.
- DEC-003: counters keyed by URL, `reset(url)` first in each test (the push runs on a blocking thread, so no thread local).
- DEC-005: red proof by mutation.
### Data model
None.
### Contracts
No change to the refusal text, its code or the names it carries. The seam API is internal to tests (see the sketch under Existing patterns).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (`bulk_seams` seam, two `#[cfg(test)]` calls in `push_tags_to_remote` and one in `list_advertised_refs`) |
| `crates/core/src/tools/git.rs` | modify (tests module: 3 tests, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`) |
### Existing patterns
Test seam to add in `crates/core/src/git/remote.rs`, inside the existing `#[cfg(test)] pub(crate) mod bulk_seams` (`:942-975`), mirroring `reject`/`drop_after` (statics keyed by URL, `Mutex<Vec<(String, ..)>>`):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PushPhase { Before(usize), After(usize) }
type Hook = Box<dyn FnMut(PushPhase) + Send>;
static HOOKS: Mutex<Vec<(String, Hook)>> = Mutex::new(Vec::new());
static COUNTS: Mutex<Vec<(String, (usize, usize))>> = Mutex::new(Vec::new());
pub(crate) fn around_push(url: &str, hook: impl FnMut(PushPhase) + Send + 'static) { /* replace the url's hook */ }
pub(crate) fn counts(url: &str) -> (usize, usize) { /* (pushes, listings) */ }
pub(crate) fn reset(url: &str) { /* drop hook and counters of url */ }
pub(super) fn note_listing(url: &str) { /* listings += 1 */ }
pub(super) fn note_push(url: &str) -> usize { /* pushes += 1, return the attempt index before the increment */ }
pub(super) fn run_hook(url: &str, phase: PushPhase) { /* take the hook out of its slot, call it, put it back */ }
```
Call sites, all under `#[cfg(test)]`: in `push_tags_to_remote` (`remote.rs:837`) after the early return for no tags and before `push_refs`: `let attempt = bulk_seams::note_push(origin_url); bulk_seams::run_hook(origin_url, PushPhase::Before(attempt));` and after `push_refs` returns (keep the result in a local, then run `After(attempt)`, then match as today); at the top of `list_advertised_refs` (`remote.rs:1057`): `bulk_seams::note_listing(origin_url);`. Take the hook out of the slot while it runs (a hook must be able to call `reset` or register nothing without deadlocking the mutex).

A flapping tag, as a closure, with `dir` the bare remote path (`two_due_previewed` returns `(dir, url, a, b, c)`; `plain_tag_bare_remote(dir, tag, target)` at `tools/git.rs:25741` creates a tag ref forced; delete with `git2::Repository::open_bare(&dir).unwrap().find_reference("refs/tags/v1.0").unwrap().delete().unwrap()`):
```rust
let (d, c) = (dir.clone(), commit_a.clone());
bulk_seams::around_push(&url, move |phase| match phase {
    PushPhase::Before(_) => plain_tag_bare_remote(&d, "v1.0", &c),   // the remote gains the tag, so the lease on "absent" fails
    PushPhase::After(_) => { /* delete refs/tags/v1.0 again, before the product re-lists */ }
});
```
Harness helpers in `tools/git.rs` `mod tests`: `two_due_previewed(e, label)` (two local tags `v1.0` and `v2.0`, preview done, `Tags to publish: 2`), `three_commit_remote`, `tag(e, json!({"name": .., "target": ..}))`, `preview_all(e)`, `confirm_all(e, json!(2))` (unwraps), `call_publish_all_tags(e, "origin", Some(json!(2)))` (returns the `Result`), `publish_audit(e, OWNER)`, `bare_refs(&dir)`, `track_remote`/`retrack`, `count_line(n)`, `published_line(tag, sha)`. Remember the seam state is per URL: call `bulk_seams::reset(&url)` at the start of each test.
Near neighbours to copy the shape of: `e2e_tag_021_publish_all_the_connection_drops_after_the_remote_applied_some` (uses `bulk_seams::drop_after`), `e2e_tag_042_publish_all_a_new_tag_appears_after_the_preview` (a remote change between preview and confirm).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. The behavior pre exists, so the RED proof of each test is a mutation of production code (listed per test), reverted afterwards.

### SPEC-0064/E2E-001: One tag keeps changing
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0`, not on the remote, whose commit the remote holds; Alice previewed `Tags to publish: 1`. **When** Alice confirms stating 1 and, each time the product is about to send, another source creates `v1.0` on the remote and removes it again before the product looks. **Then** the refusal is exactly `Remote tags kept changing while publishing: v1.0. Review the list, then confirm again.` **And** the remote holds no `v1.0` from this action and the audit trail has no new entry.
- **Priority:** Critical
- Mutation (RED proof): change the refusal text in `git_tags.rs:1219` (for example `kept changing` to `keeps changing`); this test fails on the exact text. Revert.

### SPEC-0064/E2E-002: A stable tag is named and not sent
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** local `v1.0` and `v2.0`, neither on the remote; Alice previewed `Tags to publish: 2`; the remote changes `v1.0` as in E2E-001 and never touches `v2.0`. **When** Alice confirms stating 2. **Then** the refusal is exactly `Remote tags kept changing while publishing: v1.0, v2.0. Review the list, then confirm again.` **And** `v2.0` is not on the remote and the audit trail has no new entry.
- **Priority:** Critical
- Mutation: build `names` in `git_tags.rs:1217` from only the tags whose lease was lost; this test fails (it expects `v1.0, v2.0`). Revert.

### SPEC-0064/E2E-006: One attempt, then the refusal
- Level / Driver / Location: functional e2e at the tool boundary / `call_publish_all_tags`, `preview_all`, `confirm_all` of the `Env` harness / `crates/core/src/tools/git.rs` `mod tests`, next to `e2e_tag_035_publish_all_two_members_confirm_at_once`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** local `v1.0`, not on the remote; the remote changes `v1.0` as in E2E-001 on every send. **When** Alice confirms stating 1. **Then** the product made exactly one sending attempt and listed the remote exactly twice (once for the confirmation, once after the lost attempt), and the answer is the refusal of E2E-001.
- **Priority:** High
- How to observe: `bulk_seams::counts(&url)` returns `(1, 2)` after the confirmation (one push, two listings: the confirmation's and the re-listing's).
- Mutation: replace `for _ in 0..due.len()` (`git_tags.rs:1178`) by `for _ in 0..8` and delete the `if due.len() == before { break; }` (`:1207-1209`); this test fails (`pushes` is 8, not 1). Revert.

## Constraints
- Files not to touch: `crates/core/src/tools/git_tags.rs` and every production line outside the three `#[cfg(test)]` seam calls; the existing seams `reject` and `drop_after` must keep their behavior.
- Dependencies not to add: none.
- Patterns to avoid: a thread local for the counters; a hook that sleeps; a test that shares a URL with another test.
- Scope boundary: E2E-003, 004, 005, 007, 008 and the docs line are US-0128.

## Definition of done
- [ ] Each new test shown RED by the mutation named for it (`git checkout -- <file>` after each), recorded in the commit body.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified (`cargo test -p mcp-fs-core --lib e2e_tag`); the existing seams `reject` and `drop_after` behave as before; no production line outside `#[cfg(test)]` changes.
- [ ] Docs: none in this story (the seam is documented in US-0128)
- [ ] Self review: full (time and space of the seam: O(1) per push or listing; no `unwrap` in production code).
- [ ] One commit `US-0127: reproduce a remote that keeps changing and test the refusal`, body `Spec: SPEC-0064` and the requirement ids.
