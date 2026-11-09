---
Id: BL-0139
Title: SPEC-0085 E2E-036 expects the collision to name the file, the shared scan names the folder first
Nature: DEBT
Source: found while running US-0191
Created: 2026-11-09
---

## What
SPEC-0085 E2E-036 says the refusal message is `'/uploads/report/data/a.txt' already exists; ...`. The real message is `'/uploads/report/data' already exists; ...`: the no-clobber scan also checks the folder entries and the archive lists `data` before `data/a.txt`. Same mismatch as SPEC-0080 erratum 2 (E2E-010, E2E-044).

## Why
The test asserts the real behavior (`ERR_NO_CLOBBER`, content `old`, 9 bytes remaining all match), so only the spec text is wrong.

## Evidence
`crates/core/src/tools/archive_declared_e2e.rs` (`e2e_0085_036`), `crates/core/src/tools/archive.rs` (collision scan over entries including directories), SPEC-0080 erratum 2.

## Notes
Proposal: an erratum on SPEC-0085 naming the folder, like the one applied to SPEC-0080.
