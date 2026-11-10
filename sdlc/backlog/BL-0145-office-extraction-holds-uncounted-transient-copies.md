---
Id: BL-0145
Title: Office text extraction holds transient copies the SPEC-0088 figures do not count
Nature: DEBT
Source: found while designing SPEC-0088 (BL-0134)
origin: deferred from SPEC-0088
Created: 2026-11-10
Decision: deferred by the SPEC-0088 designer under the user's blanket authorization of 2026-11-10; SPEC-0088 bounds decoded part data and extracted content as its spec defines them, not every copy made of them
---

## What
After SPEC-0088 the decoded part data and the extracted content of one extraction are bounded (104 857 600 per part, 268 435 456 per call, 268 435 456 of content). The real heap of a call is a small multiple of those figures, because the code makes copies the definitions do not count:
- an invalid UTF-8 part is repaired with `String::from_utf8_lossy`, a second copy that can be three times the part (one invalid byte becomes a 3 byte replacement character);
- `ExtractResult::of_blocks` joins every block (`blocks.join`), trims into a new `String`, and copies every grid cell trimmed; `truncate_chars` copies the kept text again;
- an empty grid cell costs a 24 byte `String` header for 4 bytes of markup (`<c/>`), so a sheet of empty cells inflates about 6 times in the grid.

## Why
Memory use within a constant factor of the bound, not unbounded: a robustness gap, not the exploit SPEC-0088 closes.

## Evidence
`crates/core/src/docs/extract.rs:418-420` (`decode`), `:96-138` (`of_blocks`), `:424-431` (`truncate_chars`), `:1071-1110` (`xlsx_rows`); SPEC-0088 design DEC-007, DEC-009.

## Notes
Candidates: trim cells in place and move the grid instead of copying; stream the lossy repair; count empty cells at their struct size.
