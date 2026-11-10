---
Id: BL-0151
Title: PDF parsed structures can be much larger than the bytes they come from
Nature: BUG
Source: /sdlc-design of SPEC-0089, out of that spec's definitions
origin: found while designing SPEC-0089
Created: 2026-11-10
Decision: filed by the SPEC-0089 design under the user's authorization of the unattended run; not required by any requirement of SPEC-0089 (its held figures count decoded stream data and extracted content, not parsed structures)
---

## What
The PDF reader turns bytes into structures: content streams into `Operation` lists (`Content::decode`), cross reference streams into entry maps, object streams into the object table. A content stream of 104 857 600 bytes made of short operators (`q ` repeated) parses into tens of millions of operations, each tens of bytes, so gigabytes are held while the decoded bytes stay under SPEC-0089's limits. The SPEC-0089 walk parses the same operations as the reader (one form's operations cached per page).

## Why
Memory amplification by parsing (CWE-400): the class is the same family as SPEC-0089's, the measure is not (operations and entries, not decoded bytes).

## Evidence
`lopdf-0.42.0/src/content.rs` (`Content::decode` collects every operation), `pdf-extract-0.12.1/src/lib.rs:1582` (decoded per page and per form draw), `lopdf-0.42.0/src/parser_aux.rs:487-560` (`decode_xref_stream`, one entry per row), `src/object_stream.rs:60-90`. SPEC-0089 design, section 7.

## Notes
A bound on the number of operations per page (counted while parsing, or a cheap pre count of tokens) would close it; measure first.
