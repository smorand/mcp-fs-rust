---
Id: BL-0100
Title: A pull that fails after the tag reconcile loses `tags_added`
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0082 (US-0087))
Created: 2026-10-30
---

## What
A pull whose merge step errors after the tag reconcile (quota, merge error, deadline) keeps the delivered tags but returns an error without `tags_added`; a retry then reports those tags as neither added nor kept local. `reconcile_incoming_tags` also runs a `list_refs` query when `incoming_tags` is empty.

## Why
The operator cannot see which tags a failed pull delivered.

## Evidence
`crates/core/src/tools/git_tags.rs` `reconcile_incoming_tags` (about line 427); pull path in `crates/core/src/tools/git.rs`.
