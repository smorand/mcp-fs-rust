# US-0109: return decimals and binary values as readable text and close the spec

> Spec: SPEC-0058 (sdlc/specs/SPEC-0058-db-typed-values-as-text/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0108
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
Decimals and binary values come back as `123.45` and `00016162` text, the same as the CSV conversion, instead of `<Decimal128(10, 2)>` and `<BinaryView>`. This story completes the spec: the full acceptance set and the requirement coverage audit run here.

## Requirements covered
- SPEC-0058/FR-NEW-002: WHEN `db.query` or `db.sample` returns a decimal value THE data analytics tools SHALL return it as a text equal to the cell text `db.convert` writes into a CSV file for the same value.
- SPEC-0058/FR-NEW-003: WHEN `db.query` or `db.sample` returns a binary value THE data analytics tools SHALL return it as lower case hexadecimal text equal to the cell text `db.convert` writes into a CSV file for the same value.
- SPEC-0058/FR-NEW-004 (partial: decimals and binary in NDJSON, completing the date part of US-0108): WHEN `db.convert` writes an NDJSON or JSON file from a source holding dates, timestamps, decimals or binary values THE data analytics tools SHALL write each of them as the same text FR-NEW-001, FR-NEW-002 and FR-NEW-003 define.
- SPEC-0058/FR-NEW-005 (partial: decimals and binary, completing the date, timestamp and nested part of US-0108): IF a date, timestamp, decimal or binary value is missing THEN THE data analytics tools SHALL return `null` for it, in query and sample results and in NDJSON or JSON files.
- SPEC-0058/FR-NEW-007 (partial: completing US-0108 with the CSV bytes and type names of the typed fixture): The data analytics tools SHALL NOT change the value, the column list, the row count, the type names or the error messages returned for text, whole numbers, floating point numbers and booleans, nor the CSV file written by `db.convert`.
- SPEC-0058/FR-MOD-001: The data analytics tools SHALL return the texts of FR-NEW-001 to FR-NEW-003 in place of the values SPEC-0057/BR-005 froze for dates, timestamps, decimals and binary values.

## Independent Test
`cargo test -p mcp-fs-core --lib bt_0058` is green (all 15 tests of the spec); then `db.query` `SELECT dec, bin FROM t` on fixture T1 returns `{"dec":"123.45","bin":"00016162"}` and `db.convert` from `/t.parquet` to `/o.ndjson` writes the same texts as the matching cells of the CSV conversion. E2E-003, 004, 006, 007, 008 and 011 are RED before the change (`<Decimal128(10, 2)>`, `<BinaryView>`); E2E-014 protects.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0058): Render the covered types with `ArrayFormatter` built from `FormatOptions::default()`, the formatter and options the CSV writer uses. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Rationale:** the spec defines the text as the CSV cell text; sharing the formatter makes parity structural. **Alternatives considered:** hand written renderers with `chrono` (parity risk, more code); SQL `CAST` to text (rewrites results and types). **Source:** existing code. **Evidence:** `arrow-csv-60.0.0/src/writer.rs:258-276`, `arrow-cast-60.0.0/src/display.rs:562-565` (binary, large and fixed width binary, binary view), `:753` (decimals), `:790-791` (timestamps).
- DEC-003 (SPEC-0058): The covered set is exactly `Date32`, `Date64`, `Timestamp`, `Decimal128`, `Decimal256`, `Binary`, `LargeBinary`, `FixedSizeBinary`, `BinaryView`; every other type keeps its present arm, including `Time32`, `Time64`, `Duration`, `Interval` and nested types, and a formatter that cannot be built falls back to that arm. **Obliged by:** SPEC-0058/FR-NEW-006, FR-NEW-007, spec Section 3.2 and 14.3. **Rationale:** nothing else is specified. **Alternatives considered:** formatting every type (changes outputs the spec freezes, and nested types have no CSV text). **Source:** spec constraint. **Evidence:** `db.rs:168-221`.
- DEC-005 (SPEC-0058): Two stories: US-0108 plumbs the formatter and covers dates and timestamps (with nested and plain regression); US-0109 adds decimals and binary, closes the spec. **Obliged by:** SPEC-0058/FR-NEW-001 to FR-NEW-007. **Rationale:** each leaves main green and independently testable; the second is a one line change to the covered set plus its tests, so it carries the closing audit. **Alternatives considered:** one story of 15 tests (T2 budget is 5 to 15 and 3 to 5 files; kept whole it would pass but mixes two behaviors); one story per type (a thinner cut with nothing to assert alone). **Source:** design. **Evidence:** budget table, T2.
- DEC-006 (SPEC-0058): E2E-001 and E2E-006 run through the MCP route, the other E2E tests through `db_harness`. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-004. **Rationale:** the MCP route proves the served surface once per behavior; the registered tool handlers are the same code, as in SPEC-0057. **Alternatives considered:** all 15 through MCP (slow, no extra coverage). **Source:** existing code. **Evidence:** `tool_surface_e2e.rs:374`, SPEC-0057 design section 9.
### Data model
None.
### Contracts
No tool, parameter or schema change. Private to `crates/core/src/tools/db.rs`: `fn text_formatter(col: &dyn Array) -> Option<ArrayFormatter<'_>>` returning `Some` only for the types of the covered set below and `None` otherwise (also when `ArrayFormatter::try_new` fails). `ArrayFormatter` and `FormatOptions` come from `arrow::util::display` (already a dependency). `batches_to_json` (`db.rs:139`) builds the formatters once per batch, then per cell: null gives JSON null; a covered column gives `Value::String(formatter.value(row).to_string())`; any other column calls `arrow_value_to_json` as today. Fixture T1 (a Parquet file `/t.parquet`, one row): `ts_s` timestamp in seconds 1700000000; `ts_ms` in milliseconds 1700000000123; `ts_us` in microseconds 1700000000123456; `ts_ns` in nanoseconds 1700000000123456789; `ts_utc` in microseconds with time zone UTC 1700000000123456; `d32` date counted in days 19675; `d64` date with time of day in milliseconds 1700000000000; `dec` decimal 123.45 (10 digits, 2 after the point); `bin` bytes `00 01 61 62`; `n` whole number 5; `name` text `Alice`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/db.rs` | modify: add `Decimal128`, `Decimal256`, `Binary`, `LargeBinary`, `FixedSizeBinary`, `BinaryView` to the covered set of `text_formatter`, tests |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (E2E-006 through the MCP route) |
| `.agent_docs/tools.md` | modify only if it describes db value formats or the old placeholders; otherwise no change and say so in the commit body |
### Existing patterns
`batches_to_json` and `arrow_value_to_json` in `crates/core/src/tools/db.rs:139-221`; the SPEC-0057 arm and tests (`bt_0057_*`, `string_parquet`, `db_harness`, `call_on`, `convert`) in the same file; `bt_0057_001_*` in `crates/core/src/tools/tool_surface_e2e.rs` for the MCP route (`setup`, `app_with_atlas`, `call_tool`, `success_payload`, `fs.write`, `fs.read_bytes` with its `base64` field). The covered set and `t1_parquet` come from US-0108 (already on the chain).

## Acceptance tests
> 100% must pass, run through `make check` (`cargo test --workspace` and the gate in `AGENTS.md`). Never run test files ad hoc. Tests named in the Independent Test as RED before the change must fail on the readable text assertion, not on a setup error; tests marked "protects" may pass at once.

### SPEC-0058/E2E-003: a decimal reads as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT dec FROM t` on `/t.parquet`
- **Then** the row is `{"dec":"123.45"}` (a text, not the number 123.45)
- **Priority:** Critical


