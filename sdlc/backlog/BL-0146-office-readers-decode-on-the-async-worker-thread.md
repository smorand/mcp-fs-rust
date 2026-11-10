---
Id: BL-0146
Title: Office, PDF and HTML text extraction decode on the async worker thread
Nature: DEBT
Source: found while designing SPEC-0088 (BL-0134)
origin: deferred from SPEC-0088
Created: 2026-11-10
Decision: deferred by the SPEC-0088 designer under the user's blanket authorization of 2026-11-10; SPEC-0088 keeps the readers where they are (its test hook relies on it, DEC-008)
---

## What
`Extractor::extract` calls the synchronous readers (`docx`, `pptx`, `xlsx`, `pdf`, `html`, `csv`) directly from the async request path, without `spawn_blocking`. Even within the SPEC-0088 limits, one call can decompress and parse up to 268 435 456 bytes of parts on a tokio worker thread, stalling every other request scheduled on it for the duration.

## Why
Latency and fairness under load, not memory. The project's own rule is "never block a request thread" (AGENTS.md conventions).

## Evidence
`crates/core/src/docs/extract.rs:151-185` (readers called inline), `crates/core/src/core/fs_ops.rs:1739-1748` (awaited from the tool path); the archive tools already use `spawn_blocking` (`crates/core/src/tools/archive.rs`).

## Notes
Moving the readers into `spawn_blocking` must move the SPEC-0088 test hook too (`OFFICE_HELD` is a thread local set on the calling thread): set it on the async side after the blocking task returns, as SPEC-0084 DEC-006 does for archives.
