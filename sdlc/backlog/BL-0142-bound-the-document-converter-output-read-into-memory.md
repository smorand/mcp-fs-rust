---
Id: BL-0142
Title: Bound the document converter output read into memory
Nature: BUG
Source: class sweep of SPEC-0088 (BL-0134), out of that spec's class
origin: deferred from SPEC-0088
Created: 2026-11-10
Decision: deferred by the SPEC-0088 author under the user's blanket authorization of 2026-11-10 ("spec BL-0134", every decision authorized); stays exploitable on a server with `doc_service` enabled until this entry is specified and shipped
---

## What
When `fs.documentize`, the REST `/upload` or `fs.write_bytes` with `trigger_documentation_service` hand a document to the external converter, the server reads everything the converter returns into memory with no bound: its standard output and standard error, an HTTP answer body, and every file it writes into its output folder. A hostile document that makes the converter emit a huge Markdown (a zip bomb inside a `.docx` the converter unpacks) costs the server that memory, even though the input itself is capped by `doc_service.max_input_bytes`.

## Why
Same family as SPEC-0088 (memory exhaustion from a small uploaded document, CWE-400), but the source is the converter's output, not an Office part decoded by the server's own reader. Off by default (`doc_service.enabled: false`), so only servers that enable it are exposed.

## Evidence
`crates/core/src/docs/service.rs:269-279` (`read_to_end` on the child's stdout and stderr into unbounded `Vec`s, only the timeout bounds it), `:387` (`response.text()` for the API mode), `:494-519` (`std::fs::read` of each output file). Input cap only: `crates/core/src/core/fs_ops.rs:649`, `:712` (`ensure_input_size`).

## Notes
A natural bound is a fixed cap on the converter's total output, refused with a clear message, the capped reader of `crates/core/src/tools/archive_single.rs:92-108` being the existing shape.
