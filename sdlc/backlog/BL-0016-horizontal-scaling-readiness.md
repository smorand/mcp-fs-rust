---
Id: BL-0016
Title: Horizontal scaling readiness (multiple server replicas)
Nature: FEAT
Source: specs/BACKLOG.md BL-014 (migrated), user request 2026-09-21
Created: 2026-09-21
---

## What
The full set of coordination gaps that block running more than one `mcp-fs` replica: shared
session/quota state ([[BL-0015]]), a shared git write lock, and a relational backend suited to
multi-writer access.

## Why
Nothing in the codebase currently assumes more than one replica; running more than one today
risks quota bypass and repository corruption (see Evidence).

## Evidence
The per-project git write lock that serializes commits, pushes and pulls is an in-memory
`tokio::sync::Mutex` per process (`crates/mcp-fs/src/git/repo.rs:44` at the time of writing), so
two replicas can each acquire their own lock for the same repository and interleave writes to it,
corrupting its history. SQLite, the default relational backend, is a single file with one writer;
PostgreSQL and SQL Server already support concurrent writers.

- (Folded from BL-0043, 2026-10-15.) The project usage counter (SPEC-0020) can lose a write across processes: a write that finds no `volume_usage` row skips the counter update (`crates/core/src/storage/meta.rs:1199-1212`), and the first open recount (`:1653-1686`) runs at READ COMMITTED on PostgreSQL and SQL Server, so a concurrent uncommitted write is neither summed nor counted. Unreachable with one server process, because every store recounts in `RelationalMetaStore::open` before its first write (`:474-480`); it becomes real with two replicas, or during a rolling upgrade from a pre SPEC-0020 binary that never updates the counter. Not a defect: `migrate` not copying `volume_usage` (offline, and the counter is rebuilt on the next open).

## Notes
Risk analysis: with more than one replica and no fix, a caller's write quota is effectively
multiplied by the replica count ([[BL-0015]]); two replicas racing the git write lock can corrupt
a repository's history; a SQLite deployment cannot scale past one writer regardless of any fix
here. Deferred because no current deployment runs more than one replica. See [[BL-0017]] for the
strategy comparison.
