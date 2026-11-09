---
Id: BL-0134
Title: Bound the decoding of Office document parts read for their text
Nature: BUG
Source: deferred from SPEC-0085 (class sweep occurrence 5)
Created: 2026-11-09
Decision: deferred by the SPEC-0085 author under the user's blanket authorization of 2026-11-09 ("spec all of them, design later"); stays exploitable until this entry is specified and shipped
---

## What
Text extraction of a `.docx`, `.xlsx` or `.pptx` decodes each part it needs (`word/document.xml`, slides, shared strings) whole into memory, with no bound but the size the part declares, so a small document whose part declares and holds gigabytes of text uses that memory.

## Why
Same class as SPEC-0085: sizes declared inside an uploaded compressed container reaching an in memory decode with no bound (CWE-409). SPEC-0085 covers archive extraction only; this reader is a different feature (text extraction, the documentation companion).

## Evidence
`crates/core/src/docs/extract.rs:563-568` (`entry_text`: `by_name` then `read_to_end` into an unbounded `Vec`), called at `:716`, `:871`, `:879`, `:896-897`, `:939`, `:992`, `:1006`, `:1046-1048`. Class: sizes declared inside an uploaded compressed container reaching an in memory decode with no bound (CWE-409).

## Notes
A natural bound is a fixed cap on the text a part may yield, refused with a clear message; the capped reader of `archive_single.rs:92-108` is the existing shape.
