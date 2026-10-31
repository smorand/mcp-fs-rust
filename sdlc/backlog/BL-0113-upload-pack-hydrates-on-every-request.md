---
Id: BL-0113
Title: The git upload-pack path hydrates libgit2 by a full export on every request
Nature: DEBT
Source: deferred from SPEC-0065 (from BL-0111)
Created: 2026-10-30
---

## What
`handle_upload_pack` (`crates/core/src/git/http/mod.rs:434`) calls `entry.objects.export_to_repo(&repo)` before building a pack, on every `git clone` and `git fetch` a client makes. It lists every object row and probes libgit2 for each, the same O(repository objects) cost SPEC-0065 removes from `hydrate` (`tools/git.rs:2069`). Switching it to `hydrate_if_stale` needs no new mechanism.

## Why
It was left out of SPEC-0065 because it is a protocol path and not a tool call. A repeated fetch of a large project pays about 7 microseconds per object per request (measured on `hydrate`: 135 to 157 ms at 20 000 objects).

## Evidence
`crates/core/src/git/http/mod.rs:434`; SPEC-0065 Section 2 baseline.
