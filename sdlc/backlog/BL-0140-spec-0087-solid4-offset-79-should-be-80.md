---
Id: BL-0140
Title: SPEC-0087 and US-0199 give the end of the first solid4.rar block as 79, the file says 80
Nature: DEBT
Source: found while running US-0199
Created: 2026-11-09
---

## What
SPEC-0087 (sdlc-spec.md lines about E2E-018, section 9.0 and the evidence appendix) and US-0199 say the first file block of `solid4.rar` ends at offset 79. Measured on the committed fixture (133 bytes): main header 7 to 20, `a.txt` header 42 plus data 18, ending at 80, then `b.txt` ending at 126, end block 126 to 133. A cut at 79 already removes a data byte, so the block walk refuses it for another reason.

## Why
E2E-018 now asserts the cut at 80 (the only one that kills the "accept the end of input after a whole block" mutant) and also the cut at 79.

## Evidence
`crates/core/tests/fixtures/rar/README.md` row for `solid4.rar`, Python block parse in the US-0199 run, `crates/core/src/tools/archive_rar_cut_e2e.rs` (e2e_0087_018).

## Notes
Proposal: an erratum on SPEC-0087 replacing 79 by 80, like the SPEC-0080 erratum 2.
