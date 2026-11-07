---
Id: BL-0131
Title: Truncated RAR4 and header encrypted RAR5 archives are not detected
Nature: DEBT
Source: found while running US-0173
Created: 2026-11-05
---

## What
`ensure_rar5_complete` (`crates/core/src/tools/archive_rar.rs`) walks the blocks of a plain RAR5 archive and refuses a truncated one as corrupt (FR-NEW-011). It does not cover two cases: a RAR4 archive (the safe `unrar` API returns Ok with fewer entries for a cut on a header boundary, and a data cut surfaces as a CRC error worded as an oversize), and a RAR5 archive with encrypted headers, where the walk stops at the archive encryption header (type 4) because later headers cannot be read without the password.

## Why
FR-NEW-011 says "truncated" without limiting it to RAR5. A truncated archive of those two kinds extracts fewer files without an error.

## Evidence
`ensure_rar5_complete` doc comment; US-0173 review notes; RAR4 fixtures are not available yet (US-0175 waits for user provided fixtures).

## Notes
Proposal: once RAR4 and encrypted name fixtures exist, add a RAR4 block walk (crc16, type, flags, size, optional add size, end block 0x7B) and, for header encrypted archives, check after decryption through the library listing that the end block was reached. Alternatively narrow FR-NEW-011 to what is verifiable.
