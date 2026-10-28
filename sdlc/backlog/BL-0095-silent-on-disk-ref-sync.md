---
Id: BL-0095
Title: branch_create and tag_create ignore a failed on disk ref write
Nature: DEBT
Source: deferred from SPEC-0056 (DEC-008)
Created: 2026-10-28
---

## What
`branch_create` and `tag_create` write the on disk ref with `let _ = repo.reference(...)`, so any libgit2 failure is dropped and the database and the on disk repository can disagree.

## Why
SPEC-0056 refuses the refused names up front, which was the one known cause; other causes (disk errors) would still be silent.

## Evidence
`crates/core/src/tools/git.rs:2465`, `crates/core/src/tools/git_tags.rs:157`.

## Notes
Decide whether a failed sync should fail the call (an observable change), or log and repair at the next hydrate.
