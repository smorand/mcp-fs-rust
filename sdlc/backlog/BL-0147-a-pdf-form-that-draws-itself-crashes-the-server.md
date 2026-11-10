---
Id: BL-0147
Title: A PDF form that draws itself crashes the server
Nature: BUG
Source: class sweep of SPEC-0089 (BL-0142, BL-0143), out of that spec's class
origin: deferred from SPEC-0089
Created: 2026-11-10
Decision: deferred by the SPEC-0089 author under the user's blanket authorization of 2026-11-10 ("spec BL-0142 and BL-0143 together", every decision authorized); stays exploitable until this entry is specified and shipped
---

## What
`fs.extract_text` on a `.pdf` whose form XObject runs `Do` on itself (directly or through other forms) recurses with no depth bound inside the PDF reader. A stack overflow is not a panic, so the `catch_unwind` around the reader does not contain it: the whole server process ends.

## Why
Uncontrolled recursion (CWE-674) on a member uploaded file: one tiny PDF stops the service for every project. SPEC-0089 bounds decoded bytes, not recursion depth; a 20 byte form overflows the stack long before any byte limit.

## Evidence
`pdf-extract-0.12.1/src/lib.rs:1860-1867` (`Do` calls `process_stream` recursively, no depth counter); `crates/core/src/docs/extract.rs:513` (`catch_unwind` only). ASSUMED: the overflow aborts the process (Rust default on stack overflow); the spec reproduces it first.

## Notes
A natural bound is a maximum form nesting depth with a refusal message in the `could not extract <path>: ` family; same component question as SPEC-0089 14.3.
