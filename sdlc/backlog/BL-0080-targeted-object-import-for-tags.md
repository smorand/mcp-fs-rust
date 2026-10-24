---
Id: BL-0080
Title: Import only the new tag object instead of the whole repository
Nature: DEBT
Source: review of US-0084 (SPEC-0052)
Created: 2026-10-24
---

## What
`git.tag_create` for an annotated tag calls `entry.objects.import_from_repo(&repo)` (`crates/core/src/git/odb.rs:188-228`), which enumerates and reads every object of the repository, under the write lock. Listing also looks up each annotated tag object up to three times (`tag_entry`, `peel_to_commit`, `annotation_details` in `crates/core/src/tools/git_tags.rs`).

## Why
Creation is O(repository size) and listing does constant factor repeated work. The commit path has the same import pattern, so this is consistent today but not cheap on large histories.

## Evidence
- `crates/core/src/tools/git_tags.rs`: `import_from_repo` call in the annotated creation path.
- `crates/core/src/git/odb.rs:188-228`.

## Notes
Add a one object import (`import_object(oid)`) and reuse a single `find_object` per tag in the listing. Measure first.
