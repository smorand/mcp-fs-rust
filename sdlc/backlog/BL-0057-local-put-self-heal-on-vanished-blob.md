---
Id: BL-0057
Title: A local put that finds its blob deleted mid call should rewrite it
Nature: DEBT
Source: US-0037 review (SPEC-0034), 2026-10-16
Created: 2026-10-16
Severity: Low
---

## What
`LocalBlobStore::put` checks that the blob exists, then opens it to refresh its modified time
(SPEC-0034/DR-002, `crates/core/src/storage/blob/local.rs:38-50`). If a concurrent delete removes
the file between the check and the open, `put` returns `NotFound`; before US-0037 it returned `Ok`
with no file at all. Falling through to the normal write on `NotFound` would heal the race.

## Why
The write then fails loudly instead of succeeding; a self heal would make it succeed correctly.

## Evidence
US-0037 review: "would ideally self-heal by falling through to the write path on `NotFound` rather
than erroring; current behavior is an acceptable, non-regressive residual" (`local.rs:44`).

## Notes
Needs a deterministic test seam (a pausing filesystem hook) to be red first. Suggested:
`/sdlc-spec --debt BL-0057`.
