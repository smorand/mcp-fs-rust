---
Id: BL-0153
Title: No total limit across the cross reference streams of a PDF /Prev chain
Nature: DEBT
Source: US-0213 review (skeptical read only review of the SPEC-0089 cross reference guard)
origin: found while reviewing US-0213
Created: 2026-11-10
Decision: filed, not implemented in US-0213 by the user's instruction; each stream is bounded, the sum is not
---

## What
The cross reference guard measures each cross reference stream of the main section, its `/Prev` chain and the `XRefStm` of a hybrid file against the stream limit, one at a time. Nothing adds those measures together, so a chain of N streams each just under 104 857 600 decoded bytes is accepted, and lopdf then inflates every one of them while it reads the cross reference: N times the stream limit decoded in total, before any object stream or page is read.

## Why
Memory and CPU amplification across sections (CWE-400). Each inflation is freed before the next, so the peak stays near one stream, but the total work grows with the chain length, which the file chooses (the seen set only stops a loop, not a long chain).

## Evidence
`crates/core/src/docs/pdf_bound.rs:397` (`measure(&stream, budget, Scope::Guard)?` per section, the result not added to any total), `xref_guard` in the same file (the `/Prev` walk), `lopdf-0.42.0/src/reader.rs:765-791` (lopdf inflates every section of the chain).

## Notes
Proposal: add the guard measurements to the load total of `PdfBudget` (the figure the object streams already charge), so the chain and the object streams share one limit. Measure the change against the existing SPEC-0089 tests: a legitimate incrementally saved file has a few small streams, far from the limit.
