---
Id: BL-0094
Title: Publish all tags tests: revert pause, transport reason, rate limit leg
Nature: DEBT
Source: review of US-0099 (SPEC-0054)
Created: 2026-10-27
---

## What
- FR-NEW-014 names merge, rebase, cherry pick and revert; E2E-036, 043 and 046 cover the first three. A paused revert has no test (`pause_with_due_tag` in the tests module of `crates/core/src/tools/git.rs` needs one more arm).
- E2E-020, 021, 029 and 050 assert the exact `fetch from host '<url>' failed: ` prefix, not the transport reason (`connection refused`, `connection reset`).
- E2E-022's rate limit leg reuses a connection dropping remote; no file remote can answer 429.

## Why
Those legs would pass if a wrong error followed the right prefix, and a paused revert could regress unnoticed.

## Evidence
Tests `e2e_tag_036_*`, `043_*`, `046_*`, `020_*`, `021_*`, `022_*`, `029_*`, `050_*` named `*_publish_all_*` in `crates/core/src/tools/git.rs`. Same family as BL-0086.

## Notes
Add the revert arm and a revert test; assert the transport reason where the transport gives a stable one.
