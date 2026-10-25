---
Id: BL-0087
Title: Paused operation tests of remote tag deletion bypass the tool layer
Nature: DEBT
Source: review of US-0094 (SPEC-0053)
Created: 2026-10-25
---

## What
E2E-029, 038 and 039 call the engine helper directly with a `file://` URL, because the tool layer refuses `file://` remotes at credential resolution. A pause guard added only in `tool_remote_delete_tag` would not be seen by them. The paused state check compares only the operation row, not HEAD, refs or conflict files. E2E-031 order B exercises the state check, not the lease; E2E-030 would not catch a missing lock if the lease alone still gave one winner.

## Why
The tests guard the engine, not the registered tool path, for the paused rule (DEC-009).

## Evidence
`e2e_tag_029_remote_delete_during_a_paused_operation`, `e2e_tag_038_*`, `e2e_tag_039_*` in `crates/core/src/tools/git.rs`.

## Notes
Add one paused merge test through the registered tool with an https origin stub, and compare HEAD plus the conflict files.
