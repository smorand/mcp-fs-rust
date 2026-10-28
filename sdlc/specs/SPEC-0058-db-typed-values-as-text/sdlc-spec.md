# Dates, timestamps, decimals and binary values as readable text in the data tools: functional specification

> Id: SPEC-0058
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 1 module (the optional data analytics tools), 7 anticipated requirements, no persisted change; an output format changes, no tool name, parameter or schema does
> Status: specified
> Generated on: 2026-10-28
> Project: existing
> From backlog: BL-0091
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
A person or an agent who reads a data file through the `db.query`, `db.sample` and `db.convert` tools gets unreadable or wrong values today for dates, timestamps, decimals and binary data: a day count instead of a date, the text `?` instead of a timestamp, a type label instead of a decimal. This increment makes those values come back as the same readable text the CSV export already produces, so a query result, a sample and an NDJSON conversion all agree with the CSV conversion of the same file. Now, because the backlog review of the Parquet string fix (SPEC-0057) showed the same loss of data on every other common type.

## 2. Current State (MANDATORY, every depth)
Today, with the optional data tools enabled, the following is observed (evidence in 14.1).

### 2.1 What users can do today
- `db.query` and `db.sample` read a CSV, Parquet, JSON or NDJSON file and return rows. Numbers, booleans and text come back as real values.
- A date column comes back as a whole number of days since 1970 (`19675` for 2023-11-14), or for a date stored with a time of day as a number of milliseconds (`1700000000000`).
- A timestamp column stored in nanoseconds comes back as a text of digits (`"1700000000123456789"`). A timestamp stored in seconds, milliseconds or microseconds, or with a time zone, comes back as the text `?`. A CSV file whose cells look like `2023-11-14T22:13:20` is read as a timestamp in seconds, so it returns `?` as well.
- A decimal column comes back as the text `<Decimal128(10, 2)>`; a binary column as `<BinaryView>`; a list column as `<List(Int64)>`. The real value is lost.
- `db.convert` to NDJSON or JSON writes those same values into the file, so the loss is persisted. `db.convert` to CSV writes readable text: `2023-11-14`, `2023-11-14T22:13:20.123456789`, `123.45`, `00016162`.
- `db.schema` names the type of each column; `db.profile` shows minimum and maximum as text, which is already readable for these types.

### 2.2 Specs governing this area
- SPEC-0009/FR-017: the data tools return "rows, a schema, a profile, or a converted file". No requirement defines the value format of any type beyond numbers, booleans and text.
- SPEC-0055/DR-009 (byte identical output after the engine upgrade) recorded the current values as they were, without deciding they were right.
- SPEC-0057/BR-005 froze every output other than Parquet text. This specification deviates from it, for dates, timestamps, decimals and binary values only. Existing specs are not modified.

### 2.3 What is verified today
No existing test exercises a date, timestamp, decimal or binary column: the 25 data tool tests use whole numbers, decimals written as floating point, and text only. A change of output for these types would pass the suite unnoticed.

## 3. Scope
### 3.1 In scope
- Dates, timestamps (every unit, with or without a time zone), decimals and binary values in the results of `db.query`, `db.sample` and in the files written by `db.convert` to NDJSON or JSON.
- Their text equals the cell text that `db.convert` already writes into a CSV file for the same value.
- A missing value stays a missing value.

### 3.2 Out of scope, and why
- Lists, structs, maps and other nested values keep their placeholder: they have no CSV text to match and need a separate product decision (BL-0096).
- The type name shown by `db.schema` and `db.profile` for a binary column read from Parquet (BL-0097).
- Times of day, durations and intervals: no product listed them; they keep today's output (BL-0096).
- `db.profile` minimum and maximum, strings, numbers, booleans and the CSV conversion: already correct, unchanged.
- A compatibility mode that keeps the old day counts or `?`: the data tools are off by default and the old values are wrong or unreadable.

### 3.3 Constraints imposed by the context
None.

