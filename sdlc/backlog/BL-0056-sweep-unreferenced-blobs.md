---
Id: BL-0056
Title: Sweep blobs no reference holds after an interrupted write
Nature: DEBT
Source: deferred from SPEC-0033
Created: 2026-10-15
---

## What
Every write stores its blob before its node transaction (`crates/core/src/storage/volume.rs:161-172`,
`crates/core/src/docs/artifacts/capture.rs:150-207`). A crash or database error between the two
leaves a blob with no `blob_refs` row, and nothing collects it: blob stores offer no `list`
(`crates/core/src/storage/traits.rs:232-236`). SPEC-0033 only covers the refused artifact commit.

## Why
Unreferenced bytes accumulate in project blob stores, invisible to users and to the quota.

## Notes
A sweep needs a `list` on the local and S3 backends, must skip `git:` and `export:` keys, and needs
an age threshold so a write in flight is never collected. Suggested: `/sdlc-spec --debt BL-0056`.
