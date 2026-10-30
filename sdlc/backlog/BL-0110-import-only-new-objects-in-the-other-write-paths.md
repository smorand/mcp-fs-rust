---
Id: BL-0110
Title: The other git write paths import the whole repository on every call
Nature: DEBT
Source: deferred from SPEC-0061 (from BL-0080 notes)
Created: 2026-10-30
---

## What
`BlobObjectDb::import_from_repo` walks every object of the libgit2 repository and probes the blob store for each, on every call. After SPEC-0061 `git.tag_create` no longer uses it, but 11 production call sites still do: `crates/core/src/git/http/mod.rs:572`; `crates/core/src/tools/git.rs:2323,3324,3918,4262,4891,4928,5632,6803,6935,7178`. Commit, merge, rebase and the like create a bounded set of objects; a targeted import (the new `import_object`, or an import of the commits written) would make them O(new objects). Clone and fetch legitimately import many objects and may keep the whole walk.

## Why
Measured on `git.tag_create` before SPEC-0061: 364 to 709 ms at 5 000 objects and 1.6 to 2.9 s at 20 000, against 23 to 35 ms and about 100 ms for the lightweight tag. The same shape is expected on the commit path. Measure first.

## Evidence
`crates/core/src/git/odb.rs:188-228`; call sites listed above; SPEC-0061 Section 2 baseline.
