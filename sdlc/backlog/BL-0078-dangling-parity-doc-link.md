---
Id: BL-0078
Title: .agent_docs/git.md links to a missing parity.md
Nature: DEBT
Source: found while writing the git divergence register (BL-0027)
Created: 2026-10-24
---

## What
`.agent_docs/git.md:752` points to `.agent_docs/parity.md`, which does not exist, and says "Do not restate it elsewhere".

## Why
A reader following the pointer finds nothing; the divergences from the C# lineage are not documented where the text says they are.

## Evidence
- `.agent_docs/git.md:752`.
- `ls .agent_docs/parity.md` fails.

## Notes
Either restore the document or point to `lineage.md`.
