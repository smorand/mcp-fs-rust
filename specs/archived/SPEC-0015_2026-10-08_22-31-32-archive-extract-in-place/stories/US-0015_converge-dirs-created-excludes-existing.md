---
Parent Spec: SPEC-0015
Spec ID: SPEC-0015
Status: ready
Depends On: US-0012
min_tier: 2
Files touched: crates/core/src/tools/archive.rs
---
# US-0015 [converge] dirs_created excludes pre-existing directory entries

Converge round 1 gap: FR-NEW-023 has no test proving a directory entry that already exists (other
than the destination root) is excluded from `dirs_created`. Test: destination `/uploads/report`
and `/uploads/report/sub` pre-exist, archive has directory entries `sub/` and `new/` plus files,
`overwrite=true`; assert `dirs_created == 1`. Also rename the audit test that reuses the id
E2E-NEW-025 (FR-NEW-004 owns it) to a non colliding name.
