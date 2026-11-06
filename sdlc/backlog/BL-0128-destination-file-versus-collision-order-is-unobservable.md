---
Id: BL-0128
Title: The order of "destination is a file" and "collision" cannot be observed, so E2E-036 and its siblings pin nothing about it
Nature: FEAT
Source: found while reviewing US-0167 (SPEC-0083)
Created: 2026-11-05
---

## What
SPEC-0083 E2E-036 (and SPEC-0082 E2E-051) give a destination that is a file plus an existing output and expect the file refusal instead of `ERR_NO_CLOBBER`. When the destination is a file, or sits under one, the output path is below that file and cannot exist, so no collision can apply: both checks never fire together and their relative order is unobservable.

## Why
FR-NEW-011 and FR-NEW-012 list "destination is a file" (7) before "collision" (8) as an ordering, but no input makes the order matter. The test passes whatever the order and cannot go red by swapping the two steps.

## Evidence
`crates/core/src/tools/archive_single.rs` tests: `e2e_0083_036_destination_is_a_file_wins_over_a_co` seeds `/uploads/data.csv` while the destination is the file `/uploads/notes.txt`; the collision check looks up `/uploads/notes.txt/data.csv` (`check_collision`, output path joined to the destination), which is absent.

## Notes
Either drop the pair from the order lists of SPEC-0081, SPEC-0082 and SPEC-0083 (no test can pin it) or find a real input for it (none exists today). The reviewer's finding on US-0167 is recorded here; the test stays as the spec states it.
