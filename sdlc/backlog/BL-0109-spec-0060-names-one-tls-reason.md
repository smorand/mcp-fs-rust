---
Id: BL-0109
Title: SPEC-0060 names a single TLS reason for a dropped connection and a batching mutation that cannot go red
Nature: DEBT
Source: found while implementing US-0115 (SPEC-0060 DR-007, DR-008, DT-001)
Created: 2026-10-30
---

## What
`sdlc/specs/SPEC-0060-tag-test-gaps/sdlc-design.md` (DR-007, DR-008, DEC-002) names `unexpected eof while reading` as the reason of a `closing_remote` leg. libgit2 reports either that or `Connection reset by peer` depending on a handshake race (5 runs failed out of 7 with the single reason). The tests accept both (`DROPPED_CONNECTION_REASONS`, `crates/core/src/tools/git.rs`). The DT-001 row for DR-003 also assumes the batching mutation turns the test red, which it does not over a file remote (BL-0108).

## Why
The design text and the code disagree. The implementation contract forbids editing the design while implementing, so the text keeps the stale wording.

## Evidence
`.agent_docs/testing.md` (gotcha of 2026-10-30), `crates/core/src/tools/git.rs` (`DROPPED_CONNECTION_REASONS`, `assert_dropped_reason`), BL-0108.
