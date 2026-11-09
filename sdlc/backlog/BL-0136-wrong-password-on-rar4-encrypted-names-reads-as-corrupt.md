---
Id: BL-0136
Title: A wrong password on a RAR version 4 archive with encrypted file names is reported as a corrupt archive
Nature: BUG
Source: found while specifying SPEC-0087 (BL-0131)
origin: deferred from SPEC-0087
Created: 2026-11-09
Decision: deferred by the user's blanket authorization for the SPEC-0087 run ("spec all of them, design later"); SPEC-0087 pins today's corrupt answer in its E2E-027 until this entry is specified
---

## What
A RAR version 4 archive made with encrypted file names (`rar a -ma4 -hp<password>`, RAR 6.24) and extracted with a wrong password is refused with `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid rar file: ...` instead of `ERR_PASSWORD_REQUIRED` `incorrect password for this archive; call again with the correct 'password' parameter`. With no password the answer is right (`MissingPassword` from the library, mapped to the password needed message).

## Why
SPEC-0080 FR-NEW-003 and FR-NEW-004 say a wrong password on an archive with encrypted file names gets the wrong password message, and FR-NEW-015 item 4 puts it before the corrupt refusal. A member with a typo in the password is told the archive is damaged.

## Evidence
- Probe on a throwaway build of `unrar` 0.5.8 (2026-11-09, `/tmp` only): `hp4.rar` (RAR 6.24, `-ma4 -hp`) with password `nope` gives `BadData` at the first header listing; with no password `MissingPassword`; with the right one, both entries.
- `crates/core/src/tools/archive_rar.rs:162-169` (`listing_error`) maps only `MissingPassword` and `BadPassword` to the password message; `BadData` becomes corrupt.
- RAR version 4 has no password check value: a wrong key decrypts the first file header to bytes whose header checksum fails, which UnRAR reports exactly like a damaged header (`unrar_sys-0.5.8/vendor/unrar/arcread.cpp:114-119`, `BrokenHeaderMsg`).

## Notes
Possible rule: when the main header declares encrypted file names and the first encrypted header fails its checksum, answer the wrong password message; a damaged archive with a correct password then reads as a wrong password, which is the lesser evil. Needs its own spec (the two cannot be told apart in general).
