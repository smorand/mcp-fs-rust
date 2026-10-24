---
Id: BL-0077
Title: Ref names the validator accepts but libgit2 refuses
Nature: BUG
Source: found while designing SPEC-0052 (DEC-019)
Created: 2026-10-24
---

## What
`validate_branch_name` accepts names such as `a//b`, `v1.` and `x@{y}`; libgit2 refuses them as reference names. A branch with such a name is created in the database, then fails later when the ref is synced to the on-disk repository (`repo.reference`), with an internal error.

## Why
The branch rules and libgit2 disagree, so the failure surfaces late and with a misleading error. SPEC-0052 inherits the branch rules for tags and records the same gap (DEC-019).

## Evidence
- `crates/core/src/tools/git.rs:2200-2236` (`validate_branch_name`, no rule for `//`, trailing `.`, `@{`).
- `crates/core/src/tools/git.rs:3416-3470` (`push_branch_inner` syncs the ref with `repo.reference`, which validates the name).
- Reproduction to confirm: create branch `a//b`, then `git.remote_push` it.

## Notes
Needs `/sdlc-analysis` to decide whether to tighten the validator (observable change) or to refuse earlier with a clear message.
