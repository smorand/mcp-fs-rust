# US-0107: return Parquet strings as text from the db tools

> Spec: SPEC-0057 (sdlc/specs/SPEC-0057-db-parquet-strings-as-text/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
A string read from a Parquet file comes back as `<Utf8View>` from `db.query`, `db.sample`, `db.profile`, `db.schema` and, worse, is written as that placeholder by `db.convert` to NDJSON or JSON. Make every string a real JSON string and make `db.schema` and `db.profile` report `Utf8`, without moving any other output.

## Requirements covered
- SPEC-0057/BR-001: WHEN `db.query` or `db.sample` returns a string value read from a Parquet file THE server SHALL return it as a JSON string equal to the stored text.
- SPEC-0057/BR-002: WHEN `db.convert` reads a Parquet file and its `dst_path` ends in `.ndjson`, `.jsonl` or `.json` THE server SHALL write every string value as a JSON string equal to the stored text.
- SPEC-0057/BR-003: WHEN `db.schema` describes a Parquet file THE server SHALL report `"data_type": "Utf8"` for each string column, the value it reports for the same column read from CSV or NDJSON.
- SPEC-0057/BR-004: WHEN `db.profile` profiles a Parquet file THE server SHALL report `"type": "Utf8"` for each string column and keep `non_null`, `distinct`, `min` and `max` as today.
- SPEC-0057/BR-005: The server SHALL NOT change the output of the `db.*` tools for any value that is not a string read from a Parquet file.

## Independent Test
`cargo test -p mcp-fs-core --lib bt_0057 spec_0055` is green; then, with `db.enabled = true`, `db.query` `SELECT id, name FROM t ORDER BY id` on a Parquet made from CSV rows `1,Alice` and `2,Bob` returns `"name":"Alice"` and `"name":"Bob"`, and `/o.ndjson` produced by `db.convert` reads `{"id":1,"name":"Alice"}\n{"id":2,"name":"Bob"}\n`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0057): Fix on the output side: a `Utf8View` arm in `arrow_value_to_json` plus a `type_name` helper at the two print sites; do not change how the Parquet reader types columns (`schema_force_view_types = false` would also change the `BinaryView` output, which BR-005 freezes).
- DEC-002 (SPEC-0057): Keep the placeholder arm and the `Timestamp` arm untouched; nothing for binary, decimal, list or non nanosecond timestamps (BL-0091).
### Data model
None.
### Contracts
No tool schema change. New private helper in `crates/core/src/tools/db.rs`: `fn type_name(t: &DataType) -> String` returning `"Utf8"` for `DataType::Utf8View`, `t.to_string()` otherwise. Used at `db.rs:360` (`"data_type"`) and `db.rs:466` (`"type"`), replacing `format!("{}", ..data_type())`. In `arrow_value_to_json` (`db.rs:158-207`) add next to the `Utf8` arm (`db.rs:186`) an arm `DataType::Utf8View => json!(col.as_any().downcast_ref::<StringViewArray>().unwrap().value(row))` (import `arrow::array::StringViewArray`; the downcast is infallible by the matched `data_type()`, as for the existing arms).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/db.rs` | modify (arm, helper, two call sites, stale comment `:101-104`, existing tests, new tests) |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (BT-001, BT-002) |
| `.agent_docs/dependencies.md` | modify (line 11: drop "A Parquet string column still comes out as the text `<Utf8View>`: that is the existing bug BL-0090, not fixed here.", note it was fixed by SPEC-0057) |
### Existing patterns
- `db.rs` tests: `db_harness` (`:721`), `three_row_harness` (`:730`), `call_on` (`:743`), `zstd_parquet()` as the model for writing a Parquet with the arrow writer (BT-012).
- `tool_surface_e2e.rs`: `bt_001_an_enabled_sqlite_family_is_callable` (`:374`) for `setup`, `c.db.enabled = true`, `app_with_atlas`, `call_tool`, `success_payload`; seed the CSV with `fs.write` (`mount_id`, `path`, `content`), read the file back with `fs.read_bytes` (`base64` field).

## Acceptance tests
> 100% must pass, run through `make check` (`cargo test --workspace` and the gate in `AGENTS.md`). Never run test files ad hoc. BUG: BT-001, BT-002, BT-010 to BT-013 are RED before the production change (the payload holds `"<Utf8View>"`), GREEN after.

### SPEC-0057/BT-001: Parquet strings come back as text through the MCP route
- Level / Driver / Location: functional e2e / MCP `tools/call` / `tool_surface_e2e.rs`, `bt_0057_001_*`
- Given `c.db.enabled = true`, `/t.csv` containing `id,name\n1,Alice\n2,Bob\n` written by `fs.write`, converted by `db.convert` to `/t.parquet`
- When `db.query` runs `SELECT id, name FROM t ORDER BY id` on `/t.parquet`
- Then `success_payload` equals exactly `{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}],"row_count":2}`
- Red before the fix because: `name` is `"<Utf8View>"` (`db.rs:206`).

### SPEC-0057/BT-002: a converted NDJSON file holds the text
- Level / Driver / Location: functional e2e / MCP `tools/call` / `tool_surface_e2e.rs`, `bt_0057_002_*`
- Given `/t.parquet` as in BT-001
- When `db.convert` converts `/t.parquet` to `/o.ndjson`
- Then the response is `{"src_path":"/t.parquet","dst_path":"/o.ndjson","rows_converted":2,"format":"ndjson"}` and the decoded `fs.read_bytes` of `/o.ndjson` is exactly `{"id":1,"name":"Alice"}\n{"id":2,"name":"Bob"}\n`
- Red before the fix because: the file holds `"name":"<Utf8View>"` (`db.rs:242`).

### SPEC-0057/BT-010: schema type of a Parquet string column
- Level / Driver / Location: integration / `db_harness` / `db.rs`, `bt_0057_010_*`
- Given `three_row_harness()` (`/t.csv`, `/t.parquet`, `/t.ndjson`)
- When `db.schema` runs on each
- Then all three return exactly `schema_of(["Int64", "Utf8", "Float64"])`

### SPEC-0057/BT-011: profile of a Parquet string column
- Level / Driver / Location: integration / `db_harness` / `db.rs`, `bt_0057_011_*`
- Given `three_row_harness()`
- When `db.profile` runs on `/t.parquet`
- Then the result equals `profile_of("Utf8")`

### SPEC-0057/BT-012: edge values survive a Parquet round trip
- Level / Driver / Location: integration / `db_harness` / `db.rs`, `bt_0057_012_*`
- Given a Parquet written with the arrow writer to `/t.parquet`, nullable string column `s` holding `"héllo 日本"`, `""`, null, `"a string well over twelve bytes"`
- When `db.query` runs `SELECT s FROM t` and `db.convert` writes `/o.ndjson`
- Then `rows` equal `[{"s":"héllo 日本"},{"s":""},{"s":null},{"s":"a string well over twelve bytes"}]` in that order, and `/o.ndjson` read by `h.client().await.read_text` is the same four objects, one per line, newline terminated

### SPEC-0057/BT-013: computed and sampled strings
- Level / Driver / Location: integration / `db_harness` / `db.rs`, `bt_0057_013_*`
- Given `three_row_harness()`
- When `db.query` runs `SELECT upper(name) AS u FROM t ORDER BY id` on `/t.parquet`, and `db.sample` runs on it with `n` 2
- Then `u` is `"ALICE"`, `"BOB"`, `"CAROL"`, and `db.sample` returns `{"id":1,"name":"Alice","score":1.5}` and `{"id":2,"name":"Bob","score":2.5}`

### SPEC-0057/BT-020: other outputs do not move
- Level / Driver / Location: integration / existing tests / `db.rs`
- Given the existing tests `spec_0055_dt_001`, `dt_003`, `dt_005`, `dt_007` to `dt_011`, `query_csv_in_volume`, `schema_returns_column_types`, `sample_returns_n_rows`
- When they run unchanged
- Then they pass with their present assertions, and `db.query` on `/t.parquet` returns `id` as `1`, `2`, `3` and `score` as `1.5`, `2.5`, `4.0`

### Existing tests to modify (expected values only)
`spec_0055_dt_002_query_parquet` (`name` `Alice`, `Bob`, `Carol`), `spec_0055_dt_004_schema_type_names` (Parquet `Utf8`), `spec_0055_dt_006_profile_parquet` (`profile_of("Utf8")`), `id_name_rows()` used by `spec_0055_dt_040_zstd_parquet_is_read` and `spec_0055_dt_041_corrupt_zstd_page_is_rejected` (`Alice`, `Bob`). Update their doc comments, which still describe the placeholder.

## Constraints
- Files not to touch: `Cargo.toml`, `Cargo.lock`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, anything under `sdlc/specs/`.
- Dependencies not to add: none; do not set `schema_force_view_types` in `session()` (DEC-001).
- Patterns to avoid: touching the placeholder arm or the `Timestamp` arm (BL-0091); `unwrap` on a fallible path (the arm's downcast is guarded by the matched type, like its neighbours).
- Scope boundary: binary, decimal, list and timestamp handling stay as they are.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (BT-001, BT-002, BT-010 to BT-013 fail on the placeholder).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`).
- [ ] Non regression: the tests listed in BT-020 pass unmodified; `tool-contract-golden.json` unchanged.
- [ ] Docs updated: `.agent_docs/dependencies.md` line 11; the comment at `db.rs:101-104`. No dashes as punctuation.
- [ ] Self review: full (idiomatic Rust per the rust skill, no `unwrap` on fallible paths, BR-005 respected).
- [ ] Closes spec: the spec's full acceptance set green (BT-001, BT-002, BT-010 to BT-013, BT-020), and every requirement BR-001 to BR-005 cited by at least one story with status done or landed.
- [ ] One commit `US-0107: return Parquet strings as text from the db tools`, body `Spec: SPEC-0057` and the requirement ids.
