---
Id: BL-0126
Title: cargo audit reports two new advisories and the security gate test is red
Nature: DEBT
Source: found while running US-0160 (make check on a branch that changes no dependency)
Created: 2026-11-05
---

## What
`cargo test --workspace` fails one test, `cargo_audit_reports_no_vulnerability_and_no_warning` (`crates/mcp-fs/tests/security_gate.rs:119`), because the advisory database now reports two warnings for crates already in `Cargo.lock`.

## Why
Every story's `make check` ends red on this one test whatever the story changes. `Cargo.lock` is unchanged by the stories that hit it, so the cause is the advisory database, not the code.

## Evidence
`cargo audit` on 2026-11-05: `pin-utils` 0.1.0 `unmaintained` RUSTSEC-2026-0441 (upstream archived 2026-10-01; replace `pin_mut!` with `core::pin::pin!`), and `arrow-data` 60.0.0 `unsound` RUSTSEC-2026-0442 (patched `>=60.1.0`, date 2026-11-02, via the pinned datafusion git revision, see BL-0089). `cargo deny check` passes. Full run: `cargo test --workspace --no-fail-fast` gives passed=3544 failed=1 with `MCPFS_TEST_PG_DSN` unset.

## Notes
Proposal: bump `arrow-data` through the datafusion pin (BL-0089) and find who pulls `pin-utils`, or add the two ids to the audit ignore list with a dated reason if no fix exists. Also note: this shell exports `MCPFS_TEST_PG_DSN=...:5432/postgres` pointing at no running server, which makes 19 PostgreSQL tests fail with `timed out waiting for a pooled connection` instead of skipping; unset it (or use the `docker-compose.test.yml` instance) when running the suite.
