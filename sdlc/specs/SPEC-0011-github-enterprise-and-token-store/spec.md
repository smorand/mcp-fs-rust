# GitHub Enterprise Support and Git Token Store Seeding — Specification Document

> Id: SPEC-0011
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0010 (collision with active SPEC-0010-project-storage-quota, kept its own number); new id SPEC-0011

> Project: mcp-fs (Rust)
> Type: Evolution Specification
> Depth: L

---

## 1. Summary

`mcp-fs` can clone a git repository from a remote, and can obtain an OAuth token through a
device flow. Both capabilities were narrower than they appeared, in ways that blocked a
consuming project.

This specification does three things, all now implemented.

**It makes provider resolution correct.** The provider used to be guessed from the clone URL by
substring match, a guess simultaneously too narrow (an enterprise GitHub host never matched) and
too broad (any URL whose path happened to contain `gitlab` was treated as GitLab). Resolution
now runs through an explicit, boot-validated host-to-provider map with exact hostname matching.

**It opens the token store.** The token store's write path used to be reachable from exactly one
place, the detached device-flow poller. A caller already holding a person's personal access
token had no way to hand it to the server. A seeding tool and a per-person web screen now exist.
Token identity is also per host, so one person can hold distinct tokens for two hosts that share
the same provider.

**It completes the remote surface.** The stored token already authorised writes once consumed as
an ordinary HTTPS password, but the only outbound remote operation that existed was clone.
`git.remote_push`, `git.remote_fetch` and `git.remote_pull` now exist, sharing one credential
pipeline.

The consuming use case was a code-reading agent that needed to cite evidence from
enterprise-hosted repositories; without this work its reach was capped at public repositories on
one public host.

---

## 2. Current State

### 2.1 Functional

`mcp-fs` is a streamable-HTTP MCP server exposing a simulated multi-project filesystem, a REST
data plane, and an optional git HTTP smart server. Server state lives in a relational store
(SQLite, PostgreSQL or SQL Server). Blob bytes live in the blob store.

Git support is gated by `git.enabled` and registers the `git.*` and `git.auth*` tool families.
Git objects are content-addressed in the blob store; a per-project relational index holds
objects, refs and remotes.

All functionality described below is implemented and shipped. See Section 6 for evidence and
Section 10, "Legacy mapping", for the originating decisions and drift entries in `design.md`.

### 2.2 Existing specs

SPEC-0007 (git) owns the pre-existing git tool families, object store and OAuth device flow that
this specification modifies and extends.

### 2.3 Test coverage

Unit and integration tests live in `#[cfg(test)]` modules alongside the code they exercise
(`crates/core/src/tools/git.rs`, `git_auth.rs`, `git/oauth/store.rs`, `git/oauth/persistence.rs`,
`token_screen.rs`, `config.rs`). A separate functional shell suite exercises the remote surface
end to end. The originating story set (19 stories, see design.md "Legacy mapping") reports every
story `done`.

---

## 3. Scope

### 3.1 In scope (delivered)

- An explicit, boot-validated host-to-provider map replacing substring detection.
- Token identity keyed `(person, host)` rather than `(person, provider)`, including the
  persisted primary key on all three relational backends.
- A token-seeding MCP tool, `git.token_set`.
- Token expiry enforced on every remote operation, before any network call.
- An optional `host` parameter on `git.auth`, `git.auth_status` and `git.auth_revoke`.
- `git.remote_clone` persisting the clone URL as remote `origin`.
- `git.remote_push`, `git.remote_fetch`, `git.remote_pull`, sharing one credential pipeline.
- Fast-forward-only push; fast-forward pull; three-way merge with a global `ours`/`theirs`
  conflict resolution strategy.
- A per-person web token screen served from the main server behind the existing identity layer.
- A single module owning host resolution, credential supply and the four remote operations.
- Regeneration of the frozen tool contract.

### 3.2 Out of scope (non-goals, several later backlogged separately)

