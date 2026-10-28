# db tools return the text `<Utf8View>` for Parquet string columns: Bug Specification

> Generated on: 2026-10-28
> Id: SPEC-0057
> Nature: BUG
> Depth: S
> Depth evidence: 1 module (`crates/core/src/tools/db.rs`), 5 requirements, no public contract change (tool names, descriptions and input schemas untouched), no data model change
> Status: implemented
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0090
> Security: n/a
> CVSS: n/a
> Affected: every build since the datafusion upgrade of SPEC-0055 (HEAD `59be600`), when the optional `db` family is enabled (`db.enabled = true`, off by default)
> Fixed in: unreleased

## 1. Defect

### BUG-001: a string read from a Parquet file comes back as the placeholder `<Utf8View>`
- **Observed behavior:** For a Parquet file, `db.query` and `db.sample` return the literal text `<Utf8View>` as the value of every non null string, `db.schema` and `db.profile` report the type `Utf8View`, and `db.convert` to `.ndjson` or `.json` writes that placeholder into the destination file. Verified by a throwaway test run on HEAD (reverted): CSV `id,name` rows `(1,Alice)`, `(2,)` converted to `/t.parquet`, then
  - `db.convert` `/t.parquet` to `/o.ndjson` wrote `{"id":1,"name":"<Utf8View>"}\n{"id":2,"name":null}\n`;
  - `db.sample` returned `{"id":1,"name":"<Utf8View>"}`;
  - `db.query` `SELECT upper(name) AS u, name FROM t` returned `"u":"<Utf8View>","name":"<Utf8View>"`;
  - `db.profile` returned `"type":"Utf8View"` with `min` and `max` both `"Alice"`.
- **Expected per authority:** `sdlc/specs/SPEC-0009-optional-integrations/spec.md:449` (E2E-019): "The same data as CSV, Parquet and JSON report identical columns/types via `db.schema`; `db.convert` CSV→Parquet produces a file `db.schema` reads with the same columns". `spec.md:306-311` (FR-017): the `db.*` tools output "rows, a schema, a profile, or a converted file". The `db.query` tool description (`crates/core/src/tools/db.rs:270-273`) promises a query run against the file, and the backlog entry names the same description.
- **Reproduction:** Preconditions: `db.enabled = true`, a volume holding `/t.csv` with `id,name` and rows `1,Alice` and `2,Bob`. Steps: `db.convert` `/t.csv` to `/t.parquet`, then `db.query` on `/t.parquet` with `SELECT id, name FROM t ORDER BY id`. Expected: `name` is `"Alice"` then `"Bob"`. Actual: `"<Utf8View>"` twice.
- **Root cause:** The Parquet table is registered with `ParquetReadOptions::default()` (`db.rs:88`), whose `schema_force_view_types` defaults to true (datafusion `datafusion/common/src/config.rs:1375` at the pinned rev `6f1436f`, `Cargo.toml:126`), so string columns arrive as arrow `Utf8View`. The row serializer `arrow_value_to_json` (`db.rs:158-207`) has arms for `Utf8` (`db.rs:186`) and `LargeUtf8` (`db.rs:189`) only and no `Utf8View` arm, so the column falls to `other => json!(format!("<{other}>"))` (`db.rs:206`). `db.schema` (`db.rs:360`) and `db.profile` (`db.rs:466`) print `f.data_type()` unchanged, so they print `Utf8View`. `db.convert` to NDJSON reuses the row serializer (`db.rs:241-243`, dispatch `db.rs:574`), so the placeholder is persisted. Round trip checked: writer `batches_to_parquet` (`db.rs:225`) stores CSV `Utf8` as a Parquet string; reader is the registration above; the CSV and NDJSON readers produce `Utf8` (SPEC-0055 DT-004, `db.rs:827-835`).
- **Contributing factors:** SPEC-0055 characterized the placeholder as current behavior and made it a requirement (DR-009 byte identical output, tests DT-002, DT-004, DT-006, DT-040, DT-041), so the datafusion upgrade could not fix it. The session pin `map_string_types_to_utf8view = false` (`db.rs:105-110`) covers SQL `VARCHAR` casts only, not the Parquet reader.
- **Impact:** Any user of `db.query`, `db.sample`, `db.schema` or `db.profile` on a Parquet file loses every string value. `db.convert` Parquet to NDJSON or JSON silently writes a wrong file; the destination exists and looks valid, and the source is untouched. Files already written that way keep the placeholder; this specification does not repair them. No workaround except converting to CSV (`db.convert` to `.csv` writes real strings, `db.rs:211` and `db.rs:572`, and DT-008).
- **Rollback posture:** A revert restores the placeholder output. Files written by `db.convert` while the fix shipped hold real text and stay readable by either version. No data is migrated.

