---
Id: BL-0137
Title: The debug test build runs the RAR key derivations and UnRAR unoptimized, so password tests over many cuts are slow
Nature: DEBT
Source: found while designing SPEC-0087 (2026-11-09)
origin: deferred from the SPEC-0087 design
Created: 2026-11-09
Decision: deferred; SPEC-0087 DEC-007 reduces the password loop of its E2E-034 instead
---

## What
`cargo test` builds every dependency with the dev profile (`opt-level = 0`): the C++ UnRAR source (`unrar_sys`), `sha1`, `sha2`, `hmac`, `aes`. Each RAR extraction with a password pays two key derivations (UnRAR's and, from SPEC-0087, ours: 262 144 SHA-1 rounds for RAR4, about 98 000 HMAC SHA-256 for RAR5 at the default count), so a test that loops over every length of an encrypted archive takes minutes in the debug build where the release probe took 3 to 10 seconds.

## Why
SPEC-0087 E2E-034 had to run a reduced set of lengths with the right password (SPEC-0087 DEC-007). An optimized dependency profile would let such loops run in full and cut the `--features rar` run of `test.sh` for everyone.

## Evidence
- `rg -n 'profile.dev.package' Cargo.toml` finds nothing (2026-11-09); `Cargo.toml:152` has only `[profile.release]` tuning.
- SPEC-0087 section 14.3 "Run time": 2.9 s (`hp4.rar`), 7.1 s (`hidden.rar`), 10.4 s (`hp5two.rar`) for every length in release.
- The rust skill's template recommends `[profile.dev.package."*"] opt-level = 3` (dependencies optimized, own crates at 0).

## Notes
Possible change: `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml`, measured on `make test` wall clock and on incremental rebuild time before adoption; then widen SPEC-0087 E2E-034 back to every length.
