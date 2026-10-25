---
Id: BL-0085
Title: .agent_docs/tools.md marks git.remote_push_tag as not destructive
Nature: DEBT
Source: review of US-0092 (SPEC-0053)
Created: 2026-10-25
---

## What
The `git.remote_push_tag` row of `.agent_docs/tools.md` shows the flag `!D`, while the contract annotation is `destructiveHint=true` (`TOOL_CONTRACT.txt:534`, set by SPEC-0052 US-0090 when overwrite was added).

## Why
The reference table and the machine contract disagree; the parity tests only compare names.

## Evidence
`.agent_docs/tools.md` row `git.remote_push_tag`; `TOOL_CONTRACT.txt:534`.

## Notes
Fix the flag in the table; consider extending the parity test to compare annotations.