- Token refresh and rotation.
- Providers beyond `github`, `gitlab`, `generic`, `anonymous`.
- Changing the credential shape (`oauth2:<token>` already authorises read and write).
- SSH transport.
- Runtime editing of the host map.
- Force push, pull request creation, squash merge, interactive merge conflict resolution,
  remote management tools (`git.remote_add`/`remove`/`list`), a remote branch name differing
  from the local one, `git.discard_changes`, a pluggable credential-provider abstraction. These
  are tracked separately in the backlog (see design.md "Legacy mapping" for the owning decision
  ids).
- Migrating pre-existing token rows: they are dropped on upgrade, by decision.

---

## 4. Actors

| Actor | Description | Identity source |
|---|---|---|
| **Person** | A human whose identity the server verifies. Owns tokens. | RS256 JWT |
| **Calling agent** | An MCP client acting on behalf of a Person. Holds no identity of its own. | Forwarded header or `Authorization` |
| **Operator** | Deploys and configures the server. Edits YAML, runs the migrate verb. | Filesystem and process control |
| **Platform admin** | Manages projects and membership. Gains no implicit file access or access to another Person's tokens. | RS256 JWT with admin claim |

---

## 5. Usage Scenarios

### SC-001: Operator configures the host-to-provider map
Operator adds host-to-provider entries, starts the server; every entry is validated (hostname
syntactically valid, provider one of the four accepted values, no duplicate host); the map is
then immutable for the process lifetime. An unknown provider value, a duplicate host, or a
malformed host each fail boot, naming the offender. An absent or empty map boots fine; every
remote operation then fails per SC-005.

### SC-002: Calling agent seeds a token it already holds
Agent calls `git.token_set` with `host`, `token`, optionally `expires_at`. The server resolves
the provider from the map, stores the token keyed `(person, host)` encrypted at rest, and
returns success without echoing the token. An undeclared host, an anonymous host, an empty or
oversized token, or a persistence failure each reject the call and store nothing (a persistence
failure additionally rolls back any in-memory write).

### SC-003: Person completes the device flow for a specific host
Person calls `git.auth` with `provider` and optionally `host`. An omitted host defaults to the
provider's canonical public host. A host mapping to a different provider, or to `generic`/
`anonymous`, is rejected. On success the token is stored keyed `(person, host)`.

### SC-004: Clone a private repository on a mapped enterprise host
Agent calls `git.remote_clone`. The server authorises the Person on the mount, parses the
hostname, resolves the provider, looks up and checks the token's expiry, then clones with that
credential and records the clone URL as remote `origin`. The response reports the resolved
`auth`.

### SC-005: Clone from an undeclared host
A host absent from the map fails the call before any network call, naming the host and stating
it must be declared. A host declared `anonymous` instead proceeds with no credential.

### SC-006: Person manages their tokens in the browser
Person navigates to the token screen (`/app/tokens`), authenticated by the existing identity
layer (forwarded header, `Authorization` header, or a read-only `mcpfs_token` cookie). The page
lists the hosts they hold tokens for (host, provider, validity), seeds or revokes a token, and
never renders a token value. No person, including a platform admin, can view or mutate another
person's tokens.

### SC-007: Status and revocation scoped to a host
`git.auth_status` with optional `provider`/`host` reports one entry per stored `(person, host)`.
`git.auth_revoke` with `provider` and/or `host` deletes exactly one stored token, defaulting to
the provider's canonical public host when `host` is omitted, and never revokes across hosts.

### SC-008: Upgrade drops existing tokens
Deploying the new version rebuilds the token table on the new primary key and drops every
pre-existing row, identically on SQLite, PostgreSQL and SQL Server. Each person re-authenticates
or reseeds once.

### SC-009/SC-010: Push a branch, refused as non-fast-forward
`git.remote_push` sends one named branch to `origin`, creating it on the remote when absent, and
reports whether it was created, already up to date, or pushed. A non-fast-forward rejection by
the remote is surfaced as a distinct error; force push is not supported.

### SC-011: Fetch updates refs and objects only
`git.remote_fetch` downloads new objects and updates `refs/remotes/origin/*` only; `refs/heads/*`
and every volume file stay byte-for-byte unchanged. A branch removed upstream is reported in
`refs_stale`, not pruned.

