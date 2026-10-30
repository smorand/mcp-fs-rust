---
Id: BL-0111
Title: Every git tool call re-lists every object to hydrate libgit2
Nature: DEBT
Source: deferred from SPEC-0061
Created: 2026-10-30
---

## What
`hydrate` (`crates/core/src/tools/git.rs:2069`) calls `BlobObjectDb::export_to_repo` (`crates/core/src/git/odb.rs`), which lists every object row of the index on each call, even for a lightweight tag. After SPEC-0061 this is the remaining O(repository objects) floor of `git.tag_create`: about 100 ms at 20 000 objects (lightweight tag, debug build) versus 3 to 8 ms at 500.

## Why
Every read and write tool pays it. A cheap "already hydrated up to generation N" marker, or hydrating only the objects reachable from the touched refs, would remove it. This changes caching and invalidation, so it needs its own design.

## Evidence
`crates/core/src/tools/git.rs:2069-2072`; SPEC-0061 Section 2 baseline (lightweight column).
