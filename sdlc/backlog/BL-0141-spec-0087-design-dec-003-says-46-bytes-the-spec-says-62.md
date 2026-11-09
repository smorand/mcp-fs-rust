---
Id: BL-0141
Title: SPEC-0087 design DEC-003 puts the leading part of an encrypted names RAR5 archive at 46 bytes, FR-NEW-004 says 62
Nature: DEBT
Source: found while running US-0203
Created: 2026-11-09
---

## What
DEC-003 of the SPEC-0087 design says that, with no password, marker plus archive encryption header (46 bytes) is the whole leading part, so a cut at 46..61 bytes is left to the reader. FR-NEW-004 and E2E-034 say 62 (46 plus the 16 byte IV of the first encrypted header): UnRAR only asks for the password from 62 bytes (probe P-3). US-0201 followed DEC-003, so `hidden.rar` and `hp5two.rar` cut at 46..61 bytes with no password extracted as success with 0 files.

## Why
The code now follows the spec (US-0203 adds the 16 byte IV check in `walk_rar5_encrypted`), so DEC-003 text is out of date.

## Evidence
`crates/core/src/tools/archive_rar.rs` (`walk_rar5_encrypted`, no password branch), `e2e_0087_034_every_length_of_the_encrypted_names` red before the one line fix.

## Notes
Proposal: correct DEC-003 in `sdlc-design.md` when the spec is next amended (the design is not edited by the implement run).
