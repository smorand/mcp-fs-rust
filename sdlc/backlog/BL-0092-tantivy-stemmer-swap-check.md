---
Id: BL-0092
Title: Check tantivy 0.27 stemming against 0.26 on real text
Nature: DEBT
Source: review of US-0103 (SPEC-0055)
Created: 2026-10-27
---

## What
tantivy 0.27 replaced `rust-stemmers` by `frostem` (Cargo.lock diff of US-0103). The search suite and the 0.26 index fixture are green, but stemming of words they do not cover could differ between the two engines, so the same query could rank documents differently.

## Why
Search results are observable behaviour of the optional search family.

## Evidence
`git diff` of `Cargo.lock` in US-0103: `rust-stemmers` removed, `frostem` added.

## Notes
Run the same corpus and queries (several languages the product supports) through both versions and compare the top results.
