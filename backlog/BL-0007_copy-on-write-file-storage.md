---
id: BL-0007
title: Add copy-on-write for files to prevent redundant storage of same file
kind: debt
suggested_command: /spec-debt
created: 2026-10-05
origin: user
---

## What
When a file is copied, avoid duplicating its stored bytes; only materialize a separate
copy of the content when one of the copies is actually written to.

## Why
Prevents redundant storage of identical file content across copies, saving blob storage
space and copy time.

## Notes
Storage is already content-addressed (write puts a blob by hash, copy is metadata only,
delete GCs at refcount 0 — see `storage/meta.rs` and AGENTS.md). So byte-level dedup across
identical content already exists today; what this entry may actually be asking for is
unclear and needs scoping during spec: is it "verify/document existing dedup", "extend
dedup to partial/chunked content", or something else entirely.