### SC-012/SC-013: Pull applies a fast-forward, or refuses a non-fast-forward without a strategy
`git.remote_pull` fetches, then advances the local branch and the volume's files only when the
local tip is an ancestor of the fetched remote tip. A dirty volume refuses before fetching. A
diverged branch with no `on_conflict` is refused, but the fetch's results (objects and
remote-tracking refs) are kept.

### SC-014: Remote operation with an expired stored token
An expired token fails the operation before any network call, naming the host; the expired
token is not deleted, so status can still report `expired` rather than absent.

### SC-015: Pull merges a diverged branch with a global strategy
With `on_conflict` of `ours` or `theirs`, a three-way merge resolves every conflicting file by
that single strategy and creates a merge commit authored by the Person, with no conflict markers
ever entering the volume.

---

## 6. Functional Requirements

EARS notation. All requirements below are **implemented**; the Evidence column cites current
code, not the pre-implementation state the legacy document described.

| Id | Requirement (summary) | Evidence |
|---|---|---|
| FR-NEW-001 | `git.hosts` config key, host→provider map (`github`\|`gitlab`\|`generic`\|`anonymous`) | `crates/core/src/config.rs:504` |
| FR-NEW-002 | Boot rejects an unknown provider value, naming host and accepted values | `crates/core/src/git/remote.rs:155` |
| FR-NEW-003 | Boot rejects a duplicate host | `crates/core/src/git/remote.rs:155` |
| FR-NEW-004 | Boot rejects a malformed host (scheme/path/port/wildcard) | `crates/core/src/git/remote.rs:155` |
| FR-NEW-005 | The map is immutable at runtime; no tool/route mutates it | `crates/core/src/git/remote.rs` (no write path) |
| FR-NEW-006 | Hostname resolution by parsing + exact case-insensitive match | `crates/core/src/git/remote.rs:182` |
| FR-NEW-007 | An undeclared host fails before any network call | `crates/core/src/git/remote.rs:182` |
| FR-NEW-008 | A host declared `anonymous` carries no credential; `auth:"anonymous"` | `crates/core/src/git/remote.rs:182` |
| FR-NEW-009 | Token identity is `(person, host)`, case-insensitive on both parts | `crates/core/src/git/oauth/store.rs:77` |
| FR-NEW-010 | Persisted primary key `(person, host)` on all three backends | `crates/core/src/git/oauth/persistence.rs:42-56` |
| FR-NEW-011 | Migration drops legacy `(person, provider)` rows | `crates/core/src/git/oauth/persistence.rs:31-60` (`recreate_if_missing_column`) |
| FR-NEW-012 | `mcp-fs migrate` copies `oauth_tokens` under the new key | `crates/core/src/migrate.rs` |
| FR-NEW-013 | `git.token_set` stores a supplied token, never echoes it | `crates/core/src/tools/git_auth.rs` (`token_set`) |
| FR-NEW-014 | Absent `expires_at` means non-expiring | `crates/core/src/git/oauth/store.rs:30-45` |
| FR-NEW-015 | Empty/whitespace/>8192-char token rejected | `crates/core/src/tools/git_auth.rs` |
| FR-NEW-016 | Undeclared or `anonymous` host rejected by the seeding tool | `crates/core/src/tools/git_auth.rs` |
| FR-NEW-017 | Seeding fails loudly on a persistence write failure | `crates/core/src/git/oauth/store.rs` |
| FR-NEW-018 | Expiry enforced before any network call | `crates/core/src/git/oauth/store.rs:203` (`has_valid_token`), `:233` (`require_valid_credential`) |
| FR-NEW-019 | An expired token is not deleted | `crates/core/src/git/oauth/store.rs:182-203` |
| FR-NEW-020 | Clone records the clone URL as remote `origin` | `crates/core/src/tools/git.rs:2928` (`remote_clone`) |
| FR-NEW-021 | No `origin` → push/fetch/pull rejected | `crates/core/src/git/db.rs` (`list_remotes`), checked in `git.rs:3181`/`3504`/`6196` |
| FR-NEW-022–025 | Push sends one named branch, creates it if absent, reports up-to-date idempotently | `crates/core/src/tools/git.rs:3181` (`remote_push`) |
| FR-NEW-026–027 | Fetch updates objects + `refs/remotes/origin/*` only, never local refs/files | `crates/core/src/tools/git.rs:3504` (`remote_fetch`) |
| FR-NEW-028–030 | Fast-forward pull; dirty volume refused; diverged pull without strategy refused but fetch results kept | `crates/core/src/tools/git.rs:6196` (`remote_pull`) |
| FR-NEW-031–034 | Diverged pull with `ours`/`theirs` merges via `MergeOptions::file_favor` + `merge_commits`, merge commit authored by Person with auto-generated message | `crates/core/src/tools/git.rs:6196`, `crates/core/src/git/merge.rs` |
| FR-NEW-035–036/069 | Pull is atomic; quota charged on the delta only, before any write | `crates/core/src/tools/git.rs:6196` |
| FR-NEW-037–040 | Token screen on the main server behind identity; lists hosts without values; seeds/revokes via the same operations; never exposes another person's tokens | `crates/core/src/token_screen.rs:1-60` |
| FR-NEW-041–042 | Non-HTTPS and userinfo-bearing remote URLs rejected | `crates/core/src/git/remote.rs` |
| FR-NEW-043 | No token on any observable surface | redaction tests, e.g. `crates/core/src/tools/git.rs:8381` |
| FR-NEW-044 | Remote operations bounded by `git.remote_timeout_secs` (default 120), lock released on timeout | `crates/core/src/config.rs:467`, `crates/core/src/git/repo.rs` |
| FR-NEW-045/071 | Frozen tool contract records the new/modified schemas, required parameters | `TOOL_CONTRACT.txt:459,470,475,559`; `tool-contract-golden.json` |
| FR-NEW-046–048/058–059 | Fixed token-screen routes, three-source identity resolution, `csrf_token` CSRF defense, no `Set-Cookie` ever emitted | `crates/core/src/token_screen.rs:1-335` |
| FR-NEW-049 | Fixed error codes/prefixes for the four named failure modes | `crates/core/src/errors.rs`; message prefixes in `tools/git.rs` |
| FR-NEW-050/060/051 | Fixed response shapes for fetch, push, `auth_status`/`auth_revoke` | `crates/core/src/tools/git.rs:3504,3181`; `crates/core/src/tools/git_auth.rs` |
| FR-NEW-052 | `expires_at` parsed as RFC 3339, rejects other forms | `crates/core/src/tools/git_auth.rs` |
| FR-NEW-053 | `auth` always reports the resolved provider | `crates/core/src/git/remote.rs` |
| FR-NEW-054/065 | One module (`git/remote.rs`) owns host resolution, URL validation, credential supply, timeout and the four remote operations; `config.rs` only calls `validate_hosts` | `crates/core/src/git/remote.rs:155,182`; `crates/core/src/config.rs:922` |
| FR-NEW-055 | (Subsumed by FR-NEW-069, same evidence) | `crates/core/src/tools/git.rs:6196` |
| FR-NEW-056 | Fetch does not prune; stale remote-tracking refs reported in `refs_stale` | `crates/core/src/tools/git.rs:3504` |
| FR-NEW-057/072 | Every remote operation records one audit entry and one `git.remote` tracing span | `crates/core/src/tools/git.rs` (`safety.record_audit` call sites), `#[tracing::instrument]` on the remote fns |
| FR-NEW-061 | A successful push advances `refs/remotes/origin/{branch}` | `crates/core/src/tools/git.rs:3181` |
| FR-NEW-062 | Fetch uses one explicit refspec, no tag following | `crates/core/src/tools/git.rs:3504` |
| FR-NEW-063 | Pull targets only the checked-out branch, rejecting a mismatch before fetching | `crates/core/src/tools/git.rs:6196` |
| FR-NEW-064 | A failed persistence write removes the in-memory entry, restoring any prior value | `crates/core/src/git/oauth/store.rs` |
| FR-NEW-066 | `instance_url` and `host` must agree on `git.auth` | `crates/core/src/tools/git_auth.rs` |
| FR-NEW-067 | Token screen rows ordered by host ascending | `crates/core/src/token_screen.rs` |
| FR-NEW-068 | `git.auth_revoke` takes optional provider/host, at least one required, reports resolved provider | `crates/core/src/tools/git_auth.rs` |
| FR-NEW-070 | Fixed `ERR_UNAUTHENTICATED` codes/prefixes for no-token and expired-token | `crates/core/src/errors.rs`; `crates/core/src/git/oauth/store.rs:233` |
| FR-MOD-001 | Provider resolution via the host map, not substring detection | `crates/core/src/git/remote.rs:182` |
| FR-MOD-002 | `git.auth` accepts optional `host`, defaults to canonical public host | `crates/core/src/tools/git_auth.rs` |
| FR-MOD-003 | `git.auth_status` reports one entry per stored `(person, host)` | `crates/core/src/git/oauth/store.rs:301` (`list_for_person`) |
| FR-MOD-004 | `git.auth_revoke` revokes exactly one `(person, host)` pair | `crates/core/src/tools/git_auth.rs` |

