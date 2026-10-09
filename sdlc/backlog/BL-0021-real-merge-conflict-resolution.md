---
Id: BL-0021
Title: Real merge conflict resolution (per-file or interactive)
Nature: FEAT
Source: specs/BACKLOG.md BL-007 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-024
Created: 2026-09-21
---

## What
Per-file or interactive conflict resolution, rather than one global `ours` or `theirs` applied to
every conflicting file.

## Why
Today `git.remote_pull` accepts `on_conflict` of `ours` or `theirs` and applies it to every
conflict; conflict markers are prohibited from entering the volume. This loses information a real
merge tool would preserve.

## Evidence
See the migrated github-enterprise-and-token-store and full-git-dev-process design.md files,
Legacy mapping tables, for DEC-024 and the shared merge engine citations.

## Notes
The volume is the working tree and there is no index, so representing an unresolved conflict
requires deciding what a half-merged simulated filesystem looks like to `fs.read`, `fs.write` and
`git.status`. That is a subsystem, not a parameter.