## 4. Actors
- **Analyst:** a person using the data tools through an assistant, to read and convert files.
- **Agent:** an automated assistant reading the same results as structured data.

## 4.5 Bounded contexts (MANDATORY at M and L)
| Context | What it owns | Key concepts |
|---|---|---|
| Data analytics tools | reading data files and returning rows, schemas, profiles and converted files | row, column, value text, conversion |

## 5. Usage scenarios
### SC-001: read typed values from a data file
**Actor:** Analyst
**Preconditions:** the data tools are enabled; a Parquet file `/t.parquet` holds one row with every kind of value of Section 9 (file T1); a CSV file `/c.csv` holds dates and dates with a time of day.
**Flow:**
1. The analyst asks `db.query` for `SELECT * FROM t` on `/t.parquet` → the row comes back with every date, timestamp, decimal and binary value as readable text.
2. The analyst asks `db.sample` for the first 5 rows of `/c.csv` → each date and date-time comes back as text.
**Postconditions:** no file is changed.
**Exceptions:**
- EXC-001a: a value is missing → it comes back as `null`.
- EXC-001b: a column holds nested values → the result still comes back, with the placeholder for that column.
**Interactions:** SC-002.

### SC-002: convert a data file to NDJSON
**Actor:** Analyst
**Preconditions:** `/t.parquet` as in SC-001.
**Flow:**
1. The analyst asks `db.convert` to turn `/t.parquet` into `/o.ndjson` → the response reports `rows_converted` 1 and format `ndjson`.
2. The analyst reads `/o.ndjson` → each line carries the same text the query returned.
**Postconditions:** `/o.ndjson` exists; the source is unchanged.
**Exceptions:**
- EXC-002a: the destination exists already → the conversion behaves as it does today (unchanged).
**Interactions:** SC-001; the CSV conversion of the same file shows the same text in its cells.

## 6. Functional requirements

### 6.1 New
#### FR-NEW-001 [EARS-E]: dates and timestamps as text
> WHEN `db.query` or `db.sample` returns a date or a timestamp THE data analytics tools SHALL return it as a text equal to the cell text `db.convert` writes into a CSV file for the same value.

- **Business rules:** a date without time of day reads `2023-11-14`; a date with a time of day or a timestamp without a time zone reads `2023-11-14T22:13:20`, followed by a fraction of 3, 6 or 9 digits when the column keeps milliseconds, microseconds or nanoseconds (`2023-11-14T22:13:20.123`, `.123456`, `.123456789`), never a fraction for seconds. A timestamp with a time zone ends with `Z` for UTC (`2023-11-14T22:13:20.123456Z`) or the offset otherwise (`2023-11-15T00:13:20.123456+02:00`). A moment before 1970 reads the same way (`1969-12-31T23:59:59`, `1969-12-31`).
- **User visible text:** the texts above, exactly.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: decimals as text
> WHEN `db.query` or `db.sample` returns a decimal value THE data analytics tools SHALL return it as a text equal to the cell text `db.convert` writes into a CSV file for the same value.

- **Business rules:** the text keeps every digit and the column's number of digits after the point: `123.45`, `-0.05`, `42` for a column with no digits after the point, `12345678901234567890.12` (no rounding, no exponent, never a JSON number).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-003 [EARS-E]: binary values as text
> WHEN `db.query` or `db.sample` returns a binary value THE data analytics tools SHALL return it as lower case hexadecimal text equal to the cell text `db.convert` writes into a CSV file for the same value.

- **Business rules:** two characters per byte, no prefix: bytes `00 01 61 62` read `00016162`, bytes `00 ff` read `00ff`; an empty value reads `` (the empty text), which differs from a missing value.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-004 [EARS-E]: conversion to NDJSON or JSON
> WHEN `db.convert` writes an NDJSON or JSON file from a source holding dates, timestamps, decimals or binary values THE data analytics tools SHALL write each of them as the same text FR-NEW-001, FR-NEW-002 and FR-NEW-003 define.

