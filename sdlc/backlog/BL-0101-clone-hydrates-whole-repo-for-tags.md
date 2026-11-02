---
Id: BL-0101
Title: Clone hydrates the whole repository before the tag reconcile
Nature: DEBT
Source: deferred from SPEC-0060 (carried from BL-0082 (US-0088))
Created: 2026-10-30
---

## What
Clone hydrates the whole project repository (`export_to_repo`, O(all objects)) before reconcile so `tags_identical` can compare against local objects. It could hydrate only when an incoming tag name already exists locally.

## Why
Clone cost grows with repository size for a check that is rarely needed.

## Evidence
`crates/core/src/tools/git.rs:3369-3373`, `crates/core/src/git/remote.rs:1429`. Measure first.

Finding (2026-11-02, while specifying SPEC-0073): the hydrate call is `hydrate_if_stale`, one `COUNT(*)` when nothing changed, and a clone usually lands in a new or small project, so the saving is probably small. Skipping it also breaks the on disk tag ref write that follows each row (libgit2 refuses a ref whose object is absent), so a lazy hydrate must still run when any tag is added. Measure a clone into a large project first, or drop this entry.
