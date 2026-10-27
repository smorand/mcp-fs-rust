---
Id: BL-0090
Title: db tools return the text <Utf8View> for Parquet string columns
Nature: BUG
Source: found while implementing US-0101 (SPEC-0055)
Created: 2026-10-27
---

## What
`db.query` on a Parquet file returns the literal text `<Utf8View>` as the value of every string column; `db.schema` reports the type `Utf8View`. Binary, decimal, list and non nanosecond timestamp values hit the same default arm.

## Why
The value is lost: a user querying a Parquet string column gets placeholder text instead of the data.

## Evidence
`crates/core/src/tools/db.rs:136` matches `Utf8` and `LargeUtf8` only; the default arm at `crates/core/src/tools/db.rs:184` is `other => json!(format!("<{other}>"))`. Recorded as the current behaviour by tests DT-002, DT-006 and DT-040 of US-0101 (datafusion 43 reads Parquet strings as `Utf8View` by default).

## Notes
Authority violated: the `db.query` tool description ("Run a SQL query ... Return the result rows"). Fix: handle `Utf8View` (and the other arms) in `arrow_value_to_json`, and update the characterized expectations in the same change. SPEC-0055 keeps the output unchanged on purpose (DEC-006: invariance).
