---
Id: BL-0117
Title: No limit on the number of have lines in one git-upload-pack request
Nature: DEBT
Source: found while designing SPEC-0067 (DEC-011)
Created: 2026-10-31
---

## What
The upload-pack request is read in full and every `have` line is kept (`crates/core/src/git/http/mod.rs:398-418`). After SPEC-0067 each held id costs a lookup in the object store, and each acknowledged one a reachability check, so a very large list costs work proportional to its length. Only the request body limit bounds it today.

## Why
An operator has no per-request bound on negotiation work. A cap changes what a client observes, so it was not added inside SPEC-0067.

## Evidence
`crates/core/src/git/http/mod.rs:398-418`; the body limit layer is applied at the route (`DefaultBodyLimit`, same file, route setup near line 96).

## Notes
Measure the cost per 1 000 haves after US-0140 of SPEC-0067, then choose a cap and decide how a client sees it (answer with the lines read so far, like a cut request, is the least visible).
