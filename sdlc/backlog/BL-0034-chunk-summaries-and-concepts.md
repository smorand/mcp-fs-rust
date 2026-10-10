---
Id: BL-0034
Title: Chunk enrichment: summaries of long chunks and concept tagging
Nature: FEAT
Source: deferred from SPEC-0019
Created: 2026-10-09
---

## What
Summarize long chunks and tag each chunk with concepts from a per project dictionary.

## Why
Needed for RAG quality; carries the only new LLM call path, so it is kept out of the artifact and chunk specs.

## Notes
Split of BL-0009 (document RAG metadata). Depends on SPEC-0019 (document artifacts).
