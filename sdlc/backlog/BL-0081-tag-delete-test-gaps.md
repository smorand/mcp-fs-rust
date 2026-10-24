---
Id: BL-0081
Title: Tag delete tests: annotated tag path and lock proof
Nature: DEBT
Source: review of US-0085 (SPEC-0052)
Created: 2026-10-24
---

## What
- No test deletes an annotated tag, so the peeled commit id reported by `git.tag_delete` (result `sha`, audit line) is unexercised (`crates/core/src/tools/git_tags.rs`, `tag_delete`).
- E2E-032 and E2E-066 use `tokio::join!` over two blocking calls; a check done outside `entry.write_lock` could pass by luck.

## Why
The behavior is specified (FR-NEW-005, FR-NEW-012, FR-NEW-016) but the tests would not catch a regression in these two spots.

## Evidence
Review of `git diff` for US-0085: tests `e2e_tag_026`, `e2e_tag_032`, `e2e_tag_066` in `crates/core/src/tools/git.rs`.

## Notes
Add a delete of an annotated tag asserting `sha` is the commit and the audit line carries it. For the lock, hold the write lock in the test and assert the second delete waits, or loop the race many times.
