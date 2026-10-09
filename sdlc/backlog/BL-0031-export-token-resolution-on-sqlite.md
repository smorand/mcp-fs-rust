---
Id: BL-0031
Title: O(P) per-project probe to resolve an export token under the SQLite backend
Nature: DEBT
Source: retro-spec 2026-10-09, SPEC-0015 (zip-export-signed-url) split
Created: 2026-10-09
Severity: Low
Fingerprint: scaling:crates/core/src/exports.rs:claim-probes-every-project
---

## What
`GET /exports/{token}` resolves which project's database holds the export record by calling
`admin.list_all_projects()` and probing each project's SQLite database in turn until one
succeeds, because under SQLite each project's relational metadata lives in its own database
file with no single connection seeing every volume's `export_links` rows at once.

## Why
Correct (every volume is tried before concluding 404, no false negative) but O(P) connection
attempts per download request, where P is the project count. Under PostgreSQL/SQL Server (one
shared database) the same loop costs O(P) redundant queries against one connection, not O(P)
separate connections; SQLite is the affected backend. Not a functional gap, a scaling
characteristic that gets worse as project count grows.

## Evidence
`crates/core/src/exports.rs` (`claim()`), `crates/core/src/storage/mod.rs` (`StoreManager::client`
per-volume database resolution).

## Notes
Candidate fixes: a lightweight global `token -> volume_id` index (its own tiny table, or a
shared/default-backend connection even under SQLite) for O(1) resolution; or encode a short
volume hint in the token/URL itself (changes the URL shape, needs a new decision, not a silent
change). Not resolved in the original implementation; shipped with full test suite green.
