---
Id: BL-0129
Title: target/debug grew to 186 GB and filled the disk during a run
Nature: DEBT
Source: found while running US-0171
Created: 2026-11-05
---

## What
`cargo test --workspace` failed with `ld: write() failed, errno=28 (No space left on device)`. `target/` held 186 GB (`target/debug/deps` 155 GB in 307 980 files, `target/debug/incremental` 29 GB), mostly stale artifacts of many feature and profile combinations built on branch switches.

## Why
Every story run rebuilds under several feature sets (`--all-features` clippy, default tests, `--features rar`), and the stale hash variants pile up until the 1.9 TB disk is full.

## Evidence
`df -h /` showed 287 MB free, 100 percent used; `du -sh target/debug/*`; `cargo clean --profile dev` removed 333 014 files and freed 186 GB.

## Notes
Proposal: a `make clean-stale` target (or `cargo sweep --time 7` in `make check`) and a line in `.agent_docs/testing.md`. A shared `CARGO_TARGET_DIR` per worktree would help too.
