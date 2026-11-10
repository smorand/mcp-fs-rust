---
Id: BL-0144
Title: A damaged Office document part is read as absent instead of refused
Nature: FEAT
Source: found while specifying SPEC-0088 (BL-0134)
origin: deferred from SPEC-0088
Created: 2026-11-10
Decision: deferred by the SPEC-0088 author under the user's blanket authorization of 2026-11-10; SPEC-0088 only turns a part decoding past its declared size into a refusal
---

## What
When a part of a `.docx`, `.xlsx` or `.pptx` cannot be decoded for any reason other than its size (damaged compressed data, a checksum mismatch, an encrypted part), text extraction treats it as if it were not there: a Word document answers `docx: word/document.xml is missing`, a damaged sheet or slide is silently left out and the extraction succeeds with less text. The member is not told the document is damaged.

## Why
Misleading answer, not a security issue. Kept out of SPEC-0088 (YAGNI: memory bound only).

## Evidence
`crates/core/src/docs/extract.rs:563-567` (`by_name(name).ok()?` and `read_to_end(...).ok()?` turn every error into `None`), callers `:716-717` (Word: missing message), `:871`, `:992` (slide or sheet skipped with `continue`).

## Notes
Decide per format whether a damaged part refuses the whole extraction or is reported in the note.
