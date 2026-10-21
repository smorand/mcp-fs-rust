---
Id: BL-0070
Title: Totals per folder in the project disk usage breakdown
Nature: FEAT
Source: deferred from SPEC-0047
Created: 2026-10-21
---

## What
Let the owner or admin see a project's size per folder (rolled up, largest first), not only per
file, so a folder of many small files shows up as the real cost.

## Why
The per file breakdown (SPEC-0047) misses a folder holding thousands of small files, none of which
ranks in the top entries.

## Evidence
SPEC-0047 section 3.2 (out of scope); the file list is capped at 500 entries, which hides such a folder.
