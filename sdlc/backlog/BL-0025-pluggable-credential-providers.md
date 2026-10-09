---
Id: BL-0025
Title: Pluggable credential providers for git transport
Nature: FEAT
Source: specs/BACKLOG.md BL-011 (migrated), suggested by SPEC-0011 (github-enterprise-and-token-store, legacy id) DEC-034
Created: 2026-09-21
---

## What
A `CredentialProvider` trait so a provider can supply a credential shape other than
`oauth2:<token>`.

## Why
Every provider currently uses `git2::Cred::userpass_plaintext("oauth2", &t)`, which works for
GitHub and GitLab (including their enterprise deployments) but not for providers with a different
credential convention, such as Azure DevOps ([[BL-0026]]).

## Evidence
`crates/mcp-fs/src/tools/git.rs:1011-1013` at the time of writing.

## Notes
Rejected as a premature abstraction at the original interview: a trait with four implementations
that all return the identical credential is indirection with no current payer. Becomes worthwhile
the moment a second credential shape genuinely exists.