- **Business rules:** one line per row as today; the response fields `src_path`, `dst_path`, `rows_converted` and `format` are unchanged.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: missing values
> IF a date, timestamp, decimal or binary value is missing THEN THE data analytics tools SHALL return `null` for it, in query and sample results and in NDJSON or JSON files.

- **Business rules:** a missing value is never the empty text, never `?`, never a placeholder.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-006 [EARS-O]: nested values keep their placeholder
> IF a column holds nested values (lists, structs, maps) THEN THE data analytics tools SHALL return for each such value the text `<` followed by the type name that `db.schema` reports for the column, followed by `>`, and the call SHALL succeed.

- **Business rules:** this is today's behavior, kept on purpose (BL-0096). Example: a list of whole numbers reads `<List(Int64)>`.
- **Scenario:** SC-001 (EXC-001b)
- **Priority:** Must

#### FR-NEW-007 [EARS-UB]: nothing else moves
> The data analytics tools SHALL NOT change the value, the column list, the row count, the type names or the error messages returned for text, whole numbers, floating point numbers and booleans, nor the CSV file written by `db.convert`.

- **Scenario:** SC-001, SC-002
- **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-U]: previously recorded values for these types (modifies SPEC-0057/BR-005)
> The data analytics tools SHALL return the texts of FR-NEW-001 to FR-NEW-003 in place of the values SPEC-0057/BR-005 froze for dates, timestamps, decimals and binary values.

- **Before:** `19675`, `1700000000000`, `"1700000000123456789"`, `"?"`, `"<Decimal128(10, 2)>"`, `"<BinaryView>"` (Section 2.1).
- **After:** the readable texts above.
- **Reason:** the old values lose the data or are unreadable.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
### 7.1 Performance
Reading 10 000 rows of these types takes no more than 2 times the time the same rows took with the old values on the same file.
### 7.2 Security and access
Unchanged: only a member of the project can read its files.
### 7.3 Privacy and compliance
Not applicable: no new data is exposed.
### 7.4 Usability and accessibility
Not applicable.
### 7.5 Reliability
A file whose types are not covered here still converts or queries as before, never failing because of this change.
### 7.6 Operability
Not applicable.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | a missing date, timestamp, decimal, binary value | `null` | E2E-007 |
| Empty states | a missing nested value | `null` | E2E-015 |
| Empty states | an empty binary value | the empty text | E2E-011 |
| Boundary values | a decimal above 2^53 | all digits, `12345678901234567890.12` | E2E-008 |
| Boundary values | a moment before 1970 | `1969-12-31T23:59:59` | E2E-009 |
| Data variations | negative decimal, no digits after the point | `-0.05`, `42` | E2E-008 |
| Data variations | binary containing zero and 255 | `00ff` | E2E-011 |
| Ordering and timing | time zone with an offset | `2023-11-15T00:13:20.123456+02:00` | E2E-010 |
| Partial operations | a nested column next to supported ones | placeholder for it, text for the others | E2E-012 |
| Exhaustion, access, concurrency, connectivity, navigation, dependency failures | not applicable: no state, no external dependency, no new refusal paths | | |

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

