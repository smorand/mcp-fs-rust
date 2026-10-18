---
Id: BL-0063
Title: Refuse zip archives whose local and central headers disagree
Nature: FEAT
Source: deferred from SPEC-0036
Created: 2026-10-18
Severity: Low
---

## What
zip 9 can validate local against central headers and refuse duplicate raw names (`ZipReadOptions::enforce_header_consistency(true)`, `zip-9.0.5/src/read/zip_archive.rs:736`); it is off by default and SPEC-0036 keeps it off.

## Why
Malformed archives (header mismatch, duplicate raw names) are extracted today with the central directory's view; refusing them is stricter but changes what `fs.extract_archive` and `fs.extract_text` accept.

## Evidence
`zip-9.0.5/src/read/mod.rs:251`, `:270` (default false); `zip_archive.rs:664`, `:685`, `:690` (all readers use defaults).
