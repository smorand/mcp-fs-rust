---
Id: BL-0061
Title: Replace the deprecated serde_yaml with a maintained fork
Nature: DEBT
Source: deferred from SPEC-0035
Created: 2026-10-16
Severity: Low
Decision: deferred by the owner on 2026-10-16 during /sdlc-spec BL-0053
---

## What
`serde_yaml = "0.9"` (`Cargo.toml:25`, used by crates/core and crates/agent for config) is deprecated and pulls unmaintained unsafe-libyaml (RUSTSEC-2024-0449); maintained forks exist (serde_norway 0.9.42, serde_yaml_ng 0.10).

## Why
Each audit run keeps reporting it (or it is recorded as accepted only until this entry lands, SPEC-0035 DEC-003).

## Evidence
`cargo tree -i unsafe-libyaml` (2026-10-16); a new dependency, so it needs an owner decision on the fork.

## Notes
Suggested next command: `/sdlc-spec --debt BL-0061`.
