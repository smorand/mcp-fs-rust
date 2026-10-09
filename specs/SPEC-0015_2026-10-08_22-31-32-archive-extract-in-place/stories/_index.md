---
Spec ID: SPEC-0015
Source Specification: specs/SPEC-0015_2026-10-08_22-31-32-archive-extract-in-place/spec.md
Nature: FEAT
Depth: L
Generated-on: 2026-10-08
Target tier: 2 (resolved from default — no --tier flag, no .spec.json in repo root)
Total story count: 12 (0 epics)
---

# SPEC-0015 — Archive Extraction In Place — Story Index

## Slicing Verdict

| Verdict | Epics refused | Drift entries carried |
|---|---|---|
| SLICEABLE | none | DRIFT-001 (US-0001), DRIFT-002 (US-0002) |

## Implementation Order

| Order | ID | Epic | Title | FRs | Scenarios | Tests | Files | Depends On | min_tier | Status |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | US-0001 | n/a | [drift] Resolve DRIFT-001 — zip metadata readable without password | (folded into US-0006) | none | none | `tools/archive.rs` | none | 2 | done (no code of its own; tracker only, orchestrator-closed — real resolution happens in US-0006's commit) |
| 2 | US-0002 | n/a | [drift] Resolve DRIFT-002 — zip entry symlink bit detection | (folded into US-0007) | none | none | `tools/archive.rs` | none | 2 | done (no code of its own; tracker only, orchestrator-closed — real resolution happens in US-0007's commit) |
| 3 | US-0003 | n/a | New error code `ERR_PASSWORD_REQUIRED` | FR-NEW-028 | none | E2E-NEW-030,045,046 | `errors.rs` | none | 2 | done |
| 4 | US-0004 | n/a | Tool skeleton: registration, authorize, path resolve, directory rejection | FR-NEW-001,002,003,004 | none | E2E-NEW-025,059,060 | `tools/archive.rs` (new), `tools/mod.rs`, `mcp/server.rs` | none | 2 | todo |
| 5 | US-0005 | n/a | Format detection, unsupported extension, tar+password gate | FR-NEW-005,006,008 | SC-011 | E2E-NEW-022,024,034,035,038,039 | `tools/archive.rs` | US-0004 | 2 | todo |
| 6 | US-0006 | n/a | Cargo dependencies; zip/7z entry listing; corrupt-archive detection; password required/incorrect handling | FR-NEW-007,009,010,011 | SC-004,SC-005,SC-011 | E2E-NEW-006,008,023,036,037,057,058 | `tools/archive.rs`, `Cargo.toml`, `crates/core/Cargo.toml` | US-0005, US-0001 (informational), US-0002 (informational), US-0003 | 2 | todo |
| 7 | US-0007 | n/a | Symlink/hardlink/device rejection; zip-slip entry-path rejection; void-on-disqualify ordering | FR-NEW-013,014,015 | SC-008,SC-009 | E2E-NEW-014,015,016,017,018,019,040,041 | `tools/archive.rs` | US-0006, US-0002 (informational) | 2 | todo |
| 8 | US-0008 | n/a | Destination computation; no-clobber/overwrite; quota charge; decoded-size-vs-declared check | FR-NEW-016,017,018,019,020 | SC-006,SC-010 | E2E-NEW-010,011,020,021,026,042,043,049 | `tools/archive.rs` | US-0006, US-0007 | 2 | todo |
| 9 | US-0009 | n/a | Write pass; result counts; response shape | FR-NEW-021,022,023,024 | SC-001,SC-002,SC-003,SC-004,SC-005,SC-006,SC-007,SC-008,SC-009,SC-011 | E2E-NEW-001,002,003,004,005,007,009,012,013,027,050,051,052,053,054,055,056 | `tools/archive.rs`, `core/fs_ops.rs` | US-0008 | 2 | todo |
| 10 | US-0010 | n/a | Audit entry; tracing; password-never-logged | FR-NEW-025,026,027 | none | E2E-NEW-028,029,044 | `tools/archive.rs` | US-0009 | 2 | todo |
| 11 | US-0011 | n/a | REST route + OpenAPI schema | FR-NEW-029 | none | E2E-NEW-031,032,047 | `api/dataplane.rs`, `api/openapi.rs` | US-0009 | 2 | todo |
| 12 | US-0012 | n/a | `TOOL_CONTRACT.txt` + golden regeneration | FR-NEW-030 | none | E2E-NEW-033,048,061 | `TOOL_CONTRACT.txt`, `tool-contract-golden.json` (generated) | US-0011 | 2 | todo |

## Dependency Graph

```
US-0001 (drift, no deps) ──┐
US-0002 (drift, no deps) ──┤
US-0003 (no deps) ─────────┤
                            v
US-0004 (no deps)
   │
   v
US-0005 (needs US-0004)
   │
   v
US-0006 (needs US-0005; informed by US-0001, US-0002; needs US-0003 for ERR_PASSWORD_REQUIRED)
   │
   v
US-0007 (needs US-0006; informed by US-0002)
   │
   v
US-0008 (needs US-0006, US-0007)
   │
   v
US-0009 (needs US-0008)
   │
   ├──> US-0010 (needs US-0009)
   │
   └──> US-0011 (needs US-0009; can run parallel to US-0010)
            │
            v
        US-0012 (needs US-0011; last story)
```

## Coverage Verification

- Requirements: 30 total / 30 assigned / unassigned: none.
  - Orphan FRs (requirement-only, no scenario) homed: FR-NEW-004 (US-0004), FR-NEW-020
    (US-0008), FR-NEW-022 (US-0009), FR-NEW-027 (US-0010), FR-NEW-028 (US-0003), FR-NEW-029
    (US-0011), FR-NEW-030 (US-0012). All 7 confirmed present in exactly one story's Functional
    Requirements section.
- Tests: 61 total / 61 assigned / unassigned: none.
  - Distribution: US-0003=3, US-0004=3, US-0005=6, US-0006=7, US-0007=8, US-0008=8, US-0009=17,
    US-0010=3, US-0011=3, US-0012=3. Sum = 61.
  - Matrix-shorthand expansion applied before counting (e.g. SC-011's row
    "E2E-NEW-022, 023, 024, 034, 035, 036, 037, 038, 039" expanded to nine full IDs; "FR-NEW-006,
    007, 008" expanded similarly) — no bare-number continuation was missed.
- Scenarios: 11 total / 11 covered (≥1 story each):
  - SC-001→US-0009, SC-002→US-0009, SC-003→US-0009, SC-004→US-0006/US-0009, SC-005→US-0006/
    US-0009, SC-006→US-0008/US-0009, SC-007→US-0009, SC-008→US-0007/US-0009, SC-009→US-0007/
    US-0009, SC-010→US-0008, SC-011→US-0005/US-0006/US-0009.
  - Uncovered: none.
- Drift entries carried: DRIFT-001 (US-0001, resolved in practice by US-0006), DRIFT-002
  (US-0002, resolved in practice by US-0007). Both open per spec §19, both now have a tracked
  story home.
