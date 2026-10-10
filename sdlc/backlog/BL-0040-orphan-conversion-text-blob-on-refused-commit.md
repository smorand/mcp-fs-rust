---
Id: BL-0040
Title: Conversion text blob left unreferenced when an artifact set commit is refused
Nature: DEBT
Source: US-0001 review (SPEC-0019), 2026-10-10
Created: 2026-10-10
---

## What
`crates/core/src/docs/artifacts/mod.rs:177` (now `capture.rs`) writes the conversion text blob, and since US-0005 every picture blob, before
`commit_artifact_set`; when the commit refuses (rev changed, database error) the blob has no
`blob_refs` row, so no sweep ever collects it.

## Proposal
Write the blob after the rev check inside the commit, or have the purge sweep collect blobs with no
reference. No observable behavior change.
