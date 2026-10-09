---
Id: BL-0014
Title: Write a specification for the crates/agent CLI client
Nature: FEAT
Source: specs/BACKLOG.md BL-002 (migrated), the eight-spec split, DEC-001
Created: 2026-09-18
---

## What
A retro-specification of the interactive CLI agent: its config schema, the LLM streaming and
tool-calling loop, the wrap-aware line editor, markdown-to-ANSI rendering, and the terminal
invariants it depends on.

## Why
The agent is currently documented only in `.agent_docs/agent.md`, not in a formal spec.

## Evidence
`crates/agent/src/`: `main.rs`, `mcp.rs`, `llm.rs`, `input.rs`, `ui.rs`, `session.rs`,
`spinner.rs`, `config.rs`; documented in `.agent_docs/agent.md`. Implemented and shipping.

## Notes
The agent is an MCP **client**, not part of the server product (`crates/agent/src/main.rs:3-5`).
Its real value is as the end-to-end exerciser of the tool surface, which the server specs already
assert directly. Deferred because it competed for effort with the eight server specs being
written at the time.
