---
Id: BL-0041
Title: Size sorted disk usage breakdown of a project
Nature: FEAT
Source: deferred from SPEC-0020
Created: 2026-10-11
---

## What
Let a platform admin or the project owner read a project's files sorted by size, capped and
flagged when truncated (SPEC-0010 FR-NEW-007, never built).

## Why
Once writes are refused over the limit (SPEC-0020), the owner needs to find what to delete.
