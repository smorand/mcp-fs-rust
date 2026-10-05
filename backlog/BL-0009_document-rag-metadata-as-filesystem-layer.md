---
id: BL-0009
title: Model extracted document RAG output as filesystem metadata, not a side folder
kind: idea
suggested_command: /spec-feat
created: 2026-10-05
origin: user
---

## What
Today document extraction (`doc_service`, `fs.extract_text`, `fs.documentize`) produces a
converted document, but RAG-style byproducts (tables, image descriptions, chunking, summaries,
concept tags, embeddings) have no first-class home. The idea: treat all of this as filesystem
*metadata* attached to the document, not a parallel folder of loose files. `document.md` keeps the
text plus table captions/links and image captions/links; tables live as CSV and/or markdown,
images stay as separate image files, chunking follows a Science Next-style semantic splitting
algorithm with long-chunk summaries and concept extraction against a configured dictionary. Search
combines BM25 and vector embeddings with a reranker when both are active. Every extraction level
(table format, inline vs aside, image description depth vs base64 inline, chunking/summary depth,
concept list, embedding model) is a per-project configuration choice at project creation time.

## Why
Without this, RAG extraction output is either thrown away or scattered as ad hoc files with no
queryable structure, and there is no consistent way to tune extraction depth per project or to
search across text + tables + images + chunks with one ranked result set.

## Notes
This is specification-sized, not a backlog-sized idea (Invariant 3): it touches project config
schema, the metadata/node model, `docs/` extraction pipeline, chunking + embedding + concept
extraction, and search (BM25 + vector + reranking). `/spec-feat` should decompose it, likely into
multiple specs (metadata schema, chunking/concept pipeline, search/reranking), rather than one.
Relates to existing `doc_service` (off by default) and `.agent_docs/search.md` plumbing already in
the codebase — worth reading both before specifying.
