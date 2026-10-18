---
Id: BL-0065
Title: spec_0024_bt_020_other_projects_untouched_tantivy is flaky under full suite load
Nature: BUG (test only)
Source: US-0045 make check, 2026-10-18
Created: 2026-10-18
Severity: Low
---

## What
`others_untouched` (`crates/core/src/tools/project_removal_e2e.rs:764-782`) takes its baseline with `settled_snapshot` ("two reads a beat apart agree") while the documentize hooks still index in the background. Under full suite load the baseline caught `bm25_docs: 1` and the later read `bm25_docs: 2`, so the assertion `atlas2 changed` failed although nothing touched atlas2.

## Evidence
`.implement/US-0045-green.log`: `left: ... "bm25_docs": Number(2) ...` vs `right: ... "bm25_docs": Number(1) ...` at `project_removal_e2e.rs:780`. Isolated: 3/3 green on the US-0045 branch and 3/3 on main (`2688c72` introduced it); the rerun of `make check` was green (2788/0). Unrelated to US-0045 (blob put only).

## Proposal
Wait for the background indexing to finish deterministically (an explicit flush or awaiting the index task) before the baseline, instead of a two read heuristic.
