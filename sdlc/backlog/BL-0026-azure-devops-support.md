---
Id: BL-0026
Title: Support Azure DevOps as a git provider
Nature: FEAT
Source: specs/BACKLOG.md BL-012 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-002
Created: 2026-09-21
---

## What
Support Azure DevOps as a provider.

## Why
The provider set is currently `github`, `gitlab`, `generic`, `anonymous`. Azure DevOps suffers the
same substring-detection defect that provider resolution was fixed to avoid, and would work today
as a `generic` host if its credential convention matched.

## Evidence
See the migrated github-enterprise-and-token-store design.md, Legacy mapping table, for DEC-002.

## Notes
Azure DevOps expects the PAT as the password with an arbitrary or empty username, not the literal
`oauth2`. Supporting it properly needs [[BL-0025]] first.
