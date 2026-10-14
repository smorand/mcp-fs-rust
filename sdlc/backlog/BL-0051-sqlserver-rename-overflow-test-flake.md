---
Id: BL-0051
Title: A live SQL Server path ceiling test failed once under the parallel storage suite
Nature: DEBT
Source: US-0029 gates, 2026-10-14
Created: 2026-10-14
---

## What
`storage::meta::tests::a_rename_that_would_overflow_a_descendant_is_refused_live`
(`crates/core/src/storage/meta.rs:2211`) failed once in 7 runs of
`cargo test -p mcp-fs-core --all-features --lib -- pg_open_e2e storage conformance` with both
`MCPFS_TEST_PG_DSN` and `MCPFS_TEST_MSSQL_DSN` set. It passed alone right after and in the 6 other
runs. The panic message was not captured. SQL Server code was not touched by US-0029.
Likely cause: the test opens a store on the shared `master` database with a fixed volume id
(`"p".repeat(MAX_VOLUME_ID_CHARS)`) and fixed paths `/src`, `/dst`, so another live test in the same
run can see or change its rows.

## Proposal
Give the test a volume id unique per run (process id suffix within the length ceiling), and capture
the failure text next time it appears.
