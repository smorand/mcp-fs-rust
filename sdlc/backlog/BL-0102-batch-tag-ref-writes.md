---
Id: BL-0102
Title: Batch the tag ref writes of the reconcile
Nature: DEBT
Source: deferred from SPEC-0060 (carried from BL-0082)
Created: 2026-10-30
---

## What
`reconcile_incoming_tags` writes tag rows one `set_ref` await at a time: n sequential writes for n tags (2000 tags in `e2e_tag_048_two_thousand_tags`).

## Why
Sequential writes make a large tag receive slow.

## Evidence
`crates/core/src/tools/git_tags.rs:427` area. Measure first.
