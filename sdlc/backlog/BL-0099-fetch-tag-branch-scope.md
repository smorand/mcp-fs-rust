---
Id: BL-0099
Title: Decide which branch's tags a fetch receives
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0082)
Created: 2026-10-30
---

## What
A fetch receives only tags reachable from the fetched branch (the checked out branch, else the remote default branch). Tags reachable only from another branch are never received. Decide with the owner whether that is the intended rule and write it down.

## Why
It is an unwritten product decision (SPEC-0052 FR-NEW-009 says "the fetched branch").

## Evidence
`crates/core/src/git/remote.rs` `history_tags` (about line 1354) and `fetch_into_incoming`; SPEC-0052 US-0086 deviation 2.
