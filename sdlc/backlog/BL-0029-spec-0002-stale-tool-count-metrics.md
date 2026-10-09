---
Id: BL-0029
Title: Update SPEC-0002's stale tool-count assertions to the current 102-tool contract
Nature: DEBT
Source: retro-spec 2026-10-09, SPEC-0002 (platform-foundation) split
Created: 2026-10-09
Severity: Low
Fingerprint: stale-doc:sdlc/specs/SPEC-0002-platform-foundation:tool-count-assertions
---

## What
`sdlc/specs/SPEC-0002-platform-foundation/spec.md`'s legacy tool-count assertions (e.g. "45
always-on tools: 35 `fs.*` + 10 `admin.*`") and matching E2E tool-count checks do not match the
current `AGENTS.md` figure of 102 tools (39 `fs.*`, 14 `admin.*`, 39 `git.*`, 4 `git.auth*`, 6
`git.pr_*`, +4 `search.*`).

## Why
Any E2E test asserting an exact tool-family total needs updating before it can be treated as
green against current code; a reader trusting SPEC-0002's counts will be misled.

## Evidence
`AGENTS.md` overview line vs. `sdlc/specs/SPEC-0002-platform-foundation/spec.md` §2/§8 tool-count
citations.

## Notes
Expected drift: later specs (now migrated as SPEC-0011 through SPEC-0018) added git, trash,
export, archive capabilities since SPEC-0002 was first written.
