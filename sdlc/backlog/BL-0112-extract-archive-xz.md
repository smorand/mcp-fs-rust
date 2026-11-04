---
Id: BL-0112
Title: Decompress a single file compressed with .xz (not tar)
Nature: FEATURE
Source: user request
Created: 2026-11-04
---

## What
Support a bare `.xz` file (xz, no tar inside), for example `data.csv.xz` giving `data.csv` in the destination. Either extend `fs.extract_archive` or add a sibling tool, decided in the spec. Same safety rules as the other formats: one quota charge, decoded size cap against decompression bombs, collision handling.

## Why
xz is a standard compression format. Only the tar wrapped forms (`.tar.xz`) work today.

## Evidence
`crates/core/src/tools/archive.rs` suffix table: only compound tar suffixes carry xz.

## Notes
The decompressor already exists for the tar path, so no new dependency is expected. The output name is the input name without the suffix; a name that does not end in `.xz` needs a rule.
