# SPEC-0057: db tools return `<Utf8View>` for Parquet string columns, design

> Spec: sdlc/specs/SPEC-0057-db-parquet-strings-as-text/sdlc-spec.md
> Nature: BUG
> Depth: S
> Security: n/a
> Designed on: 2026-10-28
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, runtime | Rust 2024, tokio (unchanged) | `Cargo.toml` | n/a |
| Analytics | `datafusion` git rev `6f1436f` (55.1.0), `arrow` 60 (unchanged, no new dependency) | `Cargo.toml:126-128` | DEC-001 |

Skills loaded: `rust` (the `make check` gate, `#[cfg(test)]` unit tests, MCP e2e through the real route, no `unwrap` on fallible paths). Gate commands of this repo: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` (`AGENTS.md`, Quality gate).

## 2. Approaches
### Chosen: render `Utf8View` on output, at the root cause site
Add the missing `Utf8View` arm to `arrow_value_to_json` (`db.rs:158-207`) and print the type name of a `Utf8View` field as `Utf8` in `db.schema` (`db.rs:360`) and `db.profile` (`db.rs:466`) through one helper. Pros: touches only the serializer and the two printed names the spec's root cause names; every other Parquet output (binary, decimal) stays byte for byte as today, which BR-005 requires. Cons: the engine still reads views internally, so the printed `Utf8` is a label, not the physical type. Scope: `db.rs`.
### Rejected: stop the Parquet reader from producing views
Set `execution.parquet.schema_force_view_types = false` in `session()` (`db.rs:105-110`). One line, and a throwaway run on HEAD confirmed it fixes BR-001 to BR-004 (NDJSON, query, schema `Utf8`, profile `Utf8`). Rejected by DEC-001: the option also turns `BinaryView` into `Binary`, so the placeholder a Parquet binary column prints (`<BinaryView>` today) and its schema type change, which BR-005 forbids and BL-0091 owns.

## 3. Architecture
One module, no new component. `arrow_value_to_json` is the single row serializer: `db.query` (`db.rs:315`), `db.sample` (`db.rs:517`), the `db.profile` min and max (`db.rs:457-458`) and the NDJSON and JSON writer `batches_to_ndjson` (`db.rs:241-243`, dispatched at `db.rs:574`) all call it, so one arm closes BR-001 and BR-002. Type names are printed at exactly two places, `db.rs:360` and `db.rs:466`.

## 4. Data model
None.

## 5. Contracts and interfaces
No tool, parameter or schema change (`tool-contract-golden.json` untouched). Output values change as the spec says: `Utf8View` strings are JSON strings, `data_type` and `type` print `Utf8`. Internal signature added in `crates/core/src/tools/db.rs`: `fn type_name(t: &DataType) -> String`, returning `"Utf8"` for `DataType::Utf8View` and `t.to_string()` otherwise.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/db.rs` | modify: `Utf8View` arm, `type_name` helper used at the two print sites, comment at `:101-104`, four existing tests modified, new tests BT-010 to BT-013 | BR-001 to BR-004 | US-0107 |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify: BT-001, BT-002 through the MCP route | BR-001, BR-002 | US-0107 |
| `.agent_docs/dependencies.md` | modify: line 11 no longer says the placeholder stays | documentation | US-0107 |

Affected existing tests (all in `db.rs`): `spec_0055_dt_002_query_parquet`, `spec_0055_dt_004_schema_type_names`, `spec_0055_dt_006_profile_parquet`, `spec_0055_dt_040_zstd_parquet_is_read` modified (expected values); `spec_0055_dt_041_corrupt_zstd_page_is_rejected` modified through `id_name_rows()`; every other `db.rs` test kept. `AGENTS.md` and `README.md` need no change (no structure or command change).

## 7. Compatibility and risks
Consumers of the optional `db.*` family (off by default) now receive real strings where they received `<Utf8View>`. Files written by `db.convert` after the fix hold real text; files written before keep the placeholder (not repaired). Rollback: revert the commit. No dependency added or removed. Risk: another path may yield `Utf8View` that the arm covers by design (computed strings); `BinaryView` stays a placeholder (BL-0091).

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | MCP `tools/call` over the app router | `crates/core/src/tools/tool_surface_e2e.rs` (`bt_0057_001_*`) | US-0107 |
| BT-002 | functional e2e | MCP `tools/call`, then `fs.read_bytes` | `tool_surface_e2e.rs` (`bt_0057_002_*`) | US-0107 |
| BT-010 | integration | registered tools via `db_harness` | `db.rs` tests (`bt_0057_010_*`) | US-0107 |
| BT-011 | integration | same | `db.rs` (`bt_0057_011_*`) | US-0107 |
| BT-012 | integration | same, Parquet from the arrow writer | `db.rs` (`bt_0057_012_*`) | US-0107 |
| BT-013 | integration | same | `db.rs` (`bt_0057_013_*`) | US-0107 |
| BT-020 | integration | the existing DT tests, unchanged | `db.rs` | US-0107 |

BT-001 and BT-002 are RED on HEAD: the payload carries `"<Utf8View>"` where `Alice` is expected. BT-010 to BT-013 are RED for the same cause. Test data is generated (CSV text seeded, Parquet produced by `db.convert` or the arrow writer); none is user provided.

## 10. Decisions log
- **DEC-001:** Fix on the output side: a `Utf8View` arm in `arrow_value_to_json` plus a `type_name` helper at the two print sites; do not change how the Parquet reader types columns. **Obliged by:** SPEC-0057/BR-001, BR-002, BR-003, BR-004, BR-005. **Rationale:** the spec's root cause is the missing arm and the unmapped printed type (`db.rs:206`, `:360`, `:466`); turning view types off also changes binary output, which BR-005 freezes. **Alternatives considered:** `schema_force_view_types = false` in `session()` (works for strings, changes `BinaryView` output, rejected); a SQL cast to `VARCHAR` of every Parquet column (rewrites user queries, rejected). **Source:** existing code. **Evidence:** `db.rs:105-110` (session pins), `db.rs:158-207`, datafusion `datafusion/common/src/config.rs:1375` at rev `6f1436f`.
- **DEC-002:** Keep the placeholder arm and the `Timestamp` arm untouched; add nothing for binary, decimal, list or non nanosecond timestamps. **Obliged by:** SPEC-0057/BR-005. **Rationale:** those belong to BL-0091. **Alternatives considered:** fixing them here (scope creep). **Source:** spec. **Evidence:** `db.rs:198-206`.
- **DEC-003:** One story, closing the spec. **Obliged by:** SPEC-0057/BR-001 to BR-005. **Rationale:** 3 files, one behavior; a thinner cut (serializer, then type names) would leave main with a half fixed Parquet path and no end to end test. **Alternatives considered:** two stories. **Source:** design. **Evidence:** budget T2, 3 to 5 files.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0107 | return Parquet strings as text from the db tools | P1 | BR-001, BR-002, BR-003, BR-004, BR-005 | main | 2 | yes |
