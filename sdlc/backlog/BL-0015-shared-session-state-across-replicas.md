---
Id: BL-0015
Title: Share session/write-quota state across replicas (horizontal scaling)
Nature: FEAT
Source: specs/BACKLOG.md BL-003 (migrated), user request 2026-09-21, suggested by SPEC-0002 (platform-foundation) retro-spec, TBD-001
Created: 2026-09-18
---

## What
Make the write quota and the read-before-write guard shared rather than per process, so that
horizontal scaling does not silently change their semantics.

## Why
With more than one replica, a caller's quota is effectively multiplied by the replica count and
the read guard can be satisfied on one replica and enforced on another.

## Evidence
Session state is an in-memory map keyed by `(person, project_id)`
(`crates/mcp-fs/src/safety.rs:2-4,79-86` at the time of writing).

## Notes
This is a product decision about the intended deployment topology, not a defect in the current
single-process design. See also [[BL-0016]] (the wider horizontal-scaling picture) and
[[BL-0017]] (the strategy comparison).
