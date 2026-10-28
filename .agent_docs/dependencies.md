# Pinned dependencies

## datafusion from a git revision (SPEC-0055, 2026-10-27)
- `Cargo.toml` takes `datafusion` from `https://github.com/apache/datafusion` at revision `6f1436fe1407495019d8afec8a9f55663f418d9a` (main, workspace version 55.1.0), with `arrow` and `parquet` at 60.
- Why: the released `datafusion` 55.2.0 needs arrow 59, whose `arrow-ipc` and `parquet` require `zstd ^0.13`; that zstd family (zstd 0.13.3, zstd-safe 7.x, zstd-sys 2.0.x) carries 11 RUSTSEC advisories, three of them vulnerabilities. The git revision uses arrow 60 and zstd 0.14.3 (zstd-safe 8.2.0, zstd-sys 2.1.2). Only the optional `db` tool family and the search index touch it.
- Companion bump: `tantivy` 0.27. tantivy 0.26 needs zstd 0.13 through `tantivy-sstable`, and tantivy 0.27 cannot resolve while parquet is 53 (parquet 53 pins `zstd-sys` below 2.0.14); so the four packages moved together.
- Gate: `deny.toml` `[sources]` allows exactly this one git URL (`allow-git`), `unknown-git` stays `deny`. `cargo audit` and `cargo deny check` pass with no zstd ignore.
- Exit condition: BL-0089. When `cargo search datafusion` shows a release above 55.2.0 whose `arrow` requirement is 60 or more, depend on it from crates.io and delete the `allow-git` entry and this section.

## Behaviour kept on purpose
- `crates/core/src/tools/db.rs` sets `sql_parser.map_string_types_to_utf8view = false` and strips the ` at Line: N, Column: N` suffix datafusion's SQL parser added to its error text, so the `db.*` tool outputs stay byte identical to datafusion 43 (characterization tests DT-001 to DT-011). A Parquet string column (read as `Utf8View`) is rendered as its text and reported as `Utf8`, which fixed BL-0090 (SPEC-0057, 2026-10-28); other view types such as `BinaryView` still print the placeholder (BL-0091).
- Search: the tantivy 0.26 index fixture (`crates/core/src/search/fixtures/tantivy_0_26/`) still opens and answers under 0.27. tantivy 0.27 swapped its stemmer crate; BL-0092 tracks a comparison on real text.
