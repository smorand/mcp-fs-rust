---
Id: BL-0024
Title: Add git.discard_changes
Nature: FEAT
Source: specs/BACKLOG.md BL-010 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-025
Created: 2026-09-21
---

## What
Reset a volume's files to its current branch tip: overwrite modified files, restore deleted ones,
remove added ones.

## Why
`git.remote_pull` refuses a dirty volume and instructs the caller to commit or discard; committing
works (divergence is resolvable by merge), but discarding has no single-call implementation.

## Evidence
The nearest tool is `git.checkout_file`, which restores one file per call and cannot remove a file
the volume added (at the time of writing, `crates/mcp-fs/src/tools/git.rs:222`).

## Notes
Once `on_conflict` merge existed, committing became a working escape, so discard stopped being the
only way out of a dirty volume. Still worth having.
