---
Id: BL-0096
Title: db tools: lists, structs and other nested values print a placeholder
Nature: FEAT
Source: deferred from SPEC-0058
Created: 2026-10-28
---

## What
`db.query`, `db.sample` and `db.convert` to NDJSON return `<List(Int64)>` for a list column (and the same shape for structs, maps and other nested types). `db.convert` to CSV fails with `csv write: Csv error: Nested type List(Int64) is not supported in CSV`.

## Why
The value is lost. SPEC-0058 fixed dates, timestamps, decimals and binary only, because nested values have no CSV text to match and need a product decision (JSON array or object).

## Evidence
`crates/core/src/tools/db.rs:220` (default arm `other => json!(format!("<{other}>"))`). Observed on HEAD with a Parquet list column: query returns `"lst":"<List(Int64)>"`, CSV conversion fails with the message above.

## Notes
Times of day, durations and intervals also keep the placeholder after SPEC-0058; include them here.
Propose rendering lists and structs as JSON arrays and objects in `db.query`, `db.sample` and NDJSON; CSV keeps refusing nested types with its present message.
