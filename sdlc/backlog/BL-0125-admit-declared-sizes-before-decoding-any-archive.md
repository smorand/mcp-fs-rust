---
Id: BL-0125
Title: Admit declared archive sizes before decoding, for every archive format
Nature: BUG
Source: found while designing SPEC-0080
Created: 2026-11-05
---

## What
`fs.extract_archive` decodes a zip, 7z, tar or RAR archive fully into memory before the quota admission runs, so a small archive that declares or expands to a very large total uses that memory before it is refused.

## Why
SPEC-0081 and SPEC-0083 bound the new bare file formats by the lower of the two limits; the archive formats keep the old order (decode, collision scan, admission). BL-0124 covers the missing size cap of the tar family only.

## Evidence
`crates/core/src/tools/archive.rs:154-205`: `decode_archive` runs inside `spawn_blocking` and returns every entry's bytes (`DecodedEntry.bytes`), then `quota::admit_paths` runs on the declared sizes. Class: attacker controlled compressed input reaching unbounded memory allocation (CWE-409).

## Notes
For zip, 7z and RAR the declared sizes are known from headers before any byte is decoded, so the admission can run first; for tar the sizes are in the entry headers after decompression, which is BL-0124. Decision: deferred by the author of the SPEC-0080 design, pending the user.
