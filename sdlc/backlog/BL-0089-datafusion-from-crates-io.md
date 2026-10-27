---
Id: BL-0089
Title: Move datafusion from the pinned git revision to crates.io release 56
Nature: DEBT
Source: deferred from SPEC-0055
Created: 2026-10-27
---

## What
SPEC-0055 pins datafusion to a git revision of apache/datafusion main (arrow 60, zstd 0.14) because the released 55.2.0 needs arrow 59 and so zstd 0.13. Replace the git dependency by the crates.io release that carries arrow 60, and drop the single git source allowed in `deny.toml`.

## Why
A git dependency is unreleased code and an allowed source exception. It is only there until a release exists.

## Evidence
`Cargo.toml` datafusion entry (after SPEC-0055); crates.io `datafusion` max version 55.2.0 on 2026-10-27; apache/datafusion `Cargo.toml` on main declares `arrow = 60.0.0` and `zstd = 0.14`.

## Notes
Trigger: `cargo search datafusion` shows a version above 55.2.0 whose `arrow` requirement is 60 or more. Remove the `allow-git` entry of `deny.toml` and `.agent_docs/dependencies.md` pin section in the same change.
