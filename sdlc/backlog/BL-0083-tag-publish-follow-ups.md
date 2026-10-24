---
Id: BL-0083
Title: Tag publish follow ups: breadth of "on the remote", rejection test, wording, name normalisation
Nature: DEBT
Source: review of US-0089 (SPEC-0052)
Created: 2026-10-24
---

## What
- DEC-020 and FR-NEW-019 say "any branch the remote holds", but `commit_on_remote` (`crates/core/src/tools/git_tags.rs`) builds its tip set from EVERY advertised ref (`refs/pull/*`, tags, HEAD, peeled `^{}` entries). A commit only reachable from a pull request ref counts as on the remote, and a refusal walks every advertised ref (O(n + u*d)). Either restrict to `refs/heads/*` plus peeled tags, or amend DEC-020 in a design update.
- The real remote rejection mapping (`PushFailure::Rejected` -> `tag_push_error` -> `remote_refused_tag`) has no test; E2E-085 is realised through `Reference::is_valid_name`, and the unit test only formats a string.
- Connection errors on tag publish reuse `connection_error`, whose text reads `fetch from host '<h>' failed`; a branch push says `push of branch '<b>' failed`. Codes match, wording differs; E2E-038 compares codes only for down and rate limited.
- `git.tag_create` and `reconcile_incoming_tags` call `repo.reference` on names such as `a//b`, which libgit2 silently normalises to `a/b` on disk (the database row keeps `a//b`). Guard those calls with `is_valid_name` as the publish path now does.
- E2E-039 and E2E-077 are simulations (the failure is at the listing step, not after the push).

## Why
Each one is a small gap between the design text and the code, or a regression the current tests would not catch.

## Evidence
Review of the US-0089 diff: `git_tags.rs` (`commit_on_remote`, `publish_tag`, the unit test at the bottom), `git/remote.rs` (`push_tag_to_remote`, `tag_push_error`, `advertise`).

## Notes
Decide with the owner whether the wider set is acceptable (it is arguably safer: those commits do exist remotely).
