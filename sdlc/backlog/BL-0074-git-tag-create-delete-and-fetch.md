---
Id: BL-0074
Title: Create, delete and fetch git tags
Nature: FEAT
Source: BL-0027 review (.agent_docs/git-divergences.md, D15)
Created: 2026-10-24
---

## What
Let a user create and delete tags and see remote tags after a fetch or pull. Today `git.tags`
only lists, no tool creates or deletes a tag, and fetch never follows or imports tags.

## Why
Marking a release version is impossible, and tags pushed by others never appear. Ranked first
in the divergence register.

## Evidence
- `crates/core/src/mcp/server.rs:3290` `git.tags` is the only tag tool.
- `crates/core/src/git/remote.rs:745` `download_tags(AutotagOption::None)`, FR-NEW-062 kept fetch
  from touching refs outside the asked branch; that requirement is the one to revisit.

## Notes
Decide in spec: annotated versus lightweight, whether push sends tags, and how this reconciles
with FR-NEW-062. Origin: deferred from BL-0027.
