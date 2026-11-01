---
Id: BL-0120
Title: The doc comment of `acknowledge` omits that an unknown id can be named by a ready line
Nature: DEBT
Source: review of US-0143 (SPEC-0068)
Created: 2026-11-01
---

## What
The doc comment of `acknowledge` (`crates/core/src/git/http/negotiate.rs:216-220`) says an unknown id is "not acknowledged". The code never sends `ACK <unknown> common`, but names the id in an `ACK <id> ready` line when it follows held commits that satisfy every want (`:258-259`), as SPEC-0068 FR-NEW-001 states.

## Why
A reader of the comment alone would expect an unknown id to appear in no line.

## Evidence
`crates/core/src/git/http/negotiate.rs:216-220,258-259`; SPEC-0068 FR-NEW-001.

## Notes
Comment only; SPEC-0068's story forbids production files, so it is a separate change.
