---
Id: BL-0045
Title: A unit test depends on reaching github.com and fails when the network is slow
Nature: DEBT
Source: SPEC-0020 landing, 2026-10-11
Created: 2026-10-11
---

## What
`tools::git::tests::remote_clone_surfaces_a_failure_without_leaking_the_token`
(`crates/core/src/tools/git.rs:8482`) contacts `github.com`; on a slow network it fails with
`remote timeout: host 'github.com' did not respond within 120s` instead of the expected failure.
It passed 3 of 3 in isolation right after.

## Proposal
Point the test at an unroutable local address or a local stub so `make check` never needs the internet.
