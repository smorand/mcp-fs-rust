---
Id: BL-0130
Title: make check compiles the rar feature but never runs its tests
Nature: DEBT
Source: found while running US-0170
Created: 2026-11-05
---

## What
`make lint` runs clippy with `--all-features`, so the opt in `rar` feature compiles on every check, but `make test` runs `cargo test --workspace` with the default features, so the RAR tests (`archive_rar`, `e2e_0080_*` behind `#[cfg(feature = "rar")]`) never run in `make check`.

## Why
A regression in `decode_rar` would pass the gate. The same holds for any future feature gated test.

## Evidence
`Makefile:15-17` (`test: ./test.sh`), `test.sh` (`cargo test --workspace`), `crates/core/Cargo.toml` (`rar = ["dep:unrar"]`, `default = []`). Stories US-0170 onward run `cargo test -p mcp-fs-core --lib --features rar` by hand and say so in their commit bodies.

## Notes
Proposal: add `cargo test -p mcp-fs-core --lib --features rar` to `test.sh`, or switch it to `--all-features` once the PostgreSQL cases skip cleanly without a DSN.
