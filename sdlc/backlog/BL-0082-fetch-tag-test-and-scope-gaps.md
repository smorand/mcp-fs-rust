---
Id: BL-0082
Title: Fetch tag reception: batching test and branch scope
Nature: DEBT
Source: review of US-0086 (SPEC-0052)
Created: 2026-10-24
---

## What
- `e2e_tag_048` creates 2000 tags of which only 500 are annotated, so exactly one batch of 500 refspecs runs and the batching loop of `fetch_into_incoming` (`crates/core/src/git/remote.rs`) is never exercised; only 5 distinct commits exist, so the commit cache is trivial.
- "The fetched branch" is the checked out branch, else the remote default branch (US-0086 deviation 2). Tags reachable only from another branch are never received.
- (US-0087) A pull whose merge step errors after the tag reconcile (quota, merge error, deadline) keeps the delivered tags but returns an error without `tags_added`; a retry then reports those tags as neither added nor kept local. `reconcile_incoming_tags` also runs a `list_refs` query when `incoming_tags` is empty.
- (US-0088) Clone hydrates the whole project repository (`export_to_repo`, O(all objects)) before reconcile so `tags_identical` can compare against local objects; it could hydrate only when an incoming name already exists locally. A clone with `depth` is untested: the ancestry filter only sees the shallow history.
- Tag rows are written one `set_ref` await at a time in `reconcile_incoming_tags` (n sequential writes for n tags).

## Why
The batching loop could regress unnoticed, the scope choice is an unwritten product decision, and 2000 tags means 2000 sequential writes.

## Evidence
Review of the US-0086 diff: `fetch_into_incoming`, `history_tags` in `git/remote.rs`; `reconcile_incoming_tags` in `tools/git_tags.rs`; test `e2e_tag_048` in `tools/git.rs`.

## Notes
Make a test with more than 500 annotated tags; confirm the branch scope with the owner (SPEC-0052 FR-NEW-009 says "the fetched branch"); batch the writes if it is slow.
