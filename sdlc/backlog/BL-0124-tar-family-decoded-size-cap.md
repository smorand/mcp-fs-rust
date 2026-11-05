---
Id: BL-0124
Title: Cap the decoded size of tar family archives against decompression bombs
Nature: BUG
Source: deferred from SPEC-0081
Created: 2026-11-04
---

## What
`fs.extract_archive` decompresses `.tar.gz`, `.tar.bz2` and `.tar.xz` into memory with no size cap, so a small hostile archive can exhaust server memory before the quota admission runs.

## Why
SPEC-0081 bounds the new bare `.gz` path by the lower of the project's remaining storage and the member's remaining session write allowance. The tar family has no such bound.

## Evidence
`crates/core/src/tools/archive.rs:636-655` reads each decoder to the end into a `Vec`; the quota check (`archive.rs:183-205`) uses declared tar header sizes, after the whole stream is already in memory.
Class: attacker controlled compressed input reaching unbounded memory allocation (CWE-409).

## Notes
Reuse the bounded decode of SPEC-0081. The tar size check against declared sizes (`ensure_decoded_size_matches`) runs per entry, after the full stream is decompressed.
Decision: deferred by the author of SPEC-0081, pending the user.
