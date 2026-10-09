---
Id: BL-0023
Title: Remote branch name distinct from the local branch on push
Nature: FEAT
Source: specs/BACKLOG.md BL-009 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-014
Created: 2026-09-21
---

## What
A `remote_branch` parameter on `git.remote_push`, allowing `local:remote` refspecs.

## Why
The remote branch name always equals the local one today, which prevents pushing a local branch
under a different remote name.

## Evidence
See the migrated github-enterprise-and-token-store design.md, Legacy mapping table.

## Notes
Deferred at the original interview as a later refinement.
