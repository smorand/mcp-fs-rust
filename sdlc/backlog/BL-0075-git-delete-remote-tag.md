---
Id: BL-0075
Title: Delete a tag on the remote
Nature: FEAT
Source: deferred from SPEC-0052 (git tags)
Created: 2026-10-24
---

## What
Let a member delete a tag on the remote, not only locally.

## Why
SPEC-0052 publishes a tag and deletes it locally only; a wrongly published tag cannot be withdrawn.

## Evidence
`sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md` section 3.2 and FR-NEW-005.

## Notes
Destructive remote action, own decision: confirmation rules and audit wording to be specified.
