---
Id: BL-0052
Title: Cover the untested MCP protocol requirements of SPEC-0028
Nature: DEBT
Source: retro-spec 20261014-205518
Created: 2026-10-14
Severity: Medium
Fingerprint: untested-requirement:sdlc/specs/SPEC-0028-mcp-protocol/spec.md:SPEC-0028
---

## What
Three SPEC-0028 requirements have no test at any level, and one test is weaker than its requirement:
- E2E-014, FR-012: a separator variant (`fs_read`) answers `tool not found`.
- E2E-015, FR-016: a session id unknown to the process is refused.
- E2E-016, FR-004: protocol version and capabilities in the `initialize` answer.
- E2E-003, FR-003: the test asserts any 4xx, not 422.

## Why
The FR-019 drop was silent once; nothing would catch a server-side resolver reappearing, an SDK
upgrade changing the pre-initialize status, or session handling drifting.

## Evidence
- `sdlc/specs/SPEC-0028-mcp-protocol/spec.md` section 8, rows marked `untested`.
- `crates/core/src/app.rs:547`: `assert!(r.status().is_client_error(), "expected a 4xx, got {}", r.status());`

## Notes
Related SPEC-0028/FR-003, FR-004, FR-012, FR-016. Suggested next command: `/sdlc-spec --debt BL-0052`
(or `/sdlc-retro-tests` when available).
