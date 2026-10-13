---
Id: BL-0048
Title: Volume teardown on a shared database leaves trash and artifact rows behind
Nature: BUG
Source: US-0024 (SPEC-0023), 2026-10-12
Created: 2026-10-12
---

## What
On PostgreSQL or SQL Server (one database for every volume), removing a project's volume deletes only
`nodes`, `blob_refs` and `volume_usage`. Rows of `trash_entries`, `doc_artifact_sets`,
`doc_artifact_current`, `doc_tables`, `doc_images` (and possibly `export_links`) for that `volume_id`
survive, so a project recreated under the same id could list the old project's trash entries.
Not reproduced end to end yet; SQLite deletes the volume file and is not affected.

## Evidence
- `crates/core/src/storage/mod.rs:511` `for table in ["nodes", "blob_refs", "volume_usage"]`.
- Tables with `volume_id`: `crates/core/src/storage/meta.rs:166-318` (`schema()`).
- Observed by the US-0024 implementer, whose SQL Server test had to delete those rows itself.

## Proposal
Derive the teardown list from the metadata schema (every table with `volume_id`), like `git::db::TABLES`;
add a test that recreates a project id and asserts an empty `fs.trash_list`.
