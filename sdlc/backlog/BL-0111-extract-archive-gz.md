---
Id: BL-0111
Title: Decompress a single file compressed with .gz (not tar)
Nature: FEATURE
Source: user request
Created: 2026-11-04
---

## What
Support a bare `.gz` file (gzip, no tar inside), for example `data.csv.gz` giving `data.csv` in the destination. Either extend `fs.extract_archive` or add a sibling tool, decided in the spec. Same safety rules as the other formats: one quota charge, decoded size cap against decompression bombs, collision handling.

## Why
gzip is a standard compression format. Only the tar wrapped forms (`.tar.gz`) work today.

## Evidence
`crates/core/src/tools/archive.rs` suffix table: only compound tar suffixes carry gz.

## Notes
The decompressor already exists for the tar path, so no new dependency is expected. The output name is the input name without the suffix; a name that does not end in `.gz` needs a rule.
