---
Id: BL-0093
Title: Publish all tags: unspecified message when the remote keeps changing
Nature: DEBT
Source: review of US-0098 (SPEC-0054)
Created: 2026-10-27
---

## What
`send_due_tags` (`crates/core/src/tools/git_tags.rs`) refuses with `Remote tags kept changing while publishing: <names>. Review the list, then confirm again.` (code `ERR_NO_CLOBBER`) when a lost lease leaves a tag still due after a fresh listing, instead of retrying forever. The spec words no such case (FR-NEW-020), and no test covers it because a file remote cannot make a tag reappear after a lease loss.

## Why
An observable message without a requirement or a test.

## Evidence
`crates/core/src/tools/git_tags.rs` loop of `send_due_tags` and `reclassify_lost`; review of US-0098.

## Notes
Decide in a spec amendment whether to word it, or to retry once more. A seam that changes the remote twice between listing and push would test it.
