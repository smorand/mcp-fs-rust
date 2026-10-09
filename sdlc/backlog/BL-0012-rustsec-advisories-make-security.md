---
id: BL-0012
title: Clear the new RustSec advisories that turn make_security_passes red
kind: debt
suggested_command: /spec-debt
created: 2026-10-09
origin: SPEC-0015 implementation run (US-0006), found while adding archive crates
---

## What
`cargo audit` now reports advisories against pre-existing dependencies, none of them added by
SPEC-0015: `git2` (RUSTSEC-2026-0183, RUSTSEC-2026-0184), `instant` (RUSTSEC-2024-0384),
`lru` (RUSTSEC-2026-0002, RUSTSEC-2026-0253), `paste` (RUSTSEC-2024-0436), `rsa`
(RUSTSEC-2023-0071), `ttf-parser` (RUSTSEC-2026-0192), plus one on `yoke-derive`. Bump, replace or
explicitly ignore each with a written reason in `deny.toml` / the audit config.

## Why
The test that runs the security gate (`make_security_passes`) fails on these, so `make check`
cannot be green, independent of feature work.

## Evidence
- `cargo audit` output on branch `feat/SPEC-0015-archive-extract-in-place` (2026-10-09), crates
  listed above; the US-0006 implementer reproduced the failure with the whole SPEC-0015 diff
  stashed, so it predates this lot.
- The test is network dependent (live advisory database), so it can pass or fail between runs
  with no code change.
