> Id: SPEC-0007
> Nature: FEAT
> Status: as-built
> Area: git
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Git — Base Git Surface Specification

## 1. Summary

The git capability exposes a simulated git repository per project: 11 `git.*` tools for history inspection and local commits, 3 `git.auth*` tools for an OAuth device-flow login to GitHub/GitLab, a blob-backed git object database, and a git smart-HTTP server at `/git/{mount_id}/` that a real `git` CLI can clone from and push to. Git objects are content-addressed stored blobs under `git:{sha}` keys, identical in bytes to what any git implementation stores; the on-disk bare repository that libgit2 reads and writes is a rebuildable cache, hydrated before every read and drained back into the blob store after every write. The whole family is gated behind `git.enabled` and is absent from the tool catalogue when disabled.

Three behaviours are deliberately stricter or more correct than the prior implementation this surface diverged from: project membership is enforced on every git HTTP route (not just tool calls), `max_pack_size_mb` is actually enforced rather than parsed and ignored, and the push response includes the `unpack ok` report-status line a real `git push` requires to report success.

## 2. Current State

### 2.1 Prior Specification
None. This is the first specification of this capability (retro-specification of already-shipped behaviour).

### 2.2 Related Specifications
- This surface is the foundation other specs build on. The branch/stash/merge/rebase/cherry-pick/revert/named-remotes/pull-request tool families, and the GitHub Enterprise host map and per-host token store, are governed by other, later specs that have since been migrated into this spec repository. See the migrated SPEC covering git hosts/tokens, and the migrated SPEC covering the full git dev process, for that surface; this document does not restate their requirements.
- The membership gate, error vocabulary and tool-catalogue gating this surface reuses are owned by the platform/filesystem foundation spec.
- The relational storage seam the git index sits on, and cross-deployment migration behaviour, are owned by the storage/backends spec.

### 2.3 Scope of This Document
This document covers only the original git surface as it stands today:
- The 11 `git.*` tools: `init`, `status`, `branches`, `tags`, `log`, `show`, `diff`, `commit`, `checkout_file`, `blame`, `remote_clone`.
- The 3 `git.auth*` tools: `auth`, `auth_status`, `auth_revoke`.
- The blob-backed git object database, the per-project repository store, and the smart HTTP server (ref advertisement, upload-pack, receive-pack, pkt-line framing, report-status, body limits, authentication challenge).
- The OAuth device flow for GitHub and GitLab, token storage and encryption at rest.
- Authorization rules specific to git, including the distinction between membership-gated tools/routes and authentication-only auth tools.

Out of scope of this document (covered by the specs named in 2.2): branch lifecycle, reset, stash, merge/rebase/cherry-pick/revert and the shared conflict-resolution engine, named remotes and force-push-with-lease, token seeding without the device flow, the pull-request surface, GitHub Enterprise host configuration, and the per-host token store.

## 3. Scope

### 3.1 In Scope
- Git object storage, keyed and indexed, with export/import against an on-disk cache.
- A per-project repository with a lazily opened, in-process cache and a per-repository write lock.
- 11 history/local-commit tools and 3 authentication tools.
- A git smart HTTP server serving clone, fetch and push over HTTP against this server's own storage.
- An OAuth device-flow login to GitHub or GitLab, encrypted token persistence.

### 3.2 Out of Scope
- Real have/want negotiation; every fetch transfers a full pack.
- Git hooks, submodules, LFS, sparse checkout.
- Any web UI for browsing history.
- Branch/stash/merge/rebase/cherry-pick/revert, named remotes, pull requests, GitHub Enterprise hosts, and the per-host token store (all covered elsewhere, see 2.2).

## 4. Actors

| Actor | Description |
|---|---|
| LLM agent | Reads history and diffs through the `git.*` tools, and commits changes it made through the filesystem tools. |
| Developer with a git CLI | Clones and pushes over HTTP against `/git/{mount_id}/`. |
| Project member | The authenticated identity behind both; membership, not platform administration, grants access. |
| Operator | Enables git, sets `max_pack_size_mb`, decides on anonymous read, and provides OAuth client credentials. |
| Remote provider | GitHub or GitLab, reached by the device flow. |

## 5. Usage Scenarios

