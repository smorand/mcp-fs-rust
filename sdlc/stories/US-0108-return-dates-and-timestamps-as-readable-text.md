# US-0108: return dates and timestamps as readable text

> Spec: SPEC-0058 (sdlc/specs/SPEC-0058-db-typed-values-as-text/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Dates and timestamps come back from `db.query`, `db.sample` and the NDJSON conversion as `2023-11-14` and `2023-11-14T22:13:20.123456Z` style text, the same text as the CSV conversion, instead of day counts, nanosecond digit strings or `?`. This story also builds the per column formatter plumbing the next story reuses.

## Requirements covered
- SPEC-0058/FR-NEW-001: WHEN `db.query` or `db.sample` returns a date or a timestamp THE data analytics tools SHALL return it as a text equal to the cell text `db.convert` writes into a CSV file for the same value.
- SPEC-0058/FR-NEW-004 (partial: dates and timestamps in NDJSON; decimals and binary belong to US-0109): WHEN `db.convert` writes an NDJSON or JSON file from a source holding dates, timestamps, decimals or binary values THE data analytics tools SHALL write each of them as the same text FR-NEW-001, FR-NEW-002 and FR-NEW-003 define.
- SPEC-0058/FR-NEW-005 (partial: dates, timestamps and nested values; decimals and binary belong to US-0109): IF a date, timestamp, decimal or binary value is missing THEN THE data analytics tools SHALL return `null` for it, in query and sample results and in NDJSON or JSON files.
- SPEC-0058/FR-NEW-006: IF a column holds nested values (lists, structs, maps) THEN THE data analytics tools SHALL return for each such value the text `<` followed by the type name that `db.schema` reports for the column, followed by `>`, and the call SHALL succeed.
- SPEC-0058/FR-NEW-007 (partial: text, whole numbers, floating point numbers, booleans and CSV bytes of the date and timestamp columns): The data analytics tools SHALL NOT change the value, the column list, the row count, the type names or the error messages returned for text, whole numbers, floating point numbers and booleans, nor the CSV file written by `db.convert`.

## Independent Test
`cargo test -p mcp-fs-core --lib bt_0058` is green; then, with `db.enabled = true`, `db.query` `SELECT d32, ts_s FROM t` on fixture T1 returns `{"d32":"2023-11-14","ts_s":"2023-11-14T22:13:20"}`. E2E-001, 002, 005, 009 and 010 are RED before the change (`19675`, `"?"`, nanosecond digits come back); E2E-012, 013 and 015 protect behavior that already holds.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0058): Render the covered types with `ArrayFormatter` built from `FormatOptions::default()`, the formatter and options the CSV writer uses. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Rationale:** the spec defines the text as the CSV cell text; sharing the formatter makes parity structural. **Alternatives considered:** hand written renderers with `chrono` (parity risk, more code); SQL `CAST` to text (rewrites results and types). **Source:** existing code. **Evidence:** `arrow-csv-60.0.0/src/writer.rs:258-276`, `arrow-cast-60.0.0/src/display.rs:562-565` (binary, large and fixed width binary, binary view), `:753` (decimals), `:790-791` (timestamps).
- DEC-002 (SPEC-0058): Build one formatter per covered column per batch inside `batches_to_json`, not one per cell. **Obliged by:** SPEC-0058/NFR 7.1 (no more than 2 times the old time). **Rationale:** a timestamp formatter parses its time zone when built; per cell construction would multiply that cost by the row count. **Alternatives considered:** a formatter per cell (simpler signature, quadratic in tz parsing); a cache in a static (global state). **Source:** design. **Evidence:** `db.rs:139-166` (the per cell loop), `display.rs:790` (tz parse in `try_new`).
- DEC-003 (SPEC-0058): The covered set is exactly `Date32`, `Date64`, `Timestamp`, `Decimal128`, `Decimal256`, `Binary`, `LargeBinary`, `FixedSizeBinary`, `BinaryView`; every other type keeps its present arm, including `Time32`, `Time64`, `Duration`, `Interval` and nested types, and a formatter that cannot be built falls back to that arm. **Obliged by:** SPEC-0058/FR-NEW-006, FR-NEW-007, spec Section 3.2 and 14.3. **Rationale:** nothing else is specified. **Alternatives considered:** formatting every type (changes outputs the spec freezes, and nested types have no CSV text). **Source:** spec constraint. **Evidence:** `db.rs:168-221`.
- DEC-004 (SPEC-0058): Remove the `Date32`, `Date64` and `Timestamp` arms of `arrow_value_to_json` once the formatter covers them, with their now unused imports; the default arm stays. **Obliged by:** SPEC-0058/FR-MOD-001 and clean code. **Rationale:** those arms become unreachable from `batches_to_json`; `db.profile` only passes text columns. **Alternatives considered:** keeping them as dead code (flagged as lean violation). **Source:** existing code. **Evidence:** `db.rs:206-219`, `db.rs:471-472`.
- DEC-006 (SPEC-0058): E2E-001 and E2E-006 run through the MCP route, the other E2E tests through `db_harness`. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-004. **Rationale:** the MCP route proves the served surface once per behavior; the registered tool handlers are the same code, as in SPEC-0057. **Alternatives considered:** all 15 through MCP (slow, no extra coverage). **Source:** existing code. **Evidence:** `tool_surface_e2e.rs:374`, SPEC-0057 design section 9.
### Data model
None.
### Contracts
No tool, parameter or schema change. Private to `crates/core/src/tools/db.rs`: `fn text_formatter(col: &dyn Array) -> Option<ArrayFormatter<'_>>` returning `Some` only for the types of the covered set below and `None` otherwise (also when `ArrayFormatter::try_new` fails). `ArrayFormatter` and `FormatOptions` come from `arrow::util::display` (already a dependency). `batches_to_json` (`db.rs:139`) builds the formatters once per batch, then per cell: null gives JSON null; a covered column gives `Value::String(formatter.value(row).to_string())`; any other column calls `arrow_value_to_json` as today. Fixture T1 (a Parquet file `/t.parquet`, one row): `ts_s` timestamp in seconds 1700000000; `ts_ms` in milliseconds 1700000000123; `ts_us` in microseconds 1700000000123456; `ts_ns` in nanoseconds 1700000000123456789; `ts_utc` in microseconds with time zone UTC 1700000000123456; `d32` date counted in days 19675; `d64` date with time of day in milliseconds 1700000000000; `dec` decimal 123.45 (10 digits, 2 after the point); `bin` bytes `00 01 61 62`; `n` whole number 5; `name` text `Alice`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/db.rs` | modify: `text_formatter` (the covered set of this story: `Date32`, `Date64`, `Timestamp`; the decimal and binary types are added by US-0109), `batches_to_json`, remove the `Date32`, `Date64`, `Timestamp` arms and unused imports, test fixture `t1_parquet`, the integration tests |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (E2E-001 through the MCP route) |
### Existing patterns
`batches_to_json` and `arrow_value_to_json` in `crates/core/src/tools/db.rs:139-221`; the SPEC-0057 arm and tests (`bt_0057_*`, `string_parquet`, `db_harness`, `call_on`, `convert`) in the same file; `bt_0057_001_*` in `crates/core/src/tools/tool_surface_e2e.rs` for the MCP route (`setup`, `app_with_atlas`, `call_tool`, `success_payload`, `fs.write`, `fs.read_bytes` with its `base64` field).

## Acceptance tests
> 100% must pass, run through `make check` (`cargo test --workspace` and the gate in `AGENTS.md`). Never run test files ad hoc. Tests named in the Independent Test as RED before the change must fail on the readable text assertion, not on a setup error; tests marked "protects" may pass at once.

### SPEC-0058/E2E-001: timestamps of every unit read as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-007
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT ts_s, ts_ms, ts_us, ts_ns, ts_utc, n, name FROM t` on `/t.parquet`
- **Then** the result is exactly one row `{"ts_s":"2023-11-14T22:13:20","ts_ms":"2023-11-14T22:13:20.123","ts_us":"2023-11-14T22:13:20.123456","ts_ns":"2023-11-14T22:13:20.123456789","ts_utc":"2023-11-14T22:13:20.123456Z","n":5,"name":"Alice"}`, `columns` lists the 7 names in that order and `row_count` is 1
- **Priority:** Critical