### SPEC-0058/E2E-004: a binary value reads as hexadecimal text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT bin FROM t` on `/t.parquet`
- **Then** the row is `{"bin":"00016162"}`
- **Priority:** Critical


### SPEC-0058/E2E-006: conversion to NDJSON writes the same text, and agrees with CSV
- **Category:** side effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005
- **Given** fixture T1 extended with a second row where every typed column is missing, `n` is 6 and `name` is `Bob`
- **When** the analyst runs `db.convert` from `/t.parquet` to `/o.ndjson`, then from `/t.parquet` to `/o.csv`
- **Then** the first response is `{"src_path":"/t.parquet","dst_path":"/o.ndjson","rows_converted":2,"format":"ndjson"}`; line 1 of `/o.ndjson` is `{"ts_s":"2023-11-14T22:13:20","ts_ms":"2023-11-14T22:13:20.123","ts_us":"2023-11-14T22:13:20.123456","ts_ns":"2023-11-14T22:13:20.123456789","ts_utc":"2023-11-14T22:13:20.123456Z","d32":"2023-11-14","d64":"2023-11-14T22:13:20","dec":"123.45","bin":"00016162","n":5,"name":"Alice"}`; line 2 has `null` for the 9 typed columns, `"n":6` and `"name":"Bob"`; and each text of line 1 equals the matching cell of `/o.csv` line 2
- **And** `/t.parquet` is unchanged
- **Priority:** Critical