(FR-NEW-055 is subsumed into FR-NEW-069, as recorded in the source; both are listed because the
legacy traceability matrix cites both ids.)

---

## 7. Non-Functional Requirements

Inherited and unchanged from SPEC-0007: tokens encrypted at rest under `MCPFS_TOKEN_KEY`;
persistence conditional on that key; git tools gated by project membership, never platform
admin; auth tools gated by authentication alone; writes serialized per repository; boot
validation on misconfiguration.

| Category | Requirement | Evidence/Measure |
|---|---|---|
| Performance | Remote operations run off the request thread, time-bounded by `git.remote_timeout_secs` | `crates/core/src/tools/git.rs` (`spawn_blocking` path), `crates/core/src/config.rs:467` |
| Security | No token on any observable surface; HTTPS-only transport; no userinfo in remote URLs; strict per-person token isolation; bounded token size (8192 chars); token-screen CSRF defense with no session cookie ever emitted | `crates/core/src/git/remote.rs`; `crates/core/src/token_screen.rs` |
| Reliability | Pull is atomic; boot validation total; push/fetch/pull all idempotent when already current; backend parity across SQLite/PostgreSQL/SQL Server via the conformance suite | `crates/core/src/storage/conformance.rs` |
| Observability | One `git.remote` tracing span and one audit entry per remote operation, success or failure, never carrying a token | `crates/core/src/tools/git.rs` |
| Deployment | No new infrastructure; `MCPFS_TOKEN_KEY` remains the only secret involved | `crates/core/src/config.rs` |
| Scalability | Token storage grows as tokens-per-person × hosts-per-person, a small constant; `host` is a bounded `TextKey` so SQL Server can index it | `crates/core/src/git/oauth/persistence.rs:25` |

