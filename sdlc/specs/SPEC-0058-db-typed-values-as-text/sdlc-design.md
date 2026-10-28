# SPEC-0058: Dates, timestamps, decimals and binary values as readable text in the data tools, design

> Spec: sdlc/specs/SPEC-0058-db-typed-values-as-text/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-28
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, runtime | Rust 2024, tokio (unchanged) | `Cargo.toml` | n/a |
| Value text | `arrow::util::display::{ArrayFormatter, FormatOptions}`, already in the dependency tree through `arrow` | arrow 60 (`Cargo.toml:127`), `arrow-cast` 60.0.0 | DEC-001 |

No dependency added. Skills loaded: `rust` (the `make check` gate, in module tests plus MCP route e2e, no `unwrap` on fallible paths). Gate commands (`AGENTS.md`, Quality gate): `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

## 2. Approaches
### Chosen: the CSV writer's own formatter, built once per column
For each covered column, build an `ArrayFormatter` with `FormatOptions::default()` once per batch and render each non null cell with it. The CSV writer builds the same formatter from the same default options (`arrow-csv-60.0.0/src/writer.rs:258-276`), so FR-NEW-001 to FR-NEW-003 hold by construction and any future type the formatter learns follows. Pros: no format to invent, parity with CSV for free, one code path for query, sample and NDJSON. Cons: ties the JSON text to the arrow formatter's output (a library upgrade could change a text; the parity test E2E-006 and the fixed texts of E2E-001 to E2E-010 catch it). Scope: `crates/core/src/tools/db.rs`.
### Rejected: a hand written renderer per type
Format dates with `chrono`, decimals by string arithmetic, hex by hand. More code, a parity risk with CSV, a new place for rounding and zone bugs (DEC-001).
### Rejected: cast every column to text in SQL
`CAST(.. AS VARCHAR)` would rewrite the user's query result and change `db.schema` types and column order of results (DEC-001).

## 3. Architecture
One module. `batches_to_json` (`db.rs:139`) is the single row builder for `db.query` (`db.rs:329`), `db.sample` (`db.rs:531`) and the NDJSON and JSON writer (`db.rs:255-257`). It gains a per batch step: for each column, `text_formatter(col)` returns an `ArrayFormatter` when the column type is covered, `None` otherwise. For each cell: null gives JSON null; a covered column gives `Value::String(formatter.value(row).to_string())`; any other column goes through `arrow_value_to_json` as today. `db.profile` min and max call `arrow_value_to_json` on already textual columns (`db.rs:471-472`) and are unaffected. The CSV writer (`db.rs:225-232`) is untouched.

## 4. Data model
None.

## 5. Contracts and interfaces
No tool, parameter or schema change (`tool-contract-golden.json` untouched). Output texts as in the spec. Private additions in `crates/core/src/tools/db.rs`:
- `fn text_formatter(col: &dyn Array) -> Option<ArrayFormatter<'_>>`: `Some` for `Date32`, `Date64`, `Timestamp(_, _)`, `Decimal128`, `Decimal256`, `Binary`, `LargeBinary`, `FixedSizeBinary`, `BinaryView`; `None` otherwise (also when `ArrayFormatter::try_new` fails, which falls back to today's arm).
- Test only: `#[cfg(test)] pub(crate) fn t1_parquet() -> Vec<u8>` building fixture T1 of the spec (Section 9) with the arrow writer.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/db.rs` | modify: `text_formatter`, `batches_to_json`, remove the `Date32`, `Date64`, `Timestamp` arms and their unused imports, fixture helpers, tests | FR-NEW-001 to FR-NEW-007 | US-0108, US-0109 |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify: E2E-001 and E2E-006 through the MCP route | FR-NEW-001, FR-NEW-004 | US-0108, US-0109 |
| `.agent_docs/tools.md` | modify if it describes db value formats (checked by the story) | documentation | US-0109 |

Affected existing tests: none asserts these types (spec Section 2.3); `spec_0055_dt_*` and `bt_0057_*` keep their assertions. Documentation: the `db.query` row in `.agent_docs/tools.md` if it exists; `README.md` and `AGENTS.md` need no change.

## 7. Compatibility and risks
Consumers of the optional `db.*` family (off by default) receive text where they received day counts, nanosecond digit strings, `?` or `<type>`. The break is declared by the spec (FR-MOD-001). Rollback: revert the commits; files written by `db.convert` to NDJSON while the change shipped hold readable text and stay readable. Risk: the formatter's text for a type is not what the CSV shows in some edge: covered by parity assertions. Ordering: US-0109 depends on US-0108 (the formatter plumbing); US-0108 leaves decimals and binary on today's output, so main stays green and consistent between them. No dependency added.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | MCP `tools/call` over the app router | `tool_surface_e2e.rs` (`bt_0058_e2e_001_*`) | US-0108 |
| E2E-002, 005, 009, 010, 012, 013, 015 | integration | registered tools via `db_harness` | `db.rs` tests (`bt_0058_e2e_NNN_*`) | US-0108 |
| E2E-006 | functional e2e | MCP `tools/call` then `fs.read_bytes` | `tool_surface_e2e.rs` | US-0109 |
| E2E-003, 004, 007, 008, 011, 014 | integration | registered tools via `db_harness` | `db.rs` tests | US-0109 |

Test data is generated: Parquet files written with the arrow writer in `#[cfg(test)]` helpers, CSV and NDJSON seeded as text. Tests that must be RED before the fix assert the readable text where `?`, day counts, nanosecond digits or placeholders come back today; E2E-012, 013, 014 and 015 protect behavior that already holds (kept placeholders, plain columns, CSV bytes, type names) and may pass at once: stated in the stories.

