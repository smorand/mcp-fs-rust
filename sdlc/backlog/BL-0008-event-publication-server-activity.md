---
id: BL-0008
title: Add event publication of what is going on into the servers
kind: idea
suggested_command: /spec-feat
created: 2026-10-05
origin: user
---

## What
Publish events describing what is happening on the server (filesystem operations, git
operations, admin actions, etc.) so external consumers can observe server activity.

## Why
Lets other systems react to or audit what the server is doing, instead of polling or
scraping logs.

## Notes
Needs scoping during spec: which events, which transport (webhook, message queue,
SSE/websocket, log sink), and whether this is per-project or global.
