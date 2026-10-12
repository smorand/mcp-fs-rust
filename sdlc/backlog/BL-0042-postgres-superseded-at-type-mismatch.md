---
Id: BL-0042
Title: PostgreSQL conformance fails on doc artifact sets superseded_at type
Nature: BUG
Source: US-0015 (SPEC-0020), 2026-10-11
Created: 2026-10-11
---

## What
The PostgreSQL conformance suite fails in the SPEC-0019 doc artifacts case:
`superseded_at is of type bigint but expression is of type text`. Main fails the same way.

## Evidence
- `crates/core/src/storage/conformance.rs` doc artifacts case, run with `MCPFS_TEST_PG_DSN` set.
- The `superseded_at` bind in the artifact set queries of `crates/core/src/storage/meta.rs`.

## Proposal
Bind `superseded_at` as an integer (or NULL) through the dialect; run `make test-e2e-full`.
