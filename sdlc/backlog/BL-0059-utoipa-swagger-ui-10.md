---
Id: BL-0059
Title: Move utoipa-swagger-ui to 10 to drop the yanked zip 3.0.0
Nature: DEBT
Source: deferred from SPEC-0035
Created: 2026-10-16
Severity: Low
Decision: deferred by the owner on 2026-10-16 during /sdlc-spec BL-0053
---

## What
`utoipa-swagger-ui = { version = "9" }` (`Cargo.toml:113`) build depends on the yanked `zip` 3.0.0; 10.0.1 is available (rust-version 1.88).

## Why
Each audit run keeps reporting it (or it is recorded as accepted only until this entry lands, SPEC-0035 DEC-003).

## Evidence
yanked zip 3.0.0 in `cargo audit` (2026-10-16), `cargo tree -i zip@3.0.0` shows only utoipa-swagger-ui 9.

## Notes
Suggested next command: `/sdlc-spec --debt BL-0059`.
