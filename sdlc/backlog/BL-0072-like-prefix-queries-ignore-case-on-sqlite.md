---
Id: BL-0072
Title: Prefix queries built with descendant_pattern ignore ASCII case on SQLite
Nature: BUG
Source: found during /sdlc-implement US-0073 (SPEC-0049), 2026-10-22
Created: 2026-10-22
---

## What
`RelationalMetaStore::descendant_pattern` builds a `LIKE '<path>/%' ESCAPE '\'` pattern. SQLite's `LIKE` is
case insensitive for ASCII by default, so a query for the subtree of `/video` also returns rows of `/Video`.
SPEC-0049's folder read had to add an exact prefix check in Rust to stay correct (test
`spec_0049_us_0073_folders_differing_only_by_case_do_not_mix`: RED with `size 3000, file_count 2` where
`2000` and `1` were expected).

## Evidence
`crates/core/src/storage/meta.rs:574` (`descendant_pattern`) and its other call sites at lines 1941, 2002,
2127, 2238 and 2251 (subtree listing, stale files, subtree removal, rename). No `case_sensitive_like` pragma
and no `COLLATE` anywhere under `crates/core/src/storage` (`rg -n 'case_sensitive_like|COLLATE' crates/core/src/storage`
prints nothing). Not verified for the removal and rename call sites: a subtree delete or rename of `/video`
might also touch `/Video`. PostgreSQL and SQL Server `LIKE` behave differently, so the effect is SQLite only.

## Notes
Proposed: write a failing test per call site (a project holding `/Video/x` and `/video/y`, then remove or rename
`/video`), then either set `PRAGMA case_sensitive_like = ON` for the SQLite connections or filter with an exact
prefix in each call site. Decision: none yet; nobody authorized deferring beyond this entry.
