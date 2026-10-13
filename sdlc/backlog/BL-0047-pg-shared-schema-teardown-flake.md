---
Id: BL-0047
Title: PostgreSQL shared schema teardown test flakes under the full opt in suite
Nature: DEBT
Source: US-0023 review (SPEC-0022), 2026-10-12
Created: 2026-10-12
---

## What
With `MCPFS_TEST_PG_DSN` set, the full `cargo test -p mcp-fs-core --all-features --lib` run failed once:
`storage::tests::teardown_on_a_shared_database_drops_the_usage_row --- FAILED` (panic at
`crates/core/src/storage/mod.rs:623:46`); it passes when run alone. It uses the shared `public`
schema while other PostgreSQL tests run concurrently. Not verified whether `main` flakes the same way.
Related: `crates/core/src/tools/pg_null_e2e.rs:50` schema names carry the pid, so a panicking test
leaves its schema behind.

## Proposal
Give the teardown test its own schema, and drop leftover `mcpfs_pgnull_*` schemas at fixture start.