---

## 8. E2E Tests

The source specification enumerated 247 new end-to-end test specifications (`E2E-NEW-001` to
`E2E-NEW-247`) plus 26 modified existing tests (`E2E-MOD-001` to `E2E-MOD-026`), each mapped to a
scenario and a functional requirement, with every scenario carrying at least one happy, one
failure and one edge test (happy 49 : failure 111 : edge 57, plus 10 side-effect, 7 state-
transition, 7 data-integrity and 6 performance tests). All are implemented as `#[cfg(test)]`
unit/integration tests alongside the code they exercise, plus functional shell-driven scenarios
for the core journeys (token seeding, clone, push, fetch, pull with and without merge).

Representative, verified-in-tree examples:

- `crates/core/src/tools/git.rs:8270` `remote_clone_is_charged_up_front_and_writes_nothing_when_over_quota`
- `crates/core/src/tools/git.rs:8317` `remote_clone_imports_files_history_and_refs`
- `crates/core/src/tools/git.rs:8381` `remote_clone_surfaces_a_failure_without_leaking_the_token`
- `crates/core/src/tools/git.rs:8417` `remote_clone_needs_membership`
- `crates/core/src/tools/git.rs:8712` `e2e_new_defect_token_set_then_remote_clone_share_the_real_host`
- `crates/core/src/tools/git.rs:11535` `e2e_new_531_remote_pull_is_blocked_while_a_merge_is_in_progress`
- `crates/core/src/tools/git.rs:23408` `a_named_remote_push_emits_exactly_the_base_response_keys`
- `crates/core/src/config.rs:1495` `a_reference_git_hosts_map_boots_via_yaml`
- `crates/core/src/config.rs:1510` (duplicate-host boot-rejection test)
- `crates/core/src/git/oauth/persistence.rs:289` `one_row_per_person_host_pair`
- `crates/core/src/token_screen.rs:454,478,490` (cookie-authenticated and CSRF-token assertions)

