---
Id: BL-0122
Title: make check is red on RUSTSEC-2026-0429 (zip 3.0.0 build dependency)
Nature: DEBT
Source: found by /sdlc-implement while running US-0147
Created: 2026-11-02
---

## What
`cargo audit` now reports RUSTSEC-2026-0429 (zip 3.0.0, `ZipArchive::extract` aborts the process on a symlink entry with a large declared size, severity 7.5, fix in zip 9). `zip` 3.0.0 is a build dependency of `utoipa-swagger-ui` 9.0.2, the same copy as the accepted RUSTSEC-2026-0367, 0368 and 0369. `make check` fails at the `security` step, and three tests that run the audit themselves fail with it: `cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`.

## Why
Main cannot be green until the advisory is either accepted (add the id to `.cargo/audit.toml` and `deny.toml` with the BL-0059 reason, and to the id lists in `crates/mcp-fs/tests/security_gate.rs` at `:158-163` and `:225`) or removed (BL-0059: stop building `utoipa-swagger-ui` against zip 3.0.0).

## Evidence
`cargo audit` output of 2026-11-02 (`Crate: zip, Version: 3.0.0, ID: RUSTSEC-2026-0429`); `cargo tree -i zip@3.0.0` shows `utoipa-swagger-ui v9.0.2` as the only parent; `.cargo/audit.toml:8-10`, `deny.toml:21-23`.

## Notes
The stories of the 2026-11-02 run were gated with `cargo audit --ignore RUSTSEC-2026-0429` and with those three tests as the only accepted failures (script `.implement/gate.sh`, untracked). Accepting it edits an existing security gate test, so it needs its own spec or plan, not a story of another change.
