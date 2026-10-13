---
Id: BL-0046
Title: SQL Server schema setup fails on the trash_entries deleted_at index
Nature: BUG
Source: US-0023 (SPEC-0022), 2026-10-12
Created: 2026-10-12
---

## What
On SQL Server the conformance suite fails before any case runs:
`sqlserver: Column 'deleted_at' in table 'trash_entries' is of a type that is invalid for use as a key column in an index. [1919]`.
Main fails the same way (verified with the US-0023 changes stashed). Every SQL Server meta store
therefore fails to migrate.

## Evidence
- `crates/core/src/storage/meta.rs:207` `Column::required("deleted_at", ColumnType::Text)` (NVARCHAR(MAX) on SQL Server).
- `crates/core/src/storage/meta.rs:305-308` index `idx_trash_entries_deleted_at` on `(volume_id, deleted_at)`.
- AGENTS.md: SQL Server cannot index `NVARCHAR(MAX)`, keyed text uses `TextKey(n)`.
- Run: `MCPFS_TEST_MSSQL_DSN=... cargo test -p mcp-fs-core --all-features --lib sqlserver_passes_the_conformance_suite`.

## Proposal
Type `deleted_at` as a bounded key (`TextKey`) or an integer timestamp, with a column migration; add it to the SQL Server conformance run.
