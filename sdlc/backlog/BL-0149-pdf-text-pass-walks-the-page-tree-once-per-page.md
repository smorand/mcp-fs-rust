---
Id: BL-0149
Title: The PDF text pass walks the page tree once per page
Nature: BUG
Source: class sweep of SPEC-0089 (BL-0142, BL-0143), out of that spec's class
origin: deferred from SPEC-0089
Created: 2026-11-10
Decision: deferred by the SPEC-0089 author under the user's blanket authorization of 2026-11-10 ("spec BL-0142 and BL-0143 together", every decision authorized); stays exploitable until this entry is specified and shipped
---

## What
`fs.extract_text` on a `.pdf` extracts page by page through `output_doc_page`, which rebuilds the full page map on every page, so the time grows with the square of the page count. A PDF with hundreds of thousands of empty pages, within SPEC-0089's object limit, keeps a worker busy for a very long time.

## Why
Algorithmic complexity on a member uploaded file (CWE-407): CPU, not memory, so outside SPEC-0089's class.

## Evidence
`pdf-extract-0.12.1/src/lib.rs:2323-2333` (`extract_text_from_mem_by_pages` loops on `extract_text_by_page`), `:2389-2396` (`output_doc_page` calls `get_pages` at `:2394`); `lopdf-0.42.0/src/document.rs:524-526` (`get_pages` walks the whole tree).

## Notes
Settle with SPEC-0089's component question (walk the tree once); measure first.
