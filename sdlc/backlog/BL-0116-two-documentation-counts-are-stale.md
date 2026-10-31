---
Id: BL-0116
Title: .agent_docs/api.md route count and git.md "no tags" are stale
Nature: DEBT
Source: found while planning doc-drift-fixes (BL-0030, BL-0078)
Created: 2026-10-31
---

## What
1. `.agent_docs/api.md:6` says the plane has 38 routes; `REST_ROUTES` in `crates/core/src/api/dataplane.rs` lists 49 (admin routes such as `disk-usage` and `folder-usage` included).
2. `.agent_docs/git.md` (the paragraph after the parity pointer) lists "no tags" among the divergences from standard git; tags are supported since SPEC-0052 (`git-divergences.md` records D15 closed).
3. Inline comments of `dataplane.rs` (`// bytes plane` at the route table and handlers for `mkdir`, `delete`, `move`, `upload`) still use the old two family wording that the module header no longer uses.

## Why
Readers get wrong numbers and a wrong capability statement.

## Evidence
`.agent_docs/api.md:6`; `rg -c '^    \("' ` over `REST_ROUTES` gives 49; `.agent_docs/git.md:779`; `.agent_docs/git-divergences.md:3`.

## Notes
Prefer wording that does not carry a count, or a test that derives it.
