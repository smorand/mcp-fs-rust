---
Id: BL-0017
Title: Two horizontal-scaling strategies, and the subsystem breakdown for the stateless one
Nature: FEAT
Source: specs/BACKLOG.md BL-015 (migrated), user request 2026-09-21
Created: 2026-09-21
---

## What
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

## Why
Recorded so the strategy choice (A vs B) and, if B, the subsystem-by-subsystem plan are available
when the need becomes concrete, rather than re-litigated then.

## Evidence
Comparison performed against IBM Storage Scale, IBM Content-Aware Storage and JuiceFS as possible
backends; neither offers a data-plane API matching `mcp-fs`'s needs (read/write/edit/patch/glob/
grep), both expose management-plane (and, for IBM, vector-search-plane) APIs only, and both
enforce access through OS-level identity (POSIX UID/GID, in practice AD/LDAP-backed), which does
not map cleanly onto `mcp-fs`'s project membership table.

## Notes
Same deferral reason as [[BL-0016]]: no current deployment needs more than one replica.
