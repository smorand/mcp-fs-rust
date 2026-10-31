---
Id: BL-0115
Title: The flag column of .agent_docs/tools.md is not tested against the contract annotations
Nature: DEBT
Source: found while planning doc-drift-fixes (BL-0085)
Created: 2026-10-31
---

## What
`.agent_docs/tools.md` carries a flag column (`D`, `RO`, `I`, `OW`) for 116 tools. Nothing compares it with the `annotations` of `TOOL_CONTRACT.txt` (120 tools: the 4 `search.*` tools are in `.agent_docs/search.md`, not in that table). `git.remote_push_tag` drifted to `!D` after SPEC-0052 changed its annotation, unnoticed for days.

## Why
The golden test compares the contract with the code, the table is the human copy and rots silently.

## Evidence
`.implement/flags.py` of the doc-drift-fixes run: contract 120 tools, table 116 rows, one mismatch before A-003 (`git.remote_push_tag`). Read-only rows list three flags (`D` omitted, false); contract entries of read-only tools omit `destructiveHint`.

## Notes
A test in `tools/contract_golden.rs` parsing the table (rows with 3 flags mean `!D`) would close it; or generate the column.
