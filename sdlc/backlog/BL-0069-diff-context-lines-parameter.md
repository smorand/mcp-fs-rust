---
Id: BL-0069
Title: Let the caller choose how many context lines the file comparison shows
Nature: FEAT
Created: 2026-10-19
origin: deferred from SPEC-0045
---

## What
The two file comparison added by SPEC-0045 always shows 3 unchanged lines around each change. Let the caller
choose that number.

## Why
Deferred (YAGNI) from SPEC-0045: 3 matches every other diff the product already returns, and nobody asked
for another value. A caller who needs more can read the files.

## Evidence
`crates/core/src/core/diff.rs:56` already takes a context size; only the caller facing parameter is missing.