The full enumerated list of 247+26 specifications is preserved in the archived legacy document
(`specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`,
Section 12) and is not reproduced verbatim here; this section records that the suite exists, is
implemented, and is distributed one-to-one with the functional requirements of Section 6.

---

## 9. Glossary

| Term | Definition |
|---|---|
| **Host** | The bare hostname of a remote, extracted by parsing a URL. Never a URL, never a substring. |
| **Provider** | A credential policy: `github`, `gitlab`, `generic` or `anonymous`. |
| **Host map** | The `git.hosts` configuration mapping hosts to providers. Boot-validated, immutable at runtime. |
| **Declared host** | A host present in the host map. Its absence is an error, never an implicit anonymous. |
| **Canonical public host** | The host assumed when `host` is omitted: `github.com` for `github`, `gitlab.com` for `gitlab`. |
| **Seeding** | Supplying an already-held token to the server directly, without an interactive flow. |
| **Token identity** | The pair `(person, host)` that uniquely identifies one stored credential. |
| **Device flow** | The OAuth device authorization grant. |
| **Origin** | The single remote recorded for a volume at clone time, under the name `origin`. |
| **Fast-forward** | An advance where the current tip is an ancestor of the target, requiring no merge. |
| **Divergence** | The state where neither the local tip nor the remote tip is an ancestor of the other. |
| **Conflict strategy** | The global `ours` or `theirs` choice resolving every conflicting file in one merge. |
| **Dirty volume** | A volume whose files differ from its current branch tip; there is no staging area. |
| **Volume** | A project's simulated filesystem, simultaneously the git working tree. |

---

## 10. Confidence Notes

- This document is a migration of an archived legacy specification
  (`specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`)
  into the current numbering, renumbered **SPEC-0011** because the legacy id SPEC-0010 collides
  with the active, separately migrated SPEC-0010-project-storage-quota. The requirement ids
  (`FR-NEW-*`, `FR-MOD-*`) are kept verbatim from the legacy document; only the spec's own id
  changed.
- Status is **implemented**: every cited `crates/core/src/...` evidence path was verified
  against the current tree during migration (not against the legacy document's own code
  citations, which pointed at a now-renamed `crates/mcp-fs` tree predating the `crates/core`
  restructuring).
- The legacy document's own Section 18 (Implementability Audit) recorded verdict
  `NOT-IMPLEMENTABLE` after three audit rounds, each finding new residual naming/response-shape
  gaps rather than functional gaps, and the user explicitly accepted the risk and proceeded. The
  story index (`stories/_index.md`) confirms all 19 resulting stories shipped `done`, and the
  evidence gathered during this migration (Section 6, above) confirms the corresponding code
  exists and matches the requirements. No attempt was made during migration to re-run a fourth
  audit round.
- Six implementation-time drift findings (`DRIFT-003`, `DRIFT-004`, `DRIFT-005`, `DRIFT-008`,
  `DRIFT-009`, `DRIFT-010`) were open at spec-authoring time; all are resolved in the shipped
  code (see design.md "Legacy mapping" for each one's resolution).
- Two of the legacy document's three open TBDs (Section 15) were resolved by later requirements
  within the same spec (token-screen route path, by FR-NEW-046–048); the `expires_at`
  representation TBD was left to implementer choice by design and is not re-litigated here. The
  punycode TBD (E2E-NEW-062) is treated as closed by the shipped `url`-crate-based hostname
  parsing in `git/remote.rs`, not independently re-verified during this migration.
- This migration did not re-audit prose for stray file:line mentions beyond an easy pass;
  Section 6's Evidence column intentionally carries file:line citations, consistent with the
  "Evidence columns only" rule for this document type.
