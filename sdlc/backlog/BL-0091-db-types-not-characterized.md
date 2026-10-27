---
Id: BL-0091
Title: db tools: timestamp, date, decimal and binary handling is not characterized
Nature: DEBT
Source: review of US-0103 (SPEC-0055)
Created: 2026-10-27
---

## What
The characterization tests of US-0101 cover integer, float and string columns only. `arrow_value_to_json` (`crates/core/src/tools/db.rs:158-208`) downcasts timestamps to nanoseconds only and sends decimals, dates, binary and nested types to the placeholder arm. After the move to datafusion 55 and arrow 60 a Parquet reader may yield timestamps in another unit, which would print `"?"` where datafusion 43 printed a number.

## Why
An output change on those types would pass the suite unnoticed.

## Evidence
`crates/core/src/tools/db.rs:184` (default arm), tests DT-001 to DT-011 (columns id, name, score only).

## Notes
Add characterization tests for timestamp (ns, us, ms), date, decimal and binary columns through `db.query`, `db.schema` and `db.convert`, then decide per type whether the placeholder is the contract (see BL-0090).
