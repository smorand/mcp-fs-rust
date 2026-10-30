---
Id: BL-0112
Title: Push receive, clone, fetch and pull still import the whole repository
Nature: DEBT
Source: deferred from SPEC-0062 (from BL-0110)
Created: 2026-10-30
---

## What
After SPEC-0062, four production sites still call `BlobObjectDb::import_from_repo`, which reads every object of the libgit2 repository and probes the blob store for each: push receive (`crates/core/src/git/http/mod.rs:572`), clone (`crates/core/src/tools/git.rs:3324`), fetch (`crates/core/src/tools/git.rs:3918`) and the fetch step of pull (`crates/core/src/tools/git.rs:6803`). A pack brings an unbounded set, so the cost is legitimate for a first clone, but a fetch or a push that adds 3 commits to a 50 000 object repository pays for all 50 000. `import_reachable` could be called with the new ref tips and the tag objects instead.

## Why
Same shape as the commit path measured in SPEC-0062 (about 55 µs per repository object). The fetch and pull paths also feed `reconcile_incoming_tags`, which relies on the tag objects being imported, so the tips passed must include every incoming tag object. Measure first.

## Evidence
`crates/core/src/git/http/mod.rs:572`; `crates/core/src/tools/git.rs:3324,3918,6803`; SPEC-0062 Section 2 baseline.
