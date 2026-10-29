---
Id: BL-0104
Title: Tag publish connection errors read `fetch from host`
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0083)
Created: 2026-10-30
---

## What
Connection errors on tag publish reuse `connection_error`, whose text reads `fetch from host '<h>' failed`; a branch push says `push of branch '<b>' failed`. Codes match, wording differs. Decide the wording and test it (E2E-038 compares codes only for down and rate limited).

## Why
An operator reading a publish failure sees the word fetch.

## Evidence
`crates/core/src/git/remote.rs:1490` `connection_error`; `push_tag_to_remote`.