**Fixture T1** (a Parquet file `/t.parquet`, one row): `ts_s` timestamp in seconds 1700000000; `ts_ms` in milliseconds 1700000000123; `ts_us` in microseconds 1700000000123456; `ts_ns` in nanoseconds 1700000000123456789; `ts_utc` in microseconds with time zone UTC 1700000000123456; `d32` date counted in days 19675; `d64` date with time of day in milliseconds 1700000000000; `dec` decimal 123.45 (10 digits, 2 after the point); `bin` bytes `00 01 61 62`; `n` whole number 5; `name` text `Alice`.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, FR-NEW-007, FR-MOD-001 | Critical |
| E2E-002 | New | journey | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-003 | New | journey | SC-001 | FR-NEW-002, FR-MOD-001 | Critical |
| E2E-004 | New | journey | SC-001 | FR-NEW-003, FR-MOD-001 | Critical |
| E2E-005 | New | journey | SC-001 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-006 | New | side effect | SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005 | Critical |
| E2E-007 | New | edge | SC-001 | FR-NEW-005 | High |
| E2E-008 | New | edge | SC-001 | FR-NEW-002 | High |
| E2E-009 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-010 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-011 | New | edge | SC-001 | FR-NEW-003, FR-NEW-004, FR-NEW-005 | High |
| E2E-012 | New | failure | SC-001 | FR-NEW-006 | High |
| E2E-013 | New | failure | SC-001, SC-002 | FR-NEW-006, FR-NEW-007 | High |
| E2E-014 | New | failure | SC-002 | FR-NEW-007 | High |
| E2E-015 | New | edge | SC-001 | FR-NEW-005, FR-NEW-006 | High |

Coverage: happy 6 (E2E-001 to 006, E2E-006 is also the side effect test), edge 6 (E2E-007 to 011 and E2E-015), non regression and nested value checks 3 (E2E-012 to 014), state transition 0 (no state), security 0 (`Security: n/a`). Failure paths in the strict sense (a refusal, an error) do not exist: this change adds none and FR-NEW-007 freezes the existing ones. Edge plus non regression (9) outnumber happy (6), ratio 1:1.5.

### 9.2 New tests
#### E2E-001: timestamps of every unit read as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-007
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT ts_s, ts_ms, ts_us, ts_ns, ts_utc, n, name FROM t` on `/t.parquet`
- **Then** the result is exactly one row `{"ts_s":"2023-11-14T22:13:20","ts_ms":"2023-11-14T22:13:20.123","ts_us":"2023-11-14T22:13:20.123456","ts_ns":"2023-11-14T22:13:20.123456789","ts_utc":"2023-11-14T22:13:20.123456Z","n":5,"name":"Alice"}`, `columns` lists the 7 names in that order and `row_count` is 1
- **Priority:** Critical

#### E2E-002: dates read as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT d32, d64 FROM t` on `/t.parquet`
- **Then** the row is `{"d32":"2023-11-14","d64":"2023-11-14T22:13:20"}`
- **Priority:** Critical

#### E2E-003: a decimal reads as text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT dec FROM t` on `/t.parquet`
- **Then** the row is `{"dec":"123.45"}` (a text, not the number 123.45)
- **Priority:** Critical

#### E2E-004: a binary value reads as hexadecimal text
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Given** fixture T1
- **When** the analyst runs `db.query` with `SELECT bin FROM t` on `/t.parquet`
- **Then** the row is `{"bin":"00016162"}`
- **Priority:** Critical

#### E2E-005: dates in a CSV file read as text, sample and query
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-005
- **Given** a CSV file `/c.csv` with the lines `d,ts,x`, `2023-11-14,2023-11-14T22:13:20,1.50` and `,,2.50`
- **When** the analyst runs `db.sample` with `n` 5 on `/c.csv`, then `db.query` with `SELECT d, ts, x FROM c ORDER BY x`
- **Then** both return rows `[{"d":"2023-11-14","ts":"2023-11-14T22:13:20","x":1.5},{"d":null,"ts":null,"x":2.5}]`
- **Priority:** Critical

