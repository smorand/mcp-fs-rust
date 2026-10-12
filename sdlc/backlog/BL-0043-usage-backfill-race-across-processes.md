---
Id: BL-0043
Title: Usage counter first backfill can lose a concurrent write across processes
Nature: DEBT
Source: US-0015 (SPEC-0020) review, 2026-10-11
Created: 2026-10-11
---

## What
On PostgreSQL or SQL Server shared by several processes, the first time backfill of `volume_usage`
(`crates/core/src/storage/meta.rs:459`, `:1626`) runs at READ COMMITTED; a write from another process
during the sum updates no counter row and is not summed. Once per volume, on upgrade. Also
`migrate.rs:258` does not copy `volume_usage`.

## Proposal
Lock the row while summing or resum after insert; copy or rebuild usage in `migrate`.
