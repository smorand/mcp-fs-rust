---
Id: BL-0049
Title: PostgreSQL migrate deadlocks when several stores open the same schema concurrently
Nature: BUG
Source: US-0025 implementation (SPEC-0024), 2026-10-13
Created: 2026-10-13
---

## What
`PostgresRelationalDb::migrate` (`crates/core/src/storage/rel/postgres.rs`) runs
`ALTER TABLE "nodes" ADD COLUMN IF NOT EXISTS "rev"` on every open. Even when the column exists,
the statement takes an ACCESS EXCLUSIVE lock, so two opens of the same schema running at once
(each also holding locks from its `CREATE TABLE IF NOT EXISTS` statements) can deadlock.

Evidence, on `main` at `8fa5a9b` (detached worktree), `MCPFS_TEST_PG_DSN` set,
`cargo test -p mcp-fs-core --all-features --lib -- storage conformance`, 4 runs, 2 failed:
```
test storage::tests::one_dsn_yields_one_pool_across_every_consumer ... FAILED
called `Result::unwrap()` on an `Err` value: ToolError { code: "ERR_INTERNAL_ERROR", message: "postgres: deadlock detected [40P01]", retryable: true }
```
On the US-0025 branch the same suite also failed once each in
`storage::conformance::postgres_passes_the_conformance_suite` and
`storage::tests::teardown_on_a_shared_database_drops_the_usage_row` (both on the shared `public`
schema); with `--test-threads=1` it is green. Production impact: two server processes, or two
first requests on different volumes, opening the shared schema at once can fail one open with
`ERR_INTERNAL_ERROR` (retryable, but `migrate` is not retried).
Likely the same root cause as BL-0047.

## Proposal
Skip the `ALTER` when `information_schema.columns` already has the column, and serialize
`migrate` with `pg_advisory_xact_lock` on a per schema key. Give the `public` schema tests in
`storage/mod.rs` and `storage/conformance.rs` their own schema.
