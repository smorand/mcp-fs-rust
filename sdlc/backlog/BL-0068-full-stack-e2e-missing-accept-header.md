---
Id: BL-0068
Title: full_stack_lifecycle fails at MCP initialize with 406 on main
Nature: BUG
Source: found during US-0050 (SPEC-0042)
Created: 2026-10-19
Severity: Low
---

## What
`make test-e2e-full` fails before reaching any storage step: `MCP initialize: http 406 Not Acceptable` (`crates/core/tests/full_stack_e2e.rs:175`). The `initialize` POST (`:165-172`) sends no `Accept: application/json, text/event-stream` header, which the streamable HTTP service requires.

## Evidence
Same failure on main before US-0050 (git stash run, 2026-10-19) and after it, with PostgreSQL (pgvector, port 55432) and a moto S3 stand in on :9000. `rg -n -i accept crates/core/tests/full_stack_e2e.rs` finds no match.

## Notes
The test is the only e2e covering real PostgreSQL + S3 + git end to end, so it is currently dead. Suggested next command: `/sdlc-spec BL-68` (or `/sdlc-analysis`).
