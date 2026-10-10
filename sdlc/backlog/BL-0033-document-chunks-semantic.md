---
Id: BL-0033
Title: Document chunks: list and get, semantic chunking
Nature: FEAT
Source: deferred from SPEC-0019
Created: 2026-10-09
---

## What
Expose the chunks of a document (list, get by id, position, source artifacts) and replace fixed character windows with semantic splitting. Search hits point at a chunk id.

## Why
Chunks exist only inside the search index today (SPEC-0008 FR-012) and cannot be read or cited.

## Notes
Split of BL-0009 (document RAG metadata). Depends on SPEC-0019 (document artifacts).