## 2. Expected Behavior

All names are the codebase's snake_case JSON fields. Tools: `db.query`, `db.sample`, `db.schema`, `db.profile`, `db.convert`.

#### BR-001 [EARS-E]: strings read from Parquet in query and sample results
> WHEN `db.query` or `db.sample` returns a string value read from a Parquet file THE server SHALL return it as a JSON string equal to the stored text.

- **Source:** restored from SPEC-0009 FR-017 and the `db.query` description; the same rows in a CSV already return the text (DT-001, DT-003).
- **Inputs:** a `.parquet` or `.parq` path, `sql` for `db.query`, `n` for `db.sample`.
- **Outputs:** `rows[i].<column>` is a JSON string. A string computed by SQL from a Parquet column (for example `upper(name)`) is also a JSON string.
- **Business Rules:** `columns` and `row_count` keep their present meaning and values.

#### BR-002 [EARS-E]: strings written by conversion
> WHEN `db.convert` reads a Parquet file and its `dst_path` ends in `.ndjson`, `.jsonl` or `.json` THE server SHALL write every string value as a JSON string equal to the stored text.

- **Source:** restored from SPEC-0009 FR-017 ("a converted file") and E2E-019.
- **Outputs:** one JSON object per line, as today (`db.rs:241-251`), with `"name":"Alice"` where the file held `Alice`. The response keys `src_path`, `dst_path`, `rows_converted`, `format` are unchanged.

#### BR-003 [EARS-E]: schema type of a Parquet string column
> WHEN `db.schema` describes a Parquet file THE server SHALL report `"data_type": "Utf8"` for each string column, the value it reports for the same column read from CSV or NDJSON.

- **Source:** restored from SPEC-0009 E2E-019 (identical columns/types across formats).
- **Exact names:** array entries `{"name", "data_type", "nullable"}` unchanged; only the string value `Utf8View` becomes `Utf8`.

#### BR-004 [EARS-E]: profile type of a Parquet string column
> WHEN `db.profile` profiles a Parquet file THE server SHALL report `"type": "Utf8"` for each string column and keep `non_null`, `distinct`, `min` and `max` as today.

- **Source:** decided here, ASSUMED: `db.profile` takes its type name from the same field as `db.schema` (`db.rs:466` and `db.rs:360`), so it follows BR-003 (E2E-019 names `db.schema` only).

#### BR-005 [EARS-UB]: no other output moves
> The server SHALL NOT change the output of the `db.*` tools for any value that is not a string read from a Parquet file.

- **Source:** decided here, ASSUMED: scope is limited to BL-0090. Integer, float, boolean, CSV and NDJSON string outputs stay as recorded by SPEC-0055 (DT-001, DT-003, DT-005, DT-007 to DT-011). The placeholder arm for binary, decimal, list and non nanosecond timestamp values (`db.rs:206`, `db.rs:198-205`) stays out of scope and belongs to BL-0091.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| SQL NULL in a Parquet string column | JSON `null` (as today, `db.rs:159-161`) | BR-001, BR-002 |
| Empty string `""` in a Parquet string column | `""`, distinct from `null` | BR-001, BR-002 |
| String longer than 12 bytes (stored out of line by arrow views) | the full text | BR-001, BR-002 |
| Non ASCII text, for example `héllo 日本` | the same text, UTF-8, no escaping loss | BR-001, BR-002 |
| `db.convert` Parquet to `.csv` | unchanged (real text already, DT-008) | BR-005 |
| `db.profile` `min` and `max` on a Parquet string column | unchanged text, `Alice` and `Carol` for DT-006 data | BR-004 |

