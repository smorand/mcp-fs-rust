# US-0001 verification note (DDRIFT-001)

Verified against rmcp 3.5.0's actual vendored source under `~/.cargo/registry/src/` (not
context7-derived docs), per DDRIFT-001's resolution instruction.

## Verdicts

1. **Claim 1** ("streamable-HTTP always requires `initialize`" under the spec's planned
   `NeverSessionManager` + `legacy_session_mode: false`): **CONTRADICTED**. See
   `drift/2026-10-03_00-38-28.md` for the full finding and the corrective decision (switch to
   `LocalSessionManager` + `legacy_session_mode: true`, user-approved 2026-10-03).
2. **Claim 2** (`stateless_protocol_metadata_required` semantics): **CONFIRMED** verbatim,
   `tower.rs:160-184`.
3. **Claim 3** (`json_response` JSON/SSE fallback trigger): **CONFIRMED** verbatim,
   `tower.rs:91-95`, `tower.rs:1562-1575`.

## DR-004 precondition check

Section 6.4's existing-test ledger reconciles against Section 2.4's guardrail count: 7 + 3 + 6 =
16 removed, 12 + 4 + 1 = 17 modified, 16 + 17 = 33, matching Section 2.4 exactly. CONFIRMED, no
spec defect.

## Outcome

Proceed to US-0002, carrying forward the session-manager correction recorded in
`drift/2026-10-03_00-38-28.md`. No `crates/core/Cargo.toml` or other production file was changed by
this story; the scratch dependency add used to read the source was reverted before this commit.
