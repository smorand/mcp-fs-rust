---
Id: BL-0088
Title: Flaky artifacts e2e tests: doc service timed out after 30s under full load
Nature: BUG
Source: US-0095 run (SPEC-0053), seen once
Created: 2026-10-25
---

## What
`spec_0019_e2e_151_a_path_escaping_the_output_is_never_read` and `spec_0019_e2e_162_a_traversal_target_stays_verbatim` failed once in a full `make check` with `fs.documentize: ERR_INTERNAL_ERROR: doc service timed out after 30s`. Alone, all 165 `spec_0019_e2e_` tests pass in 10 s.

## Evidence
`crates/core/src/tools/artifacts.rs:400`; log `.implement/US-0095-verify.log` (not committed). Same family as BL-0084 (full load flake).

## Proposed
Find what the fake doc service waits on (process spawn under load); raise the test deadline or make the spawn cheaper.
