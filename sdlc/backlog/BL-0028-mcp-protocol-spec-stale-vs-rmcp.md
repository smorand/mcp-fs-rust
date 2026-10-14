---
Id: BL-0028
Title: Re-derive SPEC-0002's MCP protocol requirements against the current rmcp transport
Nature: DEBT
Source: retro-spec 2026-10-09, SPEC-0002 (platform-foundation) split
Created: 2026-10-09
Severity: Medium
Fingerprint: doc-contradiction:crates/core/src/mcp/server.rs:hand-rolled-protocol-superseded
---

## What
`sdlc/specs/SPEC-0002-platform-foundation/spec.md` FR-012 through FR-019 (and the corresponding
E2E-015 through E2E-042) describe a hand-rolled JSON-RPC/SSE MCP protocol layer (`mcp/mod.rs`:
handshake capabilities, SSE framing with `event: message\ndata:`, a custom `ToolRegistry`/
tool-name resolution). `AGENTS.md` states the transport was migrated to the official `rmcp` SDK
(this repo's SPEC-0013/rmcp-migration work, now migrated as SPEC-0016), that `mcp/mod.rs` no
longer exists, and that the production module is `crates/core/src/mcp/server.rs`.

## Why
A material fraction of SPEC-0002's protocol-layer requirements and tests may no longer describe
current behaviour (exact wire framing, SSE envelope shape, error-vs-result distinction under
`rmcp`, tool-name resolution rules). Anyone implementing against SPEC-0002 as written risks
building to a protocol that no longer exists.

## Evidence
`AGENTS.md` ("The hand-rolled JSON-RPC/SSE framing layer this module used to own... is gone");
current tree has no `crates/mcp-fs/src/mcp/mod.rs`, only `crates/core/src/mcp/server.rs`.

## Notes
Recommended follow-up: a dedicated `/sdlc-spec` or targeted retro pass over the current
`mcp/server.rs` to re-derive FR-012 through FR-019 and their E2E coverage against the `rmcp`
behaviour, superseding the legacy section in SPEC-0002 rather than patching it in place.

## Decision (2026-10-14, owner)
- Not a BUG: the current rmcp behaviour is authorized by SPEC-0016 (DEBT, implemented), which
  declared the protocol breaks (`sdlc/specs/SPEC-0016-rmcp-3x-migration-prep/sdlc-design.md:13-16`,
  `:63-75`, `:189`): mandatory `initialize`, `-32602 "tool not found"`, SSE framing kept.
  SPEC-0002 FR-012 to FR-018 are stale text to supersede.
- SPEC-0002/FR-019 (server side separator variants, `fs_read` resolving to `fs.read`) was dropped by the
  migration without being declared. Decision: accept the drop. The served names are canonical
  (dotted) only; the CLI agent keeps its client side fuzzy resolution. Document it in the superseding text.
- Route: `/sdlc-retro-spec` to re-derive the MCP protocol section of SPEC-0002 from
  `crates/core/src/mcp/server.rs`, `crates/core/src/app.rs` and SPEC-0016, superseding FR-012 to FR-019.