### SPEC-0058/E2E-007: missing values are null
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Given** a Parquet file `/n.parquet` with one row where a date, a timestamp in microseconds, a decimal and a binary column are all missing
- **When** the analyst runs `db.query` with `SELECT * FROM n`
- **Then** the row has `null` for all four columns, and no `?`, no empty text, no placeholder
- **Priority:** High


### SPEC-0058/E2E-008: decimal edge values
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Given** a Parquet file `/d.parquet` with one row: `big` decimal of 22 digits with 2 after the point 12345678901234567890.12; `neg` decimal of 10 digits with 2 after the point -0.05; `s0` decimal of 10 digits with 0 after the point 42
- **When** the analyst runs `db.query` with `SELECT big, neg, s0 FROM d`
- **Then** the row is `{"big":"12345678901234567890.12","neg":"-0.05","s0":"42"}`
- **Priority:** High


### SPEC-0058/E2E-011: binary edge values
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-005
- **Given** a Parquet file `/b.parquet` with two columns, `b0` holding zero bytes and `bff` holding the bytes `00 ff`, plus a second row where both are missing
- **When** the analyst runs `db.query` with `SELECT b0, bff FROM b`, then `db.convert` from `/b.parquet` to `/b.ndjson`
- **Then** rows are `[{"b0":"","bff":"00ff"},{"b0":null,"bff":null}]` (the empty text differs from `null`), and `/b.ndjson` holds the two lines `{"b0":"","bff":"00ff"}` and `{"b0":null,"bff":null}`
- **Priority:** High


### SPEC-0058/E2E-014: CSV conversion and type names do not move
- **Category:** failure (non regression)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-007
- **Given** fixture T1
- **When** the analyst runs `db.convert` from `/t.parquet` to `/o.csv`, then `db.schema` on `/t.parquet`
- **Then** `/o.csv` is exactly `ts_s,ts_ms,ts_us,ts_ns,ts_utc,d32,d64,dec,bin,n,name` then `2023-11-14T22:13:20,2023-11-14T22:13:20.123,2023-11-14T22:13:20.123456,2023-11-14T22:13:20.123456789,2023-11-14T22:13:20.123456Z,2023-11-14,2023-11-14T22:13:20,123.45,00016162,5,Alice`, and `db.schema` reports the type names `Timestamp(s)`, `Timestamp(ms)`, `Timestamp(µs)`, `Timestamp(ns)`, `Timestamp(µs, "UTC")`, `Date32`, `Date64`, `Decimal128(10, 2)`, `BinaryView`, `Int64`, `Utf8` in that order
- **Priority:** High


## Constraints
- Files not to touch: `Cargo.toml`, `Cargo.lock`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: a decimal rendered as a JSON number; formatting `Time32`, `Time64`, `Duration`, `Interval` or nested types.
- Scope boundary: the type name `db.schema` shows for a Parquet binary column (BL-0097); nested and time types (BL-0096).
- Naming: test functions are named `bt_0058_e2e_NNN_<slug>` (NNN is the E2E number) so that `cargo test bt_0058` selects them.

## Definition of done
- [ ] RED evidence recorded for every new test that asserts a changed output before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `spec_0055_dt_*` and `bt_0057_*` tests pass unmodified; `tool-contract-golden.json` unchanged.
- [ ] Docs updated: `.agent_docs/tools.md` if it names the old values; add a dated entry to `.agent_docs/dependencies.md` or the db section stating the readable text rule (SPEC-0058).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001 to E2E-015), and every requirement FR-NEW-001 to FR-NEW-007 and FR-MOD-001 cited by at least one story with status done or landed. Set nothing in the spec files.
- [ ] Self review: full (time complexity O(rows x columns) with one formatter per column per batch, no per cell formatter construction, no `unwrap` on fallible paths, no dash punctuation in comments or docs).
- [ ] One commit `US-0109: return decimals and binary values as readable text and close the spec`, body `Spec: SPEC-0058` and the requirement ids.
