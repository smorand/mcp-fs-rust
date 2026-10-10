---
Id: BL-0039
Title: SPEC-0010 reads implemented but the project storage quota is neither enforced nor readable
Nature: BUG
Source: found while designing SPEC-0019 (2026-10-10)
Created: 2026-10-10
---

## What
`sdlc/specs/SPEC-0010-project-storage-quota/` is marked implemented, yet its own as-built table
records FR-NEW-005 to FR-NEW-012 as not implemented: no write is checked against the stored limit
and no tool reads a project's usage.

## Evidence
- `sdlc/specs/SPEC-0010-project-storage-quota/spec.md:129-134` (Not implemented rows).
- `crates/core/src/storage/meta.rs:906-975`: `put_file` takes no quota and compares nothing.
- Only the session write quota is enforced: `crates/core/src/safety.rs:130-136`.

## Why
Operators who set `admin.set_project_quota` believe a limit protects the project; nothing does.
SPEC-0019 charges artifacts to the session quota only because of this gap.

## Notes
Route through `/sdlc-analysis` (authority violated: SPEC-0010 says implemented).
