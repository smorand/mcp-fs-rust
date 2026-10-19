---
Id: BL-0027
Title: Review the consolidated register of divergences from standard git
Nature: FEAT
Source: specs/BACKLOG.md BL-013 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id), user request during Round 2b
Created: 2026-09-21
---

## What
A consolidated register of the ways this server's git behaviour still differs from ordinary git, so
the set can be reviewed together: no index (the volume **is** the working tree), whole-volume
commits instead of staged adds, pull refuses a dirty tree, HTTPS-only transport, credentials-in-URL
rejected, unknown host errors unless declared, one credential shape (token as password with the
`oauth2` username), no token refresh/rotation, no submodules, and a clone/pull atomicity asymmetry
(clone tolerates per-file failure and reports `skipped`; pull is atomic).

Closed since this entry was written (audit 2026-10-19, SPEC-0012): per-file conflict resolution,
rebase, cherry-pick, revert, stash, `git.reset` (hard covers discard), force push with lease,
`remote_branch` on push, remote add/list/remove and remotes beyond `origin`, pull requests.

## Why
Each divergence is individually justified, but nobody has reviewed the set as a whole, and
together they define what "git support" means for this server.

## Evidence
See the migrated github-enterprise-and-token-store and full-git-dev-process design.md files for
the originating decisions and FR citations behind each row.

## Notes
The review question is whether the set is coherent, and which gaps matter enough to close. That is
a product conversation, not a defect.