### SPEC-0058/E2E-002: dates read as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT d32, d64 FROM t` on `/t.parquet`
- **Then** the row is `{"d32":"2023-11-14","d64":"2023-11-14T22:13:20"}`
- **Priority:** Critical


### SPEC-0058/E2E-005: dates in a CSV file read as text, sample and query
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-005
- **Given** a CSV file `/c.csv` with the lines `d,ts,x`, `2023-11-14,2023-11-14T22:13:20,1.50` and `,,2.50`
- **When** the analyst runs `db.sample` with `n` 5 on `/c.csv`, then `db.query` with `SELECT d, ts, x FROM c ORDER BY x`
- **Then** both return rows `[{"d":"2023-11-14","ts":"2023-11-14T22:13:20","x":1.5},{"d":null,"ts":null,"x":2.5}]`
- **Priority:** Critical


### SPEC-0058/E2E-009: moments before 1970
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** a Parquet file `/p.parquet` with one row: a timestamp in seconds equal to one second before 1970-01-01T00:00:00 named `pre` and a date counted in days equal to one day before 1970-01-01 named `dpre`
- **When** the analyst runs `db.query` with `SELECT * FROM p`
- **Then** the row is `{"pre":"1969-12-31T23:59:59","dpre":"1969-12-31"}`
- **Priority:** High


