---
Id: BL-0055
Title: Doc service CLI timeout test flakes when the full suite loads the machine
Nature: DEBT
Source: US-0034 landing (SPEC-0031), 2026-10-15
Created: 2026-10-15
Severity: Low
Fingerprint: debt:crates/core/src/docs/service.rs:cli_timeout_kills_the_child_and_removes_the_tempdir
---

## What
`docs::service::tests::cli_timeout_kills_the_child_and_removes_the_tempdir` gives a 3 s timeout to a
script whose first line writes a marker file; under the full parallel suite the child did not start
within 3 s, so `std::fs::read_to_string(&marker).unwrap()` (`crates/core/src/docs/service.rs:854`)
panicked with `NotFound` and `make check` went red with no code change.

## Why
A red gate with no code change blocks landings and teaches people to rerun until green.

## Evidence
- `.implement/land34.log`: `panicked at crates/core/src/docs/service.rs:854:70: called Result::unwrap() on an Err value: Os { code: 2, kind: NotFound, ... }`, `test result: FAILED. 2619 passed; 1 failed` (lib suite, 286.41s).
- Same tree passed in the story run and in a full rerun (2739 passed, 0 failed); the test alone passed 3 of 3.

## Notes
Proposal: wait for the marker before asserting (the child may legitimately never start in time), or
assert the timeout path without requiring the child to have written. Suggested: `/sdlc-spec --debt BL-0055`.
