---
id: BL-0002
title: Add max MB quota per project
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
Let a project be configured with a maximum storage size in MB. Writes that would push the
project's total blob usage past that cap are rejected.

## Why
Without a per-project cap, one project can consume unbounded storage and there is no way to
isolate tenants from each other's growth.

## Notes
Needs a decision on what counts toward the quota (unique blob bytes vs referenced bytes vs
including trash), and how it composes with the existing `safety.rs` write quota mechanism.
