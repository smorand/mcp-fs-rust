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
