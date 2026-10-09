---
Id: BL-0022
Title: Remote management tools (git.remote_add/remove/list) and remotes beyond origin
Nature: FEAT
Source: specs/BACKLOG.md BL-008 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-021
Created: 2026-09-21
---

## What
`git.remote_add`, `git.remote_remove`, `git.remote_list`, exposing the existing storage API, plus
support for remotes other than `origin`.

## Why
A volume created by `git.init` has no `origin` and therefore cannot push, fetch or pull at all;
its only route to a remote is to be cloned instead.

## Evidence
`git_remotes` holds `(volume_id, name, url)`, and `add_remote`/`remove_remote`/`list_remotes` are
implemented and tested in `crates/mcp-fs/src/git/db.rs` (at the time of writing: lines 69-75,
281-305, 429-442) but the only writer is `git.remote_clone`, which records exactly one remote
named `origin`.

## Notes
Excluded at the original interview (DEC-021) to keep the remote surface to one unambiguous target.