### US-001: Agent initializes a repository and commits work
**Actor:** LLM agent.
**Preconditions:** git enabled; caller is a project member.
**Flow:**
1. Agent calls `git.init` for the project.
2. The repository is created lazily and cached in process.
3. Agent edits files through filesystem tools.
4. Agent calls `git.commit`; the new commit's objects are imported into the blob store and the object index.
5. Agent calls `git.status`, `git.log` and `git.show` to verify.

**Postconditions:** the commit exists in the blob store as the authoritative copy; the on-disk object directory holds a rebuildable cache of the same bytes.
**Exceptions:**
- Git disabled: the tools are absent from the catalogue entirely, so the call is an unknown-tool error.
- Caller is not a member: forbidden, including for a platform admin.
- A read tool called before any commit: returns an empty history rather than an error.
- The process restarted since `git.init`: the repository opens on demand from intact on-disk state rather than reporting "not initialized".

### US-002: Agent inspects history
**Actor:** LLM agent.
**Preconditions:** member; the repository has commits.
**Flow:**
1. Agent calls `git.log`, `git.show`, `git.diff`, `git.blame`, `git.branches` or `git.tags`.
2. Objects are exported into the on-disk object database before the read.
3. The read is answered from the hydrated directory.
4. `git.checkout_file` restores one file's committed content into the project volume.

**Postconditions:** the agent holds the requested history; the on-disk cache now holds the objects it needed.
**Exceptions:**
- An unknown revision: an error naming it.
- A short sha: resolved by prefix lookup against the object index; an ambiguous prefix is reported as such.
- `git.checkout_file`'s write: charged and audited like any filesystem write.

### US-003: Developer clones and pushes over HTTP
**Actor:** Developer with a git CLI.
**Preconditions:** git enabled; a credential the CLI can present; caller is a member.
**Flow:**
1. `git clone http://host/git/{mount_id}/` requests a ref advertisement.
2. The server answers with its refs and capability string.
3. The CLI requests the pack; the server builds a full pack of every wanted tip and its whole ancestry, and answers NAK.
4. The developer commits locally and pushes.
5. The incoming pack is indexed, its objects imported into the blob store, refs updated, and a report-status reply sent beginning with `unpack ok`.

**Postconditions:** the clone contains the project's history; the push has updated refs and the pushed objects are indexed, not merely written to disk.
**Exceptions:**
- No credential: a 401 carrying a bearer challenge; anonymous CLI use needs an explicit opt-in or a credential helper, since the CLI only prompts on a Basic challenge.
- A verified but non-member credential: refused.
- A push body over the configured size limit: refused with 413.
- An unknown project: refused before any protocol work.

### US-004: Agent clones from a private remote after authorizing
**Actor:** LLM agent, then a human at a browser.
**Preconditions:** git enabled; OAuth client credentials configured.
**Flow:**
1. Agent calls `git.auth` naming the provider.
2. The server requests a device code and returns as soon as a user code is available, without waiting for approval.
3. A background task polls the token endpoint.
4. The human enters the user code in a browser and approves.
5. The agent polls `git.auth_status` until the token is stored, then calls `git.remote_clone`.
6. The clone's objects are imported directly into the blob store.

**Postconditions:** the token is held for that person/provider pair and, when persistence is configured, survives a restart encrypted; the remote's history is in the project.
**Exceptions:**
- The human denies: the pending authorization ends.
- The device code expires: the pending authorization ends.
- No persistence key configured: the token store is memory-only and tokens are lost on restart.
- `git.auth_revoke`: the stored token is removed.

### US-005: Operator moves a deployment or deletes a project
**Actor:** Operator or platform admin.
**Preconditions:** a deployment with git state.
**Flow:**
1. Deleting a project tears down the volume, then purges the project's git rows and on-disk git state.
2. Migrating a deployment copies the git tables along with everything else, and deliberately skips OAuth tokens.

**Postconditions:** a project recreated under the same id inherits no stale refs; a migrated deployment keeps its git history and needs its OAuth authorizations re-established.
**Exceptions:**
- Git disabled at deletion time: the git purge is skipped.

## 6. Functional Requirements

### Object storage

**FR-001:** Git objects SHALL be stored as content-addressed blobs under the key `git:{sha}`, holding the canonical git object bytes (type, length, payload).

**FR-002:** Each stored git object SHALL carry an index row recording its hash, type and size, scoped to its project.

