---
Id: BL-0019
Title: Pull request creation tool
Nature: FEAT
Source: specs/BACKLOG.md BL-005 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-014 discussion
Created: 2026-09-21
---

## What
A tool creating a pull or merge request on the provider, after a push.

## Why
Closes the gap between pushing a branch and opening a reviewable request on the provider.

## Evidence
At the time this was deferred, the provider REST APIs were not called at all; only outbound git
transport traffic existed. Note: the `git.pr_*` surface (6 tools) has since shipped per
`AGENTS.md`'s tool count and `.agent_docs/git.md` — verify during spec work whether this entry is
now stale/superseded before specifying further.

## Notes
Originally deferred: requires per-provider REST clients, a token scope beyond repository
read/write, and a result model that differs between GitHub and GitLab.
