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

## Notes
Risk analysis: with more than one replica and no fix, a caller's write quota is effectively
multiplied by the replica count ([[BL-0015]]); two replicas racing the git write lock can corrupt
a repository's history; a SQLite deployment cannot scale past one writer regardless of any fix
here. Deferred because no current deployment runs more than one replica. See [[BL-0017]] for the
strategy comparison.
