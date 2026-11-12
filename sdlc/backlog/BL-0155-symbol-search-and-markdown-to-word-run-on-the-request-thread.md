---
Id: BL-0155
Title: Symbol search and Markdown to Word rendering still run on the request threads
Nature: DEBT
Source: SPEC-0091 follow-up, validated by the owner on 2026-11-12
Created: 2026-11-12
---

## What
SPEC-0091 moved the six document readers (docx, pptx, xlsx, pdf, html, csv) to blocking threads behind a fair ceiling. Other CPU heavy work still runs directly on the async request threads: `fs.find_definition` and `fs.find_references` parse each source file with tree-sitter (`fs_ops.rs` around :1680 and :1699, `docs/symbols.rs`), and the Markdown to Word rendering of `fs.write_docx`/the document writers runs inline (`fs_ops.rs` around :1821). The OOXML header check also reads and scans the whole document on the request thread, on the cached path (`docs/extract.rs`) and on writes (`fs_ops.rs` around :733).

## Why
While such a call runs, the thread cannot serve other requests: the project rule (`AGENTS.md`) is to never block a request thread. The work is bounded by file size, so stalls are short for normal files; they matter for very large sources or documents under load.

## Evidence
SPEC-0091 section 3.2 and its measurements (one worker thread: a trivial request waited 5.5 s behind one heavy PDF extraction); SPEC-0091 side findings; `crates/core/src/docs/extract.rs` (`DecodePlaces`, `read_apart`, the fair semaphore sized by the runtime worker count).

## Notes
Proposal: reuse the SPEC-0091 mechanism (the same fair ceiling and `spawn_blocking`, the test probes of `tools/offload_e2e.rs`) for the three jobs above, keeping every answer, limit and message unchanged. Small increment, comparable to the first story of SPEC-0091.
