---
Id: BL-0079
Title: RUSTSEC-2026-0387 on crypto-bigint 0.5.5 turns the security gate red
Nature: DEBT
Source: found while implementing US-0083 (SPEC-0052)
Created: 2026-10-24
---

## What
`cargo audit` reports RUSTSEC-2026-0387 (published 2026-10-23, timing variability in constant-time operations) for `crypto-bigint 0.5.5`, patched in `>=0.7.6`. `cargo deny check` also warns `license-not-encountered`. Three tests of `crates/mcp-fs/tests/security_gate.rs` fail on main: `cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`. `make check` is therefore red on main independently of any story.

## Why
The project gate (`make check`, includes `make security`) cannot pass until the advisory is resolved or consciously ignored.

## Evidence
- `cargo tree -i crypto-bigint@0.5.5`: pulled by `aws-sigv4 1.6.0` via `aws-runtime`, `aws-config 1.8.13`, `mcp-fs-core`.
- `Cargo.lock`, `Cargo.toml`, `deny.toml` are unchanged by US-0083 (`git diff main --stat` empty), so the failure is the same on main.
- Full run: `.implement/US-0083-verify.log`, `test result: FAILED. 11 passed; 3 failed` for `security_gate`.

## Notes
Options: upgrade the AWS crates once a release pulls `crypto-bigint >=0.7.6`, or record a justified ignore in `deny.toml`/audit config with an expiry. Not decided here.
