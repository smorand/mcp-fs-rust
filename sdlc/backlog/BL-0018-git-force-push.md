---
Id: BL-0018
Title: Force push on git.remote_push
Nature: FEAT
Source: specs/BACKLOG.md BL-004 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-013
Created: 2026-09-21
---

## What
Add `force: bool` to `git.remote_push`, allowing a non-fast-forward push to overwrite the remote
ref.

## Why
Push is currently fast-forward only; a non-fast-forward push is refused with a distinct error
stating force is not supported, which blocks legitimate history-rewrite workflows.

## Evidence
See the migrated github-enterprise-and-token-store design.md, Legacy mapping table, for the
original DEC-013 citation.

## Notes
A force push destroys commits on the remote irreversibly. The pusher here is frequently an
autonomous agent, authenticating with a person's personal token, against a repository the person
did not necessarily choose. A mistaken force push is attributed to the person and is not
recoverable from the server side. If implemented, recommendation: `force` defaults to false; it is
rejected outright for any ref matching a configured protected-branch pattern; every forced push
records an audit entry naming the overwritten sha so the prior tip can be recovered from the
reflog; and it is gated by a server-level config flag that is off by default. Deliberate scope
exclusion at the original interview; the recovery paths that make force safe are themselves a body
of work.
