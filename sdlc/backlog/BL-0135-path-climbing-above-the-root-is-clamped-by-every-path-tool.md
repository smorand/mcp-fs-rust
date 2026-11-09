---
Id: BL-0135
Title: A path climbing above the project root is clamped to the root by every path taking tool instead of refused
Nature: FEAT
Source: found while specifying SPEC-0086 (BL-0127)
origin: deferred from SPEC-0086
Created: 2026-11-09
---

## What
Every tool and REST route that takes a project path passes it through `SafetyManager::normalize_path`, which never refuses a climb: a relative path is prefixed with `/` before `PosixPath::normpath`, and `normpath` drops a `..` at the root of an absolute path, so the `starts_with("/..")` refusal never fires on a climb; it fires instead on a legitimate name starting with two dots (`..out` becomes `/..out` and is refused). `fs.write` with `path: "../../x"` writes `/x`; `/a/../../b` reads or writes `/b`. Only `fs.export_zip` refuses a climb (its own walk), and SPEC-0086 makes `fs.extract_archive`'s `destination` refuse it.

## Why
The same mistyped path is refused by two tools and silently redirected to the project root by all the others. The message `path escapes the volume root: <path>` exists for this case and is never emitted by the shared check.

## Evidence
`crates/core/src/safety.rs:99-111` (`normalize_path`; the check at `:104` never fires on a climb, only on a name such as `..out`), `crates/core/src/util/posix.rs:39-67` (`normpath` drops a root `..`), tests pinning the clamp: `crates/core/src/safety.rs:263-265`, `crates/core/src/util/posix.rs:115-118`; callers: `crates/core/src/tools/mod.rs:147`, `:153`, `crates/core/src/core/fs_ops.rs:167`, `:1858`, `:1890`, `crates/core/src/mcp/server.rs:190`, `crates/core/src/api/dataplane.rs:198`, `crates/core/src/tools/diff_files.rs:64-65`, `crates/core/src/tools/git.rs:1561`, `:1600`, `:5336`; the refusing exception: `crates/core/src/tools/export.rs:119-126` (`ensure_no_escape`).

## Notes
Decision needed: refuse a climb in every path parameter (a behavior change for most tools and both doors, so a spec, and the pinning tests above change), or document the clamp as the rule outside `fs.export_zip` and `fs.extract_archive`. Not changed by SPEC-0086.