### SPEC-0058/E2E-010: a time zone with an offset
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** a Parquet file `/z.parquet` with one microsecond timestamp column `tz2` of time zone `+02:00` holding 1700000000123456
- **When** the analyst runs `db.query` with `SELECT tz2 FROM z`, then `db.convert` from `/z.parquet` to `/z.csv`
- **Then** the query row is `{"tz2":"2023-11-15T00:13:20.123456+02:00"}` and the `tz2` cell of `/z.csv` line 2 is the same text
- **Priority:** High


### SPEC-0058/E2E-012: a nested column keeps its placeholder
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Given** a Parquet file `/l.parquet` with a list of whole numbers column `lst` holding `[1, 2]` and a date column `d32` holding day 19675
- **When** the analyst runs `db.schema` on it, then `db.query` with `SELECT lst, d32 FROM l`, then `db.convert` from `/l.parquet` to `/l.ndjson`
- **Then** `db.schema` reports `List(Int64)` for `lst`; the query succeeds with `{"lst":"<List(Int64)>","d32":"2023-11-14"}`; the conversion succeeds and `/l.ndjson` is `{"lst":"<List(Int64)>","d32":"2023-11-14"}` followed by a newline
- **Priority:** High


### SPEC-0058/E2E-013: plain columns do not move, in the same file as a nested one
- **Category:** failure (non regression)
- **Scenario:** SC-001, SC-002
- **Requirements:** FR-NEW-006, FR-NEW-007
- **Given** the 3 row file used by the existing characterization tests (whole numbers `id`, text `name`, floating point `score`) as CSV, NDJSON and Parquet, and the file `/l.parquet` of E2E-012 with an added whole number column `k` holding 7
- **When** the analyst runs `db.query` with `SELECT id, name, score FROM t ORDER BY id` on each of the three files, and `db.query` with `SELECT k FROM l` on `/l.parquet`
- **Then** the three results equal `{"columns":["id","name","score"],"rows":[{"id":1,"name":"Alice","score":1.5},{"id":2,"name":"Bob","score":2.5},{"id":3,"name":"Carol","score":4.0}],"row_count":3}` and the last equals `{"columns":["k"],"rows":[{"k":7}],"row_count":1}`
- **Priority:** High


### SPEC-0058/E2E-015: a missing nested value is null, not a placeholder
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** a Parquet file `/m.parquet` with a list of whole numbers column `lst` holding `[1, 2]` in row 1 and missing in row 2
- **When** the analyst runs `db.query` with `SELECT lst FROM m`
- **Then** rows are `[{"lst":"<List(Int64)>"},{"lst":null}]`
- **Priority:** High


## Constraints
- Files not to touch: `Cargo.toml`, `Cargo.lock`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, anything under `sdlc/specs/`.
- Dependencies not to add: none (`arrow::util::display` is already available).
- Patterns to avoid: building a formatter per cell (DEC-002); formatting types outside the covered set (DEC-003); touching the CSV writer.
- Scope boundary: decimal and binary values (US-0109); times of day, durations, intervals and nested values keep today's output (BL-0096).
- Naming: test functions are named `bt_0058_e2e_NNN_<slug>` (NNN is the E2E number) so that `cargo test bt_0058` selects them.

## Definition of done
- [ ] RED evidence recorded for every new test that asserts a changed output before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `spec_0055_dt_*` and `bt_0057_*` tests pass unmodified; `tool-contract-golden.json` unchanged.
- [ ] Docs: nothing to update in this story.
- [ ] Self review: full (time complexity O(rows x columns) with one formatter per column per batch, no per cell formatter construction, no `unwrap` on fallible paths, no dash punctuation in comments or docs).
- [ ] One commit `US-0108: return dates and timestamps as readable text`, body `Spec: SPEC-0058` and the requirement ids.