#### E2E-006: conversion to NDJSON writes the same text, and agrees with CSV
- **Category:** side effect
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005
- **Given** fixture T1 extended with a second row where every typed column is missing, `n` is 6 and `name` is `Bob`
- **When** the analyst runs `db.convert` from `/t.parquet` to `/o.ndjson`, then from `/t.parquet` to `/o.csv`
- **Then** the first response is `{"src_path":"/t.parquet","dst_path":"/o.ndjson","rows_converted":2,"format":"ndjson"}`; line 1 of `/o.ndjson` is `{"ts_s":"2023-11-14T22:13:20","ts_ms":"2023-11-14T22:13:20.123","ts_us":"2023-11-14T22:13:20.123456","ts_ns":"2023-11-14T22:13:20.123456789","ts_utc":"2023-11-14T22:13:20.123456Z","d32":"2023-11-14","d64":"2023-11-14T22:13:20","dec":"123.45","bin":"00016162","n":5,"name":"Alice"}`; line 2 has `null` for the 9 typed columns, `"n":6` and `"name":"Bob"`; and each text of line 1 equals the matching cell of `/o.csv` line 2
- **And** `/t.parquet` is unchanged
- **Priority:** Critical

#### E2E-007: missing values are null
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Given** a Parquet file `/n.parquet` with one row where a date, a timestamp in microseconds, a decimal and a binary column are all missing
- **When** the analyst runs `db.query` with `SELECT * FROM n`
- **Then** the row has `null` for all four columns, and no `?`, no empty text, no placeholder
- **Priority:** High

#### E2E-008: decimal edge values
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Given** a Parquet file `/d.parquet` with one row: `big` decimal of 22 digits with 2 after the point 12345678901234567890.12; `neg` decimal of 10 digits with 2 after the point -0.05; `s0` decimal of 10 digits with 0 after the point 42
- **When** the analyst runs `db.query` with `SELECT big, neg, s0 FROM d`
- **Then** the row is `{"big":"12345678901234567890.12","neg":"-0.05","s0":"42"}`
- **Priority:** High

#### E2E-009: moments before 1970
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** a Parquet file `/p.parquet` with one row: a timestamp in seconds equal to one second before 1970-01-01T00:00:00 named `pre` and a date counted in days equal to one day before 1970-01-01 named `dpre`
- **When** the analyst runs `db.query` with `SELECT * FROM p`
- **Then** the row is `{"pre":"1969-12-31T23:59:59","dpre":"1969-12-31"}`
- **Priority:** High

#### E2E-010: a time zone with an offset
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Given** a Parquet file `/z.parquet` with one microsecond timestamp column `tz2` of time zone `+02:00` holding 1700000000123456
- **When** the analyst runs `db.query` with `SELECT tz2 FROM z`, then `db.convert` from `/z.parquet` to `/z.csv`
- **Then** the query row is `{"tz2":"2023-11-15T00:13:20.123456+02:00"}` and the `tz2` cell of `/z.csv` line 2 is the same text
- **Priority:** High

#### E2E-011: binary edge values
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-005
- **Given** a Parquet file `/b.parquet` with two columns, `b0` holding zero bytes and `bff` holding the bytes `00 ff`, plus a second row where both are missing
- **When** the analyst runs `db.query` with `SELECT b0, bff FROM b`, then `db.convert` from `/b.parquet` to `/b.ndjson`
- **Then** rows are `[{"b0":"","bff":"00ff"},{"b0":null,"bff":null}]` (the empty text differs from `null`), and `/b.ndjson` holds the two lines `{"b0":"","bff":"00ff"}` and `{"b0":null,"bff":null}`
- **Priority:** High

#### E2E-012: a nested column keeps its placeholder
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Given** a Parquet file `/l.parquet` with a list of whole numbers column `lst` holding `[1, 2]` and a date column `d32` holding day 19675
- **When** the analyst runs `db.schema` on it, then `db.query` with `SELECT lst, d32 FROM l`, then `db.convert` from `/l.parquet` to `/l.ndjson`
- **Then** `db.schema` reports `List(Int64)` for `lst`; the query succeeds with `{"lst":"<List(Int64)>","d32":"2023-11-14"}`; the conversion succeeds and `/l.ndjson` is `{"lst":"<List(Int64)>","d32":"2023-11-14"}` followed by a newline
- **Priority:** High