**FR-003:** The server SHALL NOT register a custom libgit2 object-database backend; it SHALL instead export blob-backed objects into an on-disk cache before reads and import on-disk writes back into the blob store.

**FR-004:** WHEN an operation needs to read git objects via the embedded git engine, THE server SHALL first export the blob-backed objects into the on-disk object cache.

**FR-005:** WHEN the embedded git engine has written objects, THE server SHALL import them back into the blob store and the object index.

**FR-006:** WHEN an object is requested by a sha prefix, THE server SHALL resolve it against the index, reporting ambiguity or not-found as distinct outcomes.

### Repository

**FR-007:** Each project SHALL have at most one git repository, created on demand and cached in process.

**FR-008:** IF the in-process repository cache has no entry for a project whose on-disk git state exists, THEN the server SHALL open it from disk rather than report that the repository is not initialized.

**FR-009:** The server SHALL serialize mutating git operations against one repository with a per-repository write lock.

**FR-010:** WHEN a project is deleted and git is enabled, THE server SHALL purge that project's git rows and on-disk repository state.

### Tools

**FR-011:** The 11 `git.*` tools and 3 `git.auth*` tools SHALL be registered only when git is enabled, and SHALL be entirely absent from the tool catalogue otherwise.

**FR-012:** Every `git.*` tool SHALL authorize by project membership and SHALL NOT admit a caller solely on the basis of a platform administrator role.

**FR-013:** The 3 `git.auth*` tools SHALL require an authenticated caller and SHALL NOT require project membership.

**FR-014:** WHEN a history or inspection tool is called, THE server SHALL answer from the repository after hydrating the on-disk object cache; `git.checkout_file`'s write into the project volume SHALL be charged and audited like any other write.

### Smart HTTP

**FR-015:** The server SHALL serve a ref-advertisement route selecting between the clone/fetch and push services by a query parameter, plus one route each for the pack-upload and pack-receive services.

**FR-016:** Every git HTTP route SHALL require the caller to be a member of the project; a platform administrator role SHALL NOT substitute for membership.

**FR-017:** IF a git route is called without an acceptable credential, THEN the server SHALL answer with an HTTP 401 carrying a bearer authentication challenge.

**FR-018:** WHEN a ref advertisement is requested, THE server SHALL answer with the service identifier, the refs, and a capability string for that service.

**FR-019:** WHEN a fetch is served, THE server SHALL push every wanted tip onto a traversal, build a pack carrying those tips and their whole ancestry, and answer with a NAK (no incremental negotiation).

**FR-020:** WHEN a push completes, THE server SHALL send a report-status reply beginning with an unpack-success line, followed by per-ref outcome lines.

**FR-021:** WHEN a pack is received, THE server SHALL index its objects into the blob store and the object index, not merely write them to the on-disk cache.

**FR-022:** IF a push body exceeds the configured maximum pack size, THEN the server SHALL refuse it with an HTTP 413, and the refusal SHALL leave no partial state in the object index or blob store.

**FR-023:** The server SHALL frame every protocol message using git's line-oriented wire framing, including flush markers and the side-band channel carrying the push report.

### OAuth device flow

**FR-024:** WHEN the auth tool is called, THE server SHALL request a device code from the named provider and return as soon as a user code is available, continuing to poll the token endpoint in the background rather than blocking the call.

**FR-025:** WHEN the token endpoint is polled, THE server SHALL treat a pending-authorization or slow-down outcome as continue, and a denial or expiry outcome as terminal; a provider response omitting a poll interval SHALL fall back to a default interval.

**FR-026:** The server SHALL read an OAuth client secret from its configured environment variable at the time it is needed, and SHALL NOT hold it in a long-lived field.

**FR-027:** WHEN a token is persisted, THE server SHALL encrypt it with an authenticated cipher before writing it to storage.

**FR-028:** IF no persistence key is configured, THEN the token store SHALL be memory-only and tokens SHALL be lost on restart.

**FR-029:** WHEN the auth-status tool is called, THE server SHALL report whether a token is held for that caller and provider; WHEN the auth-revoke tool is called, THE server SHALL remove it.

## 7. Non-Functional Requirements

### 7.1 Performance
- Every fetch transfers a full pack (FR-019); there is no incremental negotiation, so fetch cost scales with total history rather than with the delta. Recorded as an open question (see §10).
- The on-disk object cache avoids re-exporting objects already present for repeated reads.
- The maximum pack size bounds the memory and time a single push can consume.

