---
id: BL-0004
title: Add inline HTML editor page per file, addressed by stable UUID and OAuth-protected
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
A dedicated MCF UI page that renders an inline HTML editor for a given file. The page URL is
generated from the file's UUID, so it is always the same stable link for that file, but still
gated behind OAuth so only authorized users can open it. Imagine the HTML editor shipped inside
mcp-fs itself.

## Why
Lets a user open and edit a specific file through a stable, shareable, authenticated link
instead of going through the MCP tool surface or REST API directly.

## Notes
This project already has an `htmleditor` MCP server referenced elsewhere; check whether this
reuses or wraps it rather than building a new editor. Needs a decision on how the UUID maps to
`mount_id` + path, and whether the editor supports write-back or is read-only.
