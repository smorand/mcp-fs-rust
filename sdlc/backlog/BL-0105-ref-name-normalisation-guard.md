---
Id: BL-0105
Title: Guard tag_create and the reconcile against names libgit2 normalises
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0083)
Created: 2026-10-30
---

## What
`git.tag_create` and `reconcile_incoming_tags` call `repo.reference` on names such as `a//b`, which libgit2 silently normalises to `a/b` on disk while the database row keeps `a//b`. Guard those calls with `is_valid_name` as the publish path now does.

## Why
The database and the repository on disk disagree. Related to BL-0095.

## Evidence
`crates/core/src/tools/git_tags.rs:157`, `crates/core/src/tools/git_tags.rs:702` (the publish guard).
