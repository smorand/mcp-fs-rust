---
Id: BL-0060
Title: Move the Rust toolchain to at least 1.94.1 to unlock a fixed aws-sdk-s3
Nature: DEBT
Source: deferred from SPEC-0035
Created: 2026-10-16
Severity: Low
Decision: deferred by the owner on 2026-10-16 during /sdlc-spec BL-0053
---

## What
the project pins Rust 1.88 (`Cargo.toml:8`, `rust-toolchain.toml:2`); aws-sdk-s3 1.152.0 needs 1.94.1, and the pinned aws-sdk-s3 1.119 pulls lru 0.12.5 (RUSTSEC-2026-0253, RUSTSEC-2026-0002, unsound).

## Why
Each audit run keeps reporting it (or it is recorded as accepted only until this entry lands, SPEC-0035 DEC-003).

## Evidence
`cargo info aws-sdk-s3@1.152.0`: rust-version 1.94.1; `cargo tree -i lru@0.12.5` shows aws-sdk-s3.

## Notes
Suggested next command: `/sdlc-spec --debt BL-0060`.
