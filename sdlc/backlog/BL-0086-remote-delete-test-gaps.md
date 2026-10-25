---
Id: BL-0086
Title: Remote tag deletion tests: rate limit leg, transport reason, blank remote name
Nature: DEBT
Source: review of US-0093 (SPEC-0053)
Created: 2026-10-25
---

## What
- E2E-032's rate limited leg reuses the connection dropping remote of E2E-033, so it does not tell a rate limit from a dropped connection; no file remote can answer 429.
- E2E-032, 033 and 034 assert the exact `fetch from host '<h>' failed: ` prefix, not the transport's reason text.
- The "down" leg uses a `file://` path, whose host in the text is the whole URL; the spec example host `github.ibm.com` is only exercised by the credential legs.
- No test names a blank remote (empty or spaces), which FR-NEW-014 says is shown as given.

## Why
These legs would still pass if a wrong error followed the right prefix, and the blank remote rule has no test.

## Evidence
Tests `e2e_tag_032_remote_delete_*`, `e2e_tag_033_remote_delete_*`, `e2e_tag_034_remote_delete_*`, `e2e_tag_044_remote_delete_*` in `crates/core/src/tools/git.rs`.

## Notes
Add a blank remote name test; if a transport double that answers 429 becomes available (the https path with a stub), use it for the rate limit leg.
