---
Id: BL-0044
Title: A branch creation refused by the project quota leaves the new branch ref behind
Nature: BUG
Source: US-0021 (SPEC-0020), 2026-10-11
Created: 2026-10-11
---

## What
`git.branch_create` with checkout writes the new branch ref before `charge_and_apply`
(`crates/core/src/tools/git.rs:2296`) and does not remove it when the project quota refuses the
checkout. Files and usage are unchanged (SPEC-0020/FR-NEW-013 holds), but a retry fails with
"branch already exists".

## Evidence
- `crates/core/src/tools/git.rs:2296` ref written; `:2300` `charge_and_apply` refusal.
- `stash_save` removes its ref on refusal (`git.rs:6907` area), showing the expected pattern.

## Proposal
Set the ref only after the apply succeeds, or delete it on refusal; add a retry test.
