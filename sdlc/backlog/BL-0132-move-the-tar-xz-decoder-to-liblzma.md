---
Id: BL-0132
Title: tar.xz and txz still use lzma-rs, which holds a whole block in memory and refuses BCJ and SHA-256
Nature: DEBT
Source: SPEC-0082 erratum 1
Created: 2026-11-05
---

## What
Once a bare `.xz` file is decoded with `liblzma`, `.tar.xz` and `.txz` still go through `lzma-rs` 0.3: a whole block is decoded into a `Vec` before anything is written, data after the first stream fails, and a BCJ filter or a SHA-256 check is refused as corrupt.

## Why
Two xz decoders in the tree, and the tar family has no decoded size cap (BL-0124).

## Evidence
`crates/core/src/tools/archive.rs` (xz branch of the tar decoder), `lzma-rs-0.3.0/src/decode/xz.rs:86-90`, `:226-285`.

## Notes
Proposal: after SPEC-0082 lands, switch the tar xz reader to the same `liblzma` reader and drop `lzma-rs`, together with BL-0124.
