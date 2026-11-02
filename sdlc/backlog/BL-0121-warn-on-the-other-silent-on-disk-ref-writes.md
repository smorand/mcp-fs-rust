---
Id: BL-0121
Title: Route the other silent on disk ref writes through sync_disk_ref
Nature: DEBT
Source: deferred from SPEC-0071 (DEC-002)
Created: 2026-11-02
---

## What
Ten `let _ = repo.reference(...)` calls in `crates/core/src/tools/git.rs` still drop a failed on disk ref write: lines 2333, 2840, 4200, 4274, 4907, 4941, 5645, 6034, 6952, 6986. Route them through `sync_disk_ref` (SPEC-0071) so each failure is logged.

## Why
SPEC-0071 fixed the three sites of BL-0095 and BL-0105 only. The rest have the same cost: an index and a disk that disagree with no trace.

## Evidence
`rg 'let _ = repo\.reference\(' crates/core/src/tools/git.rs` (recounted 2026-11-02: 11 hits, 1 of them `:2504` fixed by SPEC-0071).

## Notes
Recount before specifying: line numbers move. Some sites (replay abort `:6034`, rebase pause `:4941`) write refs outside the heads namespace.
