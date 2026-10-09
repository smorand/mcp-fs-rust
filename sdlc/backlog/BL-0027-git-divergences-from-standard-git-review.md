---
Id: BL-0027
Title: Review the consolidated register of divergences from standard git
Nature: FEAT
Source: specs/BACKLOG.md BL-013 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id), user request during Round 2b
Created: 2026-09-21
---

## What
A consolidated register of every way this server's git behaviour differs from ordinary git, so
the set can be reviewed together rather than piecemeal: no index (the volume **is** the working
tree), whole-volume commits instead of staged adds, pull refuses a dirty tree (no stash), no
single-call discard ([[BL-0024]]), one global ours/theirs merge instead of real conflict
resolution ([[BL-0021]]), no rebase/cherry-pick/revert/stash, one branch same-name push refspecs
only ([[BL-0023]]), force push refused outright ([[BL-0018]]), exactly one remote named `origin`
([[BL-0022]]), HTTPS-only transport, credentials-in-URL rejected, unknown host errors unless
declared, one credential shape ([[BL-0025]]), no token refresh/rotation, no submodules, and a
clone/pull atomicity asymmetry (clone tolerates per-file failure and reports `skipped`; pull is
atomic).

## Why
Each divergence is individually justified, but nobody has reviewed the set as a whole, and
together they define what "git support" means for this server.

## Evidence
See the migrated github-enterprise-and-token-store and full-git-dev-process design.md files for
the originating decisions and FR citations behind each row.

## Notes
The review question is whether the set is coherent, and which gaps matter enough to close. That is
a product conversation, not a defect.