## 4. Tests

Driver for e2e tests: the MCP `tools/call` request through the app router with `db.enabled = true`, the `call_tool` and `success_payload` helpers of `crates/core/src/tools/tool_surface_e2e.rs:351-370`. Unit level tests of the `db.*` handlers use the existing `db_harness` (`db.rs:721`, `three_row_harness` at `db.rs:730`).

### 4.1 End to end reproduction (mandatory)
#### BT-001: Parquet strings come back as text through the MCP route
- **Reproduces:** BUG-001
- **Driver:** MCP `tools/call` over the app router
- **Fails today because:** `arrow_value_to_json` has no `Utf8View` arm (`db.rs:206`).
- Given `/t.csv` containing `id,name\n1,Alice\n2,Bob\n` written to the volume and converted by `db.convert` to `/t.parquet`
- When `db.query` runs `SELECT id, name FROM t ORDER BY id` on `/t.parquet`
- Then the payload is exactly `{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}],"row_count":2}`

#### BT-002: a converted NDJSON file holds the text
- **Reproduces:** BUG-001
- **Driver:** MCP `tools/call`, then the file read back through `fs.read`
- **Fails today because:** the NDJSON writer reuses the row serializer (`db.rs:242`).
- Given `/t.parquet` as in BT-001
- When `db.convert` converts `/t.parquet` to `/o.ndjson`
- Then the response is `{"src_path":"/t.parquet","dst_path":"/o.ndjson","rows_converted":2,"format":"ndjson"}` and `/o.ndjson` reads exactly `{"id":1,"name":"Alice"}\n{"id":2,"name":"Bob"}\n`

### 4.2 New behavior
#### BT-010: schema type of a Parquet string column
- **Validates:** BR-003
- Given the DT-004 data (`/t.csv`, `/t.parquet`, `/t.ndjson`)
- When `db.schema` runs on each
- Then all three return exactly `schema_of(["Int64", "Utf8", "Float64"])`

#### BT-011: profile of a Parquet string column
- **Validates:** BR-004
- Given the DT-006 data
- When `db.profile` runs on `/t.parquet`
- Then the result equals `profile_of("Utf8")`, the CSV result of DT-005

#### BT-012: edge values survive a Parquet round trip
- **Validates:** BR-001, BR-002
- Given a Parquet file written with the arrow writer, column `s` (nullable string) holding `"héllo 日本"`, `""`, `NULL`, and `"a string well over twelve bytes"`
- When `db.query` runs `SELECT s FROM t` and `db.convert` writes `/o.ndjson`
- Then `rows` are `{"s":"héllo 日本"}`, `{"s":""}`, `{"s":null}`, `{"s":"a string well over twelve bytes"}` in file order, and the NDJSON lines are the same four objects

#### BT-013: computed and sampled strings
- **Validates:** BR-001
- Given the DT-002 data
- When `db.query` runs `SELECT upper(name) AS u FROM t ORDER BY id` on `/t.parquet`, and `db.sample` runs on it with `n` 2
- Then `u` is `"ALICE"`, `"BOB"`, `"CAROL"`, and `db.sample` returns the first two rows with `name` `"Alice"`, `"Bob"`