### 7.2 Security
- Membership is enforced on both tools and HTTP routes; platform administration grants nothing (FR-012, FR-016).
- OAuth client secrets are read at call time and never held in a field (FR-026).
- Tokens are encrypted at rest with an authenticated cipher (FR-027).
- Cross-deployment migration deliberately does not carry OAuth tokens.
- An operator-enabled anonymous-read relaxation is the only way the git CLI works without a credential helper (FR-017).

### 7.3 Usability
- The auth tool returns immediately with a user code rather than blocking for a human to approve (FR-024).
- A cold process opens an existing repository rather than demanding re-initialization (FR-008).
- The push report includes the line a real `git push` needs to report success (FR-020).
- Tools are absent, rather than present and failing, when git is disabled (FR-011).

### 7.4 Reliability
- The blob store is the source of truth and the on-disk directory is rebuildable (FR-001, FR-004), so losing the cache loses nothing.
- Pushed objects are indexed rather than left only on disk (FR-021).
- Per-repository write locks serialize mutation (FR-009).
- Project deletion purges git state so a recreated project starts clean (FR-010).

### 7.5 Observability
No git-specific instrumentation exists: no per-operation timing, pack-size metric, or export/import volume counter. Recorded as an open question (see §10).

### 7.6 Deployment
- Git is disabled by default and can be forced on by an operator override.
- A configured maximum pack size bounds push bodies.
- An anonymous-read setting governs unauthenticated reads.
- A persistence key, when configured, must decode to the cipher's required key length for token persistence to survive a restart.
- OAuth client id and the environment-variable name holding the client secret are configured per provider.

### 7.7 Scalability
- One repository and one index per project; project count costs directories and files.
- The on-disk object cache grows without bound; nothing prunes it. Since it is rebuildable, pruning is safe to add later. Recorded as an open question (see §10).
- Full packs mean fetch cost grows with history size rather than with the delta (restates 7.1).

## 8. E2E Tests

| Test ID | Scenario | Requirements | Category |
|---|---|---|---|
| E2E-001 | US-001 | FR-001, FR-005 | Core journey: commit objects reach the blob store |
| E2E-002 | US-001 | FR-002, FR-007 | Core journey: object index row, repository created lazily |
| E2E-003 | US-001 | FR-001, FR-011 | Error: git tools absent when disabled |
| E2E-004 | US-001 | FR-002 | Error: every stored object carries an index row, scoped per project |
| E2E-005 | US-001 | FR-005, FR-011 | Error: a commit's objects reach the blob store, not only the disk cache |
| E2E-006 | US-001 | FR-001, FR-007 | Edge: object key layout is exactly `git:{sha}` |
| E2E-007 | US-001 | FR-002, FR-011 | Edge: index tables exist and are scoped per project |
| E2E-008 | US-001 | FR-005, FR-007 | Edge: repository created lazily on first use |
| E2E-009 | US-002 | FR-003, FR-004 | Core journey: history reads hydrate the cache before reading |
| E2E-010 | US-002 | FR-006, FR-014 | Feature: history/inspection tools answer correctly |
| E2E-011 | US-002 | FR-003, FR-014 | Error: an unknown revision errors by name |
| E2E-012 | US-002 | FR-006, FR-014 | Error: an ambiguous short sha is reported as such |
| E2E-013 | US-002 | FR-003, FR-004 | Edge: history reads work from a cold object cache |
| E2E-014 | US-002 | FR-004, FR-006 | Edge: `checkout_file` restores content and is audited |
| E2E-015 | US-003 | FR-015, FR-018, FR-023 | Core journey: a real git CLI clones over HTTP |
| E2E-016 | US-003 | FR-019, FR-020, FR-021 | Core journey: a real git CLI pushes and the push reports success |
| E2E-017 | US-003 | FR-015, FR-016 | Security: git routes refuse an unauthorized/non-member caller |
| E2E-018 | US-003 | FR-017 | Security: an unauthenticated request carries the bearer challenge |
| E2E-019 | US-003 | FR-018, FR-019 | Error: a fetch transfers full history rather than a delta |
| E2E-020 | US-003 | FR-020, FR-021 | Error: pushed objects are really indexed |
| E2E-021 | US-003 | FR-022, FR-023 | Error: an oversized push is refused with 413 |
| E2E-022 | US-003 | FR-015, FR-016, FR-017 | Edge: git routes refuse a non-member and a platform admin alike |
| E2E-023 | US-003 | FR-017, FR-023 | Edge: unauthenticated request vs. anonymous-read enabled |
| E2E-024 | US-003 | FR-018, FR-019 | Edge: advertisement capability string |
| E2E-025 | US-003 | FR-020, FR-022 | Edge: oversized push leaves refs unchanged |
| E2E-026 | US-003 | FR-021, FR-022 | Edge: a rejected push leaves no partial state |
| E2E-027 | US-004 | FR-024, FR-027 | Feature: device authorization and token encryption |
| E2E-028 | US-004 | FR-013, FR-025, FR-029 | Feature: status reports the token once the flow completes |
| E2E-029 | US-004 | FR-024, FR-026 | Error: missing client secret configuration is reported |
| E2E-030 | US-004 | FR-025, FR-028 | Error: poll outcomes classified; memory-only store without a key |
| E2E-031 | US-004 | FR-026, FR-027 | Security: client secret never logged; token encrypted at rest |
| E2E-032 | US-004 | FR-013, FR-028, FR-029 | Error: auth tools need authentication but not membership |
| E2E-033 | US-004 | FR-024, FR-025 | Edge: poll outcomes are classified correctly, including missing interval |
| E2E-034 | US-004 | FR-013, FR-026, FR-029 | Edge: client secret read from environment at call time, not cached |
| E2E-035 | US-004 | FR-027, FR-028 | Edge: persistence conditional on configured key |
| E2E-036 | US-005 | FR-008, FR-010 | Core journey: a restart does not lose an initialized repository |
| E2E-037 | US-005 | FR-009, FR-016 | Security: concurrent writes to one repository are serialized |
| E2E-038 | US-005 | FR-009, FR-012 | Error: a non-member cannot commit |
| E2E-039 | US-005 | FR-010, FR-012 | Error: deleting a project purges its git state |
| E2E-040 | US-005 | FR-008, FR-009, FR-012 | Edge: a platform admin is refused on both tools and routes |
| E2E-041 | US-005 | FR-008, FR-010 | Edge: recreating a project after deletion inherits nothing |

