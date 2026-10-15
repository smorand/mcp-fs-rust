---
Id: BL-0054
Title: Give the SDLC pipeline a sanctioned way to mark an as-built spec superseded
Nature: CHORE
Source: /sdlc-spec --debt BL-0028, 2026-10-14
Created: 2026-10-14
Severity: Low
Fingerprint: chore:pi-prompts:supersede-as-built-spec
---

## What
No command may amend an existing spec: `/sdlc-spec` (Invariant 10), `/sdlc-implement` (Invariant 10),
and `/sdlc-retro-spec` defers the rewrite to `/sdlc-spec`. BL-0028 had to be closed by an owner
authorized manual note appended to SPEC-0002 (Section 11).

## Why
Every future protocol or contract change that obsoletes an as-built spec hits the same wall and needs a
manual exception.

## Evidence
- `sdlc/specs/SPEC-0002-platform-foundation/spec.md` Section 11 (manual supersession note, 2026-10-14).

## Notes
Lives in the pi prompt set (`~/.pi/agent/prompts/sdlc-*.md`), not in this repository. Suggested: allow
`/sdlc-retro-spec` or `/sdlc-spec` to append a `Superseded by SPEC-NNNN` note, never rewriting rows.
