---
Id: BL-0020
Title: Squash merge support
Nature: FEAT
Source: specs/BACKLOG.md BL-006 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-014 discussion
Created: 2026-09-21
---

## What
Collapse a branch's commits into one before or during merge.

## Why
The merge path currently creates an ordinary merge commit with two parents; some workflows want a
single squashed commit instead.

## Evidence
See the migrated github-enterprise-and-token-store design.md, Legacy mapping table.

## Notes
Out of scope at the original interview. Interacts with [[BL-0019]], since squash is usually a
property of how a pull request is merged rather than of a local operation.