#### E2E-013: plain columns do not move, in the same file as a nested one
- **Category:** failure (non regression)
- **Scenario:** SC-001, SC-002
- **Requirements:** FR-NEW-006, FR-NEW-007
- **Given** the 3 row file used by the existing characterization tests (whole numbers `id`, text `name`, floating point `score`) as CSV, NDJSON and Parquet, and the file `/l.parquet` of E2E-012 with an added whole number column `k` holding 7
- **When** the analyst runs `db.query` with `SELECT id, name, score FROM t ORDER BY id` on each of the three files, and `db.query` with `SELECT k FROM l` on `/l.parquet`
- **Then** the three results equal `{"columns":["id","name","score"],"rows":[{"id":1,"name":"Alice","score":1.5},{"id":2,"name":"Bob","score":2.5},{"id":3,"name":"Carol","score":4.0}],"row_count":3}` and the last equals `{"columns":["k"],"rows":[{"k":7}],"row_count":1}`
- **Priority:** High

#### E2E-014: CSV conversion and type names do not move
- **Category:** failure (non regression)
- **Scenario:** SC-002
- **Requirements:** FR-NEW-007
- **Given** fixture T1
- **When** the analyst runs `db.convert` from `/t.parquet` to `/o.csv`, then `db.schema` on `/t.parquet`
- **Then** `/o.csv` is exactly `ts_s,ts_ms,ts_us,ts_ns,ts_utc,d32,d64,dec,bin,n,name` then `2023-11-14T22:13:20,2023-11-14T22:13:20.123,2023-11-14T22:13:20.123456,2023-11-14T22:13:20.123456789,2023-11-14T22:13:20.123456Z,2023-11-14,2023-11-14T22:13:20,123.45,00016162,5,Alice`, and `db.schema` reports the type names `Timestamp(s)`, `Timestamp(ms)`, `Timestamp(µs)`, `Timestamp(ns)`, `Timestamp(µs, "UTC")`, `Date32`, `Date64`, `Decimal128(10, 2)`, `BinaryView`, `Int64`, `Utf8` in that order
- **Priority:** High

#### E2E-015: a missing nested value is null, not a placeholder
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** a Parquet file `/m.parquet` with a list of whole numbers column `lst` holding `[1, 2]` in row 1 and missing in row 2
- **When** the analyst runs `db.query` with `SELECT lst FROM m`
- **Then** rows are `[{"lst":"<List(Int64)>"},{"lst":null}]`
- **Priority:** High

### 9.3 Exploit tests
Not applicable: `Security: n/a`.

### 9.4 Modified tests
None: no existing test covers these types (Section 2.3).

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001 to FR-NEW-007 | E2E-001, 002, 003, 004, 005 | E2E-012, 013 | E2E-007, 008, 009, 010, 011, 015 |
| SC-002 | FR-NEW-001 to FR-NEW-007 | E2E-006 | E2E-013, 014 | E2E-011 |

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, 002, 005, 009, 010 |
| FR-NEW-002 | E2E-003, 006, 008 |
| FR-NEW-003 | E2E-004, 006, 011 |
| FR-NEW-004 | E2E-006, 011, 012 |
| FR-NEW-005 | E2E-005, 006, 007, 011, 015 |
| FR-NEW-006 | E2E-012, 013, 015 |
| FR-NEW-007 | E2E-001, 013, 014 |
| FR-MOD-001 | E2E-001, 002, 003, 004, 006 |

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Data tools | the optional `db.query`, `db.sample`, `db.schema`, `db.profile` and `db.convert` | whole spec |
| CSV cell text | the text `db.convert` writes in a CSV file for a value | FR-NEW-001 to FR-NEW-003 |
| Missing value | a value absent from the data file | FR-NEW-005 |
| Nested value | a value that contains other values (list, struct, map) | FR-NEW-006 |
| Placeholder | the text `<` + type name + `>` standing in for a value that is not shown | FR-NEW-006 |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 2 | IMPLEMENTABLE |

