---
Id: BL-0058
Title: Move zip from the yanked 2.4.2 to a maintained release
Nature: DEBT
Source: deferred from SPEC-0035
Created: 2026-10-16
Severity: Low
Decision: deferred by the owner on 2026-10-16 during /sdlc-spec BL-0053
---

## What
direct dependency `zip = { version = "2" }` (`Cargo.toml:84`), used by export, archive extraction and docx; every 2.x release is yanked and the latest is 9.0.4.

## Why
Each audit run keeps reporting it (or it is recorded as accepted only until this entry lands, SPEC-0035 DEC-003).

## Evidence
yanked zip 2.4.2 in `cargo audit` (2026-10-16); `cargo update -p zip@2.4.2` cannot select a 2.x release.

## Notes
Suggested next command: `/sdlc-spec --debt BL-0058`.