## 10. Decisions log
- **DEC-001:** Render the covered types with `ArrayFormatter` built from `FormatOptions::default()`, the formatter and options the CSV writer uses. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Rationale:** the spec defines the text as the CSV cell text; sharing the formatter makes parity structural. **Alternatives considered:** hand written renderers with `chrono` (parity risk, more code); SQL `CAST` to text (rewrites results and types). **Source:** existing code. **Evidence:** `arrow-csv-60.0.0/src/writer.rs:258-276`, `arrow-cast-60.0.0/src/display.rs:562-565` (binary, large and fixed width binary, binary view), `:753` (decimals), `:790-791` (timestamps).
- **DEC-002:** Build one formatter per covered column per batch inside `batches_to_json`, not one per cell. **Obliged by:** SPEC-0058/NFR 7.1 (no more than 2 times the old time). **Rationale:** a timestamp formatter parses its time zone when built; per cell construction would multiply that cost by the row count. **Alternatives considered:** a formatter per cell (simpler signature, quadratic in tz parsing); a cache in a static (global state). **Source:** design. **Evidence:** `db.rs:139-166` (the per cell loop), `display.rs:790` (tz parse in `try_new`).
- **DEC-003:** The covered set is exactly `Date32`, `Date64`, `Timestamp`, `Decimal128`, `Decimal256`, `Binary`, `LargeBinary`, `FixedSizeBinary`, `BinaryView`; every other type keeps its present arm, including `Time32`, `Time64`, `Duration`, `Interval` and nested types, and a formatter that cannot be built falls back to that arm. **Obliged by:** SPEC-0058/FR-NEW-006, FR-NEW-007, spec Section 3.2 and 14.3. **Rationale:** nothing else is specified. **Alternatives considered:** formatting every type (changes outputs the spec freezes, and nested types have no CSV text). **Source:** spec constraint. **Evidence:** `db.rs:168-221`.
- **DEC-004:** Remove the `Date32`, `Date64` and `Timestamp` arms of `arrow_value_to_json` once the formatter covers them, with their now unused imports; the default arm stays. **Obliged by:** SPEC-0058/FR-MOD-001 and clean code. **Rationale:** those arms become unreachable from `batches_to_json`; `db.profile` only passes text columns. **Alternatives considered:** keeping them as dead code (flagged as lean violation). **Source:** existing code. **Evidence:** `db.rs:206-219`, `db.rs:471-472`.
- **DEC-005:** Two stories: US-0108 plumbs the formatter and covers dates and timestamps (with nested and plain regression); US-0109 adds decimals and binary, closes the spec. **Obliged by:** SPEC-0058/FR-NEW-001 to FR-NEW-007. **Rationale:** each leaves main green and independently testable; the second is a one line change to the covered set plus its tests, so it carries the closing audit. **Alternatives considered:** one story of 15 tests (T2 budget is 5 to 15 and 3 to 5 files; kept whole it would pass but mixes two behaviors); one story per type (a thinner cut with nothing to assert alone). **Source:** design. **Evidence:** budget table, T2.
- **DEC-006:** E2E-001 and E2E-006 run through the MCP route, the other E2E tests through `db_harness`. **Obliged by:** SPEC-0058/FR-NEW-001, FR-NEW-004. **Rationale:** the MCP route proves the served surface once per behavior; the registered tool handlers are the same code, as in SPEC-0057. **Alternatives considered:** all 15 through MCP (slow, no extra coverage). **Source:** existing code. **Evidence:** `tool_surface_e2e.rs:374`, SPEC-0057 design section 9.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0108 | return dates and timestamps as readable text | P1 | FR-NEW-001, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007 | main | 2 | no |
| US-0109 | return decimals and binary values as readable text and close the spec | P1 | FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-007, FR-MOD-001 | US-0108 | 2 | yes |
