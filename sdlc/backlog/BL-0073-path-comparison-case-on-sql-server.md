---
Id: BL-0073
Title: Unverified: do paths differing only by case coexist on SQL Server
Nature: unknown
Source: analysis SPEC-0050
Created: 2026-10-23
---

## What
SPEC-0050 states that `/Video` and `/video` are two different paths. On SQLite they are two rows (primary key
`(volume_id, path)`, `crates/core/src/storage/meta.rs:234`). SQL Server compares text with the database
collation, and the usual default is case insensitive: the two paths may then be the same key, so a project
could not hold both, and `=` and `LIKE` would both ignore case.

## Evidence
No `COLLATE` and no case related setting anywhere under `crates/core/src/storage`
(`rg -n 'case_sensitive_like|COLLATE' crates/core/src/storage` prints nothing). The SQL Server schema renders
the path column as a bounded text key (`crates/core/src/storage/rel/dialect.rs`, `ColumnType::TextKey`). Not
confirmed: what collation the supported SQL Server deployments use, and whether the opt in SQL Server suite
(`docker-compose.test.yml`) ever stores two paths differing only by case.

## Notes
Needs a run against SQL Server (`MCPFS_TEST_MSSQL_DSN`) with `/Video/x` and `/video/y`. PostgreSQL's `LIKE` is
case sensitive by documentation; also unconfirmed by a run.
