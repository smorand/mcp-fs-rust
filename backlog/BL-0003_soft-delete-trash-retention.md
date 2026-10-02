---
id: BL-0003
title: Add soft-delete trash with MCP/GUI listing, recovery and configurable retention
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
Deleting a file soft-deletes it into a trash instead of purging it immediately. Expose MCP tools
and a GUI screen to list trashed files and recover them. Trashed files are kept for a maximum of
X days, with X configured per project at creation time or from an administration panel.

## Why
Today a delete is final; users have no way to recover an accidental delete, and there is no
visibility into what has been removed.

## Notes
The codebase already has a `safety.rs` trash mechanism; this entry is about exposing it
(MCP tools + GUI) and adding a per-project configurable retention window, not building trash
from scratch. Needs a decision on who can see/recover another person's trashed files (ACL).
