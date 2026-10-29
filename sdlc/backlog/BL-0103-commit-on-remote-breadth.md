---
Id: BL-0103
Title: Decide what counts as "on the remote" for a tag publish
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0083)
Created: 2026-10-30
---

## What
DEC-020 and FR-NEW-019 of SPEC-0052 say "any branch the remote holds", but `commit_on_remote` builds its tip set from every advertised ref (`refs/pull/*`, tags, HEAD, peeled `^{}`). A commit reachable only from a pull request ref counts, and a refusal walks every ref (O(n + u*d)). Either restrict to `refs/heads/*` plus peeled tags, or amend DEC-020. Arguably the wider set is safer: those commits exist remotely.

## Why
Design text and code disagree on a rule an operator can observe.

## Evidence
`crates/core/src/tools/git_tags.rs` `commit_on_remote`; `crates/core/src/git/remote.rs` `advertise`.