**Amendments applied:** A-1 (coverage statistics, FR-MOD-001 traced to E2E-001 to 004 and 006, SC-002 row), A-2 (design inputs for other decimal and binary widths, types left unchanged); auditor: a fresh context subagent, same model as the author
**No implementation detail outside Section 14:** PASS (tool names, file format names and the verbatim texts a user sees are the product vocabulary; no path, type, library or function name appears in sections 1 to 13)

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| dates come back as the number of days or milliseconds | `crates/core/src/tools/db.rs:206-211` (the `Date32` and `Date64` arms return the raw integer) |
| timestamps in nanoseconds come back as digits, other units as `?` | `db.rs:212-219` (downcast to the nanosecond array only, `unwrap_or_else(\|\| "?")`) |
| decimals, binary and nested values come back as `<type>` | `db.rs:220` (`other => json!(format!("<{other}>"))`) |
| a CSV date or datetime column is read as a date or a timestamp in seconds | observed on HEAD: `db.schema` on `d,ts,x` returned `Date32`, `Timestamp(s)`, `Float64`; `db.query` returned `{"d":19675,"ts":"?","x":1.5}` |
| `db.query` and `db.sample` use the row serializer | `db.rs:329` and `db.rs:531` (both call `batches_to_json`, `db.rs:139`, which calls `arrow_value_to_json`, `db.rs:168`) |
| NDJSON and JSON conversion persist the same values | `db.rs:255-257` (`batches_to_ndjson` reuses `batches_to_json`) |
| CSV conversion writes readable text | `db.rs:225-232` (the library CSV writer); observed output `…,2023-11-14,2023-11-14T22:13:20,123.45,00016162,5` on HEAD |
| `db.profile` min and max are already text | `db.rs:446-447` (`CAST(.. AS VARCHAR)`), rendered by `db.rs:471-472` |
| `db.schema` and `db.profile` type names | `db.rs:126-131` (`type_name`, SPEC-0057), used at `db.rs:374` and `db.rs:480` |
| no test covers these types | `rg "Date|Timestamp|Decimal|Binary" crates/core/src/tools/db.rs` after the line of `mod tests` (`db.rs:621`) returns no match; 25 test functions in `db.rs` |
| a list column cannot be converted to CSV | observed on HEAD: `csv write: Csv error: Nested type List(Int64) is not supported in CSV` |

Observed values of the edge tests (all read on HEAD through a throwaway test, reverted): CSV cells `12345678901234567890.12`, `-0.05`, `42`, `1969-12-31T23:59:59`, `1969-12-31`, `2023-11-15T00:13:20.123456+02:00`, the empty text for empty binary, `00ff`.

Test command: `cargo test --workspace` (gate: `make check`).

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- The requirement is parity with the CSV writer; the CSV writer is `arrow::csv::WriterBuilder` at `db.rs:230` with default formats. A formatter shared with it keeps the two paths in step; a hand written one needs a parity test (E2E-006 and E2E-010 are that test).
- A Parquet binary column is typed as a view on this engine (`BinaryView`, observed); the serializer needs an arm for that type and for plain binary, as SPEC-0057 did for `Utf8View` (`db.rs:199-203`).
- A timestamp column of any unit and zone, and the nanosecond timestamp stored as digits today, must all change; the `"?"` arm exists because the downcast assumes nanoseconds (`db.rs:212-219`).
- `db.convert` Parquet to Parquet and CSV to Parquet write arrays, not JSON: unaffected, to be confirmed by a test.
- The performance bound (Section 7.1) needs a measured baseline on the old values.
- Decimal values of any width, and plain, large and fixed width binary values, belong to FR-NEW-002 and FR-NEW-003 and need their own arms; times of day, durations and intervals are covered by no requirement and keep `<type>` through the default arm (`db.rs:220`, Section 7.5). Detection signal: a Parquet fixture per type in a parity test against the CSV conversion, as E2E-006 and E2E-010 do for the covered types.
