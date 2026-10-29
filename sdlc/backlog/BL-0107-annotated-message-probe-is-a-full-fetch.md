---
Id: BL-0107
Title: The annotated message probe is a full single tag fetch, not shallow
Nature: DEBT
Source: deferred from SPEC-0060 (carried from BL-0083)
Created: 2026-10-30
---

## What
The annotated message probe is a full single tag fetch, not the shallow fetch of DEC-017 (the local transport refuses depth, `crates/core/src/git/remote.rs:997-999`).

## Why
A gap between the design text and the code; cost on a large remote.

## Evidence
`crates/core/src/git/remote.rs:997-999`; SPEC-0052 DEC-017. Depends on BL-0098 for a test.
