---
Id: BL-0098
Title: Tag tests need a transport double: 429, hook rejection, https for the registered tool, shallow fetch
Nature: DEBT
Source: deferred from SPEC-0060 (carried from BL-0082, BL-0083, BL-0086, BL-0087, BL-0094)
Created: 2026-10-30
---

## What
A test transport that can answer HTTP 429, run server side hooks, serve https to the registered tool path (which refuses `file://`), and allow a shallow fetch. Then add the tests the file remote cannot carry:
- a rate limited leg that is not a dropped connection (remote delete `e2e_tag_032`, publish all `e2e_tag_022`);
- a real remote rejection of a tag push through `PushFailure::Rejected` -> `tag_push_error` -> `remote_refused_tag` (E2E-085 only exercises `Reference::is_valid_name`);
- a paused merge remote delete through the registered tool `tool_remote_delete_tag`, comparing HEAD and conflict files;
- a clone with `depth` and the tag ancestry filter (`clone_history_tags`);
- the simulated E2E-039 and E2E-077 made real, and the spec example host `github.ibm.com` on the down leg.

## Why
Those legs pass if a wrong error follows the right prefix, and the guard added only in the tool path is unseen by engine level tests.

## Evidence
`crates/core/src/tools/git.rs:25536,26062,27467` (comments: no file remote can answer 429); `crates/core/src/git/remote.rs:997-999` (local transport refuses shallow fetch); `crates/core/src/tools/git.rs:3119`, `crates/core/src/git/remote.rs:256` (tool layer refuses `file://`); `closing_remote` at `git.rs:9688`.
