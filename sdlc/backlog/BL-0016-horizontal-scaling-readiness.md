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
here. Deferred because no current deployment runs more than one replica. 

## Scaling strategies (merged from BL-0017, 2026-10-19)
A comparison of two ways to run more than one `mcp-fs` replica, surfaced while evaluating whether
a shared parallel filesystem (IBM Storage Scale, JuiceFS) could sit under `mcp-fs` as a backend,
plus the full list of per-subsystem work the fully-stateless strategy requires.

**Strategy A: shard by project, not fully stateless.** `mcp-fs` already isolates every project's
state: its own relational data, its own bare git repository on disk, its own blob bucket, its own
search index. A routing layer in front of N independent replicas, keyed by `project_id`
(consistent hashing or an explicit registry), gets multi-tenant capacity without touching any
subsystem. The limit is that one project's throughput stays bounded by one replica. Cheaper, needs
no new infrastructure class.

**Strategy B: any replica can serve any project.** Needs four separate fixes: a distributed
relational backend (or accept PostgreSQL/SQL Server single-primary as the bottleneck until
proven otherwise), git bare repositories on a filesystem shared across replicas (or keep
project-affinity routing scoped to git only), a distributed search/vector backend (or per-shard
indexes under Strategy A), and moving in-memory OAuth tokens and the session/write-quota map
([[BL-0015]]) to a shared store (Redis or the relational backend).

Backend evaluation: Comparison performed against IBM Storage Scale, IBM Content-Aware Storage and JuiceFS as possible
backends; neither offers a data-plane API matching `mcp-fs`'s needs (read/write/edit/patch/glob/
grep), both expose management-plane (and, for IBM, vector-search-plane) APIs only, and both
enforce access through OS-level identity (POSIX UID/GID, in practice AD/LDAP-backed), which does
not map cleanly onto `mcp-fs`'s project membership table.
