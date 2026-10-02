---
id: BL-0001
title: Auto-purge unused files and stale projects
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
Automatically remove files that have not been used for X days, and/or automatically remove
entire projects (and all their files) that have not been used for Y days.

## Why
Unused files and abandoned projects accumulate indefinitely in storage today with no retention
policy, wasting blob/metadata space over time.

## Notes
X and Y are separate, independently configurable thresholds (file-level vs project-level).
"Used" needs a definition (last read vs last write vs last access of any kind) before this can
be specified.
