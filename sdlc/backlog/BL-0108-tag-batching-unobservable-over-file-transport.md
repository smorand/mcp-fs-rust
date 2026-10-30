---
Id: BL-0108
Title: The 500 refspec batching of the tag fetch cannot be proven red over a file remote
Nature: DEBT
Source: found while implementing US-0112 (SPEC-0060 DR-003)
Created: 2026-10-30
---

## What
SPEC-0060 DR-003 asks for a test of 501 annotated tags so that the second refspec batch of `fetch_into_incoming` runs, and DT-001 asks that dropping the second batch turns it red. Over a `file://` remote it does not: mutating `tags.chunks(TAG_REFSPEC_BATCH)` to `.take(1)` (and even `chunks(10).take(1)`) leaves `e2e_tag_501_annotated_tags_span_two_batches` green, with 510 objects received and none missing. libgit2's local transport sends every tag object whatever the refspecs ask for.

## Why
The batching loop can still regress unnoticed. Needs a transport that honours `want` lines, which is the transport double of BL-0098, or a counter of fetch calls visible to tests.

## Evidence
`crates/core/src/git/remote.rs:1463` (`chunks(TAG_REFSPEC_BATCH)`), `crates/core/src/git/remote.rs:1336`; mutation run in `.implement/US-0112-red.log` (not committed); test in `crates/core/src/tools/git.rs` (`e2e_tag_501_annotated_tags_span_two_batches`). Extends BL-0098.
