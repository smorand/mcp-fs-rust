---
Id: BL-0064
Title: SPEC-0036 text no longer matches what US-0042 delivered
Nature: DEBT
Source: US-0042, SPEC-0036 (2026-10-18)
Created: 2026-10-18
Severity: Low
---

## What
Three deviations from the SPEC-0036 design text, each forced by zip 9 behavior:
- zip 9 enforces the declared uncompressed size in its reader (`zip-9.0.5/src/crc32.rs:44-50`), so `e2e_new_026` failed. `zip_read_error` (`crates/core/src/tools/archive.rs`) keeps `ERR_INVALID_ARGUMENT` naming the entry and declared size, but the FR-NEW-020 message changed from `decompressed to N bytes but declared D` to `decompressed to more bytes than declared D` (the decoded total is unknown once zip stops). Not declared in Section 7.
- DT-033's precondition `unix_mode() == Some(0o104755)` cannot hold on zip 9 (`zip-9.0.5/src/format/functions.rs:26-45` clears setuid); the test checks `external_attributes() >> 16`.
- DT-038 says `word/document.xml` equals `build_body`; it is wrapped in the document envelope (`docs/docx.rs:37-43`), so the test asserts contains.

## Proposal
An as built amendment note on SPEC-0036 (same route as BL-0062); optionally tighten DT-038 to equality against the full envelope.
