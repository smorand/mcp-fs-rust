---
Id: BL-0150
Title: A converter bundle file growing after its size is taken is read whole
Nature: BUG
Source: class sweep of SPEC-0089 (BL-0142, BL-0143), residual of a safe occurrence
origin: deferred from SPEC-0089
Created: 2026-11-10
Decision: deferred by the SPEC-0089 author under the user's blanket authorization of 2026-11-10 ("spec BL-0142 and BL-0143 together", every decision authorized); exploitable only through a converter that leaves a helper process writing into its output folder after it exits
---

## What
When reading a converter bundle, the server checks each file's length, then reads the file whole. A file that keeps growing between the two (written by a helper the converter left running) is read at its new size, past the converter limit, before the total is checked.

## Why
Same class as SPEC-0089 (converter output read into memory with no bound, CWE-400), but it needs a misbehaving operator chosen converter rather than a hostile document, so it was kept out of that increment.

## Evidence
`crates/core/src/docs/service.rs:513-518` (length checked), `:519` (`std::fs::read` whole), `:521-526` (total checked after the read).

## Notes
Read each file through the capped reader SPEC-0089 introduces, never past what remains of the limit.
