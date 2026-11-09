---
Id: BL-0138
Title: Four new RUSTSEC advisories (rmcp 3.5.0 x3, libsqlite3-sys 0.35.0) turn the security gate red
Nature: DEBT
Source: found while running US-0183 (advisory database update of 2026-11-09)
Created: 2026-11-09
---

## What
`cargo audit` now reports 4 vulnerabilities: RUSTSEC-2026-0457 (rmcp OAuth client SSRF filter), RUSTSEC-2026-0461 (rmcp unbounded tool schema cache in the Streamable HTTP server transport), RUSTSEC-2026-0462 (rmcp unbounded response body buffering in Streamable HTTP clients), RUSTSEC-2026-0459 (libsqlite3-sys 0.35.0, bundled C SQLite CVE-2026-119375). `cargo deny check` fails on advisories. Three security_gate tests are red on main: `cargo_audit_reports_no_vulnerability_and_no_warning`, `deny_toml_drops_the_fixed_git2_ignores`, `make_security_passes`.

## Why
`make check` is red on every story whatever it changes, and `./test.sh` stops at the first failing test binary, so the core library tests do not run at all (use `cargo test --workspace --no-fail-fast`).

## Evidence
`cargo audit` and `cargo deny check` on 2026-11-09: "Solution: Upgrade to >=3.9.0 (try `cargo update -p rmcp`)" for the three rmcp ones. The server uses rmcp 3.5.0 (`Cargo.toml`, `crates/core/src/mcp/server.rs`, `app.rs`); the Streamable HTTP server transport is the one in production (SPEC-0013), so RUSTSEC-2026-0461 applies to this server.

## Notes
Proposal: try `cargo update -p rmcp` (3.9 may break the `#[tool]` macros or the session manager: run the tool contract golden test), bump rusqlite or `libsqlite3-sys` to a release with the fixed SQLite, and only if no fix exists accept the id in `.cargo/audit.toml` and `deny.toml` with a dated reason and update the accepted advisories test.