## 9. Glossary

| Term | Definition |
|---|---|
| Git object | A commit, tree, tag or git blob, stored as bytes of type, length and payload. |
| Stored blob | The platform's content-addressed byte storage, holding git objects under `git:{sha}` keys. |
| Object index | The index rows supporting enumeration and short-sha prefix lookup. |
| Export | Hydrating the on-disk object cache from the stored blobs before a read. |
| Import | Copying objects written on disk back into the stored blobs and the index. |
| Bare directory | The on-disk per-project directory holding git engine bookkeeping and a rebuildable object cache. |
| Write lock | The per-repository lock serializing mutating operations. |
| pkt-line | The git wire framing carrying every protocol message. |
| Ref advertisement | The response listing refs and capabilities at the start of a clone or push. |
| upload-pack | The service serving a clone or fetch. |
| receive-pack | The service accepting a push. |
| report-status | The push reply beginning with the unpack-success line and carrying per-ref results. |
| Full pack | A pack carrying the wanted tips and their whole ancestry, with no negotiation. |
| Anonymous read | The operator-enabled relaxation letting an unauthenticated client read. |
| Device flow | The RFC 8628 grant used to obtain a provider token with a user code. |
| User code | The short string a human enters in a browser to approve an authorization. |

## 10. Confidence Notes

- Confidence: high. Source citations in the pre-move spec document (`crates/mcp-fs/src/git/...`) were spot-checked against the current tree and still hold in substance; only the crate root moved from `crates/mcp-fs/src/` to `crates/core/src/` (see design.md §9, Legacy mapping). The `unpack ok` line, the object-database doc comment, and file line counts were independently re-verified.
- Three open questions carried forward unresolved from the source document: full-pack-only fetches (no negotiation), absent git-specific instrumentation, and an unbounded on-disk object cache. These are not defects of this spec; they are accurately described current behaviour.
- This document deliberately excludes the 35 additional git tools (branches, stash, merge, rebase, cherry-pick, revert, named remotes, pull requests) and the GitHub Enterprise host/token-store surface, both shipped after this original surface and governed by other migrated specs, per the task's instruction not to merge their content in.
