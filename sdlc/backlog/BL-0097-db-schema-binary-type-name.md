---
Id: BL-0097
Title: db.schema reports BinaryView for a Parquet binary column
Nature: FEAT
Source: deferred from SPEC-0058
Created: 2026-10-28
---

## What
`db.schema` and `db.profile` report `BinaryView` for a binary column read from Parquet, the way they reported `Utf8View` for strings before SPEC-0057.

## Why
A caller sees an engine specific type name for what is plain binary data. SPEC-0058 changes the binary values only and leaves the type names alone.

## Evidence
Observed on HEAD with a Parquet binary column: `"data_type":"BinaryView"`. The string equivalent was fixed by `type_name` in `crates/core/src/tools/db.rs:126-131`.

## Notes
Report `Binary`, as SPEC-0057 did for `Utf8`.
