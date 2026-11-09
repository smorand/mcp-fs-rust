---
Id: BL-0133
Title: Recognize the .tbz2 and .tbz suffixes as bzip2 compressed tar archives
Nature: FEAT
Source: deferred from SPEC-0084
Created: 2026-11-09
---

## What
`fs.extract_archive` knows `.tar.bz2` and `.tb2` but refuses `.tbz2` and `.tbz`, two common short names for the same format, with `unsupported archive extension: '.tbz2'`.

## Why
Found while scoping SPEC-0084: the request listed `.tbz2` among the tar family, but the product never supported it. Adding a suffix is a new capability, outside a decoding bound increment.

## Evidence
`crates/core/src/tools/archive.rs:42-63` (suffix table: `.tar.bz2` and `.tb2` only).

## Notes
Once added, the SPEC-0084 decoding bound applies to them like any bzip2 tar. SPEC-0084 E2E-042 pins the refusal today and would become a modified test.
