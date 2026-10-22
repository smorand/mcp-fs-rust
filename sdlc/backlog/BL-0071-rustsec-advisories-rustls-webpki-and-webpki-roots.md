---
Id: BL-0071
Title: Bump rustls-webpki and webpki-roots for RUSTSEC-2026-0382 and RUSTSEC-2026-0381
Nature: DEBT
Source: found during /sdlc-implement US-0064, 2026-10-22
Created: 2026-10-22
---

## What
`cargo audit` reports two vulnerabilities in `Cargo.lock` that fail `make security`, so `make check`
is red on `main` independently of any story:

- `rustls-webpki` 0.103.15, RUSTSEC-2026-0382, solution `>=0.103.16, <0.104.0-alpha.1`.
- `webpki-roots` 1.0.9, RUSTSEC-2026-0381, solution `>=1.0.10`.

## Evidence
`make security` output of 2026-10-22: `error: 2 vulnerabilities found!`. Three tests of
`crates/mcp-fs/tests/security_gate.rs` fail with it: `cargo_audit_reports_no_vulnerability_and_no_warning`,
`deny_toml_drops_the_fixed_git2_ignores`, `make_security_passes`. `Cargo.lock` is unchanged by SPEC-0047.

## Notes
Proposed: `cargo update -p rustls-webpki -p webpki-roots`, then `make check`. A dependency upgrade with no
behavior change.