### 4.3 Non regression
#### BT-020: other outputs do not move
- **Protects:** BR-005
- Given the DT-001 to DT-011 data
- When the existing tests DT-001, DT-003, DT-005, DT-007, DT-008, DT-009, DT-010, DT-011 run unchanged
- Then they pass with their present assertions, and `db.query` on `/t.parquet` returns `id` as the JSON integers `1`, `2`, `3` and `score` as `1.5`, `2.5`, `4.0`

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/tools/db.rs:792` | `spec_0055_dt_002_query_parquet` | Modified | `name` becomes `Alice`, `Bob`, `Carol` (BR-001) |
| `crates/core/src/tools/db.rs:827` | `spec_0055_dt_004_schema_type_names` | Modified | Parquet string type becomes `Utf8` (BR-003) |
| `crates/core/src/tools/db.rs:859` | `spec_0055_dt_006_profile_parquet` | Modified | `profile_of("Utf8")` (BR-004) |
| `crates/core/src/tools/db.rs:1005` | `spec_0055_dt_040_zstd_parquet_is_read` | Modified | `id_name_rows()` (`db.rs:993`) holds `Alice`, `Bob` (BR-001) |
| `crates/core/src/tools/db.rs:1021` | `spec_0055_dt_041_corrupt_zstd_page_is_rejected` | Modified | final assertion uses `id_name_rows()`; rejection assertions unchanged |
| `crates/core/src/tools/db.rs` | `spec_0055_dt_001`, `dt_003`, `dt_005`, `dt_007` to `dt_011`, `query_csv_in_volume`, `schema_returns_column_types`, `sample_returns_n_rows` | Kept | CSV and NDJSON paths and numeric columns do not move (BR-005) |
| `crates/core/src/tools/tool_surface_e2e.rs` | `db.` family count and catalog tests (`:319`, `:543`) | Kept | tool names and schemas do not change |

## 5. Existing Specs Affected
- `sdlc/specs/SPEC-0055-datafusion-and-zstd-upgrade/sdlc-design.md` DR-009 (byte identical `db.*` output, DT-002, DT-004, DT-006, DT-040, DT-041 recorded as the placeholder): this specification deviates on purpose for the Parquet string column. SPEC-0055 is not modified; BL-0090 recorded the deviation as intended.
- `sdlc/specs/SPEC-0009-optional-integrations/spec.md` E2E-019, FR-017: restored, not changed.
- `.agent_docs/dependencies.md:11` says the placeholder "is the existing bug BL-0090, not fixed here" and the comment at `db.rs:101-104` says the same; both go stale when the fix ships. Not edited here.

## 6. Assumptions
- **Assumption:** the type name for a Parquet string column is `Utf8`, because E2E-019 requires identical types across formats and CSV and NDJSON report `Utf8` (`db.rs:831-835`). Shaped BR-003.
- **Assumption:** BR-004 follows BR-003 (see its Source). No question asked: the repo answers it.
- **Assumption:** the other placeholder arms (binary, decimal, list, non nanosecond timestamp) are not fixed here, although the backlog entry mentions them in its What section; they are BL-0091 (`sdlc/backlog/BL-0091-db-types-not-characterized.md`). Shaped BR-005.
- **Assumption:** the backlog evidence line numbers (`db.rs:136`, `db.rs:184`) are stale; the cited ones above were re read on HEAD `59be600`.
- **Backlog entries created by this run:** none. Consumed: BL-0090.
- Overshoot note: none.

## 7. Implementability Checklist
| # | Item | Verdict |
|---|------|---------|
| 1 | No technical decision | PASS (root cause lines are facts; no fix, file list or id) |
| 2 | No buried requirement | PASS |
| 3 | Every observable name spelled | PASS (tool names, `data_type`, `type`, `rows`, `columns`, `row_count`, snake_case, `Utf8`) |
| 4 | No forced choice | PASS (null, empty string, long and non ASCII text stated; BR-004 type value stated) |
| 5 | Every test specified, blast radius in 4.4 | PASS |
| 6 | Reproduction test end to end | PASS (BT-001, BT-002 assert the user visible payload and file) |
| 7ante | Security misclassification | PASS (no authn, injection, exposure, traversal or advisory; `Security: n/a` holds) |
| 8 | No hidden prerequisite | PASS (`db.enabled = true` and the files are in the Givens) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (behavior changes, no new capability; 5 requirements, within depth S) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS (counts: 4 modified tests plus 1 modified through helper, recounted with `rg` on `db.rs`) |
| 13 | Round trip read | PASS (writer `db.rs:225` and `db.rs:88`, reader `db.rs:158-207`; CSV and NDJSON parity by DT-004) |
| 14 | Verdict consistent | PASS (BR-001 to BR-003 cite the authority; BR-004 and BR-005 are labeled decided here) |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
