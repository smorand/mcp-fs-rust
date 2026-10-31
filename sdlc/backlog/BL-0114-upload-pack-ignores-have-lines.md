---
Id: BL-0114
Title: The git upload-pack ignores `have` lines and rebuilds the full pack on every request
Nature: DEBT
Source: deferred from SPEC-0066 (found while measuring BL-0113)
Created: 2026-10-30
---

## What
`handle_upload_pack` (`crates/core/src/git/http/mod.rs:421-452`) answers every `git clone` and `git fetch` with `NAK` and a pack of everything reachable from the wanted tips (`build_pack`, `:460`). `parse_upload_pack_request` reads the client's `have` lines (`:411`) but nothing uses them. A client that already holds the history downloads and indexes it again.

## Why
Measured on a project with N reachable objects (debug build, 5 requests each): 24 to 26 ms at 500, 244 to 311 ms at 5 000, 1 590 to 1 806 ms at 20 000. The hydrate share is only 8 to 10% of that (124 to 172 ms at 20 000, fixed by SPEC-0066); the rest is the pack walk and write, O(reachable objects), plus the bytes on the wire. A negotiation (ACK common commits, hide them from the walk with `hide`) would make an incremental fetch O(new objects). It changes the protocol exchange (`ACK` lines, multi round negotiation), so it needs its own design and compatibility tests with a real `git` client.

## Evidence
`crates/core/src/git/http/mod.rs:421-452,460-490`; the measurement above in SPEC-0066 Section 2.
