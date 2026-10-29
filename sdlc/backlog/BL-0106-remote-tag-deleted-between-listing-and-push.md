---
Id: BL-0106
Title: Wording for a remote tag deleted between the listing and the push
Nature: FEAT
Source: deferred from SPEC-0060 (carried from BL-0083 (US-0090))
Created: 2026-10-30
---

## What
A remote tag deleted between the listing and the push is refused with `has moved to 0000000`; there is no separate wording for a deleted tag. Decide the text.

## Why
The message names a zero id instead of saying the tag is gone.

## Evidence
`crates/core/src/git/remote.rs` `push_tag_to_remote`, `tag_push_error`.
