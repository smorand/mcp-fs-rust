---
Id: BL-0148
Title: Bound the web fetch body read into memory
Nature: BUG
Source: class sweep of SPEC-0089 (BL-0142, BL-0143), same idiom outside document extraction
origin: deferred from SPEC-0089
Created: 2026-11-10
Decision: deferred by the SPEC-0089 author under the user's blanket authorization of 2026-11-10 ("spec BL-0142 and BL-0143 together", every decision authorized); stays exploitable on a server with the optional web tools enabled until this entry is specified and shipped
---

## What
The optional web tools read a remote page or file whole into memory: the search answer with `resp.text()` and the fetch with `resp.bytes()`. A remote server answering gigabytes (or an endless chunked body until the request timeout) costs the server that memory.

## Why
Untrusted bytes read with no bound (CWE-400). Off by default, so only deployments enabling the web family are exposed.

## Evidence
`crates/core/src/tools/web.rs:288` (`resp.text()`), `:401` (`resp.bytes()`); no size constant in `tools/web.rs` (`rg -n 'max_bytes|MAX_|limit' crates/core/src/tools/web.rs` finds none).

## Notes
SPEC-0089's capped converter body read is the shape to reuse.
