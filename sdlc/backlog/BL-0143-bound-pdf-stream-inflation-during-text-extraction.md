---
Id: BL-0143
Title: Bound PDF stream inflation during text extraction
Nature: BUG
Source: class sweep of SPEC-0088 (BL-0134), named "not searched" by SPEC-0085 too
origin: deferred from SPEC-0088
Created: 2026-11-10
Decision: deferred by the SPEC-0088 author under the user's blanket authorization of 2026-11-10 ("spec BL-0134", every decision authorized); stays exploitable until this entry is specified and shipped
---

## What
`fs.extract_text` on a `.pdf` hands the whole file to the PDF library, which inflates every compressed content stream into memory with no bound set by the server. A small PDF whose content stream inflates to gigabytes uses that memory before anything is refused.

## Why
Same class as SPEC-0088 (a compressed container uploaded by a member reaching an in memory decode with no bound, CWE-409), but the decoder lives inside a third party library, not in the server's Office part reader, so the bound cannot be the same mechanism.

## Evidence
`crates/core/src/docs/extract.rs:497` (`pdf_extract::extract_text_from_mem_by_pages(data)`), `:523` (`pdf_extract::Document::load_mem(data)` for the title). No size argument is passed; the only control is `catch_unwind` against panics (`:495-499`).

## Notes
Options to weigh in the spec: a decoded size budget if the library exposes one, a worker with a memory limit, or a pre scan of declared stream lengths. Decide which once the library's capabilities are read.
