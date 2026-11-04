---
Id: BL-0110
Title: fs.extract_archive supports .rar
Nature: FEATURE
Source: user request
Created: 2026-11-04
---

## What
Add `.rar` to the `SUFFIXES` table of `tools/archive.rs` so `fs.extract_archive` (and `POST /api/fs/{mount_id}/extract-archive`) extracts RAR archives, with the same pre scan (entry type, zip slip, collisions, one quota charge) and the `ERR_PASSWORD_REQUIRED` retry.

## Why
RAR is a standard archive format. Today it returns `ERR_NOT_SUPPORTED` (test e2e_new_022 pins that).

## Evidence
`crates/core/src/tools/archive.rs` suffix table lists zip, tar family and 7z only.

## Notes
Check the licence of the Rust RAR crate against `deny.toml` (unrar is a non free licence, bindings to a C library). Update e2e_new_022, TOOL_CONTRACT.txt and the golden if descriptions change.
