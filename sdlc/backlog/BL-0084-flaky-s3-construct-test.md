---
Id: BL-0084
Title: Flaky test storage::blob::s3::tests::constructs_with_path_style_and_bucket_name
Nature: BUG
Source: seen during US-0089 (SPEC-0052)
Created: 2026-10-24
---

## What
In one full `cargo test --workspace --no-fail-fast` run the unit test `storage::blob::s3::tests::constructs_with_path_style_and_bucket_name` was reported "running for over 60 seconds" and FAILED; the lib suite took 315 s. Run alone it passes in 0.4 to 1.1 s, three times in a row.

## Why
A test that fails only under a full run is a false red for `make check`. The likely cause is an AWS credential or region provider probe (network metadata lookup) blocking while the suite loads the machine.

## Evidence
`.implement/US-0089-verify.log` (`has been running for over 60 seconds`, then `FAILED`); the implementer's run and later runs of the same chain had 0 lib failures.

## Notes
Needs `/sdlc-analysis`: confirm the credential chain probe, then give the test an explicit static credentials provider or disable IMDS in the test.
