---
Id: BL-0053
Title: Clear or formally accept the warning level RustSec advisories
Nature: DEBT
Source: deferred from SPEC-0029
Created: 2026-10-14
Severity: Low
Decision: deferred by the owner on 2026-10-14 during /sdlc-spec --debt BL-0012
Fingerprint: debt:Cargo.lock:warning-level-rustsec-advisories
---

## What
`cargo audit` reports 10 allowed warnings that do not fail the gate: unsound git2 0.20.4
(RUSTSEC-2026-0183, RUSTSEC-2026-0184, ignored at `deny.toml:19-20`), unsound lru 0.12.5 and 0.16.4
(RUSTSEC-2026-0253, RUSTSEC-2026-0002), unmaintained instant (RUSTSEC-2024-0384), paste
(RUSTSEC-2024-0436), ttf-parser (RUSTSEC-2026-0192), unsafe-libyaml (RUSTSEC-2024-0449), yoke-derive.
rsa RUSTSEC-2023-0071 is ignored at `deny.toml:18` and `Makefile:47`. Bump, replace, or record
each as accepted with a reason.

## Why
Unmaintained crates get no fixes; the lru `pop()` panic safety use-after-free is a memory safety
risk if a reachable path panics inside it.

## Evidence
- `cargo audit --ignore RUSTSEC-2023-0071` on 2026-10-14: `warning: 10 allowed warnings found`.
- SPEC-0029 Section 3, rows 2 to 5.

## Notes
Suggested next command: `/sdlc-spec --debt BL-0053`.
