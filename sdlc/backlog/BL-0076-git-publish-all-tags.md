---
Id: BL-0076
Title: Publish every local tag in one action
Nature: FEAT
Source: deferred from SPEC-0052 (git tags)
Created: 2026-10-24
---

## What
Let a member publish all local tags at once.

## Why
SPEC-0052 publishes one named tag only, which is tedious after many releases.

## Evidence
`sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md` section 3.2 and FR-NEW-007.

## Notes
Left out on purpose: it can publish private marks by accident. Decide the guard (confirmation, dry run, per tag report).
